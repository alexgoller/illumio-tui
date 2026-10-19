//! `E`: the Explain overlay's keys (issue #50).
use super::*;
use crate::tui::app_view::AppPanel;
use crate::tui::explain::{DstEnforcement, ExplainView, PolicySource};
use crate::tui::model::{Model, Overlay, PendingAction, PolicyPane, TrafficView, WritesState};
use crate::tui::test_support::{model_with_flows, with_policy, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::explain::MatchKind;
use pce::models::DraftDecision;

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

const ALLOW: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2";
const RS: &str = "/orgs/5/sec_policy/draft/rule_sets/100";
const HREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

/// The fixture flows as a result of our own query `HREF`, the cursor on
/// `dst` (`fqdn-1.example`: the allowed flow whose `rules` names
/// `ALLOW`; `host-1.example`: the potentially blocked one).
fn model_on(dst: &str) -> Model {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
    select(&mut m, dst);
    m
}

fn select(m: &mut Model, dst: &str) {
    m.traffic
        .flows
        .select_where(|r| r.flow.dst.display_name() == dst);
}

/// `m` after the draft evaluation: the fqdn flow allowed by `ALLOW`, the
/// others at their decision with no rule.
fn evaluated(mut m: Model) -> Model {
    let dst = m
        .traffic
        .selected_flow()
        .map(|r| r.flow.dst.display_name())
        .unwrap();
    let d = m.traffic.data.as_mut().unwrap();
    for f in &mut d.flows {
        f.draft_policy_decision = Some(DraftDecision::parse(f.policy_decision.as_str()));
        if f.dst.display_name() != "fqdn-1.example" {
            f.rules.clear();
        }
    }
    d.draft = Some(pce::traffic::DraftSummary::of(&d.flows));
    m.traffic.rebuild_views();
    select(&mut m, &dst);
    m
}

fn view(m: &Model) -> &ExplainView {
    match &m.overlay {
        Overlay::Explain(v) => v,
        o => panic!("not the explain overlay: {o:?}"),
    }
}

#[test]
fn e_explains_the_selected_flow_with_no_request() {
    let mut m = evaluated(with_policy(model_on("fqdn-1.example")));
    assert_eq!(update(&mut m, key('E')), Vec::<Cmd>::new());
    let v = view(&m);
    assert_eq!(v.flow.dst.display_name(), "fqdn-1.example");
    assert_eq!(v.policy, PolicySource::Draft);
    assert_eq!(v.explanation.verdict, "allowed by 1 rule (draft)");
    let r = &v.explanation.rules[0];
    assert_eq!((r.kind, r.resolved), (MatchKind::Allow, true));
    assert_eq!(r.ruleset.as_deref(), Some("Shop-Prod"));
    assert_eq!(v.dst, DstEnforcement::NotWorkload);
    assert!(m.status.text.contains("no request"), "{}", m.status.text);
    // Esc: back to the screen.
    assert_eq!(update(&mut m, kc(KeyCode::Esc)), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn not_evaluated_d_asks_before_the_draft_evaluation() {
    let mut m = with_workloads(model_on("host-1.example"));
    update(&mut m, key('E'));
    let v = view(&m);
    assert!(!v.explanation.evaluated);
    assert!(matches!(v.dst, DstEnforcement::Mode(_)), "{:?}", v.dst);
    // D: one confirm, nothing sent yet.
    assert_eq!(update(&mut m, key('D')), Vec::<Cmd>::new());
    let Overlay::Confirm { reason, action } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert!(
        reason.contains("1 status check, 1 PUT, polls and 1 download"),
        "{reason}"
    );
    assert!(
        matches!(action, PendingAction::RunDraft { hrefs, .. } if hrefs == &[HREF.to_string()])
    );
    // y: the existing draft run.
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunDraft { hrefs, .. }] if hrefs == &[HREF.to_string()]),
        "{cmds:?}"
    );
}

#[test]
fn d_on_an_evaluated_flow_sends_nothing() {
    let mut m = evaluated(model_on("host-1.example"));
    update(&mut m, key('E'));
    assert_eq!(view(&m).explanation.verdict, "no rule allows this flow");
    assert_eq!(update(&mut m, key('D')), Vec::<Cmd>::new());
    assert!(matches!(m.overlay, Overlay::Explain(_)));
    assert!(m.status.text.contains("already evaluated"));
}

