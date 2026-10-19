//! Change records of an app's move to enforcement (#56): what each
//! workload was before the move, what it was set to, and what became of
//! it, so a rollback restores exactly that and skips a workload someone
//! else changed since.
//!
//! A record is one JSON file under
//! `$XDG_STATE_HOME/illumio-tui/enforcements/` (else
//! `~/.local/state/illumio-tui/enforcements/`), named
//! `<app>-<UTC time>.json`, written atomically (a temporary file renamed).
//! A rollback writes its own record (`rollback_of` names the original);
//! no record is ever deleted. Records hold hrefs and hostnames only, never
//! credentials.
//!
//! Pure apart from the file helpers: building a record from a bulk plan
//! and report, choosing the newest record, planning a rollback from a
//! re-read, and grouping the blocked flows `--watch` reports.
use crate::marks::ActiveMarks;
use crate::models::{
    ActorMatch, PolicyDecision, TrafficQuery, Transmission, Workload, WorkloadFilter,
};
use crate::readiness::{AppReadiness, Gap, accepted, is_unicast, tuples, workload_group};
use crate::resolve::NameIndex;
use crate::traffic::{
    Direction, GroupKey, ResolvedFlow, TrafficQueryBuilder, group_key, keys_or_default,
};
use crate::write::bulk::{
    BulkItem, COUNT_ABOVE, EnforcementMode, ItemResult, ItemStatus, VisibilityLevel,
};
use crate::write::{BatchOrigin, BulkChange, BulkPlan, BulkTarget, WriteError, plan_bulk};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// An enforcement mode and visibility level, as the PCE names them
/// (`full`, `flow_summary`); `None` where unknown or (for a target) left
/// as it was.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeState {
    pub mode: Option<String>,
    pub visibility: Option<String>,
}

impl ModeState {
    pub fn of(w: &Workload) -> Self {
        Self {
            mode: w.enforcement_mode.clone(),
            visibility: w.visibility_level.clone(),
        }
    }

    pub fn target(mode: EnforcementMode, visibility: Option<VisibilityLevel>) -> Self {
        Self {
            mode: Some(mode.as_str().into()),
            visibility: visibility.map(|v| v.as_str().into()),
        }
    }

    /// `full`, `full · flow_drops`, `?`.
    pub fn text(&self) -> String {
        let mode = self.mode.as_deref().unwrap_or("?");
        match &self.visibility {
            Some(v) => format!("{mode} · {v}"),
            None => mode.to_string(),
        }
    }

    /// `now` is still as this target set it: the same mode, and the same
    /// level where the target names one.
    pub fn holds_in(&self, now: &ModeState) -> bool {
        self.mode.is_some()
            && self.mode == now.mode
            && (self.visibility.is_none() || self.visibility == now.visibility)
    }
}

/// The readiness the move was made on (absent from a rollback record).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordReadiness {
    pub verdict: String,
    pub coverage: Option<f64>,
    pub window: String,
    pub flows: u64,
    /// Moved with `--force` (or the typed `enforce`) though not ready.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forced: bool,
}

impl RecordReadiness {
    pub fn of(r: &AppReadiness, forced: bool) -> Self {
        Self {
            verdict: r.verdict.as_str().into(),
            coverage: r.coverage.ratio,
            window: r.evidence.window.clone(),
            flows: r.evidence.flows,
            forced,
        }
    }
}

/// One workload of a move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordWorkload {
    pub href: String,
    pub hostname: String,
    /// Its mode and level before the move: what a rollback restores.
    pub before: ModeState,
    /// What the move set (the record's `to`; for a rollback, the
    /// restored `before`): what a rollback expects to find still.
    pub after: ModeState,
    /// What became of it: `updated`, `failed`, `unanswered`, `not_sent`,
    /// `changed`, `missing`; `pending` in the record written before the
    /// send (the send did not finish if it is still there).
    pub result: String,
}

/// The result of a workload in the record written before the send.
pub const PENDING: &str = "pending";

