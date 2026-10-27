//! The runtime: the only I/O in `tui`. Owns the terminal, turns crossterm
//! events and task results into `Msg`s, runs `update`, draws, and executes
//! `Cmd`s by spawning tasks on a shared `Arc<Pce>`.
use crate::cli::error::CliError;
use crate::cli::{Context, GlobalArgs};
use crate::tui::diff;
use crate::tui::edit::{self, RulesetTarget, WriteIntent};
use crate::tui::filter::TrafficFilter;
use crate::tui::model::ObjectKind;
use crate::tui::model::{Limits, Model, ProfileInfo, WRITES_OFF, WritesState};
use crate::tui::msg::{
    Cmd, DraftFailure, DraftOutcome, EventsPayload, FocusedOutcome, Msg, ObjectsPayload,
    QueryOutcome,
};
use crate::tui::{update, view};
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    EventStream,
};
use futures::{Stream, StreamExt};
use pce::models::{PolicyScope, WorkloadFilter};
use pce::traffic::Focus;
use pce::write::{
    IfMissing, JsonlAudit, Plan, StepResult, WriteContext, WriteError, WriteGuard, WriteRefused,
};
use pce::{CacheMode, NameIndex, Pce};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

pub const BUSY_TICK: Duration = Duration::from_millis(250);
pub const IDLE_TICK: Duration = Duration::from_secs(1);

/// The sender a spawned task reports through. It belongs to one connection
/// generation: after a profile switch, whatever the old PCE's tasks still
/// report is dropped, so it can neither repopulate the screens nor skew `busy`.
#[derive(Clone)]
struct TaskTx {
    tx: UnboundedSender<Msg>,
    generation: Arc<AtomicU64>,
    mine: u64,
}

impl TaskTx {
    /// Whether the message was queued (false: stale generation or closed channel).
    fn send(&self, msg: Msg) -> bool {
        self.generation.load(Ordering::SeqCst) == self.mine && self.tx.send(msg).is_ok()
    }
}

/// Each step's result of a batch as its task hands it over, `progress`
/// told the count. When the task ends without them all (a panic, a
/// cancel), the next step is not answered (it may have applied: the audit
/// log says) and the rest are not sent.
async fn collect_batch(
    n: usize,
    mut rx: UnboundedReceiver<(usize, StepResult)>,
    worker: tokio::task::JoinHandle<()>,
    mut progress: impl FnMut(usize),
) -> Vec<StepResult> {
    let mut results = Vec::with_capacity(n);
    while let Some((_, r)) = rx.recv().await {
        results.push(r);
        progress(results.len());
    }
    if let Err(e) = worker.await
        && results.len() < n
    {
        results.push(StepResult::Failed(WriteError::Unanswered(
            pce::PceError::Network(format!(
                "the batch task failed at step {} ({e}); what it sent is unknown: check the audit log and :batch",
                results.len() + 1
            )),
        )));
    }
    while results.len() < n {
        results.push(StepResult::NotSent);
    }
    results
}

/// What a bulk report says when the send could not report itself (no
/// guard, or its task failed): taken from the plan before it is sent.
struct BulkFallback {
    target: String,
    change: String,
    items: Vec<(String, String, String)>,
}

impl BulkFallback {
    fn of(plan: &pce::write::BulkPlan, p: &ProfileInfo) -> Self {
        Self {
            target: format!("{} ({}, org {})", p.name, p.host, p.org_id),
            change: plan.change.text(),
            items: plan
                .items
                .iter()
                .map(|i| (i.href.clone(), i.name.clone(), i.change.clone()))
                .collect(),
        }
    }

    /// Nothing sent (`unknown` false), or an outcome nobody knows.
    fn report(self, why: &str, unknown: bool) -> pce::write::BulkReport {
        use pce::write::bulk::{BulkStep, ItemResult, StepStatus};
        let status = if unknown {
            pce::write::ItemStatus::Unanswered
        } else {
            pce::write::ItemStatus::NotSent
        };
        pce::write::BulkReport {
            target: self.target,
            change: self.change,
            refused: (!unknown).then(|| why.to_string()),
            reads: 0,
            steps: if unknown {
                vec![BulkStep {
                    method: pce::WriteMethod::Put,
                    path: "workloads/bulk_update".into(),
                    summary: "bulk update".into(),
                    status: StepStatus::Unanswered,
                    href: None,
                    http_status: None,
                    error: Some(why.to_string()),
                    audit_failed: None,
                }]
            } else {
                Vec::new()
            },
            items: self
                .items
                .into_iter()
                .map(|(href, name, change)| ItemResult {
                    href,
                    name,
                    change,
                    status,
                    error: None,
                })
                .collect(),
            labels_changed: unknown,
            labels_created: false,
            reread_skipped: None,
            failure: Some(if unknown {
                pce::write::bulk::BulkFailure::Unanswered
            } else {
                pce::write::bulk::BulkFailure::Refused
            }),
        }
    }
}

/// A move from the enforcement overlay, in the CLI's order: the guard,
/// the records' directory probed, the record with every workload pending,
/// the bulk send (on its own task: a panic still ends in an answer), the
/// record with the results. Refused before the record: nothing sent.
async fn enforce_task(
    pce: Arc<Pce>,
    guard: Option<Arc<WriteGuard>>,
    plan: pce::write::BulkPlan,
    send: crate::tui::enforce::EnforceSend,
    fallback: BulkFallback,
    tx: TaskTx,
) -> crate::tui::enforce::EnforceOutcome {
    use crate::tui::enforce::{self as e, EnforceOutcome};
    let app = send.app.clone();
    let refused = |fallback: BulkFallback, why: &str| EnforceOutcome {
        app: app.clone(),
        report: fallback.report(why, false),
        record: None,
        record_error: None,
    };
    let Some(guard) = guard else {
        return refused(fallback, WRITES_OFF);
    };
    if let Err(why) = e::probe(&send.dir) {
        return refused(fallback, &why);
    }
    let afters = pce::enforcement::afters(&[(send.target.clone(), &plan)]);
    let mut rec = pce::enforcement::ChangeRecord::of_move(
        &send.app,
        &send.origin,
        jiff::Timestamp::now(),
        Some(send.target.clone()),
        &afters,
        &[],
    );
    rec.readiness = Some(send.readiness.clone());
    let path = match e::record_before(&send.dir, &rec) {
        Ok(p) => p,
        Err(why) => return refused(fallback, &why),
    };
    let worker = {
        let pce = pce.clone();
        tokio::spawn(async move {
            let mut each = move |done: usize, of: usize| {
                let _ = tx.send(Msg::BatchProgress { done, of });
            };
            pce.writer(&guard).execute_bulk(plan, &mut each).await
        })
    };
    let report = match worker.await {
        Ok(r) => r,
        Err(err) => fallback.report(
            &format!(
                "the sending task failed ({err}); what it sent is unknown: check the audit log, the PCE and {}",
                path.display()
            ),
            true,
        ),
    };
    let record_error = e::record_after(&path, &rec, &afters, &report.items);
    EnforceOutcome {
        app: send.app.clone(),
        report,
        record: Some(path),
        record_error,
    }
}

