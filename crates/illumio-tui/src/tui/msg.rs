//! Every event the TUI reacts to, and every side effect it may request.
use crate::tui::diff::ObjectDiff;
use crate::tui::edit::WriteIntent;
use crate::tui::filter::TrafficFilter;
use crate::tui::model::{Limits, ObjectKind, ProfileInfo, Screen};
use crossterm::event::{KeyEvent, MouseEvent};
use pce::models::{
    DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope, RuleSet,
    Service, Workload,
};
use pce::traffic::{QueryProgress, ResolvedFlow};
use pce::write::{Outcome, Plan, WriteError};
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

/// One object list with its cache age in seconds.
#[derive(Debug)]
pub enum ObjectsPayload {
    Labels(Vec<Label>, u64),
    LabelGroups(Vec<LabelGroup>, u64),
    Services(Vec<Service>, u64),
}

/// Events, the cache age in seconds and the `(since, until)` the request used.
pub type EventsPayload = (Vec<Event>, u64, (String, String));

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
    ObjectsLoaded {
        kind: ObjectKind,
        result: Result<ObjectsPayload, String>,
    },
    /// The draft IP lists with the cache age in seconds.
    IpListsLoaded(Result<(Vec<IpList>, u64), String>),
    /// Workloads with the cache age in seconds.
    WorkloadsLoaded(Result<(Vec<Workload>, u64), String>),
    /// Events, the cache age in seconds and the `(since, until)` the request used.
    EventsLoaded(Result<EventsPayload, String>),
    /// The runtime rebuilt the connection for `:profile <name>` (or could not);
    /// the new profile's PCE-load limits come with it.
    ProfileSwitched(Result<(ProfileInfo, Limits), String>),
    /// A planned write (the exact request) or why it could not be planned.
    Planned(Result<(Plan, pce::write::ObjectKind), String>),
    /// The result of one sent write. `method` names what was sent (None for
    /// a plan that was a no-op).
    WriteDone {
        kind: pce::write::ObjectKind,
        method: Option<pce::WriteMethod>,
        result: Result<Outcome, WriteError>,
    },
    DiffLoaded(Result<ObjectDiff, String>),
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
    LoadObjects {
        kind: ObjectKind,
        mode: CacheMode,
    },
    /// The draft IP lists for the IP lists screen (the index's own list, cached).
    LoadIpLists(CacheMode),
    LoadWorkloads(CacheMode),
    LoadEvents {
        filter: EventFilter,
        mode: CacheMode,
    },
    /// Rebuild the client for another profile from the config (no request).
    SwitchProfile(String),
    /// Plan a write (one GET for IP lists, services and workloads; none for
    /// labels) and show it in the confirm overlay. Never sends a write.
    PlanWrite(WriteIntent),
    /// `y` on the confirm overlay: send this plan once.
    ExecuteWrite {
        plan: Plan,
        kind: pce::write::ObjectKind,
    },
    /// The draft and active list of one kind (cached) for a pending IP list, service or ruleset.
    LoadDiff {
        change: PendingChange,
        kind: pce::write::ObjectKind,
        /// A ruleset's draft deny rules already fetched on the Policy screen
        /// (`None`: not fetched), for rulesets whose list payload had none.
        fallback_deny: Option<Vec<pce::models::DenyRule>>,
    },
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
            (Cmd::LoadObjects { kind: ka, mode: ma }, Cmd::LoadObjects { kind: kb, mode: mb }) => {
                ka == kb && ma == mb
            }
            (Cmd::LoadIpLists(a), Cmd::LoadIpLists(b)) => a == b,
            (Cmd::LoadWorkloads(a), Cmd::LoadWorkloads(b)) => a == b,
            (
                Cmd::LoadEvents {
                    filter: fa,
                    mode: ma,
                },
                Cmd::LoadEvents {
                    filter: fb,
                    mode: mb,
                },
            ) => fa == fb && ma == mb,
            (Cmd::SwitchProfile(a), Cmd::SwitchProfile(b)) => a == b,
            (Cmd::PlanWrite(a), Cmd::PlanWrite(b)) => a == b,
            (
                Cmd::ExecuteWrite { plan: pa, kind: ka },
                Cmd::ExecuteWrite { plan: pb, kind: kb },
            ) => pa.request() == pb.request() && ka == kb,
            (
                Cmd::LoadDiff {
                    change: ca,
                    kind: ka,
                    fallback_deny: fa,
                },
                Cmd::LoadDiff {
                    change: cb,
                    kind: kb,
                    fallback_deny: fb,
                },
            ) => ca == cb && ka == kb && fa == fb,
            _ => false,
        }
    }
}
