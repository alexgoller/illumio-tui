//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::tui::ui::theme::Theme;
use pce::StatsSnapshot;
use std::time::Instant;

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
