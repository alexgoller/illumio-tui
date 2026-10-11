//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::form::Form;
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::ui::theme::Theme;
use pce::models::{
    DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope, RuleSet,
    Service, VenHealth, Workload,
};
use pce::traffic::{BlockedGroup, FlowGroup, ResolvedFlow, blocked_summary, group_by_labels};
use pce::{NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Traffic,
    Policy,
    Objects,
    Workloads,
    Events,
}

impl Screen {
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
            Screen::Objects => "Labels & objects",
            Screen::Workloads => "Workloads",
            Screen::Events => "Events",
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

/// Whether editing keys may start a write flow. The guard itself lives in
/// the runtime (`Runtime::writer`); the model never holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritesState {
    /// Both switches on and the audit sink writable.
    pub enabled: bool,
    /// Why not, for the status line.
    pub reason: Option<String>,
}

impl WritesState {
    pub fn on() -> Self {
        Self {
            enabled: true,
            reason: None,
        }
    }

    pub fn off(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            reason: Some(reason.into()),
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
    Confirm {
        reason: String,
        action: PendingAction,
    },
    /// Comma-separated label keys for the app-to-app view.
    GroupKeys {
        input: String,
    },
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
    /// The selected audit event as pretty JSON; `scroll` is the first shown line.
    EventDetail {
        event: Event,
        scroll: usize,
    },
    /// Create or edit form for a label, IP list, service or workload.
    Form(Form),
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Overlay::None, Overlay::None)
            | (Overlay::Help, Overlay::Help)
            | (Overlay::Pending, Overlay::Pending) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::GroupKeys { input: a }, Overlay::GroupKeys { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::FilterEditor(a), Overlay::FilterEditor(b)) => a == b,
            (Overlay::Form(a), Overlay::Form(b)) => a == b,
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
                },
                Overlay::EventDetail {
                    event: b,
                    scroll: sb,
                },
            ) => a.href == b.href && sa == sb,
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
}

#[derive(Debug)]
pub enum QueryState {
    Idle,
    Running {
        seq: u64,
        started: Instant,
        status: String,
        cancel: CancellationToken,
    },
    Failed(String),
}

#[derive(Debug)]
pub struct TrafficData {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub matches_count: Option<u64>,
    pub elapsed: Duration,
    pub cache_hit: bool,
    pub fetched_at: Instant,
    pub filter: TrafficFilter,
}

