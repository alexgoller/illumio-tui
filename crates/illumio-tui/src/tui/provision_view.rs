//! `I` in the pending overlay and `:impact` (#86): what provisioning the
//! draft would change, as `policy impact` prints it. The summary loads once
//! per open on the user's key (`Cmd::LoadImpact`); reopening shows the
//! loaded one with its age, and `r` in the overlay loads it again. No timer.
use crate::provision_impact::{ProvisionImpact, RuleChangeKind, coverage_line, unsupported};
use std::time::Instant;

/// The impact overlay's state (`PolicyState::impact`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImpactView {
    /// The last summary loaded.
    pub report: Option<ProvisionImpact>,
    pub loaded_at: Option<Instant>,
    /// A load is on its way (`I` and `r` do not send another).
    pub loading: bool,
    /// Why the last load failed.
    pub error: Option<String>,
    /// The first line shown.
    pub scroll: u16,
    /// Opened from the pending overlay: `Esc` goes back to it.
    pub from_pending: bool,
    /// The load an answer must carry to be taken (`Cmd::LoadImpact`);
    /// `forget` moves it on, so an answer read before a write is dropped.
    pub seq: u64,
    /// A ruleset or workload list behind the summary was cut short.
    pub rules_truncated: bool,
}

/// How a line of the report is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Heading,
    Text,
    Dim,
    /// An added rule, a flow the draft allows.
    Good,
    /// A removed rule, a flow the draft blocks.
    Bad,
    /// A changed rule.
    Changed,
}

