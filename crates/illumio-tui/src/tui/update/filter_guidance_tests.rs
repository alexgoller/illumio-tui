//! #88: a filter typed without its format. A severity that is not one
//! is refused in the editor (no request); a PCE error for an applied
//! filter comes back into the editor, which stays editable; a bare label
//! value becomes `key=value` when one loaded label has it.
use super::*;
use crate::tui::events_editor::SEVERITY;
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{model_fixture, model_with_events, model_with_policy};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn send(m: &mut Model, code: KeyCode, mods: KeyModifiers) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, mods)))
}

#[track_caller]
fn press(m: &mut Model, code: KeyCode) {
    assert_eq!(send(m, code, KeyModifiers::NONE), Vec::<Cmd>::new());
}

#[track_caller]
fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        press(m, KeyCode::Char(c));
    }
}

fn enter(m: &mut Model) -> Vec<Cmd> {
    send(m, KeyCode::Enter, KeyModifiers::NONE)
}

/// The fixture labels plus `app=ordering` and `staging` as an `env` and
/// a `loc` value.
fn with_more_labels(mut m: Model) -> Model {
    let labels: Vec<pce::models::Label> = serde_json::from_value(serde_json::json!([
        {"href": "/orgs/5/labels/11", "key": "app", "value": "ordering"},
        {"href": "/orgs/5/labels/12", "key": "env", "value": "staging"},
        {"href": "/orgs/5/labels/13", "key": "loc", "value": "staging"},
        {"href": "/orgs/5/labels/14", "key": "app", "value": "orderbook"},
    ]))
    .unwrap();
    let idx = m.index().unwrap().clone();
    let mut all: Vec<pce::models::Label> = idx.labels().into_iter().cloned().collect();
    all.extend(labels);
    m.connection = crate::tui::model::Connection::Connected {
        index: Some(std::sync::Arc::new(idx.with_labels(&all))),
        index_age_secs: 0,
    };
    m
}

fn on_events() -> Model {
    let mut m = model_with_events();
    m.screen = Screen::Events;
    m
}