impl RecordWorkload {
    /// The move may have applied to it (`updated`, `unanswered`, or
    /// `pending`: a send that never reported).
    pub fn moved(&self) -> bool {
        matches!(self.result.as_str(), "updated" | "unanswered" | PENDING)
    }

    /// Whether it applied is unknown (`unanswered`, `pending`).
    pub fn uncertain(&self) -> bool {
        matches!(self.result.as_str(), "unanswered" | PENDING)
    }

    /// What a rollback sets: the previous mode, and the previous level
    /// only when the move set a level (else the level is left as it is:
    /// someone may have changed it since).
    pub fn restore(&self) -> ModeState {
        ModeState {
            mode: self.before.mode.clone(),
            visibility: self
                .after
                .visibility
                .as_ref()
                .and(self.before.visibility.clone()),
        }
    }
}

/// A change record: one app's move (or a rollback of one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeRecord {
    /// `shop/prod`.
    pub app: String,
    pub profile: String,
    /// The PCE host.
    pub pce: String,
    pub org_id: u64,
    /// When the move was sent (UTC, to the second): `--watch` starts here.
    pub at: String,
    /// The target of a move; `None` on a rollback (each workload's
    /// `after` is its own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<ModeState>,
    /// A rollback: the record it rolled back (its path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<RecordReadiness>,
    pub workloads: Vec<RecordWorkload>,
}

/// Where a change was made: the profile, PCE host and org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordOrigin {
    pub profile: String,
    pub pce: String,
    pub org_id: u64,
}

/// The JSON word of an item status.
pub fn result_word(s: ItemStatus) -> &'static str {
    match s {
        ItemStatus::Updated => "updated",
        ItemStatus::Failed => "failed",
        ItemStatus::Unanswered => "unanswered",
        ItemStatus::NotSent => "not_sent",
        ItemStatus::Changed => "changed",
        ItemStatus::Missing => "missing",
        ItemStatus::Skipped => "skipped",
    }
}

/// `t` to the second, as the record's `at`.
pub fn record_time(t: Timestamp) -> Timestamp {
    Timestamp::from_second(t.as_second()).unwrap_or(t)
}

impl ChangeRecord {
    /// The record of a sent bulk move: each planned workload (`items`,
    /// the plan's; workloads the plan skipped were not touched) with its
    /// state before, `after` (each item's own target), and its result in
    /// `results` (the bulk reports' items; none: `not_sent`). `at` is when the move was sent.
    pub fn of_move(
        app: &str,
        origin: &RecordOrigin,
        at: Timestamp,
        to: Option<ModeState>,
        afters: &[(BulkItem, ModeState)],
        results: &[ItemResult],
    ) -> Self {
        let workloads = afters
            .iter()
            .map(|(i, after)| {
                let result = results
                    .iter()
                    .find(|r| r.href == i.href)
                    .map_or("not_sent", |r| result_word(r.status));
                RecordWorkload {
                    href: i.href.clone(),
                    hostname: i.name.clone(),
                    before: ModeState {
                        mode: i.base.enforcement.clone(),
                        visibility: i.base.visibility.clone(),
                    },
                    after: after.clone(),
                    result: result.into(),
                }
            })
            .collect();
        Self {
            app: app.into(),
            profile: origin.profile.clone(),
            pce: origin.pce.clone(),
            org_id: origin.org_id,
            at: record_time(at).to_string(),
            to,
            rollback_of: None,
            readiness: None,
            workloads,
        }
    }

    /// Some workload may have moved: a record is worth writing.
    pub fn any_moved(&self) -> bool {
        self.workloads.iter().any(RecordWorkload::moved)
    }

    /// The record written before the send: every workload `pending`.
    pub fn pending(mut self) -> Self {
        for w in &mut self.workloads {
            w.result = PENDING.into();
        }
        self
    }

    /// Made on `o`'s PCE and org.
    pub fn made_on(&self, o: &RecordOrigin) -> bool {
        self.pce == o.pce && self.org_id == o.org_id
    }

    pub fn at_time(&self) -> Option<Timestamp> {
        self.at.parse().ok()
    }

