//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::cli::output::join_list;
use crate::marks::MarksStore;
use crate::tui::app_picker::AppPicker;
use crate::tui::app_view::AppView;
use crate::tui::diff::ObjectDiff;
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::form::Form;
use crate::tui::label_editor::LabelFilterEditor;
use crate::tui::picker::KeyPicker;
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::traffic_marks::{MarkedView, Squelch, sum_marks};
use crate::tui::ui::theme::Theme;
use crate::tui::wl_groups::{
    GroupOrder, WlGroup, WlGroupRow, build_groups, layout as wl_layout, member_columns,
    order_groups,
};
use pce::impact::RuleActors;
use pce::marks::{Hit, PceId, RowMarks, annotate_groups, annotate_mesh, annotate_sections};
use pce::models::{
    Actor, DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope,
    Rule, RuleSet, Service, VenHealth, Workload,
};
use pce::traffic::treemap::{Rect as TreemapRect, Tile, Weight, layout, layout_zoomed};
use pce::traffic::{
    AppSection, Decisions, Direction, DraftCounts, FlowGroup, Focus, FocusDirection, GroupKey,
    MeshColumns, MeshKey, MeshRow, PeerRow, ResolvedFlow, SectionOrder, Totals, app_header,
    app_sections, app_sections_both, apps_in, compact_count, flow_sections, group_by_labels,
    group_by_service, group_key, group_text, in_app, in_peer, keys_or_default, keys_seen, mesh,
    mesh_key, order_sections,
};
use pce::ven::{HealthSet, VenSummary, VenTally};
use pce::write::Plan;
use pce::{
    IpListSize, KeyOrder, LabelFilter, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot,
    ip_list_size,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

mod compare;
mod events;
mod flows;
mod objects;
mod policy;
mod traffic;
mod workloads;

pub use compare::*;
pub use events::*;
pub use flows::*;
pub use objects::*;
pub use policy::*;
pub use traffic::*;
pub use workloads::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Traffic,
    Policy,
    Objects,
    Workloads,
    Events,
}

impl Screen {
    /// In tab order; a screen's key is its position plus one.
    pub const ALL: [Screen; 5] = [
        Screen::Traffic,
        Screen::Policy,
        Screen::Objects,
        Screen::Workloads,
        Screen::Events,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traffic",
            Screen::Policy => "Policy",
            Screen::Objects => "Objects",
            Screen::Workloads => "Workloads",
            Screen::Events => "Events",
        }
    }

    /// The tab row's label when the full titles do not fit.
    pub fn short_title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traf",
            Screen::Policy => "Pol",
            Screen::Objects => "Obj",
            Screen::Workloads => "Wl",
            Screen::Events => "Ev",
        }
    }

    pub fn index(self) -> usize {
        Screen::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileInfo {
    pub name: String,
    pub host: String,
    pub org_id: u64,
}

#[derive(Debug)]
pub enum Connection {
    Connecting,
    /// Health passed; the name index may still be loading (`index` None).
    Connected {
        index: Option<std::sync::Arc<pce::NameIndex>>,
        index_age_secs: u64,
    },
    Disconnected {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    pub kind: StatusKind,
}

impl StatusLine {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Info,
        }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Warn,
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Error,
        }
    }
}

/// Why writes are off when the session toggle is: at startup without
/// `--allow-writes`, and after `:writes off`.
pub const WRITES_OFF: &str = "writes off; :writes on to enable";

/// Whether editing keys may start a write flow. The guard itself lives in
/// the runtime (`Runtime::writer`); the model never holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritesState {
    /// The session toggle on, the profile allowing writes and the audit sink writable.
    pub enabled: bool,
    /// Why not, for the status line.
    pub reason: Option<String>,
    /// A plan or a send is on its way: set when `Cmd::PlanWrite` or
    /// `Cmd::ExecuteWrite` is emitted, cleared when its answer lands (and by
    /// `Model::reset_data`). Editing keys, `:profile` and `:writes` wait for it.
    pub in_flight: bool,
}

impl WritesState {
    pub fn on() -> Self {
        Self {
            enabled: true,
            reason: None,
            in_flight: false,
        }
    }

    pub fn off(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            reason: Some(reason.into()),
            in_flight: false,
        }
    }

    /// The status-line text for an editing key pressed while writes are off.
    pub fn refusal(&self) -> String {
        self.reason
            .clone()
            .unwrap_or_else(|| "writes are not enabled".into())
    }
}

