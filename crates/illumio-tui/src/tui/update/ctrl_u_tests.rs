use super::*;
use crate::tui::filter::FilterEditor;
use crate::tui::label_editor::LabelFilterEditor;
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::Msg;
use crate::tui::test_support::{model_fixture, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn send(m: &mut Model, code: KeyCode, mods: KeyModifiers) {
    update(m, Msg::Key(KeyEvent::new(code, mods)));
}

fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        send(m, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn ctrl_u(m: &mut Model) {
    send(m, KeyCode::Char('u'), KeyModifiers::CONTROL);
}

#[test]
fn ctrl_u_empties_the_command_line() {
    let mut m = model_fixture();
    m.overlay = Overlay::Prompt {
        input: "writes on".into(),
    };
    ctrl_u(&mut m);
    type_text(&mut m, "q");
    assert!(matches!(&m.overlay, Overlay::Prompt { input } if input == "q"));
}

#[test]
fn ctrl_u_empties_the_search_and_keeps_it_open() {
    let mut m = with_workloads(model_fixture());
    m.screen = Screen::Workloads;
    type_text(&mut m, "/web");
    ctrl_u(&mut m);
    assert!(m.workloads.table.is_searching());
    assert_eq!(m.workloads.table.search_text(), "");
    type_text(&mut m, "db");
    assert_eq!(m.workloads.table.search_text(), "db", "no stray u");
}

#[test]
fn ctrl_u_empties_the_active_field_of_the_traffic_filter() {
    let mut m = model_fixture();
    let mut e = FilterEditor::from_filter(&m.traffic.filter);
    e.active = 1;
    e.values[0] = "keep".into();
    e.values[1] = "app=shop".into();
    m.overlay = Overlay::FilterEditor(e);
    ctrl_u(&mut m);
    let Overlay::FilterEditor(e) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!((e.values[0].as_str(), e.values[1].as_str()), ("keep", ""));
}

#[test]
fn ctrl_u_empties_the_workloads_label_filter() {
    let mut m = with_workloads(model_fixture());
    m.screen = Screen::Workloads;
    m.overlay = Overlay::LabelFilter(LabelFilterEditor::new("app=shop env=prod"));
    ctrl_u(&mut m);
    assert!(matches!(&m.overlay, Overlay::LabelFilter(e) if e.value.is_empty()));
}

#[test]
fn ctrl_u_empties_the_active_events_field() {
    let mut m = model_fixture();
    let mut e = crate::tui::events_editor::EventsEditor::new([
        "err".into(),
        "user.login".into(),
        "24h".into(),
    ]);
    e.active = 1;
    m.overlay = Overlay::EventsEditor(e);
    ctrl_u(&mut m);
    assert!(matches!(
        &m.overlay,
        Overlay::EventsEditor(e) if e.values[0] == "err" && e.values[1].is_empty() && e.values[2] == "24h"
    ));
}
