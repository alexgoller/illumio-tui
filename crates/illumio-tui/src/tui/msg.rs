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
use pce::write::{Batch, BatchInput, Outcome, Plan, StepResult, WriteError};
use pce::{CacheMode, StatsSnapshot};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// A snapshot's workloads as the Workloads baseline (#85).
#[derive(Debug, Clone, PartialEq)]
pub struct WlBaseline {
    /// The snapshot's id: its UTC time.
    pub id: String,
    pub taken_at: jiff::Timestamp,
    pub label: Option<String>,
    pub workloads: Vec<Workload>,
    /// Its workloads read hit the row cap.
    pub truncated: bool,
}

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
    /// The async query hrefs behind the flows (one per query, `in` first
    /// for a focused result): what `D` re-evaluates without a new POST.
    pub hrefs: Vec<String>,
}

/// Compare mode's baseline answer (`Cmd::RunBaseline`).
#[derive(Debug)]
pub struct BaselineOutcome {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub incomplete: bool,
    pub hit: bool,
    /// The filter it ran with: the recent one, its window the baseline's
    /// length, its cap the baseline's.
    pub filter: TrafficFilter,
    /// The window it covered: `[recent start − length, recent start)`.
    pub start: jiff::Timestamp,
    pub end: jiff::Timestamp,
}

/// The generator's baseline answer (`Cmd::RunGenBaseline`, #81).
#[derive(Debug)]
pub struct GenBaselineOutcome {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub incomplete: bool,
    /// Every query was answered from the cache.
    pub hit: bool,
    /// The Explorer queries it took: 2 with the outbound section.
    pub queries: u32,
}

/// A focused re-query's answer: the merged flows (as one query's outcome),
/// the focus they were queried for and the number of Explorer queries.
#[derive(Debug)]
pub struct FocusedOutcome {
    pub outcome: QueryOutcome,
    pub focus: Focus,
    pub queries: u32,
}

/// `D`'s answer: the result again with draft decisions, for the focused
/// result of `focus` (`None`: the unfocused one).
#[derive(Debug)]
pub struct DraftOutcome {
    pub outcome: QueryOutcome,
    pub focus: Option<Focus>,
    /// How old the draft answer is when the cache gave it (the oldest
    /// side's for a focused result); zero for a fresh one.
    pub age: Duration,
}

/// Why `D` brought no draft decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftFailure {
    /// The PCE no longer holds the query to re-evaluate: running it again
    /// is the way, after asking.
    Gone(String),
    Failed(String),
}

/// The app overlay's workloads: the PCE's answer to the label filter, and
/// whether it came from the cache.
#[derive(Debug)]
pub struct AppWorkloadsPayload {
    pub workloads: Vec<Workload>,
    pub truncated: bool,
    pub hit: bool,
}

/// One object list with its cache age in seconds.
#[derive(Debug)]
pub enum ObjectsPayload {
    Labels(Vec<Label>, u64),
    LabelGroups(Vec<LabelGroup>, u64),
    Services(Vec<Service>, u64),
}

/// Events as loaded for the Events screen.
#[derive(Debug)]
pub struct EventsPayload {
    pub events: Vec<Event>,
    /// The cache age in seconds.
    pub age_secs: u64,
    /// A full page: more events may exist than were returned.
    pub truncated: bool,
    /// The `(since, until)` the request used.
    pub window: (String, String),
}