    pub fn is_rollback(&self) -> bool {
        self.rollback_of.is_some()
    }

    /// `shop_prod-20261019T100000.123Z.json` (`-rollback` before the
    /// extension for a rollback); `now` gives the milliseconds that keep
    /// two records of one second apart.
    pub fn file_name(&self, now: Timestamp) -> String {
        let ts = now.strftime("%Y%m%dT%H%M%S%.3fZ").to_string();
        let kind = if self.is_rollback() { "-rollback" } else { "" };
        format!("{}-{ts}{kind}.json", record_stem(&self.app))
    }
}

/// The longest app part of a record's file name.
pub const STEM_MAX: usize = 100;

/// The app as a record file name's first part: [`file_safe`], cut to
/// [`STEM_MAX`] characters plus a short hash of the whole app when longer.
pub fn record_stem(app: &str) -> String {
    let safe = file_safe(app);
    if safe.len() <= STEM_MAX {
        return safe;
    }
    // FNV-1a: stable across runs and builds.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in app.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{}~{:08x}", &safe[..STEM_MAX], h as u32)
}

/// The app as a file name part: letters, digits, `-`, `_`, `.` kept, any
/// other character (the `/` between values) as `_`.
pub fn file_safe(app: &str) -> String {
    app.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `$XDG_STATE_HOME/illumio-tui/enforcements`, else
/// `$HOME/.local/state/illumio-tui/enforcements` (beside `writes.jsonl`);
/// `None` when neither is set.
pub fn records_dir_from(xdg_state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    crate::write::JsonlAudit::path_from(xdg_state_home, home)
        .and_then(|p| p.parent().map(|d| d.join("enforcements")))
}

/// [`records_dir_from`] with the environment.
pub fn default_records_dir() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    records_dir_from(var("XDG_STATE_HOME").as_deref(), var("HOME").as_deref())
}

/// Creates `dir` and checks a file can be written there (a probe file,
/// written, synced and removed): before any request of a move, so a
/// record that cannot be written stops it with nothing sent.
pub fn probe_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".probe-{}", std::process::id()));
    let r = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        f.write_all(b"probe")?;
        f.sync_all()
    })();
    let _ = std::fs::remove_file(&probe);
    r
}

/// `body` in a synced temporary file in `dir`.
fn temp_file(dir: &Path, name: &str, body: &[u8]) -> io::Result<PathBuf> {
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let mut f = std::fs::File::create(&tmp)?;
    let r = f.write_all(body).and_then(|_| f.sync_all());
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

/// Syncs the directory, so a rename or link in it survives a crash.
fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// Writes `rec` into `dir` (created if missing) as a new file: a synced
/// temporary file hard-linked into place, which fails rather than
/// replace an existing record (then `-2`, `-3`, ... is tried). Returns
/// its path.
pub fn write_record(dir: &Path, rec: &ChangeRecord, now: Timestamp) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec_pretty(rec).map_err(io::Error::other)?;
    let name = rec.file_name(now);
    let tmp = temp_file(dir, &name, &body)?;
    let mut n = 1;
    let result = loop {
        let path = if n == 1 {
            dir.join(&name)
        } else {
            dir.join(name.replacen(".json", &format!("-{n}.json"), 1))
        };
        match std::fs::hard_link(&tmp, &path) {
            Ok(()) => break Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
            Err(e) => break Err(e),
        }
    };
    let _ = std::fs::remove_file(&tmp);
    sync_dir(dir);
    result
}

/// Replaces the record at `path` with `rec` (the results after the
/// send): a synced temporary file renamed over it.
pub fn rewrite_record(path: &Path, rec: &ChangeRecord) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let body = serde_json::to_vec_pretty(rec).map_err(io::Error::other)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("record");
    let tmp = temp_file(dir, name, &body)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(())
}

pub fn read_record(path: &Path) -> io::Result<ChangeRecord> {
    let text = std::fs::read(path)?;
    serde_json::from_slice(&text).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a change record: {e}", path.display()),
        )
    })
}

