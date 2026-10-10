//! Every event the TUI reacts to, and every side effect it may request.
use crate::tui::model::Screen;
use crossterm::event::{KeyEvent, MouseEvent};
use pce::{CacheMode, StatsSnapshot};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick(Instant),
    Focus(bool),
    /// Health check passed.
    Connected,
    ConnectFailed(String),
    IndexLoaded {
        index: Arc<pce::NameIndex>,
        age_secs: u64,
    },
    IndexFailed(String),
    /// Fresh request counters from the client (sent after every task).
    Stats(StatsSnapshot),
    SwitchScreen(Screen),
    Command(crate::tui::commands::Command),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// `GET /api/v2/health`.
    Connect,
    /// Build (or reuse) the name index: the four object lists.
    LoadIndex(CacheMode),
    Quit,
}
