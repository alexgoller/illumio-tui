//! Every event the TUI reacts to, and every side effect it may request.
use crate::tui::diff::ObjectDiff;
use crate::tui::edit::WriteIntent;
use crate::tui::filter::TrafficFilter;
use crate::tui::model::{Limits, ObjectKind, ProfileInfo, Screen, WritesState};
use crossterm::event::{KeyEvent, MouseEvent};
use pce::models::{
    DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope, RuleSet,
    Service, Workload, WorkloadFilter,
};
use pce::traffic::{Focus, QueryProgress, ResolvedFlow};
use pce::write::{Outcome, Plan, WriteError};
use pce::{CacheMode, StatsSnapshot};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct QueryOutcome {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    /// A supercluster region did not respond.
    pub incomplete: bool,
    pub elapsed: Duration,
    pub hit: bool,
    pub filter: TrafficFilter,
    /// The `(start, end)` the query asked for; `None` if unparseable.
    pub bounds: Option<(jiff::Timestamp, jiff::Timestamp)>,
}

/// A focused re-query's answer: the merged flows (as one query's outcome),
/// the focus they were queried for and the number of Explorer queries.
#[derive(Debug)]
pub struct FocusedOutcome {
    pub outcome: QueryOutcome,
    pub focus: Focus,
    pub queries: u32,
}

/// The app overlay's workloads: the PCE's answer to the label filter, and
/// whether it came from the cache.
#[derive(Debug)]
pub struct AppWorkloadsPayload {
    pub workloads: Vec<Workload>,
    pub truncated: bool,
    pub hit: bool,
}

