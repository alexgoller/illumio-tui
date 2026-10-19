//! The Policy screen's state: the rulesets and rules panes, their rows
//! and columns, the deny rules and the pending list.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyPane {
    RuleSets,
    Rules,
}

pub(super) fn plain_names<'a>(v: impl Iterator<Item = &'a str>) -> String {
    v.collect::<Vec<_>>().join(", ")
}

pub(super) fn actor_names(v: &[pce::ResolvedActor]) -> String {
    plain_names(v.iter().map(|a| a.name.as_str()))
}

pub(super) fn service_names(v: &[pce::ResolvedService]) -> String {
    plain_names(v.iter().map(|s| s.name.as_str()))
}

/// Evaluation order of the rule kinds: an override deny beats an allow,
/// which beats a plain deny.
pub(super) fn rule_rank(kind: RuleKind) -> u8 {
    match kind {
        RuleKind::OverrideDeny => 0,
        RuleKind::Allow => 1,
        RuleKind::Deny => 2,
    }
}

pub fn rule_kind_name(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Allow => "allow",
        RuleKind::Deny => "deny",
        RuleKind::OverrideDeny => "override_deny",
    }
}

/// A rule on one line as the rules pane names it: `allow role=db ← role=web
/// PostgreSQL` (kind, providers ← consumers, services), ` (disabled)` when off.
pub fn rule_line(r: &RuleView) -> String {
    let mut s = format!(
        "{} {} ← {} {}",
        rule_kind_name(r.kind),
        actor_names(&r.providers),
        actor_names(&r.consumers),
        service_names(&r.services)
    );
    if !r.enabled {
        s.push_str(" (disabled)");
    }
    s
}

pub(super) fn on_off(enabled: bool) -> String {
    if enabled { "✓".into() } else { "✗".into() }
}