/// The pair rows the overlay lists per direction.
pub const PAIRS_SHOWN: usize = crate::provision_impact::DEFAULT_PAIRS;

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The report as the overlay's lines: each section under its heading, the
/// first `limit` pairs of each direction, then what is not counted.
pub fn report_lines(r: &ProvisionImpact, limit: usize) -> Vec<(LineKind, String)> {
    use LineKind as K;
    let mut out: Vec<(LineKind, String)> = Vec::new();
    if r.pending == 0 {
        out.push((
            K::Text,
            "nothing pending: provisioning would change nothing".into(),
        ));
        return out;
    }
    out.push((
        K::Heading,
        format!(
            "Changes · {}",
            plural(r.pending as u64, "pending change", "pending changes")
        ),
    ));
    let tw = r
        .changes
        .iter()
        .map(|c| c.object_type.len())
        .max()
        .unwrap_or(0);
    for c in &r.changes {
        let mut line = format!("  {:tw$}  {:7} {}", c.object_type, c.change_type, c.count);
        if let Some(n) = c.in_request {
            line.push_str(&format!(" ({n} in the request)"));
        }
        out.push((K::Text, line));
    }
    out.push((K::Text, String::new()));
    if r.rulesets.is_empty() {
        out.push((K::Heading, "Rules · no pending ruleset".into()));
    } else {
        let (a, x, c) = r.rulesets.iter().fold((0, 0, 0), |(a, x, c), s| {
            (a + s.added, x + s.removed, c + s.changed)
        });
        out.push((
            K::Heading,
            format!(
                "Rules · {} · {a} added · {x} removed · {c} changed",
                plural(r.rulesets.len() as u64, "ruleset", "rulesets")
            ),
        ));
        out.push((
            K::Dim,
            "  workloads matched active → draft (- not in force)".into(),
        ));
        for s in &r.rulesets {
            let mut head = format!("  {} ({})", s.name, s.change);
            if s.rules.is_empty() {
                head.push_str(" no rule changes");
            }
            out.push((K::Text, head));
            for rc in &s.rules {
                let (sign, kind) = match rc.change {
                    RuleChangeKind::Added => ("+", K::Good),
                    RuleChangeKind::Removed => ("-", K::Bad),
                    RuleChangeKind::Changed => ("~", K::Changed),
                };
                out.push((kind, format!("    {sign} {}", rc.rule)));
                if let Some(w) = &rc.was {
                    out.push((K::Dim, format!("      was {w}")));
                }
                out.push((K::Dim, format!("      {}", coverage_line(rc))));
                for u in unsupported(rc) {
                    out.push((K::Dim, format!("      not counted: {u}")));
                }
            }
        }
    }
    if let Some(t) = &r.traffic {
        let x = &t.transitions;
        out.push((K::Text, String::new()));
        out.push((
            K::Heading,
            format!(
                "Traffic · {} · {} · {} newly blocked · {} newly allowed{}{}",
                t.window,
                plural(t.flows, "flow record", "flow records"),
                x.newly_blocked,
                x.newly_allowed,
                if x.not_evaluated > 0 {
                    format!(" · {} not evaluated", x.not_evaluated)
                } else {
                    String::new()
                },
                if t.truncated { " · truncated" } else { "" }
            ),
        ));
        if t.incomplete {
            out.push((
                K::Changed,
                "  a PCE region did not answer: flows are missing".into(),
            ));
        }
        let aw = x
            .changed
            .iter()
            .map(|c| c.arrow().chars().count())
            .max()
            .unwrap_or(0)
            .max("unchanged".len());
        for c in &x.changed {
            let kind = if c.draft == pce::models::PolicyDecision::Allowed {
                K::Good
            } else {
                K::Bad
            };
            out.push((
                kind,
                format!(
                    "  {:aw$}  {} · {} conns",
                    c.arrow(),
                    plural(c.flows, "flow", "flows"),
                    c.connections
                ),
            ));
        }
        out.push((
            K::Dim,
            format!(
                "  {:aw$}  {}",
                "unchanged",
                plural(x.unchanged, "flow", "flows")
            ),
        ));
        for (title, kind, pairs, total) in [
            (
                "Newly blocked or potentially blocked",
                K::Bad,
                &t.blocked_pairs,
                t.blocked_pairs_total,
            ),
            (
                "Newly allowed",
                K::Good,
                &t.allowed_pairs,
                t.allowed_pairs_total,
            ),
        ] {
            if total == 0 {
                continue;
            }
            let shown = pairs.len().min(limit);
            out.push((K::Text, String::new()));
            let of = if shown < total {
                format!("top {shown} of {total} pairs")
            } else {
                plural(total as u64, "pair", "pairs")
            };
            out.push((K::Heading, format!("{title} · {of}")));
            for p in pairs.iter().take(shown) {
                out.push((
                    kind,
                    format!(
                        "  {} → {}  {}  {}",
                        p.src.join("/"),
                        p.dst.join("/"),
                        p.services.join(", "),
                        plural(p.flows, "flow", "flows")
                    ),
                ));
            }
        }
    }
    if !r.not_counted.is_empty() {
        out.push((K::Text, String::new()));
        out.push((K::Heading, "Not counted".into()));
        out.extend(r.not_counted.iter().map(|n| (K::Dim, format!("  {n}"))));
    }
    out
}

impl ImpactView {
    /// After a write: the summary is stale; the next open loads it again.
    /// A load on its way is forgotten too: its answer, read before the
    /// write, is dropped when it lands.
    pub fn forget(&mut self) {
        self.report = None;
        self.loaded_at = None;
        self.error = None;
        self.loading = false;
        self.rules_truncated = false;
        self.seq += 1;
    }

    /// The report's lines, or what stands in for them: the load on its way
    /// or its error.
    pub fn lines(&self) -> Vec<(LineKind, String)> {
        let mut out = Vec::new();
        if let Some(e) = &self.error {
            out.push((
                LineKind::Bad,
                format!("impact failed: {e} (r loads it again)"),
            ));
        }
        if self.report.is_some() && self.rules_truncated {
            out.push((
                LineKind::Changed,
                "a ruleset or workload list was cut short: rule coverage counts are lower bounds"
                    .into(),
            ));
        }
        match &self.report {
            Some(r) => out.extend(report_lines(r, PAIRS_SHOWN)),
            None if self.loading => out.push((
                LineKind::Dim,
                "loading: the pending list, the rulesets and one draft traffic query…".into(),
            )),
            None => {}
        }
        out
    }
}
