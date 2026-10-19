use super::*;
use crate::tui::model::{Connection, Model, Screen, StatusKind};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::model_fixture;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

/// Sends `msg` and asserts it asked for nothing: a key, a tab or a screen
/// change must not reach the PCE.
#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

fn running(m: &Model) -> bool {
    m.traffic.is_running()
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn mouse(kind: crossterm::event::MouseEventKind, column: u16, row: u16) -> Msg {
    Msg::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn enter() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

fn writes_on(mut m: Model) -> Model {
    m.writes = crate::tui::model::WritesState::on();
    m
}

mod editing;
mod other_screens;
mod traffic_screen;