/// A modal drawn on top of the current screen. Later tasks add `Error`, `Detail`.
#[derive(Debug, Clone)]
pub enum Overlay {
    None,
    Help,
    Prompt {
        input: String,
    },
    /// `c` on Traffic: the baseline's length, one line (`7d`; Ctrl-N /
    /// Ctrl-P step through the window steps).
    Baseline {
        input: String,
    },
    /// `N` in the generator: the same prompt for "new since" (#81); the
    /// generator stays behind it.
    GenBaseline {
        input: String,
    },
    FilterEditor(FilterEditor),
    /// `f` on Workloads: the label filter, one line.
    LabelFilter(LabelFilterEditor),
    /// `f` on Policy: the rule search, one line.
    RuleFilter(crate::tui::rule_search::RuleFilterEditor),
    Confirm {
        reason: String,
        action: PendingAction,
    },
    /// `g`: the label keys every traffic view groups by.
    GroupKeys(KeyPicker),
    /// `F` without an app under the cursor, `:focus` alone: the apps of the
    /// loaded result.
    AppPicker(AppPicker),
    /// `A` / `:app`: the app overlay; its state is `Model::app_view`.
    App,
    /// `G` / `:gen`: the policy generator; its state is `Model::gen_view`.
    Generator,
    /// `f` on the Labels tab: the label keys it shows.
    LabelKeys(KeyPicker),
    /// The selected policy rule in full.
    RuleDetail(RuleView),
    /// `I`: the workloads behind a rule's impact; its state is
    /// `PolicyState::matched`.
    Matched,
    /// Pending (unprovisioned) changes; the list lives in `PolicyState::pending`.
    Pending,
    /// A failure too long for the status line, or an authentication failure.
    Error(String),
    /// Events filter: severity, type and actor (local), window.
    EventsEditor(crate::tui::events_editor::EventsEditor),
    /// `t` on Events: the loaded events' types (or with `actors`, after
    /// `Tab`, their actors) with their counts, most frequent first;
    /// `selected` is the cursor row.
    EventTypes {
        selected: usize,
        actors: bool,
    },
    /// `:ven` on Workloads: the VEN fleet of the loaded workloads
    /// (`WorkloadsState::ven`); `selected` is the bucket under the cursor.
    Ven {
        selected: usize,
    },
    /// The selected audit event as a change diff, or as pretty JSON when
    /// `raw` (`J` toggles); `scroll` is the first shown line.
    EventDetail {
        event: Event,
        scroll: usize,
        raw: bool,
    },
    /// Create or edit form for a label, IP list, service or workload.
    Form(Form),
    /// `m`: a mark for the selected row; saved to `marks.toml`, never sent.
    MarkForm(Box<crate::tui::mark_form::MarkForm>),
    /// `:marks`: every mark; its state is `Model::marks_view`.
    Marks,
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
    ConfirmWrite {
        plan: Plan,
        kind: pce::write::ObjectKind,
        scroll: u16,
    },
    /// Every request of a write batch (the generator's); `y` sends them
    /// all, in order; `n`/`Esc` cancel. `scroll` is the first line shown.
    ConfirmBatch {
        batch: pce::write::Batch,
        scroll: u16,
    },
    /// The last sent batch, step by step (`Model::batch_report`): what was
    /// written where, what failed, what was not sent, how to undo.
    BatchReport {
        scroll: u16,
    },
    /// `M` on Workloads: the enforcement mode and visibility level for the
    /// targets, with the change it makes.
    EnforcePicker(Box<crate::tui::bulk::EnforcePicker>),
    /// A planned bulk workload change: every change, the typed count when
    /// it enforces several workloads, the draft check (`d`); `y` sends it.
    ConfirmBulk(Box<crate::tui::bulk::BulkConfirm>),
    /// A pending IP list or service against its active version.
    Diff {
        diff: ObjectDiff,
        scroll: u16,
    },
    /// `E`: which rule allows or blocks a flow (no request).
    Explain(Box<crate::tui::explain::ExplainView>),
    /// `:readiness`: every app of the result on screen, judged; its state
    /// is `Model::readiness_view`.
    Readiness,
    /// `:enforce` / `e` in the app overlay: one app's five steps to
    /// enforcement; its state is `Model::enforce_view`.
    Enforce,
    /// `:filters`: the current screen's saved filters.
    FilterPicker(crate::tui::saved::FilterPicker),
    /// `W` / `:used` on Objects: what names the selected object; its state
    /// is `ObjectsState::where_used`.
    WhereUsed,
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Overlay::None, Overlay::None)
            | (Overlay::Help, Overlay::Help)
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App)
            | (Overlay::Generator, Overlay::Generator)
            | (Overlay::Marks, Overlay::Marks)
            | (Overlay::Readiness, Overlay::Readiness)
            | (Overlay::Enforce, Overlay::Enforce)
            | (Overlay::Matched, Overlay::Matched)
            | (Overlay::WhereUsed, Overlay::WhereUsed) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Baseline { input: a }, Overlay::Baseline { input: b })
            | (Overlay::GenBaseline { input: a }, Overlay::GenBaseline { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b))
            | (Overlay::LabelKeys(a), Overlay::LabelKeys(b)) => a == b,
            (Overlay::FilterEditor(a), Overlay::FilterEditor(b)) => a == b,
            (Overlay::LabelFilter(a), Overlay::LabelFilter(b)) => a == b,
            (Overlay::RuleFilter(a), Overlay::RuleFilter(b)) => a == b,
            (Overlay::AppPicker(a), Overlay::AppPicker(b)) => a == b,
            (Overlay::FilterPicker(a), Overlay::FilterPicker(b)) => a == b,
            (Overlay::Form(a), Overlay::Form(b)) => a == b,
            (Overlay::MarkForm(a), Overlay::MarkForm(b)) => a == b,
            (
                Overlay::ConfirmWrite {
                    plan: pa,
                    kind: ka,
                    scroll: sa,
                },
                Overlay::ConfirmWrite {
                    plan: pb,
                    kind: kb,
                    scroll: sb,
                },
            ) => pa.request() == pb.request() && ka == kb && sa == sb,
            (
                Overlay::ConfirmBatch {
                    batch: ba,
                    scroll: sa,
                },
                Overlay::ConfirmBatch {
                    batch: bb,
                    scroll: sb,
                },
            ) => ba.lines() == bb.lines() && sa == sb,
            (Overlay::BatchReport { scroll: a }, Overlay::BatchReport { scroll: b }) => a == b,
            (Overlay::EnforcePicker(a), Overlay::EnforcePicker(b)) => a == b,
            (Overlay::ConfirmBulk(a), Overlay::ConfirmBulk(b)) => a == b,
            (
                Overlay::Confirm {
                    reason: ra,
                    action: aa,
                },
                Overlay::Confirm {
                    reason: rb,
                    action: ab,
                },
            ) => ra == rb && aa == ab,
            (
                Overlay::Diff {
                    diff: a,
                    scroll: sa,
                },
                Overlay::Diff {
                    diff: b,
                    scroll: sb,
                },
            ) => a == b && sa == sb,
            // `RuleView` is not `PartialEq`; a rule is identified by its href.
            (Overlay::RuleDetail(a), Overlay::RuleDetail(b)) => a.href == b.href,
            (Overlay::EventsEditor(a), Overlay::EventsEditor(b)) => a == b,
            (
                Overlay::EventTypes {
                    selected: a,
                    actors: x,
                },
                Overlay::EventTypes {
                    selected: b,
                    actors: y,
                },
            ) => a == b && x == y,
            (Overlay::Ven { selected: a }, Overlay::Ven { selected: b }) => a == b,
            (
                Overlay::EventDetail {
                    event: a,
                    scroll: sa,
                    raw: ra,
                },
                Overlay::EventDetail {
                    event: b,
                    scroll: sb,
                    raw: rb,
                },
            ) => a.href == b.href && sa == sb && ra == rb,
            (Overlay::Explain(a), Overlay::Explain(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Overlay {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    /// A new filter from the editor or a command: runs through the cache.
    RunQuery(TrafficFilter),
    /// `r` on a filter that did not produce the data on screen: bypasses the cache.
    RunNow(TrafficFilter),
    /// `u` on the Labels tab: the workloads and the rulesets not cached
    /// yet, read with their screens' own loads.
    LoadLabelUsage,
    /// `r` on a partial focus past a cost threshold: one query per side.
    RunFocused(TrafficFilter, pce::traffic::Focus, pce::CacheMode),
    /// `D` past a cost threshold, or without a query to re-evaluate: the
    /// draft run for the result on screen (`filter`, the focused result's
    /// focus), re-evaluating `hrefs` (empty: run the queries again).
    RunDraft {
        filter: TrafficFilter,
        focus: Option<pce::traffic::Focus>,
        hrefs: Vec<String>,
    },
    /// `c` (or `B` on a truncated baseline) past a cost threshold: the
    /// baseline query, `filter` (its window the baseline's length) over
    /// `[start, end)`.
    RunBaseline {
        filter: TrafficFilter,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
    },
    /// `r` in the app overlay: the workloads and the rulesets again, past the cache.
    ReloadApp,
    /// `r` in the generator on incomplete data, past a cost threshold: the
    /// focused query for its app; `y` and `n` go back to the generator.
    GenRequery(TrafficFilter, pce::traffic::Focus),
    /// `N` in the generator past a cost threshold: its baseline, the
    /// focused query for its app (`filter`, its window the baseline's
    /// length) over `[start, end)`; `y` and `n` go back to the generator.
    GenBaseline {
        filter: TrafficFilter,
        focus: pce::traffic::Focus,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
    },
    /// `x` on `:marks`: this mark leaves `marks.toml`; `y` and `n` go back
    /// to the overlay. A local file only.
    DeleteMark(String),
    /// `r` on the enforcement overlay's Watch step past a cost threshold:
    /// the blocked-since query; `y` and `n` go back to the overlay.
    EnforceWatch,
    /// `:save` over a saved filter of that name: `y` replaces it. A local
    /// file only.
    SaveFilter(String, crate::filters::Saved),
    /// `D` in `:filters`: this filter leaves the config file; `y` and `n`
    /// go back to the picker. A local file only.
    DeleteFilter(crate::tui::saved::FilterPicker, String),
}

impl PolicyState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            scope: PolicyScope::Draft,
            pane: PolicyPane::RuleSets,
            rulesets: TableView::new(ruleset_columns()),
            rules: TableView::new(rule_columns()),
            raw: Vec::new(),
            fallback_deny: HashMap::new(),
            deny_requested: HashSet::new(),
            loaded_at: None,
            age_secs: 0,
            loading: false,
            bypass_deny: false,
            pending: None,
            pending_loading: false,
            pending_selected: 0,
            diff_loading: false,
            quiet_reload: false,
            confirm_impact: None,
            scope_warnings: Vec::new(),
            pending_marked: BTreeSet::new(),
            provision_counts: None,
            provisioned: None,
            revert_typed: String::new(),
            rule_after_ruleset: None,
            write_from_flow: false,
            form_drafts: None,
            form_drafts_loading: false,
            form_drafts_stale: false,
            jump: None,
            auto: AutoRefresh::new(min_refresh),
            search: None,
            last_search: String::new(),
            found: TableView::new(found_columns()),
            found_deny_unloaded: 0,
            found_loaded: 0,
            deny_endpoint_missing: false,
            matched: None,
        }
    }

    /// Whether the focused pane's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
        if self.search.is_some() {
            return self.found.is_searching();
        }
        match self.pane {
            PolicyPane::RuleSets => self.rulesets.is_searching(),
            PolicyPane::Rules => self.rules.is_searching(),
        }
    }

    pub fn set_rule_sets(
        &mut self,
        raw: Vec<RuleSet>,
        idx: &NameIndex,
        workloads: Option<(&[Workload], bool)>,
    ) {
        self.rulesets.set_rows(
            raw.iter()
                .map(|rs| RuleSetView::from_rule_set(rs, idx))
                .collect(),
        );
        self.raw = raw;
        self.rebuild_rules(idx, workloads);
    }

    /// Forgets everything loaded (a scope switch shows nothing of the old scope).
    pub fn clear(&mut self) {
        self.rulesets.set_rows(Vec::new());
        self.rules.set_rows(Vec::new());
        self.found.set_rows(Vec::new());
        self.raw.clear();
        self.fallback_deny.clear();
        self.deny_requested.clear();
        self.loaded_at = None;
        self.age_secs = 0;
        self.bypass_deny = false;
        self.pending_loading = false;
    }

    /// Rules of the selected ruleset in evaluation order: override deny rules,
    /// then allow rules, then deny rules (inline or fetched fallback). The
    /// order within each group is the PCE's. `workloads` (the cache, when
    /// loaded) fills the `P/C` column.
    pub fn rebuild_rules(&mut self, idx: &NameIndex, workloads: Option<(&[Workload], bool)>) {
        self.rebuild_found(idx, workloads.map_or(&[], |(w, _)| w));
        let Some(href) = self.selected_href() else {
            self.rules.set_rows(Vec::new());
            return;
        };
        let Some(rs) = self.raw.iter().find(|r| r.href == href) else {
            self.rules.set_rows(Vec::new());
            return;
        };
        // The workloads' label sets are built once for all the rules.
        let warm = workloads.map(|(w, truncated)| (w, truncated, pce::workload_labels(w)));
        let cells = |a: &RuleActors| {
            let impact = warm.as_ref().map(|(w, truncated, labels)| {
                crate::tui::impact::compute_with(&rs.scopes, a, idx, w, *truncated, labels)
            });
            (
                impact
                    .as_ref()
                    .map_or_else(|| "-".to_string(), pce::Impact::pc_compact),
                impact,
            )
        };
        let mut rows: Vec<RuleRow> = rs
            .rules
            .iter()
            .map(|r| {
                let (pc, impact) = cells(&RuleActors::of_rule(r));
                RuleRow {
                    view: RuleView::from_rule(r, idx),
                    impact,
                    pc,
                }
            })
            .collect();
        let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
            self.fallback_deny.get(&href).map_or(&[], Vec::as_slice)
        } else {
            &rs.deny_rules
        };
        rows.extend(deny.iter().map(|d| {
            let (pc, impact) = cells(&RuleActors::of_deny(d));
            RuleRow {
                view: RuleView::from_deny(d, idx),
                impact,
                pc,
            }
        }));
        rows.sort_by_key(|r| rule_order(r));
        // A rule the reload moved (its scope switched) keeps the cursor.
        let keep = self.rules.selected_item().map(|r| r.href.clone());
        self.rules.set_rows(rows);
        if let Some(href) = keep {
            self.rules.select_where(|r| r.href == href);
        }
    }

    pub fn selected_href(&self) -> Option<String> {
        self.rulesets.selected_item().map(|r| r.href.clone())
    }

    /// The selected ruleset as loaded, for an edit form.
    pub fn selected_rule_set(&self) -> Option<&RuleSet> {
        let href = self.selected_href()?;
        self.raw.iter().find(|r| r.href == href)
    }

    /// An allow rule by href, with its ruleset's href.
    pub fn find_rule(&self, href: &str) -> Option<(Rule, String)> {
        self.raw.iter().find_map(|rs| {
            rs.rules
                .iter()
                .find(|r| r.href == href)
                .map(|r| (r.clone(), rs.href.clone()))
        })
    }

    /// A deny rule by href (inline, or fetched from the fallback endpoint),
    /// with its ruleset's href.
    pub fn find_deny_rule(&self, href: &str) -> Option<(DenyRule, String)> {
        self.raw
            .iter()
            .find_map(|rs| {
                rs.deny_rules
                    .iter()
                    .find(|d| d.href == href)
                    .map(|d| (d.clone(), rs.href.clone()))
            })
            .or_else(|| {
                self.fallback_deny.iter().find_map(|(rs, list)| {
                    list.iter()
                        .find(|d| d.href == href)
                        .map(|d| (d.clone(), rs.clone()))
                })
            })
    }

    /// The rules of the loaded draft rulesets (allow, deny, fetched deny)
    /// that name the IP list `href` on either side; `None` until draft
    /// rulesets are loaded. Costs no request.
    pub fn ip_list_uses(&self, href: &str) -> Option<Vec<IpListUse>> {
        if self.scope != PolicyScope::Draft || self.loaded_at.is_none() {
            return None;
        }
        let names = |v: &[Actor]| {
            v.iter()
                .any(|a| a.ip_list.as_ref().is_some_and(|r| r.href == href))
        };
        let mut out = Vec::new();
        for rs in &self.raw {
            for r in &rs.rules {
                if names(&r.providers) || names(&r.consumers) {
                    out.push(IpListUse {
                        ruleset: rs.name.clone(),
                        kind: RuleKind::Allow,
                        providers: r.providers.clone(),
                        consumers: r.consumers.clone(),
                    });
                }
            }
            let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
                self.fallback_deny.get(&rs.href).map_or(&[], Vec::as_slice)
            } else {
                &rs.deny_rules
            };
            for d in deny {
                if names(&d.providers) || names(&d.consumers) {
                    out.push(IpListUse {
                        ruleset: rs.name.clone(),
                        kind: if d.override_ {
                            RuleKind::OverrideDeny
                        } else {
                            RuleKind::Deny
                        },
                        providers: d.providers.clone(),
                        consumers: d.consumers.clone(),
                    });
                }
            }
        }
        Some(out)
    }

    /// The selected ruleset has no inline deny rules and its endpoint list was
    /// neither fetched nor requested: the rules pane may be missing deny rules.
    pub fn deny_not_loaded(&self) -> bool {
        let Some(href) = self.selected_href() else {
            return false;
        };
        self.raw
            .iter()
            .find(|r| r.href == href)
            .is_some_and(|r| r.deny_rules.is_empty())
            && !self.fallback_deny.contains_key(&href)
            && !self.deny_requested.contains(&href)
    }

    /// The selected ruleset's href when its inline deny list is empty and the
    /// endpoint has not been asked yet. Marks it as requested.
    pub fn take_deny_fallback(&mut self) -> Option<String> {
        let href = self.selected_href()?;
        let inline_empty = self
            .raw
            .iter()
            .find(|r| r.href == href)
            .is_some_and(|r| r.deny_rules.is_empty());
        (inline_empty && self.deny_requested.insert(href.clone())).then_some(href)
    }
}

