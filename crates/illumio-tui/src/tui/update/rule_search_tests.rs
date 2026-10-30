use super::*;
use crate::tui::model::{Model, Overlay};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{model_with_policy, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn key(c: char) -> Msg {
    kc(KeyCode::Char(c))
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// Sends the key and asserts it asked for nothing.
#[track_caller]
fn press(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

#[track_caller]
fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        press(m, key(c));
    }
}

fn on_policy() -> Model {
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m
}

/// `f`, the text, `Enter`.
#[track_caller]
fn search(m: &mut Model, text: &str) {
    press(m, key('f'));
    press(m, ctrl('u'));
    type_text(m, text);
    press(m, kc(KeyCode::Enter));
}

fn found_hrefs(m: &Model) -> Vec<String> {
    m.policy
        .found
        .rows()
        .iter()
        .map(|r| r.href.rsplit('/').take(2).collect::<Vec<_>>().join("<"))
        .collect()
}

#[test]
fn f_applies_a_search_locally_and_lists_rules_across_rulesets() {
    let mut m = on_policy();
    press(&mut m, key('f'));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value.is_empty()));
    type_text(&mut m, "role=web");
    press(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.policy.search.as_ref().map(ToString::to_string).as_deref(),
        Some("role=web")
    );
    assert_eq!(found_hrefs(&m), ["1<sec_rules", "2<sec_rules"]);
    // Ruleset 100 lists deny rules inline: 101's empty list means none.
    assert_eq!(m.status.text, "2 of 5 rules · role=web");
    // env=prod: ruleset 100 by scope, rule 3 through its label group.
    search(&mut m, "env=prod");
    assert_eq!(
        found_hrefs(&m),
        [
            "8<deny_rules",
            "1<sec_rules",
            "2<sec_rules",
            "7<deny_rules",
            "3<sec_rules"
        ]
    );
    assert_eq!(m.policy.found.rows()[4].ruleset, "Global-Infra");
    assert_eq!(m.policy.found.rows()[0].scopes, "app=shop, env=prod");
    // Terms AND; a comma is any of.
    search(&mut m, "role=web,db deny");
    assert_eq!(found_hrefs(&m), ["8<deny_rules"]);
    search(&mut m, "svc:443/tcp disabled");
    assert_eq!(found_hrefs(&m), ["3<sec_rules"]);
}

#[test]
fn a_malformed_search_stays_in_the_editor_and_unknown_names_warn() {
    let mut m = on_policy();
    search(&mut m, "role=web");
    press(&mut m, key('f'));
    press(&mut m, ctrl('u'));
    // A word no label has (a label value alone becomes key=value, #88).
    type_text(&mut m, "nolabel");
    press(&mut m, kc(KeyCode::Enter));
    match &m.overlay {
        Overlay::RuleFilter(e) => assert!(e.error.as_deref().unwrap().contains("unknown term")),
        other => panic!("{other:?}"),
    }
    assert_eq!(found_hrefs(&m).len(), 2, "the search in force holds");
    press(&mut m, kc(KeyCode::Esc));
    search(&mut m, "ipl:Crop");
    assert!(m.policy.found.is_empty());
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert!(
        m.status
            .text
            .ends_with("· unknown IP list Crop (closest: Corp)"),
        "{}",
        m.status.text
    );
    // An empty search clears it.
    search(&mut m, "");
    assert!(m.policy.search.is_none());
    assert_eq!(m.status.text, "rule search cleared");
}

#[test]
fn enter_opens_the_rule_in_its_ruleset_and_f_offers_the_search_again() {
    let mut m = on_policy();
    search(&mut m, "env=prod");
    press(&mut m, kc(KeyCode::End));
    // Rule 3 of Global-Infra: opening it asks for that ruleset's deny rules.
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert!(
        matches!(cmds.as_slice(), [Cmd::LoadDenyRules { href, .. }] if href.ends_with("/101")),
        "{cmds:?}"
    );
    assert!(m.policy.search.is_none());
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    assert_eq!(
        m.policy.selected_href().as_deref(),
        Some("/orgs/5/sec_policy/draft/rule_sets/101")
    );
    assert_eq!(
        m.policy.rules.selected_item().map(|r| r.href.as_str()),
        Some("/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3")
    );
    assert!(
        m.status.text.starts_with("Global-Infra"),
        "{}",
        m.status.text
    );
    press(&mut m, key('f'));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "env=prod"));
    press(&mut m, kc(KeyCode::Enter));
    assert_eq!(found_hrefs(&m).len(), 5);
    // c clears; the panes are back.
    press(&mut m, key('c'));
    assert!(m.policy.search.is_none() && m.policy.found.rows().is_empty());
    assert_eq!(m.policy.last_search, "env=prod");
}

