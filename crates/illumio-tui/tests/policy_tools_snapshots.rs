//! Snapshots of the Policy tools: the rule search list and its editor
//! (#51), the rule impact's workload list (#53).
use illumio_tui::tui::model::{Overlay, Screen};
use illumio_tui::tui::rule_search::RuleFilterEditor;
use illumio_tui::tui::test_support::{model_with_policy, render_to_string};

fn searched(expr: &str) -> illumio_tui::tui::Model {
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.policy.search = Some(pce::RuleFilter::parse(expr).unwrap());
    m.rebuild_rules();
    m
}

#[test]
fn policy_search_list_80x24_and_160x48() {
    let m = searched("env=prod");
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Search · env=prod"), "{out}");
    assert!(out.contains("Global-Infra"), "{out}");
    insta::assert_snapshot!("policy_search_80x24", out);
    insta::assert_snapshot!("policy_search_160x48", render_to_string(&m, 160, 48));
}

#[test]
fn policy_search_editor_80x24() {
    let mut m = searched("env=prod");
    let mut e = RuleFilterEditor::new("app=shop ipl:Crop");
    e.unknown = vec!["unknown IP list Crop".into()];
    m.overlay = Overlay::RuleFilter(e);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Rule search"), "{out}");
    insta::assert_snapshot!("policy_search_editor_80x24", out);
}