#[test]
fn w_from_explain_is_the_w_form_and_refused_with_writes_off() {
    let mut m = evaluated(with_policy(model_on("host-1.example")));
    update(&mut m, key('E'));
    assert_eq!(update(&mut m, key('w')), Vec::<Cmd>::new());
    assert!(
        matches!(m.overlay, Overlay::Explain(_)),
        "refused: the overlay stays"
    );
    m.writes = WritesState::on();
    update(&mut m, key('w'));
    assert!(matches!(m.overlay, Overlay::Form(_)), "{:?}", m.overlay);
}

#[test]
fn enter_jumps_to_the_rule_in_the_loaded_draft_policy() {
    let mut m = evaluated(with_policy(model_on("fqdn-1.example")));
    update(&mut m, key('E'));
    assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.screen, Screen::Policy);
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    assert_eq!(
        m.policy.rulesets.selected_item().map(|r| r.href.as_str()),
        Some(RS)
    );
    assert_eq!(
        m.policy.rules.selected_item().map(|r| r.href.as_str()),
        Some(ALLOW)
    );
    assert!(
        m.status.text.starts_with("Shop-Prod: allow"),
        "{}",
        m.status.text
    );
}

#[test]
fn enter_before_the_policy_is_loaded_selects_the_rule_when_it_lands() {
    let mut m = evaluated(model_on("fqdn-1.example"));
    update(&mut m, key('E'));
    assert_eq!(view(&m).policy, PolicySource::NotLoaded);
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use
        }],
        "the Policy screen's own load"
    );
    let rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, 0, false)),
        },
    );
    assert_eq!(
        m.policy.rules.selected_item().map(|r| r.href.as_str()),
        Some(ALLOW)
    );
    assert_eq!(m.policy.jump, None, "once");
}

#[test]
fn enter_from_the_active_policy_switches_to_the_draft() {
    let mut m = evaluated(with_policy(model_on("fqdn-1.example")));
    m.policy.scope = PolicyScope::Active;
    update(&mut m, key('E'));
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use
        }]
    );
    assert_eq!(m.policy.scope, PolicyScope::Draft);
    assert_eq!(m.screen, Screen::Policy);
    assert_eq!(m.policy.jump.as_ref().map(|j| j.1.as_str()), Some(ALLOW));
}

#[test]
fn a_deny_rule_loaded_on_demand_is_selected_when_it_lands() {
    let mut m = evaluated(with_policy(model_on("host-1.example")));
    // Global-Infra carries no inline deny rules: they come on demand.
    let deny = "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/5";
    let d = m.traffic.data.as_mut().unwrap();
    for f in &mut d.flows {
        f.rules = vec![deny.into()];
    }
    m.traffic.rebuild_views();
    select(&mut m, "host-1.example");
    update(&mut m, key('E'));
    assert_eq!(view(&m).explanation.rules[0].kind, MatchKind::Deny);
    let cmds = update(&mut m, kc(KeyCode::Enter));
    let rs = "/orgs/5/sec_policy/draft/rule_sets/101";
    assert_eq!(
        cmds,
        vec![Cmd::LoadDenyRules {
            href: rs.into(),
            mode: CacheMode::Use
        }]
    );
    let rule: pce::models::DenyRule = serde_json::from_value(serde_json::json!({
        "href": deny, "enabled": true, "providers": [{"actors": "ams"}],
        "consumers": [{"actors": "ams"}], "ingress_services": [{"port": 23, "proto": 6}]
    }))
    .unwrap();
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: rs.into(),
            result: Ok(vec![rule]),
        },
    );
    assert_eq!(
        m.policy.rules.selected_item().map(|r| r.href.as_str()),
        Some(deny)
    );
    assert_eq!(m.policy.jump, None);
}