pub fn ruleset_columns() -> Vec<Column<RuleSetView>> {
    vec![
        Column {
            title: "ruleset",
            width: 22,
            cell: |r| r.name.clone(),
            key: |r| SortKey::Text(r.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "on",
            width: 3,
            cell: |r| on_off(r.enabled),
            key: |r| SortKey::Num(u64::from(r.enabled)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| r.enabled.to_string()),
        },
        Column {
            title: "scopes",
            width: 0,
            cell: |r| r.scopes.join(" | "),
            key: |r| SortKey::Text(r.scopes.join(" | ")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| join_list(&r.scopes)),
        },
        Column {
            title: "rules",
            width: 5,
            cell: |r| r.rule_count.to_string(),
            key: |r| SortKey::Num(r.rule_count as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |r| r.update_type.clone().unwrap_or_default(),
            key: |r| SortKey::Text(r.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

/// A rules-pane row: the resolved rule, its impact over the cached workloads
/// (`None` until they are cached) and the `P/C` cell made from it (`12/3`,
/// `1.2K/5.7K` from 1000 on, `-`). Both are computed once, when the rows are built: a column's `fn`
/// pointer cannot see the workload cache, and the rule detail reads the
/// stored impact instead of evaluating it on every draw.
#[derive(Debug, Clone)]
pub struct RuleRow {
    pub view: RuleView,
    pub impact: Option<pce::Impact>,
    pub pc: String,
}

impl RuleRow {
    /// Sort key of the `P/C` column: providers, then consumers (`≥` sorts by
    /// its number); rows without an impact last.
    pub(crate) fn pc_key(&self) -> u64 {
        self.impact.as_ref().map_or(u64::MAX, |i| {
            ((i.providers as u64) << 32) | (i.consumers as u64 & 0xFFFF_FFFF)
        })
    }
}

impl std::ops::Deref for RuleRow {
    type Target = RuleView;

    fn deref(&self) -> &RuleView {
        &self.view
    }
}

pub fn rule_columns() -> Vec<Column<RuleRow>> {
    vec![
        Column {
            title: "kind",
            width: 13,
            cell: |r| rule_kind_name(r.kind).to_string(),
            key: |r| SortKey::Text(rule_kind_name(r.kind).to_string()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "on",
            width: 3,
            cell: |r| on_off(r.enabled),
            key: |r| SortKey::Num(u64::from(r.enabled)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| r.enabled.to_string()),
        },
        Column {
            title: "consumers",
            width: 0,
            cell: |r| actor_names(&r.consumers),
            key: |r| SortKey::Text(actor_names(&r.consumers)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| join_list(r.consumers.iter().map(|a| &a.name))),
        },
        Column {
            title: "providers",
            width: 0,
            cell: |r| actor_names(&r.providers),
            key: |r| SortKey::Text(actor_names(&r.providers)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| join_list(r.providers.iter().map(|a| &a.name))),
        },
        Column {
            title: "services",
            width: 0,
            cell: |r| service_names(&r.services),
            key: |r| SortKey::Text(service_names(&r.services)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| join_list(r.services.iter().map(|s| &s.name))),
        },
        Column {
            title: "P/C",
            // `Impact::pc_compact`'s widest form, `≥123K/≥456K`.
            width: 11,
            cell: |r| r.pc.clone(),
            key: |r| SortKey::Num(r.pc_key()),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| r.impact.as_ref().map(pce::Impact::pc).unwrap_or_default()),
        },
        Column {
            title: "pending",
            width: 7,
            cell: |r| r.update_type.clone().unwrap_or_default(),
            key: |r| SortKey::Text(r.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 90,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

/// A rule the Policy search (`f`) found, with its ruleset.
#[derive(Debug, Clone)]
pub struct FoundRow {
    pub ruleset: String,
    pub ruleset_href: String,
    pub ruleset_enabled: bool,
    /// The ruleset's scopes as its form shows them; `All` without any.
    pub scopes: String,
    pub view: RuleView,
}

impl std::ops::Deref for FoundRow {
    type Target = RuleView;

    fn deref(&self) -> &RuleView {
        &self.view
    }
}

fn found_col(title: &'static str, width: u16, cell: fn(&FoundRow) -> String) -> Column<FoundRow> {
    Column {
        title,
        width,
        cell,
        key: |_| SortKey::Text(String::new()),
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
        full_title: None,
        full: None,
    }
}

/// The search's flat rule list: ruleset, on, kind, scope, consumers,
/// providers, services.
pub fn found_columns() -> Vec<Column<FoundRow>> {
    vec![
        Column {
            key: |r| SortKey::Text(r.ruleset.clone()),
            ..found_col("ruleset", 18, |r| r.ruleset.clone())
        },
        Column {
            key: |r| SortKey::Num(u64::from(r.enabled)),
            full: Some(|r| r.enabled.to_string()),
            ..found_col("on", 3, |r| on_off(r.enabled))
        },
        Column {
            key: |r| SortKey::Text(rule_kind_name(r.kind).to_string()),
            hide_below: 100,
            ..found_col("kind", 13, |r| rule_kind_name(r.kind).to_string())
        },
        Column {
            key: |r| SortKey::Text(r.scopes.clone()),
            ..found_col("scope", 0, |r| r.scopes.clone())
        },
        Column {
            key: |r| SortKey::Text(actor_names(&r.consumers)),
            full: Some(|r| join_list(r.consumers.iter().map(|a| &a.name))),
            ..found_col("consumers", 0, |r| actor_names(&r.consumers))
        },
        Column {
            key: |r| SortKey::Text(actor_names(&r.providers)),
            full: Some(|r| join_list(r.providers.iter().map(|a| &a.name))),
            ..found_col("providers", 0, |r| actor_names(&r.providers))
        },
        Column {
            key: |r| SortKey::Text(service_names(&r.services)),
            full: Some(|r| join_list(r.services.iter().map(|s| &s.name))),
            ..found_col("services", 0, |r| service_names(&r.services))
        },
        Column {
            key: |r| SortKey::Text(r.href.clone()),
            // `:export` only: the terminal has no room for it.
            hide_below: u16::MAX,
            ..found_col("href", 0, |r| r.href.clone())
        },
    ]
}

#[derive(Debug)]
pub struct PolicyState {
    pub scope: PolicyScope,
    pub pane: PolicyPane,
    pub rulesets: TableView<RuleSetView>,
    pub rules: TableView<RuleRow>,
    pub raw: Vec<RuleSet>,
    /// Deny rules fetched from the endpoint for rulesets whose inline list is empty.
    pub fallback_deny: HashMap<String, Vec<DenyRule>>,
    /// Rulesets whose deny-rule fallback was already requested (once per load).
    pub deny_requested: HashSet<String>,
    pub loaded_at: Option<Instant>,
    pub age_secs: u64,
    pub loading: bool,
    /// Set by an explicit `r`: the next ruleset reload also bypasses the cache for deny rules.
    pub bypass_deny: bool,
    pub pending: Option<Vec<PendingChange>>,
    /// A pending-changes request is in flight (`p` does not send another).
    pub pending_loading: bool,
    /// The highlighted entry of the pending overlay.
    pub pending_selected: usize,
    /// A diff request is in flight (`Enter` does not send another).
    pub diff_loading: bool,
    /// The next rulesets result was asked for by a write: it keeps the
    /// write's report in the status line instead of `N rulesets`.
    pub quiet_reload: bool,
    /// The open rule confirm's impact line, evaluated when the confirm
    /// opens and when the workloads or rulesets load (never per draw).
    pub confirm_impact: Option<String>,
    /// Pending entries marked with `Space` for `P` and `R` (hrefs); cleared
    /// when the list is loaded again.
    pub pending_marked: BTreeSet<String>,
    /// The counts of the provision just sent (`ip_lists=1, rule_sets=2`),
    /// taken from its body at `y`: the PCE's answer does not repeat them.
    pub provision_counts: Option<String>,
    /// The report of the provision just done; the pending reload it caused
    /// shows it in front of the new count.
    pub provisioned: Option<String>,
    /// What the user typed into an open revert confirm; only `revert`
    /// followed by `Enter` sends it.
    pub revert_typed: String,
    /// A `w` rule waiting for the ruleset `Ctrl-N` creates: planned into
    /// it once that write is done, dropped by any failure or cancel.
    /// The form comes along so a failed rule plan can reopen it.
    pub rule_after_ruleset: Option<(pce::write::RuleDraft, crate::tui::form::Form)>,
    /// The plan on its confirm came from a Traffic `w` form (or the rule
    /// or ruleset its `Ctrl-R` chained): its written answer evaluates the
    /// result on screen against the draft. Set by every plan to send,
    /// taken by every write's answer.
    pub write_from_flow: bool,
    /// The draft rulesets a `w` form reads while this screen shows the
    /// active ones (its own list stays what the user chose); `None` until a
    /// form loads them, and again after a policy write.
    pub form_drafts: Option<Vec<RuleSet>>,
    /// That load is on its way: a second form does not send another.
    pub form_drafts_loading: bool,
    /// A policy write forgot the drafts while that load was on its way:
    /// its answer, read before the write, is dropped.
    pub form_drafts_stale: bool,
    /// Explain's `Enter` (`E`): the ruleset and rule (hrefs) to select once
    /// the draft rulesets on their way land.
    pub jump: Option<(String, String)>,

    pub auto: AutoRefresh,

    /// `f`: the rule search in force. While set the screen shows `found`,
    /// one flat list of the matching rules of every loaded ruleset.
    pub search: Option<pce::RuleFilter>,
    /// The search's text as last applied: `f` opens on it again after
    /// `Enter` (opened a rule) or `c` (cleared).
    pub last_search: String,
    pub found: TableView<FoundRow>,
    /// Rulesets whose deny rules are not loaded (no inline list, the
    /// endpoint not fetched): the search could not look at them.
    pub found_deny_unloaded: usize,
    /// `I`: the open list of a rule's matched workloads (`Overlay::Matched`).
    pub matched: Option<crate::tui::impact_list::MatchedView>,
}

/// One loaded rule that names an IP list, for the IP lists tab.
#[derive(Debug, Clone)]
pub struct IpListUse {
    pub ruleset: String,
    pub kind: RuleKind,
    pub providers: Vec<Actor>,
    pub consumers: Vec<Actor>,
}

impl PolicyState {
    /// The search's rows from the loaded rulesets (`found`), each
    /// ruleset's rules in evaluation order; the selection stays on its
    /// rule. Nothing without a search. `workloads` (those cached) name the
    /// workload actors for `wl:`. No request.
    pub fn rebuild_found(&mut self, idx: &NameIndex, workloads: &[Workload]) {
        let Some(filter) = &self.search else {
            self.found.set_rows(Vec::new());
            self.found_deny_unloaded = 0;
            return;
        };
        let keep = self.found.selected_item().map(|r| r.href.clone());
        let mut rows = Vec::new();
        let mut unloaded = 0;
        for rs in &self.raw {
            let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
                match self.fallback_deny.get(&rs.href) {
                    Some(d) => d,
                    None => {
                        unloaded += 1;
                        &[]
                    }
                }
            } else {
                &rs.deny_rules
            };
            let scopes = crate::tui::edit::scopes_text(&rs.scopes, idx);
            let row = |view: RuleView| FoundRow {
                ruleset: rs.name.clone(),
                ruleset_href: rs.href.clone(),
                ruleset_enabled: rs.enabled,
                scopes: if scopes.is_empty() {
                    "All".to_string()
                } else {
                    scopes.clone()
                },
                view,
            };
            let mut mine: Vec<FoundRow> = rs
                .rules
                .iter()
                .filter(|r| filter.matches(&pce::RuleRef::of_rule(r, rs), idx, workloads))
                .map(|r| row(RuleView::from_rule(r, idx)))
                .chain(
                    deny.iter()
                        .filter(|d| filter.matches(&pce::RuleRef::of_deny(d, rs), idx, workloads))
                        .map(|d| row(RuleView::from_deny(d, idx))),
                )
                .collect();
            mine.sort_by_key(|r| rule_rank(r.kind));
            rows.extend(mine);
        }
        self.found.set_rows(rows);
        self.found_deny_unloaded = unloaded;
        if let Some(href) = keep {
            self.found.select_where(|r| r.href == href);
        }
    }
}
