//! Every event the TUI reacts to, and every side effect it may request.
use crate::tui::filter::TrafficFilter;
use crate::tui::model::Screen;
use crossterm::event::{KeyEvent, MouseEvent};
use pce::models::{DenyRule, PendingChange, PolicyScope, RuleSet};
use pce::traffic::{QueryProgress, ResolvedFlow};
use pce::{CacheMode, StatsSnapshot};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct QueryOutcome {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub matches_count: Option<u64>,
    pub elapsed: Duration,
    pub hit: bool,
    pub filter: TrafficFilter,
}

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
    /// The editor or a command applied a validated filter (confirmed if needed).
    ApplyFilter(crate::tui::filter::TrafficFilter),
    /// `r` on the traffic screen: re-run the current filter, bypassing the cache.
    RunNow,
    QueryProgress {
        seq: u64,
        progress: QueryProgress,
    },
    QueryDone {
        seq: u64,
        result: Result<QueryOutcome, String>,
    },
    /// Rulesets for `scope` with the cache age in seconds.
    RuleSetsLoaded {
        scope: PolicyScope,
        result: Result<(Vec<RuleSet>, u64), String>,
    },
    DenyRulesLoaded {
        href: String,
        result: Result<Vec<DenyRule>, String>,
    },
    PendingLoaded(Result<Vec<PendingChange>, String>),
}

#[derive(Debug, Clone)]
pub enum Cmd {
    /// `GET /api/v2/health`.
    Connect,
    /// Build (or reuse) the name index: the four object lists.
    LoadIndex(CacheMode),
    RunQuery {
        seq: u64,
        filter: TrafficFilter,
        mode: CacheMode,
        cancel: CancellationToken,
    },
    LoadRuleSets {
        scope: PolicyScope,
        mode: CacheMode,
    },
    /// Fallback for a ruleset whose inline `deny_rules` is empty.
    LoadDenyRules {
        href: String,
        mode: CacheMode,
    },
    LoadPending(CacheMode),
    Quit,
}

impl PartialEq for Cmd {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Cmd::Connect, Cmd::Connect) | (Cmd::Quit, Cmd::Quit) => true,
            (Cmd::LoadIndex(a), Cmd::LoadIndex(b)) => a == b,
            (
                Cmd::RunQuery {
                    seq: a,
                    filter: fa,
                    mode: ma,
                    ..
                },
                Cmd::RunQuery {
                    seq: b,
                    filter: fb,
                    mode: mb,
                    ..
                },
            ) => a == b && fa == fb && ma == mb,
            (
                Cmd::LoadRuleSets {
                    scope: sa,
                    mode: ma,
                },
                Cmd::LoadRuleSets {
                    scope: sb,
                    mode: mb,
                },
            ) => sa == sb && ma == mb,
            (
                Cmd::LoadDenyRules { href: ha, mode: ma },
                Cmd::LoadDenyRules { href: hb, mode: mb },
            ) => ha == hb && ma == mb,
            (Cmd::LoadPending(a), Cmd::LoadPending(b)) => a == b,
            _ => false,
        }
    }
}
