//! `policy impact` and `policy provision --impact` (#86): what provisioning
//! the draft would change. The summary is `crate::provision_impact`'s; this
//! builds the traffic query (window, cap, the cost guard, all before any
//! request), prints the sections and gates on `--fail-on-blocked`.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{Meta, OutputFormat, TableRows, Window, join_list, write_table};
use super::traffic::{keys_or_profile, warn};
use crate::provision_impact::{
    ChangeCount, DEFAULT_PAIRS, LoadSpec, Loaded, ProvisionImpact, RulesetImpact, coverage_text,
    load, unsupported,
};
use clap::Args;
use pce::traffic::{DraftPair, TrafficQueryBuilder, TransitionCell, Transitions, parse_window};
use std::collections::BTreeSet;
use std::io::Write;
use tokio_util::sync::CancellationToken;

#[derive(Args, Debug, Clone, Default)]
pub struct ImpactArgs {
    /// The traffic query's window, ending now, e.g. 1h, 24h, 7d (default from profile, 24h;
    /// > 7d needs --yes)
    #[arg(long)]
    pub window: Option<String>,
    /// Group pairs listed per direction (newly blocked, newly allowed): default 20 in the
    /// table, all with --json and --csv
    #[arg(long)]
    pub limit: Option<usize>,
    /// Exit 7 (would_block) when any flow's draft decision is stricter than today's (allowed →
    /// potentially blocked or blocked, potentially blocked → blocked), after the report: a
    /// change-window or CI gate. Without such a flow, a result that is truncated, incomplete or
    /// has flows the draft did not re-evaluate is exit 4
    #[arg(long)]
    pub fail_on_blocked: bool,
    /// Confirm an expensive query (window > 7d)
    #[arg(long)]
    pub yes: bool,
}

/// What the summary costs, in words (`meta.cost`).
pub fn cost_text(report: &ProvisionImpact) -> &'static str {
    match (&report.traffic, report.rulesets.is_empty()) {
        (None, _) => "1 pending read (cached); nothing pending, so no other request",
        (Some(_), true) => {
            "1 pending read, the object lists (cached); 1 Explorer query re-evaluated against the draft: 1 POST, status polls, 1 PUT update_rules, polls, 1 download"
        }
        (Some(_), false) => {
            "1 pending read, the object lists, the draft and active rulesets, active label groups and IP lists, 1 workloads read (all cached); 1 Explorer query re-evaluated against the draft: 1 POST, status polls, 1 PUT update_rules, polls, 1 download"
        }
    }
}

/// The query over `window` (the profile's default when `None`) at the
/// profile's cap, the cost guard applied: all before any request.
pub fn impact_spec(
    ctx: &Context,
    window: Option<&str>,
    yes: bool,
    requested: Option<BTreeSet<String>>,
) -> Result<LoadSpec, CliError> {
    let label = window.unwrap_or(&ctx.config.traffic_default_window);
    let w = parse_window(label).map_err(|e| CliError::Args(format!("--window: {e}")))?;
    let query = TrafficQueryBuilder::last(w)?
        .max_results(ctx.config.traffic_max_results)
        .build()?;
    if !yes && let Some(reason) = query.needs_confirmation() {
        return Err(CliError::Args(format!(
            "expensive query: {reason}; policy impact --window W --yes confirms it"
        )));
    }
    Ok(LoadSpec {
        query,
        window: label.to_string(),
        keys: keys_or_profile(ctx, &[]),
        mode: ctx.cache_mode,
        requested,
    })
}

/// The summary from the PCE, as `spec` says.
pub async fn fetch(ctx: &Context, spec: &LoadSpec) -> Result<Loaded, CliError> {
    Ok(load(&ctx.pce, spec, &CancellationToken::new()).await?)
}

