//! An app from observed traffic to enforcement in the TUI (#55, #56):
//! `:readiness` over the current traffic result, and the enforcement
//! overlay for one app (`:enforce`, `e` in the app overlay) with its five
//! steps. Pure state, readiness computed from what the model holds
//! (`pce::readiness`, no request), and the change-record steps the runtime
//! runs around a move or a rollback, in the order `illumio-tui enforce`
//! runs them: probe the records' directory before any request, the record
//! with every workload `pending` before the send, the results after it.
use crate::tui::model::Model;
use pce::enforcement::{
    BlockedSince, ChangeRecord, ModeState, RecordOrigin, RecordReadiness, RollbackSkip,
    newest_record, probe_dir, rewrite_record, rolled_back_by, with_results, write_pending,
    write_unsaved,
};
use pce::readiness::{AppReadiness, ReadinessInput, Verdict, readiness};
use pce::traffic::{Focus, GroupKey, ResolvedFlow, keys_or_default};
use pce::write::bulk::{BulkItem, BulkReport, ItemResult};
use pce::write::{BulkPlan, EnforcementMode, VisibilityLevel};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// The word typed to move an app readiness does not allow.
pub const ENFORCE_WORD: &str = "enforce";

/// The enforcement overlay's steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Readiness,
    Rules,
    Provision,
    Enforce,
    Watch,
}

impl Step {
    pub const ALL: [Step; 5] = [
        Step::Readiness,
        Step::Rules,
        Step::Provision,
        Step::Enforce,
        Step::Watch,
    ];

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Readiness => "Readiness",
            Step::Rules => "Rules",
            Step::Provision => "Provision",
            Step::Enforce => "Enforce",
            Step::Watch => "Watch",
        }
    }

    /// `Tab`.
    pub fn next(self) -> Self {
        Step::ALL[(self.index() + 1) % Step::ALL.len()]
    }

    /// `BackTab`.
    pub fn prev(self) -> Self {
        Step::ALL[(self.index() + Step::ALL.len() - 1) % Step::ALL.len()]
    }
}

/// What readiness reads that is not loaded yet (its load is on the way or
/// failed): the verdict waits for it, and no move is offered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Missing {
    pub traffic: bool,
    pub workloads: bool,
    pub pending: bool,
    pub rulesets: bool,
}

impl Missing {
    pub fn of(model: &Model) -> Self {
        Self {
            traffic: model.traffic.data.is_none(),
            workloads: model.workloads.loaded_at.is_none(),
            pending: model.policy.pending.is_none(),
            rulesets: model.draft_rulesets().is_none(),
        }
    }

    pub fn any(&self) -> bool {
        self.traffic || self.workloads || self.pending || self.rulesets
    }

