//! The policy generator's state (`G` / `:gen`): the app's rows with their
//! checkboxes and stops, the slider, the section and cursor, and the
//! proposal they lead to. Pure: `update` fills it, `ui::widgets::generator`
//! draws it. Nothing here makes a request.
use pce::NameIndex;
use pce::generate::{GenRow, PeerKind, Proposal, RowKey, Side, Stop, Target};
use pce::traffic::Focus;
use std::collections::BTreeMap;

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
        }
    }

    /// New rows; checkboxes, overrides and the cursor follow their row by
    /// key (the rows may come in another order after a re-query).
    pub fn set_rows(&mut self, rows: Vec<GenRow>) {
        let at = self.selected().map(|i| self.rows[i].key());
        let old: BTreeMap<RowKey, (bool, Option<Stop>)> = self
            .rows
            .iter()
            .map(|r| (r.key(), (r.checked, r.stop)))
            .collect();
        self.rows = rows;
        for r in &mut self.rows {
            if let Some((checked, stop)) = old.get(&r.key()) {
                r.checked = *checked && r.checkable;
                r.stop = *stop;
            }
        }
        for s in [Side::Inbound, Side::Outbound] {
            let n = self.section_rows(s).len();
            let c = &mut self.cursor[side_index(s)];
            *c = (*c).min(n.saturating_sub(1));
        }
        if let Some(key) = at {
            let section = self.section;
            if let Some(pos) = self
                .section_rows(section)
                .iter()
                .position(|i| self.rows[*i].key() == key)
            {
                self.cursor[side_index(section)] = pos;
            }
        }
    }

    /// The indexes (into `rows`) of one section's rows.
    pub fn section_rows(&self, s: Side) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|i| self.rows[*i].side == s)
            .collect()
    }

    /// The row under the cursor (an index into `rows`).
    pub fn selected(&self) -> Option<usize> {
        self.section_rows(self.section)
            .get(self.cursor[side_index(self.section)])
            .copied()
    }

    /// `j` / `k`, clamped to the section.
    /// Past the last inbound row `j` goes on into the outbound section
    /// (when it is shown and has rows), `k` from its first row back.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.section_rows(self.section).len();
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
            let n = self.section_rows(Side::Inbound).len();
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
        format!(
            "Generate · {} · last {} · {flows} flows · {}",
            self.focus.app_text(),
            self.window,
            if self.gap.is_some() {
                "incomplete"
            } else {
                "complete"
            }
        )
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
        if flows == 0 {
            return "no flows in the window: nothing to propose".into();
        }
        let Some(p) = &self.proposal else {
            return "rules follow when the draft rulesets arrive".into();
        };
        let c = p.counts();
        format!(
            "{} rules ({} new · {} add services · {} exist) · covers {} of {} flows · {} not",
            c.rules,
            c.create,
            c.add_services,
            c.exists,
            p.coverage.covered,
            p.coverage.covered + p.coverage.uncovered,
            p.coverage.uncovered
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
            &[],
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
            &[],
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
        v.set_rows(pce::generate::rows(&flows, &shop(), "role", &idx, &[]));
        let i = v.selected().unwrap();
        assert_eq!(v.rows[i].peer_text(), "IP list Corp · 2 IPs");
        assert!(v.rows[i].checked);
        assert_eq!(v.rows[i].stop, Some(Stop::RolePorts));
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
}
