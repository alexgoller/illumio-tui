//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::commands::{self, Command};
use crate::tui::edit::{self, EditIntent, Start, WriteIntent};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::form::FormEvent;
use crate::tui::model::{
    Connection, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction, PolicyPane,
    PolicyState, QueryState, Screen, StatusLine, TrafficData, TrafficView,
};
use crate::tui::msg::{Cmd, Msg, ObjectsPayload, QueryOutcome};
use crate::tui::ui::widgets::overlay::{confirm_write_last_scroll, help_last_scroll};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::models::{EventFilter, PolicyScope};
use pce::traffic::QueryProgress;
use pce::write::{Outcome, Plan, ProvisionRequest, ProvisionSubset, WriteError, provision_counts};
use pce::{CacheMode, RuleKind, WriteMethod};
use std::time::Instant;

pub fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(k) => on_key(model, k),
        Msg::Mouse(me) => on_mouse(model, me),
        Msg::Resize(w, h) => {
            model.size = (w, h);
            Vec::new()
        }
        Msg::Tick(now) => {
            if model.traffic.is_running() {
                refresh_running_status_at(model, now);
            }
            // Auto-refresh only on a live connection (never while connecting or
            // after a failed connect or profile switch).
            if !model.is_connected() {
                return Vec::new();
            }
            if !model.traffic.is_running()
                && model.screen == Screen::Traffic
                && model.focused
                && model.overlay == Overlay::None
                && model.traffic.data.is_some()
                && model.traffic.auto.due(now)
            {
                let f = model.traffic.filter.clone();
                let cmds = start_query(model, f, CacheMode::Bypass);
                model.traffic.auto.from_tick = true;
                return cmds;
            }
            if model.screen == Screen::Policy
                && model.focused
                && model.overlay == Overlay::None
                && model.policy.loaded_at.is_some()
                && !model.policy.loading
                && model.policy.auto.due(now)
            {
                let cmds = load_rule_sets(model, CacheMode::Bypass);
                model.policy.auto.from_tick = true;
                return cmds;
            }
            let o = &model.objects;
            let kind = o.kind;
            if model.screen == Screen::Objects
                && model.focused
                && model.overlay == Overlay::None
                && o.ages[kind.index()].is_some()
                && !o.loading[kind.index()]
                && o.auto.due(now)
            {
                // Opted in with `a`: a real refresh of the current kind, like Traffic and Policy.
                let cmds = load_objects(model, kind, CacheMode::Bypass);
                model.objects.auto.from_tick = true;
                return cmds;
            }
            if model.screen == Screen::Workloads
                && model.focused
                && model.overlay == Overlay::None
                && model.workloads.loaded_at.is_some()
                && !model.workloads.loading
                && model.workloads.auto.due(now)
            {
                let cmds = load_workloads(model, CacheMode::Bypass);
                model.workloads.auto.from_tick = true;
                return cmds;
            }
            if model.screen == Screen::Events
                && model.focused
                && model.overlay == Overlay::None
                && model.events.loaded_at.is_some()
                && !model.events.loading
                && model.events.auto.due(now)
            {
                let cmds = load_events(model, CacheMode::Bypass);
                model.events.auto.from_tick = !cmds.is_empty();
                return cmds;
            }
            Vec::new()
        }
        Msg::Focus(f) => {
            model.focused = f;
            Vec::new()
        }
        Msg::Connected => {
            model.connection = Connection::Connected {
                index: None,
                index_age_secs: 0,
            };
            model.status =
                StatusLine::info("connected; loading labels, label groups, IP lists, services");
            let mut cmds = vec![Cmd::LoadIndex(CacheMode::Use)];
            // Events do not need the name index, only a connection: the screen was
            // opened while connecting.
            if model.screen == Screen::Events {
                cmds.extend(load_missing_events(model));
            }
            cmds
        }
        Msg::ConnectFailed(e) => {
            fail(model, "not connected", &e, false);
            model.connection = Connection::Disconnected { error: e };
            Vec::new()
        }
        Msg::IndexLoaded { index, age_secs } => {
            model.connection = Connection::Connected {
                index: Some(index),
                index_age_secs: age_secs,
            };
            model.status = StatusLine::info("ready");
            model.objects.index_failed = false;
            model.workloads.index_failed = false;
            let mut cmds = Vec::new();
            // Rulesets that arrived before the names could be resolved.
            if !model.policy.raw.is_empty()
                && model.policy.rulesets.is_empty()
                && let Some(idx) = model.index().cloned()
            {
                let raw = std::mem::take(&mut model.policy.raw);
                model.policy.set_rule_sets(raw, &idx);
                cmds.extend(deny_fallback(model, CacheMode::Use));
            }
            // The Objects screen was opened while the startup load was still running.
            if model.screen == Screen::Objects {
                cmds.extend(load_missing_objects(model));
            }
            if model.screen == Screen::Workloads {
                cmds.extend(load_missing_workloads(model));
            }
            // After a reconnect or profile switch nothing else would load Policy.
            if model.screen == Screen::Policy
                && model.policy.loaded_at.is_none()
                && !model.policy.loading
            {
                cmds.extend(load_rule_sets(model, CacheMode::Use));
            }
            cmds
        }
        Msg::IndexFailed(e) => {
            fail(model, "could not load objects", &e, false);
            model.objects.index_failed = true;
            model.workloads.index_failed = true;
            Vec::new()
        }
        Msg::Stats(s) => {
            model.stats = s;
            Vec::new()
        }
        Msg::SwitchScreen(s) => switch(model, s),
        Msg::Command(c) => on_command(model, c),
        Msg::ApplyFilter(f) => {
            model.screen = Screen::Traffic;
            // Key auto-repeat on `t` would otherwise restart (and re-POST) the same query.
            if model.traffic.is_running() && model.traffic.filter == f {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            model.traffic.filter = f.clone();
            start_query(model, f, CacheMode::Use)
        }
        Msg::RunNow => {
            if model.traffic.is_running() {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            let f = model.traffic.filter.clone();
            // The cost guard applies unless this exact filter produced the data on
            // screen (e.g. profile defaults above the thresholds, never confirmed).
            let fetched_with = model.traffic.data.as_ref().is_some_and(|d| d.filter == f);
            if !fetched_with && let Some(reason) = f.needs_confirmation() {
                model.overlay = Overlay::Confirm {
                    reason: format!("{reason}. Run anyway? (y/n)"),
                    action: PendingAction::RunNow(f),
                };
                return Vec::new();
            }
            run_now(model, f)
        }
        Msg::QueryProgress { seq, progress } => {
            if let QueryState::Running {
                seq: cur, status, ..
            } = &mut model.traffic.query
                && *cur == seq
            {
                *status = match progress {
                    QueryProgress::Submitted { .. } => "submitted".into(),
                    QueryProgress::Polling { status, .. } => format!("polling ({status})"),
                    QueryProgress::Downloading => "downloading".into(),
                    QueryProgress::Done { rows } => format!("done, {rows} rows"),
                };
                refresh_running_status(model);
            }
            Vec::new()
        }
        Msg::QueryDone { seq, result } => {
            if model.traffic.seq != seq {
                return Vec::new(); // a cancelled or superseded query
            }
            match result {
                Ok(o) => {
                    let QueryOutcome {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        hit,
                        filter,
                    } = o;
                    let n = flows.len();
                    model.traffic.data = Some(TrafficData {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        cache_hit: hit,
                        fetched_at: Instant::now(),
                        filter,
                    });
                    model.traffic.drill = None;
                    model.traffic.rebuild_views();
                    model.traffic.show_detail = false;
                    model.traffic.query = QueryState::Idle;
                    model.traffic.auto.from_tick = false;
                    model.status = if truncated {
                        StatusLine::warn(format!(
                            "{n} flows (truncated: the PCE matched {}; raise the cap or narrow the filter)",
                            matches_count
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| "more".into())
                        ))
                    } else {
                        StatusLine::info(format!(
                            "{n} flows in {:.1}s{}",
                            elapsed.as_secs_f64(),
                            if hit { " (cached)" } else { "" }
                        ))
                    };
                }
                Err(e) => {
                    model.traffic.query = QueryState::Failed(e.clone());
                    let tripped = model.traffic.auto.take_tick_failure();
                    fail(model, "query", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::RuleSetsLoaded { scope, result } => {
            // A reload a write asked for keeps the write's report. Taken before
            // the scope check: an ignored answer must not leave the flag set.
            let quiet = std::mem::take(&mut model.policy.quiet_reload);
            if scope != model.policy.scope {
                return Vec::new(); // the user toggled draft/active meanwhile
            }
            model.policy.loading = false;
            match result {
                Ok((raw, age)) => {
                    model.policy.loaded_at = Some(Instant::now());
                    model.policy.auto.from_tick = false;
                    model.policy.age_secs = age;
                    // A reload may change deny rules: ask again, but keep the rows
                    // already fetched until the new answer replaces them.
                    model
                        .policy
                        .fallback_deny
                        .retain(|h, _| raw.iter().any(|r| &r.href == h));
                    model.policy.deny_requested.clear();
                    match model.index().cloned() {
                        Some(idx) => model.policy.set_rule_sets(raw, &idx),
                        None => model.policy.raw = raw, // built when the index arrives
                    }
                    if !quiet {
                        model.status = StatusLine::info(format!(
                            "{} rulesets ({})",
                            model.policy.rulesets.len(),
                            model.policy.scope.segment()
                        ));
                    }
                    // Only the selected ruleset's deny rules are fetched on demand;
                    // an explicit `r` bypasses the cache for them as well.
                    let mode = if std::mem::take(&mut model.policy.bypass_deny) {
                        CacheMode::Bypass
                    } else {
                        CacheMode::Use
                    };
                    deny_fallback(model, mode)
                }
                Err(e) => {
                    // The explicit refresh failed: its bypass must not leak into a later reload.
                    model.policy.bypass_deny = false;
                    let tripped = model.policy.auto.take_tick_failure();
                    fail(model, "rulesets", &e, tripped);
                    Vec::new()
                }
            }
        }
        Msg::DenyRulesLoaded { href, result } => {
            match result {
                Ok(d) => {
                    model.policy.fallback_deny.insert(href, d);
                    if let Some(idx) = model.index().cloned() {
                        model.policy.rebuild_rules(&idx);
                    }
                }
                Err(e) if e.contains("not found") => {
                    model.policy.fallback_deny.insert(href, Vec::new());
                }
                Err(e) => {
                    model.policy.deny_requested.remove(&href); // allow a retry on reselect
                    model.status = StatusLine::warn(format!("deny rules: {e}"));
                }
            }
            Vec::new()
        }
        Msg::WorkloadsLoaded(result) => {
            model.workloads.loading = false;
            match result {
                Ok((rows, age)) => {
                    let w = &mut model.workloads;
                    w.raw = rows;
                    w.rebuild();
                    w.loaded_at = Some(Instant::now());
                    w.auto.from_tick = false;
                    w.age_secs = age;
                    w.show_detail &= w.table.selected_item().is_some();
                    model.status =
                        StatusLine::info(format!("{} workloads", model.workloads.table.len()));
                }
                Err(e) => {
                    let tripped = model.workloads.auto.take_tick_failure();
                    fail(model, "could not load workloads", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::EventsLoaded(result) => {
            model.events.loading = false;
            match result {
                Ok((rows, age, window)) => {
                    let e = &mut model.events;
                    e.raw = rows;
                    e.rebuild();
                    e.window = Some(window);
                    e.loaded_at = Some(Instant::now());
                    e.auto.from_tick = false;
                    e.age_secs = age;
                    model.status = StatusLine::info(format!("{} events", model.events.table.len()));
                }
                Err(e) => {
                    let tripped = model.events.auto.take_tick_failure();
                    fail(model, "could not load events", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::ObjectsLoaded { kind, result } => {
            let i = kind.index();
            model.objects.loading[i] = false;
            match result {
                Ok(payload) => {
                    // New labels reach the name index (and so the forms and
                    // filters) at no extra request; the other lists are kept.
                    if let (
                        ObjectsPayload::Labels(v, _),
                        Connection::Connected {
                            index: Some(idx), ..
                        },
                    ) = (&payload, &mut model.connection)
                    {
                        *idx = std::sync::Arc::new(idx.with_labels(v));
                    }
                    let o = &mut model.objects;
                    let age = match payload {
                        ObjectsPayload::Labels(v, a) => {
                            o.labels.set_rows(v);
                            a
                        }
                        ObjectsPayload::LabelGroups(v, a) => {
                            o.groups.set_rows(v);
                            a
                        }
                        ObjectsPayload::IpLists(v, a) => {
                            o.ip_lists.set_rows(v);
                            a
                        }
                        ObjectsPayload::Services(v, a) => {
                            o.services.set_rows(v);
                            a
                        }
                    };
                    o.ages[i] = Some(age);
                    o.loaded_at[i] = Some(Instant::now());
                    o.auto.from_tick = false;
                }
                Err(e) => {
                    let tripped = model.objects.auto.take_tick_failure();
                    fail(model, kind.title(), &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::ProfileSwitched(result) => match result {
            Ok((info, limits)) => {
                model.switching = false;
                model.profile = info;
                model.limits = limits;
                model.reset_data();
                model.connection = Connection::Connecting;
                model.status =
                    StatusLine::info(format!("switched to {}; connecting…", model.profile.name));
                vec![Cmd::Connect]
            }
            Err(e) => {
                model.switching = false;
                model.status = StatusLine::error(format!("profile: {e}"));
                Vec::new()
            }
        },
        Msg::PendingLoaded(result) => {
            model.policy.pending_loading = false;
            match result {
                Ok(p) => {
                    let n = p.len();
                    // After a provision the reload keeps its report in front.
                    model.status = StatusLine::info(match model.policy.provisioned.take() {
                        Some(done) => format!("{done}; {n} pending changes left"),
                        None => format!("{n} pending changes"),
                    });
                    model.policy.pending_selected = 0;
                    model.policy.pending_marked.clear();
                    model.policy.pending = Some(p);
                    // Never over another overlay (an error a provision left open).
                    if matches!(model.overlay, Overlay::None | Overlay::Pending) {
                        model.overlay = Overlay::Pending;
                    }
                }
                Err(e) => {
                    model.policy.provisioned = None;
                    fail(model, "pending", &e, false);
                }
            }
            Vec::new()
        }
        Msg::DiffLoaded(result) => {
            model.policy.diff_loading = false;
            match result {
                Ok(diff) => {
                    model.status = StatusLine::info(format!("diff of {}", diff.name));
                    model.overlay = Overlay::Diff { diff, scroll: 0 };
                }
                Err(e) => fail(model, "diff", &e, false),
            }
            Vec::new()
        }
        Msg::WriteDone {
            kind,
            method,
            result,
        } => {
            model.writes.in_flight = false;
            // The runtime dropped `policy:pending`; the next `p` fetches again.
            model.policy.pending = None;
            model.policy.pending_marked.clear();
            let counts = model.policy.provision_counts.take();
            match result {
                Ok(Outcome::Written(w)) => {
                    let mut text = if kind == pce::write::ObjectKind::Provision {
                        format!("provisioned {}: {}", w.href, counts.unwrap_or_default())
                    } else {
                        let verb = match method {
                            Some(WriteMethod::Post) => "created",
                            Some(WriteMethod::Put) => "updated",
                            Some(WriteMethod::Delete) => "deleted",
                            None => "wrote",
                        };
                        let mut t = format!("{verb} {}", w.href);
                        if w.pending {
                            t.push_str(" (pending)");
                        }
                        t
                    };
                    if !w.dropped_fields.is_empty() {
                        text.push_str(&format!(
                            "; the PCE dropped {}",
                            w.dropped_fields.join(", ")
                        ));
                    }
                    for warning in &w.warnings {
                        text.push_str("; ");
                        text.push_str(warning);
                    }
                    model.status = if w.warnings.is_empty() && w.dropped_fields.is_empty() {
                        StatusLine::info(text)
                    } else {
                        StatusLine::warn(text)
                    };
                    if kind == pce::write::ObjectKind::Provision {
                        model.policy.provisioned = Some(model.status.text.clone());
                    }
                    reload_after_write(model, kind)
                }
                Ok(Outcome::Skipped { reason, .. }) => {
                    model.status = StatusLine::info(format!("skipped: {reason}"));
                    Vec::new()
                }
                // The write may have applied: say so once (the error's own
                // text carries the advice) and reload to show the real state.
                // A failed audit names its log in the error text.
                Err(e) if is_unanswered(&e) => {
                    let audited = !matches!(e, WriteError::AuditFailed { .. });
                    model.status = StatusLine::error(if audited {
                        "write not answered; reloading"
                    } else {
                        "write not answered and not audited; reloading"
                    });
                    model.overlay =
                        Overlay::Error(format!("the request was sent but not answered: {e}"));
                    reload_after_write(model, kind)
                }
                Err(e) => {
                    model.status = StatusLine::error("write failed; nothing reloaded");
                    model.overlay = Overlay::Error(format!("write failed: {e}"));
                    Vec::new()
                }
            }
        }
        Msg::Planned(result) => {
            model.writes.in_flight = false;
            match result {
                // Keys typed while the plan was on its way belong to the
                // overlay they went to; a `y` there must not confirm this.
                Ok((plan, _)) if plan.request().is_some() && model.overlay != Overlay::None => {
                    model.status = StatusLine::warn("request discarded, nothing sent");
                }
                Ok((plan, kind)) if plan.request().is_some() => {
                    model.status = StatusLine::info("review the request: y sends it, n cancels");
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: 0,
                    };
                }
                // Provisioning with nothing pending is an answer, not a failure.
                Ok((Plan::Noop { reason, .. }, pce::write::ObjectKind::Provision)) => {
                    model.status = StatusLine::info(reason);
                }
                // Cannot happen with `IfMissing::Create`; reported, never sent.
                Ok((plan, _)) => {
                    let reason = match plan {
                        Plan::Noop { reason, .. } => reason,
                        Plan::Send { .. } => String::new(),
                    };
                    fail(
                        model,
                        "write",
                        &format!("nothing to send ({reason})"),
                        false,
                    );
                }
                Err(e) => fail(model, "write", &e, false),
            }
            Vec::new()
        }
    }
}

/// A failed request: the status line says what failed (or that a failing
/// auto-refresh switched itself off); long or authentication errors also open
/// the error overlay unless another overlay is in use.
/// Lines of the pretty-printed event, the unit the detail overlay scrolls in.
pub fn event_detail_lines(event: &pce::models::Event) -> usize {
    serde_json::to_string_pretty(event)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

fn fail(model: &mut Model, context: &str, e: &str, auto_off: bool) {
    model.status = StatusLine::error(if auto_off {
        format!("auto-refresh off: {e}")
    } else {
        format!("{context}: {e}")
    });
    if model.overlay == Overlay::None && (e.len() > 60 || e.contains("authentication failed")) {
        model.overlay = Overlay::Error(format!("{context}: {e}"));
    }
}

/// `r`: reruns the current filter past the cache (after the cost guard).
fn run_now(model: &mut Model, f: TrafficFilter) -> Vec<Cmd> {
    let cmds = start_query(model, f, CacheMode::Bypass);
    model.traffic.auto.rearm();
    cmds
}

fn start_query(model: &mut Model, filter: TrafficFilter, mode: CacheMode) -> Vec<Cmd> {
    let (seq, cancel) = model.traffic.start_query(Instant::now());
    model.status = StatusLine::info("query running");
    vec![Cmd::RunQuery {
        seq,
        filter,
        mode,
        cancel,
    }]
}

fn refresh_running_status(model: &mut Model) {
    refresh_running_status_at(model, Instant::now());
}

fn refresh_running_status_at(model: &mut Model, now: Instant) {
    if let QueryState::Running {
        started, status, ..
    } = &model.traffic.query
    {
        let secs = now.saturating_duration_since(*started).as_secs_f64();
        model.status = StatusLine::info(format!("query running {secs:.1}s · {status}"));
    }
}

fn on_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    if k.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if k.code == KeyCode::Char('c') && k.modifiers == KeyModifiers::CONTROL {
        model.quit = true;
        return vec![Cmd::Quit];
    }
    match model.overlay.clone() {
        Overlay::Help => {
            let last = help_last_scroll(model.size.1);
            match k.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.help_scroll = (model.help_scroll + 1).min(last);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.help_scroll = model.help_scroll.saturating_sub(1);
                }
                KeyCode::End | KeyCode::Char('G') => model.help_scroll = last,
                KeyCode::Home | KeyCode::Char('g') => model.help_scroll = 0,
                _ => {}
            }
            Vec::new()
        }
        Overlay::Prompt { mut input } => match k.code {
            KeyCode::Esc => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Enter => {
                model.overlay = Overlay::None;
                match commands::parse(&input) {
                    Ok(cmd) => update(model, Msg::Command(cmd)),
                    Err(e) => {
                        model.status = StatusLine::error(e);
                        Vec::new()
                    }
                }
            }
            KeyCode::Backspace => {
                input.pop();
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            KeyCode::Char(c) => {
                input.push(c);
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::GroupKeys { mut input } => match k.code {
            KeyCode::Esc => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Enter => {
                model.overlay = Overlay::None;
                let mut keys: Vec<String> = input
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if keys.is_empty() {
                    keys = vec!["app".into(), "env".into()];
                }
                model.traffic.group_keys = keys;
                model.traffic.drill = None;
                model.traffic.rebuild_views();
                Vec::new()
            }
            KeyCode::Backspace => {
                input.pop();
                model.overlay = Overlay::GroupKeys { input };
                Vec::new()
            }
            KeyCode::Char(c) => {
                input.push(c);
                model.overlay = Overlay::GroupKeys { input };
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::FilterEditor(mut e) => {
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::Down => {
                    e.next();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    e.prev();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Backspace => {
                    e.pop();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Char(c) => {
                    e.push(c);
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Enter => match e.to_filter() {
                    Ok(f) => {
                        model.overlay = Overlay::None;
                        return request_query(model, f);
                    }
                    Err(err) => {
                        e.error = Some(err);
                        model.overlay = Overlay::FilterEditor(e);
                    }
                },
                _ => model.overlay = Overlay::FilterEditor(e),
            }
            Vec::new()
        }
        Overlay::Confirm { action, .. } => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                model.overlay = Overlay::None;
                match action {
                    PendingAction::RunQuery(f) => update(model, Msg::ApplyFilter(f)),
                    PendingAction::RunNow(f) => run_now(model, f),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled");
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::EventsEditor {
            mut values,
            mut active,
            ..
        } => {
            let editor =
                |values: [String; 3], active: usize, error: Option<String>| Overlay::EventsEditor {
                    values,
                    active,
                    error,
                };
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::Down => {
                    active = (active + 1) % values.len();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active = (active + values.len() - 1) % values.len();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Backspace => {
                    values[active].pop();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Char(c) => {
                    values[active].push(c);
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Enter => {
                    let edit = EventsFilterEdit {
                        severity: values[0].trim().to_string(),
                        event_type: values[1].trim().to_string(),
                        window: values[2].trim().to_string(),
                    };
                    let applied = match edit.to_filter(jiff::Timestamp::now()) {
                        Ok(_) if model.events.loading => Err("events are still loading".into()),
                        other => other,
                    };
                    match applied {
                        Ok(filter) => {
                            model.overlay = Overlay::None;
                            model.events.filter = edit;
                            model.events.rebuild();
                            return request_events(model, filter, CacheMode::Use);
                        }
                        Err(err) => model.overlay = editor(values, active, Some(err)),
                    }
                }
                _ => model.overlay = editor(values, active, None),
            }
            Vec::new()
        }
        Overlay::EventDetail { event, scroll } => {
            // The renderer clamps to the last page; here only the top is a hard stop.
            let last = event_detail_lines(&event).saturating_sub(1);
            let next = match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                    return Vec::new();
                }
                KeyCode::Down | KeyCode::Char('j') => (scroll + 1).min(last),
                KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => (scroll + 10).min(last),
                KeyCode::PageUp => scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => 0,
                KeyCode::End | KeyCode::Char('G') => last,
                _ => scroll,
            };
            model.overlay = Overlay::EventDetail {
                event,
                scroll: next,
            };
            Vec::new()
        }
        Overlay::Form(mut form) => match form.on_key(k) {
            FormEvent::Continue => {
                model.overlay = Overlay::Form(form);
                Vec::new()
            }
            FormEvent::Cancel => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled, nothing sent");
                Vec::new()
            }
            FormEvent::Submit => {
                match edit::build_intent(
                    &form,
                    model.index().map(|i| i.as_ref()),
                    model.profile.org_id,
                ) {
                    Ok(Some(intent)) => {
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("planning…");
                        model.writes.in_flight = true;
                        vec![Cmd::PlanWrite(intent)]
                    }
                    Ok(None) => {
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("no changes");
                        Vec::new()
                    }
                    Err(e) => {
                        form.error = Some(e);
                        model.overlay = Overlay::Form(form);
                        Vec::new()
                    }
                }
            }
        },
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let last = plan
                .request()
                .map_or(0, |r| confirm_write_last_scroll(r, area));
            let last = u16::try_from(last).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if kind == pce::write::ObjectKind::Provision {
                        // The answer is the new policy version; the counts
                        // come from what is sent.
                        model.policy.provision_counts =
                            Some(counts_text(plan.request().and_then(|r| r.body.as_ref())));
                    }
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("sending …");
                    model.writes.in_flight = true;
                    vec![Cmd::ExecuteWrite { plan, kind }]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("cancelled, nothing sent");
                    Vec::new()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Overlay::Pending => {
            let n = model.policy.pending.as_ref().map_or(0, Vec::len);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.policy.pending_selected =
                        (model.policy.pending_selected + 1).min(n.saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.policy.pending_selected = model.policy.pending_selected.saturating_sub(1);
                }
                KeyCode::Char(' ') => {
                    let p = &mut model.policy;
                    if let Some(href) = p
                        .pending
                        .as_ref()
                        .and_then(|l| l.get(p.pending_selected))
                        .map(|c| c.href.clone())
                        && !p.pending_marked.remove(&href)
                    {
                        p.pending_marked.insert(href);
                    }
                }
                KeyCode::Char('P') => return provision_key(model),
                KeyCode::Enter => return pending_diff(model),
                _ => {}
            }
            Vec::new()
        }
        Overlay::Diff { diff, scroll } => {
            let last = u16::try_from(diff.lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::Pending,
                KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: (scroll + 1).min(last),
                    };
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: scroll.saturating_sub(1),
                    };
                }
                _ => {}
            }
            Vec::new()
        }
        Overlay::RuleDetail(_) | Overlay::Error(_) => {
            if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                model.overlay = Overlay::None;
            }
            Vec::new()
        }
        Overlay::None => on_screen_key(model, k),
    }
}

fn on_screen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    // An active table search captures every key except Ctrl-C.
    let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
    if !ctrl_c {
        if model.screen == Screen::Traffic && model.traffic.is_searching() {
            return traffic_key(model, k);
        }
        if model.screen == Screen::Policy && model.policy.is_searching() {
            return policy_key(model, k);
        }
        if model.screen == Screen::Objects && model.objects.is_searching() {
            return objects_key(model, k);
        }
        if model.screen == Screen::Workloads && model.workloads.table.is_searching() {
            return workloads_key(model, k);
        }
        if model.screen == Screen::Events && model.events.table.is_searching() {
            return events_key(model, k);
        }
    }
    if k.code == KeyCode::Char('r')
        && k.modifiers == KeyModifiers::NONE
        && matches!(model.connection, Connection::Disconnected { .. })
    {
        return reconnect(model);
    }
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => {
            model.quit = true;
            vec![Cmd::Quit]
        }
        (KeyCode::Char(':'), _) => {
            model.overlay = Overlay::Prompt {
                input: String::new(),
            };
            Vec::new()
        }
        (KeyCode::Char('?'), _) => {
            model.help_scroll = 0;
            model.overlay = Overlay::Help;
            Vec::new()
        }
        (KeyCode::Char('1'), _) => switch(model, Screen::Traffic),
        (KeyCode::Char('2'), _) => switch(model, Screen::Policy),
        (KeyCode::Char('3'), _) => switch(model, Screen::Objects),
        (KeyCode::Char('4'), _) => switch(model, Screen::Workloads),
        (KeyCode::Char('5'), _) => switch(model, Screen::Events),
        _ if model.screen == Screen::Traffic => traffic_key(model, k),
        _ if model.screen == Screen::Policy => policy_key(model, k),
        _ if model.screen == Screen::Objects => objects_key(model, k),
        _ if model.screen == Screen::Workloads => workloads_key(model, k),
        _ if model.screen == Screen::Events => events_key(model, k),
        _ => Vec::new(),
    }
}

/// `r` while disconnected: one user-caused reconnect attempt. Moving to
/// `Connecting` is what keeps a second `r` from sending another.
fn reconnect(model: &mut Model) -> Vec<Cmd> {
    model.connection = Connection::Connecting;
    model.objects.index_failed = false;
    model.workloads.index_failed = false;
    model.status = StatusLine::info("reconnecting…");
    vec![Cmd::Connect]
}

fn traffic_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::traffic::table_rows(c.body))
        .unwrap_or(10)
        .max(1)
}

/// Search editing keys, shared by every view's table.
fn search_key<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent) {
    match k.code {
        KeyCode::Esc => table.search_clear(),
        KeyCode::Enter => table.search_commit(),
        KeyCode::Backspace => table.search_pop(),
        KeyCode::Char(c) => table.search_push(c),
        _ => {}
    }
}

/// Navigation, sort and search-start keys, shared by every view's table.
fn table_key<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent, page: usize) {
    match k.code {
        KeyCode::Down | KeyCode::Char('j') => table.select_next(),
        KeyCode::Up | KeyCode::Char('k') => table.select_prev(),
        KeyCode::PageDown => table.page_down(page),
        KeyCode::PageUp => table.page_up(page),
        KeyCode::Home => table.home(),
        KeyCode::End => table.end(),
        KeyCode::Char('s') => table.sort_next(),
        KeyCode::Char('/') => table.start_search(),
        _ => {}
    }
}

fn traffic_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = traffic_page(model);
    if model.traffic.is_searching() {
        let t = &mut model.traffic;
        match t.view {
            TrafficView::Flows => {
                search_key(&mut t.flows, k);
                t.flows.ensure_visible(page);
            }
            TrafficView::Aggregate => {
                search_key(&mut t.groups, k);
                t.groups.ensure_visible(page);
            }
            TrafficView::Blocked => {
                search_key(&mut t.blocked, k);
                t.blocked.ensure_visible(page);
            }
        }
        return Vec::new();
    }
    match k.code {
        KeyCode::Char('f') => {
            model.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&model.traffic.filter));
            return Vec::new();
        }
        KeyCode::Char('r') => return update(model, Msg::RunNow),
        KeyCode::Char('v') => model.traffic.view = model.traffic.view.next(),
        KeyCode::Char('a') => {
            model.status = model.traffic.auto.toggle();
        }
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys {
                input: model.traffic.group_keys.join(","),
            };
            return Vec::new();
        }
        KeyCode::Enter => match model.traffic.view {
            TrafficView::Aggregate => {
                if let Some(g) = model.traffic.groups.selected_item() {
                    model.traffic.drill = Some((g.src.clone(), g.dst.clone()));
                    model.traffic.view = TrafficView::Flows;
                    model.traffic.rebuild_views();
                }
            }
            TrafficView::Flows => model.traffic.show_detail = !model.traffic.show_detail,
            TrafficView::Blocked => {}
        },
        KeyCode::Esc | KeyCode::Backspace
            if model.traffic.view == TrafficView::Flows && model.traffic.drill.is_some() =>
        {
            model.traffic.drill = None;
            model.traffic.rebuild_views();
        }
        _ => {
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows => table_key(&mut t.flows, k, page),
                TrafficView::Aggregate => table_key(&mut t.groups, k, page),
                TrafficView::Blocked => table_key(&mut t.blocked, k, page),
            }
        }
    }
    let t = &mut model.traffic;
    match t.view {
        TrafficView::Flows => t.flows.ensure_visible(page),
        TrafficView::Aggregate => t.groups.ensure_visible(page),
        TrafficView::Blocked => t.blocked.ensure_visible(page),
    }
    Vec::new()
}

/// Validated filter from the editor or a command: confirm if expensive, else apply.
fn request_query(model: &mut Model, f: TrafficFilter) -> Vec<Cmd> {
    if let Some(reason) = f.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!("{reason}. Run anyway? (y/n)"),
            action: PendingAction::RunQuery(f),
        };
        return Vec::new();
    }
    update(model, Msg::ApplyFilter(f))
}

/// Mouse: the wheel moves the focused table one row (also while a search is
/// active), a left click on the tabs row switches screen through the same path
/// as the number keys. Everything else is ignored; the wheel never requests anything.
fn on_mouse(model: &mut Model, me: crossterm::event::MouseEvent) -> Vec<Cmd> {
    use crossterm::event::{MouseButton, MouseEventKind};
    if model.overlay != Overlay::None {
        return Vec::new();
    }
    match me.kind {
        MouseEventKind::ScrollDown => scroll_focused(model, true),
        MouseEventKind::ScrollUp => scroll_focused(model, false),
        MouseEventKind::Down(MouseButton::Left) if me.row == 1 => {
            return match crate::tui::ui::tab_at(me.column, model.size.0) {
                Some(s) => switch(model, s),
                None => Vec::new(),
            };
        }
        _ => {}
    }
    Vec::new()
}

/// One row up or down in the current screen's focused table.
fn scroll_focused(model: &mut Model, down: bool) {
    fn step<T>(table: &mut crate::tui::table::TableView<T>, down: bool, page: usize) {
        if down {
            table.select_next();
        } else {
            table.select_prev();
        }
        table.ensure_visible(page);
    }
    match model.screen {
        Screen::Traffic => {
            let page = traffic_page(model);
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows => step(&mut t.flows, down, page),
                TrafficView::Aggregate => step(&mut t.groups, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
            }
        }
        Screen::Policy => {
            let page = policy_page(model);
            match model.policy.pane {
                PolicyPane::RuleSets => {
                    let before = model.policy.selected_href();
                    step(&mut model.policy.rulesets, down, page);
                    // The rules follow the ruleset; a missing deny list is not
                    // fetched from here (the wheel never requests).
                    if model.policy.selected_href() != before
                        && let Some(idx) = model.index().cloned()
                    {
                        model.policy.rebuild_rules(&idx);
                    }
                }
                PolicyPane::Rules => step(&mut model.policy.rules, down, page),
            }
        }
        Screen::Objects => {
            let page = objects_page(model);
            let o = &mut model.objects;
            match o.kind {
                ObjectKind::Labels => step(&mut o.labels, down, page),
                ObjectKind::LabelGroups => step(&mut o.groups, down, page),
                ObjectKind::IpLists => step(&mut o.ip_lists, down, page),
                ObjectKind::Services => step(&mut o.services, down, page),
            }
        }
        Screen::Workloads => {
            let page = workloads_page(model);
            step(&mut model.workloads.table, down, page);
        }
        Screen::Events => {
            let page = events_page(model);
            step(&mut model.events.table, down, page);
        }
    }
}

fn switch(model: &mut Model, s: Screen) -> Vec<Cmd> {
    model.screen = s;
    match s {
        // Loaded data is kept: returning to the screen never requests again.
        Screen::Policy
            if model.policy.loaded_at.is_none()
                && !model.policy.loading
                && model.is_connected() =>
        {
            load_rule_sets(model, CacheMode::Use)
        }
        Screen::Objects => load_missing_objects(model),
        Screen::Workloads => load_missing_workloads(model),
        Screen::Events => load_missing_events(model),
        _ => Vec::new(),
    }
}

fn load_rule_sets(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.policy.loading = true;
    model.policy.auto.last_run = Some(Instant::now());
    model.policy.auto.from_tick = false;
    vec![Cmd::LoadRuleSets {
        scope: model.policy.scope,
        mode,
    }]
}

/// Deny-rule endpoint request for the selected ruleset when its inline list is empty.
fn deny_fallback(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model
        .policy
        .take_deny_fallback()
        .map(|href| vec![Cmd::LoadDenyRules { href, mode }])
        .unwrap_or_default()
}

fn policy_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::policy::table_rows(c.body, model.policy.pane))
        .unwrap_or(10)
        .max(1)
}

