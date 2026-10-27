//! The policy generator's state (`G` / `:gen`): the app's rows with their
//! checkboxes and stops, the slider, the section and cursor, and the
//! proposal they lead to. Pure: `update` fills it, `ui::widgets::generator`
//! draws it. Nothing here makes a request.
use pce::NameIndex;
use pce::generate::{
    Candidate, Consolidate, GenRow, PeerKind, Proposal, RowKey, Side, Stop, Target, candidates,
};
use pce::traffic::Focus;
use pce::write::PortSpec;
use std::collections::{BTreeMap, BTreeSet};

/// The draft rulesets the proposal is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drafts {
    Loading,
    Loaded,
    /// The load failed; `r` retries.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct GenView {
    /// The app, `in` (or `both` with the outbound section on).
    pub focus: Focus,
    /// The traffic window, as the filter names it (`24h`).
    pub window: String,
    pub seq: u64,
    pub rows: Vec<GenRow>,
    /// The slider.
    pub global: Stop,
    pub section: Side,
    /// One cursor per section (inbound, outbound), into its rows.
    pub cursor: [usize; 2],
    /// `o`: the outbound section is shown and proposed.
    pub outbound: bool,
    /// The app ruleset `R` picked among several (href).
    pub picked: Option<String>,
    /// Requests this generator sent (the draft rulesets).
    pub requests: u32,
    /// Of those, the ones answered from the cache.
    pub cached: u32,
    /// The draft ruleset load was sent by this generator.
    pub drafts_sent: bool,
    pub drafts: Drafts,
    /// The rules the rows lead to; `None` until the draft rulesets are in.
    pub proposal: Option<Proposal>,
    /// Why the data may miss flows (the header's text); `None`: complete.
    pub gap: Option<String>,
    /// `r` can close the gap (a focused query); not when the focused
    /// result itself is capped.
    pub requery: bool,
    /// The PCE has an All Services object (the `·all` stops work).
    pub all_services: bool,
    /// `p`: the rule list is shown, scrolled to this line.
    pub list: Option<u16>,
    /// `Enter` sent a plan; its answer is on the way.
    pub planning: bool,
    /// The last plan's error (nothing was sent).
    pub error: Option<String>,
    /// The proposal `Enter` planned: a plan landing after it changed
    /// (keys are frozen, but traffic may land) is discarded.
    pub planned: Option<Proposal>,
    /// The draft rulesets a plan read fresh: the proposal is derived from
    /// them until a newer load arrives.
    pub fresh_drafts: Option<Vec<pce::models::RuleSet>>,
    /// `generator.consolidate_min_peers`.
    pub min_peers: usize,
    /// The inbound fan-in ports, from the rows (#39): one line each above
    /// the inbound rows, where the cursor goes first.
    pub candidates: Vec<Candidate>,
    /// `c`: each port's consolidation, by port and protocol; kept across
    /// rebuilds (a port that is no candidate now is not applied).
    pub consolidate: BTreeMap<PortSpec, Consolidate>,
    /// Why the Any option is unavailable (no built-in Any IP list).
    pub any_missing: Option<String>,
    /// The app's flows a squelch mark hides elsewhere: the rules include
    /// them (marks are not policy), the header says so.
    pub squelched: u64,
    /// `N`: "new since" (#81), the rows and rules from the new traffic
    /// only; `None` off.
    pub since: Option<GenSince>,
}

/// `N`'s state: the baseline, the same focused query over the window
/// before the result's, and what it leaves of the recent records.
#[derive(Debug, Clone)]
pub struct GenSince {
    /// The baseline's length as typed (`7d`).
    pub length: String,
    /// The recent result's filter it is for (any cap): another one ends it.
    pub filter: crate::tui::filter::TrafficFilter,
    /// The sides it was queried for: `o` turned on later ends it.
    pub direction: pce::traffic::FocusDirection,
    /// The baseline query's sequence number (the traffic screen's: one
    /// query at a time).
    pub seq: u64,
    /// `None` while the baseline query runs.
    pub baseline: Option<GenBaseline>,
    /// The recent records judged against it (`rebuild_gen`).
    pub result: Option<pce::generate::Since>,
}

/// The baseline's answer.
#[derive(Debug, Clone)]
pub struct GenBaseline {
    pub flows: Vec<pce::traffic::ResolvedFlow>,
    pub truncated: bool,
    pub incomplete: bool,
}

impl GenSince {
    /// Whether `f` is the filter it is for (any cap).
    pub fn same_filter(&self, f: &crate::tui::filter::TrafficFilter) -> bool {
        crate::tui::filter::TrafficFilter {
            max_results: self.filter.max_results,
            ..f.clone()
        } == self.filter
    }

    /// The header's part: `new since 7d before · 40 flows seen before`,
    /// `… · baseline running`, and a truncated or incomplete baseline.
    pub fn text(&self) -> String {
        let head = format!("new since {} before", self.length);
        let Some(b) = &self.baseline else {
            return format!("{head} · baseline running");
        };
        let seen = self.result.as_ref().map_or(0, |r| r.seen_before);
        let mut s = format!("{head} · {seen} flows seen before");
        if b.truncated {
            s.push_str(" · baseline truncated: new over-reports");
        }
        if b.incomplete {
            s.push_str(" · baseline incomplete: new over-reports");
        }
        s
    }
}