/// Every record in `dir` (any app), with its path; files that are not
/// records are passed over; no directory: none.
fn all_records(dir: &Path, stem: Option<&str>) -> io::Result<Vec<(String, PathBuf, ChangeRecord)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let prefix = stem.map(|s| format!("{s}-"));
    let mut out = Vec::new();
    for e in entries {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        if let Some(p) = &prefix
            && !name.starts_with(p.as_str())
        {
            continue;
        }
        let path = e.path();
        if let Ok(rec) = read_record(&path) {
            out.push((name, path, rec));
        }
    }
    Ok(out)
}

/// The app's newest move record in `dir` (rollbacks left out; with
/// `origin`, only those made on its PCE and org), by `at`, then by file
/// name; `None` when there is none (or no directory).
pub fn newest_record(
    dir: &Path,
    app: &str,
    origin: Option<&RecordOrigin>,
) -> io::Result<Option<(PathBuf, ChangeRecord)>> {
    let found = all_records(dir, Some(&record_stem(app)))?
        .into_iter()
        .filter(|(_, _, r)| r.app == app && !r.is_rollback())
        .filter(|(_, _, r)| origin.is_none_or(|o| r.made_on(o)))
        .filter_map(|(n, p, r)| r.at_time().map(|t| (t, n, p, r)));
    Ok(found
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, _, p, r)| (p, r)))
}

/// The rollback record in `dir` whose `rollback_of` names `record`, if
/// any (the newest). Matched by file name: `rollback_of` keeps the path as
/// it was given (`--record ./x.json`), and record names are unique.
pub fn rolled_back_by(dir: &Path, record: &Path) -> io::Result<Option<PathBuf>> {
    let want = record.file_name();
    let found = all_records(dir, None)?
        .into_iter()
        .filter(|(_, _, r)| {
            r.rollback_of
                .as_deref()
                .is_some_and(|o| want.is_some() && Path::new(o).file_name() == want)
        })
        .max_by(|a, b| a.0.cmp(&b.0));
    Ok(found.map(|(_, p, _)| p))
}

/// A workload a rollback leaves as it is, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RollbackSkip {
    pub href: String,
    pub hostname: String,
    pub reason: String,
}

/// Workloads restored to one `(mode, visibility)`: one bulk request (per
/// 100).
#[derive(Debug, Clone, PartialEq)]
pub struct RollbackGroup {
    pub mode: EnforcementMode,
    pub visibility: Option<VisibilityLevel>,
    /// The workloads as re-read, in the record's order.
    pub workloads: Vec<Workload>,
}

impl RollbackGroup {
    pub fn target(&self) -> ModeState {
        ModeState::target(self.mode, self.visibility)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RollbackPlan {
    pub groups: Vec<RollbackGroup>,
    pub skipped: Vec<RollbackSkip>,
}

impl RollbackPlan {
    /// The number of workloads it restores.
    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.workloads.len()).sum()
    }
}