#[test]
fn ctrl_n_completes_names_and_ctrl_u_clears() {
    let mut m = with_workloads(on_policy());
    press(&mut m, key('f'));
    type_text(&mut m, "ipl:");
    press(&mut m, ctrl('n'));
    assert!(
        matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "ipl:\"Any (0.0.0.0/0 and ::/0)\""),
        "{:?}",
        m.overlay
    );
    press(&mut m, ctrl('n'));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "ipl:Corp"));
    press(&mut m, ctrl('u'));
    type_text(&mut m, "wl:host-1");
    press(&mut m, ctrl('n'));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "wl:host-1.example"));
    press(&mut m, ctrl('u'));
    type_text(&mut m, "dis");
    press(&mut m, ctrl('n'));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "disabled"));
    press(&mut m, kc(KeyCode::Char(' ')));
    type_text(&mut m, "app=sh");
    press(&mut m, ctrl('n'));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "disabled app=shop"));
    press(&mut m, kc(KeyCode::Enter));
    assert_eq!(found_hrefs(&m), Vec::<String>::new());
}

#[test]
fn the_search_survives_a_scope_switch_and_follows_the_reload() {
    let mut m = on_policy();
    search(&mut m, "role=web");
    let cmds = update(&mut m, key('d'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::LoadRuleSets { .. }]),
        "{cmds:?}"
    );
    assert!(m.policy.search.is_some());
    assert!(m.policy.found.rows().is_empty(), "nothing of the old scope");
    let raw: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    let idx = m.index().cloned().unwrap();
    m.policy.set_rule_sets(raw, &idx, None);
    assert_eq!(found_hrefs(&m).len(), 2);
}

#[test]
fn t_on_the_search_list_toggles_the_rule_and_tab_says_how_to_leave() {
    let mut m = on_policy();
    m.writes = WritesState::on();
    search(&mut m, "deny");
    assert_eq!(
        update(&mut m, key('t')),
        vec![Cmd::PlanWrite(WriteIntent::UpdateDenyRule {
            href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
            patch: pce::write::DenyRulePatch {
                enabled: Some(false),
                ..Default::default()
            },
        })]
    );
    m.writes.in_flight = false;
    for c in [kc(KeyCode::Tab), key('e'), key('x'), key('n')] {
        press(&mut m, c);
        assert_eq!(
            m.status.text,
            "Enter opens the rule in its ruleset · c clears the search"
        );
        assert_eq!(m.overlay, Overlay::None);
    }
    // `/` searches the list itself; `c` typed there is text.
    press(&mut m, key('/'));
    press(&mut m, key('c'));
    assert!(m.policy.search.is_some());
    assert_eq!(m.policy.found.search_text(), "c");
}

#[test]
fn export_writes_the_search_list() {
    let mut m = on_policy();
    search(&mut m, "role=web");
    let (headers, rows) = crate::tui::export::current_table(&m).unwrap();
    assert_eq!(
        headers,
        [
            "ruleset",
            "enabled",
            "kind",
            "scope",
            "consumers",
            "providers",
            "services",
            "href"
        ]
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0], "Shop-Prod");
    assert_eq!(
        rows[0][7],
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"
    );
}

#[test]
fn names_resolve_by_a_unique_prefix() {
    let mut m = on_policy();
    search(&mut m, "ipl:any");
    assert_eq!(found_hrefs(&m), ["7<deny_rules"]);
    assert_eq!(m.policy.last_search, "ipl:\"Any (0.0.0.0/0 and ::/0)\"");
    assert_eq!(m.status.kind, StatusKind::Info);
}

#[test]
fn wl_names_without_cached_workloads_warn() {
    let mut m = on_policy();
    search(&mut m, "wl:host-1.example");
    assert!(m.policy.found.is_empty());
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert!(
        m.status
            .text
            .ends_with("wl: names need the workloads (i loads them)"),
        "{}",
        m.status.text
    );
    // With them, no warning.
    let mut m = with_workloads(on_policy());
    search(&mut m, "wl:host-1.example");
    assert_eq!(m.status.kind, StatusKind::Info);
}

#[test]
fn deny_rules_may_be_unloaded_only_where_the_pce_keeps_them_apart() {
    let mut m = on_policy();
    // No ruleset lists deny rules inline: an older PCE may keep them apart.
    for rs in &mut m.policy.raw {
        rs.deny_rules.clear();
    }
    search(&mut m, "role=web");
    assert_eq!(
        m.status.text,
        "2 of 3 rules · role=web · 2 rulesets may hold deny rules not loaded"
    );
    // The endpoint answers 404: there is none, an empty list means none.
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            result: Err("not found".into()),
        },
    );
    assert!(m.policy.deny_endpoint_missing);
    assert_eq!(m.policy.found_deny_unloaded, 0);
}

