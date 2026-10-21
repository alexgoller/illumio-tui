//! #67: the Events type filter (terms, `!` excludes, globs), the row keys
//! `x`/`o`, the `t` type summary, `X+` severities and Type completion.
//! Every one of them filters the loaded events: no request.
use super::*;
use crate::tui::model::{Limits, Model, Overlay, Screen};
use crate::tui::msg::Msg;
use crate::tui::test_support::model_with_events;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn send(m: &mut Model, code: KeyCode, mods: KeyModifiers) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, mods)))
}

fn press(m: &mut Model, c: char) -> Vec<Cmd> {
    send(m, KeyCode::Char(c), KeyModifiers::NONE)
}

fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        assert!(press(m, c).is_empty());
    }
}

fn events() -> Model {
    let mut m = model_with_events();
    m.screen = Screen::Events;
    m
}

fn shown_types(m: &Model) -> Vec<String> {
    let mut t: Vec<String> = m
        .events
        .table
        .rows()
        .iter()
        .filter_map(|e| e.event_type.clone())
        .collect();
    t.sort();
    t
}

fn select(m: &mut Model, ty: &str) {
    m.events
        .table
        .select_where(|e| e.event_type.as_deref() == Some(ty));
}

/// Opens the editor and replaces field `field` with `text`.
fn edit_field(m: &mut Model, field: usize, text: &str) {
    if !matches!(m.overlay, Overlay::EventsEditor(_)) {
        assert!(press(m, 'f').is_empty());
    }
    let Overlay::EventsEditor(e) = &mut m.overlay else {
        panic!("{:?}", m.overlay)
    };
    e.active = field;
    send(m, KeyCode::Char('u'), KeyModifiers::CONTROL);
    type_text(m, text);
}

#[test]
fn x_hides_the_rows_type_and_o_shows_only_it_without_a_request() {
    let mut m = events();
    select(&mut m, "user.login");
    assert!(press(&mut m, 'x').is_empty());
    assert_eq!(m.events.filter.event_type, "!user.login");
    assert_eq!(m.events.hidden_types, 1);
    assert_eq!(m.events.table.len(), 3);
    assert!(
        m.status.text.contains("hiding user.login"),
        "{:?}",
        m.status
    );
    assert!(
        m.status.text.contains("3 of 4 events shown"),
        "{:?}",
        m.status
    );
    // `!` is the same key; hiding twice keeps one term.
    select(&mut m, "sec_policy.create");
    assert!(press(&mut m, '!').is_empty());
    assert_eq!(
        m.events.filter.event_type,
        "!user.login, !sec_policy.create"
    );
    // `o`: only this type; the other excludes stay, none hides it.
    select(&mut m, "agent.tampering");
    assert!(press(&mut m, 'o').is_empty());
    assert_eq!(
        m.events.filter.event_type,
        "agent.tampering, !user.login, !sec_policy.create"
    );
    assert_eq!(shown_types(&m), ["agent.tampering"]);
    assert_eq!(m.events.hidden_types, 3);
    assert!(!m.events.loading);
}

#[test]
fn only_drops_an_exclude_that_would_hide_the_type() {
    use crate::tui::model::{hide_type, only_type};
    assert_eq!(
        only_type("!agent.*, !user.login, x", "agent.tampering"),
        "agent.tampering, !user.login"
    );
    assert_eq!(
        hide_type("agent.tampering, !x", "agent.tampering"),
        "!x, !agent.tampering"
    );
    assert_eq!(hide_type("", "user.login"), "!user.login");
    assert_eq!(only_type("", "user.login"), "user.login");
}