/// What a rollback of `rec` does, from `current` (the workloads re-read
/// just now): each workload the move changed and that is still as the
/// move set it goes back to its `before`, grouped by `(mode, visibility)`
/// in the order first seen; the level goes back only when the move set
/// one. Skipped and reported: a workload the move did not change (its
/// result), one gone, one changed by someone else since, one whose
/// unanswered or unfinished move did not apply, one whose previous mode
/// is unknown.
pub fn plan_rollback(rec: &ChangeRecord, current: &[Workload]) -> RollbackPlan {
    let mut plan = RollbackPlan::default();
    for w in &rec.workloads {
        let skip = |reason: String| RollbackSkip {
            href: w.href.clone(),
            hostname: w.hostname.clone(),
            reason,
        };
        if !w.moved() {
            plan.skipped
                .push(skip(format!("not moved by that change ({})", w.result)));
            continue;
        }
        let Some(now) = current
            .iter()
            .find(|c| c.href == w.href && c.deleted != Some(true))
        else {
            plan.skipped
                .push(skip("not found among the app's workloads".into()));
            continue;
        };
        let state = ModeState::of(now);
        let restore = w.restore();
        if !w.after.holds_in(&state) {
            let reason = if w.uncertain() && restore.holds_in(&state) {
                format!(
                    "not applied ({}: still {})",
                    if w.result == PENDING {
                        "the send did not finish"
                    } else {
                        "not answered"
                    },
                    state.text()
                )
            } else {
                format!(
                    "changed since ({} now, {} set): left as it is",
                    state.text(),
                    w.after.text()
                )
            };
            plan.skipped.push(skip(reason));
            continue;
        }
        let mode = w.before.mode.as_deref().and_then(EnforcementMode::parse);
        let Some(mode) = mode else {
            plan.skipped.push(skip(format!(
                "previous mode unknown ({})",
                w.before.mode.as_deref().unwrap_or("none recorded")
            )));
            continue;
        };
        // The level only when the move set one.
        let visibility = restore
            .visibility
            .as_deref()
            .and_then(VisibilityLevel::parse);
        match plan
            .groups
            .iter_mut()
            .find(|g| g.mode == mode && g.visibility == visibility)
        {
            Some(g) => g.workloads.push(now.clone()),
            None => plan.groups.push(RollbackGroup {
                mode,
                visibility,
                workloads: vec![now.clone()],
            }),
        }
    }
    plan
}

/// What `--watch` found: the blocked flows involving the app, grouped as
/// tuples (inbound: the source peer; outbound: the destination peer).
#[derive(Debug, Clone, Default, Serialize)]
pub struct BlockedSince {
    /// Blocked unicast flows involving the app (accepted ones left out).
    pub flows: u64,
    pub connections: u64,
    /// Blocked flows a live `squelch` or `accept-blocked` mark accepts:
    /// counted, not alerted on.
    pub accepted: u64,
    pub tuples: Vec<Gap>,
}

/// The flows of `flows` with the reported decision `blocked` and `app`
/// on either side (a flow inside the app counts once, as inbound), grouped
/// as readiness groups its gaps.
pub fn blocked_since(
    flows: &[ResolvedFlow],
    app: &GroupKey,
    scope_keys: &[String],
    marks: Option<&ActiveMarks>,
) -> BlockedSince {
    let keys = keys_or_default(scope_keys);
    let mut out = BlockedSince::default();
    let mut inbound = Vec::new();
    let mut outbound = Vec::new();
    for f in flows.iter().filter(|f| is_unicast(f)) {
        if f.policy_decision != PolicyDecision::Blocked {
            continue;
        }
        let dst = group_key(&f.dst, &keys) == *app;
        let src = group_key(&f.src, &keys) == *app;
        if !dst && !src {
            continue;
        }
        if accepted(marks, f) {
            out.accepted += 1;
            continue;
        }
        out.flows += 1;
        out.connections += f.num_connections;
        if dst {
            inbound.push(f);
        } else {
            outbound.push(f);
        }
    }
    out.tuples.extend(tuples(&inbound, &keys, Direction::In));
    out.tuples.extend(tuples(&outbound, &keys, Direction::Out));
    out.tuples.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.direction.cmp(&b.direction))
            .then_with(|| a.peer.cmp(&b.peer))
            .then_with(|| a.service.cmp(&b.service))
    });
    out
}

/// The app's workloads in `ws` (not deleted, of the app under `keys`):
/// what a move selects. The CLI and the TUI select the same way.
pub fn app_workloads<'a>(
    ws: &'a [Workload],
    app: &GroupKey,
    keys: &[String],
    idx: &NameIndex,
) -> Vec<&'a Workload> {
    ws.iter()
        .filter(|w| w.deleted != Some(true))
        .filter(|w| workload_group(w, keys, idx).as_ref() == Some(app))
        .collect()
}

