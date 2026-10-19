use super::*;
use crate::tui::model::{Model, Overlay};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::model_with_policy;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::write::{DenyRulePatch, RulePatch, RuleSetPatch};

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn key(c: char) -> Msg {
    kc(KeyCode::Char(c))
}

fn on_policy(writes: bool) -> Model {
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    if writes {
        m.writes = WritesState::on();
    }
    m
}

#[test]
fn t_on_the_rulesets_pane_plans_the_ruleset_flipped() {
    let mut m = on_policy(true);
    assert_eq!(
        update(&mut m, key('t')),
        vec![Cmd::PlanWrite(WriteIntent::UpdateRuleSet {
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            patch: RuleSetPatch {
                enabled: Some(false),
                ..Default::default()
            },
        })]
    );
    assert!(m.writes.in_flight);
    assert_eq!(m.status.text, "planning…");
    // The disabled ruleset is enabled again.
    let mut m = on_policy(true);
    update(&mut m, key('j'));
    assert_eq!(
        update(&mut m, key('t')),
        vec![Cmd::PlanWrite(WriteIntent::UpdateRuleSet {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            patch: RuleSetPatch {
                enabled: Some(true),
                ..Default::default()
            },
        })]
    );
}

#[test]
fn t_on_the_rules_pane_plans_an_allow_or_a_deny_rule_by_kind() {
    let mut m = on_policy(true);
    update(&mut m, kc(KeyCode::Tab));
    // Row 0 is the override deny rule 8 (evaluation order).
    assert_eq!(
        update(&mut m, key('t')),
        vec![Cmd::PlanWrite(WriteIntent::UpdateDenyRule {
            href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
            patch: DenyRulePatch {
                enabled: Some(false),
                ..Default::default()
            },
        })]
    );
    m.writes.in_flight = false;
    update(&mut m, key('j'));
    assert_eq!(
        update(&mut m, key('t')),
        vec![Cmd::PlanWrite(WriteIntent::UpdateRule {
            href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
            patch: RulePatch {
                enabled: Some(false),
                ..Default::default()
            },
        })]
    );
}

#[test]
fn t_goes_through_the_write_refusals() {
    let mut m = on_policy(false);
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
    let mut m = on_policy(true);
    m.policy.scope = PolicyScope::Active;
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.status.text, "switch to draft to edit policy");
    let mut m = on_policy(true);
    m.writes.in_flight = true;
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.status.text, "a write is already in progress");
    let mut m = on_policy(true);
    update(&mut m, kc(KeyCode::Tab));
    m.policy.rules.set_rows(Vec::new());
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.status.text, "no row selected");
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn t_types_into_an_active_table_search() {
    let mut m = on_policy(true);
    update(&mut m, key('/'));
    let cmds = update(&mut m, key('t'));
    assert!(
        !cmds.iter().any(|c| matches!(c, Cmd::PlanWrite(_))),
        "{cmds:?}"
    );
    assert!(!m.writes.in_flight);
    assert_eq!(m.policy.rulesets.search_text(), "t");
}