/// `meta` of the summary: its cost, the traffic query's window, the draft
/// counts, warnings for a cut-short read and, with `targets`, that it
/// covers the whole draft.
pub fn impact_meta(ctx: &Context, loaded: &Loaded, spec: &LoadSpec, targets: bool) -> Meta {
    let r = &loaded.report;
    let traffic_cut = r.traffic.as_ref().is_some_and(|t| t.truncated);
    let mut meta = ctx.meta(
        cache_label(loaded.cache_hit, ctx.cache_mode),
        loaded.rules_truncated || traffic_cut,
        r.traffic.as_ref().map(|_| Window {
            start: spec.query.start_date.clone(),
            end: spec.query.end_date.clone(),
        }),
    );
    meta.cost = Some(cost_text(r).into());
    if let Some(t) = &r.traffic {
        meta.cap = Some(spec.query.max_results);
        meta.incomplete = Some(t.incomplete);
        meta.draft = loaded.draft;
    }
    if targets && r.pending > 0 {
        warn(
            &mut meta,
            "the rules and traffic sections cover the whole draft: the PCE evaluates every pending change, not only those in this request (changes: in_request)".into(),
        );
    }
    if traffic_cut {
        warn(
            &mut meta,
            format!(
                "the traffic query reached its cap of {}: the counts are lower bounds (a shorter --window narrows it)",
                spec.query.max_results
            ),
        );
    }
    if loaded.rules_truncated {
        warn(
            &mut meta,
            "a ruleset or workload list was cut short: rule coverage counts are lower bounds"
                .into(),
        );
    }
    meta
}

struct Changes<'a>(&'a [ChangeCount]);

impl TableRows for Changes<'_> {
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec!["object_type", "change_type", "count"];
        if self.0.iter().any(|c| c.in_request.is_some()) {
            h.push("in_request");
        }
        h
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|c| {
                let mut row = vec![
                    c.object_type.clone(),
                    c.change_type.clone(),
                    c.count.to_string(),
                ];
                row.extend(c.in_request.map(|n| n.to_string()));
                row
            })
            .collect()
    }
}

struct Rules<'a>(&'a [RulesetImpact]);

impl TableRows for Rules<'_> {
    fn units(&self) -> Option<&'static str> {
        Some("providers, consumers: workloads matched, active → draft (- not in force)")
    }
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "ruleset",
            "change",
            "kind",
            "rule",
            "providers",
            "consumers",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for rs in self.0 {
            if rs.rules.is_empty() {
                out.push(vec![
                    rs.name.clone(),
                    rs.change.clone(),
                    String::new(),
                    "(no rule changes)".into(),
                    String::new(),
                    String::new(),
                ]);
            }
            for r in &rs.rules {
                out.push(vec![
                    rs.name.clone(),
                    r.change.as_str().to_string(),
                    super::policy::kind_text(r.kind),
                    r.rule.clone(),
                    coverage_text(r, |i| i.providers),
                    coverage_text(r, |i| i.consumers),
                ]);
            }
        }
        out
    }
    /// Under a changed rule its active text; under any rule what its
    /// coverage could not count.
    fn notes(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for rs in self.0 {
            if rs.rules.is_empty() {
                out.push(Vec::new());
            }
            for r in &rs.rules {
                let mut lines: Vec<String> = r.was.iter().map(|w| format!("  was: {w}")).collect();
                lines.extend(
                    unsupported(r)
                        .into_iter()
                        .map(|u| format!("  not counted: {u}")),
                );
                out.push(lines);
            }
        }
        out
    }
}

/// The reported → draft table: each changed cell, then `unchanged`.
struct Moves<'a>(&'a Transitions);

impl TableRows for Moves<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["reported → draft", "flows", "conns"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let mut rows: Vec<Vec<String>> = self
            .0
            .changed
            .iter()
            .map(|c| vec![c.arrow(), c.flows.to_string(), c.connections.to_string()])
            .collect();
        rows.push(vec![
            "unchanged".into(),
            self.0.unchanged.to_string(),
            String::new(),
        ]);
        rows
    }
}

/// `allowed → blocked 3 · allowed → pot.blocked 1`.
pub fn cells_text(cells: &[TransitionCell]) -> String {
    cells
        .iter()
        .map(|c| format!("{} {}", c.arrow(), c.flows))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Pair rows: one direction in the table, both in CSV (`direction`).
struct Pairs<'a> {
    rows: Vec<(&'static str, &'a DraftPair)>,
}