#[track_caller]
fn events_editor(m: &Model) -> &crate::tui::events_editor::EventsEditor {
    match &m.overlay {
        Overlay::EventsEditor(e) => e,
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_severity_stays_in_the_editor_lists_the_values_and_asks_nothing() {
    let mut m = on_events();
    press(&mut m, KeyCode::Char('f'));
    assert_eq!(events_editor(&m).active, SEVERITY);
    type_text(&mut m, "Ordering");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new(), "nothing sent");
    let e = events_editor(&m);
    let err = e.error.clone().expect("an inline error");
    for v in ["emerg", "err", "warning", "debug", "X+"] {
        assert!(err.contains(v), "{err}");
    }
    assert_eq!(m.events.filter.severity, "", "the filter in force is kept");
    // Still editable: Backspace and x reach the field.
    press(&mut m, KeyCode::Backspace);
    press(&mut m, KeyCode::Char('x'));
    assert_eq!(events_editor(&m).values[SEVERITY], "Orderinx");
    assert_eq!(events_editor(&m).error, None);
}

#[test]
fn a_severity_alias_is_sent_as_the_pce_word() {
    let mut m = on_events();
    press(&mut m, KeyCode::Char('f'));
    type_text(&mut m, "error");
    let cmds = enter(&mut m);
    let [Cmd::LoadEvents { filter, .. }] = &cmds[..] else {
        panic!("{cmds:?}")
    };
    assert_eq!(filter.severity.as_deref(), Some("err"));
}

#[test]
fn the_pce_rejecting_an_applied_events_filter_reopens_the_editor_editable() {
    let mut m = on_events();
    press(&mut m, KeyCode::Char('f'));
    type_text(&mut m, "err");
    let cmds = enter(&mut m);
    assert!(matches!(&cmds[..], [Cmd::LoadEvents { .. }]), "{cmds:?}");
    assert_eq!(m.overlay, Overlay::None);
    let pce = r#"PCE returned 406: {"error":"severity does not have a valid value"}"#;
    assert_eq!(
        update(&mut m, Msg::EventsLoaded(Err(pce.into()))),
        Vec::<Cmd>::new()
    );
    let e = events_editor(&m);
    assert!(
        e.error
            .as_deref()
            .is_some_and(|x| x.contains("severity does not have")),
        "{e:?}"
    );
    assert_eq!(e.values[SEVERITY], "err", "the edit is kept to fix");
    assert_eq!(m.events.filter.severity, "", "the filter before it holds");
    // No key-swallowing box: the field takes keys.
    press(&mut m, KeyCode::Backspace);
    press(&mut m, KeyCode::Char('x'));
    assert_eq!(events_editor(&m).values[SEVERITY], "erx");
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn a_failed_load_not_from_the_editor_keeps_the_old_behaviour() {
    let mut m = on_events();
    let cmds = send(&mut m, KeyCode::Char('r'), KeyModifiers::NONE);
    assert!(matches!(&cmds[..], [Cmd::LoadEvents { .. }]), "{cmds:?}");
    update(&mut m, Msg::EventsLoaded(Err("boom".into())));
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.status.text.contains("boom"), "{:?}", m.status);
}

// --- Traffic -------------------------------------------------------------

fn traffic_editor(m: &Model) -> &crate::tui::filter::FilterEditor {
    match &m.overlay {
        Overlay::FilterEditor(e) => e,
        other => panic!("{other:?}"),
    }
}

/// Traffic, `f`, Tab to `field`, `text` typed there.
fn traffic_typed(field: usize, text: &str) -> Model {
    let mut m = with_more_labels(model_fixture());
    m.screen = Screen::Traffic;
    press(&mut m, KeyCode::Char('f'));
    for _ in 0..field {
        press(&mut m, KeyCode::Tab);
    }
    send(&mut m, KeyCode::Char('u'), KeyModifiers::CONTROL);
    type_text(&mut m, text);
    m
}

#[test]
fn a_bare_traffic_source_value_of_one_label_becomes_key_value() {
    let mut m = traffic_typed(1, "Ordering");
    let cmds = enter(&mut m);
    assert!(matches!(&cmds[..], [Cmd::RunQuery { .. }]), "{cmds:?}");
    assert_eq!(m.traffic.filter.src, "app=ordering");
}

#[test]
fn bare_values_keep_their_bang_and_separators_and_ip_terms_stay() {
    let mut m = traffic_typed(2, "ordering | 10.0.0.1, !shop");
    let cmds = enter(&mut m);
    assert!(matches!(&cmds[..], [Cmd::RunQuery { .. }]), "{cmds:?}");
    assert_eq!(m.traffic.filter.dst, "app=ordering | 10.0.0.1, !app=shop");
}

#[test]
fn a_bare_traffic_value_of_several_keys_asks_which() {
    let mut m = traffic_typed(1, "staging");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new());
    let e = traffic_editor(&m);
    assert_eq!(
        e.error.as_deref(),
        Some("source: \"staging\" is a value of env and loc: write env=staging or loc=staging")
    );
    assert_eq!(e.values[1], "staging", "nothing rewritten");
}

#[test]
fn a_bare_traffic_value_of_no_label_names_similar_ones() {
    let mut m = traffic_typed(1, "order");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new());
    let err = traffic_editor(&m).error.clone().unwrap();
    assert!(err.contains("cannot parse filter \"order\""), "{err}");
    assert!(err.contains("use key=value"), "{err}");
    assert!(
        err.contains("\"order\"; similar: app=orderbook, app=ordering; use key=value"),
        "{err}"
    );
}

