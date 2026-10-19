use super::*;
use crate::tui::filter::FilterEditor;
use crate::tui::model::{Model, Overlay};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{model_fixture, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Sends the key and asserts it asked for nothing.
#[track_caller]
fn send(m: &mut Model, code: KeyCode, mods: KeyModifiers) {
    assert_eq!(
        update(m, Msg::Key(KeyEvent::new(code, mods))),
        Vec::<Cmd>::new()
    );
}

#[track_caller]
fn press(m: &mut Model, code: KeyCode) {
    send(m, code, KeyModifiers::NONE);
}

#[track_caller]
fn ctrl(m: &mut Model, c: char) {
    send(m, KeyCode::Char(c), KeyModifiers::CONTROL);
}

#[track_caller]
fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        press(m, KeyCode::Char(c));
    }
}

/// The fixture model with the filter editor open on field `field`, its
/// text replaced by `text`.
fn editing(field: usize, text: &str) -> Model {
    let mut m = model_fixture();
    let mut e = FilterEditor::from_filter(&m.traffic.filter);
    e.active = field;
    e.values[field] = text.into();
    m.overlay = Overlay::FilterEditor(e);
    m
}

#[track_caller]
fn editor(m: &Model) -> &FilterEditor {
    match &m.overlay {
        Overlay::FilterEditor(e) => e,
        other => panic!("{other:?}"),
    }
}

#[track_caller]
fn value(m: &Model, field: usize) -> String {
    editor(m).values[field].clone()
}

#[test]
fn src_cycles_dives_into_a_key_and_esc_restores_then_cancels() {
    let mut m = editing(1, "app=shop, r");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 1), "app=shop, role=", "the key first");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 1), "app=shop, role=db", "then its values");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 1), "app=shop, role=web");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 1), "app=shop, role=db", "wraps");
    ctrl(&mut m, 'p');
    assert_eq!(value(&m, 1), "app=shop, role=web");
    assert_eq!(
        editor(&m).completion_strip(),
        Some(vec![
            ("role=db".to_string(), false),
            ("role=web".to_string(), true)
        ])
    );
    press(&mut m, KeyCode::Esc);
    assert_eq!(value(&m, 1), "app=shop, r", "the typed text is back");
    assert_eq!(editor(&m).completion, None, "Esc ended the completion only");
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None, "a second Esc cancels the editor");
}

#[test]
fn any_other_key_accepts_and_operators_and_earlier_tokens_stay() {
    let mut m = editing(2, "app=shop | ro");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 2), "app=shop | role=");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 2), "app=shop | role=db");
    press(&mut m, KeyCode::Char(','));
    assert_eq!(value(&m, 2), "app=shop | role=db,", "accepted, then typed");
    assert_eq!(editor(&m).completion, None);
    type_text(&mut m, "ipl:c");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 2), "app=shop | role=db, ipl:Corp");
    press(&mut m, KeyCode::Tab);
    assert_eq!(
        (value(&m, 2).as_str(), editor(&m).active),
        ("app=shop | role=db, ipl:Corp", 3),
        "Tab accepts and moves on"
    );
    assert_eq!(editor(&m).completion, None);
    // Backspace accepts and then deletes.
    let mut m = editing(1, "ipl:a");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 1), "ipl:Any (0.0.0.0/0 and ::/0)");
    press(&mut m, KeyCode::Backspace);
    assert_eq!(value(&m, 1), "ipl:Any (0.0.0.0/0 and ::/0");
}

#[test]
fn window_service_and_decision_complete_their_words() {
    let mut m = editing(0, "");
    ctrl(&mut m, 'p');
    assert_eq!(value(&m, 0), "90d", "Ctrl-P opens on the last step");
    ctrl(&mut m, 'p');
    assert_eq!(value(&m, 0), "4w");
    let mut m = editing(0, "24h");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 0), "2d", "the pre-filled window: where `+` goes");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 0), "3d");
    let mut m = editing(1, "!ro");
    let before = m.overlay.clone();
    ctrl(&mut m, 'n');
    assert_eq!(
        m.overlay, before,
        "no negation in the filter: left as typed"
    );
    let mut m = editing(3, "post");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 3), "5432/tcp");
    let mut m = editing(3, "443/");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 3), "443/", "a port is left as typed");
    let mut m = editing(4, "allowed,b");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 4), "allowed, blocked");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 4), "allowed, potentially_blocked");
    press(&mut m, KeyCode::Tab);
    let f = editor(&m).to_filter().expect("a valid filter");
    assert_eq!(f.decisions_text(), "allowed,potentially_blocked");
}

#[test]
fn workloads_are_offered_only_once_the_workloads_screen_has_loaded() {
    let mut m = editing(2, "wl:");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 2), "wl:", "nothing loaded, nothing offered");
    assert_eq!(editor(&m).completion, None);
    let mut m = with_workloads(m);
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 2), "wl:batch04");
    ctrl(&mut m, 'n');
    assert_eq!(value(&m, 2), "wl:host-1.example");
}

#[test]
fn a_text_field_or_no_index_completes_nothing() {
    let mut m = editing(5, "100");
    let before = m.overlay.clone();
    ctrl(&mut m, 'n');
    assert_eq!(m.overlay, before, "max results: not typed, not completed");
    let mut m = editing(1, "ro");
    m.connection = Connection::Connected {
        index: None,
        index_age_secs: 0,
    };
    let before = m.overlay.clone();
    ctrl(&mut m, 'n');
    ctrl(&mut m, 'p');
    assert_eq!(m.overlay, before);
}
