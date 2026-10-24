//! #71: the Events actor filter: the row keys `u`/`U`, the Actors view of
//! the `t` summary, the editor's Actor field and its completion. Every one
//! of them filters the loaded events: no request.
use super::*;
use crate::tui::events_editor::{ACTOR, WINDOW};
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::Msg;
use crate::tui::test_support::{model_with_events, render_to_string};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn send(m: &mut Model, code: KeyCode, mods: KeyModifiers) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, mods)))
}

fn press(m: &mut Model, c: char) -> Vec<Cmd> {
    send(m, KeyCode::Char(c), KeyModifiers::NONE)
}

/// The fixture's actors: `user:user-1` (user.login), `user:user-2`
/// (sec_policy.create), `agent:host-1.example` (agent.tampering) and
/// `system` (system.license_expiring).
fn events() -> Model {
    let mut m = model_with_events();
    m.screen = Screen::Events;
    m
}

fn shown_actors(m: &Model) -> Vec<String> {
    let mut a: Vec<String> = m.events.table.rows().iter().map(|e| e.actor()).collect();
    a.sort();
    a
}

fn select(m: &mut Model, actor: &str) {
    m.events.table.select_where(|e| e.actor() == actor);
}

#[test]
fn u_shows_only_the_rows_actor_and_shift_u_hides_it_without_a_request() {
    let mut m = events();
    select(&mut m, "user:user-1");
    assert!(press(&mut m, 'u').is_empty());
    assert_eq!(m.events.filter.actor, "=user:user-1");
    assert_eq!(shown_actors(&m), ["user:user-1"]);
    assert_eq!(
        m.status.text, "only user:user-1 · 1 of 4 events shown",
        "like o/x"
    );
    assert!(!m.events.loading);
    assert!(
        render_to_string(&m, 160, 48).contains("actor =user:user-1"),
        "the header names the actor terms"
    );
    // Back to all, then hide two.
    m.events.filter.actor.clear();
    m.events.rebuild();
    select(&mut m, "system");
    assert!(press(&mut m, 'U').is_empty());
    select(&mut m, "agent:host-1.example");
    assert!(press(&mut m, 'U').is_empty());
    assert_eq!(m.events.filter.actor, "!=system, !=agent:host-1.example");
    assert_eq!(shown_actors(&m), ["user:user-1", "user:user-2"]);
    assert!(
        m.status
            .text
            .starts_with("hiding agent:host-1.example · 2 of 4 events shown"),
        "{}",
        m.status.text
    );
    // Type and actor filters combine.
    m.events.filter.event_type = "sec_policy".into();
    m.events.rebuild();
    assert_eq!(shown_actors(&m), ["user:user-2"]);
    // Ctrl-U is not `u`.
    select(&mut m, "user:user-2");
    send(&mut m, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(m.events.filter.actor, "!=system, !=agent:host-1.example");
}

#[test]
fn hiding_and_showing_an_actor_rewrites_terms_naming_it_either_way() {
    use crate::tui::model::{hide_actor, only_actor};
    let alex = "user:alex@ryte.de";
    assert_eq!(hide_actor("", alex), "!=user:alex@ryte.de");
    // An exclude naming it by its name alone counts as naming it.
    assert_eq!(
        hide_actor("user:*, ! = Alex@Ryte.de", alex),
        "user:*, !=user:alex@ryte.de"
    );
    assert_eq!(
        only_actor("user:*, !system, !=alex@ryte.de, !agent:*", alex),
        "=user:alex@ryte.de, !system, !agent:*"
    );
    // An exclude that would hide it goes, even by a glob.
    assert_eq!(
        only_actor("!user:*, !system", alex),
        "=user:alex@ryte.de, !system"
    );
    assert_eq!(only_actor("", "system"), "=system");
}

#[test]
fn tab_in_the_summary_switches_to_actors_where_the_keys_act_on_the_actor() {
    let mut m = events();
    let mut e = m.events.raw[0].clone();
    e.href = "/orgs/5/events/e5".into();
    m.events.raw.push(e);
    m.events.rebuild();
    assert!(press(&mut m, 't').is_empty());
    assert!(press(&mut m, 'j').is_empty());
    assert!(send(&mut m, KeyCode::Tab, KeyModifiers::NONE).is_empty());
    assert_eq!(
        m.overlay,
        Overlay::EventTypes {
            selected: 0,
            actors: true
        },
        "the cursor starts at the top"
    );
    let counts = m.events.actor_counts();
    assert_eq!(
        counts[0],
        ("user:user-1".to_string(), 2),
        "most frequent first"
    );
    assert_eq!(counts[1].0, "agent:host-1.example", "then by name");
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("Event actors") && out.contains("4 actors"),
        "{out}"
    );
    // `x` and `!` hide the actor under the cursor and the list stays open.
    assert!(press(&mut m, 'j').is_empty());
    assert!(press(&mut m, 'x').is_empty());
    assert_eq!(m.events.filter.actor, "!=agent:host-1.example");
    assert_eq!(
        m.events.filter.event_type, "",
        "the type filter is untouched"
    );
    assert!(press(&mut m, 'j').is_empty());
    assert!(press(&mut m, '!').is_empty());
    assert_eq!(m.events.filter.actor, "!=agent:host-1.example, !=system");
    assert_eq!(
        m.overlay,
        Overlay::EventTypes {
            selected: 2,
            actors: true
        }
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("hidden"), "{out}");
    // `o` keeps only it and closes, as Enter does.
    assert!(press(&mut m, 'g').is_empty());
    assert!(press(&mut m, 'o').is_empty());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.events.filter.actor, "=user:user-1, !=agent:host-1.example, !=system",
        "the excludes that do not hide it stay"
    );
    assert_eq!(shown_actors(&m), ["user:user-1", "user:user-1"]);
    // Tab back to the types; `o` there shows only the type.
    assert!(press(&mut m, 't').is_empty());
    assert!(send(&mut m, KeyCode::Tab, KeyModifiers::NONE).is_empty());
    assert!(send(&mut m, KeyCode::BackTab, KeyModifiers::NONE).is_empty());
    assert_eq!(
        m.overlay,
        Overlay::EventTypes {
            selected: 0,
            actors: false
        }
    );
    // In the types view, a type the actor filter empties is marked.
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("(actor)"), "{out}");
    assert!(press(&mut m, 'o').is_empty());
    assert_eq!(m.events.filter.event_type, "=user.login");
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn the_editor_has_an_actor_field_that_filters_locally_and_refuses_bad_terms() {
    let mut m = events();
    assert!(press(&mut m, 'f').is_empty());
    let Overlay::EventsEditor(e) = &mut m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(e.values.len(), 4);
    assert_eq!(e.values[WINDOW], "24h");
    e.active = ACTOR;
    for c in "user:*, !=user-2".chars() {
        assert!(press(&mut m, c).is_empty());
    }
    assert!(
        send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty(),
        "an actor edit sends nothing"
    );
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.events.filter.actor, "user:*, !=user-2");
    assert_eq!(shown_actors(&m), ["user:user-1"]);
    assert!(m.status.text.contains("no request"), "{:?}", m.status);
    // The field opens with the actor terms in force; a bad term is refused.
    assert!(press(&mut m, 'f').is_empty());
    let Overlay::EventsEditor(e) = &mut m.overlay else {
        panic!()
    };
    assert_eq!(e.values[ACTOR], "user:*, !=user-2");
    e.active = ACTOR;
    for c in " !b".chars() {
        assert!(press(&mut m, c).is_empty());
    }
    assert!(send(&mut m, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    let Overlay::EventsEditor(e) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let err = e.error.clone().unwrap();
    assert!(err.starts_with("actor:") && err.contains("use ,"), "{err}");
    assert_eq!(m.events.filter.actor, "user:*, !=user-2", "nothing applied");
}

#[test]
fn ctrl_n_completes_the_loaded_actors_in_the_actor_field() {
    let mut m = events();
    assert!(press(&mut m, 'f').is_empty());
    let Overlay::EventsEditor(e) = &mut m.overlay else {
        panic!()
    };
    e.active = ACTOR;
    for c in "=user-1, !u".chars() {
        assert!(press(&mut m, c).is_empty());
    }
    assert!(send(&mut m, KeyCode::Char('n'), KeyModifiers::CONTROL).is_empty());
    let Overlay::EventsEditor(e) = &m.overlay else {
        panic!()
    };
    let strip: Vec<String> = e
        .completion_strip()
        .unwrap()
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    assert_eq!(
        strip,
        ["!user:user-2"],
        "user-1 is named already (by its name alone)"
    );
    assert_eq!(e.values[ACTOR], "=user-1, !user:user-2");
}

#[test]
fn actor_candidates_need_no_index_and_keep_a_typed_prefix() {
    use crate::tui::complete::event_actors;
    let m = events();
    assert_eq!(
        event_actors("", &[], &m.events.raw),
        [
            "agent:host-1.example",
            "system",
            "user:user-1",
            "user:user-2"
        ]
    );
    assert_eq!(
        event_actors("!=h", &[], &m.events.raw),
        ["!=agent:host-1.example"]
    );
    assert_eq!(
        event_actors(
            "",
            &["!system".into(), "= user:USER-2".into()],
            &m.events.raw
        ),
        ["agent:host-1.example", "user:user-1"]
    );
    assert!(event_actors("x", &[], &[]).is_empty());
}