/// What the fresh read at `Enter` changed against the preview, for the
/// confirm's status line; `None` when nothing did.
pub fn refresh_note(old: &Proposal, new: &Proposal) -> Option<String> {
    use pce::generate::RuleAction;
    let mut notes = Vec::new();
    for (i, (o, n)) in old.targets.iter().zip(&new.targets).enumerate() {
        if o != n {
            let was = match o {
                Target::Create(d) => format!("new {}", d.name),
                t => t.name(),
            };
            let scope = new.scopes.get(i).map_or_else(String::new, |s| s.text());
            notes.push(format!(
                "target of {scope} changed: {} (was {was})",
                n.name()
            ));
        }
    }
    let moved = |f: fn(&RuleAction) -> bool| {
        old.rules
            .iter()
            .zip(&new.rules)
            .filter(|(o, n)| !f(&o.action) && f(&n.action))
            .count()
    };
    let exists = moved(|a| matches!(a, RuleAction::Exists { .. }));
    if exists > 0 {
        notes.push(format!(
            "{exists} {} in the draft since the preview: not written",
            if exists == 1 { "rule is" } else { "rules are" }
        ));
    }
    let gone = old
        .rules
        .iter()
        .zip(&new.rules)
        .filter(|(o, n)| {
            matches!(o.action, RuleAction::Exists { .. })
                && !matches!(n.action, RuleAction::Exists { .. })
        })
        .count();
    if gone > 0 {
        notes.push(format!(
            "{gone} {} previewed as existing no longer in the draft: written now",
            if gone == 1 { "rule" } else { "rules" }
        ));
    }
    let adds = moved(|a| matches!(a, RuleAction::AddServices { .. }));
    if adds > 0 {
        notes.push(format!("{adds} now only add services to an existing rule"));
    }
    let denied = |p: &Proposal| p.rules.iter().map(|r| r.denied_flows).sum::<u64>();
    if denied(new) > denied(old) {
        notes.push(format!("override denies now block {} flows", denied(new)));
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// The IP peers behind an IP or IP list row.
fn row_ips(r: &GenRow) -> Vec<String> {
    match &r.peer {
        PeerKind::Ip { ip } => vec![ip.clone()],
        PeerKind::IpList { ips, .. } => ips.clone(),
        _ => Vec::new(),
    }
}

fn side_index(s: Side) -> usize {
    match s {
        Side::Inbound => 0,
        Side::Outbound => 1,
    }
}

impl GenView {
    pub fn new(focus: Focus, window: String, seq: u64) -> Self {
        Self {
            focus,
            window,
            seq,
            rows: Vec::new(),
            global: Stop::default(),
            section: Side::Inbound,
            cursor: [0; 2],
            outbound: false,
            picked: None,
            requests: 0,
            cached: 0,
            drafts_sent: false,
            drafts: Drafts::Loading,
            proposal: None,
            gap: None,
            requery: false,
            all_services: true,
            list: None,
            planning: false,
            error: None,
            planned: None,
            fresh_drafts: None,
            min_peers: pce::generate::consolidate::DEFAULT_MIN_PEERS,
            candidates: Vec::new(),
            consolidate: BTreeMap::new(),
            any_missing: None,
            squelched: 0,
            since: None,
        }
    }

    /// New rows; checkboxes, overrides and the cursor follow their row by
    /// key (the rows may come in another order after a re-query).
    pub fn set_rows(&mut self, rows: Vec<GenRow>) {
        let at = self.selected().map(|i| self.rows[i].key());
        let at_fan = self
            .selected_candidate()
            .map(|c| self.candidates[c].spec.clone());
        let old: BTreeMap<RowKey, (bool, Option<Stop>)> = self
            .rows
            .iter()
            .map(|r| (r.key(), (r.checked, r.stop)))
            .collect();
        // The IPs of checked IP rows that are gone (regrouped: the drafts
        // arrived, `R` picked another ruleset): their new rows stay checked.
        let moved: BTreeSet<(Side, String)> = self
            .rows
            .iter()
            .filter(|r| r.checked && !rows.iter().any(|n| n.key() == r.key()))
            .flat_map(|r| row_ips(r).into_iter().map(move |ip| (r.side, ip)))
            .collect();
        self.rows = rows;
        self.candidates = candidates(&self.rows, self.min_peers);
        // A port that is no candidate now forgets its choice: it never
        // comes back by itself.
        let fans: BTreeSet<&PortSpec> = self.candidates.iter().map(|c| &c.spec).collect();
        self.consolidate.retain(|spec, _| fans.contains(spec));
        for r in &mut self.rows {
            if let Some((checked, stop)) = old.get(&r.key()) {
                r.checked = *checked && r.checkable;
                r.stop = *stop;
            }
            if row_ips(r)
                .into_iter()
                .any(|ip| moved.contains(&(r.side, ip)))
            {
                r.checked = r.checkable;
            }
        }
        for s in [Side::Inbound, Side::Outbound] {
            let n = self.section_len(s);
            let c = &mut self.cursor[side_index(s)];
            *c = (*c).min(n.saturating_sub(1));
        }
        let section = self.section;
        if let Some(key) = at {
            if let Some(pos) = self
                .section_rows(section)
                .iter()
                .position(|i| self.rows[*i].key() == key)
            {
                self.cursor[side_index(section)] = self.lead(section) + pos;
            }
        } else if let Some(spec) = at_fan
            && let Some(pos) = self.candidates.iter().position(|c| c.spec == spec)
        {
            self.cursor[0] = pos;
        }
    }

    /// The fan-in lines above a section's rows (inbound only).
    pub fn lead(&self, s: Side) -> usize {
        match s {
            Side::Inbound => self.candidates.len(),
            Side::Outbound => 0,
        }
    }

    /// The cursor's stops in a section: its fan-in lines, then its rows.
    pub fn section_len(&self, s: Side) -> usize {
        self.lead(s) + self.section_rows(s).len()
    }

    /// The fan-in line under the cursor (an index into `candidates`).
    pub fn selected_candidate(&self) -> Option<usize> {
        let c = self.cursor[side_index(self.section)];
        (c < self.lead(self.section)).then_some(c)
    }

    /// Each candidate port's consolidation in force (`Off` left out).
    pub fn consolidations(&self) -> BTreeMap<PortSpec, Consolidate> {
        self.candidates
            .iter()
            .filter_map(|c| {
                let m = self.consolidate.get(&c.spec).copied().unwrap_or_default();
                (m != Consolidate::Off).then(|| (c.spec.clone(), m))
            })
            .collect()
    }

    /// The consolidation of a candidate port.
    pub fn mode_of(&self, spec: &PortSpec) -> Consolidate {
        self.consolidate.get(spec).copied().unwrap_or_default()
    }

    /// `c` on a fan-in line: off → Any → All Workloads → off for that port.
    /// `any_missing`: why Any is unavailable (then it is skipped). The
    /// text says what the port does now; the error why nothing changed.
    pub fn cycle_consolidation(&mut self, any_missing: Option<&str>) -> Result<String, String> {
        let Some(i) = self.selected_candidate() else {
            return Err(if self.candidates.is_empty() {
                "no fan-in port: c has nothing to consolidate".into()
            } else {
                "c works on a fan-in line (⚠ above the inbound rows)".into()
            });
        };
        let spec = self.candidates[i].spec.clone();
        let mut next = self.mode_of(&spec).next();
        let mut skipped = None;
        if next == Consolidate::Any
            && let Some(why) = any_missing
        {
            skipped = Some(why);
            next = next.next();
        }
        self.consolidate.insert(spec.clone(), next);
        let app = self.focus.app_text();
        let port = spec.display();
        let what = match next {
            Consolidate::Off => format!("per-peer rules again for {port}"),
            Consolidate::Any => format!(
                "destination only: {port} from any IP → {app} in one rule; checked workload peers keep theirs"
            ),
            Consolidate::AllWorkloads => {
                format!("every VEN recomputes: {port} from All Workloads → {app} in one rule")
            }
        };
        let skipped = skipped.map_or_else(String::new, |w| format!(" (Any unavailable: {w})"));
        Ok(format!("{what}{skipped}"))
    }

    /// Whether the proposal holds a consolidated rule (`Enter` plans it
    /// with no row checked).
    pub fn has_consolidated_rule(&self) -> bool {
        self.proposal
            .as_ref()
            .is_some_and(|p| p.rules.iter().any(|r| r.consolidated.is_some()))
    }

    /// Row `i`'s flag when a consolidated rule takes some of its ports:
    /// `consolidated` (all of them) or `5432/tcp consolidated`.
    pub fn consolidated_flag(&self, i: usize) -> Option<String> {
        let p = self.proposal.as_ref()?;
        let taken: Vec<String> = p
            .rules
            .iter()
            .filter(|r| r.rows.contains(&i))
            .filter_map(|r| r.consolidated.as_ref())
            .map(|c| c.spec.display())
            .collect();
        if taken.is_empty() {
            return None;
        }
        Some(if taken.len() == self.rows[i].services.len() {
            "consolidated".to_string()
        } else {
            format!("{} consolidated", taken.join(" "))
        })
    }

    /// The indexes (into `rows`) of one section's rows.
    pub fn section_rows(&self, s: Side) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|i| self.rows[*i].side == s)
            .collect()
    }

    /// The row under the cursor (an index into `rows`).
    pub fn selected(&self) -> Option<usize> {
        let c = self.cursor[side_index(self.section)];
        let lead = self.lead(self.section);
        c.checked_sub(lead)
            .and_then(|c| self.section_rows(self.section).get(c).copied())
    }

    /// `j` / `k`, clamped to the section.
    /// Past the last inbound row `j` goes on into the outbound section
    /// (when it is shown and has rows), `k` from its first row back.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.section_len(self.section);
        let at = self.cursor[side_index(self.section)];
        let outbound_rows = self.section_rows(Side::Outbound).len();
        if delta > 0 && self.section == Side::Inbound && at + 1 >= n {
            if self.outbound && outbound_rows > 0 {
                self.section = Side::Outbound;
                self.cursor[1] = 0;
            }
            return;
        }
        if delta < 0 && self.section == Side::Outbound && at == 0 {
            self.section = Side::Inbound;
            let n = self.section_len(Side::Inbound);
            self.cursor[0] = n.saturating_sub(1);
            return;
        }
        let c = &mut self.cursor[side_index(self.section)];
        *c = c.saturating_add_signed(delta).min(n.saturating_sub(1));
    }

    /// `Tab`: the other section, when it is shown.
    pub fn switch_section(&mut self) {
        self.section = match self.section {
            Side::Inbound if self.outbound => Side::Outbound,
            _ => Side::Inbound,
        };
    }

    /// `o`: the outbound section on or off; the focus follows (`both`
    /// needs the out side's flows too).
    pub fn toggle_outbound(&mut self) {
        self.outbound = !self.outbound;
        self.focus.direction = if self.outbound {
            pce::traffic::FocusDirection::Both
        } else {
            pce::traffic::FocusDirection::In
        };
        if !self.outbound {
            self.section = Side::Inbound;
        }
    }

    /// `Space`; the error says why a row cannot be checked.
    pub fn toggle(&mut self, scope_keys: &[String]) -> Result<(), String> {
        if self.selected_candidate().is_some() {
            return Err("Space checks rows: c consolidates this port".into());
        }
        let i = self.selected().ok_or("no row here")?;
        let r = &mut self.rows[i];
        if !r.checkable {
            let why = r
                .flag_texts(r.effective(self.global), scope_keys)
                .into_iter()
                .next()
                .unwrap_or_else(|| "this row is not proposed".into());
            return Err(format!("{} cannot be checked: {why}", r.peer_text()));
        }
        r.checked = !r.checked;
        Ok(())
    }

    /// The next stop from `from` in `dir` (-1, +1), skipping the `·all`
    /// stops without an All Services object; `None` at the end.
    fn step(&self, from: Stop, dir: isize) -> Option<Stop> {
        let mut s = from;
        loop {
            let next = if dir < 0 { s.prev() } else { s.next() };
            if next == s {
                return None;
            }
            if self.all_services || !next.is_all() {
                return Some(next);
            }
            s = next;
        }
    }

    /// `←` / `→`: the slider.
    pub fn slide(&mut self, dir: isize) {
        if let Some(s) = self.step(self.global, dir) {
            self.global = s;
        }
    }

    /// `<` / `>`: the selected row's own stop, from the one in force.
    pub fn slide_row(&mut self, dir: isize) {
        let Some(i) = self.selected() else {
            return;
        };
        let from = self.rows[i].effective(self.global);
        if let Some(s) = self.step(from, dir) {
            self.rows[i].stop = Some(s);
        }
    }

    /// `=`: the selected row follows the slider again.
    pub fn clear_row_stop(&mut self) {
        if let Some(i) = self.selected() {
            self.rows[i].stop = None;
        }
    }

    /// `R`: the next of several rulesets covering the app.
    pub fn pick_next(&mut self) -> Result<String, String> {
        let Some(Target::Ambiguous(c)) = self.proposal.as_ref().and_then(|p| p.targets.first())
        else {
            return Err("one ruleset or none covers the app: nothing to pick".into());
        };
        let at = self
            .picked
            .as_ref()
            .and_then(|h| c.iter().position(|x| &x.href == h));
        let next = &c[at.map_or(0, |i| (i + 1) % c.len())];
        self.picked = Some(next.href.clone());
        Ok(next.name.clone())
    }

    /// The rows of the sections in force.
    fn in_force(&self) -> impl Iterator<Item = &GenRow> {
        self.rows
            .iter()
            .filter(|r| r.side == Side::Inbound || self.outbound)
    }

    /// `Generate · shop/prod · last 24h · 1310 flows · complete`.
    pub fn header_title(&self) -> String {
        let flows: u64 = self.in_force().map(|r| r.flows).sum();
        let squelched = if self.squelched > 0 {
            format!(" · {} squelched (rules include them)", self.squelched)
        } else {
            String::new()
        };
        let since = self
            .since
            .as_ref()
            .map_or_else(String::new, |s| format!(" · {}", s.text()));
        format!(
            "Generate · {} · last {} · {flows} flows · {}{since}{squelched}",
            self.focus.app_text(),
            self.window,
            if self.gap.is_some() {
                "incomplete"
            } else {
                "complete"
            }
        )
    }

    /// The line under the target in "new since" once its baseline is in:
    /// `new since 7d before · 40 flows seen before · N: every flow`.
    pub fn since_line(&self) -> Option<String> {
        let s = self.since.as_ref().filter(|s| s.baseline.is_some())?;
        Some(format!("{} · N: every flow", s.text()))
    }

    /// `1 req · cached`, `0 req · cached`.
    pub fn header_meta(&self) -> String {
        let mut s = format!("{} req", self.requests);
        if self.requests == 0 || self.cached >= self.requests {
            s.push_str(" · cached");
        }
        s
    }

    /// The slider line: the stop in force in brackets.
    pub fn slider_line(&self) -> String {
        let stops: Vec<String> = Stop::ALL
            .iter()
            .map(|s| {
                let t = if !self.all_services && s.is_all() {
                    format!("({})", s.text())
                } else {
                    s.text().to_string()
                };
                if *s == self.global {
                    format!("[{t}]")
                } else {
                    t
                }
            })
            .collect();
        format!("granularity  {}", stops.join(" ─ "))
    }

    /// `target  Shop-Prod (draft, 2 rules)`, `target  new ruleset
    /// RF-shop-prod, scoped app=shop, env=prod`, `3 rulesets cover
    /// shop/prod: R picks`, `loading rulesets`.
    pub fn target_line(&self, drafts: Option<&[pce::models::RuleSet]>, idx: &NameIndex) -> String {
        let t = match (&self.drafts, &self.proposal) {
            (Drafts::Failed(e), _) => format!("rulesets: {e} · r retries"),
            (_, None) => "loading rulesets".into(),
            (_, Some(p)) => match p.targets.first() {
                Some(Target::Existing { href, name }) => {
                    let rules = drafts
                        .and_then(|d| d.iter().find(|r| &r.href == href))
                        .map_or(0, |r| r.rules.len());
                    format!("{name} (draft, {rules} rules)")
                }
                Some(Target::Create(d)) => {
                    let scope: Vec<String> = d
                        .scopes
                        .iter()
                        .flatten()
                        .map(|a| idx.label_text(a.href.as_deref().unwrap_or_default()))
                        .collect();
                    format!("new ruleset {}, scoped {}", d.name, scope.join(", "))
                }
                Some(Target::Ambiguous(c)) => {
                    format!(
                        "{} rulesets cover {}: R picks",
                        c.len(),
                        self.focus.app_text()
                    )
                }
                Some(Target::None(why)) => why.clone(),
                None => "no target".into(),
            },
        };
        format!("target  {t}")
    }

    /// The name a section's title gives the app's ruleset.
    pub fn target_name(&self) -> String {
        self.proposal
            .as_ref()
            .and_then(|p| p.targets.first())
            .map_or_else(
                || self.focus.app_text(),
                |t| match t {
                    Target::None(_) => "(no target)".to_string(),
                    t => t.name(),
                },
            )
    }

    /// The flags a row's rules earn (row `i`, an index into `rows`): an
    /// override deny blocks some of their flows anyway, or their ruleset
    /// cannot be named (ambiguous or none). Empty before the proposal.
    pub fn rule_flags(&self, i: usize) -> Vec<String> {
        let Some(p) = &self.proposal else {
            return Vec::new();
        };
        let mine = || p.rules.iter().filter(|r| r.rows.contains(&i));
        let mut out = Vec::new();
        let denied: u64 = mine().map(|r| r.denied_flows.min(r.flows)).sum();
        if denied > 0 {
            out.push(format!("override deny: {denied}"));
        }
        let no_target = mine().any(|r| {
            matches!(
                p.targets.get(r.target),
                Some(Target::Ambiguous(_) | Target::None(_)) | None
            )
        });
        if no_target {
            out.push("no target".to_string());
        }
        out
    }

    /// A fan-in line: `5666/tcp from 9 peers (monitoring agent?) · c
    /// consolidates: off`, and once on what its rule does.
    pub fn candidate_line(&self, i: usize) -> String {
        let c = &self.candidates[i];
        let mode = self.mode_of(&c.spec);
        let mut s = format!("⚠ {} · c consolidates: {}", c.text(), mode.text());
        if mode == Consolidate::Off {
            return s;
        }
        match self.fan_rule(&c.spec) {
            Some(r) => match &r.action {
                pce::generate::RuleAction::Exists { rule_href } => {
                    s.push_str(&format!(" · already allowed by {rule_href}"));
                }
                pce::generate::RuleAction::AddServices { rule_href, .. } => {
                    s.push_str(&format!(" · adds the port to {rule_href}"));
                }
                pce::generate::RuleAction::Create => s.push_str(" · 1 rule"),
            },
            None if self.proposal.is_some() => {
                s.push_str(&format!(" · no rule: {}", self.no_rule_reason(mode)));
            }
            None => {}
        }
        s
    }

    /// Why a port switched on yields no rule.
    fn no_rule_reason(&self, mode: Consolidate) -> String {
        match (mode, &self.any_missing) {
            (Consolidate::Any, Some(why)) => format!("Any unavailable: {why}"),
            (Consolidate::Any, None) => {
                "every peer is a checked workload peer and keeps its own rule".into()
            }
            _ => "All Workloads covers none of these peers (IP only)".into(),
        }
    }

    /// The lines under a consolidated fan-in port: its catch first
    /// (outbound rules, every VEN), then where it opens the port and what
    /// it also allows.
    pub fn candidate_detail(&self, i: usize) -> Vec<String> {
        let Some(c) = self
            .candidates
            .get(i)
            .and_then(|c| self.fan_rule(&c.spec))
            .and_then(|r| r.consolidated.as_ref())
        else {
            return Vec::new();
        };
        let mut out: Vec<String> = c.warnings().iter().map(|w| format!("  ⚠ {w}")).collect();
        out.push(format!(
            "  → {} · also allows: {}",
            c.to,
            c.also_allows().join(", ")
        ));
        out
    }

    /// The consolidated rule of a port, in the proposal.
    fn fan_rule(&self, spec: &PortSpec) -> Option<&pce::generate::GenRule> {
        self.proposal
            .as_ref()?
            .rules
            .iter()
            .find(|r| r.consolidated.as_ref().is_some_and(|c| &c.spec == spec))
    }

    /// An outbound row's target: the peer's ruleset (or the app's for an IP).
    pub fn row_target(&self, row: &GenRow) -> Option<String> {
        let p = self.proposal.as_ref()?;
        let PeerKind::App(values) = &row.peer else {
            return (row.side == Side::Outbound).then(|| self.target_name());
        };
        if row.side != Side::Outbound {
            return None;
        }
        let i = p.scopes.iter().position(|s| &s.values == values)?;
        Some(p.targets.get(i)?.name())
    }

    /// The footer: what the proposal holds and covers, or the plan's error.
    pub fn footer(&self) -> String {
        if let Some(e) = &self.error {
            return format!("plan failed, nothing sent: {e}");
        }
        if self.planning {
            return "planning: reading the drafts fresh…".into();
        }
        let flows: u64 = self.in_force().map(|r| r.flows).sum();
        // New since, its baseline in: how many were left out.
        let seen = self
            .since
            .as_ref()
            .filter(|s| s.baseline.is_some())
            .map(|s| (s, s.result.as_ref().map_or(0, |r| r.seen_before)));
        if flows == 0 {
            return match seen {
                Some((s, n)) => pce::generate::since_text(&s.length, 0, n),
                None => "no flows in the window: nothing to propose".into(),
            };
        }
        let Some(p) = &self.proposal else {
            return "rules follow when the draft rulesets arrive".into();
        };
        let c = p.counts();
        format!(
            "{} rules ({} new · {} add services · {} exist) · covers {} of {} {}flows · {} not{}",
            c.rules,
            c.create,
            c.add_services,
            c.exists,
            p.coverage.covered,
            p.coverage.covered + p.coverage.uncovered,
            if seen.is_some() { "new " } else { "" },
            p.coverage.uncovered,
            seen.map_or_else(String::new, |(_, n)| format!(" · {n} seen before"))
        )
    }

    /// `p`: one block per write, as the confirm will show them, then what
    /// is not covered and why.
    pub fn rule_list(&self, idx: &NameIndex) -> Vec<String> {
        use pce::generate::RuleAction;
        let Some(p) = &self.proposal else {
            return vec!["the draft rulesets are not loaded yet".into()];
        };
        let mut out = Vec::new();
        for d in &p.ip_lists {
            let ranges: Vec<String> = d.ranges.iter().map(|r| r.display()).collect();
            out.push(format!("new ip_list {} ({})", d.name, ranges.join(", ")));
        }
        for t in &p.targets {
            if let Target::Create(d) = t {
                out.push(format!("new rule_set {}", d.name));
            }
        }
        for (i, r) in p.rules.iter().enumerate() {
            let side = |a: &[pce::generate::GenActor]| {
                a.iter()
                    .map(|x| x.text(p, idx))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let services: Vec<String> = r
                .services
                .iter()
                .map(|s| match s {
                    pce::write::ServiceRef::Href(h)
                        if Some(h.as_str()) == idx.all_services_href() =>
                    {
                        "all services".to_string()
                    }
                    pce::write::ServiceRef::Href(h) => idx.service_name(h).unwrap_or(h).to_string(),
                    pce::write::ServiceRef::Port(x) => x.display(),
                })
                .collect();
            let what = match &r.action {
                RuleAction::Create => "create".to_string(),
                RuleAction::AddServices { rule_href, add } => format!(
                    "add {} to {rule_href}",
                    add.iter()
                        .map(|s| s.display())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                RuleAction::Exists { rule_href } => format!("exists: {rule_href}"),
            };
            out.push(format!(
                "{}. {what} · {}: {} → {} on {}{}",
                i + 1,
                p.targets[r.target].name(),
                side(&r.consumers),
                side(&r.providers),
                services.join(", "),
                if r.unscoped_consumers {
                    " · unscoped"
                } else {
                    ""
                }
            ));
            // An existing rule keeps its own description: nothing is written.
            if !matches!(r.action, RuleAction::Exists { .. }) {
                out.push(format!("   {}", r.description));
            }
            for (list, ips) in &r.ips {
                out.push(format!("   {}", pce::generate::seen_text(list, ips)));
            }
            if let Some(c) = &r.consolidated {
                // The warnings first, then what it consolidates.
                let notes = c.notes();
                let last = notes.len().saturating_sub(1);
                for (n, note) in notes.into_iter().enumerate() {
                    out.push(format!("   {}{note}", if n == last { "" } else { "⚠ " }));
                }
            }
            if r.denied_flows > 0 {
                out.push(format!(
                    "   ⚠ override deny {}: {} of its {} flows stay blocked",
                    r.denied_by.join(", "),
                    r.denied_flows.min(r.flows),
                    r.flows
                ));
            }
        }
        let c = &p.coverage;
        out.push(format!(
            "not covered: {} unchecked · {} folded IPs · {} unsupported proto · {} broadcast/multicast · {} no target · {} override deny",
            c.unchecked,
            c.folded_ips,
            c.unsupported_proto,
            c.broadcast_multicast,
            c.no_target,
            c.override_deny
        ));
        if let Some(s) = self.since.as_ref().filter(|s| s.baseline.is_some()) {
            out.push(format!(
                "left out: {} flows seen before (baseline {})",
                c.seen_before, s.length
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_flows_rich;
    use pce::traffic::{FocusDirection, parse_app};

    fn shop() -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
            .with_direction(FocusDirection::In)
    }

    fn view() -> GenView {
        let idx = crate::tui::test_support::fixture_index();
        let mut v = GenView::new(shop(), "24h".into(), 1);
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
            None,
        ));
        v
    }

    #[test]
    fn checkboxes_and_overrides_survive_new_rows() {
        let mut v = view();
        v.move_cursor(2); // shop/dev
        v.toggle(&shop().keys).unwrap();
        v.slide_row(1);
        assert_eq!(v.rows[2].stop, Some(Stop::RolePorts));
        let idx = crate::tui::test_support::fixture_index();
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
            None,
        ));
        assert!(v.rows[2].checked, "kept by its key");
        assert_eq!(v.rows[2].stop, Some(Stop::RolePorts));
        v.clear_row_stop();
        assert_eq!(v.rows[2].stop, None);
    }

    #[test]
    fn a_list_row_keeps_its_checkbox_override_and_cursor_as_its_ips_change() {
        let mut v = view();
        v.move_cursor(3);
        assert_eq!(v.rows[3].peer_text(), "IP list Corp · 1 IP");
        assert!(!v.rows[3].checked, "IP rows start unchecked");
        v.toggle(&shop().keys).unwrap();
        v.slide_row(1);
        // A second Corp address arrives: the same row (same list href).
        let mut flows = fixture_flows_rich();
        let mut more = flows[17].clone();
        more.src.ip = Some("10.1.4.10".into());
        flows.insert(0, more);
        let idx = crate::tui::test_support::fixture_index();
        v.set_rows(pce::generate::rows(&flows, &shop(), "role", &idx, None));
        let i = v.selected().unwrap();
        assert_eq!(v.rows[i].peer_text(), "IP list Corp · 2 IPs");
        assert!(v.rows[i].checked);
        assert_eq!(v.rows[i].stop, Some(Stop::RolePorts));
    }

    #[test]
    fn a_checked_ip_row_regrouped_on_a_rebuild_stays_checked_in_its_new_row() {
        let idx = crate::tui::test_support::fixture_index();
        let mut flows = fixture_flows_rich();
        let mut alone = flows[17].clone();
        alone.src.ip = Some("10.1.4.10".into());
        alone.src.ip_lists.clear();
        flows.push(alone);
        let mut v = GenView::new(shop(), "24h".into(), 1);
        v.set_rows(pce::generate::rows(&flows, &shop(), "role", &idx, None));
        let at = |v: &GenView, text: &str| v.rows.iter().position(|r| r.peer_text() == text);
        let lone = at(&v, "10.1.4.10").expect("no list: its own row");
        v.rows[lone].checked = true;
        // Now Corp holds it too: the row is gone, its IP in Corp's row.
        flows.last_mut().unwrap().src.ip_lists = vec!["Corp".into()];
        v.set_rows(pce::generate::rows(&flows, &shop(), "role", &idx, None));
        assert_eq!(at(&v, "10.1.4.10"), None);
        let corp = at(&v, "IP list Corp · 2 IPs").unwrap();
        assert!(v.rows[corp].checked, "the check followed its IP");
    }

    #[test]
    fn the_slider_stops_at_its_ends_and_skips_all_stops_without_all_services() {
        let mut v = view();
        v.slide(-1);
        assert_eq!(v.global, Stop::AppAll);
        v.slide(-1);
        assert_eq!(v.global, Stop::AppAll);
        v.global = Stop::AppPorts;
        v.all_services = false;
        v.slide(-1);
        assert_eq!(v.global, Stop::AppPorts, "app·all is off");
        v.slide(1);
        v.slide(1);
        assert_eq!(v.global, Stop::RolePorts, "role·all is off");
        assert_eq!(
            v.slider_line(),
            "granularity  (app·all) ─ app·ports ─ [role·ports] ─ (role·all)"
        );
    }

    #[test]
    fn tab_reaches_outbound_only_when_it_is_shown() {
        let mut v = view();
        v.switch_section();
        assert_eq!(v.section, Side::Inbound);
        v.toggle_outbound();
        assert_eq!(v.focus.direction, FocusDirection::Both);
        v.switch_section();
        assert_eq!(v.section, Side::Outbound);
        assert_eq!(v.selected(), Some(4), "crm/prod, the first outbound row");
        v.toggle_outbound();
        assert_eq!(v.section, Side::Inbound);
    }

    #[test]
    fn an_uncheckable_row_says_why() {
        let mut v = view();
        v.toggle_outbound();
        v.section = Side::Outbound;
        v.rows[4].checkable = false;
        v.rows[4].flags = vec![pce::generate::RowFlag::NoScopeLabels];
        assert_eq!(
            v.toggle(&shop().keys).unwrap_err(),
            "crm/prod cannot be checked: no scope labels: no ruleset to write into"
        );
    }

    #[test]
    fn rows_behind_a_denied_or_untargeted_rule_are_flagged() {
        let mut v = view();
        let idx = crate::tui::test_support::fixture_index();
        let drafts: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let flows = fixture_flows_rich();
        let input = pce::generate::GenInput {
            flows: &flows,
            focus: &v.focus,
            tier_key: "role",
            drafts: &drafts,
            idx: &idx,
            ruleset_name: "RF-{app}-{env}",
            window: "24h",
            outbound: false,
            picked: None,
            consolidate: &BTreeMap::new(),
            seen_before: 0,
        };
        let mut p = pce::generate::propose(&v.rows, v.global, &input);
        let row = p.rules[0].rows[0];
        assert_eq!(v.rule_flags(row), Vec::<String>::new(), "no proposal yet");
        p.rules[0].denied_flows = 2;
        p.targets[0] = Target::None("no ruleset".into());
        p.rules[1].action = pce::generate::RuleAction::Exists {
            rule_href: "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/9".into(),
        };
        v.proposal = Some(p);
        let idx = crate::tui::test_support::fixture_index();
        let list = v.rule_list(&idx);
        let at = list
            .iter()
            .position(|l| l.starts_with("2. exists"))
            .unwrap();
        assert!(
            !list[at + 1].contains("generated"),
            "an existing rule has no new description: {list:#?}"
        );
        assert_eq!(v.target_name(), "(no target)");
        assert_eq!(
            v.rule_flags(row),
            vec!["override deny: 2".to_string(), "no target".to_string()]
        );
    }

    #[test]
    fn the_header_counts_the_sections_in_force() {
        let mut v = view();
        assert_eq!(
            v.header_title(),
            "Generate · shop/prod · last 24h · 8 flows · complete"
        );
        v.toggle_outbound();
        assert!(v.header_title().contains("10 flows"));
        v.gap = Some("incomplete".into());
        assert!(v.header_title().ends_with("· incomplete"));
        assert_eq!(v.header_meta(), "0 req · cached");
        v.requests = 1;
        assert_eq!(v.header_meta(), "1 req");
    }

    /// `view` with fan-in from 3 peers: 5432/tcp (hr/prod, shop/dev, Corp).
    fn fan_view() -> GenView {
        let idx = crate::tui::test_support::fixture_index();
        let mut v = GenView::new(shop(), "24h".into(), 1);
        v.min_peers = 3;
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
            None,
        ));
        v
    }

    #[test]
    fn a_fan_in_port_gets_a_line_above_the_rows_that_c_cycles() {
        assert!(view().candidates.is_empty(), "3 peers is below 5");
        let mut v = fan_view();
        assert_eq!(v.candidates.len(), 1);
        assert_eq!(
            v.candidate_line(0),
            "⚠ 5432/tcp from 3 peers · c consolidates: off"
        );
        assert_eq!(v.selected_candidate(), Some(0), "the cursor starts on it");
        assert_eq!(v.selected(), None);
        v.move_cursor(1);
        assert_eq!(v.selected(), Some(0), "then intra-app");
        assert!(v.cycle_consolidation(None).is_err(), "not on its line");
        v.move_cursor(-1);
        let said = v.cycle_consolidation(None).unwrap();
        assert!(
            said.starts_with("destination only: 5432/tcp from any IP → shop/prod in one rule"),
            "{said}"
        );
        v.cycle_consolidation(None).unwrap();
        assert_eq!(
            v.consolidations().into_values().collect::<Vec<_>>(),
            vec![Consolidate::AllWorkloads]
        );
        v.cycle_consolidation(None).unwrap();
        assert!(v.consolidations().is_empty(), "back to off");
        // Without the Any list, Any is skipped and says why.
        let said = v.cycle_consolidation(Some("no Any list")).unwrap();
        assert!(said.ends_with("(Any unavailable: no Any list)"), "{said}");
        assert_eq!(
            v.mode_of(&PortSpec::parse("5432/tcp").unwrap()),
            Consolidate::AllWorkloads
        );
    }

    #[test]
    fn the_consolidation_and_the_cursor_on_its_line_survive_a_rebuild() {
        let mut v = fan_view();
        v.cycle_consolidation(None).unwrap();
        let idx = crate::tui::test_support::fixture_index();
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
            None,
        ));
        assert_eq!(v.consolidations().len(), 1);
        assert_eq!(v.selected_candidate(), Some(0));
        // On a row, the cursor stays on it past the fan-in line.
        v.move_cursor(2);
        let at = v.selected().unwrap();
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
            None,
        ));
        assert_eq!(v.selected(), Some(at));
    }

    #[test]
    fn rows_behind_a_consolidated_port_say_so_and_the_list_shows_the_rule() {
        let mut v = fan_view();
        v.cycle_consolidation(None).unwrap();
        let idx = crate::tui::test_support::fixture_index();
        let drafts: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let flows = fixture_flows_rich();
        let cons = v.consolidations();
        let input = pce::generate::GenInput {
            flows: &flows,
            focus: &v.focus,
            tier_key: "role",
            drafts: &drafts,
            idx: &idx,
            ruleset_name: "RF-{app}-{env}",
            window: "24h",
            outbound: false,
            picked: None,
            consolidate: &cons,
            seen_before: 0,
        };
        v.proposal = Some(pce::generate::propose(&v.rows, v.global, &input));
        assert_eq!(
            v.consolidated_flag(1),
            None,
            "hr/prod is checked: keeps its rule"
        );
        assert_eq!(
            v.consolidated_flag(2).as_deref(),
            Some("consolidated"),
            "shop/dev"
        );
        assert_eq!(
            v.consolidated_flag(3).as_deref(),
            Some("5432/tcp consolidated"),
            "Corp keeps 443/tcp"
        );
        assert!(v.rule_flags(2).is_empty(), "the flag is not a rule flag");
        assert!(v.has_consolidated_rule());
        assert_eq!(
            v.candidate_line(0),
            "⚠ 5432/tcp from 3 peers · c consolidates: Any · 1 rule"
        );
        let detail = v.candidate_detail(0);
        assert!(
            detail[0].starts_with("  ⚠ destination only for shop/dev"),
            "{detail:?}"
        );
        assert!(
            detail[1].starts_with("  ⚠ keeps 2 managed peer rules (their outbound)"),
            "{detail:?}"
        );
        assert_eq!(
            detail[2],
            "  → every workload of shop/prod · also allows: shop/dev, IP list Corp · 1 IP, any source not seen"
        );
        let list = v.rule_list(&idx);
        let at = list
            .iter()
            .position(|l| l.contains("ipl:Any (0.0.0.0/0 and ::/0) → all workloads on 5432/tcp"))
            .unwrap_or_else(|| panic!("{list:#?}"));
        assert!(
            list[at + 2].starts_with("   ⚠ destination only"),
            "{list:#?}"
        );
        assert!(
            list[at + 4].starts_with("   consolidates 5432/tcp from 2 peers"),
            "{list:#?}"
        );
    }

    #[test]
    fn a_consolidation_without_a_rule_says_why_and_a_lost_candidate_forgets_it() {
        let mut v = fan_view();
        v.cycle_consolidation(None).unwrap();
        v.cycle_consolidation(None).unwrap(); // All Workloads
        v.proposal = Some(pce::generate::Proposal {
            scopes: Vec::new(),
            targets: Vec::new(),
            ip_lists: Vec::new(),
            rules: Vec::new(),
            coverage: Default::default(),
        });
        assert!(!v.has_consolidated_rule());
        assert!(
            v.candidate_line(0)
                .ends_with("no rule: All Workloads covers none of these peers (IP only)")
        );
        v.consolidate
            .insert(PortSpec::parse("5432/tcp").unwrap(), Consolidate::Any);
        v.any_missing = Some("no Any list".into());
        assert!(
            v.candidate_line(0)
                .ends_with("no rule: Any unavailable: no Any list")
        );
        // Below the threshold now: the choice is dropped, not kept for later.
        v.min_peers = 5;
        let idx = crate::tui::test_support::fixture_index();
        v.set_rows(pce::generate::rows(
            &fixture_flows_rich(),
            &shop(),
            "role",
            &idx,
            None,
        ));
        assert!(v.candidates.is_empty());
        assert!(v.consolidate.is_empty());
    }
}