    /// `traffic, workloads, pending changes`: what is missing.
    pub fn text(&self) -> String {
        [
            (self.traffic, "traffic"),
            (self.workloads, "workloads"),
            (self.pending, "pending changes"),
            (self.rulesets, "draft rulesets"),
        ]
        .into_iter()
        .filter(|(m, _)| *m)
        .map(|(_, s)| s)
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// Readiness of `apps` (every app of the result on screen when empty),
/// from what the model holds: the result on screen and its draft
/// decisions, the cached workloads, pending changes and draft rulesets,
/// the active marks. What is not loaded counts as empty ([`Missing`] says
/// so). No request; nothing without the name index.
pub fn compute(model: &Model, apps: &[GroupKey], gaps: bool) -> Vec<AppReadiness> {
    let Some(idx) = model.index() else {
        return Vec::new();
    };
    let t = &model.traffic;
    let d = t.data.as_ref();
    let window = d
        .map(|d| &d.filter)
        .unwrap_or(&t.filter)
        .window_duration()
        .unwrap_or_default();
    let marks = t.active_marks();
    let input = ReadinessInput {
        flows: t.loaded_flows(),
        draft: t.draft_loaded(),
        truncated: d.is_some_and(|d| d.truncated),
        incomplete: d.is_some_and(|d| d.incomplete),
        gaps,
        window,
        workloads: &model.workloads.raw,
        pending: model.policy.pending.as_deref().unwrap_or_default(),
        rule_sets: model.draft_rulesets().unwrap_or_default(),
        marks: marks.as_ref(),
        idx,
        scope_keys: &model.limits.scope_keys,
        tier_key: &model.limits.tier_key,
        now: model.workloads.now.unwrap_or_else(jiff::Timestamp::now),
    };
    readiness(&input, apps)
}

/// Why the result on screen may not hold every flow of `app`, so that a
/// `ready` verdict on it is not to be trusted: a filter narrower than the
/// window and the cap, a truncated or incomplete result, a focused result
/// for another app, no flow of the app at all. Empty: the result is whole
/// for the app. A move on a partial result is typed through, as a move of
/// an app that is not ready.
pub fn partial_reasons(model: &Model, app: &GroupKey, r: Option<&AppReadiness>) -> Vec<String> {
    let t = &model.traffic;
    let Some(d) = t.data.as_ref() else {
        return vec!["no traffic loaded".into()];
    };
    let mut out = Vec::new();
    if let Some(what) = d.filter.readiness_narrowing() {
        out.push(format!("the traffic filter narrows the flows ({what})"));
    }
    if d.truncated {
        out.push(format!(
            "the result hit its cap ({}): flows are missing",
            d.filter.max_results
        ));
    }
    if d.incomplete {
        out.push("a PCE region did not answer: flows are missing".into());
    }
    let keys = keys_or_default(&model.limits.scope_keys);
    if let Some(f) = &t.focus_result
        && !f.covers(&Focus::new(app.clone(), keys))
    {
        out.push(format!(
            "the result is focused on {}: other apps' flows are partial",
            f.focus.app_text()
        ));
    }
    if r.is_some_and(|r| r.evidence.flows == 0) {
        out.push("no flow of the app in the result".into());
    }
    out
}

/// The order `s` cycles the readiness rows through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadinessSort {
    /// Furthest from enforcement first (not evaluated, needs rules, …).
    #[default]
    Verdict,
    /// Most would-block connections first.
    WouldBlock,
    /// Lowest coverage first.
    Coverage,
    /// Most app flows first.
    Flows,
    App,
}

impl ReadinessSort {
    pub fn next(self) -> Self {
        match self {
            Self::Verdict => Self::WouldBlock,
            Self::WouldBlock => Self::Coverage,
            Self::Coverage => Self::Flows,
            Self::Flows => Self::App,
            Self::App => Self::Verdict,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::Verdict => "verdict",
            Self::WouldBlock => "would block ↓",
            Self::Coverage => "coverage ↑",
            Self::Flows => "flows ↓",
            Self::App => "app A→Z",
        }
    }
}

/// `:readiness`: every app of the result on screen, judged.
#[derive(Debug, Clone, Default)]
pub struct ReadinessView {
    pub rows: Vec<AppReadiness>,
    pub sort: ReadinessSort,
    /// `/`: the rows whose app contains it (any case).
    pub search: String,
    /// `/` typing.
    pub searching: bool,
    /// Into [`Self::shown`].
    pub cursor: usize,
    /// Requests this overlay sent (loads, `D`).
    pub requests: u32,
    /// The readiness of the profile's snapshots (#85): read once, from the
    /// files, the first time the list opens; `None` until then.
    pub trend: Option<crate::snapshot::baseline::Trend>,
}

impl ReadinessView {
    /// The rows under the search, in the sort.
    pub fn shown(&self) -> Vec<&AppReadiness> {
        let needle = self.search.to_lowercase();
        let mut rows: Vec<&AppReadiness> = self
            .rows
            .iter()
            .filter(|r| needle.is_empty() || r.app.to_lowercase().contains(&needle))
            .collect();
        let cov = |r: &AppReadiness| r.coverage.ratio.unwrap_or(-1.0);
        match self.sort {
            ReadinessSort::Verdict => rows.sort_by(|a, b| {
                a.verdict
                    .cmp(&b.verdict)
                    .then(
                        b.coverage
                            .total
                            .would_block
                            .cmp(&a.coverage.total.would_block),
                    )
                    .then(a.app.cmp(&b.app))
            }),
            ReadinessSort::WouldBlock => rows.sort_by(|a, b| {
                b.coverage
                    .total
                    .would_block
                    .cmp(&a.coverage.total.would_block)
                    .then(a.app.cmp(&b.app))
            }),
            ReadinessSort::Coverage => {
                rows.sort_by(|a, b| cov(a).total_cmp(&cov(b)).then(a.app.cmp(&b.app)))
            }
            ReadinessSort::Flows => rows.sort_by(|a, b| {
                b.evidence
                    .flows
                    .cmp(&a.evidence.flows)
                    .then(a.app.cmp(&b.app))
            }),
            ReadinessSort::App => rows.sort_by(|a, b| a.app.cmp(&b.app)),
        }
        rows
    }

    pub fn selected(&self) -> Option<&AppReadiness> {
        self.shown().get(self.cursor).copied()
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.shown().len();
        self.cursor = self
            .cursor
            .saturating_add_signed(delta)
            .min(n.saturating_sub(1));
    }