pub fn flow_columns() -> Vec<Column<ResolvedFlow>> {
    vec![
        Column {
            title: "last seen",
            width: 16,
            cell: |f| {
                f.last_detected
                    .as_deref()
                    .map(short_time)
                    .unwrap_or_default()
            },
            key: |f| SortKey::Text(f.last_detected.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: Some(|f| {
                f.last_detected
                    .as_deref()
                    .map(short_time_narrow)
                    .unwrap_or_default()
            }),
            narrow_width: 11,
        },
        Column {
            title: "src",
            width: 22,
            cell: |f| f.src.display_name(),
            key: |f| SortKey::Text(f.src.display_name()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "src labels",
            width: 0,
            cell: |f| f.src.label_text(),
            key: |f| SortKey::Text(f.src.label_text()),
            right: false,
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "dst",
            width: 22,
            cell: |f| f.dst.display_name(),
            key: |f| SortKey::Text(f.dst.display_name()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "service",
            width: 10,
            cell: |f| f.service_text(),
            key: |f| SortKey::Num(u64::from(f.port.unwrap_or(0))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |f| f.num_connections.to_string(),
            key: |f| SortKey::Num(f.num_connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "decision",
            width: 19,
            cell: |f| f.policy_decision.as_str().to_string(),
            key: |f| SortKey::Text(f.policy_decision.as_str().to_string()),
            right: false,
            hide_below: 0,
            narrow: Some(|f| match f.policy_decision.as_str() {
                "potentially_blocked" => "pot.blk".to_string(),
                other => other.to_string(),
            }),
            narrow_width: 8,
        },
    ]
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31:02`
pub fn short_time(s: &str) -> String {
    s.get(5..19)
        .map(|x| x.replace('T', " "))
        .unwrap_or_else(|| s.to_string())
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31`
fn short_time_narrow(s: &str) -> String {
    s.get(5..16)
        .map(|x| x.replace('T', " "))
        .unwrap_or_else(|| s.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficView {
    Flows,
    Aggregate,
    Blocked,
}

impl TrafficView {
    pub fn next(self) -> Self {
        match self {
            TrafficView::Flows => TrafficView::Aggregate,
            TrafficView::Aggregate => TrafficView::Blocked,
            TrafficView::Blocked => TrafficView::Flows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficView::Flows => "Flows",
            TrafficView::Aggregate => "App-to-app",
            TrafficView::Blocked => "Blocked",
        }
    }
}

pub fn group_columns() -> Vec<Column<FlowGroup>> {
    vec![
        Column {
            title: "src",
            width: 26,
            cell: |g| g.src.join("/"),
            key: |g| SortKey::Text(g.src.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "dst",
            width: 26,
            cell: |g| g.dst.join("/"),
            key: |g| SortKey::Text(g.dst.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |g| g.flows.to_string(),
            key: |g| SortKey::Num(g.flows),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "conns",
            width: 8,
            cell: |g| g.connections.to_string(),
            key: |g| SortKey::Num(g.connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 6,
        },
        Column {
            title: "allowed",
            width: 7,
            cell: |g| g.allowed.to_string(),
            key: |g| SortKey::Num(g.allowed),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pot.bl",
            width: 6,
            cell: |g| g.potentially_blocked.to_string(),
            key: |g| SortKey::Num(g.potentially_blocked),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "blocked",
            width: 7,
            cell: |g| g.blocked.to_string(),
            key: |g| SortKey::Num(g.blocked),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

pub fn blocked_columns() -> Vec<Column<BlockedGroup>> {
    vec![
        Column {
            title: "last seen",
            width: 16,
            cell: |b| {
                b.last_detected
                    .as_deref()
                    .map(short_time)
                    .unwrap_or_default()
            },
            key: |b| SortKey::Text(b.last_detected.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: Some(|b| {
                b.last_detected
                    .as_deref()
                    .map(short_time_narrow)
                    .unwrap_or_default()
            }),
            narrow_width: 11,
        },
        Column {
            title: "dst",
            width: 22,
            cell: |b| b.dst.clone(),
            key: |b| SortKey::Text(b.dst.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "dst labels",
            width: 0,
            cell: |b| b.dst_labels.clone(),
            key: |b| SortKey::Text(b.dst_labels.clone()),
            right: false,
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "service",
            width: 10,
            cell: |b| match b.port {
                Some(p) => format!("{p}/{}", b.proto),
                None => b.proto.clone(),
            },
            key: |b| SortKey::Num(u64::from(b.port.unwrap_or(0))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |b| b.flows.to_string(),
            key: |b| SortKey::Num(b.flows),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |b| b.connections.to_string(),
            key: |b| SortKey::Num(b.connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "sources",
            width: 30,
            cell: |b| b.sources.join(", "),
            key: |b| SortKey::Text(b.sources.join(", ")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 20,
        },
    ]
}

/// Opt-in periodic refresh. Never below 60 s nor the profile minimum.
#[derive(Debug, Clone)]
pub struct AutoRefresh {
    pub enabled: bool,
    pub interval: Duration,
    pub last_run: Option<Instant>,
    /// The request in flight was started by the auto-refresh tick.
    pub from_tick: bool,
    /// A failed tick switched auto-refresh off; a manual `r` re-arms it.
    pub tripped: bool,
}

impl AutoRefresh {
    pub fn new(min_seconds: u64) -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(min_seconds.max(60)),
            last_run: None,
            from_tick: false,
            tripped: false,
        }
    }

    /// A failed auto-refresh request: disable, never retry unattended.
    /// Returns whether this failure was the tick's own.
    pub fn take_tick_failure(&mut self) -> bool {
        let was = std::mem::take(&mut self.from_tick);
        if was {
            self.enabled = false;
            self.tripped = true;
        }
        was
    }

    /// `a`: switches auto-refresh on or off (clearing a tripped state) and
    /// returns the status line to show.
    pub fn toggle(&mut self) -> StatusLine {
        self.enabled = !self.enabled;
        self.tripped = false;
        if self.enabled {
            StatusLine::info(format!("auto-refresh every {}s", self.interval.as_secs()))
        } else {
            StatusLine::info("auto-refresh off")
        }
    }

    /// A manual refresh re-enables what a failed tick switched off.
    pub fn rearm(&mut self) {
        if std::mem::take(&mut self.tripped) {
            self.enabled = true;
        }
    }

    /// Panel meta suffix: the countdown while on, the reason after a failed tick.
    pub fn meta(&self, now: Instant) -> String {
        if self.enabled {
            format!(" · auto in {}s", self.remaining(now).as_secs())
        } else if self.tripped {
            " · auto off · failed".into()
        } else {
            String::new()
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        self.enabled
            && self
                .last_run
                .is_none_or(|l| now.saturating_duration_since(l) >= self.interval)
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.last_run
            .map(|l| {
                self.interval
                    .saturating_sub(now.saturating_duration_since(l))
            })
            .unwrap_or(Duration::ZERO)
    }
}

#[derive(Debug)]
pub struct TrafficState {
    pub filter: TrafficFilter,
    pub query: QueryState,
    pub data: Option<TrafficData>,
    pub seq: u64,
    pub flows: TableView<ResolvedFlow>,
    pub show_detail: bool,
    pub view: TrafficView,
    pub group_keys: Vec<String>,
    pub groups: TableView<FlowGroup>,
    pub blocked: TableView<BlockedGroup>,
    /// Aggregate pair (src, dst group keys) the Flows view is narrowed to.
    pub drill: Option<(Vec<String>, Vec<String>)>,
    pub auto: AutoRefresh,
}

impl TrafficState {
    pub fn new(window: &str, max_results: u32, auto_min_seconds: u64) -> Self {
        Self {
            filter: TrafficFilter::defaults(window, max_results),
            query: QueryState::Idle,
            data: None,
            seq: 0,
            flows: TableView::new(flow_columns()),
            show_detail: false,
            view: TrafficView::Flows,
            group_keys: vec!["app".into(), "env".into()],
            groups: TableView::new(group_columns()),
            blocked: TableView::new(blocked_columns()),
            drill: None,
            auto: AutoRefresh::new(auto_min_seconds),
        }
    }

    /// Recomputes every view from `data`. Pure; never requests.
    pub fn rebuild_views(&mut self) {
        let flows: Vec<ResolvedFlow> = self
            .data
            .as_ref()
            .map(|d| d.flows.clone())
            .unwrap_or_default();
        let shown: Vec<ResolvedFlow> = match &self.drill {
            Some((src, dst)) => flows
                .iter()
                .filter(|f| {
                    group_by_labels(std::slice::from_ref(f), &self.group_keys)
                        .first()
                        .is_some_and(|g| &g.src == src && &g.dst == dst)
                })
                .cloned()
                .collect(),
            None => flows.clone(),
        };
        self.flows.set_rows(shown);
        self.groups
            .set_rows(group_by_labels(&flows, &self.group_keys));
        self.blocked.set_rows(blocked_summary(&flows));
    }

    /// Whether the active view's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
        match self.view {
            TrafficView::Flows => self.flows.is_searching(),
            TrafficView::Aggregate => self.groups.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.query, QueryState::Running { .. })
    }

    /// Cancels a running query (if any) and returns the next sequence number.
    pub fn start_query(&mut self, now: Instant) -> (u64, CancellationToken) {
        if let QueryState::Running { cancel, .. } = &self.query {
            cancel.cancel();
        }
        self.seq += 1;
        self.auto.last_run = Some(now);
        self.auto.from_tick = false;
        let cancel = CancellationToken::new();
        self.query = QueryState::Running {
            seq: self.seq,
            started: now,
            status: "submitting".into(),
            cancel: cancel.clone(),
        };
        (self.seq, cancel)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyPane {
    RuleSets,
    Rules,
}

fn plain_names<'a>(v: impl Iterator<Item = &'a str>) -> String {
    v.collect::<Vec<_>>().join(", ")
}

fn actor_names(v: &[pce::ResolvedActor]) -> String {
    plain_names(v.iter().map(|a| a.name.as_str()))
}

fn service_names(v: &[pce::ResolvedService]) -> String {
    plain_names(v.iter().map(|s| s.name.as_str()))
}

/// Evaluation order of the rule kinds: an override deny beats an allow,
/// which beats a plain deny.
fn rule_rank(kind: RuleKind) -> u8 {
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

fn on_off(enabled: bool) -> String {
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
        },
    ]
}

pub fn rule_columns() -> Vec<Column<RuleView>> {
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
        },
    ]
}

#[derive(Debug)]
pub struct PolicyState {
    pub scope: PolicyScope,
    pub pane: PolicyPane,
    pub rulesets: TableView<RuleSetView>,
    pub rules: TableView<RuleView>,
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
    pub auto: AutoRefresh,
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

    pub fn set_rule_sets(&mut self, raw: Vec<RuleSet>, idx: &NameIndex) {
        self.rulesets.set_rows(
            raw.iter()
                .map(|rs| RuleSetView::from_rule_set(rs, idx))
                .collect(),
        );
        self.raw = raw;
        self.rebuild_rules(idx);
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
    /// order within each group is the PCE's.
    pub fn rebuild_rules(&mut self, idx: &NameIndex) {
        let Some(href) = self.selected_href() else {
            self.rules.set_rows(Vec::new());
            return;
        };
        let Some(rs) = self.raw.iter().find(|r| r.href == href) else {
            self.rules.set_rows(Vec::new());
            return;
        };
        let mut rows: Vec<RuleView> = rs
            .rules
            .iter()
            .map(|r| RuleView::from_rule(r, idx))
            .collect();
        let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
            self.fallback_deny.get(&href).map_or(&[], Vec::as_slice)
        } else {
            &rs.deny_rules
        };
        rows.extend(deny.iter().map(|d| RuleView::from_deny(d, idx)));
        rows.sort_by_key(|r| rule_rank(r.kind));
        self.rules.set_rows(rows);
    }

    pub fn selected_href(&self) -> Option<String> {
        self.rulesets.selected_item().map(|r| r.href.clone())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Labels,
    LabelGroups,
    IpLists,
    Services,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 4] = [
        ObjectKind::Labels,
        ObjectKind::LabelGroups,
        ObjectKind::IpLists,
        ObjectKind::Services,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % 4]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + 3) % 4]
    }

    pub fn title(self) -> &'static str {
        match self {
            ObjectKind::Labels => "Labels",
            ObjectKind::LabelGroups => "Label groups",
            ObjectKind::IpLists => "IP lists",
            ObjectKind::Services => "Services",
        }
    }
}

pub fn label_columns() -> Vec<Column<Label>> {
    vec![
        Column {
            title: "key",
            width: 16,
            cell: |l| l.key.clone(),
            key: |l| SortKey::Text(l.key.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "value",
            width: 0,
            cell: |l| l.value.clone(),
            key: |l| SortKey::Text(l.value.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "deleted",
            width: 7,
            cell: |l| {
                if l.is_deleted() {
                    "yes".into()
                } else {
                    String::new()
                }
            },
            key: |l| SortKey::Num(u64::from(l.is_deleted())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "href",
            width: 24,
            cell: |l| l.href.clone(),
            key: |l| SortKey::Text(l.href.clone()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

pub fn group_columns_objects() -> Vec<Column<LabelGroup>> {
    vec![
        Column {
            title: "name",
            width: 0,
            cell: |g| g.name.clone(),
            key: |g| SortKey::Text(g.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "key",
            width: 12,
            cell: |g| g.key.clone(),
            key: |g| SortKey::Text(g.key.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "labels",
            width: 6,
            cell: |g| g.labels.len().to_string(),
            key: |g| SortKey::Num(g.labels.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "groups",
            width: 6,
            cell: |g| g.sub_groups.len().to_string(),
            key: |g| SortKey::Num(g.sub_groups.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |g| g.update_type.clone().unwrap_or_default(),
            key: |g| SortKey::Text(g.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

pub fn ip_list_columns() -> Vec<Column<IpList>> {
    vec![
        Column {
            title: "name",
            width: 28,
            cell: |l| l.name.clone(),
            key: |l| SortKey::Text(l.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
        },
        Column {
            title: "ranges",
            width: 0,
            cell: |l| l.summary(),
            key: |l| SortKey::Num(l.ip_ranges.len() as u64),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |l| l.update_type.clone().unwrap_or_default(),
            key: |l| SortKey::Text(l.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

pub fn service_columns() -> Vec<Column<Service>> {
    vec![
        Column {
            title: "name",
            width: 28,
            cell: |s| s.name.clone(),
            key: |s| SortKey::Text(s.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
        },
        Column {
            title: "ports",
            width: 0,
            cell: |s| s.summary(),
            key: |s| SortKey::Num(s.service_ports.len() as u64),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |s| s.update_type.clone().unwrap_or_default(),
            key: |s| SortKey::Text(s.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

#[derive(Debug)]
pub struct ObjectsState {
    pub kind: ObjectKind,
    pub labels: TableView<Label>,
    pub groups: TableView<LabelGroup>,
    pub ip_lists: TableView<IpList>,
    pub services: TableView<Service>,
    /// Cache age in seconds when each kind (indexed by `ObjectKind::index`) was loaded.
    pub ages: [Option<u64>; 4],
    /// When each kind arrived, so the shown age keeps growing.
    pub loaded_at: [Option<Instant>; 4],
    pub loading: [bool; 4],
    pub show_detail: bool,
    /// The startup index load failed; `r` retries it.
    pub index_failed: bool,
    pub auto: AutoRefresh,
}

impl ObjectsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            kind: ObjectKind::Labels,
            labels: TableView::new(label_columns()),
            groups: TableView::new(group_columns_objects()),
            ip_lists: TableView::new(ip_list_columns()),
            services: TableView::new(service_columns()),
            ages: [None; 4],
            loaded_at: [None; 4],
            loading: [false; 4],
            show_detail: false,
            index_failed: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    pub fn loaded(&self) -> bool {
        self.ages.iter().all(Option::is_some)
    }

    /// Whether the current kind's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
        match self.kind {
            ObjectKind::Labels => self.labels.is_searching(),
            ObjectKind::LabelGroups => self.groups.is_searching(),
            ObjectKind::IpLists => self.ip_lists.is_searching(),
            ObjectKind::Services => self.services.is_searching(),
        }
    }

    /// Whether the current kind has a selected row.
    pub fn has_selection(&self) -> bool {
        match self.kind {
            ObjectKind::Labels => self.labels.selected_item().is_some(),
            ObjectKind::LabelGroups => self.groups.selected_item().is_some(),
            ObjectKind::IpLists => self.ip_lists.selected_item().is_some(),
            ObjectKind::Services => self.services.selected_item().is_some(),
        }
    }

    /// Age of the current kind in seconds, growing since it was loaded.
    pub fn age_secs(&self, kind: ObjectKind) -> Option<u64> {
        let i = kind.index();
        self.ages[i].map(|a| a + self.loaded_at[i].map_or(0, |t| t.elapsed().as_secs()))
    }
}

/// A workload with its derived health and display strings, computed once per
/// rebuild so the column `fn` pointers stay pure.
#[derive(Debug, Clone)]
pub struct WorkloadRowView {
    pub workload: Workload,
    pub health: VenHealth,
    pub heartbeat: String,
    /// Heartbeat age in seconds, for sorting (`None` sorts last when ascending).
    pub heartbeat_secs: Option<u64>,
    pub sync: String,
    pub ven: String,
}

impl WorkloadRowView {
    pub fn new(workload: Workload, now: jiff::Timestamp) -> Self {
        let health = workload.ven_health(now);
        let a = workload.agent_status_view();
        let age = workload.heartbeat_age(now);
        let heartbeat = age
            .map(crate::tui::ui::widgets::bars::age_text)
            .unwrap_or_default();
        Self {
            heartbeat,
            heartbeat_secs: age.map(|d| d.as_secs()),
            sync: a
                .as_ref()
                .and_then(|a| a.sync_state.clone())
                .unwrap_or_default(),
            ven: a.and_then(|a| a.version).unwrap_or_default(),
            health,
            workload,
        }
    }
}

type RowCell = fn(&WorkloadRowView) -> String;
type RowKey = fn(&WorkloadRowView) -> SortKey;

fn text_col(
    title: &'static str,
    width: u16,
    cell: RowCell,
    key: RowKey,
) -> Column<WorkloadRowView> {
    Column {
        title,
        width,
        cell,
        key,
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
    }
}

fn hostname_of(r: &WorkloadRowView) -> String {
    r.workload.hostname.clone().unwrap_or_default()
}

fn name_of(r: &WorkloadRowView) -> String {
    r.workload.name.clone().unwrap_or_default()
}

fn ip_of(r: &WorkloadRowView) -> String {
    r.workload.primary_ip().unwrap_or("").to_string()
}

fn enforcement_of(r: &WorkloadRowView) -> String {
    r.workload.enforcement_mode.clone().unwrap_or_default()
}

fn visibility_of(r: &WorkloadRowView) -> String {
    r.workload.visibility_level.clone().unwrap_or_default()
}

fn online_of(r: &WorkloadRowView) -> String {
    match r.workload.online {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
    .to_string()
}

fn agent_of(r: &WorkloadRowView) -> String {
    r.workload.agent_status().unwrap_or_else(|| "-".into())
}

/// Index of the `health` column in `workload_columns()`.
const HEALTH_COL: usize = 8;

pub fn workload_columns() -> Vec<Column<WorkloadRowView>> {
    // Below 120 columns only hostname, ip, enforcement, heartbeat and health
    // stay, so the table is readable at 80.
    let wide = |c: Column<WorkloadRowView>| Column {
        hide_below: 120,
        ..c
    };
    vec![
        Column {
            // A workload without a hostname is still identifiable when the name column is hidden.
            narrow: Some(|r| r.workload.display_name()),
            narrow_width: 20,
            ..text_col("hostname", 22, hostname_of, |r| {
                SortKey::Text(hostname_of(r))
            })
        },
        wide(text_col("name", 14, name_of, |r| SortKey::Text(name_of(r)))),
        text_col("ip", 15, ip_of, |r| SortKey::Text(ip_of(r))),
        wide(text_col(
            "labels",
            0,
            |r| r.workload.label_text(),
            |r| SortKey::Text(r.workload.label_text()),
        )),
        Column {
            narrow: Some(|r| enforcement_of(r).replace("visibility_only", "visibility")),
            narrow_width: 11,
            ..text_col("enforcement", 15, enforcement_of, |r| {
                SortKey::Text(enforcement_of(r))
            })
        },
        Column {
            // Secondary: gives way to the health columns until the terminal is very wide.
            hide_below: 180,
            ..text_col("visibility", 13, visibility_of, |r| {
                SortKey::Text(visibility_of(r))
            })
        },
        wide(text_col("online", 6, online_of, |r| {
            SortKey::Num(u64::from(r.workload.online.unwrap_or(false)))
        })),
        wide(text_col("agent", 9, agent_of, |r| {
            SortKey::Text(agent_of(r))
        })),
        text_col(
            "health",
            9,
            |r| r.health.as_str().to_string(),
            |r| SortKey::Num(u64::from(r.health.rank())),
        ),
        text_col(
            "heartbeat",
            9,
            |r| r.heartbeat.clone(),
            |r| SortKey::Num(r.heartbeat_secs.unwrap_or(u64::MAX)),
        ),
        wide(text_col(
            "sync",
            8,
            |r| r.sync.clone(),
            |r| SortKey::Text(r.sync.clone()),
        )),
        Column {
            hide_below: 140,
            ..text_col(
                "ven",
                12,
                |r| r.ven.clone(),
                |r| SortKey::Text(r.ven.clone()),
            )
        },
    ]
}

#[derive(Debug)]
pub struct WorkloadsState {
    pub table: TableView<WorkloadRowView>,
    /// The workloads as fetched; `table` is a projection of these.
    pub raw: Vec<Workload>,
    pub unhealthy_only: bool,
    /// Test-only fixed clock so ages in snapshots are deterministic.
    pub now: Option<jiff::Timestamp>,
    /// Workloads per health state in rank order (healthy, unmanaged, warning, offline, error).
    counts: [usize; 5],
    pub age_secs: u64,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
    pub show_detail: bool,
    /// The name index failed, so the screen could not load; `r` retries the index.
    pub index_failed: bool,
    pub auto: AutoRefresh,
}

impl WorkloadsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(workload_columns()),
            raw: Vec::new(),
            unhealthy_only: false,
            now: None,
            counts: [0; 5],
            age_secs: 0,
            loaded_at: None,
            loading: false,
            show_detail: false,
            index_failed: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Recomputes the table rows (and health counts) from `raw`.
    pub fn rebuild(&mut self) {
        let now = self.now.unwrap_or_else(jiff::Timestamp::now);
        let views: Vec<WorkloadRowView> = self
            .raw
            .iter()
            .cloned()
            .map(|w| WorkloadRowView::new(w, now))
            .collect();
        self.counts = [0; 5];
        for v in &views {
            self.counts[usize::from(v.health.rank())] += 1;
        }
        let unhealthy = |h: VenHealth| {
            matches!(
                h,
                VenHealth::Warning | VenHealth::Error | VenHealth::Offline
            )
        };
        let rows = if self.unhealthy_only {
            views.into_iter().filter(|v| unhealthy(v.health)).collect()
        } else {
            views
        };
        self.table.set_rows(rows);
    }

    /// Counts per state in rank order (healthy, unmanaged, warning, offline, error)
    /// over all loaded workloads, ignoring the unhealthy-only filter.
    pub fn summary(&self) -> [usize; 5] {
        self.counts
    }

    /// `H`: worst health first; pressing it again flips the direction.
    pub fn sort_by_health(&mut self) {
        let desc = self.table.sort() != Some((HEALTH_COL, true));
        self.table.sort_by(HEALTH_COL, desc);
    }
}

/// The severity column, the target of `S`.
const SEVERITY_COL: usize = 1;

pub fn event_columns() -> Vec<Column<Event>> {
    vec![
        Column {
            title: "timestamp",
            width: 20,
            cell: |e| {
                e.timestamp
                    .as_deref()
                    .map(|t| t.get(..19).unwrap_or(t).replace('T', " "))
                    .unwrap_or_default()
            },
            key: |e| SortKey::Text(e.timestamp.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: Some(|e| {
                e.timestamp
                    .as_deref()
                    .map(|t| t.get(5..19).unwrap_or(t).replace('T', " "))
                    .unwrap_or_default()
            }),
            narrow_width: 14,
        },
        Column {
            title: "severity",
            width: 8,
            cell: |e| e.severity.clone().unwrap_or_default(),
            key: |e| SortKey::Num(u64::from(e.severity_rank())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "type",
            width: 28,
            cell: |e| e.event_type.clone().unwrap_or_default(),
            key: |e| SortKey::Text(e.event_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
        },
        Column {
            title: "actor",
            width: 22,
            cell: |e| e.actor(),
            key: |e| SortKey::Text(e.actor()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 20,
        },
        Column {
            title: "resource",
            width: 0,
            cell: |e| e.resource(),
            key: |e| SortKey::Text(e.resource()),
            right: false,
            // Secondary: only on wide terminals.
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "status",
            width: 8,
            cell: |e| e.status.clone().unwrap_or_default(),
            key: |e| SortKey::Text(e.status.clone().unwrap_or_default()),
            right: false,
            hide_below: 100,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

/// What the events editor holds: the server-side severity and window, and the
/// local `type` substring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsFilterEdit {
    pub severity: String,
    pub event_type: String,
    pub window: String,
}

impl EventsFilterEdit {
    /// The request filter. `since` is rounded down to the whole minute and
    /// `until` is left open, so applying the same filter again within the cache
    /// TTL produces the same query (and a cache hit) instead of a new one every second.
    pub fn to_filter(&self, now: jiff::Timestamp) -> Result<EventFilter, String> {
        let w = pce::traffic::parse_window(&self.window).map_err(|e| format!("window: {e}"))?;
        let span = jiff::Span::try_from(w).map_err(|e| format!("window: {e}"))?;
        let minute = jiff::Timestamp::from_second(now.as_second().div_euclid(60) * 60)
            .map_err(|e| e.to_string())?;
        let since = minute
            .checked_sub(span)
            .map_err(|e| format!("window: {e}"))?;
        Ok(EventFilter {
            severity: Some(self.severity.trim().to_string()).filter(|s| !s.is_empty()),
            event_type: None,
            status: None,
            since: Some(since),
            until: None,
            max_results: None,
        })
    }
}

#[derive(Debug)]
pub struct EventsState {
    pub table: TableView<Event>,
    /// The events as fetched; `table` is the local `type` projection of these.
    pub raw: Vec<Event>,
    pub filter: EventsFilterEdit,
    /// The `since` and `until` of the last answered request (`until` is empty when open).
    pub window: Option<(String, String)>,
    pub loaded_at: Option<Instant>,
    pub age_secs: u64,
    pub loading: bool,
    pub auto: AutoRefresh,
}

impl EventsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(event_columns()),
            raw: Vec::new(),
            filter: EventsFilterEdit {
                severity: String::new(),
                event_type: String::new(),
                window: "24h".into(),
            },
            window: None,
            loaded_at: None,
            age_secs: 0,
            loading: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Applies the local type substring filter.
    pub fn rebuild(&mut self) {
        let needle = self.filter.event_type.trim().to_ascii_lowercase();
        let rows: Vec<Event> = self
            .raw
            .iter()
            .filter(|e| {
                needle.is_empty()
                    || e.event_type
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&needle)
            })
            .cloned()
            .collect();
        self.table.set_rows(rows);
    }

    /// `S`: worst severity first; pressing it again flips the direction.
    pub fn sort_by_severity(&mut self) {
        let desc = self.table.sort() != Some((SEVERITY_COL, true));
        self.table.sort_by(SEVERITY_COL, desc);
    }
}

/// Limits copied from the resolved profile so `update` never touches config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub auto_refresh_min_seconds: u64,
    pub traffic_max_results: u32,
    pub traffic_default_window: String,
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
    pub connection: Connection,
    pub stats: StatsSnapshot,
    pub size: (u16, u16),
    pub focused: bool,
    pub status: StatusLine,
    /// Set at startup and on every profile switch from the runtime's guard.
    pub writes: WritesState,
    /// First help line shown; `Overlay::Help` is reopened at the top.
    pub help_scroll: usize,
    /// Number of background tasks in flight; the runtime ticks at 250 ms while > 0.
    pub busy: usize,
    /// A `:profile` switch is being built off the event loop; a second one
    /// waits until it lands so two switches cannot race.
    pub switching: bool,
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
        let traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
        );
        let policy = PolicyState::new(auto_refresh_min_seconds);
        let objects = ObjectsState::new(auto_refresh_min_seconds);
        let workloads = WorkloadsState::new(auto_refresh_min_seconds);
        let events = EventsState::new(auto_refresh_min_seconds);
        Self {
            profile,
            theme_name,
            theme,
            limits: Limits {
                auto_refresh_min_seconds,
                traffic_max_results,
                traffic_default_window,
            },
            screen: Screen::Traffic,
            overlay: Overlay::None,
            traffic,
            policy,
            objects,
            workloads,
            events,
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
            size: (80, 24),
            focused: true,
            status: StatusLine::info("connecting…"),
            writes,
            help_scroll: 0,
            busy: 0,
            switching: false,
            quit: false,
            started: Instant::now(),
        }
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme_name = name.to_string();
        self.theme = Theme::from_name(name);
    }

    pub fn index(&self) -> Option<&std::sync::Arc<pce::NameIndex>> {
        match &self.connection {
            Connection::Connected { index, .. } => index.as_ref(),
            _ => None,
        }
    }

    /// Whether any screen has opted in to auto-refresh (drives the idle tick).
    pub fn any_auto_enabled(&self) -> bool {
        self.traffic.auto.enabled
            || self.policy.auto.enabled
            || self.objects.auto.enabled
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
        );
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        self.traffic = traffic;
        self.policy = PolicyState::new(min);
        self.objects = ObjectsState::new(min);
        self.workloads = WorkloadsState::new(min);
        self.events = EventsState::new(min);
        self.stats = StatsSnapshot { calls: 0, bytes: 0 };
        self.busy = 0;
        self.overlay = Overlay::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