/// One exact-timeline bucket's answer: the blocked connections the focus
/// keeps, whether a query hit the cap (the count is then a lower bound),
/// whether every query was a cache hit, and the run's base query when this
/// bucket resolved it (the first one does).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineBucketOutcome {
    pub connections: u64,
    pub truncated: bool,
    pub hit: bool,
    pub base: Option<Arc<pce::models::TrafficQuery>>,
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
    /// The answer to `Cmd::RunFocused`: one query per side, merged.
    FocusedDone {
        seq: u64,
        result: Result<FocusedOutcome, String>,
    },
    /// Rulesets for `scope` with the cache age in seconds.
    RuleSetsLoaded {
        scope: PolicyScope,
        /// The rulesets, their cache age in seconds, and whether the cache answered.
        result: Result<(Vec<RuleSet>, u64, bool), String>,
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
    /// The labels with the PCE's usage flags (`Cmd::LoadLabelUsage`).
    LabelUsageLoaded(Result<Vec<Label>, String>),
    /// The draft IP lists with the cache age in seconds.
    IpListsLoaded(Result<(Vec<IpList>, u64), String>),
    /// Workloads with the cache age in seconds.
    /// Workloads, the cache age in seconds and whether the PCE holds more
    /// than were returned.
    WorkloadsLoaded(Result<(Vec<Workload>, u64, bool), String>),
    /// The app overlay's workloads (`Cmd::LoadAppWorkloads`) for overlay `seq`.
    AppWorkloadsLoaded {
        seq: u64,
        result: Result<AppWorkloadsPayload, String>,
    },
    /// The answer to `Cmd::RunTimelineBucket` for overlay `seq`, request
    /// `ticket`, bucket `index`.
    TimelineBucketDone {
        seq: u64,
        ticket: u64,
        index: usize,
        result: Result<TimelineBucketOutcome, String>,
    },
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
    /// The runtime applied `:writes on|off`: the guard was built (or refused,
    /// with the reason) or dropped.
    WritesSet(WritesState),
    /// The runtime wrote (or could not write) `:export`'s file: the status
    /// line text.
    Exported(Result<String, String>),
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
    /// `r` on a partial focus: `filter` with the app's labels on its side,
    /// one query per side (`Pce::focused_traffic`), answered with
    /// `Msg::FocusedDone`.
    RunFocused {
        seq: u64,
        filter: TrafficFilter,
        focus: Focus,
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
    /// The draft IP lists for the Objects IP lists tab (the index's own list, cached).
    LoadIpLists(CacheMode),
    /// `u` on the Labels tab: the labels with the PCE's usage flags, one request.
    LoadLabelUsage(CacheMode),
    LoadWorkloads(CacheMode),
    /// The app overlay's workloads: one read filtered by the app's labels.
    LoadAppWorkloads {
        seq: u64,
        filter: WorkloadFilter,
        mode: CacheMode,
    },
    /// `t` in the app overlay: one bucket of the exact timeline, `start` to
    /// `end`, blocked only, the app's labels on its side(s) (one query per
    /// side), in cache slot `timeline:{index}` (`Pce::timeline_bucket`).
    /// `base` is the run's resolved query (`None`: this bucket resolves
    /// `filter` and returns it); `names` the run's name index. Answered
    /// with `Msg::TimelineBucketDone` carrying `seq`, `ticket` and `index`.
    RunTimelineBucket {
        seq: u64,
        ticket: u64,
        index: usize,
        filter: TrafficFilter,
        base: Option<Arc<pce::models::TrafficQuery>>,
        names: Arc<pce::NameIndex>,
        focus: Focus,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
        cancel: CancellationToken,
    },
    LoadEvents {
        filter: EventFilter,
        mode: CacheMode,
    },
    /// Rebuild the client for another profile from the config (no request).
    SwitchProfile(String),
    /// `:writes on|off`: build the current profile's write guard, or drop it
    /// (no request). The runtime answers with `Msg::WritesSet`.
    SetWrites(bool),
    /// `:export`: write this table as CSV to `path`, an existing file only
    /// with `force`. Local only, never the PCE; the runtime answers with
    /// `Msg::Exported`.
    Export {
        path: String,
        force: bool,
        headers: Vec<&'static str>,
        rows: Vec<Vec<String>>,
    },
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
                Cmd::RunFocused {
                    seq: a,
                    filter: fa,
                    focus: xa,
                    mode: ma,
                    ..
                },
                Cmd::RunFocused {
                    seq: b,
                    filter: fb,
                    focus: xb,
                    mode: mb,
                    ..
                },
            ) => a == b && fa == fb && xa == xb && ma == mb,
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
            (Cmd::LoadLabelUsage(a), Cmd::LoadLabelUsage(b)) => a == b,
            (Cmd::LoadWorkloads(a), Cmd::LoadWorkloads(b)) => a == b,
            (
                Cmd::LoadAppWorkloads {
                    seq: a,
                    filter: fa,
                    mode: ma,
                },
                Cmd::LoadAppWorkloads {
                    seq: b,
                    filter: fb,
                    mode: mb,
                },
            ) => a == b && fa == fb && ma == mb,
            (
                Cmd::RunTimelineBucket {
                    seq: a,
                    ticket: ta,
                    index: ia,
                    filter: fa,
                    base: ba,
                    focus: xa,
                    start: sa,
                    end: ea,
                    ..
                },
                Cmd::RunTimelineBucket {
                    seq: b,
                    ticket: tb,
                    index: ib,
                    filter: fb,
                    base: bb,
                    focus: xb,
                    start: sb,
                    end: eb,
                    ..
                },
            ) => {
                a == b
                    && ta == tb
                    && ia == ib
                    && fa == fb
                    && ba == bb
                    && xa == xb
                    && sa == sb
                    && ea == eb
            }
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
            (Cmd::SetWrites(a), Cmd::SetWrites(b)) => a == b,
            (
                Cmd::Export {
                    path: pa,
                    force: fa,
                    headers: ha,
                    rows: ra,
                },
                Cmd::Export {
                    path: pb,
                    force: fb,
                    headers: hb,
                    rows: rb,
                },
            ) => pa == pb && fa == fb && ha == hb && ra == rb,
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