#[test]
fn e_in_the_app_overlay_explains_a_peer_row_and_esc_goes_back() {
    let mut m = evaluated(with_policy(with_workloads(model_on("fqdn-1.example"))));
    update(&mut m, key('A'));
    assert_eq!(m.overlay, Overlay::App);
    let v = m.app_view.as_mut().unwrap();
    v.panel = AppPanel::Outbound;
    let i = v
        .outbound
        .iter()
        .position(|p| p.service == "443/tcp")
        .expect("the fqdn peer");
    v.cursor[AppPanel::Outbound.index()] = i;
    assert_eq!(update(&mut m, key('E')), Vec::<Cmd>::new());
    assert!(view(&m).from_app);
    assert_eq!(view(&m).explanation.rules.len(), 1);
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::App, "back where it came from");
}

#[test]
fn e_on_a_pair_explains_only_a_single_flow() {
    let mut m = evaluated(model_on("fqdn-1.example"));
    m.traffic.view = TrafficView::Aggregate;
    m.traffic.rebuild_views();
    let rows = m.traffic.pairs_table().rows();
    let many = rows.iter().position(|g| g.flows > 1);
    let one = rows
        .iter()
        .position(|g| g.flows == 1)
        .expect("a pair of one flow");
    let to = |m: &mut Model, i: usize| {
        let t = m.traffic.pairs_table_mut();
        t.home();
        for _ in 0..i {
            t.select_next();
        }
    };
    if let Some(many) = many {
        let flows = m.traffic.pairs_table().rows()[many].flows;
        to(&mut m, many);
        assert_eq!(update(&mut m, key('E')), Vec::<Cmd>::new());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(
            m.status.text,
            format!("{flows} flows behind this pair: Enter, then E on one")
        );
    }
    to(&mut m, one);
    update(&mut m, key('E'));
    assert!(
        matches!(m.overlay, Overlay::Explain(_)),
        "{}",
        m.status.text
    );
}

#[test]
fn e_elsewhere_points_to_the_flows_view() {
    let mut m = model_on("fqdn-1.example");
    m.traffic.view = TrafficView::Mesh;
    assert_eq!(update(&mut m, key('E')), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.status.text.contains("Flows view"));
}

#[test]
fn j_and_k_move_between_rules() {
    let mut m = evaluated(with_policy(model_on("fqdn-1.example")));
    let d = m.traffic.data.as_mut().unwrap();
    for f in &mut d.flows {
        f.rules = vec![ALLOW.into(), "essential:dhcp".into()];
    }
    m.traffic.rebuild_views();
    select(&mut m, "fqdn-1.example");
    update(&mut m, key('E'));
    update(&mut m, key('j'));
    assert_eq!(view(&m).selected, 1);
    // An essential rule has no ruleset to open.
    assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::<Cmd>::new());
    assert!(matches!(m.overlay, Overlay::Explain(_)));
    assert!(m.status.text.contains("essential"));
    update(&mut m, key('k'));
    assert_eq!(view(&m).selected, 0);
}

#[test]
fn an_unresolved_override_deny_from_its_side_field_stays_one_and_offers_no_w() {
    let mut m = evaluated(with_policy(model_on("host-1.example")));
    let od = "/orgs/5/sec_policy/draft/rule_sets/999/deny_rules/1";
    let d = m.traffic.data.as_mut().unwrap();
    for f in &mut d.flows {
        f.rules = vec![od.into()];
        f.side_rules = vec![(od.into(), MatchKind::OverrideDeny)];
    }
    m.traffic.rebuild_views();
    select(&mut m, "host-1.example");
    m.writes = WritesState::on();
    update(&mut m, key('E'));
    let r = &view(&m).explanation.rules[0];
    assert_eq!((r.kind, r.resolved), (MatchKind::OverrideDeny, false));
    assert!(!view(&m).offers_rule());
    update(&mut m, key('w'));
    assert!(matches!(m.overlay, Overlay::Explain(_)), "no form");
    assert!(m.status.text.contains("override deny"), "{}", m.status.text);
}