    /// New rows (data landed): the cursor stays on its app when it can.
    pub fn set_rows(&mut self, rows: Vec<AppReadiness>) {
        let app = self.selected().map(|r| r.app.clone());
        self.rows = rows;
        let shown = self.shown();
        self.cursor = app
            .and_then(|a| shown.iter().position(|r| r.app == a))
            .unwrap_or(self.cursor)
            .min(shown.len().saturating_sub(1));
    }

    /// `7 apps · 2 ready · 3 needs rules`: the verdict counts.
    pub fn counts_text(&self) -> String {
        let mut parts = vec![format!(
            "{} app{}",
            self.rows.len(),
            if self.rows.len() == 1 { "" } else { "s" }
        )];
        for v in [
            Verdict::Ready,
            Verdict::Enforced,
            Verdict::NeedsRules,
            Verdict::NeedsProvision,
            Verdict::NotEvaluated,
        ] {
            let n = self.rows.iter().filter(|r| r.verdict == v).count();
            if n > 0 {
                parts.push(format!("{n} {}", v.text()));
            }
        }
        parts.join(" · ")
    }
}

/// Step 5's query: the blocked flows since the change.
#[derive(Debug, Clone, Default)]
pub enum Watch {
    /// Not asked (`r` runs it).
    #[default]
    Idle,
    Running {
        seq: u64,
        cancel: CancellationToken,
    },
    Done(Box<WatchResult>),
    Failed(String),
}

/// What step 5's query found.
#[derive(Debug, Clone)]
pub struct WatchResult {
    /// The change time it started at (the record's `at`).
    pub since: String,
    pub blocked: BlockedSince,
    /// Flows the query returned.
    pub flows: usize,
    pub truncated: bool,
    pub incomplete: bool,
}

/// A rollback planned from the re-read, waiting for `y` (and the typed
/// count when it needs one).
#[derive(Debug, Clone)]
pub struct RollbackPreview {
    pub record_path: PathBuf,
    pub record: ChangeRecord,
    /// One bulk plan per `(mode, visibility)`, in the record's order.
    pub plans: Vec<(ModeState, BulkPlan)>,
    pub skipped: Vec<RollbackSkip>,
    /// The workloads restored.
    pub count: usize,
    /// Several to full or selective, or more than 25: the count is typed.
    pub needs_count: bool,
    pub typed: String,
}

impl RollbackPreview {
    pub fn count_ok(&self) -> bool {
        !self.needs_count || self.typed.parse::<usize>().ok() == Some(self.count)
    }
}

/// The enforcement overlay for one app.
#[derive(Debug, Clone)]
pub struct EnforceView {
    pub app: GroupKey,
    /// Answers carrying another `seq` belong to an earlier overlay.
    pub seq: u64,
    pub step: Step,
    /// The app's readiness (with its gaps), from what is loaded.
    pub readiness: Option<AppReadiness>,
    /// One cursor per step, by [`Step::index`].
    pub cursor: [usize; 5],
    /// Step 4's target: full or selective.
    pub mode: EnforcementMode,
    /// 0: keep each workload's level; else [`VisibilityLevel::ALL`]`[i - 1]`.
    pub visibility: usize,
    /// Step 4 on an app readiness does not allow: the word typed so far.
    pub typing: Option<String>,
    /// The app's newest move record on this PCE and org, with its path.
    pub record: Option<(PathBuf, ChangeRecord)>,
    /// The rollback record of `record`, when it was rolled back.
    pub rolled_back_by: Option<PathBuf>,
    /// The records could not be read (why).
    pub record_error: Option<String>,
    pub watch: Watch,
    /// The re-read before a rollback is on its way.
    pub rollback_reading: bool,
    pub rollback: Option<RollbackPreview>,
    /// Why the result on screen may miss flows of the app
    /// ([`partial_reasons`]); a move then needs the typed word.
    pub partial: Vec<String>,
    /// Step 4's plan as it stands (computed when data or the target
    /// changes, not every frame).
    pub plan_text: String,
    /// Requests this overlay sent (loads, `D`, the watch query, the
    /// rollback re-read); the writes are counted by the write layer.
    pub requests: u32,
}

impl EnforceView {
    pub fn new(app: GroupKey, seq: u64) -> Self {
        Self {
            app,
            seq,
            step: Step::Readiness,
            readiness: None,
            cursor: [0; 5],
            mode: EnforcementMode::Full,
            visibility: 0,
            typing: None,
            record: None,
            rolled_back_by: None,
            record_error: None,
            watch: Watch::Idle,
            rollback_reading: false,
            rollback: None,
            partial: Vec::new(),
            plan_text: String::new(),
            requests: 0,
        }
    }