/// The `--csv` columns (`columns::csv_columns` too).
pub const CSV_COLUMNS: [&str; 7] = [
    "direction",
    "src",
    "dst",
    "services",
    "flows",
    "connections",
    "transitions",
];

impl TableRows for Pairs<'_> {
    fn units(&self) -> Option<&'static str> {
        Some("flows: flow records the draft moves · conns: their lifetime counts")
    }
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "src",
            "dst",
            "services",
            "flows",
            "conns",
            "reported → draft",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|(_, p)| {
                vec![
                    p.src.join("/"),
                    p.dst.join("/"),
                    p.services.join(", "),
                    p.flows.to_string(),
                    p.connections.to_string(),
                    cells_text(&p.transitions),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        CSV_COLUMNS.to_vec()
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|(dir, p)| {
                vec![
                    (*dir).to_string(),
                    p.src.join("/"),
                    p.dst.join("/"),
                    join_list(&p.services),
                    p.flows.to_string(),
                    p.connections.to_string(),
                    join_list(
                        p.transitions
                            .iter()
                            .map(|c| format!("{} {}", c.arrow(), c.flows)),
                    ),
                ]
            })
            .collect()
    }
}

/// ` · 3 not evaluated` (flow records the draft re-evaluation did not
/// reach), empty for none.
pub fn not_evaluated_text(n: u64) -> String {
    if n == 0 {
        String::new()
    } else {
        format!(" · {n} not evaluated")
    }
}

/// `N of M pairs` when `--limit` (or the default) cut the list.
fn shown_of(shown: usize, total: usize) -> String {
    if shown < total {
        format!("top {shown} of {total} pairs")
    } else {
        format!("{total} pair{}", if total == 1 { "" } else { "s" })
    }
}

/// The table output: each section under its title, then what is not
/// counted. Nothing pending is one line. Pairs: the first `limit` of
/// each direction.
pub fn write_tables(
    out: &mut impl Write,
    r: &ProvisionImpact,
    limit: usize,
) -> Result<(), CliError> {
    if r.pending == 0 {
        writeln!(out, "nothing pending: provisioning would change nothing")?;
        return Ok(());
    }
    writeln!(
        out,
        "changes · {} pending change{}",
        r.pending,
        if r.pending == 1 { "" } else { "s" }
    )?;
    write_table(out, &Changes(&r.changes))?;
    writeln!(out)?;
    let n: usize = r.rulesets.iter().map(|s| s.rules.len()).sum();
    if r.rulesets.is_empty() {
        writeln!(out, "rules · no pending ruleset")?;
    } else {
        let (a, x, c) = r.rulesets.iter().fold((0, 0, 0), |(a, x, c), s| {
            (a + s.added, x + s.removed, c + s.changed)
        });
        writeln!(
            out,
            "rules · {} pending ruleset{} · {a} added · {x} removed · {c} changed",
            r.rulesets.len(),
            if r.rulesets.len() == 1 { "" } else { "s" }
        )?;
        if n > 0 || r.rulesets.iter().any(|s| s.rules.is_empty()) {
            write_table(out, &Rules(&r.rulesets))?;
        }
    }
    if let Some(t) = &r.traffic {
        writeln!(out)?;
        let x = &t.transitions;
        writeln!(
            out,
            "traffic · {} · {} flow records · {} newly blocked · {} newly allowed{}{}",
            t.window,
            t.flows,
            x.newly_blocked,
            x.newly_allowed,
            not_evaluated_text(x.not_evaluated),
            if t.truncated { " · truncated" } else { "" }
        )?;
        write_table(out, &Moves(x))?;
        for (title, dir, pairs, total) in [
            (
                "newly blocked or potentially blocked",
                "newly_blocked",
                &t.blocked_pairs,
                t.blocked_pairs_total,
            ),
            (
                "newly allowed",
                "newly_allowed",
                &t.allowed_pairs,
                t.allowed_pairs_total,
            ),
        ] {
            if total == 0 {
                continue;
            }
            let shown = pairs.len().min(limit);
            writeln!(out)?;
            writeln!(out, "{title} · {}", shown_of(shown, total))?;
            let rows = pairs.iter().take(shown).map(|p| (dir, p)).collect();
            write_table(out, &Pairs { rows })?;
        }
    }
    if !r.not_counted.is_empty() {
        writeln!(out)?;
        writeln!(out, "not counted:")?;
        for n in &r.not_counted {
            writeln!(out, "  {n}")?;
        }
    }
    Ok(())
}

