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
use crate::models::{PolicyDecision, Workload};
use crate::readiness::{AppReadiness, Gap, accepted, is_unicast, tuples};
use crate::traffic::{Direction, GroupKey, ResolvedFlow, group_key, keys_or_default};
use crate::write::bulk::{BulkItem, BulkReport, EnforcementMode, ItemStatus, VisibilityLevel};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::io;
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
    /// `changed`, `missing`.
    pub result: String,
}

impl RecordWorkload {
    /// The move may have applied to it (`updated`, or `unanswered`).
    pub fn moved(&self) -> bool {
        matches!(self.result.as_str(), "updated" | "unanswered")
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
    /// `report`. `at` is when the move was sent.
    pub fn of_move(
        app: &str,
        origin: &RecordOrigin,
        at: Timestamp,
        to: Option<ModeState>,
        afters: &[(BulkItem, ModeState)],
        report: &BulkReport,
    ) -> Self {
        let workloads = afters
            .iter()
            .map(|(i, after)| {
                let result = report
                    .items
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
        format!("{}-{ts}{kind}.json", file_safe(&self.app))
    }
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

/// Writes `rec` into `dir` (created if missing) as a new file: a
/// temporary file renamed into place, never over an existing record.
/// Returns its path.
pub fn write_record(dir: &Path, rec: &ChangeRecord, now: Timestamp) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec_pretty(rec).map_err(io::Error::other)?;
    let name = rec.file_name(now);
    let mut path = dir.join(&name);
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(name.replacen(".json", &format!("-{n}.json"), 1));
    }
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("record"),
        std::process::id()
    ));
    std::fs::write(&tmp, &body)?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(path)
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

/// The app's newest move record in `dir` (rollbacks left out), by `at`,
/// then by file name; `None` when there is none (or no directory). A file
/// that is not a record is passed over.
pub fn newest_record(dir: &Path, app: &str) -> io::Result<Option<(PathBuf, ChangeRecord)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let prefix = format!("{}-", file_safe(app));
    let mut found: Vec<(Timestamp, String, PathBuf, ChangeRecord)> = Vec::new();
    for e in entries {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".json") {
            continue;
        }
        let path = e.path();
        let Ok(rec) = read_record(&path) else {
            continue;
        };
        if rec.app != app || rec.is_rollback() {
            continue;
        }
        let Some(at) = rec.at_time() else {
            continue;
        };
        found.push((at, name, path, rec));
    }
    Ok(found
        .into_iter()
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, _, p, r)| (p, r)))
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
/// in the order first seen. Skipped and reported: a workload the move did
/// not change (its result), one gone, one changed by someone else since,
/// one whose previous mode is unknown.
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
        if !w.after.holds_in(&state) {
            plan.skipped.push(skip(format!(
                "changed since ({} now, {} set): left as it is",
                state.text(),
                w.after.text()
            )));
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
        let visibility = w
            .before
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

#[cfg(test)]
mod tests;