/// Limits copied from the resolved profile so `update` never touches config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub auto_refresh_min_seconds: u64,
    pub traffic_max_results: u32,
    pub traffic_default_window: String,
    /// After a truncated query below 10000, re-run it at 10000 without `B`.
    pub traffic_auto_backfill: bool,
    /// Label keys a ruleset scope is made of (`app`, `env`).
    pub scope_keys: Vec<String>,
    /// The label key that names a tier (`role`).
    pub tier_key: String,
    /// Label keys hidden from label text (columns, the `g` picker).
    pub ignore_keys: Vec<String>,
    /// `generator.ruleset_name`: the name of a ruleset the generator creates.
    pub ruleset_name: String,
    /// `generator.consolidate_min_peers`: inbound peers on one port that
    /// make it a fan-in candidate.
    pub consolidate_min_peers: usize,
    /// `events.exclude_types`: the event types the Events screen starts
    /// hiding.
    pub event_exclude_types: Vec<String>,
    /// `[exposure]`: the risky services `:exposure` looks for, or why the
    /// profile's list is malformed (the status line says it).
    pub exposure_services: Result<Vec<pce::traffic::RiskyService>, String>,
}

impl Limits {
    /// The resolved profile's limits and label model (startup's and a
    /// `:profile` switch's).
    pub fn from_config(cfg: &crate::config::ResolvedConfig) -> Self {
        Self {
            auto_refresh_min_seconds: cfg.auto_refresh_min_seconds,
            traffic_max_results: cfg.traffic_max_results,
            traffic_default_window: cfg.traffic_default_window.clone(),
            traffic_auto_backfill: cfg.traffic_auto_backfill,
            scope_keys: cfg.scope_keys.clone(),
            tier_key: cfg.tier_key.clone(),
            ignore_keys: cfg.ignore_keys.clone(),
            ruleset_name: cfg.generator_ruleset_name.clone(),
            consolidate_min_peers: cfg.generator_consolidate_min_peers,
            event_exclude_types: cfg.event_exclude_types.clone(),
            exposure_services: cfg.exposure_services.clone(),
        }
    }