/// `policy impact`: the query checked, the summary loaded, printed; then
/// `--fail-on-blocked`.
pub async fn run(ctx: &Context, a: &ImpactArgs) -> Result<(), CliError> {
    let spec = impact_spec(ctx, a.window.as_deref(), a.yes, None)?;
    let loaded = fetch(ctx, &spec).await?;
    let meta = impact_meta(ctx, &loaded, &spec, false);
    let mut report = loaded.report;
    if ctx.format != OutputFormat::Table
        && let Some(n) = a.limit
    {
        report.limit_pairs(n);
    }
    match ctx.format {
        OutputFormat::Json => {
            super::output::write_json(&mut std::io::stdout().lock(), &report, &meta)?
        }
        OutputFormat::Table => {
            write_tables(
                &mut std::io::stdout().lock(),
                &report,
                a.limit.unwrap_or(DEFAULT_PAIRS),
            )?;
            eprintln!("{}", super::output::meta_line(&meta));
        }
        OutputFormat::Csv => {
            let t = report.traffic.as_ref();
            let rows = t
                .map(|t| {
                    t.blocked_pairs
                        .iter()
                        .map(|p| ("newly_blocked", p))
                        .chain(t.allowed_pairs.iter().map(|p| ("newly_allowed", p)))
                        .collect()
                })
                .unwrap_or_default();
            super::output::write_table_csv(&mut std::io::stdout().lock(), &Pairs { rows })?;
            eprintln!("{}", super::output::meta_line(&meta));
        }
    }
    if a.fail_on_blocked {
        let n = report.newly_blocked();
        if n > 0 {
            return Err(CliError::WouldBlock(format!(
                "{n} flow record{} would become blocked or potentially blocked once provisioned",
                if n == 1 { "" } else { "s" }
            )));
        }
        if let Some(t) = report.traffic.as_ref() {
            let why = if t.transitions.not_evaluated > 0 {
                Some(format!(
                    "{} of {} flow records were not re-evaluated against the draft",
                    t.transitions.not_evaluated, t.flows
                ))
            } else if t.truncated {
                Some("truncated (cap reached; a shorter --window narrows it)".to_string())
            } else if t.incomplete {
                Some("incomplete (a region did not answer)".to_string())
            } else {
                None
            };
            if let Some(why) = why {
                return Err(CliError::Incomplete(format!(
                    "no flow would become blocked, but the traffic result is not complete: {why}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ruleset with no rule change: the change column holds its change
    /// type only, the note sits in the rule column.
    #[test]
    fn a_ruleset_without_rule_changes_keeps_the_change_column_narrow() {
        let rs = RulesetImpact {
            name: "Shop-Prod".into(),
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            change: "create".into(),
            added: 0,
            removed: 0,
            changed: 0,
            deny_compared: true,
            rules: Vec::new(),
        };
        let t = Rules(std::slice::from_ref(&rs));
        assert_eq!(
            t.rows(),
            [vec![
                "Shop-Prod".to_string(),
                "create".into(),
                String::new(),
                "(no rule changes)".into(),
                String::new(),
                String::new(),
            ]]
        );
        assert_eq!(t.notes(), [Vec::<String>::new()]);
    }

    #[test]
    fn not_evaluated_shows_only_when_some_are() {
        assert_eq!(not_evaluated_text(0), "");
        assert_eq!(not_evaluated_text(3), " · 3 not evaluated");
    }
}