fn policy_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = policy_page(model);
    if !model.policy.is_searching() {
        match k.code {
            KeyCode::Char('d') => {
                if model.policy.loading {
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                model.policy.scope = match model.policy.scope {
                    PolicyScope::Draft => PolicyScope::Active,
                    PolicyScope::Active => PolicyScope::Draft,
                };
                model.policy.clear();
                return load_rule_sets(model, CacheMode::Use);
            }
            KeyCode::Char('r') => {
                if model.policy.loading {
                    // A tick result would consume the deny bypass meant for this one.
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                model.policy.bypass_deny = true;
                model.policy.auto.rearm();
                return load_rule_sets(model, CacheMode::Bypass);
            }
            KeyCode::Char('p') => {
                if model.policy.pending_loading {
                    model.status = StatusLine::info("pending changes already loading");
                    return Vec::new();
                }
                model.policy.pending_loading = true;
                model.status = StatusLine::info("loading pending changes");
                return vec![Cmd::LoadPending(CacheMode::Use)];
            }
            KeyCode::Char('a') => {
                model.status = model.policy.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return policy_edit_key(model, c),
            // `N` (a deny rule) is bound on the rules pane only.
            KeyCode::Char('N') if model.policy.pane == PolicyPane::Rules => {
                return policy_edit_key(model, 'N');
            }
            KeyCode::Tab | KeyCode::BackTab => {
                model.policy.pane = match model.policy.pane {
                    PolicyPane::RuleSets => PolicyPane::Rules,
                    PolicyPane::Rules => PolicyPane::RuleSets,
                };
                return Vec::new();
            }
            KeyCode::Enter if model.policy.pane == PolicyPane::RuleSets => {
                return deny_fallback(model, CacheMode::Use);
            }
            KeyCode::Enter if model.policy.pane == PolicyPane::Rules => {
                if let Some(r) = model.policy.rules.selected_item() {
                    model.overlay = Overlay::RuleDetail(r.clone());
                }
                return Vec::new();
            }
            _ => {}
        }
    }
    let before = model.policy.selected_href();
    match model.policy.pane {
        PolicyPane::RuleSets => {
            let t = &mut model.policy.rulesets;
            if t.is_searching() {
                search_key(t, k);
            } else {
                table_key(t, k, page);
            }
            t.ensure_visible(page);
        }
        PolicyPane::Rules => {
            let t = &mut model.policy.rules;
            if t.is_searching() {
                search_key(t, k);
            } else {
                table_key(t, k, page);
            }
            t.ensure_visible(page);
        }
    }
    if model.policy.selected_href() != before
        && let Some(idx) = model.index().cloned()
    {
        model.policy.rebuild_rules(&idx);
        return deny_fallback(model, CacheMode::Use);
    }
    Vec::new()
}

/// Loads every kind that has neither arrived nor is in flight. Needs the name
/// index: it is built from the same four lists, so these are cache hits and
/// the screen costs no request.
fn load_missing_objects(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() {
        return Vec::new();
    }
    let missing: Vec<ObjectKind> = ObjectKind::ALL
        .into_iter()
        .filter(|k| model.objects.ages[k.index()].is_none() && !model.objects.loading[k.index()])
        .collect();
    missing
        .into_iter()
        .flat_map(|k| load_objects(model, k, CacheMode::Use))
        .collect()
}

fn load_objects(model: &mut Model, kind: ObjectKind, mode: CacheMode) -> Vec<Cmd> {
    model.objects.loading[kind.index()] = true;
    model.objects.auto.last_run = Some(Instant::now());
    model.objects.auto.from_tick = false;
    vec![Cmd::LoadObjects { kind, mode }]
}

/// Loads the workloads once. Waits for the name index like Objects does, so the
/// startup load never races a second request for the same data.
fn load_missing_workloads(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() || model.workloads.loaded_at.is_some() || model.workloads.loading {
        return Vec::new();
    }
    load_workloads(model, CacheMode::Use)
}

fn load_workloads(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.workloads.loading = true;
    model.workloads.auto.last_run = Some(Instant::now());
    model.workloads.auto.from_tick = false;
    vec![Cmd::LoadWorkloads(mode)]
}

/// Sent but not answered, whether or not its audit entry was written.
fn is_unanswered(e: &WriteError) -> bool {
    match e {
        WriteError::Unanswered(_) => true,
        WriteError::AuditFailed { write, .. } => is_unanswered(write),
        _ => false,
    }
}

/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
    match kind {
        pce::write::ObjectKind::Label => load_objects(model, ObjectKind::Labels, CacheMode::Bypass),
        pce::write::ObjectKind::IpList => {
            load_objects(model, ObjectKind::IpLists, CacheMode::Bypass)
        }
        pce::write::ObjectKind::Service => {
            load_objects(model, ObjectKind::Services, CacheMode::Bypass)
        }
        pce::write::ObjectKind::Workload => load_workloads(model, CacheMode::Bypass),
        // A draft write changes the draft view only; the reload keeps the
        // write's report in the status line.
        pce::write::ObjectKind::RuleSet
        | pce::write::ObjectKind::Rule
        | pce::write::ObjectKind::DenyRule => {
            if model.policy.scope != PolicyScope::Draft {
                return Vec::new();
            }
            model.policy.quiet_reload = true;
            load_rule_sets(model, CacheMode::Bypass)
        }
        // Provisioning changes both scopes and the pending list.
        pce::write::ObjectKind::Provision => {
            model.policy.quiet_reload = true;
            let mut cmds = load_rule_sets(model, CacheMode::Bypass);
            model.policy.pending_loading = true;
            cmds.push(Cmd::LoadPending(CacheMode::Bypass));
            cmds
        }
    }
}

/// `Enter` on the pending overlay: one diff request for an IP list, a
/// service or a ruleset; anything else says why there is none.
fn pending_diff(model: &mut Model) -> Vec<Cmd> {
    let Some(change) = model
        .policy
        .pending
        .as_ref()
        .and_then(|p| p.get(model.policy.pending_selected))
        .cloned()
    else {
        return Vec::new();
    };
    let kind = match change.object_type.as_str() {
        "ip_lists" => pce::write::ObjectKind::IpList,
        "services" => pce::write::ObjectKind::Service,
        "rule_sets" => pce::write::ObjectKind::RuleSet,
        // A rule is provisioned, and diffed, through its ruleset.
        "sec_rules" | "deny_rules" => {
            model.status = StatusLine::info("rule changes show in their ruleset's diff");
            return Vec::new();
        }
        other => {
            model.status = StatusLine::info(format!("no diff for {other}"));
            return Vec::new();
        }
    };
    if model.policy.diff_loading {
        model.status = StatusLine::info("diff already loading");
        return Vec::new();
    }
    model.policy.diff_loading = true;
    model.status = StatusLine::info(format!(
        "loading the active version of {}",
        change.name.as_deref().unwrap_or(&change.href)
    ));
    vec![Cmd::LoadDiff { change, kind }]
}

const PROVISION_DESCRIPTION: &str = "illumio-tui provision";

/// `P` in the pending overlay: provision the marked entries, or everything
/// pending when none is marked. The plan reads the pending list once more,
/// past the cache, and the confirm overlay shows the exact body.
fn provision_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.pending.as_ref().is_none_or(Vec::is_empty) {
        model.status = StatusLine::info("nothing pending");
        return Vec::new();
    }
    let marked = &model.policy.pending_marked;
    let subset = if marked.is_empty() {
        ProvisionSubset::All
    } else {
        ProvisionSubset::Hrefs(marked.iter().cloned().collect())
    };
    model.overlay = Overlay::None;
    model.status = StatusLine::info("planning…");
    model.writes.in_flight = true;
    vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
        description: Some(PROVISION_DESCRIPTION.into()),
        subset,
    }))]
}

