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

fn key(code: crossterm::event::KeyCode) -> illumio_tui::tui::Msg {
    illumio_tui::tui::Msg::Key(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ))
}

/// Policy with the workloads cached, the rules pane on row `row`, `I`.
fn matched(row: usize) -> illumio_tui::tui::Model {
    use crossterm::event::KeyCode;
    use illumio_tui::tui::test_support::with_workloads;
    use illumio_tui::tui::update;
    let mut m = with_workloads(model_with_policy());
    m.screen = Screen::Policy;
    update(&mut m, key(KeyCode::Tab));
    for _ in 0..row {
        update(&mut m, key(KeyCode::Down));
    }
    update(&mut m, key(KeyCode::Char('I')));
    assert_eq!(m.overlay, Overlay::Matched);
    m
}

#[test]
fn policy_matched_workloads_80x24() {
    use crossterm::event::KeyCode;
    let mut m = matched(0);
    illumio_tui::tui::update(&mut m, key(KeyCode::Tab));
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("host-1.example") && out.contains("host-2.example"),
        "{out}"
    );
    insta::assert_snapshot!("policy_matched_80x24", out);
}

#[test]
fn policy_matched_ip_list_side_120x30() {
    use crossterm::event::KeyCode;
    let mut m = matched(2);
    illumio_tui::tui::update(&mut m, key(KeyCode::Tab));
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains("nothing to list"), "{out}");
    insta::assert_snapshot!("policy_matched_ip_lists_120x30", out);
}