#[test]
fn d_refuses_a_flow_that_is_not_in_the_result_on_screen() {
    let mut m = model_on("host-1.example");
    let mut other = m.traffic.selected_flow().unwrap().flow.as_ref().clone();
    other.port = Some(9999);
    let v = explain_view(&m, other, true);
    assert!(!v.on_screen);
    m.overlay = Overlay::Explain(Box::new(v));
    assert_eq!(update(&mut m, key('D')), Vec::<Cmd>::new());
    assert!(matches!(m.overlay, Overlay::Explain(_)), "{:?}", m.overlay);
    assert!(m.status.text.contains("not in the result on screen"));
}

#[test]
fn n_on_the_draft_confirm_goes_back_to_explain_and_y_back_to_the_app_overlay() {
    let mut m = with_policy(with_workloads(model_on("host-1.example")));
    update(&mut m, key('E'));
    update(&mut m, key('D'));
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    update(&mut m, kc(KeyCode::Esc));
    assert!(matches!(m.overlay, Overlay::Explain(_)), "{:?}", m.overlay);
    // From the app overlay: y runs it and goes back there.
    update(&mut m, kc(KeyCode::Esc));
    update(&mut m, key('A'));
    assert_eq!(m.overlay, Overlay::App);
    let flow = m.traffic.selected_flow().unwrap().flow.as_ref().clone();
    m.overlay = Overlay::Explain(Box::new(explain_view(&m, flow, true)));
    update(&mut m, key('D'));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunDraft { .. }]),
        "{cmds:?}"
    );
    assert_eq!(m.overlay, Overlay::App);
    assert!(m.explain_back.is_none());
}

#[test]
fn a_pending_jump_is_dropped_by_d_leaving_policy_and_a_failed_load() {
    let rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    let landed = |rs: &Vec<pce::models::RuleSet>| Msg::RuleSetsLoaded {
        scope: PolicyScope::Draft,
        result: Ok((rs.clone(), 0, false)),
    };
    // Leaving the Policy screen.
    let mut m = evaluated(model_on("fqdn-1.example"));
    update(&mut m, key('E'));
    update(&mut m, kc(KeyCode::Enter));
    assert!(m.policy.jump.is_some());
    update(&mut m, key('1'));
    assert_eq!(m.policy.jump, None);
    update(&mut m, landed(&rs));
    assert_ne!(
        m.policy.rules.selected_item().map(|r| r.href.as_str()),
        Some(ALLOW),
        "no jump after leaving"
    );
    // `d` on Policy (a scope change), even while the load runs.
    let mut m = evaluated(model_on("fqdn-1.example"));
    update(&mut m, key('E'));
    update(&mut m, kc(KeyCode::Enter));
    update(&mut m, key('d'));
    assert_eq!(m.policy.jump, None);
    update(&mut m, landed(&rs));
    assert_ne!(
        m.policy.rules.selected_item().map(|r| r.href.as_str()),
        Some(ALLOW)
    );
    // A failed load.
    let mut m = evaluated(model_on("fqdn-1.example"));
    update(&mut m, key('E'));
    update(&mut m, kc(KeyCode::Enter));
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Err("boom".into()),
        },
    );
    assert_eq!(m.policy.jump, None);
}

#[test]
fn deny_rules_of_another_ruleset_leave_the_jump_waiting() {
    let mut m = evaluated(with_policy(model_on("host-1.example")));
    let deny = "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/5";
    m.policy.jump = Some(("/orgs/5/sec_policy/draft/rule_sets/101".into(), deny.into()));
    m.screen = Screen::Policy;
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/555".into(),
            result: Ok(Vec::new()),
        },
    );
    assert!(m.policy.jump.is_some(), "not its ruleset");
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            result: Err("boom".into()),
        },
    );
    assert_eq!(m.policy.jump, None, "its ruleset failed");
}

#[test]
fn with_only_the_active_policy_loaded_rules_resolve_through_it() {
    let mut m = evaluated(with_policy(model_on("fqdn-1.example")));
    m.policy.scope = PolicyScope::Active;
    update(&mut m, key('E'));
    let v = view(&m);
    assert_eq!(v.policy, PolicySource::Active);
    assert!(v.explanation.rules[0].resolved);
    assert!(
        v.rows().iter().any(|r| matches!(
            r,
            crate::tui::explain::Row::Rule { detail, .. } if detail.starts_with("ruleset Shop-Prod (active)")
        )),
        "{:?}",
        v.rows()
    );
}