/// `ip_lists=1, rule_sets=2`: what a provision body lists, per type, as the
/// CLI prints it.
fn counts_text(body: Option<&serde_json::Value>) -> String {
    body.map(provision_counts)
        .unwrap_or_default()
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn workloads_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            crate::tui::ui::screens::workloads::table_rows(c.body, model.workloads.show_detail)
        })
        .unwrap_or(10)
        .max(1)
}

/// What the traffic filter calls this workload: its readable name when that
/// resolves to exactly this workload, else its href id. `,` and `|` separate
/// filter terms, and the resolver takes the first hostname, name or href-suffix
/// match, so a name with those characters or shared with another loaded
/// workload is addressed by the id.
fn workload_filter_ref(w: &pce::models::Workload, loaded: &[pce::models::Workload]) -> String {
    let name = w.display_name();
    let ambiguous = loaded.iter().any(|o| {
        o.href != w.href
            && (o
                .hostname
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&name))
                || o.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                || o.href
                    .to_ascii_lowercase()
                    .ends_with(&name.to_ascii_lowercase()))
    });
    let name = if ambiguous || name.contains([',', '|']) {
        pce::models::Ref::new(&w.href).id().to_string()
    } else {
        name
    };
    format!("wl:{name}")
}

fn workloads_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = workloads_page(model);
    if !model.workloads.table.is_searching() {
        match k.code {
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.workloads.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("workloads are still loading");
                } else if model.workloads.loading {
                    model.status = StatusLine::info("workloads already loading");
                } else {
                    model.workloads.auto.rearm();
                    return load_workloads(model, CacheMode::Bypass);
                }
                return Vec::new();
            }
            KeyCode::Enter => {
                let w = &mut model.workloads;
                w.show_detail = !w.show_detail && w.table.selected_item().is_some();
                return Vec::new();
            }
            KeyCode::Char('h') => {
                let w = &mut model.workloads;
                w.unhealthy_only = !w.unhealthy_only;
                w.rebuild();
                w.show_detail &= w.table.selected_item().is_some();
                return Vec::new();
            }
            KeyCode::Char('H') => {
                model.workloads.sort_by_health();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.workloads.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('t' | 'T')) => {
                let Some(w) = model.workloads.table.selected_item() else {
                    return Vec::new();
                };
                let expr = workload_filter_ref(&w.workload, &model.workloads.raw);
                let mut f = model.traffic.filter.clone();
                if c == 't' {
                    f.src = expr;
                } else {
                    f.dst = expr;
                }
                // The switch to Traffic happens when the query starts (`ApplyFilter`),
                // so declining the cost guard leaves the user on Workloads.
                return request_query(model, f);
            }
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return workloads_edit_key(model, c),
            _ => {}
        }
    }
    drive_table(&mut model.workloads.table, k, page);
    Vec::new()
}

/// Loads the events once. Unlike the other lists they are not in the startup
/// index, and they need no name index: a connection is enough.
fn load_missing_events(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() || model.events.loaded_at.is_some() || model.events.loading {
        return Vec::new();
    }
    load_events(model, CacheMode::Use)
}

/// Requests the events for the applied filter. A filter that no longer
/// validates (it was validated when applied) is reported, never sent.
fn load_events(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    match model.events.filter.to_filter(jiff::Timestamp::now()) {
        Ok(filter) => request_events(model, filter, mode),
        Err(e) => {
            model.status = StatusLine::error(e);
            Vec::new()
        }
    }
}

fn request_events(model: &mut Model, filter: EventFilter, mode: CacheMode) -> Vec<Cmd> {
    model.events.loading = true;
    model.events.auto.last_run = Some(Instant::now());
    model.events.auto.from_tick = false;
    vec![Cmd::LoadEvents { filter, mode }]
}

fn events_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::events::table_rows(c.body))
        .unwrap_or(10)
        .max(1)
}

fn events_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = events_page(model);
    if !model.events.table.is_searching() {
        match k.code {
            KeyCode::Char('f') => {
                let f = &model.events.filter;
                model.overlay = Overlay::EventsEditor {
                    values: [f.severity.clone(), f.event_type.clone(), f.window.clone()],
                    active: 0,
                    error: None,
                };
                return Vec::new();
            }
            KeyCode::Char('r') => {
                if model.events.loading {
                    model.status = StatusLine::info("events already loading");
                    return Vec::new();
                }
                model.events.auto.rearm();
                return load_events(model, CacheMode::Bypass);
            }
            KeyCode::Enter => {
                if let Some(e) = model.events.table.selected_item() {
                    model.overlay = Overlay::EventDetail {
                        event: e.clone(),
                        scroll: 0,
                    };
                }
                return Vec::new();
            }
            KeyCode::Char('S') => {
                model.events.sort_by_severity();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.events.auto.toggle();
                return Vec::new();
            }
            _ => {}
        }
    }
    drive_table(&mut model.events.table, k, page);
    Vec::new()
}

fn objects_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::objects::table_rows(c.body, model.objects.show_detail))
        .unwrap_or(10)
        .max(1)
}

/// Search editing, navigation and sort on one table, keeping the selection in view.
fn drive_table<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent, page: usize) {
    if table.is_searching() {
        search_key(table, k);
    } else {
        table_key(table, k, page);
    }
    table.ensure_visible(page);
}

fn objects_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = objects_page(model);
    if !model.objects.is_searching() {
        let kind = model.objects.kind;
        match k.code {
            KeyCode::Tab | KeyCode::BackTab => {
                model.objects.kind = if k.code == KeyCode::Tab {
                    kind.next()
                } else {
                    kind.prev()
                };
                model.objects.show_detail = false;
                return Vec::new();
            }
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.objects.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("objects are still loading");
                } else if model.objects.loading[kind.index()] {
                    model.status = StatusLine::info(format!("{} already loading", kind.title()));
                } else {
                    model.objects.auto.rearm();
                    return load_objects(model, kind, CacheMode::Bypass);
                }
                return Vec::new();
            }
            KeyCode::Enter => {
                let o = &mut model.objects;
                o.show_detail = !o.show_detail && o.has_selection();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.objects.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return objects_edit_key(model, c),
            _ => {}
        }
    }
    let o = &mut model.objects;
    match o.kind {
        ObjectKind::Labels => drive_table(&mut o.labels, k, page),
        ObjectKind::LabelGroups => drive_table(&mut o.groups, k, page),
        ObjectKind::IpLists => drive_table(&mut o.ip_lists, k, page),
        ObjectKind::Services => drive_table(&mut o.services, k, page),
    }
    Vec::new()
}

/// Writes off, or a write already on its way: the reason goes to the status
/// line and the key does nothing else.
fn writes_refused(model: &mut Model) -> bool {
    if !model.writes.enabled {
        model.status = StatusLine::warn(model.writes.refusal());
        return true;
    }
    if model.writes.in_flight {
        model.status = StatusLine::warn("a write is already in progress");
        return true;
    }
    false
}

/// A create or update opens its form; a delete goes straight to planning.
fn begin_edit(model: &mut Model, intent: EditIntent) -> Vec<Cmd> {
    match edit::start(intent, model.index().map(|i| i.as_ref())) {
        Start::Form(f) => {
            model.overlay = Overlay::Form(f);
            Vec::new()
        }
        Start::Plan(w) => {
            model.status = StatusLine::info("planning…");
            model.writes.in_flight = true;
            vec![Cmd::PlanWrite(w)]
        }
    }
}

/// `n` / `e` / `x` on Objects: labels, IP lists, services; label groups are read-only.
fn objects_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let o = &model.objects;
    let intent = match (o.kind, c) {
        (ObjectKind::LabelGroups, _) => {
            model.status = StatusLine::warn("label groups are read-only in this version");
            return Vec::new();
        }
        (ObjectKind::Labels, 'n') => Some(EditIntent::CreateLabel),
        (ObjectKind::IpLists, 'n') => Some(EditIntent::CreateIpList),
        (ObjectKind::Services, 'n') => Some(EditIntent::CreateService),
        (ObjectKind::Labels, 'e') => o
            .labels
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateLabel),
        (ObjectKind::Labels, _) => o
            .labels
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteLabel),
        (ObjectKind::IpLists, 'e') => o
            .ip_lists
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateIpList),
        (ObjectKind::IpLists, _) => o
            .ip_lists
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteIpList),
        (ObjectKind::Services, 'e') => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateService),
        (ObjectKind::Services, _) => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteService),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` / `e` / `x` on Workloads. Edit and delete only for workloads the PCE
/// reports as unmanaged with no VEN (the library's rule, checked again there).
fn workloads_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if c == 'n' {
        return begin_edit(model, EditIntent::CreateWorkload);
    }
    let Some(w) = model
        .workloads
        .table
        .selected_item()
        .map(|r| r.workload.clone())
    else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    if w.managed != Some(false) || w.ven.is_some() {
        model.status = StatusLine::warn("only unmanaged workloads can be edited here");
        return Vec::new();
    }
    let intent = if c == 'e' {
        EditIntent::UpdateWorkload(w)
    } else {
        EditIntent::DeleteWorkload(w)
    };
    begin_edit(model, intent)
}

/// `n` / `e` / `x` on the rulesets pane: draft only.
fn policy_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.scope == PolicyScope::Active {
        model.status = StatusLine::warn("switch to draft to edit policy");
        return Vec::new();
    }
    let intent = match model.policy.pane {
        PolicyPane::RuleSets => ruleset_intent(&model.policy, c),
        PolicyPane::Rules => rule_intent(&model.policy, c),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` a new ruleset; `e` / `x` the selected one.
fn ruleset_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    if c == 'n' {
        return Some(EditIntent::CreateRuleSet);
    }
    let rs = p.selected_rule_set()?.clone();
    Some(if c == 'e' {
        EditIntent::UpdateRuleSet(rs)
    } else {
        EditIntent::DeleteRuleSet(rs)
    })
}

/// `n` / `N` a new allow / deny rule in the selected ruleset; `e` / `x` the
/// selected rule, allow or deny by its kind, found by href in what was loaded.
fn rule_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    let ruleset_href = p.selected_href()?;
    match c {
        'n' => return Some(EditIntent::CreateRule { ruleset_href }),
        'N' => return Some(EditIntent::CreateDenyRule { ruleset_href }),
        _ => {}
    }
    let view = p.rules.selected_item()?;
    let href = view.href.clone();
    match (c, view.kind == RuleKind::Allow) {
        ('x', true) => Some(EditIntent::DeleteRule { href }),
        ('x', false) => Some(EditIntent::DeleteDenyRule { href }),
        (_, true) => p
            .find_rule(&href)
            .map(|(r, rs)| EditIntent::UpdateRule(r, rs)),
        (_, false) => p
            .find_deny_rule(&href)
            .map(|(d, rs)| EditIntent::UpdateDenyRule(d, rs)),
    }
}

/// `:draft` / `:active`: like `d` on the Policy screen, but explicit. The scope
/// already shown is not reloaded (like `2`); a load in flight is not doubled.
fn show_policy(model: &mut Model, scope: PolicyScope) -> Vec<Cmd> {
    if !model.is_connected() {
        model.status = StatusLine::error("not connected");
        return Vec::new();
    }
    if model.policy.scope != scope {
        if model.policy.loading {
            model.screen = Screen::Policy;
            model.status = StatusLine::info("already loading");
            return Vec::new();
        }
        model.policy.scope = scope;
        model.policy.clear();
    }
    switch(model, Screen::Policy)
}