/// A rollback: the guard, the probe, its own record (`rollback_of` the
/// original) with every workload pending, each group's bulk send in
/// order (the rest not sent after one that fails), the record with the
/// results.
async fn rollback_task(
    pce: Arc<Pce>,
    guard: Option<Arc<WriteGuard>>,
    send: crate::tui::enforce::RollbackSend,
    fallbacks: Vec<BulkFallback>,
) -> crate::tui::enforce::RollbackOutcome {
    use crate::tui::enforce::{self as e, RollbackOutcome};
    let app = send.app.clone();
    let refused = |fallbacks: Vec<BulkFallback>, why: &str| RollbackOutcome {
        app: app.clone(),
        reports: fallbacks
            .into_iter()
            .map(|f| f.report(why, false))
            .collect(),
        record: None,
        record_error: None,
    };
    let Some(guard) = guard else {
        return refused(fallbacks, WRITES_OFF);
    };
    if let Err(why) = e::probe(&send.dir) {
        return refused(fallbacks, &why);
    }
    let plans: Vec<(pce::enforcement::ModeState, &pce::write::BulkPlan)> =
        send.plans.iter().map(|(t, p)| (t.clone(), p)).collect();
    let afters = pce::enforcement::afters(&plans);
    let mut rb = pce::enforcement::ChangeRecord::of_move(
        &send.app,
        &send.origin,
        jiff::Timestamp::now(),
        None,
        &afters,
        &[],
    );
    rb.rollback_of = Some(send.record_path.display().to_string());
    let path = match e::record_before(&send.dir, &rb) {
        Ok(p) => p,
        Err(why) => return refused(fallbacks, &why),
    };
    let plans: Vec<pce::write::BulkPlan> = send.plans.into_iter().map(|(_, p)| p).collect();
    let worker = {
        let pce = pce.clone();
        tokio::spawn(async move {
            let mut out = Vec::new();
            let mut failed = false;
            for plan in plans {
                if failed {
                    out.push(None);
                    continue;
                }
                let r = pce.writer(&guard).execute_bulk(plan, &mut |_, _| {}).await;
                failed = r.failed();
                out.push(Some(r));
            }
            out
        })
    };
    let reports: Vec<pce::write::BulkReport> = match worker.await {
        Ok(out) => out
            .into_iter()
            .zip(fallbacks)
            .map(|(r, f)| r.unwrap_or_else(|| f.report("an earlier group failed", false)))
            .collect(),
        Err(err) => fallbacks
            .into_iter()
            .map(|f| {
                f.report(
                    &format!(
                        "the sending task failed ({err}); what it sent is unknown: check the audit log, the PCE and {}",
                        path.display()
                    ),
                    true,
                )
            })
            .collect(),
    };
    let results: Vec<_> = reports.iter().flat_map(|r| r.items.clone()).collect();
    let record_error = e::record_after(&path, &rb, &afters, &results);
    RollbackOutcome {
        app: send.app.clone(),
        reports,
        record: Some(path),
        record_error,
    }
}

/// What a `:profile` switch built off the event loop: the new client, what
/// the model needs to know about it and what its write guard is built from,
/// or the error to show.
type SwitchResult = Result<(Pce, ProfileInfo, Limits, WriteInputs), String>;

pub struct Runtime {
    pub pce: Arc<Pce>,
    pub model: Model,
    /// The current profile's write guard; `None` while writes are off. Only
    /// the runtime holds it: `update` sees `model.writes` and nothing else.
    pub writer: Option<Arc<WriteGuard>>,
    /// The session's write toggle (`--allow-writes` at startup, `:writes
    /// on|off` later). It outlives `:profile`: a switch builds the new
    /// profile's guard only while it is on.
    pub writes_wanted: bool,
    /// What the current profile's guard is built from; `None` for a runtime
    /// built without a profile (tests), which refuses `:writes on`.
    pub write_inputs: Option<WriteInputs>,
    /// The flags the TUI started with; `:profile` rebuilds the connection from them.
    global: Option<GlobalArgs>,
    generation: Arc<AtomicU64>,
    tx: UnboundedSender<Msg>,
    rx: UnboundedReceiver<Msg>,
    /// Runtime-internal: finished profile switches. Kept off the `Msg` channel
    /// because the runtime, not `update`, swaps the client.
    switch_tx: UnboundedSender<SwitchResult>,
    switch_rx: UnboundedReceiver<SwitchResult>,
}