#[test]
fn a_failed_query_from_the_editor_reopens_it_editable_with_the_error() {
    let mut m = traffic_typed(1, "app=shop");
    let before = m.traffic.filter.clone();
    let cmds = enter(&mut m);
    let [Cmd::RunQuery { seq, .. }] = &cmds[..] else {
        panic!("{cmds:?}")
    };
    let seq = *seq;
    assert_eq!(m.overlay, Overlay::None);
    let pce = "PCE returned 406: {\"error\":\"sources does not have a valid value, please check the input\"}";
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Err(pce.into()),
        },
    );
    let e = traffic_editor(&m);
    assert!(
        e.error
            .as_deref()
            .is_some_and(|x| x.contains("sources does not have")),
        "{e:?}"
    );
    assert_eq!(e.values[1], "app=shop");
    assert_eq!(m.traffic.filter, before, "the filter before it holds");
    press(&mut m, KeyCode::Backspace);
    assert_eq!(traffic_editor(&m).values[1], "app=sho");
}

#[test]
fn a_failed_query_not_from_the_editor_keeps_the_old_behaviour() {
    let mut m = traffic_typed(1, "app=shop");
    let cmds = enter(&mut m);
    let [Cmd::RunQuery { seq, .. }] = &cmds[..] else {
        panic!("{cmds:?}")
    };
    let first = *seq;
    // Another filter (as a saved one applies) supersedes it.
    let mut other = m.traffic.filter.clone();
    other.src = "env=prod".into();
    let cmds = update(&mut m, Msg::ApplyFilter(other));
    let [Cmd::RunQuery { seq, .. }] = &cmds[..] else {
        panic!("{cmds:?}")
    };
    assert_ne!(*seq, first);
    let long = "x".repeat(80);
    update(
        &mut m,
        Msg::QueryDone {
            seq: *seq,
            result: Err(long.clone()),
        },
    );
    assert!(
        matches!(&m.overlay, Overlay::Error(x) if x.contains(&long)),
        "{:?}",
        m.overlay
    );
}

// --- Workloads -------------------------------------------------------------

fn label_editor(m: &Model) -> &crate::tui::label_editor::LabelFilterEditor {
    match &m.overlay {
        Overlay::LabelFilter(e) => e,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_bare_workloads_label_value_becomes_key_value() {
    let mut m = with_more_labels(model_fixture());
    m.screen = Screen::Workloads;
    press(&mut m, KeyCode::Char('f'));
    type_text(&mut m, "env=prod !SHOP");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None, "{:?}", m.overlay);
    assert_eq!(m.workloads.labels.to_string(), "env=prod !app=shop");
}

#[test]
fn an_ambiguous_workloads_label_value_stays_in_the_editor() {
    let mut m = with_more_labels(model_fixture());
    m.screen = Screen::Workloads;
    press(&mut m, KeyCode::Char('f'));
    type_text(&mut m, "staging");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new());
    assert_eq!(
        label_editor(&m).error.as_deref(),
        Some("\"staging\" is a value of env and loc: write env=staging or loc=staging")
    );
}

// --- Policy ---------------------------------------------------------------

#[test]
fn a_bare_rule_search_value_becomes_key_value_and_words_stay() {
    let mut m = with_more_labels(model_with_policy());
    m.screen = Screen::Policy;
    press(&mut m, KeyCode::Char('f'));
    type_text(&mut m, "deny -Shop");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None, "{:?}", m.overlay);
    assert_eq!(
        m.policy.search.as_ref().unwrap().to_string(),
        "deny -app=shop"
    );
}

#[test]
fn an_unknown_bare_rule_search_word_keeps_the_grammar_and_adds_similar() {
    let mut m = with_more_labels(model_with_policy());
    m.screen = Screen::Policy;
    press(&mut m, KeyCode::Char('f'));
    type_text(&mut m, "orde");
    assert_eq!(enter(&mut m), Vec::<Cmd>::new());
    let Overlay::RuleFilter(e) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let err = e.error.clone().unwrap();
    assert!(
        err.starts_with("\"orde\": unknown term; similar: app=orderbook, app=ordering; use "),
        "{err}"
    );
}