    /// The Events type filter's starting text: `!user.login, !agent.*`.
    pub fn event_type_filter(&self) -> String {
        pce::models::exclude_text(&self.event_exclude_types)
    }

    /// The order label keys are shown in: `scope_keys`, `tier_key`, then
    /// the rest alphabetically; `ignore_keys` hidden.
    pub fn key_order(&self) -> KeyOrder {
        KeyOrder::new(&self.scope_keys, &self.tier_key).with_ignored(&self.ignore_keys)
    }
}

#[derive(Debug)]
pub struct Model {
    pub profile: ProfileInfo,
    pub theme_name: String,
    pub theme: Theme,
    pub limits: Limits,
    pub screen: Screen,
    pub overlay: Overlay,
    pub traffic: TrafficState,
    pub policy: PolicyState,
    pub objects: ObjectsState,
    pub workloads: WorkloadsState,
    pub events: EventsState,
    /// The app overlay's state while it is open (and after, until the next
    /// `A`: reopening rebuilds it).
    pub app_view: Option<AppView>,
    /// The open form (or the plan and confirm it led to) was opened from
    /// the app overlay: closing it returns there, not to the screen.
    pub form_from_app: bool,
    /// Explain's view while its `D` confirm is open: `n`/`Esc` go back to it.
    pub explain_back: Option<Box<crate::tui::explain::ExplainView>>,
    /// The last app overlay's sequence number: answers of older ones are dropped.
    pub app_seq: u64,
    /// The generator's state while it is open (and after, until the next
    /// `G` for another app: reopening the same app keeps it).
    pub gen_view: Option<crate::tui::gen_view::GenView>,
    /// The generator was opened from the app overlay: `Esc` returns there.
    pub gen_from_app: bool,
    /// The last generator's sequence number.
    pub gen_seq: u64,
    /// The `:marks` overlay's state while it is open.
    pub marks_view: Option<crate::tui::marks_view::MarksView>,
    /// Saved filters: the config file they live in and the `[name]` tags.
    pub saved: crate::tui::saved::SavedState,
    pub connection: Connection,
    pub stats: StatsSnapshot,
    pub size: (u16, u16),
    pub focused: bool,
    pub status: StatusLine,
    /// Set at startup, on every profile switch and on `:writes on|off` from
    /// the runtime's guard.
    pub writes: WritesState,
    /// First help line shown; `Overlay::Help` is reopened at the top.
    pub help_scroll: usize,
    /// The help's groups: the context page (`?`), one group (`Tab`), all (`a`).
    pub help_page: crate::tui::help::HelpPage,
    /// The overlay `?` was pressed in (the app view, the generator, an
    /// event's detail, pending changes, the marks): closing the help goes
    /// back to it, and the help opens on its keys.
    pub help_return: Overlay,
    /// Number of background tasks in flight; the runtime ticks at 250 ms while > 0.
    pub busy: usize,
    /// A `:profile` switch is being built off the event loop; a second one
    /// waits until it lands so two switches cannot race.
    pub switching: bool,
    /// A write batch is being sent (from `y` to `Msg::BatchDone`): quitting
    /// would drop a request on the wire unaudited, so it waits.
    pub batch_sending: bool,
    /// The last sent batch, step by step (`:batch` shows it again).
    pub batch_report: Option<Vec<String>>,
    /// The batch on its way (planned, confirmed, sent) came from the
    /// generator: its confirm, report and end return there.
    pub batch_from_gen: bool,
    /// The last bulk confirm's draft check (`d`); a late answer for an
    /// older one is dropped.
    pub bulk_check_seq: u64,
    /// `:readiness`'s state while it is open (and after, until reopened).
    pub readiness_view: Option<crate::tui::enforce::ReadinessView>,
    /// The enforcement overlay's state (kept for the same app).
    pub enforce_view: Option<crate::tui::enforce::EnforceView>,
    /// The last enforcement overlay's (and watch query's) sequence number.
    pub enforce_seq: u64,
    /// Where change records are kept (`…/illumio-tui/enforcements`); `None`
    /// without `$XDG_STATE_HOME` and `$HOME` (a move is refused). Set by the
    /// runtime; tests point it at a temp dir.
    pub records_dir: Option<std::path::PathBuf>,
    /// Where snapshots are kept (`…/illumio-tui/snapshots`, one directory
    /// per profile below); `None` without `$XDG_DATA_HOME` and `$HOME`
    /// (`:snapshot` is refused). Set by the runtime; tests point it at a
    /// temp dir.
    pub snapshots_root: Option<std::path::PathBuf>,
    /// A `:snapshot` is reading or writing: another waits for it.
    pub snapshot_saving: bool,
    /// A key on `:readiness` or the enforcement overlay opened something
    /// else (the generator, a form, a confirm): once nothing is open and no
    /// write is on its way, that overlay comes back.
    pub enforce_back: Option<Overlay>,
    /// Where `Esc` on the enforcement overlay returns: `:readiness`, the
    /// app overlay, or the screen (`Overlay::None`).
    pub enforce_return: Overlay,
    /// The open bulk confirm was planned by the enforcement overlay: `y`
    /// sends it with a change record.
    pub enforce_send: Option<Box<crate::tui::enforce::EnforceSend>>,
    pub quit: bool,
    pub started: Instant,
}