impl Runtime {
    pub fn new(pce: Arc<Pce>, model: Model) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, None, None)
    }

    pub fn with_args(
        pce: Arc<Pce>,
        model: Model,
        global: GlobalArgs,
    ) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, Some(global), None)
    }

    /// A runtime that may write (tests; `run` builds the guard from config).
    pub fn with_writer(
        pce: Arc<Pce>,
        model: Model,
        writer: WriteGuard,
    ) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, None, Some(Arc::new(writer)))
    }

    /// [`Runtime::with_writer`] that can also switch profiles (tests).
    pub fn with_args_and_writer(
        pce: Arc<Pce>,
        model: Model,
        global: GlobalArgs,
        writer: WriteGuard,
    ) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, Some(global), Some(Arc::new(writer)))
    }

    fn build(
        pce: Arc<Pce>,
        model: Model,
        global: Option<GlobalArgs>,
        writer: Option<Arc<WriteGuard>>,
    ) -> (Self, UnboundedSender<Msg>) {
        let (tx, rx) = unbounded_channel();
        let (switch_tx, switch_rx) = unbounded_channel();
        (
            Self {
                pce,
                model,
                writes_wanted: writer.is_some() || global.as_ref().is_some_and(|g| g.allow_writes),
                writer,
                write_inputs: None,
                global,
                generation: Arc::new(AtomicU64::new(0)),
                tx: tx.clone(),
                rx,
                switch_tx,
                switch_rx,
            },
            tx,
        )
    }

    /// The client and result channel for a task spawned now.
    fn task_ctx(&self) -> (Arc<Pce>, TaskTx) {
        let tx = TaskTx {
            tx: self.tx.clone(),
            generation: self.generation.clone(),
            mine: self.generation.load(Ordering::SeqCst),
        };
        (self.pce.clone(), tx)
    }

    /// One Explorer query in the background for the last `filter.window`,
    /// answered with `QueryDone`.
    fn run_traffic(
        &mut self,
        seq: u64,
        filter: TrafficFilter,
        mode: CacheMode,
        cancel: CancellationToken,
    ) {
        self.model.busy += 1;
        let (pce, tx) = self.task_ctx();
        tokio::spawn(async move {
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
            let forward_tx = tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                }
            });
            let result = async {
                let query = filter.to_query(&pce, CacheMode::Use).await?;
                let cached = pce.traffic(&query, mode, &cancel, Some(&ptx)).await?;
                let idx = pce.name_index(CacheMode::Use).await?;
                let flows = pce::traffic::resolve_flows(&cached.value.flows, &idx);
                let bounds = query
                    .start_date
                    .parse()
                    .ok()
                    .zip(query.end_date.parse().ok());
                Ok::<_, pce::PceError>(QueryOutcome {
                    flows,
                    truncated: cached.value.truncated,
                    incomplete: cached.value.incomplete,
                    elapsed: cached.value.elapsed,
                    hit: cached.hit,
                    filter,
                    bounds,
                    hrefs: vec![cached.value.query_href.clone()],
                })
            }
            .await
            .map_err(|e| e.to_string());
            drop(ptx);
            let _ = forward.await;
            let _ = tx.send(Msg::QueryDone { seq, result });
            let _ = tx.send(Msg::Stats(pce.stats()));
        });
    }

    /// `r` on a partial focus: the filter's query with the app's labels on
    /// its side, one query per side (`Pce::focused_traffic`, cache slots
    /// `focus:in` / `focus:out`), answered with `Msg::FocusedDone`.
    fn run_focused(
        &mut self,
        seq: u64,
        filter: TrafficFilter,
        focus: Focus,
        mode: CacheMode,
        cancel: CancellationToken,
    ) {
        self.model.busy += 1;
        let (pce, tx) = self.task_ctx();
        tokio::spawn(async move {
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
            let forward_tx = tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                }
            });
            let result = async {
                let query = filter.to_query(&pce, CacheMode::Use).await?;
                let idx = pce.name_index(CacheMode::Use).await?;
                let ft = pce
                    .focused_traffic("focus", &focus, &query, &idx, mode, &cancel, Some(&ptx))
                    .await?;
                let bounds = query
                    .start_date
                    .parse()
                    .ok()
                    .zip(query.end_date.parse().ok());
                let queries = ft.query_count();
                Ok::<_, pce::PceError>(FocusedOutcome {
                    outcome: QueryOutcome {
                        flows: ft.flows,
                        truncated: ft.truncated,
                        incomplete: ft.incomplete,
                        elapsed: ft.elapsed,
                        hit: ft.hit,
                        filter,
                        bounds,
                        hrefs: ft.hrefs,
                    },
                    focus,
                    queries,
                })
            }
            .await
            .map_err(|e| e.to_string());
            drop(ptx);
            let _ = forward.await;
            let _ = tx.send(Msg::FocusedDone { seq, result });
            let _ = tx.send(Msg::Stats(pce.stats()));
        });
    }

    /// `c`: compare mode's baseline (`Cmd::RunBaseline`) in its own cache
    /// slot, answered with `Msg::BaselineDone`.
    fn run_baseline(
        &mut self,
        seq: u64,
        filter: TrafficFilter,
        (start, end): (jiff::Timestamp, jiff::Timestamp),
        cancel: CancellationToken,
    ) {
        self.model.busy += 1;
        let (pce, tx) = self.task_ctx();
        tokio::spawn(async move {
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
            let forward_tx = tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                }
            });
            let result = async {
                let query = filter
                    .to_query_between(start, end, &pce, CacheMode::Use)
                    .await?;
                let cached = pce
                    .baseline_traffic(&query, CacheMode::Use, &cancel, Some(&ptx))
                    .await?;
                let idx = pce.name_index(CacheMode::Use).await?;
                Ok::<_, pce::PceError>(crate::tui::msg::BaselineOutcome {
                    flows: pce::traffic::resolve_flows(&cached.value.flows, &idx),
                    truncated: cached.value.truncated,
                    incomplete: cached.value.incomplete,
                    hit: cached.hit,
                    filter,
                    start,
                    end,
                })
            }
            .await
            .map_err(|e| e.to_string());
            drop(ptx);
            let _ = forward.await;
            let _ = tx.send(Msg::BaselineDone { seq, result });
            let _ = tx.send(Msg::Stats(pce.stats()));
        });
    }

    /// `N` in the generator: its baseline (`Cmd::RunGenBaseline`), one
    /// query per side in the `focus-baseline:{in,out}` slots, answered
    /// with `Msg::GenBaselineDone`.
    fn run_gen_baseline(
        &mut self,
        seq: u64,
        (filter, focus): (TrafficFilter, Focus),
        (start, end): (jiff::Timestamp, jiff::Timestamp),
        cancel: CancellationToken,
    ) {
        self.model.busy += 1;
        let (pce, tx) = self.task_ctx();
        tokio::spawn(async move {
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
            let forward_tx = tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                }
            });
            let result = async {
                let query = filter
                    .to_query_between(start, end, &pce, CacheMode::Use)
                    .await?;
                let idx = pce.name_index(CacheMode::Use).await?;
                let ft = pce
                    .focused_traffic(
                        pce::FOCUS_BASELINE_SLOT,
                        &focus,
                        &query,
                        &idx,
                        CacheMode::Use,
                        &cancel,
                        Some(&ptx),
                    )
                    .await?;
                let queries = ft.query_count();
                Ok::<_, pce::PceError>(crate::tui::msg::GenBaselineOutcome {
                    flows: ft.flows,
                    truncated: ft.truncated,
                    incomplete: ft.incomplete,
                    hit: ft.hit,
                    queries,
                })
            }
            .await
            .map_err(|e| e.to_string());
            drop(ptx);
            let _ = forward.await;
            let _ = tx.send(Msg::GenBaselineDone { seq, result });
            let _ = tx.send(Msg::Stats(pce.stats()));
        });
    }

    /// `D`: the result on screen against the draft policy (`Cmd::RunDraft`),
    /// answered with `Msg::DraftDone`. Through the cache's draft slot.
    fn run_draft(
        &mut self,
        seq: u64,
        filter: TrafficFilter,
        focus: Option<Focus>,
        hrefs: Vec<String>,
        cancel: CancellationToken,
    ) {
        self.model.busy += 1;
        let (pce, tx) = self.task_ctx();
        tokio::spawn(async move {
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
            let forward_tx = tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                }
            });
            let known = !hrefs.is_empty();
            let result = async {
                let query = filter.to_query(&pce, CacheMode::Use).await?;
                let idx = pce.name_index(CacheMode::Use).await?;
                let bounds = query
                    .start_date
                    .parse()
                    .ok()
                    .zip(query.end_date.parse().ok());
                let (outcome, age) = match &focus {
                    Some(f) => {
                        let ft = pce
                            .focused_traffic_eval(
                                "focus",
                                f,
                                &query,
                                &idx,
                                Some(&hrefs),
                                CacheMode::Use,
                                &cancel,
                                Some(&ptx),
                            )
                            .await?;
                        let age = ft.age;
                        let o = QueryOutcome {
                            flows: ft.flows,
                            truncated: ft.truncated,
                            incomplete: ft.incomplete,
                            elapsed: ft.elapsed,
                            hit: ft.hit,
                            filter,
                            bounds,
                            hrefs: ft.hrefs,
                        };
                        (o, age)
                    }
                    None => {
                        let r = pce
                            .traffic_draft(
                                &query,
                                hrefs.first().map(String::as_str),
                                CacheMode::Use,
                                &cancel,
                                Some(&ptx),
                            )
                            .await?;
                        let o = QueryOutcome {
                            flows: pce::traffic::resolve_flows(&r.value.flows, &idx),
                            truncated: r.value.truncated,
                            incomplete: r.value.incomplete,
                            elapsed: r.value.elapsed,
                            hit: r.hit,
                            filter,
                            bounds,
                            hrefs: vec![r.value.query_href.clone()],
                        };
                        (o, r.age)
                    }
                };
                Ok::<_, pce::PceError>(DraftOutcome {
                    outcome,
                    focus,
                    age,
                })
            }
            .await
            .map_err(|e| match e {
                pce::PceError::NotFound { .. } if known => DraftFailure::Gone(e.to_string()),
                e => DraftFailure::Failed(e.to_string()),
            });
            drop(ptx);
            let _ = forward.await;
            let _ = tx.send(Msg::DraftDone { seq, result });
            let _ = tx.send(Msg::Stats(pce.stats()));
        });
    }

    /// Executes one side effect. Tasks report back over the channel and
    /// always finish with `Msg::Stats` so the header counters stay current.
    pub fn execute(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Quit => self.model.quit = true,
            // Synchronous and request-free: the guard only checks the audit log.
            Cmd::SetWrites(on) => {
                self.writes_wanted = on;
                let (writer, writes) = match &self.write_inputs {
                    Some(inputs) => build_writer(inputs, on),
                    None if on => (
                        None,
                        WritesState::off(WriteRefused::ProfileDenies.to_string()),
                    ),
                    None => (None, WritesState::off(WRITES_OFF)),
                };
                self.writer = writer.map(Arc::new);
                self.apply(Msg::WritesSet(writes));
            }
            // Synchronous and local: a file, never the PCE.
            Cmd::Export {
                path,
                force,
                headers,
                rows,
            } => {
                let result = crate::tui::export::write(&path, force, &headers, &rows);
                self.apply(Msg::Exported(result));
            }
            Cmd::PlanWrite(intent) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                tokio::spawn(async move {
                    let result = match guard {
                        None => Err("writes are not enabled".to_string()),
                        Some(guard) => plan_intent(&pce, &guard, intent)
                            .await
                            .map_err(|e| e.to_string()),
                    };
                    let _ = tx.send(Msg::Planned(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::ExecuteWrite { plan, kind } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let method = plan.request().map(|r| r.method);
                tokio::spawn(async move {
                    // `execute` sends once, audits, and invalidates the kind's cache prefix.
                    let result = match &guard {
                        Some(guard) => pce.writer(guard).execute(plan).await,
                        None => Err(WriteError::Invalid(vec![WRITES_OFF.into()])),
                    };
                    // Pending changes are not under the kind's prefix.
                    pce.invalidate("policy:pending");
                    let _ = tx.send(Msg::WriteDone {
                        kind,
                        method,
                        result,
                    });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::PlanBatch { input, index } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                tokio::spawn(async move {
                    // Reads only: the fresh drafts, the IP lists, a GET per update.
                    let result = match guard {
                        None => Err("writes are not enabled".to_string()),
                        Some(guard) => pce
                            .writer(&guard)
                            .plan_batch(*input, &index)
                            .await
                            .map_err(|e| e.to_string()),
                    };
                    let _ = tx.send(Msg::BatchPlanned(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::ExecuteBatch(batch) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let kinds: Vec<pce::write::ObjectKind> =
                    batch.steps.iter().map(|s| s.kind).collect();
                let summaries: Vec<String> = batch
                    .steps
                    .iter()
                    .map(|s| match &s.plan {
                        Plan::Send { request, .. } => request.summary.clone(),
                        Plan::Noop { reason, .. } => reason.clone(),
                    })
                    .collect();
                let target = batch
                    .origin
                    .as_ref()
                    .map_or_else(|| "no profile".to_string(), |o| o.text());
                let n = kinds.len();
                tokio::spawn(async move {
                    // The steps go on their own task, each result handed
                    // over as it is known: a panic there still ends in an
                    // answer, exact up to the step it hit.
                    let (step_tx, step_rx) = unbounded_channel();
                    let worker = {
                        let pce = pce.clone();
                        tokio::spawn(async move {
                            let mut each = |i: usize, r: StepResult| {
                                let _ = step_tx.send((i, r));
                            };
                            match &guard {
                                // One `execute` per step, in order; the first failure stops it.
                                Some(guard) => {
                                    pce.writer(guard).execute_batch_each(batch, &mut each).await
                                }
                                None => {
                                    each(
                                        0,
                                        StepResult::Failed(WriteError::Invalid(vec![
                                            WRITES_OFF.into(),
                                        ])),
                                    );
                                    for i in 1..n {
                                        each(i, StepResult::NotSent);
                                    }
                                }
                            }
                        })
                    };
                    let progress = tx.clone();
                    let results = collect_batch(n, step_rx, worker, |done| {
                        let _ = progress.send(Msg::BatchProgress { done, of: n });
                    })
                    .await;
                    // Pending changes are not under the kinds' prefixes.
                    pce.invalidate("policy:pending");
                    let _ = tx.send(Msg::BatchDone {
                        kinds,
                        summaries,
                        target,
                        results,
                    });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::ExecuteBulk(plan) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let fallback = BulkFallback::of(&plan, &self.model.profile);
                tokio::spawn(async move {
                    // The send goes on its own task: a panic there still
                    // ends in an answer (the outcome then unknown).
                    let progress = tx.clone();
                    let worker = {
                        let pce = pce.clone();
                        tokio::spawn(async move {
                            let guard = guard?;
                            let mut each = move |done: usize, of: usize| {
                                let _ = progress.send(Msg::BatchProgress { done, of });
                            };
                            // The re-read, the label creates, one chunk at a time.
                            Some(pce.writer(&guard).execute_bulk(*plan, &mut each).await)
                        })
                    };
                    let report = match worker.await {
                        Ok(Some(r)) => r,
                        Ok(None) => fallback.report(WRITES_OFF, false),
                        Err(e) => fallback.report(
                            &format!(
                                "the sending task failed ({e}); what it sent is unknown: check the audit log and the PCE"
                            ),
                            true,
                        ),
                    };
                    let _ = tx.send(Msg::BulkDone(Box::new(report)));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::EnforceApp { plan, send } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let fallback = BulkFallback::of(&plan, &self.model.profile);
                tokio::spawn(async move {
                    let o =
                        enforce_task(pce.clone(), guard, *plan, *send, fallback, tx.clone()).await;
                    let _ = tx.send(Msg::EnforceDone(Box::new(o)));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::ExecuteRollback(send) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let fallbacks = send
                    .plans
                    .iter()
                    .map(|(_, p)| BulkFallback::of(p, &self.model.profile))
                    .collect();
                tokio::spawn(async move {
                    let o = rollback_task(pce.clone(), guard, *send, fallbacks).await;
                    let _ = tx.send(Msg::RollbackDone(Box::new(o)));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::RollbackRead { seq, filter, dir } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    // The records' place first, before any request; then
                    // past the cache: what the workloads are now.
                    let result = match crate::tui::enforce::probe(&dir) {
                        Err(e) => Err(e),
                        Ok(()) => pce
                            .workloads(&filter, CacheMode::Bypass)
                            .await
                            .map(|c| (c.value, c.truncated))
                            .map_err(|e| e.to_string()),
                    };
                    let _ = tx.send(Msg::RollbackRead { seq, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::WatchBlocked {
                seq,
                app,
                keys,
                since,
                max_results,
                cancel,
            } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let index = self.model.index().cloned();
                tokio::spawn(async move {
                    let result = async {
                        let idx =
                            index.ok_or_else(|| "the name index is not loaded".to_string())?;
                        let m = pce::traffic::app_labels(&app, &keys, &idx)?;
                        let q = pce::enforcement::watch_query(
                            since,
                            jiff::Timestamp::now(),
                            m,
                            max_results,
                        )
                        .map_err(|e| e.to_string())?;
                        // Its own slot: the result on Traffic stays.
                        let r = pce
                            .traffic_slot("watch", &q, CacheMode::Bypass, &cancel, None)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(crate::tui::enforce::WatchOutcome {
                            flows: pce::traffic::resolve_flows(&r.value.flows, &idx),
                            truncated: r.truncated,
                            incomplete: r.value.incomplete,
                            since,
                        })
                    }
                    .await;
                    let _ = tx.send(Msg::WatchDone { seq, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::BulkDraftCheck {
                seq,
                hrefs,
                window,
                window_text,
                max_results,
                cancel,
            } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let index = self.model.index().cloned();
                tokio::spawn(async move {
                    let before = pce.stats().calls;
                    let result = async {
                        let idx =
                            index.ok_or_else(|| "the name index is not loaded".to_string())?;
                        let query =
                            pce::write::bulk::draft_check_query(&hrefs, window, max_results)
                                .map_err(|e| e.to_string())?;
                        // Its own slot: the draft result on Traffic stays.
                        let r = pce
                            .traffic_eval(
                                "bulk",
                                &query,
                                pce::Evaluation::Draft(None),
                                CacheMode::Bypass,
                                &cancel,
                                None,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        let flows = pce::traffic::resolve_flows(&r.value.flows, &idx);
                        Ok(crate::tui::bulk::CheckResult::new(
                            &flows,
                            &hrefs,
                            pce.stats().calls.saturating_sub(before),
                            r.value.truncated,
                            window_text,
                        ))
                    }
                    .await;
                    let _ = tx.send(Msg::BulkCheckDone { seq, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadDiff {
                change,
                kind,
                fallback_deny,
            } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                // A ruleset diff names labels and lists through the loaded index.
                let index = self.model.index().cloned();
                tokio::spawn(async move {
                    let result = diff::load_diff(
                        &pce,
                        &change,
                        kind,
                        index.as_deref(),
                        fallback_deny.as_deref(),
                    )
                    .await
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::DiffLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::SwitchProfile(name) => {
                self.model.busy += 1;
                // Only the profile name carries over: a startup `--host`, `--port`
                // or `--org-id` would otherwise pin the new profile to the old PCE.
                let mut global = self.global.clone().unwrap_or_default();
                global.profile = Some(name);
                global.host = None;
                global.pce_port = None;
                global.org_id = None;
                // Config and client construction issue no request, but may run
                // `api_secret_cmd` through `sh`: off the event loop, so the screen
                // keeps drawing. `finish_switch` swaps the client when it is done.
                let done = self.switch_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let result = Context::new(&global)
                        .map(|ctx| {
                            let info = ProfileInfo {
                                name: ctx.config.profile_name.clone(),
                                host: ctx.config.host.clone(),
                                org_id: ctx.config.org_id,
                            };
                            let limits = Limits::from_config(&ctx.config);
                            let inputs = WriteInputs::from_context(&ctx);
                            (ctx.pce, info, limits, inputs)
                        })
                        .map_err(|e| e.to_string());
                    let _ = done.send(result);
                });
            }
            Cmd::Connect => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let msg = match pce.health().await {
                        Ok(_) => Msg::Connected,
                        Err(e) => Msg::ConnectFailed(e.to_string()),
                    };
                    let _ = tx.send(msg);
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::RunQuery {
                seq,
                filter,
                mode,
                cancel,
            } => self.run_traffic(seq, filter, mode, cancel),
            Cmd::RunFocused {
                seq,
                filter,
                focus,
                mode,
                cancel,
            } => self.run_focused(seq, filter, focus, mode, cancel),
            Cmd::RunDraft {
                seq,
                filter,
                focus,
                hrefs,
                cancel,
            } => self.run_draft(seq, filter, focus, hrefs, cancel),
            Cmd::RunBaseline {
                seq,
                filter,
                start,
                end,
                cancel,
            } => self.run_baseline(seq, filter, (start, end), cancel),
            Cmd::RunGenBaseline {
                seq,
                filter,
                focus,
                start,
                end,
                cancel,
            } => self.run_gen_baseline(seq, (filter, focus), (start, end), cancel),
            Cmd::LoadRuleSets { scope, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .rule_sets(scope, mode)
                        .await
                        .map(|c| (c.value, c.age.as_secs(), c.hit))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::RuleSetsLoaded { scope, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadDenyRules { href, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .deny_rules(&href, mode)
                        .await
                        .map(|c| c.value)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::DenyRulesLoaded { href, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadPending(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .pending_changes(mode)
                        .await
                        .map(|c| c.value)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::PendingLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            // The IP lists tab loads through its own command.
            Cmd::LoadObjects {
                kind: ObjectKind::IpLists,
                mode,
            } => self.execute(Cmd::LoadIpLists(mode)),
            Cmd::LoadObjects { kind, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = match kind {
                        ObjectKind::Labels => pce
                            .labels(mode)
                            .await
                            .map(|c| ObjectsPayload::Labels(c.value, c.age.as_secs())),
                        ObjectKind::LabelGroups => pce
                            .label_groups(mode)
                            .await
                            .map(|c| ObjectsPayload::LabelGroups(c.value, c.age.as_secs())),
                        ObjectKind::Services => pce
                            .services(mode)
                            .await
                            .map(|c| ObjectsPayload::Services(c.value, c.age.as_secs())),
                        ObjectKind::IpLists => unreachable!("forwarded to Cmd::LoadIpLists above"),
                    }
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::ObjectsLoaded { kind, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadLabelUsage(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .labels_with_usage(mode)
                        .await
                        .map(|c| c.value)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::LabelUsageLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::CountLabelWorkloads { href, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .label_workload_probe(&href, mode)
                        .await
                        .map(|c| c.value)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::LabelWorkloadsCounted { href, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadPairingProfiles(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .pairing_profiles(mode)
                        .await
                        .map(|c| c.value)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::PairingProfilesLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadIpLists(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .ip_lists(mode)
                        .await
                        .map(|c| (c.value, c.age.as_secs()))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::IpListsLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadWorkloads(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .workloads(&WorkloadFilter::default(), mode)
                        .await
                        .map(|c| (c.value, c.age.as_secs(), c.truncated))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::WorkloadsLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadAppWorkloads { seq, filter, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .workloads(&filter, mode)
                        .await
                        .map(|c| crate::tui::msg::AppWorkloadsPayload {
                            workloads: c.value,
                            truncated: c.truncated,
                            hit: c.hit,
                        })
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::AppWorkloadsLoaded { seq, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadEvents { filter, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let stamp = |t: Option<jiff::Timestamp>| {
                        t.map(pce::traffic::format_timestamp).unwrap_or_default()
                    };
                    let window = (stamp(filter.since), stamp(filter.until));
                    let result = pce
                        .events(&filter, mode)
                        .await
                        .map(|c| EventsPayload {
                            age_secs: c.age.as_secs(),
                            truncated: c.truncated,
                            events: c.value,
                            window,
                        })
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::EventsLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadIndex(mode) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let msg = match pce.name_index(mode).await {
                        Ok(index) => Msg::IndexLoaded { index, age_secs: 0 },
                        Err(e) => Msg::IndexFailed(e.to_string()),
                    };
                    let _ = tx.send(msg);
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
        }
    }

    /// A finished `:profile` switch: the new client replaces the old one and the
    /// generation moves on *before* `update` sees `ProfileSwitched`, so the
    /// tasks it spawns (connect, index) run against the new PCE and whatever the
    /// old one still reports is dropped.
    fn finish_switch(&mut self, result: SwitchResult) {
        // Results the old PCE queued before this point passed the old
        // generation check, so they belong to the old model: apply them
        // before the swap rather than letting them land in the reset one.
        while let Ok(m) = self.rx.try_recv() {
            self.apply(m);
        }
        let msg = match result {
            Ok((pce, info, limits, inputs)) => {
                // The new profile's own `writes` decides, while the toggle is on.
                let (writer, writes) = build_writer(&inputs, self.writes_wanted);
                self.pce = Arc::new(pce);
                self.writer = writer.map(Arc::new);
                self.write_inputs = Some(inputs);
                self.model.writes = writes;
                self.generation.fetch_add(1, Ordering::SeqCst);
                Msg::ProfileSwitched(Ok((info, limits)))
            }
            Err(e) => Msg::ProfileSwitched(Err(e)),
        };
        self.apply(msg);
    }

    fn apply(&mut self, msg: Msg) {
        // A task result always lowers `busy`; terminal events do not.
        let task_result = matches!(
            msg,
            Msg::Connected
                | Msg::ConnectFailed(_)
                | Msg::IndexLoaded { .. }
                | Msg::IndexFailed(_)
                | Msg::QueryDone { .. }
                | Msg::FocusedDone { .. }
                | Msg::DraftDone { .. }
                | Msg::BaselineDone { .. }
                | Msg::GenBaselineDone { .. }
                | Msg::RuleSetsLoaded { .. }
                | Msg::DenyRulesLoaded { .. }
                | Msg::PendingLoaded(_)
                | Msg::ObjectsLoaded { .. }
                | Msg::IpListsLoaded(_)
                | Msg::LabelUsageLoaded(_)
                | Msg::LabelWorkloadsCounted { .. }
                | Msg::PairingProfilesLoaded(_)
                | Msg::WorkloadsLoaded(_)
                | Msg::AppWorkloadsLoaded { .. }
                | Msg::EventsLoaded(_)
                | Msg::ProfileSwitched(_)
                | Msg::Planned(_)
                | Msg::WriteDone { .. }
                | Msg::DiffLoaded(_)
                | Msg::BatchPlanned(_)
                | Msg::BatchDone { .. }
                | Msg::BulkDone(_)
                | Msg::BulkCheckDone { .. }
                | Msg::EnforceDone(_)
                | Msg::RollbackDone(_)
                | Msg::RollbackRead { .. }
                | Msg::WatchDone { .. }
        );
        let cmds = update(&mut self.model, msg);
        if task_result {
            self.model.busy = self.model.busy.saturating_sub(1);
        }
        for c in cmds {
            self.execute(c);
        }
    }

    /// Tick period, or None when idle (no CPU use while nothing happens).
    pub fn tick_period(&self) -> Option<Duration> {
        if self.model.busy > 0 {
            Some(BUSY_TICK)
        } else if self.model.any_auto_enabled() {
            Some(IDLE_TICK)
        } else {
            None
        }
    }
}

const AUDIT_PATH_UNKNOWN: &str = "audit log path unknown: no $XDG_STATE_HOME and no home directory";

/// What the current profile's write guard is built from, the same inputs as
/// the CLI's (`cli/write.rs`, `write_guard`): the profile's `writes`, the
/// JSONL audit log, and the context its entries carry (`source: "tui"`).
#[derive(Debug, Clone)]
pub struct WriteInputs {
    pub profile_allows: bool,
    /// `None` without `$XDG_STATE_HOME` and a home directory.
    pub audit_path: Option<PathBuf>,
    pub ctx: WriteContext,
}

impl WriteInputs {
    fn from_context(ctx: &Context) -> Self {
        Self {
            profile_allows: ctx.config.writes,
            audit_path: JsonlAudit::default_path(),
            ctx: WriteContext {
                profile: ctx.config.profile_name.clone(),
                host: ctx.config.host.clone(),
                org_id: ctx.config.org_id,
                source: "tui".into(),
            },
        }
    }
}

/// The TUI's write guard for one profile. `enabled` is the session toggle
/// (`--allow-writes` at startup, `:writes on|off` later); with it off, or on
/// any refusal, writes stay off with the reason. Startup, `:profile` and
/// `:writes` all build through here.
fn build_writer(inputs: &WriteInputs, enabled: bool) -> (Option<WriteGuard>, WritesState) {
    if !enabled {
        return (None, WritesState::off(WRITES_OFF));
    }
    // The profile first: a profile that denies writes never touches the audit path.
    if let Err(refused) = WriteGuard::check(inputs.profile_allows) {
        return (None, WritesState::off(refused.to_string()));
    }
    let Some(path) = &inputs.audit_path else {
        return (None, WritesState::off(AUDIT_PATH_UNKNOWN));
    };
    match WriteGuard::new(
        inputs.profile_allows,
        Box::new(JsonlAudit::new(path.clone())),
        inputs.ctx.clone(),
    ) {
        Ok(guard) => (Some(guard), WritesState::on()),
        Err(refused) => (None, WritesState::off(refused.to_string())),
    }
}

/// The library planner for one intent, through the guard. Creates use
/// `IfMissing::Create`; IP list, service and workload updates and workload
/// deletes fetch the object once; label plans make no request. A ruleset typed
/// by name costs one draft ruleset read (through the cache) before the rule is planned.
async fn plan_intent(
    pce: &Pce,
    guard: &WriteGuard,
    intent: WriteIntent,
) -> Result<(Plan, pce::write::ObjectKind), WriteError> {
    let w = pce.writer(guard);
    let kind = intent.kind();
    let plan = match intent {
        WriteIntent::CreateLabel(d) => w.plan_create_label(&d, IfMissing::Create).await?,
        WriteIntent::UpdateLabel { href, value } => w.plan_update_label(&href, &value)?,
        WriteIntent::DeleteLabel { href, name } => {
            named(w.plan_delete_label(&href)?, "label", &name)
        }
        WriteIntent::CreateIpList(d) => w.plan_create_ip_list(&d, IfMissing::Create).await?,
        WriteIntent::UpdateIpList { href, patch } => w.plan_update_ip_list(&href, &patch).await?,
        WriteIntent::DeleteIpList { href, name } => {
            named(w.plan_delete_ip_list(&href)?, "ip_list", &name)
        }
        WriteIntent::CreateService(d) => w.plan_create_service(&d, IfMissing::Create).await?,
        WriteIntent::UpdateService { href, patch } => w.plan_update_service(&href, &patch).await?,
        WriteIntent::DeleteService { href, name } => {
            named(w.plan_delete_service(&href)?, "service", &name)
        }
        WriteIntent::CreateWorkload(d) => w.plan_create_workload(&d, IfMissing::Create).await?,
        // The TUI only sends `set_labels`, which `WorkloadPatch::apply` never
        // resolves through the index (only add/remove do), so an empty index
        // costs no request.
        WriteIntent::UpdateWorkload { href, patch } => {
            w.plan_update_workload(&href, &patch, &NameIndex::default())
                .await?
        }
        // The library's summary already names the workload.
        WriteIntent::DeleteWorkload { href, .. } => w.plan_delete_workload(&href).await?,
        WriteIntent::CreateRuleSet(d) => w.plan_create_rule_set(&d, IfMissing::Create).await?,
        // The user's Ctrl-R: one fresh draft rulesets read finds a ruleset
        // already named so (made since the form's list, or still loading).
        WriteIntent::CreateRuleSetIfMissing(d) => {
            w.plan_create_rule_set(&d, IfMissing::SkipExisting).await?
        }
        WriteIntent::UpdateRuleSet { href, patch } => w.plan_update_rule_set(&href, &patch).await?,
        WriteIntent::DeleteRuleSet { href, name } => {
            named(w.plan_delete_rule_set(&href)?, "rule_set", &name)
        }
        WriteIntent::CreateRule { ruleset, draft } => {
            let href = match ruleset {
                RulesetTarget::Href(h) => h,
                // The only read a form causes: the draft rulesets, through the cache.
                RulesetTarget::Name(n) => {
                    let sets = pce
                        .rule_sets(PolicyScope::Draft, CacheMode::Use)
                        .await?
                        .value;
                    edit::ruleset_href_named(&sets, &n).map_err(WriteError::invalid)?
                }
            };
            w.plan_create_rule(&href, &draft)?
        }
        WriteIntent::UpdateRule { href, patch } => w.plan_update_rule(&href, &patch).await?,
        WriteIntent::DeleteRule { href, name } => named(w.plan_delete_rule(&href)?, "rule", &name),
        WriteIntent::CreateDenyRule {
            ruleset_href,
            draft,
        } => w.plan_create_deny_rule(&ruleset_href, &draft)?,
        WriteIntent::UpdateDenyRule { href, patch } => {
            w.plan_update_deny_rule(&href, &patch).await?
        }
        WriteIntent::DeleteDenyRule { href, name } => {
            named(w.plan_delete_deny_rule(&href)?, "rule", &name)
        }
        // One pending GET past the cache, then the explicit change subset.
        WriteIntent::Provision(req) => w.plan_provision(&req).await?,
        // The same read; the summary names each object and its fate.
        WriteIntent::Revert(req) => w.plan_revert(&req).await?,
    };
    Ok((plan, kind))
}

/// The library's delete summary names the href; the confirm overlay names
/// the object. Only `summary` changes: it is neither sent nor audited.
fn named(mut plan: Plan, kind: &str, name: &str) -> Plan {
    if let Plan::Send { request, .. } = &mut plan {
        request.summary = format!("delete {kind} {name}");
    }
    plan
}

fn to_msg(e: Event) -> Option<Msg> {
    use crossterm::event::MouseEventKind;
    match e {
        Event::Key(k) => Some(Msg::Key(k)),
        // Motion changes nothing; dropping it here saves a redraw per event.
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) => {
            None
        }
        Event::Mouse(m) => Some(Msg::Mouse(m)),
        Event::Resize(w, h) => Some(Msg::Resize(w, h)),
        Event::FocusGained => Some(Msg::Focus(true)),
        Event::FocusLost => Some(Msg::Focus(false)),
        Event::Paste(_) => None,
    }
}

/// Drives the model until `quit` or the event stream ends. `draw` is called
/// after every applied message batch. Generic over the event source so tests
/// can inject events.
pub async fn run_loop<S>(rt: &mut Runtime, mut events: S, draw: &mut dyn FnMut(&Model)) -> Model
where
    S: Stream<Item = Event> + Unpin,
{
    draw(&rt.model);
    loop {
        if rt.model.quit {
            break;
        }
        let tick = rt.tick_period();
        tokio::select! {
            ev = events.next() => match ev {
                Some(e) => { if let Some(m) = to_msg(e) { rt.apply(m); } }
                None => break,
            },
            Some(m) = rt.rx.recv() => rt.apply(m),
            Some(r) = rt.switch_rx.recv() => rt.finish_switch(r),
            _ = async { match tick { Some(d) => tokio::time::sleep(d).await, None => std::future::pending::<()>().await } } => {
                rt.apply(Msg::Tick(Instant::now()));
            }
        }
        // Drain whatever else is queued before drawing once.
        while let Ok(m) = rt.rx.try_recv() {
            rt.apply(m);
        }
        draw(&rt.model);
    }
    std::mem::replace(
        &mut rt.model,
        Model::new_connecting(
            ProfileInfo {
                name: String::new(),
                host: String::new(),
                org_id: 0,
            },
            "dark".into(),
            60,
            1000,
            "24h".into(),
            WritesState::default(),
        ),
    )
}

/// The model starts at 80x24 and otherwise learns the size only from
/// `Event::Resize`: tell it the real one before the first draw, through
/// `update` like any resize, so the treemap fills the panel it is drawn in.
fn learn_size<B: ratatui::backend::Backend>(rt: &mut Runtime, terminal: &ratatui::Terminal<B>) {
    if let Ok(size) = terminal.size() {
        rt.apply(Msg::Resize(size.width, size.height));
    }
}

/// Runs `restore` first on every panic, then the hook that was installed
/// before (ratatui's, which leaves raw mode and the alternate screen).
fn chain_panic_hook(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Without this a panic would leave the shell with mouse and focus reporting
/// on, because ratatui's hook restores only raw mode and the screen.
fn install_panic_hook() {
    chain_panic_hook(|| {
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
    });
}

/// Real entry point: builds the connection from the same config path as the
/// CLI, takes over the terminal, and restores it on every exit path.
pub async fn run(global: GlobalArgs) -> Result<(), CliError> {
    let ctx = Context::new(&global)?;
    let inputs = WriteInputs::from_context(&ctx);
    let (writer, writes) = build_writer(&inputs, global.allow_writes);
    let cfg = &ctx.config;
    let profile = ProfileInfo {
        name: cfg.profile_name.clone(),
        host: cfg.host.clone(),
        org_id: cfg.org_id,
    };
    let theme = global.theme.clone().unwrap_or_else(|| "dark".into());
    let mut model = Model::new_connecting(
        profile,
        theme,
        cfg.auto_refresh_min_seconds,
        cfg.traffic_max_results,
        cfg.traffic_default_window.clone(),
        writes,
    );
    // Change records beside the audit log (`illumio-tui enforce` writes them too).
    model.records_dir = pce::enforcement::default_records_dir();
    // marks.toml next to the config file: read at every rebuild (a stat).
    model.traffic.marks =
        crate::marks::MarksStore::new(crate::marks::marks_path(global.config.as_deref()));
    // Saved filters: the config file's, of the profile in use (read when
    // used, so a `:profile` switch brings its own).
    model.saved.path = match &global.env_file {
        Some(_) => Err("an --env-file connection has no config file profile".into()),
        None => Ok(global
            .config
            .clone()
            .unwrap_or_else(crate::config::default_config_path)),
    };
    // Every profile limit, as a `:profile` switch applies them (the
    // generator's settings and auto-backfill included).
    model.limits = Limits::from_config(cfg);
    model.events.filter.event_type = model.limits.event_type_filter();
    model.set_label_model(
        cfg.scope_keys.clone(),
        cfg.tier_key.clone(),
        cfg.ignore_keys.clone(),
    );
    // The profile's default filters, before anything loads (no request).
    crate::tui::update::apply_defaults(&mut model);
    let (mut rt, _tx) = Runtime::with_args(Arc::new(ctx.pce), model, global);
    rt.writer = writer.map(Arc::new);
    rt.write_inputs = Some(inputs);
    rt.execute(Cmd::Connect);

    let mut terminal = ratatui::init();
    learn_size(&mut rt, &terminal);
    install_panic_hook(); // after ratatui's, so mouse reporting is switched off first
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange);
    let result = {
        let depth = crate::tui::ui::colors::Depth::detect();
        let mut draw = |m: &Model| {
            let _ = terminal.draw(|f| {
                view(m, f);
                if depth == crate::tui::ui::colors::Depth::Ansi256 {
                    crate::tui::ui::colors::downgrade(f.buffer_mut());
                }
            });
        };
        // A terminal read error ends the stream (and the loop) through the normal
        // restore path below instead of being skipped forever.
        let events = Box::pin(
            EventStream::new()
                .take_while(|r| std::future::ready(r.is_ok()))
                .filter_map(|r| std::future::ready(r.ok())),
        );
        run_loop(&mut rt, events, &mut draw).await
    };
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
    ratatui::restore();
    let _ = result;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn a_batch_task_that_panics_still_answers_up_to_the_step_it_hit() {
        let (tx, rx) = unbounded_channel();
        let worker = tokio::spawn(async move {
            let _ = tx.send((
                0,
                StepResult::Skipped {
                    existing_href: "/orgs/5/sec_policy/draft/ip_lists/55".into(),
                },
            ));
            panic!("boom in step 2");
        });
        let mut seen = Vec::new();
        let results = collect_batch(3, rx, worker, |d| seen.push(d)).await;
        assert_eq!(seen, vec![1], "step 1 reported as it finished");
        assert_eq!(results.len(), 3);
        assert!(
            matches!(&results[0], StepResult::Skipped { existing_href } if existing_href.ends_with("/55"))
        );
        let StepResult::Failed(e @ WriteError::Unanswered(_)) = &results[1] else {
            panic!("{results:?}")
        };
        assert!(
            e.to_string().contains("the batch task failed at step 2"),
            "{e}"
        );
        assert!(matches!(results[2], StepResult::NotSent));
    }

    #[test]
    fn mouse_motion_is_dropped_and_the_wheel_kept() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let ev = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: 3,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert!(to_msg(ev(MouseEventKind::Moved)).is_none());
        assert!(to_msg(ev(MouseEventKind::Drag(MouseButton::Left))).is_none());
        assert!(matches!(
            to_msg(ev(MouseEventKind::ScrollDown)),
            Some(Msg::Mouse(_))
        ));
        assert!(matches!(
            to_msg(ev(MouseEventKind::Down(MouseButton::Left))),
            Some(Msg::Mouse(_))
        ));
    }

    #[test]
    fn panic_hook_restores_first_and_then_calls_the_previous_hook() {
        let seen: Arc<Mutex<Vec<&'static str>>> = Arc::default();
        let original = std::panic::take_hook();
        let before = seen.clone();
        std::panic::set_hook(Box::new(move |_| before.lock().unwrap().push("previous")));
        let restore = seen.clone();
        chain_panic_hook(move || restore.lock().unwrap().push("restore"));
        let _ = std::panic::catch_unwind(|| panic!("boom"));
        let _ = std::panic::take_hook();
        std::panic::set_hook(original);
        let seen = seen.lock().unwrap();
        let first = |what| seen.iter().position(|s| *s == what);
        assert!(first("restore").is_some(), "{seen:?}");
        assert!(first("restore") < first("previous"), "{seen:?}");
    }

    fn write_ctx() -> WriteContext {
        WriteContext {
            profile: "demo".into(),
            host: "pce.example".into(),
            org_id: 5,
            source: "tui".into(),
        }
    }

    #[test]
    fn writer_needs_the_toggle_the_profile_and_a_writable_audit_log() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = |profile_allows, audit_path| WriteInputs {
            profile_allows,
            audit_path,
            ctx: write_ctx(),
        };
        let path = || Some(dir.path().join("illumio-tui").join("writes.jsonl"));
        let (g, w) = build_writer(&inputs(true, path()), false);
        assert!(g.is_none());
        assert_eq!(w, WritesState::off(WRITES_OFF));
        let (g, w) = build_writer(&inputs(false, path()), true);
        assert!(g.is_none());
        assert!(!w.enabled);
        assert!(
            w.reason
                .as_deref()
                .unwrap()
                .contains("not enabled for this profile"),
            "{w:?}"
        );
        let (g, w) = build_writer(&inputs(true, None), true);
        assert!(g.is_none());
        assert_eq!(w.reason.as_deref(), Some(AUDIT_PATH_UNKNOWN));
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, "x").unwrap();
        let (g, w) = build_writer(&inputs(true, Some(blocker.join("writes.jsonl"))), true);
        assert!(g.is_none());
        assert!(
            w.reason
                .as_deref()
                .unwrap()
                .starts_with("audit log is not writable"),
            "{w:?}"
        );
        let (g, w) = build_writer(&inputs(true, path()), true);
        assert_eq!(w, WritesState::on());
        assert_eq!(g.unwrap().context().source, "tui");
    }

    fn test_pce() -> Pce {
        Pce::new(
            pce::PceClient::new(pce::ClientConfig::new(
                "pce.invalid",
                5,
                pce::Credentials::new("k", "s"),
            ))
            .unwrap(),
        )
    }

    fn limits() -> Limits {
        Limits {
            auto_refresh_min_seconds: 60,
            traffic_max_results: 1000,
            traffic_default_window: "24h".into(),
            traffic_auto_backfill: false,
            scope_keys: vec!["app".into(), "env".into()],
            tier_key: "role".into(),
            ignore_keys: Vec::new(),
            ruleset_name: "RF-{app}-{env}".into(),
            consolidate_min_peers: 5,
            event_exclude_types: Vec::new(),
        }
    }

    fn profile(name: &str) -> ProfileInfo {
        ProfileInfo {
            name: name.into(),
            host: "pce.invalid".into(),
            org_id: 5,
        }
    }

    fn inputs(profile_allows: bool, audit_path: Option<PathBuf>) -> WriteInputs {
        WriteInputs {
            profile_allows,
            audit_path,
            ctx: write_ctx(),
        }
    }

    fn key(c: char) -> Msg {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    /// Types `:<cmd>` and Enter into the runtime, as a user would.
    fn command(rt: &mut Runtime, cmd: &str) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for c in std::iter::once(':').chain(cmd.chars()) {
            rt.apply(key(c));
        }
        rt.apply(Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    }

    #[tokio::test]
    async fn a_switch_to_a_read_only_profile_drops_the_guard_and_clears_the_marker() {
        use pce::write::MemoryAudit;
        let guard = WriteGuard::new(true, Box::new(MemoryAudit::new()), write_ctx()).unwrap();
        let mut model = crate::tui::test_support::model_fixture();
        model.writes = WritesState::on();
        let (mut rt, _tx) = Runtime::with_writer(Arc::new(test_pce()), model, guard);
        assert!(rt.writer.is_some());
        assert!(rt.writes_wanted);
        // A failed switch keeps the guard and the marker.
        rt.finish_switch(Err("profile \"nope\" not found".into()));
        assert!(rt.writer.is_some());
        assert!(rt.model.writes.enabled);
        rt.finish_switch(Ok((
            test_pce(),
            profile("ro"),
            limits(),
            inputs(false, None),
        )));
        assert!(rt.writer.is_none());
        assert!(!rt.model.writes.enabled);
        assert_eq!(
            rt.model.writes.reason.as_deref(),
            Some(WriteRefused::ProfileDenies.to_string().as_str())
        );
        assert!(rt.writes_wanted, "the toggle outlives the switch");
    }

    #[tokio::test]
    async fn the_model_learns_the_terminal_size_before_the_first_draw_and_keeps_it_across_a_switch()
    {
        use crate::tui::ui::screens::traffic::treemap_area;
        let model = crate::tui::test_support::model_fixture();
        let (mut rt, _tx) = Runtime::new(Arc::new(test_pce()), model);
        assert_eq!(rt.model.size, (80, 24));
        let terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 50)).unwrap();
        learn_size(&mut rt, &terminal);
        assert_eq!(rt.model.size, (200, 50));
        assert_eq!(rt.model.traffic.treemap.area, treemap_area((200, 50)));
        assert_eq!(rt.model.traffic.treemap.area.w, 198);
        // A profile switch resets the data but not the terminal.
        rt.model.switching = true;
        rt.finish_switch(Ok((
            test_pce(),
            profile("rw"),
            limits(),
            inputs(false, None),
        )));
        assert_eq!(rt.model.size, (200, 50));
        assert_eq!(rt.model.traffic.treemap.area, treemap_area((200, 50)));
    }

    #[tokio::test]
    async fn the_toggle_decides_whether_a_switch_builds_a_guard() {
        let dir = tempfile::tempdir().unwrap();
        let path = || Some(dir.path().join("writes.jsonl"));
        let model = crate::tui::test_support::model_fixture();
        let (mut rt, _tx) = Runtime::new(Arc::new(test_pce()), model);
        assert!(!rt.writes_wanted);
        // Toggle off: a writes-enabled profile stays off.
        rt.model.switching = true;
        rt.finish_switch(Ok((
            test_pce(),
            profile("rw"),
            limits(),
            inputs(true, path()),
        )));
        assert!(rt.writer.is_none());
        assert_eq!(rt.model.writes, WritesState::off(WRITES_OFF));
        command(&mut rt, "writes on");
        assert!(rt.writer.is_some());
        // Toggle on: the next profile's own `writes` decides.
        rt.model.switching = true;
        rt.finish_switch(Ok((
            test_pce(),
            profile("rw2"),
            limits(),
            inputs(true, path()),
        )));
        assert!(rt.writer.is_some());
        assert_eq!(rt.model.writes, WritesState::on());
    }

    #[tokio::test]
    async fn writes_on_arms_the_edit_keys_and_writes_off_disarms_them() {
        use crate::tui::model::{Overlay, Screen, StatusKind};
        let dir = tempfile::tempdir().unwrap();
        let mut model = crate::tui::test_support::model_with_objects();
        model.screen = Screen::Objects;
        let (mut rt, _tx) = Runtime::new(Arc::new(test_pce()), model);
        rt.write_inputs = Some(inputs(true, Some(dir.path().join("writes.jsonl"))));
        rt.apply(key('n'));
        assert_eq!(rt.model.status.text, WRITES_OFF);
        assert_eq!(rt.model.overlay, Overlay::None);

        command(&mut rt, "writes on");
        assert!(rt.writer.is_some());
        assert!(rt.writes_wanted);
        assert_eq!(rt.model.writes, WritesState::on());
        assert_eq!(rt.model.status.text, "writes on");
        rt.apply(key('n'));
        assert!(
            matches!(rt.model.overlay, Overlay::Form(_)),
            "{:?}",
            rt.model.overlay
        );
        rt.apply(Msg::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        )));

        command(&mut rt, "writes off");
        assert!(rt.writer.is_none());
        assert!(!rt.writes_wanted);
        assert_eq!(rt.model.writes, WritesState::off(WRITES_OFF));
        assert_eq!(rt.model.status.kind, StatusKind::Warn);
        assert_eq!(rt.model.status.text, WRITES_OFF);
        rt.apply(key('n'));
        assert_eq!(rt.model.status.text, WRITES_OFF);
        assert_eq!(rt.model.overlay, Overlay::None);
        assert_eq!(rt.pce.stats().calls, 0, "the toggle sends no request");
    }

    #[tokio::test]
    async fn writes_on_says_why_it_stays_off() {
        let dir = tempfile::tempdir().unwrap();
        let model = crate::tui::test_support::model_fixture();
        let (mut rt, _tx) = Runtime::new(Arc::new(test_pce()), model);
        let denies = WriteRefused::ProfileDenies.to_string();
        for (inputs, reason) in [
            (
                Some(inputs(false, Some(dir.path().join("writes.jsonl")))),
                denies.as_str(),
            ),
            (Some(inputs(true, None)), AUDIT_PATH_UNKNOWN),
            (None, denies.as_str()),
        ] {
            rt.write_inputs = inputs;
            command(&mut rt, "writes on");
            assert!(rt.writer.is_none(), "{reason}");
            assert_eq!(rt.model.writes, WritesState::off(reason));
            assert_eq!(rt.model.status.text, format!("writes off: {reason}"));
        }
        command(&mut rt, "writes");
        assert_eq!(
            rt.model.status.text,
            format!("writes off: {}", WriteRefused::ProfileDenies)
        );
    }

    #[test]
    fn a_delete_summary_names_the_object_not_the_href() {
        let plan = Plan::Send {
            request: pce::write::WriteRequest {
                method: pce::WriteMethod::Delete,
                path: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                body: None,
                summary: "delete /orgs/5/sec_policy/draft/rule_sets/100".into(),
                objects: Vec::new(),
            },
            kind: pce::write::ObjectKind::RuleSet,
        };
        let Plan::Send { request, .. } = named(plan, "rule_set", "Shop-Prod") else {
            panic!("a send stays a send")
        };
        assert_eq!(request.summary, "delete rule_set Shop-Prod");
    }

    /// A rule delete plans without a request; its confirm names the rule as
    /// the rules pane shows it.
    #[tokio::test]
    async fn a_rule_delete_summary_is_the_rule_line() {
        use pce::write::MemoryAudit;
        let pce = Pce::new(
            pce::PceClient::new(pce::ClientConfig::new(
                "pce.invalid",
                5,
                pce::Credentials::new("k", "s"),
            ))
            .unwrap(),
        );
        let guard = WriteGuard::new(true, Box::new(MemoryAudit::new()), write_ctx()).unwrap();
        for (intent, want) in [
            (
                WriteIntent::DeleteRule {
                    href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
                    name: "allow role=db ← role=web PostgreSQL".into(),
                },
                "delete rule allow role=db ← role=web PostgreSQL",
            ),
            (
                WriteIntent::DeleteDenyRule {
                    href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
                    name: "override_deny role=db ← All workloads 22/tcp".into(),
                },
                "delete rule override_deny role=db ← All workloads 22/tcp",
            ),
        ] {
            let (plan, _) = plan_intent(&pce, &guard, intent).await.unwrap();
            assert_eq!(plan.request().unwrap().summary, want);
        }
        assert_eq!(pce.stats().calls, 0, "a delete plans without a request");
    }
}
