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
        Msg::Mouse(_) => Vec::new(),
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
                return start_query(model, f, CacheMode::Bypass);
            }
            if model.screen == Screen::Policy
                && model.focused
                && model.overlay == Overlay::None
                && model.policy.loaded_at.is_some()
                && !model.policy.loading
                && model.policy.auto.due(now)
            {
                return load_rule_sets(model, CacheMode::Bypass);
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
                // Through the cache: the PCE is only asked once the object TTL has run out.
                return load_objects(model, kind, CacheMode::Use);
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
            model.status = StatusLine::error(format!("not connected: {e}"));
            model.connection = Connection::Disconnected { error: e };
            Vec::new()
        }
        Msg::IndexLoaded { index, age_secs } => {
            model.connection = Connection::Connected {
                index: Some(index),
                index_age_secs: age_secs,
            };
            model.status = StatusLine::info("ready");
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
            cmds
        }
        Msg::IndexFailed(e) => {
            model.status = StatusLine::error(format!("could not load objects: {e}"));
            Vec::new()
        }
        Msg::Stats(s) => {
            model.stats = s;
            Vec::new()
        }
        Msg::SwitchScreen(s) => switch(model, s),
        Msg::Command(c) => on_command(model, c),
        Msg::ApplyFilter(f) => {
            model.traffic.filter = f.clone();
            start_query(model, f, CacheMode::Use)
        }
        Msg::RunNow => {
            if model.traffic.is_running() {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            let f = model.traffic.filter.clone();
            start_query(model, f, CacheMode::Bypass)
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
                    model.status = StatusLine::error(e);
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
                    model.status = StatusLine::error(format!("rulesets: {e}"));
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
                }
                Err(e) => {
                    model.status = StatusLine::error(format!("{}: {e}", kind.title()));
                }
            }
            Vec::new()
        }
        Msg::PendingLoaded(result) => {
            match result {
                Ok(p) => {
                    model.status = StatusLine::info(format!("{} pending changes", p.len()));
                    model.policy.pending = Some(p);
                    model.overlay = Overlay::Pending;
                }
                Err(e) => model.status = StatusLine::error(format!("pending: {e}")),
            }
            Vec::new()
        }
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
        Overlay::RuleDetail(_) | Overlay::Pending => {
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
        _ => Vec::new(),
    }
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
        _ => Vec::new(),
    }
}

fn load_rule_sets(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.policy.loading = true;
    model.policy.auto.last_run = Some(Instant::now());
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
                model.policy.bypass_deny = true;
                return load_rule_sets(model, CacheMode::Bypass);
            }
            KeyCode::Char('p') => {
                model.status = StatusLine::info("loading pending changes");
                return vec![Cmd::LoadPending(CacheMode::Use)];
            }
            KeyCode::Char('a') => {
                let a = &mut model.policy.auto;
                a.enabled = !a.enabled;
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
    vec![Cmd::LoadObjects { kind, mode }]
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
                    model.status = StatusLine::info("objects are still loading");
                } else if model.objects.loading[kind.index()] {
                    model.status = StatusLine::info(format!("{} already loading", kind.title()));
                } else {
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
        Command::Profile(_) | Command::Draft | Command::Active => {
            model.status = StatusLine::warn("not available yet");
            Vec::new()
        }
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
        update(&mut m, key('a'));
        assert!(!m.traffic.auto.enabled);
        assert_eq!(m.status.text, "auto-refresh off");
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
    fn number_keys_switch_screens_only_policy_and_objects_load() {
        let mut m = model_fixture();
        assert_eq!(m.screen, Screen::Traffic);
        // entering Policy for the first time loads its rulesets (see the policy tests)
        assert_eq!(update(&mut m, key('2')).len(), 1);
        assert_eq!(m.screen, Screen::Policy);
        // entering Objects for the first time loads all four kinds (cache hits)
        assert_eq!(update(&mut m, key('3')).len(), 4);
        assert_eq!(m.screen, Screen::Objects);
        assert!(update(&mut m, key('4')).is_empty());
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
                mode: pce::CacheMode::Use
            }],
            "current kind only, through the cache"
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
}
