//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::commands::{self, Command};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::model::{
    Connection, Model, Overlay, PendingAction, QueryState, Screen, StatusLine, TrafficData,
};
use crate::tui::msg::{Cmd, Msg, QueryOutcome};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::CacheMode;
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
            Vec::new()
        }
        Msg::IndexFailed(e) => {
            model.status = StatusLine::error(format!("could not load objects: {e}"));
            Vec::new()
        }
        Msg::Stats(s) => {
            model.stats = s;
            Vec::new()
        }
        Msg::SwitchScreen(s) => {
            model.screen = s;
            Vec::new()
        }
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
                    model.traffic.flows.set_rows(
                        model
                            .traffic
                            .data
                            .as_ref()
                            .map(|d| d.flows.clone())
                            .unwrap_or_default(),
                    );
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
        Overlay::None => on_screen_key(model, k),
    }
}

fn on_screen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    // An active table search captures every key except Ctrl-C.
    if model.screen == Screen::Traffic
        && model.traffic.flows.is_searching()
        && !(k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return traffic_key(model, k);
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

fn traffic_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = traffic_page(model);
    let table = &mut model.traffic.flows;
    if table.is_searching() {
        match k.code {
            KeyCode::Esc => table.search_clear(),
            KeyCode::Enter => table.search_commit(),
            KeyCode::Backspace => table.search_pop(),
            KeyCode::Char(c) => table.search_push(c),
            _ => {}
        }
        table.ensure_visible(page);
        return Vec::new();
    }
    match k.code {
        KeyCode::Char('f') => {
            model.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&model.traffic.filter));
            return Vec::new();
        }
        KeyCode::Char('r') => return update(model, Msg::RunNow),
        KeyCode::Down | KeyCode::Char('j') => table.select_next(),
        KeyCode::Up | KeyCode::Char('k') => table.select_prev(),
        KeyCode::PageDown => table.page_down(page),
        KeyCode::PageUp => table.page_up(page),
        KeyCode::Home => table.home(),
        KeyCode::End => table.end(),
        KeyCode::Char('s') => table.sort_next(),
        KeyCode::Char('/') => table.start_search(),
        KeyCode::Enter => model.traffic.show_detail = !model.traffic.show_detail,
        _ => {}
    }
    model.traffic.flows.ensure_visible(page);
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
    fn number_keys_switch_screens_without_commands() {
        let mut m = model_fixture();
        assert_eq!(m.screen, Screen::Traffic);
        assert!(update(&mut m, key('2')).is_empty());
        assert_eq!(m.screen, Screen::Policy);
        update(&mut m, key('3'));
        assert_eq!(m.screen, Screen::Objects);
        update(&mut m, key('4'));
        assert_eq!(m.screen, Screen::Workloads);
        update(&mut m, key('1'));
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
}
