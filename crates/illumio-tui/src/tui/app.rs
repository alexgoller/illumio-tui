//! The runtime: the only I/O in `tui`. Owns the terminal, turns crossterm
//! events and task results into `Msg`s, runs `update`, draws, and executes
//! `Cmd`s by spawning tasks on a shared `Arc<Pce>`.
use crate::cli::error::CliError;
use crate::cli::{Context, GlobalArgs};
use crate::tui::model::{Model, ProfileInfo};
use crate::tui::msg::{Cmd, Msg, QueryOutcome};
use crate::tui::{update, view};
use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    EventStream,
};
use futures::{Stream, StreamExt};
use pce::{CacheMode, Pce};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const BUSY_TICK: Duration = Duration::from_millis(250);
pub const IDLE_TICK: Duration = Duration::from_secs(1);

pub struct Runtime {
    pub pce: Arc<Pce>,
    pub model: Model,
    tx: UnboundedSender<Msg>,
    rx: UnboundedReceiver<Msg>,
}

impl Runtime {
    pub fn new(pce: Arc<Pce>, model: Model) -> (Self, UnboundedSender<Msg>) {
        let (tx, rx) = unbounded_channel();
        (
            Self {
                pce,
                model,
                tx: tx.clone(),
                rx,
            },
            tx,
        )
    }

    /// Executes one side effect. Tasks report back over the channel and
    /// always finish with `Msg::Stats` so the header counters stay current.
    pub fn execute(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Quit => self.model.quit = true,
            Cmd::Connect => {
                self.model.busy += 1;
                let (pce, tx) = (self.pce.clone(), self.tx.clone());
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
                let (pce, tx) = (self.pce.clone(), self.tx.clone());
                tokio::spawn(async move {
                    let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
                    let forward_tx = tx.clone();
                    let forward = tokio::spawn(async move {
                        while let Some(p) = prx.recv().await {
                            let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                        }
                    });
                    let result = async {
                        let query = filter.to_query(&pce, mode).await?;
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
            Cmd::LoadIndex(mode) => {
                self.model.busy += 1;
                let (pce, tx) = (self.pce.clone(), self.tx.clone());
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

    fn apply(&mut self, msg: Msg) {
        // A task result always lowers `busy`; terminal events do not.
        let task_result = matches!(
            msg,
            Msg::Connected
                | Msg::ConnectFailed(_)
                | Msg::IndexLoaded { .. }
                | Msg::IndexFailed(_)
                | Msg::QueryDone { .. }
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
        } else {
            None
        }
    }
}

fn to_msg(e: Event) -> Option<Msg> {
    match e {
        Event::Key(k) => Some(Msg::Key(k)),
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
        ),
    )
}

/// Real entry point: builds the connection from the same config path as the
/// CLI, takes over the terminal, and restores it on every exit path.
pub async fn run(global: GlobalArgs) -> Result<(), CliError> {
    let ctx = Context::new(&global)?;
    let cfg = &ctx.config;
    let profile = ProfileInfo {
        name: cfg.profile_name.clone(),
        host: cfg.host.clone(),
        org_id: cfg.org_id,
    };
    let theme = global.theme.clone().unwrap_or_else(|| "dark".into());
    let model = Model::new_connecting(
        profile,
        theme,
        cfg.auto_refresh_min_seconds,
        cfg.traffic_max_results,
        cfg.traffic_default_window.clone(),
    );
    let (mut rt, _tx) = Runtime::new(Arc::new(ctx.pce), model);
    rt.execute(Cmd::Connect);

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange);
    let result = {
        let mut draw = |m: &Model| {
            let _ = terminal.draw(|f| view(m, f));
        };
        let events = Box::pin(EventStream::new().filter_map(|r| async move { r.ok() }));
        run_loop(&mut rt, events, &mut draw).await
    };
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
    ratatui::restore();
    let _ = result;
    Ok(())
}
