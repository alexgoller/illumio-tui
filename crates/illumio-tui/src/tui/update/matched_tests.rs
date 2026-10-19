use super::*;
use crate::tui::model::{Model, Overlay};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{model_with_policy, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::impact::Side;

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn key(c: char) -> Msg {
    kc(KeyCode::Char(c))
}

#[track_caller]
fn press(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

/// Policy, rules pane, the override deny rule 8 selected (row 0).
fn on_rules(workloads: bool) -> Model {
    let mut m = model_with_policy();
    if workloads {
        m = with_workloads(m);
    }
    m.screen = Screen::Policy;
    press(&mut m, kc(KeyCode::Tab));
    m
}

fn hosts(m: &Model) -> Vec<String> {
    let t = &m.policy.matched.as_ref().unwrap().table;
    t.visible()
        .iter()
        .map(|&i| t.rows()[i].hostname.clone())
        .collect()
}

#[test]
fn capital_i_without_cached_workloads_says_to_press_i_and_asks_nothing() {
    let mut m = on_rules(false);
    press(&mut m, key('I'));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(
        m.status.text,
        "press i first: the impact needs the workloads (1 request)"
    );
    // `I` on the rulesets pane means nothing.
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.status = StatusLine::info("ready");
    press(&mut m, key('I'));
    assert_eq!(m.status.text, "ready");
}

#[test]
fn capital_i_lists_each_side_and_tab_switches() {
    let mut m = on_rules(true);
    press(&mut m, key('I'));
    assert_eq!(m.overlay, Overlay::Matched);
    let v = m.policy.matched.as_ref().unwrap();
    assert_eq!(v.side, Side::Provider);
    assert_eq!(v.rule, "override_deny role=db ← All workloads 22/tcp");
    assert_eq!(hosts(&m), ["host-2.example"]);
    press(&mut m, kc(KeyCode::Tab));
    assert_eq!(m.policy.matched.as_ref().unwrap().side, Side::Consumer);
    assert_eq!(hosts(&m), ["host-1.example", "host-2.example"]);
    // `/` searches the list.
    press(&mut m, key('/'));
    for c in "host-1".chars() {
        press(&mut m, key(c));
    }
    press(&mut m, kc(KeyCode::Enter));
    assert_eq!(hosts(&m), ["host-1.example"]);
    press(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.policy.matched.is_none());
}

#[test]
fn an_ip_list_side_explains_instead_of_listing() {
    let mut m = on_rules(true);
    // Rows: deny 8, allow 1, allow 2 (Corp → role=web), deny 7.
    press(&mut m, key('j'));
    press(&mut m, key('j'));
    press(&mut m, key('I'));
    let v = m.policy.matched.as_ref().unwrap();
    assert_eq!(hosts(&m), ["host-1.example"]);
    assert_eq!(v.note(Side::Provider), None);
    assert!(v.rows(Side::Consumer).is_empty());
    assert_eq!(
        v.note(Side::Consumer),
        Some("the consumers are IP lists (Corp): addresses, not workloads; nothing to list")
    );
}

#[test]
fn colon_export_writes_the_shown_list() {
    let mut m = on_rules(true);
    press(&mut m, key('I'));
    press(&mut m, kc(KeyCode::Tab));
    press(&mut m, key(':'));
    for c in "export /tmp/matched.csv".chars() {
        press(&mut m, key(c));
    }
    let cmds = update(&mut m, kc(KeyCode::Enter));
    match cmds.as_slice() {
        [
            Cmd::Export {
                path,
                force,
                headers,
                rows,
            },
        ] => {
            assert_eq!(path, "/tmp/matched.csv");
            assert!(!force);
            assert_eq!(
                headers,
                &["hostname", "labels", "enforcement", "online", "href"]
            );
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows[0],
                [
                    "host-1.example",
                    "app=shop; env=prod; role=web",
                    "full",
                    "true",
                    "/orgs/5/workloads/aaaa-1"
                ]
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(m.overlay, Overlay::Matched, "the list stays open");
    // Another command is refused there.
    press(&mut m, key(':'));
    for c in "draft".chars() {
        press(&mut m, key(c));
    }
    press(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.status.text, "only :export <path> works in this list");
    assert_eq!(m.policy.scope, PolicyScope::Draft);
}

#[test]
fn enter_jumps_to_the_workload_on_workloads() {
    let mut m = on_rules(true);
    press(&mut m, key('I'));
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert!(cmds.is_empty(), "the workloads are loaded: {cmds:?}");
    assert_eq!(m.screen, Screen::Workloads);
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.workloads.selected_row().map(|r| r.workload.href.as_str()),
        Some("/orgs/5/workloads/aaaa-2")
    );
    assert_eq!(m.status.text, "host-2.example");
}

#[test]
fn capital_i_in_the_rule_detail_opens_the_list() {
    let mut m = on_rules(true);
    press(&mut m, kc(KeyCode::Enter));
    assert!(matches!(m.overlay, Overlay::RuleDetail(_)));
    press(&mut m, key('I'));
    assert_eq!(m.overlay, Overlay::Matched);
    assert_eq!(hosts(&m), ["host-2.example"]);
    // `?` shows the help over the list; closing it comes back to the list.
    press(&mut m, key('?'));
    assert_eq!(m.overlay, Overlay::Help);
    press(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Matched);
    assert!(m.policy.matched.is_some());
    // `Esc` goes back to the detail it was opened from.
    press(&mut m, kc(KeyCode::Esc));
    assert!(
        matches!(&m.overlay, Overlay::RuleDetail(r) if r.href.ends_with("/deny_rules/8")),
        "{:?}",
        m.overlay
    );
    assert!(m.policy.matched.is_none());
    press(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn capital_i_on_a_search_row_lists_that_rule() {
    let mut m = with_workloads(model_with_policy());
    m.screen = Screen::Policy;
    press(&mut m, key('f'));
    for c in "role=web".chars() {
        press(&mut m, key(c));
    }
    press(&mut m, kc(KeyCode::Enter));
    // Row 1: rule 2 (Corp → role=web).
    press(&mut m, key('j'));
    press(&mut m, key('I'));
    assert_eq!(m.overlay, Overlay::Matched);
    let v = m.policy.matched.as_ref().unwrap();
    assert!(v.rule_href.ends_with("/sec_rules/2"), "{}", v.rule_href);
    assert!(
        v.rule.contains("Corp"),
        "named from the search row: {}",
        v.rule
    );
    assert_eq!(hosts(&m), ["host-1.example"]);
    press(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.policy.search.is_some(), "the search list is still there");
}
