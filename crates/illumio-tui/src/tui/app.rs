//! The runtime: the only I/O in `tui`. Owns the terminal, turns crossterm
//! events and task results into `Msg`s, runs `update`, draws, and executes
//! `Cmd`s by spawning tasks on a shared `Arc<Pce>`.
use crate::cli::error::CliError;
use crate::cli::{Context, GlobalArgs};
use crate::tui::diff::{self, ObjectDiff};
use crate::tui::edit::{self, RulesetTarget, WriteIntent};
use crate::tui::model::ObjectKind;
use crate::tui::model::{Limits, Model, ProfileInfo, WRITES_OFF, WritesState};
use crate::tui::msg::{Cmd, Msg, ObjectsPayload, QueryOutcome};
use crate::tui::{update, view};
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    EventStream,
};
use futures::{Stream, StreamExt};
use pce::models::{DenyRule, PendingChange, PolicyScope, WorkloadFilter};
use pce::write::{IfMissing, JsonlAudit, Plan, WriteContext, WriteError, WriteGuard};
use pce::{CacheMode, NameIndex, Pce};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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

/// What a `:profile` switch built off the event loop: the new client, what
/// the model needs to know about it and the new profile's write guard, or
/// the error to show.
type SwitchResult = Result<(Pce, ProfileInfo, Limits, Option<WriteGuard>, WritesState), String>;

pub struct Runtime {
    pub pce: Arc<Pce>,
    pub model: Model,
    /// The current profile's write guard; `None` while writes are off. Only
    /// the runtime holds it: `update` sees `model.writes` and nothing else.
    pub writer: Option<Arc<WriteGuard>>,
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
                writer,
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