pub fn on_command(model: &mut Model, cmd: Command) -> Vec<Cmd> {
    match cmd {
        Command::Screen(s) => switch(model, s),
        Command::Quit => {
            model.quit = true;
            vec![Cmd::Quit]
        }
        Command::Help => {
            model.help_scroll = 0;
            model.overlay = Overlay::Help;
            Vec::new()
        }
        Command::Filter(expr) => {
            let mut f = model.traffic.filter.clone();
            f.src = expr;
            match f.validate() {
                Ok(()) => request_query(model, f),
                Err(e) => {
                    model.status = StatusLine::error(e);
                    Vec::new()
                }
            }
        }
        Command::Window(w) => {
            let mut f = model.traffic.filter.clone();
            f.window = w;
            match f.validate() {
                Ok(()) => request_query(model, f),
                Err(e) => {
                    model.status = StatusLine::error(e);
                    Vec::new()
                }
            }
        }
        Command::Profile(name) => {
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
                return Vec::new();
            }
            if model.switching {
                model.status = StatusLine::info("profile switch in progress");
                return Vec::new();
            }
            model.switching = true;
            vec![Cmd::SwitchProfile(name)]
        }
        Command::Draft => show_policy(model, PolicyScope::Draft),
        Command::Active => show_policy(model, PolicyScope::Active),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::model::{Connection, Model, Screen, StatusKind};
    use crate::tui::msg::{Cmd, Msg};
    use crate::tui::test_support::model_fixture;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn v_cycles_views_and_g_sets_group_keys_without_requests() {
        use crate::tui::model::{Overlay, TrafficView};
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        assert!(
            m.traffic.groups.len() >= 3,
            "groups built from data: {}",
            m.traffic.groups.len()
        );
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert!(update(&mut m, key('g')).is_empty());
        assert!(matches!(m.overlay, Overlay::GroupKeys { .. }));
        for _ in 0..7 {
            assert!(
                update(
                    &mut m,
                    Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
                )
                .is_empty()
            );
        }
        for c in "role".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(m.traffic.group_keys, vec!["role"]);
        assert!(
            m.traffic.groups.rows().iter().any(|g| g.src == vec!["web"]),
            "{:?}",
            m.traffic.groups.rows()
        );
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Blocked);
        assert_eq!(m.traffic.blocked.len(), 2);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
    }

    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
        use crate::tui::model::TrafficView;
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        update(&mut m, key('v'));
        // first group is the 310-connection shop/prod -> shop/prod pair
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert!(m.traffic.drill.is_some());
        assert_eq!(m.traffic.flows.len(), 1);
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(m.traffic.drill.is_none());
        assert_eq!(m.traffic.flows.len(), 4);
    }

    #[test]
    fn query_done_rebuilds_all_views() {
        use crate::tui::filter::TrafficFilter;
        use crate::tui::test_support::fixture_flows;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        update(&mut m, Msg::ApplyFilter(f.clone()));
        let outcome = crate::tui::msg::QueryOutcome {
            flows: fixture_flows(),
            truncated: false,
            matches_count: Some(4),
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert!(!m.traffic.groups.is_empty());
        assert_eq!(m.traffic.blocked.len(), 2);
    }

    #[test]
    fn auto_refresh_respects_interval_running_and_focus() {
        use crate::tui::test_support::model_with_flows;
        use std::time::{Duration, Instant};
        let mut m = model_with_flows();
        let t0 = Instant::now();
        assert!(update(&mut m, Msg::Tick(t0)).is_empty(), "off by default");
        update(&mut m, key('a'));
        assert!(m.traffic.auto.enabled);
        assert!(m.status.text.contains("60s"), "{}", m.status.text);
        // first tick after enabling: not due, last_run is the data's fetch time, 3 s ago.
        assert!(update(&mut m, Msg::Tick(t0)).is_empty());
        let later = t0 + Duration::from_secs(61);
        let cmds = update(&mut m, Msg::Tick(later));
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::RunQuery {
                    mode: pce::CacheMode::Bypass,
                    ..
                }]
            ),
            "{cmds:?}"
        );
        // while running, no second run
        assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(120))).is_empty());
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("boom".into()),
            },
        );
        assert!(!m.traffic.auto.enabled, "a failed tick query disarms it");
        update(&mut m, key('a'));
        assert!(m.traffic.auto.enabled, "`a` re-arms");
        // unfocused: no run even when due
        update(&mut m, Msg::Focus(false));
        assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(200))).is_empty());
        update(&mut m, Msg::Focus(true));
        assert!(!update(&mut m, Msg::Tick(later + Duration::from_secs(200))).is_empty());
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("boom".into()),
            },
        );
        assert!(!m.traffic.auto.enabled);
        assert_eq!(m.status.text, "auto-refresh off: boom");
        assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(1000))).is_empty());
    }

    #[test]
    fn auto_refresh_interval_never_below_profile_minimum() {
        use crate::tui::model::AutoRefresh;
        let a = AutoRefresh::new(10);
        assert_eq!(a.interval, std::time::Duration::from_secs(60));
        let b = AutoRefresh::new(300);
        assert_eq!(b.interval, std::time::Duration::from_secs(300));
    }

    #[test]
    fn auto_refresh_without_data_does_nothing() {
        use std::time::{Duration, Instant};
        let mut m = model_fixture();
        update(&mut m, key('a'));
        assert!(
            update(&mut m, Msg::Tick(Instant::now() + Duration::from_secs(600))).is_empty(),
            "nothing to refresh until a query ran"
        );
    }

    #[test]
    fn result_panel_shows_countdown_only_while_auto_is_enabled() {
        use crate::tui::test_support::{model_with_flows, render_to_string};
        let mut m = model_with_flows();
        assert!(!render_to_string(&m, 120, 30).contains("auto in"));
        update(&mut m, key('a'));
        assert!(render_to_string(&m, 120, 30).contains("auto in 5"));
    }

    #[test]
    fn auto_refresh_is_held_back_by_overlay_and_other_screens() {
        use crate::tui::test_support::model_with_flows;
        use std::time::{Duration, Instant};
        let mut m = model_with_flows();
        update(&mut m, key('a'));
        let due = Instant::now() + Duration::from_secs(600);
        m.overlay = Overlay::Help;
        assert!(update(&mut m, Msg::Tick(due)).is_empty());
        m.overlay = Overlay::None;
        m.screen = Screen::Policy;
        assert!(update(&mut m, Msg::Tick(due)).is_empty());
    }

    #[test]
    fn search_captures_keys_on_aggregate_and_blocked_views() {
        use crate::tui::test_support::model_with_flows;
        for presses in [1, 2] {
            let mut m = model_with_flows();
            for _ in 0..presses {
                update(&mut m, key('v'));
            }
            update(&mut m, key('/'));
            for c in ['q', ':', '?'] {
                update(&mut m, key(c));
            }
            assert_eq!(m.screen, Screen::Traffic);
            assert!(!m.quit);
            assert_eq!(m.overlay, Overlay::None);
            let table_search = |m: &Model| match m.traffic.view {
                TrafficView::Aggregate => m.traffic.groups.search_text().to_string(),
                TrafficView::Blocked => m.traffic.blocked.search_text().to_string(),
                TrafficView::Flows => unreachable!("v moved off Flows"),
            };
            assert_eq!(table_search(&m), "q:?");
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            );
            assert!(!m.traffic.is_searching());
            assert_eq!(table_search(&m), "");
        }
    }

    #[test]
    fn search_captures_global_keys() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        update(&mut m, key('/'));
        for c in ['1', '0', '.', 'q', ':', '?'] {
            update(&mut m, key(c));
        }
        assert_eq!(m.screen, Screen::Traffic);
        assert!(!m.quit);
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.flows.search_text(), "10.q:?");
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(!m.traffic.flows.is_searching());
        assert_eq!(m.traffic.flows.search_text(), "");
        update(&mut m, key('q'));
        assert!(m.quit);
    }

    #[test]
    fn flow_table_navigation_sort_search_and_detail_never_emit_commands() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        assert_eq!(m.traffic.flows.len(), 4);
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(m.traffic.flows.selected_pos(), 1);
        assert!(update(&mut m, key('j')).is_empty());
        assert_eq!(m.traffic.flows.selected_pos(), 2);
        assert!(update(&mut m, key('k')).is_empty());
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(m.traffic.flows.selected_pos(), 3);
        assert!(update(&mut m, key('s')).is_empty());
        assert_eq!(m.traffic.flows.sort(), Some((0, false)));
        assert!(update(&mut m, key('/')).is_empty());
        assert!(m.traffic.flows.is_searching());
        for c in "nas".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert_eq!(m.traffic.flows.len(), 1);
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert!(!m.traffic.flows.is_searching());
        assert!(update(&mut m, key('/')).is_empty());
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(m.traffic.flows.len(), 4, "Esc clears the search");
        assert!(!m.traffic.show_detail);
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert!(m.traffic.show_detail);
    }

    #[test]
    fn query_done_fills_the_flow_table() {
        use crate::tui::filter::TrafficFilter;
        use crate::tui::test_support::fixture_flows;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        update(&mut m, Msg::ApplyFilter(f.clone()));
        let outcome = crate::tui::msg::QueryOutcome {
            flows: fixture_flows(),
            truncated: false,
            matches_count: Some(4),
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert_eq!(m.traffic.flows.len(), 4);
        assert_eq!(m.traffic.flows.selected_pos(), 0);
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut m = model_fixture();
        assert_eq!(update(&mut m, key('q')), vec![Cmd::Quit]);
        assert!(m.quit);
        let mut m = model_fixture();
        let cmds = update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );
        assert_eq!(cmds, vec![Cmd::Quit]);
    }

    #[test]
    fn number_keys_switch_screens_policy_objects_and_workloads_load() {
        let mut m = model_fixture();
        assert_eq!(m.screen, Screen::Traffic);
        // entering Policy for the first time loads its rulesets (see the policy tests)
        assert_eq!(update(&mut m, key('2')).len(), 1);
        assert_eq!(m.screen, Screen::Policy);
        // entering Objects for the first time loads all four kinds (cache hits)
        assert_eq!(update(&mut m, key('3')).len(), 4);
        assert_eq!(m.screen, Screen::Objects);
        // entering Workloads for the first time loads the list once (a cache hit)
        assert_eq!(
            update(&mut m, key('4')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
        );
        assert_eq!(m.screen, Screen::Workloads);
        assert!(update(&mut m, key('1')).is_empty());
        assert_eq!(m.screen, Screen::Traffic);
    }

    #[test]
    fn startup_connect_flow() {
        let mut m = Model::new_connecting(
            model_fixture().profile,
            model_fixture().theme_name.clone(),
            60,
            1000,
            "24h".into(),
            crate::tui::model::WritesState::default(),
        );
        assert!(matches!(m.connection, Connection::Connecting));
        let cmds = update(&mut m, Msg::Connected);
        assert_eq!(cmds, vec![Cmd::LoadIndex(pce::CacheMode::Use)]);
        assert_eq!(
            m.status.text,
            "connected; loading labels, label groups, IP lists, services"
        );
        let cmds = update(
            &mut m,
            Msg::ConnectFailed("authentication failed (HTTP 401)".into()),
        );
        assert!(cmds.is_empty());
        match &m.connection {
            Connection::Disconnected { error } => assert!(error.contains("401")),
            other => panic!("{other:?}"),
        }
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn resize_and_focus_update_model() {
        let mut m = model_fixture();
        update(&mut m, Msg::Resize(120, 40));
        assert_eq!(m.size, (120, 40));
        update(&mut m, Msg::Focus(false));
        assert!(!m.focused);
    }

    #[test]
    fn colon_opens_prompt_enter_runs_command_and_errors_show_in_status() {
        use crate::tui::model::Overlay;
        let mut m = model_fixture();
        update(&mut m, key(':'));
        assert!(matches!(m.overlay, Overlay::Prompt { .. }));
        for c in "policy".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.screen, Screen::Policy);

        update(&mut m, key(':'));
        for c in "bogus".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(
            m.status.text.contains("unknown command"),
            "{}",
            m.status.text
        );
        assert_eq!(m.screen, Screen::Policy, "a bad command changes nothing");

        update(&mut m, key(':'));
        update(&mut m, key('x'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
    }

    #[test]
    fn help_overlay_toggles_and_swallows_keys() {
        use crate::tui::model::Overlay;
        let mut m = model_fixture();
        update(&mut m, key('?'));
        assert!(matches!(m.overlay, Overlay::Help));
        update(&mut m, key('2'));
        assert_eq!(
            m.screen,
            Screen::Traffic,
            "keys go to the overlay, not the screen"
        );
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        update(&mut m, key('?'));
        update(&mut m, key('?'));
        assert!(matches!(m.overlay, Overlay::None), "? toggles");
        assert!(
            update(&mut m, Msg::Command(crate::tui::commands::Command::Quit)).contains(&Cmd::Quit)
        );
    }

    #[test]
    fn f_opens_editor_enter_applies_or_confirms() {
        use crate::tui::model::{Overlay, PendingAction};
        let mut m = model_fixture();
        update(&mut m, key('f'));
        assert!(matches!(m.overlay, Overlay::FilterEditor(_)));
        // Type a source filter into field 1.
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        for c in "app=shop".chars() {
            update(&mut m, key(c));
        }
        let msgs = update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.traffic.filter.src, "app=shop");
        assert!(
            matches!(msgs.as_slice(), [Cmd::RunQuery { .. }]),
            "an applied filter runs the query"
        );

        // A big window asks for confirmation before anything happens.
        update(&mut m, key('f'));
        // field 0 = window: clear and type 30d
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "30d".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        match &m.overlay {
            Overlay::Confirm {
                reason,
                action: PendingAction::RunQuery(f),
            } => {
                assert!(reason.contains("30d"), "{reason}");
                assert_eq!(f.window, "30d");
            }
            other => panic!("{other:?}"),
        }
        update(&mut m, key('n'));
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.traffic.filter.window, "24h", "declined: filter unchanged");
        update(&mut m, key('f'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
    }

    #[test]
    fn editor_shows_validation_error_and_stays_open() {
        use crate::tui::model::Overlay;
        let mut m = model_fixture();
        update(&mut m, key('f'));
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "soon".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        match &m.overlay {
            Overlay::FilterEditor(e) => {
                assert!(e.error.as_deref().unwrap_or("").contains("window"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn filter_and_window_commands_apply_without_the_editor() {
        use crate::tui::commands::Command;
        let mut m = model_fixture();
        update(&mut m, Msg::Command(Command::Filter("env=prod".into())));
        assert_eq!(m.traffic.filter.src, "env=prod");
        update(&mut m, Msg::Command(Command::Window("2h".into())));
        assert_eq!(m.traffic.filter.window, "2h");
        update(&mut m, Msg::Command(Command::Window("never".into())));
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.traffic.filter.window, "2h");
    }

    fn running(m: &Model) -> bool {
        m.traffic.is_running()
    }

    #[test]
    fn apply_filter_runs_query_and_r_reruns_with_bypass() {
        use crate::tui::filter::TrafficFilter;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        let cmds = update(&mut m, Msg::ApplyFilter(f.clone()));
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Cmd::RunQuery {
                seq, filter, mode, ..
            } => {
                assert_eq!(*seq, 1);
                assert_eq!(filter, &f);
                assert_eq!(*mode, pce::CacheMode::Use);
            }
            other => panic!("{other:?}"),
        }
        assert!(running(&m));
        // r while running: no-op
        let cmds = update(&mut m, key('r'));
        assert!(cmds.is_empty());
        assert!(
            m.status.text.contains("already running"),
            "{}",
            m.status.text
        );
        // finish it
        let outcome = crate::tui::msg::QueryOutcome {
            flows: vec![],
            truncated: false,
            matches_count: Some(0),
            elapsed: std::time::Duration::from_millis(5),
            hit: false,
            filter: f.clone(),
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert!(!running(&m));
        assert!(m.traffic.data.is_some());
        // r now reruns with Bypass and a new seq
        let cmds = update(&mut m, key('r'));
        match &cmds[0] {
            Cmd::RunQuery { seq, mode, .. } => {
                assert_eq!(*seq, 2);
                assert_eq!(*mode, pce::CacheMode::Bypass);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn new_filter_cancels_running_query_and_ignores_its_late_result() {
        use crate::tui::filter::TrafficFilter;
        use crate::tui::model::QueryState;
        let mut m = model_fixture();
        update(&mut m, Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)));
        let first_token = match &m.traffic.query {
            QueryState::Running { cancel, .. } => cancel.clone(),
            other => panic!("{other:?}"),
        };
        let mut f2 = TrafficFilter::defaults("1h", 50);
        f2.src = "app=shop".into();
        let cmds = update(&mut m, Msg::ApplyFilter(f2));
        assert!(first_token.is_cancelled(), "old query cancelled");
        assert!(matches!(cmds[0], Cmd::RunQuery { seq: 2, .. }));
        // The old query's late error must not disturb the new one.
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Err("cancelled".into()),
            },
        );
        assert!(running(&m));
        assert!(!matches!(m.traffic.query, QueryState::Failed(_)));
        update(
            &mut m,
            Msg::QueryDone {
                seq: 2,
                result: Err("traffic query failed: boom".into()),
            },
        );
        assert!(matches!(&m.traffic.query, QueryState::Failed(e) if e.contains("boom")));
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn progress_updates_status_while_running() {
        use crate::tui::filter::TrafficFilter;
        use pce::traffic::QueryProgress;
        let mut m = model_fixture();
        update(&mut m, Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)));
        update(
            &mut m,
            Msg::QueryProgress {
                seq: 1,
                progress: QueryProgress::Polling {
                    status: "working".into(),
                    elapsed: std::time::Duration::from_secs(2),
                },
            },
        );
        assert!(m.status.text.contains("working"), "{}", m.status.text);
        update(
            &mut m,
            Msg::QueryProgress {
                seq: 7,
                progress: QueryProgress::Downloading,
            },
        );
        assert!(m.status.text.contains("working"), "stale seq ignored");
    }

    #[test]
    fn truncated_result_warns() {
        use crate::tui::filter::TrafficFilter;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        update(&mut m, Msg::ApplyFilter(f.clone()));
        let outcome = crate::tui::msg::QueryOutcome {
            flows: vec![],
            truncated: true,
            matches_count: Some(900),
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert!(m.status.text.contains("truncated"), "{}", m.status.text);
    }

    #[test]
    fn confirm_requires_y_and_enter_is_ignored() {
        use crate::tui::model::Overlay;
        let enter = || Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut m = model_fixture();
        update(&mut m, key('f'));
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "30d".chars() {
            update(&mut m, key(c));
        }
        assert!(update(&mut m, enter()).is_empty());
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        let cmds = update(&mut m, enter());
        assert!(cmds.is_empty(), "Enter must not confirm");
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        let cmds = update(&mut m, key('y'));
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Cmd::RunQuery {
                mode: pce::CacheMode::Use,
                ..
            }
        ));
        assert_eq!(m.traffic.filter.window, "30d");
    }

    // ---- Policy screen (Task 10) ----

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn opening_policy_loads_rulesets_once_and_d_toggles_scope() {
        use pce::models::PolicyScope;
        let mut m = model_fixture();
        let cmds = update(&mut m, key('2'));
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: pce::CacheMode::Use
            }]
        );
        assert!(m.policy.loading);
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0)),
            },
        );
        assert!(!m.policy.loading);
        assert_eq!(m.policy.rulesets.len(), 2);
        assert_eq!(
            m.policy.rules.len(),
            4,
            "allow + inline deny rules of the first ruleset"
        );
        let kinds: Vec<_> = m.policy.rules.rows().iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                pce::RuleKind::OverrideDeny,
                pce::RuleKind::Allow,
                pce::RuleKind::Allow,
                pce::RuleKind::Deny
            ],
            "evaluation order: override deny, allow, deny"
        );
        assert!(update(&mut m, key('1')).is_empty());
        assert!(
            update(&mut m, key('2')).is_empty(),
            "already loaded: no request"
        );
        let cmds = update(&mut m, key('d'));
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Active,
                mode: pce::CacheMode::Use
            }]
        );
        assert_eq!(m.policy.scope, PolicyScope::Active);
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Active,
                result: Ok((Vec::new(), 0)),
            },
        );
        let cmds = update(&mut m, key('r'));
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Active,
                mode: pce::CacheMode::Bypass
            }]
        );
    }

    #[test]
    fn selecting_a_ruleset_rebuilds_rules_and_enter_opens_detail() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        assert_eq!(m.policy.rules.len(), 4);
        update(&mut m, kc(KeyCode::Down));
        assert_eq!(m.policy.rules.len(), 1, "second ruleset has one rule");
        update(&mut m, kc(KeyCode::Tab));
        update(&mut m, kc(KeyCode::Enter));
        match &m.overlay {
            Overlay::RuleDetail(r) => assert_eq!(r.providers[0].name, "All workloads"),
            other => panic!("{other:?}"),
        }
        update(&mut m, kc(KeyCode::Esc));
        assert!(matches!(m.overlay, Overlay::None));
    }

    #[test]
    fn p_loads_pending_and_shows_the_panel() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        let cmds = update(&mut m, key('p'));
        assert_eq!(cmds, vec![Cmd::LoadPending(pce::CacheMode::Use)]);
        let pending = pce::models::parse_pending(
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
        );
        update(&mut m, Msg::PendingLoaded(Ok(pending)));
        assert!(matches!(m.overlay, Overlay::Pending));
        assert_eq!(m.policy.pending.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn empty_inline_deny_rules_fall_back_to_the_endpoint() {
        use pce::models::PolicyScope;
        let mut m = model_fixture();
        update(&mut m, key('2'));
        let mut rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        rs[0].deny_rules.clear();
        let cmds = update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0)),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadDenyRules {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                mode: pce::CacheMode::Use
            }]
        );
        let deny: Vec<pce::models::DenyRule> =
            serde_json::from_str(include_str!("../../../../fixtures/deny_rules.json")).unwrap();
        update(
            &mut m,
            Msg::DenyRulesLoaded {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                result: Ok(deny),
            },
        );
        assert_eq!(m.policy.rules.len(), 4);
    }

    #[test]
    fn fallback_is_requested_once_per_ruleset_even_when_revisited() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        // the second ruleset has no inline deny rules: first visit requests, revisit does not
        let first = update(&mut m, kc(KeyCode::Down));
        assert_eq!(
            first,
            vec![Cmd::LoadDenyRules {
                href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
                mode: pce::CacheMode::Use
            }]
        );
        update(&mut m, kc(KeyCode::Up));
        assert!(update(&mut m, kc(KeyCode::Down)).is_empty());
        // a "not found" answer is remembered as empty
        update(
            &mut m,
            Msg::DenyRulesLoaded {
                href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
                result: Err("not found".into()),
            },
        );
        update(&mut m, kc(KeyCode::Up));
        assert!(update(&mut m, kc(KeyCode::Down)).is_empty());
    }

    #[test]
    fn result_for_the_other_scope_is_ignored_and_keeps_loading() {
        use pce::models::PolicyScope;
        let mut m = model_fixture();
        update(&mut m, key('2'));
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs.clone(), 0)),
            },
        );
        update(&mut m, key('d')); // Active now loading
        // A late draft answer (e.g. a duplicate) must not land in the active scope.
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0)),
            },
        );
        assert!(m.policy.loading, "the active-scope load is still in flight");
        assert!(m.policy.rulesets.is_empty());
    }

    #[test]
    fn enter_without_a_rule_does_nothing_and_search_captures_keys() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        update(&mut m, key('/'));
        for c in "qd2".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert!(!m.quit, "q is typed into the search");
        assert_eq!(m.screen, Screen::Policy);
        assert_eq!(m.policy.scope, pce::models::PolicyScope::Draft);
        assert!(m.policy.rulesets.is_empty(), "nothing matches qd2");
        assert!(m.policy.rules.is_empty());
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.policy.rulesets.len(), 2);
        update(&mut m, kc(KeyCode::Tab));
        m.policy.rules.set_rows(Vec::new());
        update(&mut m, kc(KeyCode::Enter));
        assert!(matches!(m.overlay, Overlay::None));
    }

    #[test]
    fn policy_without_an_index_requests_nothing_until_connected() {
        let mut m = model_fixture();
        m.connection = Connection::Connecting;
        assert!(update(&mut m, key('2')).is_empty());
        assert!(!m.policy.loading);
    }

    #[test]
    fn policy_auto_refresh_obeys_the_guards() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_policy;
        use std::time::Duration;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        let now = Instant::now() + Duration::from_secs(120);
        m.policy.auto.last_run = Some(Instant::now());
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
        update(&mut m, key('a'));
        assert!(m.policy.auto.enabled);
        let early = Instant::now() + Duration::from_secs(10);
        assert!(
            update(&mut m, Msg::Tick(early)).is_empty(),
            "minimum interval"
        );
        m.focused = false;
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
        m.focused = true;
        m.overlay = Overlay::Help;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not under an overlay"
        );
        m.overlay = Overlay::None;
        m.policy.loading = true;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not while loading"
        );
        m.policy.loading = false;
        assert_eq!(
            update(&mut m, Msg::Tick(now)),
            vec![Cmd::LoadRuleSets {
                scope: pce::models::PolicyScope::Draft,
                mode: pce::CacheMode::Bypass
            }]
        );
        m.screen = Screen::Traffic;
        m.policy.loading = false;
        m.policy.auto.last_run = Some(Instant::now());
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "only on its own screen"
        );
    }

    fn objects_fixture_msgs() -> Vec<Msg> {
        use crate::tui::model::ObjectKind;
        use crate::tui::msg::ObjectsPayload;
        let labels =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        vec![
            Msg::ObjectsLoaded {
                kind: ObjectKind::Labels,
                result: Ok(ObjectsPayload::Labels(labels, 120)),
            },
            Msg::ObjectsLoaded {
                kind: ObjectKind::LabelGroups,
                result: Ok(ObjectsPayload::LabelGroups(groups, 120)),
            },
            Msg::ObjectsLoaded {
                kind: ObjectKind::IpLists,
                result: Ok(ObjectsPayload::IpLists(lists, 120)),
            },
            Msg::ObjectsLoaded {
                kind: ObjectKind::Services,
                result: Ok(ObjectsPayload::Services(services, 120)),
            },
        ]
    }

    #[test]
    fn opening_objects_loads_all_four_once_and_tab_cycles() {
        use crate::tui::model::ObjectKind;
        use crate::tui::msg::ObjectsPayload;
        let mut m = model_fixture();
        let cmds = update(&mut m, key('3'));
        assert_eq!(cmds.len(), 4);
        assert!(cmds.iter().all(|c| matches!(
            c,
            Cmd::LoadObjects {
                mode: pce::CacheMode::Use,
                ..
            }
        )));
        let labels: Vec<pce::models::Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        update(
            &mut m,
            Msg::ObjectsLoaded {
                kind: ObjectKind::Labels,
                result: Ok(ObjectsPayload::Labels(labels, 120)),
            },
        );
        assert_eq!(m.objects.labels.len(), 6);
        assert_eq!(m.objects.ages[0], Some(120));
        assert!(update(&mut m, key('1')).is_empty());
        assert!(
            update(&mut m, key('3')).is_empty(),
            "no reload while the other kinds are still loading"
        );
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!(m.objects.kind, ObjectKind::LabelGroups);
        update(&mut m, kc(KeyCode::BackTab));
        assert_eq!(m.objects.kind, ObjectKind::Labels);
        let cmds = update(&mut m, key('r'));
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Labels,
                mode: pce::CacheMode::Bypass
            }]
        );
        assert!(
            update(&mut m, key('r')).is_empty(),
            "a second r while the kind is loading adds no request"
        );
    }

    #[test]
    fn objects_load_only_what_is_missing_and_never_before_the_index() {
        use crate::tui::model::ObjectKind;
        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        let idx = index.take();
        assert!(
            update(&mut m, key('3')).is_empty(),
            "the startup load is still running: no duplicate requests"
        );
        assert!(m.objects.loading.iter().all(|l| !l));
        update(
            &mut m,
            Msg::IndexLoaded {
                index: idx.unwrap(),
                age_secs: 0,
            },
        );
        assert!(m.objects.loading.iter().all(|l| *l));
        // a failed kind is retried on re-entry, loaded kinds are not
        for msg in objects_fixture_msgs().into_iter().take(3) {
            update(&mut m, msg);
        }
        update(
            &mut m,
            Msg::ObjectsLoaded {
                kind: ObjectKind::Services,
                result: Err("boom".into()),
            },
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(m.objects.ages[3].is_none());
        update(&mut m, key('1'));
        assert_eq!(
            update(&mut m, key('3')),
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Services,
                mode: pce::CacheMode::Use
            }]
        );
    }

    #[test]
    fn a_late_result_for_another_kind_still_fills_its_table() {
        use crate::tui::model::ObjectKind;
        let mut m = model_fixture();
        update(&mut m, key('3'));
        update(&mut m, kc(KeyCode::Tab)); // LabelGroups is current
        let msgs = objects_fixture_msgs();
        update(&mut m, msgs.into_iter().nth(2).unwrap()); // IpLists arrives
        assert_eq!(m.objects.kind, ObjectKind::LabelGroups);
        assert_eq!(m.objects.ip_lists.len(), 2);
        assert_eq!(m.objects.ages[2], Some(120));
        assert!(!m.objects.loading[2]);
        assert!(m.objects.loading[1]);
    }

    #[test]
    fn objects_search_and_detail_are_local() {
        use crate::tui::test_support::model_with_objects;
        let mut m = model_with_objects();
        assert!(update(&mut m, key('3')).is_empty());
        assert!(update(&mut m, key('/')).is_empty());
        for c in "role".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert_eq!(m.objects.labels.len(), 2);
        assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
        assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
        assert!(m.objects.show_detail);
    }

    #[test]
    fn objects_search_captures_keys_including_tab() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = model_with_objects();
        update(&mut m, key('3'));
        update(&mut m, key('/'));
        for c in "qr3".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        update(&mut m, kc(KeyCode::Tab));
        assert!(!m.quit);
        assert_eq!(m.screen, Screen::Objects);
        assert_eq!(
            m.objects.kind,
            ObjectKind::Labels,
            "Tab does not switch while searching"
        );
        assert!(m.objects.labels.is_empty(), "nothing matches qr3");
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.objects.labels.len(), 6);
    }

    #[test]
    fn enter_toggles_detail_only_with_a_selection_and_tab_closes_it() {
        use crate::tui::test_support::model_with_objects;
        let mut m = model_with_objects();
        update(&mut m, key('3'));
        m.objects.labels.set_rows(Vec::new());
        update(&mut m, kc(KeyCode::Enter));
        assert!(!m.objects.show_detail, "nothing selected");
        let mut m = model_with_objects();
        update(&mut m, key('3'));
        update(&mut m, kc(KeyCode::Enter));
        assert!(m.objects.show_detail);
        update(&mut m, kc(KeyCode::Tab));
        assert!(!m.objects.show_detail);
        update(&mut m, kc(KeyCode::Enter));
        assert!(m.objects.show_detail);
        update(&mut m, kc(KeyCode::Enter));
        assert!(!m.objects.show_detail);
    }

    #[test]
    fn objects_auto_refresh_obeys_the_guards_and_reloads_only_the_current_kind() {
        use crate::tui::model::{ObjectKind, Overlay};
        use crate::tui::test_support::model_with_objects;
        use std::time::Duration;
        let mut m = model_with_objects();
        update(&mut m, key('3'));
        update(&mut m, kc(KeyCode::Tab)); // LabelGroups
        let now = Instant::now() + Duration::from_secs(120);
        m.objects.auto.last_run = Some(Instant::now());
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
        update(&mut m, key('a'));
        assert!(m.objects.auto.enabled);
        let early = Instant::now() + Duration::from_secs(10);
        assert!(
            update(&mut m, Msg::Tick(early)).is_empty(),
            "minimum interval"
        );
        m.focused = false;
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
        m.focused = true;
        m.overlay = Overlay::Help;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not under an overlay"
        );
        m.overlay = Overlay::None;
        m.objects.loading[1] = true;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not while that kind loads"
        );
        m.objects.loading[1] = false;
        assert_eq!(
            update(&mut m, Msg::Tick(now)),
            vec![Cmd::LoadObjects {
                kind: ObjectKind::LabelGroups,
                mode: pce::CacheMode::Bypass
            }],
            "current kind only, bypassing the cache (opted in)"
        );
        m.screen = Screen::Traffic;
        m.objects.loading[1] = false;
        m.objects.auto.last_run = Some(Instant::now());
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "only on its own screen"
        );
    }

    #[test]
    fn explicit_r_on_policy_bypasses_the_deny_fallback_but_auto_refresh_does_not() {
        use crate::tui::test_support::model_with_policy;
        use pce::models::PolicyScope;
        use std::time::Duration;
        let href = "/orgs/5/sec_policy/draft/rule_sets/100";
        let reload = |m: &mut Model| {
            let mut rs: Vec<pce::models::RuleSet> =
                serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
            rs[0].deny_rules.clear();
            update(
                m,
                Msg::RuleSetsLoaded {
                    scope: PolicyScope::Draft,
                    result: Ok((rs, 0)),
                },
            )
        };
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        update(&mut m, key('r'));
        assert_eq!(
            reload(&mut m),
            vec![Cmd::LoadDenyRules {
                href: href.into(),
                mode: pce::CacheMode::Bypass
            }]
        );
        // the bypass applies to that one reload only
        assert_eq!(
            reload(&mut m),
            vec![Cmd::LoadDenyRules {
                href: href.into(),
                mode: pce::CacheMode::Use
            }]
        );
        update(&mut m, key('a'));
        let now = Instant::now() + Duration::from_secs(120);
        m.policy.auto.last_run = Some(Instant::now());
        assert!(matches!(
            update(&mut m, Msg::Tick(now)).as_slice(),
            [Cmd::LoadRuleSets {
                mode: pce::CacheMode::Bypass,
                ..
            }]
        ));
        assert_eq!(
            reload(&mut m),
            vec![Cmd::LoadDenyRules {
                href: href.into(),
                mode: pce::CacheMode::Use
            }]
        );
    }

    #[test]
    fn policy_reload_keeps_fetched_deny_rows_until_the_new_ones_arrive() {
        use pce::models::PolicyScope;
        let href = "/orgs/5/sec_policy/draft/rule_sets/100";
        let load = |m: &mut Model| {
            let mut rs: Vec<pce::models::RuleSet> =
                serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
            rs[0].deny_rules.clear();
            update(
                m,
                Msg::RuleSetsLoaded {
                    scope: PolicyScope::Draft,
                    result: Ok((rs, 0)),
                },
            )
        };
        let mut m = model_fixture();
        update(&mut m, key('2'));
        load(&mut m);
        let deny: Vec<pce::models::DenyRule> =
            serde_json::from_str(include_str!("../../../../fixtures/deny_rules.json")).unwrap();
        update(
            &mut m,
            Msg::DenyRulesLoaded {
                href: href.into(),
                result: Ok(deny),
            },
        );
        assert_eq!(m.policy.rules.len(), 4);
        let cmds = load(&mut m);
        assert_eq!(cmds.len(), 1, "the fallback is asked again after a reload");
        assert_eq!(m.policy.rules.len(), 4, "no flicker while it is in flight");
        // a scope switch still forgets everything
        update(&mut m, key('d'));
        assert!(m.policy.rules.is_empty());
    }

    #[test]
    fn objects_recover_from_a_failed_index_with_r() {
        use crate::tui::model::ObjectKind;
        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        let idx = index.take().unwrap();
        update(&mut m, Msg::IndexFailed("boom".into()));
        update(&mut m, key('1'));
        assert!(update(&mut m, Msg::SwitchScreen(Screen::Objects)).is_empty());
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadIndex(pce::CacheMode::Use)]
        );
        assert_eq!(m.status.text, "loading name index…");
        assert!(
            update(&mut m, key('r')).is_empty(),
            "no second index request while it is in flight"
        );
        let cmds = update(
            &mut m,
            Msg::IndexLoaded {
                index: idx,
                age_secs: 0,
            },
        );
        assert_eq!(cmds.len(), 4);
        assert!(cmds.contains(&Cmd::LoadObjects {
            kind: ObjectKind::Labels,
            mode: pce::CacheMode::Use
        }));
    }

    #[test]
    fn a_failed_explicit_policy_refresh_does_not_leak_its_bypass() {
        use crate::tui::test_support::model_with_policy;
        use pce::models::PolicyScope;
        use std::time::Duration;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        update(&mut m, key('r'));
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Err("boom".into()),
            },
        );
        update(&mut m, key('a'));
        m.policy.auto.last_run = Some(Instant::now());
        let now = Instant::now() + Duration::from_secs(120);
        assert!(matches!(
            update(&mut m, Msg::Tick(now)).as_slice(),
            [Cmd::LoadRuleSets { .. }]
        ));
        let mut rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        rs[0].deny_rules.clear();
        let cmds = update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0)),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadDenyRules {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                mode: pce::CacheMode::Use
            }]
        );
    }

    #[test]
    fn opening_workloads_loads_once_and_t_jumps_to_traffic() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_fixture();
        let cmds = update(&mut m, key('4'));
        assert_eq!(cmds, vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]);
        let wl: Vec<pce::models::Workload> =
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0))));
        assert_eq!(m.workloads.table.len(), 4);
        assert!(update(&mut m, key('4')).is_empty());

        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        let cmds = update(&mut m, key('t'));
        assert_eq!(m.screen, Screen::Traffic);
        assert_eq!(m.traffic.filter.src, "wl:host-2.example");
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::RunQuery {
                    mode: pce::CacheMode::Use,
                    ..
                }]
            ),
            "{cmds:?}"
        );
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("x".into()),
            },
        );
        update(&mut m, key('4'));
        let cmds = update(&mut m, key('T'));
        assert_eq!(m.traffic.filter.dst, "wl:host-2.example");
        assert_eq!(
            m.traffic.filter.src, "wl:host-2.example",
            "src kept from before"
        );
        assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
    }

    #[test]
    fn workloads_wait_for_the_index_and_recover_from_a_failed_one() {
        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        let idx = index.take().unwrap();
        assert!(
            update(&mut m, key('4')).is_empty(),
            "the startup load is still running: no duplicate requests"
        );
        assert!(!m.workloads.loading);
        assert_eq!(
            update(
                &mut m,
                Msg::IndexLoaded {
                    index: idx,
                    age_secs: 0
                }
            ),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
        );
        assert!(m.workloads.loading);

        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        index.take();
        update(&mut m, key('4'));
        update(&mut m, Msg::IndexFailed("boom".into()));
        assert!(m.workloads.index_failed);
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadIndex(pce::CacheMode::Use)]
        );
        assert!(update(&mut m, key('r')).is_empty(), "retry is issued once");
    }

    #[test]
    fn workloads_error_keeps_rows_and_r_bypasses_unless_loading() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
        );
        assert!(m.workloads.loading);
        assert!(update(&mut m, key('r')).is_empty(), "already loading");
        update(&mut m, Msg::WorkloadsLoaded(Err("boom".into())));
        assert!(!m.workloads.loading);
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.workloads.table.len(), 4, "rows kept on error");
    }

    #[test]
    fn workloads_enter_toggles_detail_only_with_a_selection() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_fixture();
        m.screen = Screen::Workloads;
        update(&mut m, kc(KeyCode::Enter));
        assert!(!m.workloads.show_detail, "nothing selected");
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        update(&mut m, kc(KeyCode::Enter));
        assert!(m.workloads.show_detail);
        update(&mut m, kc(KeyCode::Enter));
        assert!(!m.workloads.show_detail);
    }

    #[test]
    fn t_needs_a_selection_and_does_not_fire_while_searching() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_fixture();
        m.screen = Screen::Workloads;
        assert!(update(&mut m, key('t')).is_empty());
        assert_eq!(m.screen, Screen::Workloads, "nothing selected: stay");

        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        update(&mut m, key('/'));
        assert!(update(&mut m, key('t')).is_empty());
        assert_eq!(m.screen, Screen::Workloads);
        assert_eq!(
            m.workloads.table.search_text(),
            "t",
            "typed into the search"
        );
        assert!(m.traffic.filter.src.is_empty());
    }

    #[test]
    fn t_goes_through_the_cost_guard() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        m.traffic.filter.window = "30d".into();
        let cmds = update(&mut m, key('t'));
        assert!(cmds.is_empty(), "{cmds:?}");
        assert_eq!(m.screen, Screen::Workloads, "the query has not started");
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        assert!(m.traffic.filter.src.is_empty(), "not applied before y");
        let cmds = update(&mut m, key('y'));
        assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
        assert_eq!(m.screen, Screen::Traffic);
        assert_eq!(m.traffic.filter.src, "wl:host-1.example");
    }

    #[test]
    fn t_while_a_query_runs_replaces_it_with_one_new_query() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        let first = update(&mut m, key('t'));
        assert!(matches!(first.as_slice(), [Cmd::RunQuery { .. }]));
        let seq = m.traffic.seq;
        update(&mut m, key('4'));
        let second = update(&mut m, key('T'));
        assert!(matches!(second.as_slice(), [Cmd::RunQuery { .. }]));
        assert_eq!(m.traffic.seq, seq + 1, "the older query is superseded");
    }

    #[test]
    fn jump_uses_the_href_id_when_the_name_would_break_the_filter_syntax() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        m.workloads.raw[0].hostname = Some("db, primary|1".into());
        m.workloads.raw[1].hostname = Some("my host".into());
        m.workloads.rebuild();
        update(&mut m, key('4'));
        update(&mut m, key('t'));
        assert_eq!(m.traffic.filter.src, "wl:aaaa-1");
        assert!(m.traffic.filter.validate().is_ok());
        update(&mut m, key('4'));
        update(&mut m, key('j'));
        update(&mut m, key('T'));
        assert_eq!(m.traffic.filter.dst, "wl:my host", "spaces are fine");
        assert!(m.traffic.filter.validate().is_ok());
    }

    #[test]
    fn h_toggles_unhealthy_only_and_capital_h_sorts_by_health() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        assert!(update(&mut m, key('4')).is_empty());
        assert_eq!(m.workloads.table.len(), 4);
        assert!(update(&mut m, key('h')).is_empty());
        assert!(m.workloads.unhealthy_only);
        assert_eq!(
            m.workloads.table.len(),
            2,
            "host-2 (offline) and host-4 (suspended) are unhealthy at the fixed test clock"
        );
        assert!(update(&mut m, key('h')).is_empty());
        assert_eq!(m.workloads.table.len(), 4);
        assert!(update(&mut m, key('H')).is_empty());
        assert_eq!(
            m.workloads
                .table
                .selected_item()
                .unwrap()
                .workload
                .hostname
                .as_deref(),
            Some("host-2.example"),
            "worst health first"
        );
    }

    #[test]
    fn h_with_no_unhealthy_rows_is_empty_not_a_panic_and_capital_h_flips() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        m.workloads.raw.remove(3);
        m.workloads.raw.remove(1);
        m.workloads.rebuild();
        update(&mut m, key('h'));
        assert_eq!(m.workloads.table.len(), 0);
        assert!(m.workloads.table.selected_item().is_none());
        assert_eq!(
            m.workloads.summary(),
            [1, 1, 0, 0, 0],
            "summary ignores the filter"
        );
        update(&mut m, key('h'));
        update(&mut m, key('H'));
        assert_eq!(m.workloads.table.sort(), Some((8, true)));
        update(&mut m, key('H'));
        assert_eq!(m.workloads.table.sort(), Some((8, false)), "second H flips");
        assert_eq!(
            m.workloads
                .table
                .selected_item()
                .unwrap()
                .workload
                .hostname
                .as_deref(),
            Some("host-1.example"),
            "ascending: healthy before unmanaged"
        );
    }

    #[test]
    fn workloads_auto_refresh_obeys_the_guards_and_bypasses() {
        use crate::tui::test_support::model_with_workloads;
        use std::time::Duration;
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        let now = Instant::now() + Duration::from_secs(120);
        m.workloads.auto.last_run = Some(Instant::now());
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
        update(&mut m, key('a'));
        assert!(m.workloads.auto.enabled);
        let early = Instant::now() + Duration::from_secs(10);
        assert!(
            update(&mut m, Msg::Tick(early)).is_empty(),
            "minimum interval"
        );
        m.focused = false;
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
        m.focused = true;
        m.overlay = Overlay::Help;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not under an overlay"
        );
        m.overlay = Overlay::None;
        m.workloads.loading = true;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not while loading"
        );
        m.workloads.loading = false;
        assert_eq!(
            update(&mut m, Msg::Tick(now)),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
        );
        m.screen = Screen::Traffic;
        m.workloads.loading = false;
        m.workloads.auto.last_run = Some(Instant::now());
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "only on its own screen"
        );
    }

    #[test]
    fn a_second_profile_switch_waits_for_the_pending_one() {
        use crate::tui::commands::Command;
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        assert_eq!(
            update(&mut m, Msg::Command(Command::Profile("a".into()))),
            vec![Cmd::SwitchProfile("a".into())]
        );
        assert!(m.switching);
        assert!(update(&mut m, Msg::Command(Command::Profile("b".into()))).is_empty());
        assert_eq!(m.status.text, "profile switch in progress");
        update(&mut m, Msg::ProfileSwitched(Err("boom".into())));
        assert!(!m.switching);
        assert_eq!(
            update(&mut m, Msg::Command(Command::Profile("b".into()))),
            vec![Cmd::SwitchProfile("b".into())]
        );
    }

    #[test]
    fn profile_command_resets_state_and_reconnects() {
        use crate::tui::commands::Command;
        use crate::tui::model::{Limits, ProfileInfo};
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        let cmds = update(&mut m, Msg::Command(Command::Profile("prod".into())));
        assert_eq!(cmds, vec![Cmd::SwitchProfile("prod".into())]);
        assert!(
            m.traffic.data.is_some(),
            "nothing changes until the switch succeeds"
        );
        let cmds = update(
            &mut m,
            Msg::ProfileSwitched(Ok((
                ProfileInfo {
                    name: "prod".into(),
                    host: "prod.example".into(),
                    org_id: 9,
                },
                Limits {
                    auto_refresh_min_seconds: 300,
                    traffic_max_results: 500,
                    traffic_default_window: "1h".into(),
                },
            ))),
        );
        assert_eq!(cmds, vec![Cmd::Connect]);
        assert_eq!(m.limits.traffic_max_results, 500);
        assert_eq!(m.limits.auto_refresh_min_seconds, 300);
        assert_eq!(m.limits.traffic_default_window, "1h");
        assert_eq!(m.traffic.filter.max_results, 500);
        assert_eq!(m.traffic.filter.window, "1h");
        assert_eq!(m.traffic.auto.interval.as_secs(), 300);
        assert!(matches!(m.connection, Connection::Connecting));
        assert!(m.traffic.data.is_none());
        assert_eq!(m.traffic.flows.len(), 0);
        assert_eq!(m.profile.host, "prod.example");
        assert_eq!(m.stats.calls, 0);
        let mut m2 = model_with_flows();
        update(
            &mut m2,
            Msg::ProfileSwitched(Err("profile \"nope\" not found".into())),
        );
        assert!(m2.traffic.data.is_some());
        assert_eq!(m2.status.kind, StatusKind::Error);
        assert_eq!(m2.profile.name, "demo");
    }

    #[test]
    fn disconnected_model_still_takes_profile_and_quit() {
        use crate::tui::commands::Command;
        let mut m = model_fixture();
        m.connection = Connection::Disconnected {
            error: "authentication failed (HTTP 401)".into(),
        };
        assert_eq!(
            update(&mut m, Msg::Command(Command::Profile("other".into()))),
            vec![Cmd::SwitchProfile("other".into())]
        );
        assert_eq!(update(&mut m, key('q')), vec![Cmd::Quit]);
        assert!(
            update(&mut m, key('2')).is_empty(),
            "no loads while disconnected"
        );
    }

    #[test]
    fn draft_and_active_commands_and_error_overlay() {
        use crate::tui::commands::Command;
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_policy;
        use pce::models::PolicyScope;
        let mut m = model_with_policy();
        let cmds = update(&mut m, Msg::Command(Command::Active));
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Active,
                mode: pce::CacheMode::Use
            }]
        );
        assert_eq!(m.screen, Screen::Policy);
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Active,
                result: Err(
                    "authentication failed (HTTP 401); check API key, secret and org id".into(),
                ),
            },
        );
        assert!(matches!(m.overlay, Overlay::Error(_)));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        let cmds = update(&mut m, Msg::Command(Command::Draft));
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: pce::CacheMode::Use
            }]
        );
    }

    #[test]
    fn draft_and_active_do_nothing_while_disconnected() {
        use crate::tui::commands::Command;
        let mut m = model_fixture();
        m.connection = Connection::Disconnected { error: "x".into() };
        assert!(update(&mut m, Msg::Command(Command::Active)).is_empty());
        assert_eq!(m.status.text, "not connected");
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn long_errors_open_the_overlay_short_ones_stay_in_the_status_line() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        update(&mut m, Msg::WorkloadsLoaded(Err("timeout".into())));
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.status.text, "could not load workloads: timeout");
        let long = "x".repeat(200);
        update(&mut m, Msg::WorkloadsLoaded(Err(long.clone())));
        match &m.overlay {
            Overlay::Error(t) => assert!(t.contains(&long)),
            o => panic!("{o:?}"),
        }
        for code in [KeyCode::Enter, KeyCode::Esc] {
            m.overlay = Overlay::Error("e".into());
            update(&mut m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            assert!(matches!(m.overlay, Overlay::None));
        }
        m.overlay = Overlay::Error("e".into());
        assert!(
            update(&mut m, key('2')).is_empty(),
            "keys do not leak through"
        );
        assert!(matches!(m.overlay, Overlay::Error(_)));
    }

    #[test]
    fn an_error_never_replaces_an_open_editor() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        m.overlay = Overlay::Prompt {
            input: "fil".into(),
        };
        update(&mut m, Msg::WorkloadsLoaded(Err("x".repeat(100))));
        assert!(matches!(m.overlay, Overlay::Prompt { .. }));
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn auth_failure_at_startup_opens_the_overlay_and_disconnects() {
        let mut m = model_fixture();
        m.connection = Connection::Connecting;
        update(
            &mut m,
            Msg::ConnectFailed("authentication failed (HTTP 401)".into()),
        );
        assert!(matches!(m.connection, Connection::Disconnected { .. }));
        assert!(matches!(m.overlay, Overlay::Error(_)));
    }

    #[test]
    fn reset_data_cancels_the_query_and_disarms_every_auto_refresh() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        let f = m.traffic.filter.clone();
        let cmds = update(&mut m, Msg::ApplyFilter(f));
        let Cmd::RunQuery { cancel, .. } = &cmds[0] else {
            panic!("{cmds:?}")
        };
        let cancel = cancel.clone();
        m.policy.auto.enabled = true;
        m.objects.auto.enabled = true;
        m.workloads.auto.enabled = true;
        m.traffic.auto.enabled = true;
        m.traffic.auto.from_tick = true;
        m.objects.index_failed = true;
        m.workloads.index_failed = true;
        m.workloads.loading = true;
        m.policy.loading = true;
        m.policy.bypass_deny = true;
        m.busy = 3;
        m.overlay = Overlay::Help;
        m.reset_data();
        assert!(cancel.is_cancelled(), "a running query is cancelled");
        assert!(!m.any_auto_enabled());
        assert!(!m.traffic.auto.from_tick);
        assert!(!m.objects.index_failed && !m.workloads.index_failed);
        assert!(!m.workloads.loading && !m.policy.loading && !m.policy.bypass_deny);
        assert!(m.workloads.loaded_at.is_none() && m.policy.loaded_at.is_none());
        assert!(!m.traffic.is_running());
        assert_eq!((m.busy, m.overlay.clone()), (0, Overlay::None));
        assert_eq!(m.workloads.table.len(), 0);
    }

    #[test]
    fn auto_refresh_failure_disables_it_on_traffic() {
        use crate::tui::test_support::model_with_flows;
        use std::time::Duration;
        let mut m = model_with_flows();
        update(&mut m, key('a'));
        let now = Instant::now() + Duration::from_secs(120);
        let cmds = update(&mut m, Msg::Tick(now));
        assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("HTTP 429".into()),
            },
        );
        assert!(!m.traffic.auto.enabled, "never retried unattended");
        assert_eq!(m.status.text, "auto-refresh off: HTTP 429");
        assert!(update(&mut m, Msg::Tick(now + Duration::from_secs(600))).is_empty());
        // A manual run that fails is reported normally.
        update(&mut m, key('a'));
        assert!(m.traffic.auto.enabled, "`a` re-arms");
        update(&mut m, key('r'));
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("HTTP 500".into()),
            },
        );
        assert!(m.traffic.auto.enabled, "a manual failure does not trip it");
        assert_eq!(m.status.text, "query: HTTP 500");
    }

    #[test]
    fn auto_refresh_failure_disables_it_on_load_based_screens() {
        use crate::tui::test_support::{
            model_with_objects, model_with_policy, model_with_workloads,
        };
        use std::time::Duration;
        let now = Instant::now() + Duration::from_secs(120);

        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        update(&mut m, key('a'));
        m.workloads.auto.last_run = Some(Instant::now());
        assert_eq!(
            update(&mut m, Msg::Tick(now)),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
        );
        update(&mut m, Msg::WorkloadsLoaded(Err("HTTP 429".into())));
        assert!(!m.workloads.auto.enabled);
        assert_eq!(m.status.text, "auto-refresh off: HTTP 429");
        assert!(update(&mut m, Msg::Tick(now + Duration::from_secs(600))).is_empty());
        // `r` is a user-caused retry and re-arms what the failure switched off.
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
        );
        assert!(m.workloads.auto.enabled);

        let mut m = model_with_policy();
        update(&mut m, key('2'));
        update(&mut m, key('a'));
        m.policy.auto.last_run = Some(Instant::now());
        assert!(matches!(
            update(&mut m, Msg::Tick(now)).as_slice(),
            [Cmd::LoadRuleSets { .. }]
        ));
        let scope = m.policy.scope;
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope,
                result: Err("HTTP 429".into()),
            },
        );
        assert!(!m.policy.auto.enabled);
        assert_eq!(m.status.text, "auto-refresh off: HTTP 429");

        let mut m = model_with_objects();
        update(&mut m, key('3'));
        update(&mut m, key('a'));
        m.objects.auto.last_run = Some(Instant::now());
        assert!(matches!(
            update(&mut m, Msg::Tick(now)).as_slice(),
            [Cmd::LoadObjects { .. }]
        ));
        let kind = m.objects.kind;
        update(
            &mut m,
            Msg::ObjectsLoaded {
                kind,
                result: Err("HTTP 429".into()),
            },
        );
        assert!(!m.objects.auto.enabled);
        assert_eq!(m.status.text, "auto-refresh off: HTTP 429");
    }

    #[test]
    fn r_while_disconnected_reconnects_once_and_never_says_still_loading() {
        for screen in ['3', '4'] {
            let mut m = model_fixture();
            m.objects.index_failed = true;
            m.workloads.index_failed = true;
            m.connection = Connection::Disconnected {
                error: "authentication failed (HTTP 401)".into(),
            };
            update(&mut m, key(screen));
            assert_eq!(update(&mut m, key('r')), vec![Cmd::Connect]);
            assert!(matches!(m.connection, Connection::Connecting));
            assert!(!m.objects.index_failed && !m.workloads.index_failed);
            assert!(
                !m.status.text.contains("still loading"),
                "{}",
                m.status.text
            );
            assert!(
                update(&mut m, key('r')).is_empty(),
                "one attempt while a connect is in flight"
            );
        }
    }

    #[test]
    fn disconnected_bodies_show_the_error_not_loading() {
        use crate::tui::test_support::render_to_string;
        for screen in [Screen::Objects, Screen::Workloads] {
            let mut m = model_fixture();
            m.connection = Connection::Disconnected {
                error: "connection refused".into(),
            };
            m.screen = screen;
            let out = render_to_string(&m, 80, 24);
            assert!(out.contains("connection refused"), "{out}");
            assert!(!out.contains("loading"), "{out}");
        }
    }

    #[test]
    fn policy_r_while_loading_is_a_noop_that_keeps_the_deny_bypass_unspent() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        assert_eq!(update(&mut m, key('r')).len(), 1);
        assert!(m.policy.loading && m.policy.bypass_deny);
        assert!(update(&mut m, key('r')).is_empty());
        assert_eq!(m.status.text, "already loading");
        assert!(m.policy.bypass_deny);
    }

    #[test]
    fn declining_the_cost_guard_after_t_stays_on_workloads() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        m.traffic.filter.window = "30d".into();
        update(&mut m, key('4'));
        let before = m.traffic.filter.clone();
        let status = m.status.clone();
        assert!(update(&mut m, key('t')).is_empty());
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        assert_eq!(
            m.screen,
            Screen::Workloads,
            "no switch before the query starts"
        );
        assert_eq!(m.status, status);
        update(&mut m, key('n'));
        assert_eq!(m.screen, Screen::Workloads);
        assert_eq!(m.traffic.filter, before);
        assert!(!m.traffic.is_running());
        // Confirming switches and starts the query.
        update(&mut m, key('t'));
        let cmds = update(&mut m, key('y'));
        assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
        assert_eq!(m.screen, Screen::Traffic);
        assert_eq!(m.traffic.filter.src, "wl:host-1.example");
    }

    #[test]
    fn policy_reloads_after_a_profile_switch_once_the_index_is_back() {
        use crate::tui::commands::Command;
        use crate::tui::model::{Limits, ProfileInfo};
        use crate::tui::test_support::{fixture_index, model_with_policy, render_to_string};
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        assert_eq!(
            update(&mut m, Msg::Command(Command::Profile("other".into()))),
            vec![Cmd::SwitchProfile("other".into())]
        );
        let cmds = update(
            &mut m,
            Msg::ProfileSwitched(Ok((
                ProfileInfo {
                    name: "other".into(),
                    host: "o.example".into(),
                    org_id: 7,
                },
                Limits {
                    auto_refresh_min_seconds: 60,
                    traffic_max_results: 1000,
                    traffic_default_window: "24h".into(),
                },
            ))),
        );
        assert_eq!(cmds, vec![Cmd::Connect]);
        assert_eq!(m.screen, Screen::Policy);
        assert!(!m.policy.loading && m.policy.loaded_at.is_none());
        let out = render_to_string(&m, 80, 24);
        assert!(out.contains("connecting"), "{out}");
        assert!(!out.contains("ruleset-"), "no stale rows: {out}");
        assert_eq!(
            update(&mut m, Msg::Connected),
            vec![Cmd::LoadIndex(CacheMode::Use)]
        );
        let cmds = update(
            &mut m,
            Msg::IndexLoaded {
                index: fixture_index(),
                age_secs: 0,
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use
            }]
        );
        assert!(m.policy.loading);
        let out = render_to_string(&m, 80, 24);
        assert!(out.contains("loading…"), "{out}");
        // A second index event does not double-load.
        let again = update(
            &mut m,
            Msg::IndexLoaded {
                index: fixture_index(),
                age_secs: 0,
            },
        );
        assert!(again.is_empty(), "{again:?}");
    }

    fn mouse(kind: crossterm::event::MouseEventKind, column: u16, row: u16) -> Msg {
        Msg::Mouse(crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn mouse_wheel_scrolls_and_tab_click_switches() {
        use crate::tui::test_support::model_with_flows;
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut m = model_with_flows();
        assert!(update(&mut m, Msg::Resize(80, 24)).is_empty());
        assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
        assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
        assert_eq!(m.traffic.flows.selected_pos(), 2);
        assert!(update(&mut m, mouse(MouseEventKind::ScrollUp, 10, 10)).is_empty());
        assert_eq!(m.traffic.flows.selected_pos(), 1);
        let x = crate::tui::ui::tab_x(Screen::Policy);
        update(&mut m, mouse(MouseEventKind::Down(MouseButton::Left), x, 1));
        assert_eq!(m.screen, Screen::Policy);
    }

    #[test]
    fn wheel_selected_ruleset_notes_missing_deny_rules_until_a_key_loads_them() {
        use crate::tui::test_support::{model_with_policy, render_to_string};
        use crossterm::event::MouseEventKind;
        let href = "/orgs/5/sec_policy/draft/rule_sets/101".to_string();
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(&mut m, Msg::Resize(80, 24));
        assert!(!render_to_string(&m, 80, 24).contains("deny rules not loaded"));
        assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
        assert_eq!(m.policy.selected_href().as_deref(), Some(href.as_str()));
        assert!(render_to_string(&m, 80, 24).contains("deny rules not loaded"));
        // A key move away (Up) and back (Down) loads them through the existing path, once.
        let up = update(&mut m, kc(KeyCode::Up));
        let down = update(&mut m, kc(KeyCode::Down));
        let loads: Vec<_> = up.iter().chain(down.iter()).collect();
        assert!(
            loads.contains(&&Cmd::LoadDenyRules {
                href: href.clone(),
                mode: pce::CacheMode::Use
            }),
            "{loads:?}"
        );
        assert!(!render_to_string(&m, 80, 24).contains("deny rules not loaded"));
        update(
            &mut m,
            Msg::DenyRulesLoaded {
                href,
                result: Ok(Vec::new()),
            },
        );
        assert!(!render_to_string(&m, 80, 24).contains("deny rules not loaded"));
        // Enter on the ruleset row is the other way in.
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10));
        assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
        assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
    }

    #[test]
    fn mouse_ignores_everything_else_and_overlays() {
        use crate::tui::test_support::model_with_flows;
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut m = model_with_flows();
        update(&mut m, Msg::Resize(80, 24));
        let x = crate::tui::ui::tab_x(Screen::Policy);
        for kind in [
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Moved,
            MouseEventKind::Drag(MouseButton::Left),
        ] {
            assert!(update(&mut m, mouse(kind, x, 1)).is_empty());
        }
        // Clicks off the tabs row, past the last tab, and on a divider do nothing.
        let left = MouseEventKind::Down(MouseButton::Left);
        assert!(update(&mut m, mouse(left, x, 5)).is_empty());
        assert!(update(&mut m, mouse(left, 79, 1)).is_empty());
        let divider = crate::tui::ui::tab_x(Screen::Policy) - 2;
        assert!(update(&mut m, mouse(left, divider, 1)).is_empty());
        assert_eq!(m.screen, Screen::Traffic);
        assert_eq!(m.traffic.flows.selected_pos(), 0);
        // Overlays swallow wheel and clicks.
        m.overlay = Overlay::Help;
        update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10));
        update(&mut m, mouse(left, x, 1));
        assert_eq!(m.traffic.flows.selected_pos(), 0);
        assert_eq!(m.screen, Screen::Traffic);
    }

    #[test]
    fn mouse_wheel_follows_policy_pane_aggregate_view_and_search() {
        use crate::tui::test_support::{model_with_flows, model_with_policy, model_with_workloads};
        use crossterm::event::MouseEventKind;
        let down = || mouse(MouseEventKind::ScrollDown, 10, 10);
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(&mut m, Msg::Resize(160, 48));
        assert!(
            update(&mut m, down()).is_empty(),
            "the wheel never requests"
        );
        assert_eq!(m.policy.rulesets.selected_pos(), 1);
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        assert_eq!(m.policy.pane, PolicyPane::Rules);
        let before = m.policy.rulesets.selected_pos();
        update(&mut m, down());
        assert_eq!(m.policy.rulesets.selected_pos(), before);

        let mut m = model_with_flows();
        update(&mut m, Msg::Resize(80, 24));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        update(&mut m, down());
        assert_eq!(m.traffic.groups.selected_pos(), 1);
        assert_eq!(m.traffic.flows.selected_pos(), 0);
        // An active search keeps routing the wheel to the table.
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
        );
        update(&mut m, down());
        assert_eq!(
            m.traffic.groups.selected_pos(),
            2.min(m.traffic.groups.len() - 1)
        );

        // Empty table: no panic, no request.
        let mut m = model_with_workloads();
        m.screen = Screen::Workloads;
        m.workloads.table.set_rows(Vec::new());
        assert!(update(&mut m, down()).is_empty());
    }

    fn enter() -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    }

    #[test]
    fn opening_events_loads_once_and_editor_reloads() {
        use crate::tui::model::Overlay;
        use crate::tui::msg::Cmd;
        let mut m = model_fixture();
        let cmds = update(&mut m, key('5'));
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], Cmd::LoadEvents { mode: pce::CacheMode::Use, filter } if filter.severity.is_none() && filter.since.is_some())
        );
        assert_eq!(m.screen, Screen::Events);
        let events: Vec<pce::models::Event> =
            serde_json::from_str(include_str!("../../../../fixtures/events.json")).unwrap();
        update(
            &mut m,
            Msg::EventsLoaded(Ok((
                events,
                0,
                ("2026-10-09T08:00:00Z".into(), "2026-10-10T08:00:00Z".into()),
            ))),
        );
        assert_eq!(m.events.table.len(), 4);
        assert!(update(&mut m, key('1')).is_empty());
        assert!(update(&mut m, key('5')).is_empty(), "already loaded");
        update(&mut m, key('f'));
        assert!(matches!(m.overlay, Overlay::EventsEditor { .. }));
        for c in "err".chars() {
            update(&mut m, key(c));
        }
        let cmds = update(&mut m, enter());
        assert!(
            matches!(&cmds[0], Cmd::LoadEvents { filter, .. } if filter.severity.as_deref() == Some("err"))
        );
        assert!(matches!(m.overlay, Overlay::None));
        update(&mut m, key('f'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "soon".chars() {
            update(&mut m, key(c));
        }
        update(&mut m, enter());
        assert!(
            matches!(&m.overlay, Overlay::EventsEditor { error: Some(e), .. } if e.contains("window"))
        );
    }

    #[test]
    fn events_type_filter_detail_and_severity_sort_are_local() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_events;
        let mut m = model_with_events();
        assert!(update(&mut m, key('5')).is_empty());
        assert_eq!(m.events.table.len(), 4);
        m.events.filter.event_type = "agent.".into();
        m.events.rebuild();
        assert_eq!(m.events.table.len(), 1);
        m.events.filter.event_type.clear();
        m.events.rebuild();
        assert!(update(&mut m, key('S')).is_empty());
        assert_eq!(
            m.events
                .table
                .selected_item()
                .unwrap()
                .event_type
                .as_deref(),
            Some("agent.tampering"),
            "worst severity first"
        );
        assert!(update(&mut m, enter()).is_empty());
        assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 0, .. }));
        // The detail scrolls line by line and stops at the top and the last line.
        let total = event_detail_lines(m.events.table.selected_item().unwrap());
        assert!(total > 2, "{total}");
        assert!(update(&mut m, key('j')).is_empty());
        assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 1, .. }));
        assert!(update(&mut m, kc(KeyCode::End)).is_empty());
        assert!(matches!(&m.overlay, Overlay::EventDetail { scroll, .. } if *scroll == total - 1));
        assert!(update(&mut m, key('j')).is_empty());
        assert!(matches!(&m.overlay, Overlay::EventDetail { scroll, .. } if *scroll == total - 1));
        assert!(update(&mut m, key('g')).is_empty());
        assert!(update(&mut m, key('k')).is_empty());
        assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 0, .. }));
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(
            crate::tui::commands::parse("events").unwrap(),
            crate::tui::commands::Command::Screen(Screen::Events)
        );
    }

    #[test]
    fn events_invalid_window_sends_nothing_and_enter_needs_a_selection() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_events;
        let mut m = model_with_events();
        m.screen = Screen::Events;
        update(&mut m, key('f'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
        );
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        assert!(
            update(&mut m, enter()).is_empty(),
            "invalid window: no request"
        );
        assert!(!m.events.loading);
        assert!(matches!(
            &m.overlay,
            Overlay::EventsEditor { error: Some(_), .. }
        ));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert_eq!(
            m.events.filter.window, "24h",
            "a rejected edit is not stored"
        );
        m.events.filter.event_type = "no.such.type".into();
        m.events.rebuild();
        assert!(update(&mut m, enter()).is_empty());
        assert!(
            matches!(m.overlay, Overlay::None),
            "nothing selected, no detail"
        );
    }

    #[test]
    fn events_r_bypasses_once_and_not_while_loading() {
        use crate::tui::test_support::model_with_events;
        let mut m = model_with_events();
        m.screen = Screen::Events;
        let cmds = update(&mut m, key('r'));
        assert!(matches!(
            &cmds[..],
            [Cmd::LoadEvents {
                mode: pce::CacheMode::Bypass,
                ..
            }]
        ));
        assert!(m.events.loading);
        assert!(update(&mut m, key('r')).is_empty(), "already loading");
    }

    #[test]
    fn events_auto_refresh_obeys_the_guards_and_trips_on_failure() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_events;
        use std::time::Duration;
        let mut m = model_with_events();
        update(&mut m, key('5'));
        let now = Instant::now() + Duration::from_secs(120);
        m.events.auto.last_run = Some(Instant::now());
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
        update(&mut m, key('a'));
        assert!(m.events.auto.enabled);
        assert!(m.any_auto_enabled());
        assert!(update(&mut m, Msg::Tick(Instant::now() + Duration::from_secs(10))).is_empty());
        m.focused = false;
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
        m.focused = true;
        m.overlay = Overlay::Help;
        assert!(update(&mut m, Msg::Tick(now)).is_empty(), "no overlay");
        m.overlay = Overlay::None;
        m.events.loading = true;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "not while loading"
        );
        m.events.loading = false;
        let cmds = update(&mut m, Msg::Tick(now));
        assert!(matches!(
            &cmds[..],
            [Cmd::LoadEvents {
                mode: pce::CacheMode::Bypass,
                ..
            }]
        ));
        assert!(m.events.auto.from_tick);
        update(&mut m, Msg::EventsLoaded(Err("boom".into())));
        assert!(!m.events.auto.enabled && m.events.auto.tripped);
        assert!(
            m.status.text.contains("auto-refresh off"),
            "{}",
            m.status.text
        );
        assert!(!m.events.loading);
        m.events.auto.enabled = true;
        m.events.loading = false;
        m.events.auto.last_run = Some(Instant::now());
        m.screen = Screen::Traffic;
        assert!(
            update(&mut m, Msg::Tick(now)).is_empty(),
            "only its own screen"
        );
    }

    #[test]
    fn events_mouse_wheel_scrolls_without_requests_and_reset_clears_state() {
        use crate::tui::test_support::model_with_events;
        use crossterm::event::MouseEventKind;
        let mut m = model_with_events();
        m.screen = Screen::Events;
        assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
        assert_eq!(m.events.table.selected_pos(), 1);
        m.events.auto.enabled = true;
        m.events.filter.window = "7d".into();
        m.reset_data();
        assert!(m.events.table.is_empty() && m.events.raw.is_empty());
        assert!(m.events.loaded_at.is_none() && !m.events.auto.enabled);
        assert_eq!(m.events.filter.window, "24h");
    }

    #[test]
    fn events_open_while_connecting_loads_when_connected() {
        let mut m = model_fixture();
        m.connection = Connection::Connecting;
        assert!(update(&mut m, key('5')).is_empty(), "not connected yet");
        let cmds = update(&mut m, Msg::Connected);
        assert!(cmds.iter().any(|c| matches!(
            c,
            Cmd::LoadEvents {
                mode: pce::CacheMode::Use,
                ..
            }
        )));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::LoadIndex(_))));
    }

    fn query_done_with(m: &mut Model, filter: TrafficFilter) {
        let seq = m.traffic.seq;
        update(
            m,
            Msg::QueryDone {
                seq,
                result: Ok(QueryOutcome {
                    flows: crate::tui::test_support::fixture_flows(),
                    truncated: false,
                    matches_count: Some(4),
                    elapsed: std::time::Duration::from_secs(1),
                    hit: false,
                    filter,
                }),
            },
        );
    }

    fn is_bypass_run(cmds: &[Cmd]) -> bool {
        matches!(
            cmds,
            [Cmd::RunQuery {
                mode: CacheMode::Bypass,
                ..
            }]
        )
    }

    #[test]
    fn r_honours_the_cost_guard_unless_the_filter_produced_the_data() {
        use crate::tui::model::Limits;
        let mut m = model_fixture();
        // Profile defaults above the confirmation cap: nothing confirmed them yet.
        m.limits = Limits {
            traffic_max_results: 20_000,
            ..m.limits.clone()
        };
        m.reset_data();
        assert_eq!(m.traffic.filter.max_results, 20_000);
        let cmds = update(&mut m, key('r'));
        assert!(cmds.is_empty(), "{cmds:?}");
        assert!(
            matches!(&m.overlay, Overlay::Confirm { reason, action: PendingAction::RunNow(_) } if reason.contains("20000"))
        );
        // Declining sends nothing.
        assert!(update(&mut m, key('n')).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        update(&mut m, key('r'));
        let cmds = update(&mut m, key('y'));
        assert!(is_bypass_run(&cmds), "{cmds:?}");
        assert!(m.traffic.is_running());
        let f = m.traffic.filter.clone();
        query_done_with(&mut m, f);
        // The same filter produced the data on screen: `r` reruns it directly.
        let cmds = update(&mut m, key('r'));
        assert!(is_bypass_run(&cmds), "{cmds:?}");
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn r_with_a_long_default_window_asks_first() {
        let mut m = model_fixture();
        m.traffic.filter = TrafficFilter::defaults("30d", 1000);
        assert!(update(&mut m, key('r')).is_empty());
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        assert!(is_bypass_run(&update(&mut m, key('y'))));
    }

    #[test]
    fn an_identical_filter_does_not_restart_the_running_query() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        assert_eq!(update(&mut m, Msg::ApplyFilter(f.clone())).len(), 1);
        let seq = m.traffic.seq;
        let again = update(&mut m, Msg::ApplyFilter(f));
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(m.status.text, "query already running");
        assert_eq!(m.traffic.seq, seq);
        assert!(m.traffic.is_running());
        // Key auto-repeat on `t` from Workloads: one query, not one per repeat.
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
        assert_eq!(update(&mut m, key('t')).len(), 1);
        update(&mut m, key('4'));
        assert!(update(&mut m, key('t')).is_empty());
        assert_eq!(m.screen, Screen::Traffic);
    }

    #[test]
    fn jump_uses_the_href_id_when_the_name_is_not_unique() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        m.workloads.raw[1].hostname = m.workloads.raw[0].hostname.clone();
        m.workloads.rebuild();
        update(&mut m, key('4'));
        update(&mut m, key('t'));
        assert_eq!(m.traffic.filter.src, "wl:aaaa-1");
        update(&mut m, key('4'));
        update(&mut m, key('j'));
        update(&mut m, key('t'));
        assert_eq!(m.traffic.filter.src, "wl:aaaa-2");
        // A unique name stays readable.
        update(&mut m, key('4'));
        update(&mut m, key('j'));
        update(&mut m, key('T'));
        assert_eq!(m.traffic.filter.dst, "wl:unmanaged-nas");
    }

    #[test]
    fn policy_d_and_p_do_not_double_a_load_in_flight() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        update(&mut m, key('2'));
        assert_eq!(update(&mut m, key('r')).len(), 1);
        assert!(m.policy.loading);
        assert!(update(&mut m, key('d')).is_empty());
        assert_eq!(m.status.text, "already loading");
        assert_eq!(m.policy.scope, PolicyScope::Draft);

        let mut m = model_with_policy();
        update(&mut m, key('2'));
        assert_eq!(
            update(&mut m, key('p')),
            vec![Cmd::LoadPending(CacheMode::Use)]
        );
        assert!(m.policy.pending_loading);
        assert!(update(&mut m, key('p')).is_empty());
        update(&mut m, Msg::PendingLoaded(Err("boom".into())));
        assert!(!m.policy.pending_loading, "cleared on Err");
        assert_eq!(update(&mut m, key('p')).len(), 1);
        update(&mut m, Msg::PendingLoaded(Ok(Vec::new())));
        assert!(!m.policy.pending_loading, "cleared on Ok");
        m.policy.pending_loading = true;
        m.policy.clear();
        assert!(!m.policy.pending_loading, "reset by clear()");
    }

    #[test]
    fn auto_refresh_ticks_only_while_connected() {
        use crate::tui::test_support::model_with_flows;
        use std::time::{Duration, Instant};
        let mut m = model_with_flows();
        update(&mut m, key('a'));
        let later = Instant::now() + Duration::from_secs(61);
        m.connection = Connection::Disconnected {
            error: "gone".into(),
        };
        assert!(update(&mut m, Msg::Tick(later)).is_empty());
        m.connection = Connection::Connecting;
        assert!(update(&mut m, Msg::Tick(later)).is_empty());
        m.connection = Connection::Connected {
            index: Some(crate::tui::test_support::fixture_index()),
            index_age_secs: 0,
        };
        assert!(is_bypass_run(&update(&mut m, Msg::Tick(later))));
    }

    #[test]
    fn draft_and_active_commands_reuse_the_loaded_scope() {
        use crate::tui::commands::Command;
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Traffic;
        let cmds = update(&mut m, Msg::Command(Command::Draft));
        assert!(cmds.is_empty(), "draft already loaded: {cmds:?}");
        assert_eq!(m.screen, Screen::Policy);
        assert!(!m.policy.rulesets.is_empty());
        assert_eq!(
            update(&mut m, Msg::Command(Command::Active)),
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Active,
                mode: CacheMode::Use
            }]
        );
        // In flight: neither the same nor the other scope requests again.
        assert!(update(&mut m, Msg::Command(Command::Active)).is_empty());
        assert!(update(&mut m, Msg::Command(Command::Draft)).is_empty());
        assert_eq!(m.status.text, "already loading");
        assert_eq!(m.policy.scope, PolicyScope::Active);
    }

    #[test]
    fn index_retry_from_either_screen_clears_both_flags() {
        let mut m = model_fixture();
        m.connection = Connection::Connected {
            index: None,
            index_age_secs: 0,
        };
        update(&mut m, Msg::IndexFailed("boom".into()));
        update(&mut m, key('3'));
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadIndex(CacheMode::Use)]
        );
        assert!(!m.objects.index_failed && !m.workloads.index_failed);
        update(&mut m, Msg::IndexFailed("boom".into()));
        update(&mut m, key('4'));
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadIndex(CacheMode::Use)]
        );
        assert!(!m.objects.index_failed && !m.workloads.index_failed);
    }

    #[test]
    fn a_form_overlay_takes_every_key_and_esc_sends_nothing() {
        use crate::tui::edit::EditIntent;
        use crate::tui::form::{Field, Form};
        let mut m = model_fixture();
        m.overlay = Overlay::Form(Form::new(
            "New label",
            vec![
                Field::new("Key", "", "app", true),
                Field::new("Value", "", "web", true),
            ],
            EditIntent::CreateLabel,
        ));
        for c in "q2?".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert!(!m.quit, "q is typed, not a quit");
        assert_eq!(m.screen, Screen::Traffic);
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.value("Key"), "q2?");
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

    // ---- Editing keys (Plan 4) ----

    fn writes_on(mut m: Model) -> Model {
        m.writes = crate::tui::model::WritesState::on();
        m
    }

    #[test]
    fn editing_keys_with_writes_off_only_say_why() {
        use crate::tui::test_support::{model_with_objects, model_with_workloads};
        let mut m = model_with_objects();
        m.screen = Screen::Objects;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = model_with_workloads();
        m.screen = Screen::Workloads;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
    }

    #[test]
    fn label_groups_and_managed_workloads_are_refused_before_any_form() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::{model_with_objects, model_with_workloads};
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::LabelGroups;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "label groups are read-only in this version");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_workloads());
        m.screen = Screen::Workloads;
        // Row 0 is host-1.example, VEN-managed.
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "only unmanaged workloads can be edited here");
            assert_eq!(m.overlay, Overlay::None);
        }
        update(&mut m, key('j'));
        update(&mut m, key('j')); // unmanaged-nas
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteWorkload {
                href: "/orgs/5/workloads/aaaa-3".into(),
                name: "unmanaged-nas".into(),
            })],
            "a delete plans at once, the confirm overlay comes next"
        );
        // Editing waits for the plan's answer.
        update(&mut m, Msg::Planned(Err("not found".into())));
        assert!(update(&mut m, key('e')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "Edit workload unmanaged-nas"),
            "{:?}",
            m.overlay
        );
    }

    #[test]
    fn e_opens_a_prefilled_form_and_enter_plans_only_the_change() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::IpLists;
        update(&mut m, key('j')); // Corp
        assert!(update(&mut m, key('e')).is_empty());
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.value("Ranges"), "10.1.0.0-10.1.255.255");
        // Unchanged: the form closes, nothing is planned.
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.status.text, "no changes");
        assert_eq!(m.overlay, Overlay::None);
        update(&mut m, key('e'));
        update(&mut m, kc(KeyCode::Tab));
        update(&mut m, kc(KeyCode::Tab));
        for c in ", 10.2.0.0/16".chars() {
            update(&mut m, key(c));
        }
        let cmds = update(&mut m, enter());
        assert_eq!(
            cmds,
            vec![Cmd::PlanWrite(WriteIntent::UpdateIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                patch: pce::write::IpListPatch {
                    add_ranges: vec![pce::write::IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
                    ..Default::default()
                },
            })]
        );
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "planning…");
    }

    #[test]
    fn n_opens_an_empty_form_that_validates_and_esc_cancels() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::Services;
        assert!(update(&mut m, key('n')).is_empty());
        assert!(update(&mut m, enter()).is_empty());
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.error.as_deref(), Some("Name is required"));
        assert_eq!(f.active, 0);
        for c in "SSH".chars() {
            update(&mut m, key(c));
        }
        update(&mut m, kc(KeyCode::Tab));
        update(&mut m, kc(KeyCode::Tab));
        for c in "22".chars() {
            update(&mut m, key(c));
        }
        assert!(
            update(&mut m, enter()).is_empty(),
            "a bare port is refused locally"
        );
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert!(
            f.error.as_deref().unwrap().contains("22/tcp"),
            "{:?}",
            f.error
        );
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

    #[test]
    fn help_scrolls_to_its_last_line_and_reopens_at_the_top() {
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 25);
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 3, "25 lines, 22 rows inside the box");
        for _ in 0..40 {
            update(&mut m, key('j'));
        }
        assert_eq!(m.help_scroll, last);
        update(&mut m, key('k'));
        assert_eq!(m.help_scroll, last - 1);
        update(&mut m, key('G'));
        assert_eq!(m.help_scroll, last);
        update(&mut m, key('g'));
        assert_eq!(m.help_scroll, 0);
        update(&mut m, key('G'));
        update(&mut m, kc(KeyCode::Esc));
        update(&mut m, key('?'));
        assert_eq!(m.help_scroll, 0, "the help reopens at the top");
    }

    #[test]
    fn planned_opens_the_confirm_overlay_and_y_sends_once() {
        use crate::tui::test_support::label_post_plan;
        let mut m = writes_on(model_fixture());
        let planned = Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label)));
        assert!(update(&mut m, planned).is_empty());
        assert!(matches!(
            &m.overlay,
            Overlay::ConfirmWrite { scroll: 0, .. }
        ));
        // The body is four lines: `{`, key, value, `}`; at 80x24 all show.
        update(&mut m, key('j'));
        assert!(
            matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 0, .. }),
            "nothing to scroll: {:?}",
            m.overlay
        );
        // Eight rows leave three for the body: the last page starts at line 1.
        m.size = (80, 8);
        for _ in 0..10 {
            update(&mut m, key('j'));
        }
        assert!(
            matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 1, .. }),
            "stops where the last line is drawn: {:?}",
            m.overlay
        );
        update(&mut m, key('k'));
        assert!(matches!(
            &m.overlay,
            Overlay::ConfirmWrite { scroll: 0, .. }
        ));
        assert!(update(&mut m, key('q')).is_empty());
        assert!(!m.quit, "q does not quit from the confirm overlay");
        let cmds = update(&mut m, key('y'));
        assert_eq!(
            cmds,
            vec![Cmd::ExecuteWrite {
                plan: label_post_plan(),
                kind: pce::write::ObjectKind::Label
            }]
        );
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "sending …");
        assert!(
            update(&mut m, key('y')).is_empty(),
            "a second y has no overlay to answer"
        );
    }

    #[test]
    fn n_or_esc_cancels_and_a_failed_plan_reports() {
        use crate::tui::test_support::ip_list_delete_plan;
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut m = writes_on(model_fixture());
            update(
                &mut m,
                Msg::Planned(Ok((ip_list_delete_plan(), pce::write::ObjectKind::IpList))),
            );
            assert!(update(&mut m, kc(code)).is_empty());
            assert_eq!(m.overlay, Overlay::None);
            assert_eq!(m.status.text, "cancelled, nothing sent");
        }
        let mut m = writes_on(model_fixture());
        update(
            &mut m,
            Msg::Planned(Err("not found: /orgs/5/sec_policy/draft/ip_lists/9".into())),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(
            m.status.text,
            "write: not found: /orgs/5/sec_policy/draft/ip_lists/9"
        );
        assert_eq!(
            m.overlay,
            Overlay::None,
            "short errors stay in the status line"
        );
        let long = "range 10.9.0.0/16 is not in Corp; an IP list needs at least one range or FQDN";
        update(&mut m, Msg::Planned(Err(long.into())));
        assert_eq!(m.overlay, Overlay::Error(format!("write: {long}")));
        let mut m = writes_on(model_fixture());
        let noop = pce::write::Plan::Noop {
            existing_href: "/orgs/5/labels/1".into(),
            reason: "label role=web exists".into(),
        };
        update(
            &mut m,
            Msg::Planned(Ok((noop, pce::write::ObjectKind::Label))),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(
            m.status
                .text
                .contains("nothing to send (label role=web exists)"),
            "{}",
            m.status.text
        );
        assert_eq!(m.overlay, Overlay::None);
    }

    fn written(href: &str, pending: bool) -> pce::write::Written {
        pce::write::Written {
            href: href.into(),
            pending,
            status: 201,
            object: serde_json::json!({ "href": href }),
            dropped_fields: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn write_done_reports_reloads_the_kind_once_and_forgets_pending() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        use pce::write::Outcome;
        let mut m = model_with_objects();
        m.policy.pending = Some(vec![]);
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::IpList,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/ip_lists/9",
                    true,
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::IpLists,
                mode: pce::CacheMode::Bypass
            }]
        );
        assert_eq!(
            m.status.text,
            "created /orgs/5/sec_policy/draft/ip_lists/9 (pending)"
        );
        assert_eq!(m.status.kind, StatusKind::Info);
        assert!(m.policy.pending.is_none(), "the next p fetches again");
        let mut w = written("/orgs/5/labels/9", false);
        w.warnings = vec!["audit entry could not be written to x: denied".into()];
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Label,
                method: Some(pce::WriteMethod::Put),
                result: Ok(Outcome::Written(w)),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Labels,
                mode: pce::CacheMode::Bypass
            }]
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(
            m.status.text,
            "updated /orgs/5/labels/9; audit entry could not be written to x: denied"
        );
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Workload,
                method: Some(pce::WriteMethod::Delete),
                result: Ok(Outcome::Written(written("/orgs/5/workloads/aaaa-3", false))),
            },
        );
        assert_eq!(cmds, vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]);
        assert_eq!(m.status.text, "deleted /orgs/5/workloads/aaaa-3");
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Label,
                method: None,
                result: Ok(Outcome::Skipped {
                    existing_href: "/orgs/5/labels/1".into(),
                    reason: "label role=web exists".into(),
                }),
            },
        );
        assert!(cmds.is_empty());
        assert_eq!(m.status.text, "skipped: label role=web exists");
    }

    #[test]
    fn write_errors_open_the_overlay_and_only_unanswered_reloads() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        use pce::PceError;
        use pce::write::WriteError;
        let mut m = model_with_objects();
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Service,
                method: Some(pce::WriteMethod::Delete),
                result: Err(WriteError::Pce(PceError::Rejected {
                    status: 406,
                    body: "object_in_use: referenced by a rule".into(),
                })),
            },
        );
        assert!(cmds.is_empty(), "a rejected write reloads nothing");
        match &m.overlay {
            Overlay::Error(t) => assert!(
                t.starts_with("write failed: ")
                    && t.contains("object_in_use: referenced by a rule"),
                "{t}"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(m.status.kind, StatusKind::Error);
        m.overlay = Overlay::None;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Service,
                method: Some(pce::WriteMethod::Put),
                result: Err(WriteError::Unanswered(PceError::Timeout(
                    std::time::Duration::from_secs(30),
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Services,
                mode: pce::CacheMode::Bypass
            }],
            "the user sees the real state"
        );
        match &m.overlay {
            Overlay::Error(t) => {
                assert!(
                    t.starts_with("the request was sent but not answered: "),
                    "{t}"
                );
                assert_eq!(
                    t.matches("check the PCE before retrying").count(),
                    1,
                    "the advice reads once: {t}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unanswered_write_whose_audit_failed_still_reloads_and_names_the_audit_log() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        use pce::PceError;
        use pce::write::WriteError;
        let mut m = model_with_objects();
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::IpList,
                method: Some(pce::WriteMethod::Put),
                result: Err(WriteError::AuditFailed {
                    write: Box::new(WriteError::Unanswered(PceError::Timeout(
                        std::time::Duration::from_secs(30),
                    ))),
                    error: "/home/a/.local/state/illumio-tui/audit.jsonl: permission denied".into(),
                }),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::IpLists,
                mode: pce::CacheMode::Bypass
            }],
            "the write may have applied: one reload shows the real state"
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(
            m.status.text,
            "write not answered and not audited; reloading"
        );
        match &m.overlay {
            Overlay::Error(t) => {
                assert!(
                    t.starts_with("the request was sent but not answered: "),
                    "{t}"
                );
                assert!(
                    t.contains("/home/a/.local/state/illumio-tui/audit.jsonl: permission denied"),
                    "{t}"
                );
                assert_eq!(t.matches("check the PCE before retrying").count(), 1, "{t}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pending_list_selects_with_j_k_and_enter_asks_for_one_diff() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        let mut pending = pce::models::parse_pending(
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
        );
        pending.push(pce::models::PendingChange {
            href: "/orgs/5/sec_policy/draft/label_groups/7".into(),
            change_type: "update".into(),
            name: Some("G".into()),
            object_type: "label_groups".into(),
        });
        update(&mut m, Msg::PendingLoaded(Ok(pending.clone())));
        assert_eq!(m.policy.pending_selected, 0);
        assert_eq!(
            update(&mut m, enter()),
            vec![Cmd::LoadDiff {
                change: pending[0].clone(),
                kind: pce::write::ObjectKind::RuleSet
            }],
            "entry 0 is a ruleset"
        );
        m.policy.diff_loading = false;
        assert_eq!(m.overlay, Overlay::Pending);
        update(&mut m, key('j')); // Corp
        let cmds = update(&mut m, enter());
        assert_eq!(
            cmds,
            vec![Cmd::LoadDiff {
                change: pending[1].clone(),
                kind: pce::write::ObjectKind::IpList
            }]
        );
        assert!(m.policy.diff_loading);
        assert!(
            update(&mut m, enter()).is_empty(),
            "one diff in flight at a time"
        );
        assert_eq!(m.status.text, "diff already loading");
        m.policy.diff_loading = false;
        update(&mut m, key('j'));
        update(&mut m, key('j'));
        assert_eq!(m.policy.pending_selected, 2, "stops at the last entry");
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.status.text, "no diff for label_groups");
        update(&mut m, key('k'));
        assert_eq!(m.policy.pending_selected, 1);
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn diff_loaded_opens_the_overlay_esc_returns_and_q_closes() {
        use crate::tui::diff::{DiffKind, DiffLine, ObjectDiff, Sign};
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.policy.pending = Some(vec![]);
        m.overlay = Overlay::Pending;
        m.policy.diff_loading = true;
        m.status = StatusLine::info("loading the active version of Corp");
        let diff = ObjectDiff {
            name: "Corp".into(),
            href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
            change: DiffKind::Created,
            lines: vec![
                DiffLine {
                    sign: Sign::Added,
                    text: "name: Corp".into(),
                },
                DiffLine {
                    sign: Sign::Added,
                    text: "range 10.1.0.0-10.1.255.255".into(),
                },
            ],
        };
        assert!(update(&mut m, Msg::DiffLoaded(Ok(diff.clone()))).is_empty());
        assert!(!m.policy.diff_loading);
        assert_eq!(m.status.kind, StatusKind::Info);
        assert_eq!(
            m.status.text, "diff of Corp",
            "the loading status is replaced"
        );
        assert_eq!(
            m.overlay,
            Overlay::Diff {
                diff: diff.clone(),
                scroll: 0
            }
        );
        update(&mut m, key('j'));
        update(&mut m, key('j'));
        assert_eq!(
            m.overlay,
            Overlay::Diff {
                diff: diff.clone(),
                scroll: 1
            },
            "stops at the last line"
        );
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::Pending, "Esc returns to the list");
        update(&mut m, Msg::DiffLoaded(Ok(diff)));
        update(&mut m, key('q'));
        assert_eq!(m.overlay, Overlay::None, "q closes both");
        m.overlay = Overlay::Pending;
        m.status = StatusLine::info("loading the active version of Corp");
        update(
            &mut m,
            Msg::DiffLoaded(Err("not found: /orgs/5/sec_policy/active/ip_lists".into())),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(
            m.status.text,
            "diff: not found: /orgs/5/sec_policy/active/ip_lists"
        );
        assert_eq!(m.overlay, Overlay::Pending);
    }

    #[test]
    fn a_write_in_flight_refuses_edit_keys_and_profile_switches() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::{ip_list_delete_plan, model_with_objects};
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::Services;
        let cmds = update(&mut m, key('x'));
        assert!(matches!(cmds.as_slice(), [Cmd::PlanWrite(_)]), "{cmds:?}");
        assert!(m.writes.in_flight, "a plan is in flight");
        let refused = |m: &mut Model, msg: Msg, what: &str| {
            assert!(update(m, msg).is_empty(), "{what}");
            assert_eq!(m.status.kind, StatusKind::Warn, "{what}");
            assert_eq!(m.status.text, "a write is already in progress", "{what}");
            assert_eq!(m.overlay, Overlay::None, "{what}");
        };
        for c in ['n', 'e', 'x'] {
            refused(&mut m, key(c), "objects");
        }
        m.screen = Screen::Workloads;
        for c in ['n', 'e', 'x'] {
            refused(&mut m, key(c), "workloads");
        }
        m.screen = Screen::Objects;
        assert!(update(&mut m, Msg::Command(Command::Profile("b".into()))).is_empty());
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, "finish or cancel the write first");
        assert!(!m.switching);
        // The plan lands: the confirm opens and nothing is in flight.
        let planned = Msg::Planned(Ok((ip_list_delete_plan(), pce::write::ObjectKind::IpList)));
        update(&mut m, planned);
        assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
        assert!(!m.writes.in_flight);
        // `y` sends: in flight again until the write is done.
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::ExecuteWrite { .. }]),
            "{cmds:?}"
        );
        assert!(m.writes.in_flight);
        refused(&mut m, key('x'), "while sending");
        update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::IpList,
                method: None,
                result: Ok(pce::write::Outcome::Skipped {
                    existing_href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                    reason: "gone".into(),
                }),
            },
        );
        assert!(!m.writes.in_flight);
        assert_eq!(
            update(&mut m, Msg::Command(Command::Profile("b".into()))),
            vec![Cmd::SwitchProfile("b".into())]
        );
        m.writes.in_flight = true;
        m.reset_data();
        assert!(!m.writes.in_flight, "a profile switch forgets the write");
    }

    #[test]
    fn a_plan_landing_on_another_overlay_is_discarded() {
        use crate::tui::test_support::label_post_plan;
        let mut m = writes_on(model_fixture());
        m.writes.in_flight = true;
        // The user opened the prompt and typed `y` while the plan was on its way.
        m.overlay = Overlay::Prompt { input: "y".into() };
        let planned = Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label)));
        assert!(update(&mut m, planned).is_empty());
        assert_eq!(m.overlay, Overlay::Prompt { input: "y".into() });
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, "request discarded, nothing sent");
        assert!(!m.writes.in_flight);
    }

    #[test]
    fn a_created_label_joins_the_name_index_when_the_labels_reload() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = model_with_objects();
        assert_eq!(m.index().unwrap().label_href("app", "web2"), None);
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Label,
                method: Some(pce::WriteMethod::Post),
                result: Ok(pce::write::Outcome::Written(written(
                    "/orgs/5/labels/9",
                    false,
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Labels,
                mode: pce::CacheMode::Bypass
            }],
            "the one reload the write already costs"
        );
        let mut labels: Vec<pce::models::Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        labels.push(
            serde_json::from_value(serde_json::json!({
                "href": "/orgs/5/labels/9", "key": "app", "value": "web2"
            }))
            .unwrap(),
        );
        let cmds = update(
            &mut m,
            Msg::ObjectsLoaded {
                kind: ObjectKind::Labels,
                result: Ok(ObjectsPayload::Labels(labels, 0)),
            },
        );
        assert!(cmds.is_empty(), "the index is rebuilt without a request");
        let idx = m.index().expect("still connected with an index");
        assert_eq!(idx.label_href("app", "web2"), Some("/orgs/5/labels/9"));
        assert_eq!(
            idx.ip_list_name("/orgs/5/sec_policy/draft/ip_lists/21"),
            Some("Corp"),
            "IP lists are kept"
        );
        assert_eq!(
            idx.service_name("/orgs/5/sec_policy/draft/services/31"),
            Some("PostgreSQL"),
            "services are kept"
        );
        // A form opened now resolves the new label.
        assert!(
            edit::build_intent(
                &{
                    let mut f = edit::form_for(EditIntent::CreateWorkload, Some(idx.as_ref()))
                        .expect("a form");
                    f.fields[0].value = "printer-1".into();
                    f.fields[4].value = "app=web2".into();
                    f
                },
                Some(idx.as_ref()),
                5
            )
            .is_ok_and(|i| i.is_some())
        );
    }

    #[test]
    fn confirm_scroll_counts_wrapped_lines() {
        let long = "w".repeat(150);
        let plan = pce::write::Plan::Send {
            request: pce::write::WriteRequest {
                method: pce::WriteMethod::Post,
                path: "/orgs/5/labels".into(),
                body: Some(serde_json::json!({"key": "app", "value": long})),
                summary: "create label".into(),
            },
            kind: pce::write::ObjectKind::Label,
        };
        let mut m = writes_on(model_fixture());
        update(
            &mut m,
            Msg::Planned(Ok((plan, pce::write::ObjectKind::Label))),
        );
        // 74 columns inside the box: the value line takes three rows, so six
        // body rows in all; nine terminal rows leave four.
        m.size = (80, 9);
        for _ in 0..10 {
            update(&mut m, key('j'));
        }
        assert!(
            matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 2, .. }),
            "{:?}",
            m.overlay
        );
    }

    #[test]
    fn ruleset_keys_open_forms_and_x_plans_the_delete() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_policy;
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        assert!(update(&mut m, key('n')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "New ruleset"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        assert!(update(&mut m, key('e')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "Edit ruleset Shop-Prod"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteRuleSet {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                name: "Shop-Prod".into(),
            })]
        );
        assert!(m.writes.in_flight);
        assert!(update(&mut m, key('e')).is_empty(), "one write at a time");
        assert_eq!(m.status.text, "a write is already in progress");
    }

    #[test]
    fn policy_keys_refuse_in_the_active_scope_and_without_a_selection() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.policy.scope = PolicyScope::Active;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.status.text, "switch to draft to edit policy");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_fixture());
        m.screen = Screen::Policy;
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "no row selected");
            assert_eq!(m.overlay, Overlay::None);
        }
    }

    #[test]
    fn a_policy_write_reloads_the_draft_rulesets_and_keeps_the_report() {
        use crate::tui::test_support::model_with_policy;
        use pce::write::Outcome;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::RuleSet,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/rule_sets/102",
                    true,
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: pce::CacheMode::Bypass
            }]
        );
        let report = "created /orgs/5/sec_policy/draft/rule_sets/102 (pending)";
        assert_eq!(m.status.text, report);
        let sets = m.policy.raw.clone();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets.clone(), 0)),
            },
        );
        assert_eq!(m.status.text, report, "the reload keeps the write's report");
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets, 0)),
            },
        );
        assert_eq!(
            m.status.text, "2 rulesets (draft)",
            "only that one reload is quiet"
        );
        m.policy.scope = PolicyScope::Active;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Delete),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                    true,
                ))),
            },
        );
        assert!(
            cmds.is_empty(),
            "a draft write does not change the active view"
        );
    }

    #[test]
    fn quiet_reload_does_not_outlive_a_scope_switch() {
        use crate::tui::test_support::model_with_policy;
        use pce::write::Outcome;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::RuleSet,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/rule_sets/102",
                    true,
                ))),
            },
        );
        assert!(m.policy.quiet_reload);
        m.policy.loading = false;
        update(&mut m, key('d')); // to active while the reload is in flight
        let sets = m.policy.raw.clone();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets.clone(), 0)),
            },
        );
        assert!(!m.policy.quiet_reload, "the ignored answer took the flag");
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Active,
                result: Ok((sets, 0)),
            },
        );
        assert!(
            m.status.text.ends_with("rulesets (active)"),
            "{}",
            m.status.text
        );
    }

    #[test]
    fn rule_keys_open_prefilled_forms_and_x_plans_by_kind() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_policy;
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!(m.policy.pane, PolicyPane::Rules);
        update(&mut m, key('n'));
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "New allow rule in ruleset 100"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        update(&mut m, key('N'));
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "New deny rule in ruleset 100"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        // Row 0 is the override deny rule 8 (evaluation order).
        update(&mut m, key('e'));
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.title, "Edit deny rule 8 in ruleset 100");
                assert_eq!(f.value("Override"), "y");
            }
            other => panic!("{other:?}"),
        }
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteDenyRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into()
            })]
        );
        m.writes.in_flight = false;
        update(&mut m, key('j')); // allow rule 1
        update(&mut m, key('e'));
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.title, "Edit allow rule 1 in ruleset 100");
                assert_eq!(f.value("Services"), "PostgreSQL");
            }
            other => panic!("{other:?}"),
        }
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into()
            })]
        );
    }

    #[test]
    fn rules_pane_keys_go_through_the_same_refusals() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(&mut m, kc(KeyCode::Tab));
        for c in ['n', 'N', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.policy.scope = PolicyScope::Active;
        update(&mut m, kc(KeyCode::Tab));
        for c in ['n', 'N', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "switch to draft to edit policy");
            assert_eq!(m.overlay, Overlay::None);
        }
        // `N` means nothing on the rulesets pane.
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.status = StatusLine::info("ready");
        assert!(update(&mut m, key('N')).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "ready");
        // No rule rows: e and x have nothing to act on.
        update(&mut m, kc(KeyCode::Tab));
        m.policy.rules.set_rows(Vec::new());
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "no row selected");
        }
    }

    fn pending_fixture() -> Vec<pce::models::PendingChange> {
        pce::models::parse_pending(
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
        )
    }

    #[test]
    fn space_marks_entries_and_p_plans_the_marked_or_all() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_policy;
        let provision = |subset| {
            vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
                description: Some("illumio-tui provision".into()),
                subset,
            }))]
        };
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
        assert_eq!(m.overlay, Overlay::Pending);
        update(&mut m, key(' '));
        assert!(
            m.policy
                .pending_marked
                .contains("/orgs/5/sec_policy/draft/rule_sets/101")
        );
        update(&mut m, key(' '));
        assert!(m.policy.pending_marked.is_empty(), "Space toggles");
        update(&mut m, key('j'));
        update(&mut m, key(' '));
        assert_eq!(
            update(&mut m, key('P')),
            provision(ProvisionSubset::Hrefs(vec![
                "/orgs/5/sec_policy/draft/ip_lists/21".into()
            ]))
        );
        assert_eq!(
            m.overlay,
            Overlay::None,
            "the confirm overlay opens on the plan's answer"
        );
        assert!(m.writes.in_flight);
        m.writes.in_flight = false;
        update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
        assert!(
            m.policy.pending_marked.is_empty(),
            "a reload clears the marks"
        );
        assert_eq!(
            update(&mut m, key('P')),
            provision(ProvisionSubset::All),
            "no marks: everything pending"
        );
        m.writes.in_flight = false;
        update(&mut m, Msg::PendingLoaded(Ok(vec![])));
        assert!(update(&mut m, key('P')).is_empty());
        assert_eq!(m.status.text, "nothing pending");
        let mut off = model_with_policy();
        off.screen = Screen::Policy;
        update(&mut off, Msg::PendingLoaded(Ok(pending_fixture())));
        assert!(update(&mut off, key('P')).is_empty());
        assert_eq!(off.status.text, "started without --allow-writes");
        assert_eq!(off.overlay, Overlay::Pending);
    }

    #[test]
    fn provision_confirm_reports_the_counts_and_reloads_pending_and_rulesets() {
        use crate::tui::test_support::{model_with_policy, provision_plan};
        use pce::write::Outcome;
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(
            &mut m,
            Msg::Planned(Ok((provision_plan(), pce::write::ObjectKind::Provision))),
        );
        assert_eq!(
            update(&mut m, key('y')),
            vec![Cmd::ExecuteWrite {
                plan: provision_plan(),
                kind: pce::write::ObjectKind::Provision
            }]
        );
        assert_eq!(
            m.policy.provision_counts.as_deref(),
            Some("ip_lists=1, rule_sets=1")
        );
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Provision,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written("/orgs/5/sec_policy/7", false))),
            },
        );
        assert_eq!(
            cmds,
            vec![
                Cmd::LoadRuleSets {
                    scope: PolicyScope::Draft,
                    mode: pce::CacheMode::Bypass
                },
                Cmd::LoadPending(pce::CacheMode::Bypass),
            ]
        );
        let report = "provisioned /orgs/5/sec_policy/7: ip_lists=1, rule_sets=1";
        assert_eq!(m.status.text, report);
        assert!(m.policy.provision_counts.is_none());
        let sets = m.policy.raw.clone();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets, 0)),
            },
        );
        assert_eq!(
            m.status.text, report,
            "the rulesets reload keeps the report"
        );
        update(&mut m, Msg::PendingLoaded(Ok(vec![])));
        assert_eq!(m.status.text, format!("{report}; 0 pending changes left"));
        assert_eq!(m.overlay, Overlay::Pending);
    }

    #[test]
    fn nothing_to_provision_is_an_info_and_a_pending_reload_never_hides_an_error() {
        let mut m = writes_on(model_fixture());
        update(
            &mut m,
            Msg::Planned(Ok((
                pce::write::Plan::Noop {
                    existing_href: "/orgs/5/sec_policy/pending".into(),
                    reason: "nothing pending".into(),
                },
                pce::write::ObjectKind::Provision,
            ))),
        );
        assert_eq!(m.status.kind, StatusKind::Info);
        assert_eq!(m.status.text, "nothing pending");
        assert_eq!(m.overlay, Overlay::None);
        m.overlay = Overlay::Error("write failed: 406".into());
        update(&mut m, Msg::PendingLoaded(Ok(vec![])));
        assert_eq!(
            m.overlay,
            Overlay::Error("write failed: 406".into()),
            "the reload does not replace the error"
        );
    }

    #[test]
    fn a_pending_rule_entry_points_at_its_ruleset() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(
            &mut m,
            Msg::PendingLoaded(Ok(vec![pce::models::PendingChange {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
                change_type: "update".into(),
                name: None,
                object_type: "sec_rules".into(),
            }])),
        );
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.status.text, "rule changes show in their ruleset's diff");
        assert!(!m.policy.diff_loading);
    }
}