    /// A watch query on its way is cancelled and forgotten.
    pub fn drop_watch(&mut self) {
        if let Watch::Running { cancel, .. } = &self.watch {
            cancel.cancel();
        }
        self.watch = Watch::Idle;
    }

    /// The rollback preview, and a re-read on its way, are forgotten.
    pub fn drop_rollback(&mut self) {
        self.rollback = None;
        self.rollback_reading = false;
    }

    /// A move would need the typed word: readiness does not allow it, or
    /// the result it rests on is partial.
    pub fn needs_word(&self) -> bool {
        !self.partial.is_empty()
            || self
                .readiness
                .as_ref()
                .is_none_or(|r| !r.verdict.allows_enforcement())
    }

    pub fn app_text(&self) -> String {
        self.app.join("/")
    }

    pub fn visibility_level(&self) -> Option<VisibilityLevel> {
        self.visibility
            .checked_sub(1)
            .and_then(|i| VisibilityLevel::ALL.get(i).copied())
    }

    /// `keep each workload's level`, `flow_drops`.
    pub fn visibility_text(&self) -> &'static str {
        self.visibility_level()
            .map_or("keep each workload's level", VisibilityLevel::as_str)
    }

    pub fn target(&self) -> ModeState {
        ModeState::target(self.mode, self.visibility_level())
    }

    /// The rows of the current step's list.
    pub fn step_len(&self, step: Step) -> usize {
        let r = self.readiness.as_ref();
        match step {
            Step::Rules => r.and_then(|r| r.gaps.as_ref()).map_or(0, Vec::len),
            Step::Provision => r.map_or(0, |r| r.pending.changes.len()),
            Step::Enforce => 2,
            Step::Readiness | Step::Watch => 0,
        }
    }

    /// `j` / `k` in the current step, clamped.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.step_len(self.step);
        let c = &mut self.cursor[self.step.index()];
        *c = c.saturating_add_signed(delta).min(n.saturating_sub(1));
        if self.step == Step::Enforce {
            self.mode = if *c == 0 {
                EnforcementMode::Full
            } else {
                EnforcementMode::Selective
            };
        }
    }

    /// The gap under step 2's cursor.
    pub fn selected_gap(&self) -> Option<&pce::readiness::Gap> {
        self.readiness
            .as_ref()?
            .gaps
            .as_ref()?
            .get(self.cursor[Step::Rules.index()])
    }

    /// New readiness (data landed): the cursors clamped.
    pub fn set_readiness(&mut self, r: Option<AppReadiness>) {
        self.readiness = r;
        for s in [Step::Rules, Step::Provision] {
            let n = self.step_len(s);
            self.cursor[s.index()] = self.cursor[s.index()].min(n.saturating_sub(1));
        }
    }

    /// The app's newest record (and whether it was rolled back) again
    /// from `dir`: a local read, no request.
    pub fn load_record(&mut self, dir: Option<&Path>, origin: &RecordOrigin) {
        self.record_error = None;
        let Some(dir) = dir else {
            self.record = None;
            self.rolled_back_by = None;
            return;
        };
        match newest_record(dir, &self.app_text(), Some(origin)) {
            Ok(rec) => {
                self.rolled_back_by = rec
                    .as_ref()
                    .and_then(|(p, _)| rolled_back_by(dir, p).ok().flatten());
                // A new record (a move since): the old watch is stale.
                if rec.as_ref().map(|(p, _)| p) != self.record.as_ref().map(|(p, _)| p) {
                    self.drop_watch();
                }
                self.record = rec;
            }
            Err(e) => {
                self.record = None;
                self.rolled_back_by = None;
                self.record_error = Some(format!("{}: {e}", dir.display()));
            }
        }
    }

    /// The left column's word for a step.
    pub fn step_state(&self, step: Step, missing: &Missing) -> String {
        if step == Step::Watch {
            return self.watch_state();
        }
        let Some(r) = &self.readiness else {
            return "-".into();
        };
        match step {
            Step::Readiness if missing.any() => "loading…".into(),
            Step::Readiness => r.verdict.text().into(),
            Step::Rules if r.verdict == Verdict::NotEvaluated => "D first".into(),
            Step::Rules if r.gap_count == 0 => "no gaps".into(),
            Step::Rules => format!("{} gaps", r.gap_count),
            Step::Provision if r.pending.count == 0 => "0 pending".into(),
            Step::Provision => format!("{} pending", r.pending.count),
            Step::Enforce if r.verdict == Verdict::Enforced => "enforced".into(),
            Step::Enforce if r.verdict.allows_enforcement() && !missing.any() => {
                if self.partial.is_empty() {
                    "ready".into()
                } else {
                    "partial".into()
                }
            }
            Step::Enforce | Step::Watch => "blocked".into(),
        }
    }

    /// Step 5's word: no record, moved, rolled back, what the watch found.
    fn watch_state(&self) -> String {
        match (&self.record, &self.rolled_back_by) {
            (None, _) => "no record".into(),
            (Some(_), Some(_)) => "rolled back".into(),
            (Some(_), None) => match &self.watch {
                Watch::Done(w) if w.blocked.flows > 0 => format!("{} blocked", w.blocked.flows),
                Watch::Done(_) => "0 blocked".into(),
                Watch::Running { .. } => "querying…".into(),
                _ => "moved".into(),
            },
        }
    }
}

