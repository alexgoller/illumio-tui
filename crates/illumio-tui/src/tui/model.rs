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
use pce::write::Plan;
use pce::{
    IpListSize, KeyOrder, LabelFilter, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot,
    ip_list_size,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

mod events;
pub use events::*;
mod workloads;
pub use workloads::*;
mod objects;
pub use objects::*;
mod policy;
pub use policy::*;
mod traffic;
pub use traffic::*;
mod flows;
pub use flows::*;

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
    FilterEditor(FilterEditor),
    /// `f` on Workloads: the label filter, one line.
    LabelFilter(LabelFilterEditor),
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
    /// Pending (unprovisioned) changes; the list lives in `PolicyState::pending`.
    Pending,
    /// A failure too long for the status line, or an authentication failure.
    Error(String),
    /// Events filter: severity, type (local), window.
    EventsEditor {
        values: [String; 3],
        active: usize,
        error: Option<String>,
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
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Overlay::None, Overlay::None)
            | (Overlay::Help, Overlay::Help)
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App)
            | (Overlay::Generator, Overlay::Generator)
            | (Overlay::Marks, Overlay::Marks) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b))
            | (Overlay::LabelKeys(a), Overlay::LabelKeys(b)) => a == b,
            (Overlay::FilterEditor(a), Overlay::FilterEditor(b)) => a == b,
            (Overlay::LabelFilter(a), Overlay::LabelFilter(b)) => a == b,
            (Overlay::AppPicker(a), Overlay::AppPicker(b)) => a == b,
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
            (
                Overlay::EventsEditor {
                    values: va,
                    active: aa,
                    error: ea,
                },
                Overlay::EventsEditor {
                    values: vb,
                    active: ab,
                    error: eb,
                },
            ) => va == vb && aa == ab && ea == eb,
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
    /// `r` in the app overlay: the workloads and the rulesets again, past the cache.
    ReloadApp,
    /// `r` in the generator on incomplete data, past a cost threshold: the
    /// focused query for its app; `y` and `n` go back to the generator.
    GenRequery(TrafficFilter, pce::traffic::Focus),
    /// `x` on `:marks`: this mark leaves `marks.toml`; `y` and `n` go back
    /// to the overlay. A local file only.
    DeleteMark(String),
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
            pending_marked: BTreeSet::new(),
            provision_counts: None,
            provisioned: None,
            revert_typed: String::new(),
            rule_after_ruleset: None,
            write_from_flow: false,
            form_drafts: None,
            form_drafts_loading: false,
            form_drafts_stale: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Whether the focused pane's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
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
        rows.sort_by_key(|r| rule_rank(r.kind));
        self.rules.set_rows(rows);
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
        }
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
        let events = EventsState::new(auto_refresh_min_seconds);
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
            app_seq: 0,
            gen_view: None,
            gen_from_app: false,
            gen_seq: 0,
            marks_view: None,
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
        self.events = EventsState::new(min);
        self.app_view = None;
        self.form_from_app = false;
        self.gen_view = None;
        self.gen_from_app = false;
        self.stats = StatsSnapshot { calls: 0, bytes: 0 };
        self.busy = 0;
        self.overlay = Overlay::None;
        self.writes.in_flight = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_for_judges_the_result_actually_used() {
        use crate::tui::test_support::model_with_flows;
        use pce::traffic::{FocusDirection, parse_app};
        let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
            .with_direction(FocusDirection::In);
        let mut m = model_with_flows();
        assert_eq!(m.traffic.complete_for(&shop), Ok(()));
        m.traffic.data.as_mut().unwrap().incomplete = true;
        assert_eq!(
            m.traffic.complete_for(&shop),
            Err(Gap::Region { focused: false })
        );
        m.traffic.data.as_mut().unwrap().incomplete = false;
        m.traffic.data.as_mut().unwrap().truncated = true;
        assert_eq!(
            m.traffic.complete_for(&shop),
            Err(Gap::Capped {
                cap: 1000,
                focused: false
            })
        );
        // A focused result for the app, itself truncated: partial_for
        // calls it whole, complete_for does not.
        let data = TrafficData {
            flows: m.traffic.loaded_flows().to_vec(),
            truncated: true,
            incomplete: false,
            elapsed: Duration::ZERO,
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        m.traffic.apply_focused(shop.clone(), 1, data);
        assert!(!m.traffic.partial_for(&shop));
        assert_eq!(
            m.traffic.complete_for(&shop),
            Err(Gap::Capped {
                cap: 1000,
                focused: true
            })
        );
        // The out side is not covered by an `in` result: the unfocused one counts.
        let out = shop.clone().with_direction(FocusDirection::Out);
        assert_eq!(
            m.traffic.complete_for(&out),
            Err(Gap::Capped {
                cap: 1000,
                focused: false
            })
        );
        let empty = crate::tui::test_support::model_fixture();
        assert_eq!(empty.traffic.complete_for(&shop), Err(Gap::NoData));
        // A focused `in` result as the only result: the out side is not loaded.
        let mut only = crate::tui::test_support::model_fixture();
        let data = TrafficData {
            flows: m.traffic.loaded_flows().to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::ZERO,
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        only.traffic.apply_focused(shop.clone(), 1, data);
        assert!(only.traffic.unfocused.is_none());
        assert_eq!(only.traffic.complete_for(&out), Err(Gap::Uncovered));
        assert!(
            only.traffic.partial_for(&out),
            "nothing covers the out side"
        );
    }

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn events_window_is_whole_minutes_and_open_ended_so_the_cache_can_hit() {
        let edit = EventsFilterEdit {
            severity: " err ".into(),
            event_type: "agent".into(),
            window: "24h".into(),
        };
        let a = edit.to_filter(at("2026-10-10T08:00:07Z")).unwrap();
        let b = edit.to_filter(at("2026-10-10T08:00:52Z")).unwrap();
        assert_eq!(a.to_query(), b.to_query(), "same minute, same query");
        assert_eq!(
            a.to_query(),
            vec![
                ("severity", "err".to_string()),
                ("timestamp[gte]", "2026-10-09T08:00:00Z".to_string()),
            ],
            "severity trimmed, `until` left open, the local type not sent"
        );
        let c = edit.to_filter(at("2026-10-10T08:01:00Z")).unwrap();
        assert_ne!(
            a.to_query(),
            c.to_query(),
            "the next minute moves the window"
        );
    }

    #[test]
    fn events_empty_severity_is_none_and_a_bad_window_is_an_error() {
        let mut edit = EventsFilterEdit {
            severity: "  ".into(),
            event_type: String::new(),
            window: "1h".into(),
        };
        assert!(
            edit.to_filter(at("2026-10-10T08:00:00Z"))
                .unwrap()
                .severity
                .is_none()
        );
        edit.window = "soon".into();
        assert!(
            edit.to_filter(at("2026-10-10T08:00:00Z"))
                .unwrap_err()
                .starts_with("window:")
        );
    }

    #[test]
    fn an_event_without_a_timestamp_renders_blank() {
        let e: Event = serde_json::from_str(r#"{"href":"/orgs/5/events/x"}"#).unwrap();
        for c in event_columns() {
            assert!((c.cell)(&e).len() < 5, "{}", c.title);
        }
        assert_eq!((event_columns()[0].cell)(&e), "");
    }

    #[test]
    fn rules_are_found_by_href_inline_or_in_the_deny_fallback() {
        let mut p = PolicyState::new(60);
        p.raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let (r, rs) = p
            .find_rule("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2")
            .unwrap();
        assert_eq!(r.description.as_deref(), Some("corp to web"));
        assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/100");
        let (d, _) = p
            .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
            .unwrap();
        assert!(d.override_);
        assert!(
            p.find_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
                .is_none()
        );
        let fetched: DenyRule = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
            "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 23, "proto": 6}]
        }))
        .unwrap();
        p.fallback_deny.insert(
            "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            vec![fetched],
        );
        let (_, rs) = p
            .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9")
            .unwrap();
        assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/101");
    }

    #[test]
    fn the_flow_and_blocked_columns_fit_80_columns_with_the_group_columns() {
        use crate::tui::ui::widgets::table::visible_columns;
        let row_width = |cols: &[(usize, u16, bool)]| -> u16 {
            cols.iter().map(|c| c.1).sum::<u16>() + cols.len() as u16 - 1 + 2
        };
        let flows = flow_columns();
        let at80 = visible_columns(&flows, 80);
        let titles: Vec<&str> = at80.iter().map(|c| flows[c.0].title).collect();
        assert_eq!(
            titles,
            vec![
                "src",
                "src group",
                "dst",
                "dst group",
                "service",
                "conns",
                "decision"
            ]
        );
        assert!(
            row_width(&at80) <= 78,
            "inside the border at 80: {}",
            row_width(&at80)
        );
        assert!(row_width(&visible_columns(&flows, 120)) <= 118);
        let at160 = visible_columns(&flows, 160);
        assert_eq!(flows[at160[0].0].title, "last seen");
        assert!(row_width(&at160) <= 158);
        let blocked = blocked_columns();
        let titles: Vec<&str> = blocked.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec!["src", "dst name", "service", "flows", "conns", "last seen"]
        );
        // A fill column (width 0) gets at least 8 cells.
        let fill_width = |cols: &[(usize, u16, bool)]| -> u16 {
            cols.iter()
                .map(|c| if c.1 == 0 { 8 } else { c.1 })
                .sum::<u16>()
                + cols.len() as u16
                - 1
                + 2
        };
        let at80 = visible_columns(&blocked, 80);
        assert!(fill_width(&at80) <= 78 - 16, "room for the sources at 80");
        let apps = app_columns();
        let at80 = visible_columns(&apps, 80);
        let titles: Vec<&str> = at80.iter().map(|c| apps[c.0].title).collect();
        assert_eq!(
            titles,
            vec!["peer", "service", "flows", "conns", "bytes", "decision"]
        );
        assert!(fill_width(&at80) <= 78 - 16, "room for the peer at 80");
        let at160 = visible_columns(&apps, 160);
        assert_eq!(apps[at160[1].0].title, "hosts");
    }

    fn rich_sections(direction: Direction, decisions: Decisions, arrows: bool) -> Vec<SectionRow> {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let keys = vec!["app".to_string(), "env".to_string()];
        let sections = if arrows {
            app_sections_both(&flows, &keys, decisions)
        } else {
            app_sections(&flows, &keys, direction, decisions)
        };
        section_rows(&sections, &BTreeSet::new(), arrows)
    }

    #[test]
    fn blocked_cells_name_one_host_or_count_them() {
        let rows = rich_sections(Direction::In, Decisions::Blocked, false);
        let cols = blocked_columns();
        let header = rows
            .iter()
            .find(|r| r.is_header() && r.app == vec!["shop", "prod"])
            .unwrap();
        assert_eq!(
            header.header_line().unwrap(),
            "▾ shop/prod  in · 3 groups · 24 conns · 2 blocked · 1 pot.blocked"
        );
        assert!(
            cols.iter()
                .all(|c| (c.cell)(header).is_empty() || c.title == "src")
        );
        let peers: Vec<Vec<String>> = rows
            .iter()
            .filter(|r| !r.is_header() && r.app == vec!["shop", "prod"])
            .map(|r| cols.iter().map(|c| (c.cell)(r)).collect())
            .collect();
        assert_eq!(peers.len(), 3);
        assert!(
            peers
                .iter()
                .all(|p| p[1] == "shop-db-p.example" && p[2] == "5432/tcp")
        );
        let srcs: BTreeSet<&str> = peers.iter().map(|p| p[0].as_str()).collect();
        assert_eq!(
            srcs,
            BTreeSet::from(["  hr/prod", "  shop/dev", "  ipl:Corp/-"]),
            "indented sources"
        );
        let mut many = rows
            .iter()
            .find(|r| !r.is_header() && r.app == vec!["shop", "prod"])
            .unwrap()
            .clone();
        if let RowKind::Peer { row, .. } = &mut many.kind {
            row.app_hosts = 8;
            row.app_names = vec!["a".into(), "b".into()];
        }
        assert_eq!((cols[1].cell)(&many), "8 hosts");
    }

    #[test]
    fn both_rows_mark_the_direction_and_share_one_header_per_app() {
        let rows = rich_sections(Direction::In, Decisions::All, true);
        let header = &rows[0];
        let line = header.header_line().unwrap();
        assert_eq!(
            line,
            "▾ shop/prod  ← 1545 conns · 2 blocked · 1 pot.blocked  → 1004 conns"
        );
        let peers: Vec<String> = rows
            .iter()
            .skip(1)
            .take_while(|r| !r.is_header())
            .map(|r| (app_columns()[0].cell)(r))
            .collect();
        assert!(peers.iter().any(|p| p.starts_with("  ← ")), "{peers:?}");
        assert!(peers.iter().any(|p| p.starts_with("  → ")), "{peers:?}");
        let first_out = peers.iter().position(|p| p.starts_with("  → ")).unwrap();
        assert!(
            peers[first_out..].iter().all(|p| p.starts_with("  → ")),
            "inbound rows, then outbound"
        );
        let headers = rows.iter().filter(|r| r.is_header()).count();
        let apps: BTreeSet<&Vec<String>> = rows.iter().map(|r| &r.app).collect();
        assert_eq!(headers, apps.len());
    }

    #[test]
    fn a_folded_section_keeps_its_header_and_search_keeps_whole_sections() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let keys = vec!["app".to_string(), "env".to_string()];
        let sections = app_sections(&flows, &keys, Direction::In, Decisions::All);
        let folded = BTreeSet::from([vec!["crm".to_string(), "prod".to_string()]]);
        let rows = section_rows(&sections, &folded, false);
        assert!(rows[0].header_line().unwrap().starts_with("▹ crm/prod"));
        assert!(rows[1].is_header(), "no peers under a folded header");
        let mut t = TableView::new(app_columns()).with_matcher(section_matches);
        t.set_rows(section_rows(&sections, &BTreeSet::new(), false));
        let all = t.len();
        t.start_search();
        for c in "api.partner".chars() {
            t.search_push(c);
        }
        assert!(t.len() < all && !t.is_empty());
        let shown: Vec<&SectionRow> = t.visible().iter().map(|&i| &t.rows()[i]).collect();
        let app = &shown[0].app;
        assert!(shown[0].is_header());
        assert!(shown.iter().all(|r| &r.app == app), "one whole section");
        assert!(shown.len() > 2, "with its rows that do not match too");
    }

    #[test]
    fn mesh_columns_follow_the_toggles_and_fit_80_columns() {
        use crate::tui::ui::widgets::table::visible_columns;
        let titles = |cols: MeshColumns| -> Vec<&'static str> {
            mesh_columns(cols).iter().map(|c| c.title).collect()
        };
        assert_eq!(
            titles(MESH_ALL),
            vec![
                "user", "process", "src", "dst", "service", "dst proc", "conns", "decision"
            ]
        );
        assert_eq!(
            titles(MeshColumns::default()),
            vec!["src", "dst", "service", "dst proc", "conns", "decision"]
        );
        let cols = mesh_columns(MESH_ALL);
        let at80 = visible_columns(&cols, 80);
        let width: u16 = at80.iter().map(|c| c.1).sum::<u16>() + at80.len() as u16 - 1 + 2;
        assert!(width <= 78, "{width}");
        assert!(
            !at80.iter().any(|c| cols[c.0].title == "dst proc"),
            "from 120 columns"
        );
        let at160 = visible_columns(&cols, 160);
        let width: u16 = at160.iter().map(|c| c.1).sum::<u16>() + at160.len() as u16 - 1 + 2;
        assert!(width <= 158, "{width}");
        let rows = mesh(
            &crate::tui::test_support::fixture_flows_rich(),
            &["app".to_string(), "env".to_string()],
            MESH_ALL,
        );
        let pb = rows
            .iter()
            .find(|r| r.dominant() == pce::models::PolicyDecision::PotentiallyBlocked)
            .unwrap();
        assert_eq!((cols[7].cell)(pb), "pot.blocked");
        assert_eq!((cols[7].narrow.unwrap())(pb), "pot.blk");
        let icmp = rows.iter().find(|r| r.proto == "icmp").unwrap();
        assert_eq!((cols[1].cell)(icmp), "-", "no process");
    }

    #[test]
    fn workload_labels_column_follows_the_key_order_and_sorts_by_it() {
        let mut m = crate::tui::test_support::model_with_workloads();
        m.set_label_model(vec!["env".into()], "role".into(), Vec::new());
        // host-1 sorts first alphabetically (`app=aaa`), host-2 by the order (`role=db`).
        m.workloads.raw[0].labels[1].value = Some("aaa".into());
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t.columns.iter().position(|c| c.title == "labels").unwrap();
        assert_eq!(t.cell(&t.rows()[0], col), "env=prod role=web app=aaa");
        assert_eq!(t.rows()[0].labels, "env=prod role=web app=aaa");
        m.workloads.table.sort_by(col, false);
        let t = &m.workloads.table;
        let sorted: Vec<&str> = t
            .visible()
            .iter()
            .map(|&i| t.rows()[i].labels.as_str())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(
            sorted,
            vec!["env=prod role=db app=shop", "env=prod role=web app=aaa"]
        );
    }

    fn shown_hosts(m: &Model) -> Vec<String> {
        let t = &m.workloads.table;
        t.visible()
            .iter()
            .map(|&i| t.rows()[i].workload.display_name())
            .collect()
    }

    #[test]
    fn workload_filters_combine_and_name_themselves_with_the_count() {
        let mut m = crate::tui::test_support::model_with_workloads();
        let order = m.limits.key_order();
        let w = &mut m.workloads;
        assert_eq!(w.filter_line(), None, "nothing active: no line");
        // E: all → full → selective → visibility_only → idle → unmanaged → all.
        let mut seen = Vec::new();
        for _ in 0..6 {
            w.filters.next_enforcement();
            seen.push(w.filters.enforcement);
        }
        assert_eq!(
            seen,
            [
                Some("full"),
                Some("selective"),
                Some("visibility_only"),
                Some("idle"),
                Some("unmanaged"),
                None
            ]
        );
        // m: all → managed → unmanaged → all.
        let mut seen = Vec::new();
        for _ in 0..3 {
            w.filters.next_managed();
            seen.push(w.filters.managed);
        }
        assert_eq!(seen, [Some(true), Some(false), None]);

        w.filters.enforcement = Some("unmanaged");
        w.rebuild(&order);
        assert_eq!(shown_hosts(&m), ["unmanaged-nas"], "effective: no VEN");
        let w = &mut m.workloads;
        w.filters.enforcement = Some("idle");
        w.rebuild(&order);
        assert!(
            shown_hosts(&m).is_empty(),
            "the nas reports idle, but has no VEN"
        );

        let w = &mut m.workloads;
        w.filters = WorkloadFilters {
            unlabeled: true,
            ..Default::default()
        };
        w.rebuild(&order);
        assert_eq!(shown_hosts(&m), ["unmanaged-nas", "host-4.example"]);
        let w = &mut m.workloads;
        w.filters.managed = Some(true);
        w.rebuild(&order);
        assert_eq!(shown_hosts(&m), ["host-4.example"]);
        assert_eq!(
            m.workloads.filter_line().as_deref(),
            Some("managed · unlabeled (app, env, role) · 1 of 4")
        );

        let w = &mut m.workloads;
        w.filters = WorkloadFilters {
            offline: true,
            ..Default::default()
        };
        w.unhealthy_only = true;
        w.rebuild(&order);
        assert_eq!(shown_hosts(&m), ["host-2.example"]);
        assert_eq!(
            m.workloads.filter_line().as_deref(),
            Some("unhealthy · offline · 1 of 4")
        );
        // Search narrows the shown count too.
        m.workloads.table.start_search();
        m.workloads.table.search_push('z');
        m.workloads.table.search_commit();
        assert_eq!(
            m.workloads.filter_line().as_deref(),
            Some("unhealthy · offline · 0 of 4")
        );
        m.workloads.truncated = true;
        assert_eq!(
            m.workloads.filter_line().as_deref(),
            Some("unhealthy · offline · 0 of 4 · partial: the list is truncated")
        );
        m.workloads.unhealthy_only = false;
        m.workloads.filters = WorkloadFilters {
            enforcement: Some("visibility_only"),
            managed: Some(false),
            ..Default::default()
        };
        m.workloads.table.search_clear();
        m.workloads.rebuild(&order);
        assert_eq!(
            m.workloads.filter_line().as_deref(),
            Some(
                "enforcement visibility_only · unmanaged · 0 of 4 · partial: the list is truncated"
            )
        );
    }

    #[test]
    fn workload_missing_column_names_the_absent_model_keys() {
        let mut m = crate::tui::test_support::model_with_workloads();
        m.set_label_model(vec!["env".into()], "role".into(), Vec::new());
        m.workloads.raw[0]
            .labels
            .retain(|l| l.key.as_deref() != Some("role"));
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t.columns.iter().position(|c| c.title == "missing").unwrap();
        let cells: Vec<String> = t.rows().iter().map(|r| t.cell(r, col)).collect();
        assert_eq!(cells, ["role", "", "env, role", "env, role"]);
        // `:export` joins them as every list cell: `; `.
        assert_eq!(t.full_cells(&t.rows()[2])[col], "env; role");
        // Hidden at ordinary widths unless `u` is on.
        assert!(t.columns[col].hide_below > 180);
        m.workloads.filters.unlabeled = true;
        m.workloads.rebuild(&m.limits.key_order());
        assert_eq!(m.workloads.table.columns[col].hide_below, 0);
    }

    /// 10k workloads, grouped, with a label filter and a search: a
    /// rebuild and a regrouping per search key stay one pass each (well
    /// under the generous bound even unoptimised; a guard against a
    /// quadratic step, not a timing gate).
    #[test]
    fn grouped_workloads_at_10k_rebuild_and_search_quickly() {
        let mut m = crate::tui::test_support::model_with_workloads();
        let base = m.workloads.raw.clone();
        m.workloads.raw = (0..10_000)
            .map(|i| {
                let mut w = base[i % 2].clone();
                w.href = format!("/w/{i}");
                w.hostname = Some(format!("host-{i}.example"));
                w.labels[1].value = Some(format!("app{}", i % 300));
                w
            })
            .collect();
        let order = m.limits.key_order();
        let start = std::time::Instant::now();
        m.workloads.toggle_grouped(vec!["app".into(), "env".into()]);
        m.workloads.rebuild(&order);
        assert_eq!(m.workloads.groups.len(), 300);
        let labels = LabelFilter::parse("app=app1,app2,app3 !role=db").unwrap();
        m.workloads.set_labels(labels, None, &order);
        // Odd workloads are role=db: app1 and app3 are all excluded.
        assert_eq!(m.workloads.groups.len(), 1);
        m.workloads.set_labels(LabelFilter::default(), None, &order);
        m.workloads.table.start_search();
        for c in "host-99".chars() {
            m.workloads.table.search_push(c);
            m.workloads.regroup();
        }
        let took = start.elapsed();
        assert!(m.workloads.table.len() < 200);
        assert!(took < Duration::from_secs(20), "{took:?}");
    }

    #[test]
    fn workload_visibility_is_a_dash_without_a_ven() {
        let mut m = crate::tui::test_support::model_with_workloads();
        for w in &mut m.workloads.raw {
            w.visibility_level = Some("flow_summary".into());
        }
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t
            .columns
            .iter()
            .position(|c| c.title == "visibility")
            .unwrap();
        let cells: Vec<String> = t.rows().iter().map(|r| t.cell(r, col)).collect();
        let nas = t
            .rows()
            .iter()
            .position(|r| r.workload.is_unmanaged())
            .unwrap();
        assert_eq!(cells[nas], "-");
        assert_eq!(cells[0], "flow_summary");
    }

    #[test]
    fn workload_labels_column_hides_ignored_keys() {
        let mut m = crate::tui::test_support::model_with_workloads();
        m.set_label_model(vec!["app".into()], "role".into(), vec!["env".into()]);
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t.columns.iter().position(|c| c.title == "labels").unwrap();
        assert_eq!(t.cell(&t.rows()[0], col), "app=shop role=web");
        assert!(
            m.workloads.raw[0]
                .labels
                .iter()
                .any(|l| l.key.as_deref() == Some("env")),
            "the workload itself keeps the label"
        );
    }

    #[test]
    fn limits_from_config_carry_the_ignored_keys() {
        let file: crate::config::ConfigFile = toml::from_str(
            r#"
[profiles.default]
host = "h"
org_id = 1
api_key = "k"
api_secret = "s"

[profiles.default.labels]
ignore_keys = ["os", "app"]
"#,
        )
        .unwrap();
        let cfg = crate::config::resolve(
            &file,
            &crate::config::Overrides::default(),
            &Default::default(),
        )
        .unwrap();
        let limits = Limits::from_config(&cfg);
        assert_eq!(limits.ignore_keys, vec!["os"], "app is a scope key");
        assert_eq!(limits.ruleset_name, "RF-{app}-{env}");
        assert_eq!(limits.key_order().ignored(), vec!["os"]);
        let mut m = crate::tui::test_support::model_fixture();
        m.limits = limits;
        m.reset_data();
        assert!(!m.traffic.key_order.shows("os"));
    }

    #[test]
    fn the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env() {
        let mut m = crate::tui::test_support::model_fixture();
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert_eq!(m.traffic.key_order.first(), vec!["app", "env", "role"]);
        m.set_label_model(
            vec!["site".into(), "zone".into()],
            "tier".into(),
            vec!["os".into()],
        );
        assert_eq!(m.traffic.group_keys, vec!["site", "zone"]);
        assert_eq!(m.limits.tier_key, "tier");
        assert_eq!(m.traffic.key_order.first(), vec!["site", "zone", "tier"]);
        assert_eq!(m.traffic.key_order.ignored(), vec!["os"]);
        m.reset_data();
        assert_eq!(
            m.traffic.key_order.first(),
            vec!["site", "zone", "tier"],
            "a profile switch's reset keeps the profile's order"
        );
        assert_eq!(m.traffic.key_order.ignored(), vec!["os"]);
        m.set_label_model(Vec::new(), "role".into(), Vec::new());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert_eq!(m.traffic.key_order.first(), vec!["role"]);
        assert!(
            m.limits.scope_keys.is_empty(),
            "the profile value itself is kept"
        );
    }

    #[test]
    fn ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them() {
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let mut s = IpListsState::new(60);
        let mut p = PolicyState::new(60);
        s.set_lists(lists, &p);
        assert_eq!(s.table.rows()[1].size.display(), "65.5K");
        assert_eq!(s.table.rows()[1].used_by, None, "no rulesets loaded");
        p.raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        p.loaded_at = Some(Instant::now());
        s.recount(&p);
        let used: Vec<Option<usize>> = s.table.rows().iter().map(|r| r.used_by).collect();
        assert_eq!(used, vec![Some(1), Some(1)], "deny rule 7 and allow rule 2");
        let uses = p
            .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21")
            .unwrap();
        assert_eq!(uses[0].ruleset, "Shop-Prod");
        assert_eq!(uses[0].kind, RuleKind::Allow);
        p.scope = PolicyScope::Active;
        s.recount(&p);
        assert_eq!(s.table.rows()[0].used_by, None, "only draft rulesets count");
        let cols = ip_list_columns();
        let titles: Vec<&str> = cols.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec!["name", "size", "ranges", "fqdns", "used by", "pending"]
        );
        let rows = s.table.rows();
        assert_eq!((cols[1].cell)(&rows[0]), "any");
        assert_eq!((cols[1].cell)(&rows[1]), "65.5K");
        assert_eq!((cols[2].cell)(&rows[0]), "2");
        assert_eq!((cols[3].cell)(&rows[1]), "1");
        assert_eq!((cols[4].cell)(&rows[0]), "-");
        assert!(
            (cols[1].key)(&rows[0]) > (cols[1].key)(&rows[1]),
            "any sorts last"
        );
        assert_eq!(Screen::ALL.len(), 5);
        assert_eq!(Screen::Events.index(), 4);
        assert_eq!(Screen::Objects.title(), "Objects");
        assert_eq!(ObjectKind::ALL.len(), 4);
        assert_eq!(ObjectKind::IpLists.title(), "IP lists");
        assert_eq!(ObjectKind::IpLists.index(), 3);
        assert_eq!(ObjectKind::Labels.prev(), ObjectKind::IpLists);
        assert_eq!(ObjectKind::Services.next(), ObjectKind::IpLists);
        assert_eq!(ObjectKind::IpLists.next(), ObjectKind::Labels);
        assert!(!ObjectKind::FETCHED.contains(&ObjectKind::IpLists));
    }

    #[test]
    fn ip_list_uses_match_the_provider_side_and_fetched_deny_rules() {
        let mut sets: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let deny_on_20 = sets[0]["deny_rules"][0].clone();
        let mut edge = sets[1].clone();
        edge["name"] = "Edge".into();
        edge["href"] = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        edge["rules"][0]["providers"] =
            serde_json::json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]);
        sets.push(edge);
        let mut p = PolicyState::new(60);
        p.raw = serde_json::from_value(serde_json::Value::Array(sets)).unwrap();
        p.loaded_at = Some(Instant::now());
        p.fallback_deny.insert(
            "/orgs/5/sec_policy/draft/rule_sets/102".into(),
            vec![serde_json::from_value(deny_on_20).unwrap()],
        );
        let corp = p
            .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21")
            .unwrap();
        let corp: Vec<(&str, RuleKind)> =
            corp.iter().map(|u| (u.ruleset.as_str(), u.kind)).collect();
        assert_eq!(
            corp,
            vec![("Shop-Prod", RuleKind::Allow), ("Edge", RuleKind::Allow)],
            "Edge names the list as a provider"
        );
        let any = p
            .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/20")
            .unwrap();
        let any: Vec<(&str, RuleKind)> = any.iter().map(|u| (u.ruleset.as_str(), u.kind)).collect();
        assert_eq!(
            any,
            vec![("Shop-Prod", RuleKind::Deny), ("Edge", RuleKind::Deny)],
            "the second deny rule comes from fallback_deny"
        );
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::tui::test_support::{model_with_flows, model_with_rich_flows};
    use pce::traffic::{Focus, FocusDirection};

    fn app(s: &str) -> Vec<String> {
        s.split('/').map(str::to_string).collect()
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn data_of(t: &TrafficState, n: usize) -> TrafficData {
        TrafficData {
            flows: t.loaded_flows()[..n].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: t.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        }
    }

    #[test]
    fn the_focus_narrows_every_view_and_clearing_brings_them_back() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        assert_eq!(t.flows.len(), 30);
        let crm = app("crm/prod");
        t.set_focus(Focus::new(crm.clone(), keys()).with_direction(FocusDirection::In));
        assert_eq!(t.flows.len(), 7);
        assert!(t.flows.rows().iter().all(|r| &*r.dst_group == "crm/prod"));
        assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| g.dst == crm));
        assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| r.dst == crm));
        assert!(t.treemap.groups.iter().all(|g| g.dst == crm));
        assert!(
            !t.blocked.is_empty() && t.blocked.rows().iter().all(|r| r.app == crm),
            "flow 6 is blocked into crm/prod"
        );
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Out));
        assert_eq!(t.flows.len(), 4);
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Both));
        assert_eq!(
            t.flows.len(),
            9,
            "flows 3 and 4 are crm/prod to itself: once"
        );
        assert!(t.clear_focus());
        assert_eq!(t.flows.len(), 30);
        assert!(!t.clear_focus(), "nothing left to clear");
        assert_eq!(t.cycle_focus_direction(), None);
    }

    #[test]
    fn the_decision_filter_and_by_service_work_on_top_of_the_focus() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        t.set_focus(Focus::new(app("crm/prod"), keys()));
        t.apps_state.decisions = Decisions::BlockedOnly;
        t.apps_state.by_service = true;
        t.rebuild_pairs();
        let rows: Vec<String> = t
            .services
            .rows()
            .iter()
            .map(|g| {
                format!(
                    "{}>{} {}",
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.service.clone().unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec!["crm/dev>crm/prod 3306/tcp"],
            "the one blocked flow of crm/prod"
        );
    }

    #[test]
    fn f_takes_the_source_then_the_destination_and_in_sections_the_destination_first() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("fqdn-1.example/-")]
        );
        assert_eq!(t.next_focus_app(), Some(app("shop/prod")));
        t.set_focus(Focus::new(app("shop/prod"), keys()));
        t.flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        assert_eq!(
            t.next_focus_app(),
            Some(app("fqdn-1.example/-")),
            "a second F switches sides"
        );
        t.flows.select_where(|r| r.flow.num_connections == 310);
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod")],
            "a flow to itself has one app"
        );
        t.view = TrafficView::Blocked;
        t.blocked.select_where(|r| !r.is_header());
        assert_eq!(
            t.row_apps()[0],
            app("shop/prod"),
            "Blocked is in: destination first"
        );
        t.blocked.select_where(|r| r.is_header());
        assert_eq!(t.row_apps(), vec![app("shop/prod")], "a header is its app");
        t.view = TrafficView::Mesh;
        t.mesh
            .select_where(|r| r.dst.join("/") == "fqdn-1.example/-");
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("fqdn-1.example/-")],
            "a Mesh row: the source, then the destination"
        );
        t.mesh.select_where(|r| r.src == r.dst);
        assert_eq!(t.row_apps(), vec![app("shop/prod")], "to itself: one app");
        t.flows.set_rows(Vec::new());
        t.view = TrafficView::Flows;
        assert!(t.row_apps().is_empty());
        assert_eq!(t.next_focus_app(), None);
        assert_eq!(t.loaded_apps(), vec!["shop/prod"]);
    }

    #[test]
    fn a_focus_on_a_truncated_result_is_partial_until_its_focused_query_is_loaded() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.data.as_mut().unwrap().truncated = true;
        let f = Focus::new(app("shop/prod"), keys());
        t.set_focus(f.clone());
        assert!(t.focus_partial());
        assert_eq!(t.focus_requery(), Some(f.clone()));
        assert_eq!(
            t.focus_meta().as_deref(),
            Some("focus is partial: r re-queries shop/prod · both")
        );
        let focused = data_of(t, 2);
        t.apply_focused(f.clone(), 2, focused);
        assert!(!t.focus_partial());
        assert_eq!(
            t.focus_meta().as_deref(),
            Some("focus shop/prod · both · 2 queries")
        );
        assert_eq!(t.loaded_flows().len(), 2);
        assert_eq!(
            t.focus_requery(),
            Some(f.clone()),
            "r refreshes the focused result"
        );
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::In));
        assert!(!t.focus_partial(), "a both result covers in");
        assert!(t.clear_focus());
        assert_eq!(t.loaded_flows().len(), 4, "the unfocused result is back");
        assert!(t.data.as_ref().unwrap().truncated);
        assert!(t.unfocused.is_none() && t.focus_result.is_none());
    }

    #[test]
    fn a_focused_result_of_another_app_goes_back_first_and_window_keys_go_back_too() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.data.as_mut().unwrap().truncated = true;
        let f = Focus::new(app("shop/prod"), keys()).with_direction(FocusDirection::In);
        let focused = data_of(t, 1);
        t.apply_focused(f, 1, focused);
        assert_eq!(t.loaded_flows().len(), 1);
        t.set_focus(Focus::new(app("unmanaged-nas/-"), keys()));
        assert!(
            t.unfocused.is_none(),
            "another app: the unfocused result is back first"
        );
        assert_eq!(t.loaded_flows().len(), 4);
        assert!(t.focus_partial(), "still truncated: partial");
        assert!(!t.back_to_unfocused(), "nothing focused to go back from");
        let f = Focus::new(app("shop/prod"), keys());
        let focused = data_of(t, 2);
        t.apply_focused(f, 2, focused);
        assert!(t.back_to_unfocused());
        assert_eq!(t.loaded_flows().len(), 4);
        assert!(t.focus.is_some(), "the focus stays");
        assert!(t.drop_focus(), "drop_focus reports the focus it dropped");
        assert!(t.focus.is_none() && t.unfocused.is_none());
    }

    #[test]
    fn queries_text_counts() {
        assert_eq!(queries_text(1), "1 query");
        assert_eq!(queries_text(2), "2 queries");
    }

    #[test]
    fn the_focused_flows_are_computed_once_per_focus_or_data_change() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        assert!(
            std::ptr::eq(t.view_flows().as_ptr(), t.loaded_flows().as_ptr()),
            "no focus: the loaded flows, borrowed"
        );
        t.set_focus(Focus::new(app("crm/prod"), keys()).with_direction(FocusDirection::In));
        assert_eq!(t.view_flows().len(), 7, "focus set");
        let cached = t.view_flows().as_ptr();
        t.rebuild_pairs();
        assert_eq!(
            t.view_flows().as_ptr(),
            cached,
            "a rebuild of one view reuses it"
        );
        t.cycle_focus_direction();
        assert_eq!(t.view_flows().len(), 4, "X: recomputed");
        let more = t.loaded_flows()[..3].to_vec();
        t.data.as_mut().unwrap().flows = more;
        t.rebuild_views();
        assert_eq!(
            t.view_flows().len(),
            t.loaded_flows()
                .iter()
                .filter(|f| t.focus.as_ref().unwrap().keeps(f))
                .count(),
            "new data: recomputed"
        );
        assert!(t.drop_focus());
        assert_eq!(
            t.view_flows().len(),
            3,
            "dropped without a rebuild: every flow"
        );
        let data = data_of(t, 3);
        t.apply_focused(Focus::new(app("crm/prod"), keys()), 2, data);
        assert!(t.restore_unfocused());
        assert_eq!(
            t.view_flows().len(),
            t.loaded_flows()
                .iter()
                .filter(|f| t.focus.as_ref().unwrap().keeps(f))
                .count(),
            "the unfocused result back: recomputed without a rebuild"
        );
    }

    #[test]
    fn a_truncated_result_names_its_cap_and_claims_no_span() {
        let mut m = model_with_flows();
        let d = m.traffic.data.as_mut().unwrap();
        let n = d.flows.len();
        assert_eq!(d.count_text(), format!("{n} flows"));
        d.truncated = true;
        d.filter.max_results = 1000;
        assert_eq!(
            d.count_text(),
            format!("{n} flows, capped at 1000; B raises the cap")
        );
    }

    #[test]
    fn f_on_a_peer_row_of_both_sections_takes_its_sections_app_first() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.view = TrafficView::Aggregate;
        t.apps_state.by = By::Both;
        t.rebuild_sections();
        let peer = |d: Direction, text: &'static str| {
            move |r: &SectionRow| {
                r.peer()
                    .is_some_and(|(rd, p)| rd == d && p.peer_text() == text)
            }
        };
        t.apps.select_where(peer(Direction::In, "unmanaged-nas/-"));
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("unmanaged-nas/-")],
            "an inbound section: its app (the destination) first"
        );
        t.apps
            .select_where(peer(Direction::Out, "fqdn-1.example/-"));
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("fqdn-1.example/-")],
            "an outbound section: its app (the source) first"
        );
    }
}
