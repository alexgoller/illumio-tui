//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::ui::theme::Theme;
use pce::StatsSnapshot;
use pce::traffic::ResolvedFlow;
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
        },
        Column {
            title: "src",
            width: 22,
            cell: |f| f.src.display_name(),
            key: |f| SortKey::Text(f.src.display_name()),
            right: false,
        },
        Column {
            title: "src labels",
            width: 0,
            cell: |f| f.src.label_text(),
            key: |f| SortKey::Text(f.src.label_text()),
            right: false,
        },
        Column {
            title: "dst",
            width: 22,
            cell: |f| f.dst.display_name(),
            key: |f| SortKey::Text(f.dst.display_name()),
            right: false,
        },
        Column {
            title: "service",
            width: 10,
            cell: |f| f.service_text(),
            key: |f| SortKey::Num(u64::from(f.port.unwrap_or(0))),
            right: false,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |f| f.num_connections.to_string(),
            key: |f| SortKey::Num(f.num_connections),
            right: true,
        },
        Column {
            title: "decision",
            width: 19,
            cell: |f| f.policy_decision.as_str().to_string(),
            key: |f| SortKey::Text(f.policy_decision.as_str().to_string()),
            right: false,
        },
    ]
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31:02`
pub fn short_time(s: &str) -> String {
    s.get(5..19)
        .map(|x| x.replace('T', " "))
        .unwrap_or_else(|| s.to_string())
}

#[derive(Debug)]
pub struct TrafficState {
    pub filter: TrafficFilter,
    pub query: QueryState,
    pub data: Option<TrafficData>,
    pub seq: u64,
    pub flows: TableView<ResolvedFlow>,
    pub show_detail: bool,
}

impl TrafficState {
    pub fn new(window: &str, max_results: u32) -> Self {
        Self {
            filter: TrafficFilter::defaults(window, max_results),
            query: QueryState::Idle,
            data: None,
            seq: 0,
            flows: TableView::new(flow_columns()),
            show_detail: false,
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
        let traffic = TrafficState::new(&traffic_default_window, traffic_max_results);
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

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, Connection::Connected { .. })
    }
}