/// Where a change is made: the profile, PCE host and org.
pub fn origin_of(p: &crate::tui::model::ProfileInfo) -> RecordOrigin {
    RecordOrigin {
        profile: p.name.clone(),
        pce: p.host.clone(),
        org_id: p.org_id,
    }
}

/// What the bulk confirm's `y` sends with a move from the overlay: the
/// record it writes around the send.
#[derive(Debug, Clone, PartialEq)]
pub struct EnforceSend {
    pub app: String,
    pub target: ModeState,
    pub readiness: RecordReadiness,
    pub dir: PathBuf,
    pub origin: RecordOrigin,
}

/// A confirmed rollback: the plans, in order, and the record they roll
/// back.
#[derive(Debug, Clone)]
pub struct RollbackSend {
    pub app: String,
    pub record_path: PathBuf,
    pub plans: Vec<(ModeState, BulkPlan)>,
    pub dir: PathBuf,
    pub origin: RecordOrigin,
}

/// The record's place cannot take a record: nothing may be sent.
pub fn probe(dir: &Path) -> Result<(), String> {
    probe_dir(dir).map_err(|e| {
        format!(
            "change records cannot be written in {}: {e}; nothing sent",
            dir.display()
        )
    })
}

/// Before the send: the record with every workload `pending`, so a send
/// that never finishes still leaves one a rollback can use. `Err`: it
/// could not be written (nothing may be sent).
pub fn record_before(dir: &Path, rec: &ChangeRecord) -> Result<PathBuf, String> {
    write_pending(dir, rec, jiff::Timestamp::now()).map_err(|e| {
        format!(
            "the change record could not be written in {}: {e}; nothing sent",
            dir.display()
        )
    })
}

/// After the send: the record with the results. `Some`: what to show
/// when it could not be rewritten: the pending record still on disk (a
/// rollback can use it: it checks each workload as it is now), where the
/// record with the results was saved instead, and that record in full.
pub fn record_after(
    path: &Path,
    rec: &ChangeRecord,
    afters: &[(BulkItem, ModeState)],
    results: &[ItemResult],
) -> Option<String> {
    let rec = with_results(rec, afters, results);
    let e = rewrite_record(path, &rec).err()?;
    let saved = match write_unsaved(path, &rec) {
        Ok(p) => format!("The record with the results was saved as {}.", p.display()),
        Err(e) => format!("It could not be saved elsewhere either ({e}): keep the record below."),
    };
    let json = serde_json::to_string_pretty(&rec).unwrap_or_else(|e| format!("(unprintable: {e})"));
    Some(format!(
        "CHANGE RECORD NOT UPDATED: {} could not be rewritten with the results ({e}).\n\
         It still lists every workload as pending: a rollback can still use it (it checks \
         each workload as it is now and skips one the move did not apply to).\n\
         {saved}\n\n{json}",
        path.display()
    ))
}

/// What the watch query brought back.
#[derive(Debug)]
pub struct WatchOutcome {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub incomplete: bool,
    pub since: jiff::Timestamp,
}

/// A move's answer: the bulk report, the record written (none when it was
/// refused before), and the loud text when the record could not be
/// written or updated.
#[derive(Debug)]
pub struct EnforceOutcome {
    /// The app moved (`shop/prod`): only its overlay takes the answer.
    pub app: String,
    pub report: BulkReport,
    pub record: Option<PathBuf>,
    pub record_error: Option<String>,
}

/// A rollback's answer: each group's report, the rollback record.
#[derive(Debug)]
pub struct RollbackOutcome {
    /// The app rolled back.
    pub app: String,
    pub reports: Vec<BulkReport>,
    pub record: Option<PathBuf>,
    pub record_error: Option<String>,
}

#[cfg(test)]
pub(crate) mod tests;