/// A list whose truncation `:hygiene` needs (`Msg::ListCut`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutList {
    DraftRuleSets,
    LabelGroups,
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
    /// The answer to `Cmd::RunFocused`: one query per side, merged.
    FocusedDone {
        seq: u64,
        result: Result<FocusedOutcome, String>,
    },
    /// The answer to `Cmd::RunBaseline`.
    BaselineDone {
        seq: u64,
        result: Result<BaselineOutcome, String>,
    },
    /// The answer to `Cmd::RunGenBaseline`.
    GenBaselineDone {
        seq: u64,
        result: Result<GenBaselineOutcome, String>,
    },
    /// The answer to `Cmd::RunDraft`.
    DraftDone {
        seq: u64,
        result: Result<DraftOutcome, DraftFailure>,
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
    /// What provisioning would change (`Cmd::LoadImpact`).
    ImpactLoaded(Result<crate::provision_impact::ProvisionImpact, String>),
    ObjectsLoaded {
        kind: ObjectKind,
        result: Result<ObjectsPayload, String>,
    },
    /// The labels with the PCE's usage flags (`Cmd::LoadLabelUsage`).
    LabelUsageLoaded(Result<Vec<Label>, String>),
    /// A label's workload probe (`Cmd::CountLabelWorkloads`).
    LabelWorkloadsCounted {
        href: String,
        result: Result<pce::where_used::WorkloadCount, String>,
    },
    /// The pairing profiles (`Cmd::LoadPairingProfiles`).
    PairingProfilesLoaded(Result<Vec<pce::models::PairingProfile>, String>),
    /// Whether a list `:hygiene` reads came back cut short, sent just
    /// before its own answer (`RuleSetsLoaded` of the draft, the label
    /// groups' `ObjectsLoaded`).
    ListCut {
        list: CutList,
        cut: bool,
    },
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
    /// A planned write batch (every request, in order) or why it could not
    /// be planned.
    BatchPlanned(Result<Batch, String>),
    /// What became of each step of a sent batch, with each step's kind
    /// and summary (`create ip_list …`).
    BatchDone {
        kinds: Vec<pce::write::ObjectKind>,
        summaries: Vec<String>,
        /// Where it was sent: `demo (pce.example, org 5)`.
        target: String,
        results: Vec<StepResult>,
    },
    /// What became of a sent bulk workload change (`Cmd::ExecuteBulk`):
    /// always arrives, also when the sending task failed.
    BulkDone(Box<pce::write::BulkReport>),
    /// The confirm's draft check (`Cmd::BulkDraftCheck`) for check `seq`.
    BulkCheckDone {
        seq: u64,
        result: Result<crate::tui::bulk::CheckResult, String>,
    },
    /// `done` of the `of` steps of the batch being sent have an answer.
    BatchProgress {
        done: usize,
        of: usize,
    },
    /// The runtime applied `:writes on|off`: the guard was built (or refused,
    /// with the reason) or dropped.
    WritesSet(WritesState),
    /// The runtime wrote (or could not write) `:export`'s file: the status
    /// line text.
    Exported(Result<String, String>),
    /// `:snapshot`'s file written (the status line text) or why not.
    SnapshotSaved(Result<String, String>),
    /// `b` on Workloads (`Cmd::ListBaselines`): the profile's snapshots
    /// that hold workloads, oldest first, or why they cannot be listed.
    BaselinesListed(Result<Vec<crate::snapshot::SnapshotMeta>, String>),
    /// The snapshot picked (`Cmd::LoadWlBaseline`): its workloads, or why
    /// it cannot be the baseline (unreadable, another PCE's).
    WlBaselineLoaded(Result<WlBaseline, String>),
    /// `:readiness`'s trend (`Cmd::LoadTrend`): the snapshots' readiness.
    TrendLoaded(Result<crate::snapshot::baseline::Trend, String>),
    /// A move from the enforcement overlay (`Cmd::EnforceApp`): the bulk
    /// report and its change record. Always arrives.
    EnforceDone(Box<crate::tui::enforce::EnforceOutcome>),
    /// A rollback (`Cmd::ExecuteRollback`): each group's report and the
    /// rollback record. Always arrives.
    RollbackDone(Box<crate::tui::enforce::RollbackOutcome>),
    /// The re-read before a rollback (`Cmd::RollbackRead`) for overlay `seq`:
    /// the app's workloads now, and whether the read was cut.
    RollbackRead {
        seq: u64,
        result: Result<(Vec<Workload>, bool), String>,
    },
    /// Step 5's query (`Cmd::WatchBlocked`) for watch `seq`.
    WatchDone {
        seq: u64,
        result: Result<crate::tui::enforce::WatchOutcome, String>,
    },
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
        /// The cache slot: `main`, or the exposure query's own (#82).
        slot: &'static str,
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
    /// `c`: compare mode's baseline, `filter` over exactly `[start, end)`
    /// (the window before the recent result's start), in the cache's own
    /// `baseline` slot: the same filter and window again is a hit.
    /// Answered with `Msg::BaselineDone`.
    RunBaseline {
        seq: u64,
        filter: TrafficFilter,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
        cancel: CancellationToken,
    },
    /// `N` in the generator: its baseline, `filter` with the app's labels
    /// on its side over exactly `[start, end)`, one query per side of
    /// `focus`, in the cache's `focus-baseline:{in,out}` slots. Answered
    /// with `Msg::GenBaselineDone`.
    RunGenBaseline {
        seq: u64,
        filter: TrafficFilter,
        focus: Focus,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
        cancel: CancellationToken,
    },
    /// `D`: the result on screen evaluated against the draft policy:
    /// `hrefs` (its async queries) re-evaluated without a new POST, or
    /// with none the queries run again with the draft. `focus`: the
    /// focused result's focus (one query per side). Answered with
    /// `Msg::DraftDone`; the draft result is cached in its own slot.
    RunDraft {
        seq: u64,
        filter: TrafficFilter,
        focus: Option<Focus>,
        hrefs: Vec<String>,
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
    /// `I` / `:impact`: the provision impact (`provision_impact::load`):
    /// the pending list, the rulesets and lists when a ruleset is pending,
    /// one draft traffic query.
    LoadImpact(Box<crate::provision_impact::LoadSpec>),
    LoadObjects {
        kind: ObjectKind,
        mode: CacheMode,
    },
    /// The draft IP lists for the Objects IP lists tab (the index's own list, cached).
    LoadIpLists(CacheMode),
    /// `u` on the Labels tab: the labels with the PCE's usage flags, one request.
    LoadLabelUsage(CacheMode),
    /// Where-used on a label with no workload list loaded: one read of at
    /// most one row filtered by the label (`Pce::label_workload_probe`).
    CountLabelWorkloads {
        href: String,
        mode: CacheMode,
    },
    /// Where-used on a label: the pairing profiles, one cached read.
    LoadPairingProfiles(CacheMode),
    LoadWorkloads(CacheMode),
    /// The app overlay's workloads: one read filtered by the app's labels.
    LoadAppWorkloads {
        seq: u64,
        filter: WorkloadFilter,
        mode: CacheMode,
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
    /// `:snapshot`: the snapshot reads (cached), then the file in `dir`;
    /// answered with `Msg::SnapshotSaved`.
    SaveSnapshot {
        dir: std::path::PathBuf,
        /// The profile; the runtime adds its PCE's host, port and org.
        profile: String,
        label: Option<String>,
    },
    /// `b` on Workloads: the snapshots in `dir` that hold workloads.
    /// Local only, never the PCE; answered with `Msg::BaselinesListed`.
    ListBaselines {
        dir: std::path::PathBuf,
    },
    /// The picked snapshot's workloads, checked to be of the profile's PCE.
    /// Local only; answered with `Msg::WlBaselineLoaded`.
    LoadWlBaseline {
        path: std::path::PathBuf,
        id: String,
    },
    /// `:readiness`: the readiness sections of the snapshots in `dir`.
    /// Local only; answered with `Msg::TrendLoaded`.
    LoadTrend {
        dir: std::path::PathBuf,
    },
    /// Plan a write (one GET for IP lists, services and workloads; none for
    /// labels) and show it in the confirm overlay. Never sends a write.
    PlanWrite(WriteIntent),
    /// `y` on the confirm overlay: send this plan once.
    ExecuteWrite {
        plan: Plan,
        kind: pce::write::ObjectKind,
    },
    /// Plan a write batch: one fresh draft ruleset read (dedupe again,
    /// create-if-missing), one IP list read when it creates IP lists, one
    /// GET per rule update. Never sends a write. `index` resolves names.
    PlanBatch {
        input: Box<BatchInput>,
        index: Arc<pce::NameIndex>,
    },
    /// `y` on the batch confirm: send every step once, in order, stopping
    /// at the first failure.
    ExecuteBatch(Batch),
    /// `y` on the bulk confirm: the re-read (one list read past the
    /// cache), the label creates, the `bulk_update` chunks; answered with
    /// `Msg::BulkDone`.
    ExecuteBulk(Box<pce::write::BulkPlan>),
    /// `d` on the bulk confirm: one Explorer query of the targets' traffic
    /// (as destinations) over `window`, with the draft decision; answered
    /// with `Msg::BulkCheckDone`. Never sent without the key.
    BulkDraftCheck {
        seq: u64,
        hrefs: Vec<String>,
        window: Duration,
        /// The window as typed (`24h`), for the result's text.
        window_text: String,
        max_results: u32,
        cancel: CancellationToken,
    },
    /// `y` on a bulk confirm the enforcement overlay planned: the records'
    /// directory probed, the record written with every workload pending,
    /// then the bulk send (as `ExecuteBulk`), then the record with the
    /// results; answered with `Msg::EnforceDone`.
    EnforceApp {
        plan: Box<pce::write::BulkPlan>,
        send: Box<crate::tui::enforce::EnforceSend>,
    },
    /// `R` on step 5: the records' directory `dir` probed first (as the
    /// CLI does, before any request), then the app's workloads now (one
    /// list read past the cache); answered with `Msg::RollbackRead`.
    RollbackRead {
        seq: u64,
        filter: WorkloadFilter,
        dir: std::path::PathBuf,
    },
    /// `y` on the rollback preview: the records' directory probed, the
    /// rollback record written, each group's bulk send in order (stopping
    /// at the first that fails), the record with the results; answered
    /// with `Msg::RollbackDone`.
    ExecuteRollback(Box<crate::tui::enforce::RollbackSend>),
    /// `r` on step 5: one Explorer query from the change time to now, the
    /// app on either side, blocked flows only; answered with
    /// `Msg::WatchDone`.
    WatchBlocked {
        seq: u64,
        app: pce::traffic::GroupKey,
        keys: Vec<String>,
        since: jiff::Timestamp,
        max_results: u32,
        cancel: CancellationToken,
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
                    slot: sa,
                    ..
                },
                Cmd::RunQuery {
                    seq: b,
                    filter: fb,
                    mode: mb,
                    slot: sb,
                    ..
                },
            ) => a == b && fa == fb && ma == mb && sa == sb,
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
                Cmd::RunBaseline {
                    seq: a,
                    filter: fa,
                    start: sa,
                    end: ea,
                    ..
                },
                Cmd::RunBaseline {
                    seq: b,
                    filter: fb,
                    start: sb,
                    end: eb,
                    ..
                },
            ) => a == b && fa == fb && sa == sb && ea == eb,
            (
                Cmd::RunGenBaseline {
                    seq: a,
                    filter: fa,
                    focus: xa,
                    start: sa,
                    end: ea,
                    ..
                },
                Cmd::RunGenBaseline {
                    seq: b,
                    filter: fb,
                    focus: xb,
                    start: sb,
                    end: eb,
                    ..
                },
            ) => a == b && fa == fb && xa == xb && sa == sb && ea == eb,
            (
                Cmd::RunDraft {
                    seq: a,
                    filter: fa,
                    focus: xa,
                    hrefs: ha,
                    ..
                },
                Cmd::RunDraft {
                    seq: b,
                    filter: fb,
                    focus: xb,
                    hrefs: hb,
                    ..
                },
            ) => a == b && fa == fb && xa == xb && ha == hb,
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
            (Cmd::LoadImpact(a), Cmd::LoadImpact(b)) => a == b,
            (Cmd::LoadObjects { kind: ka, mode: ma }, Cmd::LoadObjects { kind: kb, mode: mb }) => {
                ka == kb && ma == mb
            }
            (Cmd::LoadIpLists(a), Cmd::LoadIpLists(b)) => a == b,
            (Cmd::LoadLabelUsage(a), Cmd::LoadLabelUsage(b)) => a == b,
            (
                Cmd::CountLabelWorkloads { href: ha, mode: ma },
                Cmd::CountLabelWorkloads { href: hb, mode: mb },
            ) => ha == hb && ma == mb,
            (Cmd::LoadPairingProfiles(a), Cmd::LoadPairingProfiles(b)) => a == b,
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
            (Cmd::PlanBatch { input: a, .. }, Cmd::PlanBatch { input: b, .. }) => a == b,
            (Cmd::ExecuteBatch(a), Cmd::ExecuteBatch(b)) => a.lines() == b.lines(),
            (Cmd::EnforceApp { plan: a, send: sa }, Cmd::EnforceApp { plan: b, send: sb }) => {
                a.hrefs() == b.hrefs()
                    && a.change == b.change
                    && a.confirmed == b.confirmed
                    && sa == sb
            }
            (
                Cmd::RollbackRead {
                    seq: a,
                    filter: fa,
                    dir: da,
                },
                Cmd::RollbackRead {
                    seq: b,
                    filter: fb,
                    dir: db,
                },
            ) => a == b && fa == fb && da == db,
            (Cmd::ExecuteRollback(a), Cmd::ExecuteRollback(b)) => {
                a.record_path == b.record_path
                    && a.plans.len() == b.plans.len()
                    && a.plans
                        .iter()
                        .zip(&b.plans)
                        .all(|((ta, pa), (tb, pb))| ta == tb && pa.hrefs() == pb.hrefs())
            }
            (
                Cmd::WatchBlocked {
                    seq: a,
                    app: aa,
                    since: sa,
                    max_results: ma,
                    ..
                },
                Cmd::WatchBlocked {
                    seq: b,
                    app: ab,
                    since: sb,
                    max_results: mb,
                    ..
                },
            ) => a == b && aa == ab && sa == sb && ma == mb,
            (Cmd::ExecuteBulk(a), Cmd::ExecuteBulk(b)) => {
                a.hrefs() == b.hrefs() && a.change == b.change && a.confirmed == b.confirmed
            }
            (
                Cmd::BulkDraftCheck {
                    seq: a,
                    hrefs: ha,
                    window: wa,
                    max_results: ma,
                    ..
                },
                Cmd::BulkDraftCheck {
                    seq: b,
                    hrefs: hb,
                    window: wb,
                    max_results: mb,
                    ..
                },
            ) => a == b && ha == hb && wa == wb && ma == mb,
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
            (Cmd::ListBaselines { dir: a }, Cmd::ListBaselines { dir: b })
            | (Cmd::LoadTrend { dir: a }, Cmd::LoadTrend { dir: b }) => a == b,
            (
                Cmd::LoadWlBaseline { path: pa, id: ia },
                Cmd::LoadWlBaseline { path: pb, id: ib },
            ) => pa == pb && ia == ib,
            _ => false,
        }
    }
}
