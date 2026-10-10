//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::commands::{self, Command};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::model::{Connection, Model, Overlay, PendingAction, Screen, StatusLine};
use crate::tui::msg::{Cmd, Msg};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::CacheMode;

pub fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(k) => on_key(model, k),
        Msg::Mouse(_) => Vec::new(),
        Msg::Resize(w, h) => {
            model.size = (w, h);
            Vec::new()
        }
        Msg::Tick(_) => Vec::new(),
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
            model.traffic.filter = f;
            model.status =
                StatusLine::info(format!("filter set: {}", model.traffic.filter.summary()));
            Vec::new()
        }
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
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
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

fn traffic_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    match k.code {
        KeyCode::Char('f') => {
            model.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&model.traffic.filter));
            Vec::new()
        }
        _ => Vec::new(),
    }
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
            msgs.is_empty(),
            "Task 6 turns the applied filter into Cmd::RunQuery; for now nothing is emitted"
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
}