    /// Executes one side effect. Tasks report back over the channel and
    /// always finish with `Msg::Stats` so the header counters stay current.
    pub fn execute(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Quit => self.model.quit = true,
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
                    let result = load_diff(
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
                            let limits = Limits {
                                auto_refresh_min_seconds: ctx.config.auto_refresh_min_seconds,
                                traffic_max_results: ctx.config.traffic_max_results,
                                traffic_default_window: ctx.config.traffic_default_window.clone(),
                                scope_keys: ctx.config.scope_keys.clone(),
                                tier_key: ctx.config.tier_key.clone(),
                            };
                            let (writer, writes) =
                                build_writer(&WriteInputs::from_context(&ctx), global.allow_writes);
                            (ctx.pce, info, limits, writer, writes)
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
            } => {
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
                        Ok::<_, pce::PceError>(QueryOutcome {
                            flows,
                            truncated: cached.value.truncated,
                            matches_count: cached.value.matches_count,
                            elapsed: cached.value.elapsed,
                            hit: cached.hit,
                            filter,
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
            Cmd::LoadRuleSets { scope, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .rule_sets(scope, mode)
                        .await
                        .map(|c| (c.value, c.age.as_secs()))
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
                    }
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::ObjectsLoaded { kind, result });
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
                        .map(|c| (c.value, c.age.as_secs(), window))
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
            Ok((pce, info, limits, writer, writes)) => {
                self.pce = Arc::new(pce);
                self.writer = writer.map(Arc::new);
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
                | Msg::RuleSetsLoaded { .. }
                | Msg::DenyRulesLoaded { .. }
                | Msg::PendingLoaded(_)
                | Msg::ObjectsLoaded { .. }
                | Msg::IpListsLoaded(_)
                | Msg::WorkloadsLoaded(_)
                | Msg::EventsLoaded(_)
                | Msg::ProfileSwitched(_)
                | Msg::Planned(_)
                | Msg::WriteDone { .. }
                | Msg::DiffLoaded(_)
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

/// The draft and active list of one kind through the cache, compared
/// locally. The draft list is usually cached by the name index, so the
/// common cost is one GET for the active list, and none when both are fresh.
async fn load_diff(
    pce: &Pce,
    change: &PendingChange,
    kind: pce::write::ObjectKind,
    index: Option<&NameIndex>,
    fallback_deny: Option<&[DenyRule]>,
) -> pce::Result<ObjectDiff> {
    match kind {
        pce::write::ObjectKind::IpList => {
            let draft = pce.ip_lists(CacheMode::Use).await?.value;
            let active = pce.active_ip_lists(CacheMode::Use).await?.value;
            Ok(diff::ip_list_diff_for(change, &draft, &active))
        }
        pce::write::ObjectKind::Service => {
            let draft = pce.services(CacheMode::Use).await?.value;
            let active = pce.active_services(CacheMode::Use).await?.value;
            Ok(diff::service_diff_for(change, &draft, &active))
        }
        pce::write::ObjectKind::RuleSet => {
            let draft = pce
                .rule_sets(PolicyScope::Draft, CacheMode::Use)
                .await?
                .value;
            let active = pce
                .rule_sets(PolicyScope::Active, CacheMode::Use)
                .await?
                .value;
            let empty = NameIndex::default();
            Ok(diff::rule_set_diff_for(
                change,
                &draft,
                &active,
                index.unwrap_or(&empty),
                fallback_deny,
            ))
        }
        other => Err(pce::PceError::Invalid(format!(
            "no diff for {}",
            other.as_str()
        ))),
    }
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
    let (writer, writes) = build_writer(&WriteInputs::from_context(&ctx), global.allow_writes);
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
    model.limits.scope_keys = cfg.scope_keys.clone();
    model.limits.tier_key = cfg.tier_key.clone();
    let (mut rt, _tx) = Runtime::with_args(Arc::new(ctx.pce), model, global);
    rt.writer = writer.map(Arc::new);
    rt.execute(Cmd::Connect);

    let mut terminal = ratatui::init();
    install_panic_hook(); // after ratatui's, so mouse reporting is switched off first
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange);
    let result = {
        let mut draw = |m: &Model| {
            let _ = terminal.draw(|f| view(m, f));
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

    #[tokio::test]
    async fn a_switch_without_a_guard_drops_it_and_clears_the_marker() {
        use pce::write::MemoryAudit;
        let pce = || {
            Pce::new(
                pce::PceClient::new(pce::ClientConfig::new(
                    "pce.invalid",
                    5,
                    pce::Credentials::new("k", "s"),
                ))
                .unwrap(),
            )
        };
        let guard = WriteGuard::new(true, Box::new(MemoryAudit::new()), write_ctx()).unwrap();
        let mut model = crate::tui::test_support::model_fixture();
        model.writes = WritesState::on();
        let (mut rt, _tx) = Runtime::with_writer(Arc::new(pce()), model, guard);
        assert!(rt.writer.is_some());
        // A failed switch keeps the guard and the marker.
        rt.finish_switch(Err("profile \"nope\" not found".into()));
        assert!(rt.writer.is_some());
        assert!(rt.model.writes.enabled);
        rt.finish_switch(Ok((
            pce(),
            ProfileInfo {
                name: "ro".into(),
                host: "pce.invalid".into(),
                org_id: 5,
            },
            Limits {
                auto_refresh_min_seconds: 60,
                traffic_max_results: 1000,
                traffic_default_window: "24h".into(),
                scope_keys: vec!["app".into(), "env".into()],
                tier_key: "role".into(),
            },
            None,
            WritesState::off(WRITES_OFF),
        )));
        assert!(rt.writer.is_none());
        assert!(!rt.model.writes.enabled);
        assert_eq!(rt.model.writes.reason.as_deref(), Some(WRITES_OFF));
    }

    #[test]
    fn a_delete_summary_names_the_object_not_the_href() {
        let plan = Plan::Send {
            request: pce::write::WriteRequest {
                method: pce::WriteMethod::Delete,
                path: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                body: None,
                summary: "delete /orgs/5/sec_policy/draft/rule_sets/100".into(),
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