impl Model {
    pub fn new_connecting(
        profile: ProfileInfo,
        theme_name: String,
        auto_refresh_min_seconds: u64,
        traffic_max_results: u32,
        traffic_default_window: String,
        writes: WritesState,
    ) -> Self {
        let theme = Theme::from_name(&theme_name);
        let scope_keys: Vec<String> = crate::config::DEFAULT_SCOPE_KEYS.map(String::from).to_vec();
        let size = (80, 24);
        let mut traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
            &scope_keys,
        );
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(size);
        traffic.set_key_order(&KeyOrder::new(&scope_keys, crate::config::DEFAULT_TIER_KEY));
        traffic.marks_pce = PceId {
            host: profile.host.clone(),
            org: profile.org_id,
        };
        let policy = PolicyState::new(auto_refresh_min_seconds);
        let objects = ObjectsState::new(auto_refresh_min_seconds);
        let workloads = WorkloadsState::new(auto_refresh_min_seconds);
        let events = EventsState::new(auto_refresh_min_seconds, String::new());
        Self {
            profile,
            theme_name,
            theme,
            // The defaults; `app::run` and a profile switch set the profile's.
            limits: Limits {
                auto_refresh_min_seconds,
                traffic_max_results,
                traffic_default_window,
                traffic_auto_backfill: false,
                scope_keys,
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
                ignore_keys: Vec::new(),
                ruleset_name: pce::generate::DEFAULT_RULESET_NAME.into(),
                consolidate_min_peers: pce::generate::consolidate::DEFAULT_MIN_PEERS,
                event_exclude_types: Vec::new(),
                exposure_services: Ok(pce::traffic::default_services()),
            },
            screen: Screen::Traffic,
            overlay: Overlay::None,
            traffic,
            policy,
            objects,
            workloads,
            events,
            app_view: None,
            form_from_app: false,
            explain_back: None,
            app_seq: 0,
            gen_view: None,
            gen_from_app: false,
            gen_seq: 0,
            marks_view: None,
            saved: crate::tui::saved::SavedState::default(),
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
            size,
            focused: true,
            status: StatusLine::info("connecting…"),
            writes,
            help_scroll: 0,
            help_page: crate::tui::help::HelpPage::Context,
            help_return: Overlay::None,
            busy: 0,
            switching: false,
            batch_sending: false,
            batch_report: None,
            batch_from_gen: false,
            bulk_check_seq: 0,
            readiness_view: None,
            enforce_view: None,
            enforce_seq: 0,
            records_dir: None,
            snapshots_root: None,
            snapshot_saving: false,
            enforce_back: None,
            enforce_return: Overlay::None,
            enforce_send: None,
            quit: false,
            started: Instant::now(),
        }
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme_name = name.to_string();
        self.theme = Theme::from_name(name);
    }

    /// The profile's label model: the `w` form's dimensions, the keys
    /// every traffic view groups by until `g` changes them, and the keys
    /// label text hides.
    pub fn set_label_model(
        &mut self,
        scope_keys: Vec<String>,
        tier_key: String,
        ignore_keys: Vec<String>,
    ) {
        self.traffic.group_keys = keys_or_default(&scope_keys);
        self.traffic.all_keys = false;
        self.limits.scope_keys = scope_keys;
        self.limits.tier_key = tier_key;
        self.limits.ignore_keys = ignore_keys;
        let order = self.limits.key_order();
        self.traffic.set_key_order(&order);
        self.traffic.rebuild_views();
        self.workloads.group_keys = self.workload_group_keys();
        self.workloads.folded.clear();
        self.workloads.rebuild(&order);
        self.objects.hide_ignored = !self.limits.ignore_keys.is_empty();
        self.objects.rebuild_labels(&order);
    }

    /// The keys `x` groups the workloads by: the traffic views' (`g`),
    /// else the profile's scope keys (`app,env` without any).
    pub fn workload_group_keys(&self) -> Vec<String> {
        let keys = self.traffic.effective_keys();
        if keys.is_empty() {
            keys_or_default(&self.limits.scope_keys)
        } else {
            keys
        }
    }

    /// The label filter's unknown keys and values again, after the
    /// workloads or the label list changed.
    pub fn refresh_label_warnings(&mut self) {
        let idx = self.index().cloned();
        let w = &mut self.workloads;
        w.label_warnings = w.unknown_labels(&w.labels, idx.as_deref());
    }

    /// Whether the Labels tab's usage can be counted: the workloads, the
    /// rulesets and the label groups are all cached.
    pub fn usage_ready(&self) -> bool {
        self.workloads.loaded_at.is_some()
            && self.policy.loaded_at.is_some()
            && self.objects.ages[ObjectKind::LabelGroups.index()].is_some()
    }

    /// Counts label usage from what is cached (none until `usage_ready`)
    /// and rebuilds the Labels tab. No request.
    pub fn recount_labels(&mut self) {
        self.objects.label_usage = self.usage_ready().then(|| {
            pce::label_usage(
                &self.workloads.raw,
                &self.policy.raw,
                self.policy.fallback_deny.values().flatten(),
                self.objects.groups.rows(),
            )
        });
        self.objects.usage_truncated = self.workloads.truncated;
        self.objects.rebuild_labels(&self.limits.key_order());
    }

    /// The loaded draft rulesets: the Policy screen's list when it shows
    /// the draft, else the ones a `w` form loaded; `None` while neither is.
    pub fn draft_rulesets(&self) -> Option<&[RuleSet]> {
        if self.policy.scope == PolicyScope::Draft {
            self.policy.loaded_at.map(|_| self.policy.raw.as_slice())
        } else {
            self.policy.form_drafts.as_deref()
        }
    }

    /// What a `w` form is pre-filled from: the profile's label model and
    /// the loaded draft rulesets ([`Self::draft_rulesets`]).
    pub fn flow_prefill(&self) -> crate::tui::edit::FlowPrefill<'_> {
        crate::tui::edit::FlowPrefill {
            scope_keys: &self.limits.scope_keys,
            tier_key: &self.limits.tier_key,
            rulesets: self.draft_rulesets(),
            group_keys: None,
        }
    }

    /// An open allow rule form's Scope notes: what its consumers mean and
    /// the warnings ([`crate::tui::edit::scope_notes`]) against its
    /// ruleset's scopes as loaded. No request.
    pub fn rule_scope_notes(&self, form: &crate::tui::form::Form) -> Option<(String, Vec<String>)> {
        form.rule_scope.as_ref()?;
        let empty = pce::NameIndex::default();
        let idx = self.index().map_or(&empty, |i| i.as_ref());
        let loaded = self
            .policy
            .loaded_at
            .map_or(&[][..], |_| &self.policy.raw[..]);
        let (scopes, groups) =
            crate::tui::edit::scope_inputs(form, loaded, self.draft_rulesets(), idx);
        crate::tui::edit::scope_notes(form, scopes.as_deref(), &groups)
    }

    /// An allow rule form's Scope switch brought up to date before a key
    /// reaches it ([`crate::tui::edit::refresh_rule_scope`]). No request.
    pub fn refresh_rule_scope(&self, form: &mut crate::tui::form::Form) {
        let empty = pce::NameIndex::default();
        let idx = self.index().map_or(&empty, |i| i.as_ref());
        let loaded = self
            .policy
            .loaded_at
            .map_or(&[][..], |_| &self.policy.raw[..]);
        crate::tui::edit::refresh_rule_scope(form, loaded, self.draft_rulesets(), idx);
    }

    /// What names hrefs in an event detail: the index and whichever
    /// rulesets are loaded (draft and active share names). No request.
    pub fn event_names(&self) -> crate::tui::event_diff::Names<'_> {
        let loaded = self.policy.loaded_at.map(|_| self.policy.raw.as_slice());
        crate::tui::event_diff::Names {
            idx: self.index().map(|i| i.as_ref()),
            rule_sets: self.draft_rulesets().or(loaded).unwrap_or(&[]),
            local: Default::default(),
        }
    }

    pub fn index(&self) -> Option<&std::sync::Arc<pce::NameIndex>> {
        match &self.connection {
            Connection::Connected { index, .. } => index.as_ref(),
            _ => None,
        }
    }

    /// Rebuilds the rules pane from what is loaded, its `P/C` column from
    /// the cached workloads (`-` without them). No request.
    pub fn rebuild_rules(&mut self) {
        let Some(idx) = self.index().cloned() else {
            return;
        };
        let workloads = self.workloads.impact_source();
        self.policy.rebuild_rules(&idx, workloads);
    }

    /// Evaluates the open confirm's impact line into `policy.confirm_impact`
    /// (`None` unless a rule write is on screen). No request.
    pub fn refresh_confirm_impact(&mut self) {
        self.policy.confirm_impact = match &self.overlay {
            Overlay::ConfirmWrite { plan, .. } => crate::tui::impact::confirm_line(self, plan),
            _ => None,
        };
    }

    /// Whether any screen has opted in to auto-refresh (drives the idle tick).
    pub fn any_auto_enabled(&self) -> bool {
        self.traffic.auto.enabled
            || self.policy.auto.enabled
            || self.objects.auto.enabled
            || self.objects.ip_lists.auto.enabled
            || self.workloads.auto.enabled
            || self.events.auto.enabled
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, Connection::Connected { .. })
    }

    /// Forgets everything loaded from the previous PCE: a running query is
    /// cancelled, every screen starts empty with auto-refresh off, counters and
    /// overlay are cleared. The traffic filter returns to the defaults in `limits`.
    pub fn reset_data(&mut self) {
        if let QueryState::Running { cancel, .. } = &self.traffic.query {
            cancel.cancel();
        }
        let min = self.limits.auto_refresh_min_seconds;
        let mut traffic = TrafficState::new(
            &self.limits.traffic_default_window,
            self.limits.traffic_max_results,
            min,
            &self.limits.scope_keys,
        );
        traffic.set_key_order(&self.limits.key_order());
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(self.size);
        // The marks file and `M` are the session's; their PCE the new profile's.
        traffic.marks = std::mem::replace(&mut self.traffic.marks, MarksStore::none());
        traffic.marks_announced = self.traffic.marks_announced;
        traffic.show_squelched = self.traffic.show_squelched;
        traffic.marks_pce = PceId {
            host: self.profile.host.clone(),
            org: self.profile.org_id,
        };
        self.traffic = traffic;
        self.policy = PolicyState::new(min);
        self.objects = ObjectsState::new(min);
        self.objects.hide_ignored = !self.limits.ignore_keys.is_empty();
        self.workloads = WorkloadsState::new(min);
        self.events = EventsState::new(min, self.limits.event_type_filter());
        self.app_view = None;
        self.form_from_app = false;
        self.gen_view = None;
        self.gen_from_app = false;
        self.readiness_view = None;
        self.enforce_view = None;
        self.enforce_back = None;
        self.enforce_return = Overlay::None;
        self.enforce_send = None;
        self.saved.applied = crate::tui::saved::Applied::default();
        self.stats = StatsSnapshot { calls: 0, bytes: 0 };
        self.busy = 0;
        self.overlay = Overlay::None;
        self.writes.in_flight = false;
    }
}

#[cfg(test)]
mod focus_tests;
#[cfg(test)]
mod tests;