#[test]
fn the_enabled_column_is_the_rules_flag_and_its_rulesets() {
    let mut m = on_policy();
    search(&mut m, "svc:443/tcp");
    let cells: Vec<String> = m
        .policy
        .found
        .rows()
        .iter()
        .map(|r| crate::tui::model::enabled_text(r.enabled, r.ruleset_enabled, false))
        .collect();
    // Rule 2 of Shop-Prod; rule 3, on, of the disabled Global-Infra.
    assert_eq!(cells, ["on", "on (ruleset off)"]);
    let (_, rows) = crate::tui::export::current_table(&m).unwrap();
    assert_eq!(rows[1][1], "on (ruleset off)");
}

#[test]
fn question_mark_in_the_editor_shows_the_help_and_comes_back() {
    let mut m = on_policy();
    press(&mut m, key('f'));
    type_text(&mut m, "role=web");
    press(&mut m, key('?'));
    assert_eq!(m.overlay, Overlay::Help);
    press(&mut m, kc(KeyCode::Esc));
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "role=web"));
    // Inside a quoted name `?` is text.
    type_text(&mut m, " ipl:\"a?");
    assert!(matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == "role=web ipl:\"a?"));
}

#[test]
fn explains_jump_puts_the_search_aside_to_show_the_rule() {
    let mut m = on_policy();
    search(&mut m, "deny");
    assert!(m.policy.search.is_some());
    let jump = explain::select_rule(
        &mut m,
        "/orgs/5/sec_policy/draft/rule_sets/100",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2",
    );
    assert_eq!(jump, explain::Jump::Done);
    assert!(m.policy.search.is_none() && m.policy.found.is_empty());
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    assert_eq!(m.policy.last_search, "deny", "f offers it again");
}

#[test]
fn sides_exclusions_negation_and_properties_search_locally() {
    let mut m = on_policy();
    search(&mut m, "src:role=web");
    assert_eq!(found_hrefs(&m), ["1<sec_rules"]);
    search(&mut m, "dst:role=web extra");
    assert_eq!(found_hrefs(&m), ["2<sec_rules"]);
    search(&mut m, "scope:app=shop -deny");
    assert_eq!(found_hrefs(&m), ["1<sec_rules", "2<sec_rules"]);
    assert_eq!(m.status.text, "2 of 5 rules · scope:app=shop -deny");
    search(&mut m, "consumer:all");
    assert_eq!(found_hrefs(&m), ["8<deny_rules"]);
    assert_eq!(m.policy.last_search, "src:all");
    search(&mut m, "scope:global enabled");
    assert!(found_hrefs(&m).is_empty(), "its ruleset is off");
    search(&mut m, "scope:global intra");
    assert_eq!(found_hrefs(&m), ["3<sec_rules"]);
    search(&mut m, "src:!env=prod");
    assert!(found_hrefs(&m).is_empty());
    // A malformed negation stays in the editor; the search in force holds.
    press(&mut m, key('f'));
    press(&mut m, ctrl('u'));
    type_text(&mut m, "--deny");
    press(&mut m, kc(KeyCode::Enter));
    match &m.overlay {
        Overlay::RuleFilter(e) => assert!(e.error.as_deref().unwrap().contains("one - negates")),
        other => panic!("{other:?}"),
    }
    assert_eq!(m.policy.last_search, "src:!env=prod");
}

#[test]
fn ctrl_n_completes_after_a_side_a_bang_and_a_dash() {
    let mut m = with_workloads(on_policy());
    for (typed, completed) in [
        ("src:role=w", "src:role=web"),
        ("-scope:app=sh", "-scope:app=shop"),
        ("dst:ipl:C", "dst:ipl:Corp"),
        ("src:!role=w", "src:!role=web"),
        ("!env=p", "!env=prod"),
        ("scope:gl", "scope:global"),
        ("dst:al", "dst:all"),
        ("provider:wl:host-1", "provider:wl:host-1.example"),
        ("-ext", "-extra"),
        ("intr", "intra"),
    ] {
        press(&mut m, key('f'));
        press(&mut m, ctrl('u'));
        type_text(&mut m, typed);
        press(&mut m, ctrl('n'));
        assert!(
            matches!(&m.overlay, Overlay::RuleFilter(e) if e.value == completed),
            "{typed}: {:?}",
            m.overlay
        );
        press(&mut m, kc(KeyCode::Esc));
        press(&mut m, kc(KeyCode::Esc));
    }
}