#[test]
fn the_type_summary_counts_hides_and_picks_one() {
    let mut m = events();
    let mut e = m.events.raw[0].clone();
    e.href = "/orgs/5/events/e5".into();
    m.events.raw.push(e);
    m.events.rebuild();
    assert!(press(&mut m, 't').is_empty());
    assert_eq!(m.overlay, Overlay::EventTypes { selected: 0 });
    let counts = m.events.type_counts();
    assert_eq!(
        counts[0],
        ("user.login".to_string(), 2),
        "most frequent first"
    );
    assert_eq!(counts[1].0, "agent.tampering", "then by name");
    // `x` hides the type under the cursor and the list stays open.
    assert!(press(&mut m, 'j').is_empty());
    assert!(press(&mut m, 'x').is_empty());
    assert_eq!(m.events.filter.event_type, "!agent.tampering");
    assert_eq!(m.overlay, Overlay::EventTypes { selected: 1 });
    // Enter: only this type, and the list closes.
    assert!(press(&mut m, 'k').is_empty());
    assert!(send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.events.filter.event_type, "user.login, !agent.tampering");
    assert_eq!(m.events.table.len(), 2);
    // `?` and back; Esc closes.
    assert!(press(&mut m, 't').is_empty());
    assert!(press(&mut m, '?').is_empty());
    assert_eq!(m.overlay, Overlay::Help);
    assert!(send(&mut m, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(matches!(m.overlay, Overlay::EventTypes { .. }));
    assert!(send(&mut m, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn the_editor_refilters_locally_unless_the_pce_side_changes() {
    let mut m = events();
    edit_field(&mut m, 1, "agent, sec_policy.*");
    assert!(
        send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty(),
        "type only: no request"
    );
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(shown_types(&m), ["agent.tampering", "sec_policy.create"]);
    assert!(m.status.text.contains("no request"), "{:?}", m.status);
    // `warning+` asks the PCE for nothing new (it sends no severity).
    edit_field(&mut m, 0, "warning+");
    edit_field(&mut m, 1, "");
    assert!(send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert_eq!(
        shown_types(&m),
        ["agent.tampering", "system.license_expiring"]
    );
    edit_field(&mut m, 0, "err+");
    assert!(send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert_eq!(shown_types(&m), ["agent.tampering"]);
    // One severity is the PCE's filter: a request.
    edit_field(&mut m, 0, "err");
    let cmds = send(&mut m, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        matches!(&cmds[..], [Cmd::LoadEvents { filter, mode: CacheMode::Use }] if filter.severity.as_deref() == Some("err") && filter.event_type.is_none()),
        "{cmds:?}"
    );
    // A new window too.
    m.events.loading = false;
    edit_field(&mut m, 0, "");
    edit_field(&mut m, 2, "1h");
    let cmds = send(&mut m, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        matches!(&cmds[..], [Cmd::LoadEvents { filter, .. }] if filter.severity.is_none()),
        "{cmds:?}"
    );
}

#[test]
fn bad_terms_and_severities_are_refused_and_never_sent() {
    let mut m = events();
    for (field, bad, says) in [
        (1, "!", "type: !"),
        (1, "a !b", "use ,"),
        (0, "loud+", "X+"),
    ] {
        edit_field(&mut m, field, bad);
        assert!(send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty());
        let Overlay::EventsEditor(e) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        let err = e.error.clone().unwrap();
        assert!(err.contains(says), "{bad}: {err}");
        assert!(send(&mut m, KeyCode::Esc, KeyModifiers::NONE).is_empty());
        assert_eq!(m.events.filter.event_type, "", "nothing applied");
    }
}

#[test]
fn ctrl_n_completes_the_loaded_types_in_the_type_field_only() {
    let mut m = events();
    edit_field(&mut m, 1, "user.login, !s");
    let ctrl_n = |m: &mut Model| send(m, KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert!(ctrl_n(&mut m).is_empty());
    let Overlay::EventsEditor(e) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(e.values[1], "user.login, !sec_policy.create");
    let strip: Vec<String> = e
        .completion_strip()
        .unwrap()
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    assert_eq!(strip, ["!sec_policy.create", "!system.license_expiring"]);
    assert!(ctrl_n(&mut m).is_empty());
    let Overlay::EventsEditor(e) = &m.overlay else {
        panic!()
    };
    assert_eq!(e.values[1], "user.login, !system.license_expiring");
    // Esc puts the typed token back and keeps the editor.
    assert!(send(&mut m, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    let Overlay::EventsEditor(e) = &m.overlay else {
        panic!()
    };
    assert_eq!(e.values[1], "user.login, !s");
    assert!(e.completion.is_none());
    // The severity field completes nothing.
    edit_field(&mut m, 0, "w");
    assert!(ctrl_n(&mut m).is_empty());
    let Overlay::EventsEditor(e) = &m.overlay else {
        panic!()
    };
    assert_eq!(e.values[0], "w");
    assert!(e.completion.is_none());
}

#[test]
fn event_type_candidates_need_no_index_and_skip_named_types() {
    let m = events();
    let c = crate::tui::complete::event_types("", &["!User.Login".into()], &m.events.raw);
    assert_eq!(
        c,
        [
            "agent.tampering",
            "sec_policy.create",
            "system.license_expiring"
        ]
    );
    let c = crate::tui::complete::event_types("!tam", &[], &m.events.raw);
    assert_eq!(c, ["!agent.tampering"]);
    assert!(crate::tui::complete::event_types("x", &[], &[]).is_empty());
}

#[test]
fn the_profiles_excludes_start_the_type_filter_and_survive_a_switch() {
    let mut m = events();
    m.limits = Limits {
        event_exclude_types: vec!["user.login".into(), "agent.*".into()],
        ..m.limits.clone()
    };
    m.reset_data();
    assert_eq!(m.events.filter.event_type, "!user.login, !agent.*");
    m.events.raw = model_with_events().events.raw;
    m.events.rebuild();
    assert_eq!(
        shown_types(&m),
        ["sec_policy.create", "system.license_expiring"]
    );
    assert_eq!(m.events.hidden_types, 2);
}
