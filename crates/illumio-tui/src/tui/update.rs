//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::model::{Connection, Model, Screen, StatusLine};
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
    }
}

fn on_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    if k.kind == KeyEventKind::Release {
        return Vec::new();
    }
    match (k.code, k.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
            model.quit = true;
            vec![Cmd::Quit]
        }
        (KeyCode::Char('1'), _) => switch(model, Screen::Traffic),
        (KeyCode::Char('2'), _) => switch(model, Screen::Policy),
        (KeyCode::Char('3'), _) => switch(model, Screen::Objects),
        (KeyCode::Char('4'), _) => switch(model, Screen::Workloads),
        _ => Vec::new(),
    }
}

fn switch(model: &mut Model, s: Screen) -> Vec<Cmd> {
    model.screen = s;
    Vec::new()
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
}
