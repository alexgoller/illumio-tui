//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::commands::{self, Command};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::model::{
    Connection, Model, ObjectKind, Overlay, PendingAction, PolicyPane, QueryState, Screen,
    StatusLine, TrafficData, TrafficView,
};
use crate::tui::msg::{Cmd, Msg, ObjectsPayload, QueryOutcome};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::CacheMode;
use pce::models::PolicyScope;
use pce::traffic::QueryProgress;
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
            } else if model.screen == Screen::Traffic
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
            vec![Cmd::LoadIndex(CacheMode::Use)]
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
            model.traffic.filter = f.clone();
            start_query(model, f, CacheMode::Use)
        }
        Msg::RunNow => {
            if model.traffic.is_running() {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            let f = model.traffic.filter.clone();
            let cmds = start_query(model, f, CacheMode::Bypass);
            model.traffic.auto.rearm();
            cmds
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
                    model.status = StatusLine::info(format!(
                        "{} rulesets ({})",
                        model.policy.rulesets.len(),
                        model.policy.scope.segment()
                    ));
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
        Msg::ObjectsLoaded { kind, result } => {
            let i = kind.index();
            model.objects.loading[i] = false;
            match result {
                Ok(payload) => {
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
                model.profile = info;
                model.limits = limits;
                model.reset_data();
                model.connection = Connection::Connecting;
                model.status =
                    StatusLine::info(format!("switched to {}; connecting…", model.profile.name));
                vec![Cmd::Connect]
            }
            Err(e) => {
                model.status = StatusLine::error(format!("profile: {e}"));
                Vec::new()
            }
        },
        Msg::PendingLoaded(result) => {
            match result {
                Ok(p) => {
                    model.status = StatusLine::info(format!("{} pending changes", p.len()));
                    model.policy.pending = Some(p);
                    model.overlay = Overlay::Pending;
                }
                Err(e) => fail(model, "pending", &e, false),
            }
            Vec::new()
        }
    }
}

/// A failed request: the status line says what failed (or that a failing
/// auto-refresh switched itself off); long or authentication errors also open
/// the error overlay unless another overlay is in use.
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
            if matches!(
                k.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                model.overlay = Overlay::None;
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
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled");
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::RuleDetail(_) | Overlay::Pending | Overlay::Error(_) => {
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
            model.overlay = Overlay::Help;
            Vec::new()
        }
        (KeyCode::Char('1'), _) => switch(model, Screen::Traffic),
        (KeyCode::Char('2'), _) => switch(model, Screen::Policy),
        (KeyCode::Char('3'), _) => switch(model, Screen::Objects),
        (KeyCode::Char('4'), _) => switch(model, Screen::Workloads),
        _ if model.screen == Screen::Traffic => traffic_key(model, k),
        _ if model.screen == Screen::Policy => policy_key(model, k),
        _ if model.screen == Screen::Objects => objects_key(model, k),
        _ if model.screen == Screen::Workloads => workloads_key(model, k),
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
            let a = &mut model.traffic.auto;
            a.enabled = !a.enabled;
            a.tripped = false;
            model.status = if a.enabled {
                StatusLine::info(format!("auto-refresh every {}s", a.interval.as_secs()))
            } else {
                StatusLine::info("auto-refresh off")
            };
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
                model.status = StatusLine::info("loading pending changes");
                return vec![Cmd::LoadPending(CacheMode::Use)];
            }
            KeyCode::Char('a') => {
                let a = &mut model.policy.auto;
                a.enabled = !a.enabled;
                a.tripped = false;
                model.status = if a.enabled {
                    StatusLine::info(format!("auto-refresh every {}s", a.interval.as_secs()))
                } else {
                    StatusLine::info("auto-refresh off")
                };
                return Vec::new();
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

fn workloads_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            crate::tui::ui::screens::workloads::table_rows(c.body, model.workloads.show_detail)
        })
        .unwrap_or(10)
        .max(1)
}

/// What the traffic filter calls this workload. `,` and `|` separate filter
/// terms, so a name containing them is addressed by its href id instead.
fn workload_filter_ref(w: &pce::models::Workload) -> String {
    let name = w.display_name();
    let name = if name.contains([',', '|']) {
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
                let a = &mut model.workloads.auto;
                a.enabled = !a.enabled;
                a.tripped = false;
                model.status = if a.enabled {
                    StatusLine::info(format!("auto-refresh every {}s", a.interval.as_secs()))
                } else {
                    StatusLine::info("auto-refresh off")
                };
                return Vec::new();
            }
            KeyCode::Char(c @ ('t' | 'T')) => {
                let Some(w) = model.workloads.table.selected_item() else {
                    return Vec::new();
                };
                let expr = workload_filter_ref(&w.workload);
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
            _ => {}
        }
    }
    drive_table(&mut model.workloads.table, k, page);
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
                let a = &mut model.objects.auto;
                a.enabled = !a.enabled;
                a.tripped = false;
                model.status = if a.enabled {
                    StatusLine::info(format!("auto-refresh every {}s", a.interval.as_secs()))
                } else {
                    StatusLine::info("auto-refresh off")
                };
                return Vec::new();
            }
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

