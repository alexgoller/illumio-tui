//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::ui::theme::Theme;
use pce::StatsSnapshot;
use pce::traffic::{BlockedGroup, FlowGroup, ResolvedFlow, blocked_summary, group_by_labels};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Traffic,
    Policy,
    Objects,
    Workloads,
}

impl Screen {
    pub const ALL: [Screen; 4] = [
        Screen::Traffic,
        Screen::Policy,
        Screen::Objects,
        Screen::Workloads,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traffic",
            Screen::Policy => "Policy",
            Screen::Objects => "Labels & objects",
            Screen::Workloads => "Workloads",
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

/// A modal drawn on top of the current screen. Later tasks add `Error`, `Detail`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    RunQuery(TrafficFilter),
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
}

impl AutoRefresh {
    pub fn new(min_seconds: u64) -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(min_seconds.max(60)),
            last_run: None,
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
    pub connection: Connection,
    pub stats: StatsSnapshot,
    pub size: (u16, u16),
    pub focused: bool,
    pub status: StatusLine,
    /// Number of background tasks in flight; the runtime ticks at 250 ms while > 0.
    pub busy: usize,
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
    ) -> Self {
        let theme = Theme::from_name(&theme_name);
        let traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
        );
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
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
            size: (80, 24),
            focused: true,
            status: StatusLine::info("connecting…"),
            busy: 0,
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
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, Connection::Connected { .. })
    }
}