/// The blocked-since query of `--watch` and the TUI's Watch step: unicast
/// flows from `since` to `until` whose reported decision is `blocked`,
/// the app (`labels`) on either side (`or`), at most `cap` rows.
pub fn watch_query(
    since: Timestamp,
    until: Timestamp,
    labels: Vec<ActorMatch>,
    cap: u32,
) -> crate::Result<TrafficQuery> {
    let mut q = TrafficQueryBuilder::between(since, until)
        .transmission(&[Transmission::Unicast])
        .decisions(vec![PolicyDecision::Blocked])
        .max_results(cap)
        .source(labels.clone())
        .destination(labels)
        .build()?;
    q.sources_destinations_query_op = "or".into();
    Ok(q)
}

/// A rollback's bulk plans, one per `(mode, visibility)` that restores
/// something, and whether its count must be typed.
#[derive(Debug, Clone)]
pub struct RollbackPlans {
    pub plans: Vec<(ModeState, BulkPlan)>,
    /// The workloads restored.
    pub count: usize,
    /// Several to full or selective, or more than 25: the typed count
    /// covers the whole rollback.
    pub needs_count: bool,
}

/// The bulk plans of `rp`, each re-reading the app's workloads by `reread`.
pub fn rollback_plans(
    rp: &RollbackPlan,
    reread: &WorkloadFilter,
    origin: Option<BatchOrigin>,
) -> Result<RollbackPlans, WriteError> {
    let none = NameIndex::default();
    let mut plans: Vec<(ModeState, BulkPlan)> = Vec::new();
    for g in &rp.groups {
        let targets: Vec<BulkTarget> = g
            .workloads
            .iter()
            .map(|w| BulkTarget::of(w, &none))
            .collect();
        let plan = plan_bulk(
            &targets,
            &BulkChange::enforce(g.mode, g.visibility),
            reread.clone(),
            origin.clone(),
        )?;
        plans.push((g.target(), plan));
    }
    plans.retain(|(_, p)| !p.items.is_empty());
    let count: usize = plans.iter().map(|(_, p)| p.items.len()).sum();
    let needs_count = plans.iter().any(|(_, p)| p.needs_count()) || count > COUNT_ABOVE;
    Ok(RollbackPlans {
        plans,
        count,
        needs_count,
    })
}

/// Each planned workload with the state its plan sets it to.
pub fn afters(plans: &[(ModeState, &BulkPlan)]) -> Vec<(BulkItem, ModeState)> {
    plans
        .iter()
        .flat_map(|(to, p)| p.items.iter().map(move |i| (i.clone(), to.clone())))
        .collect()
}

/// The record written before the send: `rec` with every workload
/// `pending`, as a new file in `dir`.
pub fn write_pending(dir: &Path, rec: &ChangeRecord, now: Timestamp) -> io::Result<PathBuf> {
    write_record(dir, &rec.clone().pending(), now)
}

/// `rec` with each workload's result from `results` (the bulk reports'
/// items; none: `not_sent`).
pub fn with_results(
    rec: &ChangeRecord,
    afters: &[(BulkItem, ModeState)],
    results: &[ItemResult],
) -> ChangeRecord {
    let origin = RecordOrigin {
        profile: rec.profile.clone(),
        pce: rec.pce.clone(),
        org_id: rec.org_id,
    };
    ChangeRecord {
        workloads: ChangeRecord::of_move(
            &rec.app,
            &origin,
            Timestamp::now(),
            None,
            afters,
            results,
        )
        .workloads,
        ..rec.clone()
    }
}

/// Where a record that could not be rewritten at `path` is saved instead:
/// beside the records' directory (the state directory),
/// `<file stem>.unsaved.json`.
pub fn unsaved_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let state = dir.parent().unwrap_or(dir);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("record");
    state.join(format!("{stem}.unsaved.json"))
}

/// Saves `rec` at [`unsaved_path`] of `path` (a new file); its path.
pub fn write_unsaved(path: &Path, rec: &ChangeRecord) -> io::Result<PathBuf> {
    let to = unsaved_path(path);
    let body = serde_json::to_vec_pretty(rec).map_err(io::Error::other)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&to)?;
    f.write_all(&body)?;
    f.sync_all()?;
    Ok(to)
}

#[cfg(test)]
mod tests;