/// `:draft` / `:active`: like `d` on the Policy screen, but explicit.
fn show_policy(model: &mut Model, scope: PolicyScope) -> Vec<Cmd> {
    if !model.is_connected() {
        model.status = StatusLine::error("not connected");
        return Vec::new();
    }
    model.screen = Screen::Policy;
    if model.policy.scope != scope {
        model.policy.scope = scope;
        model.policy.clear();
    }
    model.policy.loaded_at = None;
    load_rule_sets(model, CacheMode::Use)
}

pub fn on_command(model: &mut Model, cmd: Command) -> Vec<Cmd> {
    match cmd {
        Command::Screen(s) => switch(model, s),
        Command::Quit => {
            model.quit = true;
            vec![Cmd::Quit]
        }
        Command::Help => {
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
        Command::Profile(name) => vec![Cmd::SwitchProfile(name)],
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
        update(&mut m, key('g'));
        assert!(matches!(m.overlay, Overlay::GroupKeys { .. }));
        for _ in 0..7 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "role".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.group_keys, vec!["role"]);
        assert!(
            m.traffic.groups.rows().iter().any(|g| g.src == vec!["web"]),
            "{:?}",
            m.traffic.groups.rows()
        );
        update(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Blocked);
        assert_eq!(m.traffic.blocked.len(), 2);
        update(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.stats.calls, 5);
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
        update(&mut m, key('j'));
        assert_eq!(m.traffic.flows.selected_pos(), 2);
        update(&mut m, key('k'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.flows.selected_pos(), 3);
        assert!(update(&mut m, key('s')).is_empty());
        assert_eq!(m.traffic.flows.sort(), Some((0, false)));
        update(&mut m, key('/'));
        assert!(m.traffic.flows.is_searching());
        for c in "nas".chars() {
            update(&mut m, key(c));
        }
        assert_eq!(m.traffic.flows.len(), 1);
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(!m.traffic.flows.is_searching());
        update(&mut m, key('/'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.flows.len(), 4, "Esc clears the search");
        assert!(!m.traffic.show_detail);
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(m.traffic.show_detail);
        assert_eq!(m.stats.calls, 5, "no request was made by any of this");
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
        update(&mut m, key('d')); // Active now loading
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
        update(&mut m, key('3'));
        update(&mut m, key('/'));
        for c in "role".chars() {
            update(&mut m, key(c));
        }
        assert_eq!(m.objects.labels.len(), 2);
        update(&mut m, kc(KeyCode::Enter));
        update(&mut m, kc(KeyCode::Enter));
        assert!(m.objects.show_detail);
        assert_eq!(m.stats.calls, 5);
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
        assert_eq!(m.workloads.table.len(), 3);
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
        assert_eq!(m.workloads.table.len(), 3, "rows kept on error");
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
        update(&mut m, key('4'));
        assert_eq!(m.workloads.table.len(), 3);
        update(&mut m, key('h'));
        assert!(m.workloads.unhealthy_only);
        assert_eq!(
            m.workloads.table.len(),
            1,
            "only host-2 (offline) is unhealthy at the fixed test clock"
        );
        update(&mut m, key('h'));
        assert_eq!(m.workloads.table.len(), 3);
        update(&mut m, key('H'));
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
        assert_eq!(m.stats.calls, 5, "pure projections: no request");
    }

    #[test]
    fn h_with_no_unhealthy_rows_is_empty_not_a_panic_and_capital_h_flips() {
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        update(&mut m, key('4'));
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
        update(&mut m, Msg::Resize(80, 24));
        update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10));
        update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10));
        assert_eq!(m.traffic.flows.selected_pos(), 2);
        update(&mut m, mouse(MouseEventKind::ScrollUp, 10, 10));
        assert_eq!(m.traffic.flows.selected_pos(), 1);
        let x = crate::tui::ui::tab_x(Screen::Policy);
        update(&mut m, mouse(MouseEventKind::Down(MouseButton::Left), x, 1));
        assert_eq!(m.screen, Screen::Policy);
        assert_eq!(m.stats.calls, 5);
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
}
