use super::*;
use crate::tui::app_view::{AppPanel, Loadable};
use crate::tui::model::{Model, Overlay, TrafficView, WritesState};
use crate::tui::msg::AppWorkloadsPayload;
use crate::tui::test_support::{model_with_flows, with_policy, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::models::Workload;
use pce::traffic::FocusDirection;

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

/// Every command that reaches the PCE (all but writes-off, export, quit).
fn requests(cmds: &[Cmd]) -> u32 {
    cmds.iter()
        .filter(|c| !matches!(c, Cmd::SetWrites(_) | Cmd::Export { .. } | Cmd::Quit))
        .count() as u32
}

/// The fixture rulesets with Global-Infra (no inline deny rules) first:
/// selected on the Policy screen, it would ask for its deny rules.
fn rulesets_answer(hit: bool) -> Msg {
    let mut rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    rs.reverse();
    assert!(rs[0].deny_rules.is_empty());
    Msg::RuleSetsLoaded {
        scope: PolicyScope::Draft,
        result: Ok((rs, u64::from(hit) * 3, hit)),
    }
}

fn view(m: &Model) -> &crate::tui::app_view::AppView {
    m.app_view.as_ref().expect("the overlay's state")
}

fn shop_labels() -> Vec<String> {
    vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]
}

fn fixture_workloads() -> Vec<Workload> {
    serde_json::from_str(include_str!("../../../../../fixtures/workloads.json")).unwrap()
}

/// Flows, workloads and rulesets all loaded: the overlay costs nothing.
fn everything() -> Model {
    with_policy(with_workloads(model_with_flows()))
}

#[test]
fn a_opens_the_overlay_for_the_row_app_with_at_most_two_requests() {
    let mut m = model_with_flows();
    m.traffic
        .flows
        .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
    let cmds = update(&mut m, key('A'));
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadAppWorkloads {
                seq: 1,
                filter: pce::app::app_workload_filter(&shop_labels()),
                mode: CacheMode::Use,
            },
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use,
            },
        ]
    );
    assert_eq!(m.overlay, Overlay::App);
    let v = view(&m);
    assert_eq!(v.requests, 2);
    assert_eq!(v.focus.app_text(), "shop/prod");
    assert_eq!(v.focus.direction, FocusDirection::Both, "no focus: both");
    assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
    assert_eq!(v.header_title(), "shop/prod · in+out · last 24h");
    assert!(matches!(v.workloads, Loadable::Loading) && matches!(v.policy, Loadable::Loading));
}

#[test]
fn loaded_screens_make_the_overlay_free() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    let v = view(&m);
    assert_eq!(v.requests, 0);
    assert_eq!(v.header_meta(), "0 req · cached");
    let Loadable::Loaded((rows, s)) = &v.workloads else {
        panic!("{:?}", v.workloads)
    };
    assert_eq!(
        rows.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
        vec!["host-1.example", "host-2.example"]
    );
    assert_eq!(s.enforcement.text(), "enforced 1 · visibility 1");
    assert_eq!(s.ven_text(), "1 active · 1 stale");
    let Loadable::Loaded(p) = &v.policy else {
        panic!("{:?}", v.policy)
    };
    assert_eq!(
        p.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["Shop-Prod", "Global-Infra"]
    );
}

#[test]
fn the_overlay_takes_the_focus_direction_and_x_cycles_it_without_requests() {
    let mut m = everything();
    quiet(
        &mut m,
        Msg::Command(Command::Focus(Some("shop/prod".into()))),
    );
    quiet(&mut m, key('X')); // in
    quiet(&mut m, key('A'));
    assert_eq!(view(&m).focus.direction, FocusDirection::In);
    assert_eq!(view(&m).outbound.len(), 0);
    quiet(&mut m, key('X'));
    assert_eq!(view(&m).focus.direction, FocusDirection::Out);
    assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (0, 2));
    assert_eq!(
        m.traffic.focus.as_ref().unwrap().direction,
        FocusDirection::In,
        "the overlay's X leaves the traffic focus alone"
    );
}

#[test]
fn esc_closes_and_tab_and_j_k_move_without_requests() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    quiet(&mut m, key('j'));
    assert_eq!(view(&m).cursor[0], 1);
    quiet(&mut m, kc(KeyCode::Tab));
    assert_eq!(view(&m).panel, AppPanel::Outbound);
    quiet(&mut m, kc(KeyCode::BackTab));
    assert_eq!(view(&m).panel, AppPanel::Inbound);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.screen, Screen::Traffic);
    assert_eq!(
        m.traffic.view,
        TrafficView::Flows,
        "back to the view it was opened from"
    );
}

#[test]
fn enter_on_a_peer_focuses_that_pair_in_flows() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    quiet(&mut m, kc(KeyCode::Enter)); // inbound row 0: shop/prod -> shop/prod 5432/tcp
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(
        m.traffic.focus.as_ref().map(|f| f.app_text()).as_deref(),
        Some("shop/prod")
    );
    assert_eq!(
        m.traffic.drill,
        Some(Drill::Service(
            vec!["shop".into(), "prod".into()],
            vec!["shop".into(), "prod".into()],
            "5432/tcp".into(),
            Decisions::All
        ))
    );
    assert_eq!(m.traffic.flows.len(), 1);
}

#[test]
fn enter_on_a_workload_or_a_ruleset_opens_it_on_its_screen() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    for _ in 0..3 {
        quiet(&mut m, kc(KeyCode::Tab));
    }
    assert_eq!(view(&m).panel, AppPanel::Workloads);
    quiet(&mut m, key('j'));
    let _ = update(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.screen, Screen::Workloads);
    assert!(m.workloads.show_detail);
    assert_eq!(
        m.workloads
            .table
            .selected_item()
            .map(|r| r.workload.href.as_str()),
        Some("/orgs/5/workloads/aaaa-2")
    );
    m.screen = Screen::Traffic;
    quiet(&mut m, key('A'));
    assert_eq!(
        view(&m).panel,
        AppPanel::Workloads,
        "reopened where it was left"
    );
    quiet(&mut m, kc(KeyCode::BackTab));
    // Policy's own selection sits on a ruleset left to the fallback.
    m.policy.rulesets.select_where(|r| r.name != "Shop-Prod");
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.screen, Screen::Policy);
    let chosen = m.policy.rulesets.selected_item().map(|r| r.href.clone());
    assert_eq!(
        m.policy.rulesets.selected_item().map(|r| r.name.as_str()),
        Some("Shop-Prod")
    );
    // No deny rules fetched for the row selected before Enter.
    for c in &cmds {
        if let Cmd::LoadDenyRules { href, .. } = c {
            assert_eq!(Some(href), chosen.as_ref(), "{cmds:?}");
        }
    }
}

#[test]
fn entering_policy_during_a_rulesets_reload_waits_for_its_answer() {
    let mut m = everything();
    m.policy.rulesets.select_where(|r| r.name != "Shop-Prod");
    m.policy.loading = true;
    let cmds = switch(&mut m, Screen::Policy);
    assert!(
        !cmds.iter().any(|c| matches!(c, Cmd::LoadDenyRules { .. })),
        "{cmds:?}"
    );
    m.policy.loading = false;
    m.screen = Screen::Traffic;
    let cmds = switch(&mut m, Screen::Policy);
    assert!(
        cmds.iter().any(|c| matches!(c, Cmd::LoadDenyRules { .. })),
        "{cmds:?}"
    );
}

#[test]
fn w_drafts_an_allow_rule_from_a_blocked_peer_only() {
    let mut m = everything();
    m.writes = WritesState::on();
    quiet(&mut m, key('A'));
    quiet(&mut m, kc(KeyCode::Enter)); // leave via a peer...
    quiet(&mut m, key('A')); // ...and reopen: nothing sent
    quiet(&mut m, key('w'));
    assert_eq!(m.status.text, "this row is allowed; nothing to add");
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j')); // unmanaged-nas/- icmp, blocked
    let _ = update(&mut m, key('w'));
    assert!(matches!(m.overlay, Overlay::Form(_)), "{:?}", m.overlay);
}

#[test]
fn a_w_form_from_the_overlay_returns_to_it_after_a_cancel_or_the_write() {
    use crate::tui::test_support::label_post_plan;
    let mut m = everything();
    m.writes = WritesState::on();
    quiet(&mut m, key('A'));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j')); // a blocked peer
    let open = |m: &mut Model| {
        quiet(m, key('w'));
        let Overlay::Form(f) = &mut m.overlay else {
            panic!("{:?}", m.overlay)
        };
        for (label, value) in [("Ruleset", "Shop-Prod"), ("Services", "443/tcp")] {
            f.fields
                .iter_mut()
                .find(|x| x.label == label)
                .unwrap()
                .value = value.into();
        }
    };
    // Esc in the form.
    open(&mut m);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::App, "back, nothing sent");
    assert_eq!(view(&m).panel, AppPanel::Inbound, "where it was");
    // `n` on the confirm.
    open(&mut m);
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
    let planned = || Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Rule)));
    quiet(&mut m, planned());
    assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::App);
    // The write done.
    open(&mut m);
    update(&mut m, kc(KeyCode::Enter));
    quiet(&mut m, planned());
    assert_eq!(update(&mut m, key('y')).len(), 1);
    let requests_before = view(&m).requests;
    update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                pending: true,
                status: 201,
                object: serde_json::json!({}),
                dropped_fields: vec![],
                warnings: vec![],
                audit_failed: None,
            })),
        },
    );
    assert_eq!(m.overlay, Overlay::App);
    assert_eq!(view(&m).requests, requests_before, "the view is cached");
    // A form opened on Traffic still closes to the screen.
    quiet(&mut m, kc(KeyCode::Esc));
    m.traffic.view = TrafficView::Flows;
    m.traffic.flows.select_where(|r| {
        matches!(
            r.flow.policy_decision,
            PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
        )
    });
    quiet(&mut m, key('w'));
    assert!(matches!(m.overlay, Overlay::Form(_)));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn a_w_write_from_the_overlay_returns_to_it_with_one_draft_run() {
    use crate::tui::test_support::rule_post_plan;
    let mut m = everything();
    m.traffic.data.as_mut().unwrap().hrefs = vec!["/orgs/5/traffic_flows/async_queries/q-1".into()];
    m.writes = WritesState::on();
    quiet(&mut m, key('A'));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j')); // a blocked peer
    quiet(&mut m, key('w'));
    let Overlay::Form(f) = &mut m.overlay else {
        panic!("{:?}", m.overlay)
    };
    for (label, value) in [("Ruleset", "Shop-Prod"), ("Services", "443/tcp")] {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap()
            .value = value.into();
    }
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
    quiet(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    assert_eq!(update(&mut m, key('y')).len(), 1);
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                pending: true,
                status: 201,
                object: serde_json::json!({}),
                dropped_fields: vec![],
                warnings: vec![],
                audit_failed: None,
            })),
        },
    );
    let runs = cmds
        .iter()
        .filter(|c| matches!(c, Cmd::RunDraft { .. }))
        .count();
    assert_eq!(runs, 1, "{cmds:?}");
    assert_eq!(m.overlay, Overlay::App, "back to the overlay");
    assert!(
        m.status
            .text
            .ends_with("draft re-evaluation running · Esc on Traffic cancels"),
        "{}",
        m.status.text
    );
}

#[test]
fn the_way_back_to_the_overlay_never_outlives_its_write() {
    use crate::tui::test_support::label_post_plan;
    let mut m = everything();
    m.writes = WritesState::on();
    quiet(&mut m, key('A'));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('w'));
    let Overlay::Form(f) = &mut m.overlay else {
        panic!("{:?}", m.overlay)
    };
    for (label, value) in [("Ruleset", "Shop-Prod"), ("Services", "443/tcp")] {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap()
            .value = value.into();
    }
    update(&mut m, kc(KeyCode::Enter));
    quiet(
        &mut m,
        Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Rule))),
    );
    assert_eq!(update(&mut m, key('y')).len(), 1);
    // Help opened while the write is on its way: it stays, the way back goes.
    m.overlay = Overlay::Help;
    update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Skipped {
                existing_href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                reason: "exists".into(),
            }),
        },
    );
    assert_eq!(m.overlay, Overlay::Help);
    assert!(!m.form_from_app);
    // A later provision's `n` closes to the screen, not the overlay.
    m.overlay = Overlay::None;
    quiet(
        &mut m,
        Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Provision))),
    );
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn the_peer_panel_in_force_is_one_the_direction_shows() {
    let mut m = everything();
    m.writes = WritesState::on();
    quiet(&mut m, key('A'));
    assert_eq!(view(&m).panel, AppPanel::Inbound);
    quiet(&mut m, key('X')); // in
    quiet(&mut m, key('X')); // out: Inbound now says "X shows inbound"
    assert_eq!(view(&m).panel, AppPanel::Outbound);
    quiet(&mut m, key('w'));
    assert_eq!(m.status.text, "this row is allowed; nothing to add");
    quiet(&mut m, key('X')); // both: stays where it is
    assert_eq!(view(&m).panel, AppPanel::Outbound);
    quiet(&mut m, key('X')); // in
    assert_eq!(view(&m).panel, AppPanel::Inbound);
    // Opened at the traffic focus's direction (`out`): on Outbound.
    quiet(&mut m, kc(KeyCode::Esc));
    m.app_view = None;
    m.traffic.set_focus(
        Focus::new(vec!["shop".into(), "prod".into()], Vec::new())
            .with_direction(FocusDirection::Out),
    );
    quiet(&mut m, key('A'));
    assert_eq!(view(&m).focus.direction, FocusDirection::Out);
    assert_eq!(view(&m).panel, AppPanel::Outbound);
    // The other panels are left alone.
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, key('X'));
    assert_eq!(view(&m).panel, AppPanel::Policy);
}

#[test]
fn r_asks_then_reloads_both_panels_past_the_cache() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    quiet(&mut m, key('r'));
    let Overlay::Confirm { reason, .. } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(
        reason,
        "Reload the workloads and the rulesets of shop/prod: 2 requests? (y/n)"
    );
    quiet(&mut m, key('n'));
    assert_eq!(
        m.overlay,
        Overlay::App,
        "no: back to the overlay, nothing sent"
    );
    quiet(&mut m, key('r'));
    let cmds = update(&mut m, key('y'));
    assert_eq!(requests(&cmds), 2);
    assert!(cmds.iter().all(|c| matches!(
        c,
        Cmd::LoadAppWorkloads {
            mode: CacheMode::Bypass,
            ..
        } | Cmd::LoadRuleSets {
            mode: CacheMode::Bypass,
            ..
        }
    )));
    assert_eq!(m.overlay, Overlay::App);
    assert_eq!(view(&m).requests, 2);
}

#[test]
fn the_overlay_never_sends_a_request_it_did_not_count() {
    let mut m = model_with_flows();
    let mut sent = 0;
    for msg in [
        key('A'),
        kc(KeyCode::Tab),
        key('j'),
        key('X'),
        key('X'),
        key('X'),
        kc(KeyCode::BackTab),
        key('k'),
        key('r'),
        key('y'),
        key('X'),
    ] {
        sent += requests(&update(&mut m, msg));
    }
    assert_eq!(
        sent, 3,
        "open: workloads and rulesets; r: the workloads again (the rulesets are still loading)"
    );
    assert_eq!(view(&m).requests, sent);
}

#[test]
fn t_in_the_overlay_does_nothing_now_the_timeline_is_gone() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    let before = m.status.text.clone();
    assert!(update(&mut m, key('t')).is_empty(), "no request");
    assert_eq!(m.overlay, Overlay::App, "no prompt");
    assert_eq!(m.status.text, before);
}

#[test]
fn answers_fill_the_panels_and_stale_ones_are_dropped() {
    let mut m = model_with_flows();
    let _ = update(&mut m, key('A'));
    let seq = view(&m).seq;
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq: seq + 7,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: true,
            }),
        },
    );
    assert!(
        matches!(view(&m).workloads, Loadable::Loading),
        "a stale answer is dropped"
    );
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: true,
            }),
        },
    );
    assert!(
        matches!(&view(&m).workloads, Loadable::Loaded((rows, _)) if rows.len() == 2),
        "filtered to the app"
    );
    assert_eq!(view(&m).cached, 1);
    quiet(&mut m, rulesets_answer(true));
    assert!(matches!(&view(&m).policy, Loadable::Loaded(rows) if rows.len() == 2));
    assert_eq!(view(&m).header_meta(), "2 req · cached");
}

#[test]
fn a_stale_workloads_answer_is_dropped() {
    let mut m = model_with_flows();
    let _ = update(&mut m, key('A'));
    let old = view(&m).seq;
    quiet(&mut m, kc(KeyCode::Esc));
    // Another app (row 2's source): a new overlay, a new sequence number.
    m.traffic
        .flows
        .select_where(|r| r.flow.src.display_name() == "unmanaged-nas");
    quiet(&mut m, key('A'));
    assert_ne!(view(&m).seq, old);
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq: old,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: false,
            }),
        },
    );
    assert!(matches!(view(&m).workloads, Loadable::Unavailable(_)));
    assert_eq!(view(&m).focus.app_text(), "unmanaged-nas/-");
}

#[test]
fn a_traffic_answer_under_the_open_overlay_rebuilds_its_peers() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    assert_eq!(view(&m).inbound.len(), 3);
    let flows = m.traffic.loaded_flows()[..1].to_vec(); // shop/prod -> shop/prod
    let seq = m.traffic.seq;
    let filter = m.traffic.filter.clone();
    let _ = update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(crate::tui::msg::QueryOutcome {
                flows,
                truncated: false,
                incomplete: false,
                elapsed: std::time::Duration::ZERO,
                hit: false,
                filter,
                bounds: None,
                hrefs: Vec::new(),
            }),
        },
    );
    assert_eq!(m.overlay, Overlay::App);
    assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (1, 1));
}

#[test]
fn the_request_budget_holds_end_to_end_answers_included() {
    let mut m = model_with_flows();
    let mut sent = requests(&update(&mut m, key('A')));
    let seq = view(&m).seq;
    let workloads = || Msg::AppWorkloadsLoaded {
        seq,
        result: Ok(AppWorkloadsPayload {
            workloads: fixture_workloads(),
            truncated: false,
            hit: false,
        }),
    };
    sent += requests(&update(&mut m, workloads()));
    sent += requests(&update(&mut m, rulesets_answer(false)));
    assert_eq!(
        (sent, view(&m).requests),
        (2, 2),
        "no deny rules off the Policy screen"
    );
    assert_eq!(view(&m).header_meta(), "2 req");
    for msg in [key('r'), key('y')] {
        sent += requests(&update(&mut m, msg));
    }
    sent += requests(&update(&mut m, workloads()));
    sent += requests(&update(&mut m, rulesets_answer(true)));
    assert_eq!((sent, view(&m).requests), (4, 4));
    assert_eq!(view(&m).header_meta(), "4 req");
    // The Policy screen asks for the selected ruleset's deny rules on entry.
    quiet(&mut m, kc(KeyCode::Esc));
    let cmds = update(&mut m, Msg::Command(Command::Screen(Screen::Policy)));
    assert!(
        matches!(cmds.as_slice(), [Cmd::LoadDenyRules { .. }]),
        "{cmds:?}"
    );
}

fn one_flow_answer(m: &Model) -> Msg {
    Msg::QueryDone {
        seq: m.traffic.seq,
        result: Ok(crate::tui::msg::QueryOutcome {
            flows: m.traffic.loaded_flows()[..1].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::ZERO,
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        }),
    }
}

#[test]
fn a_on_pairs_mesh_and_treemap_opens_the_rows_app() {
    let mut m = everything();
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.apps_state.by, crate::tui::model::By::Pairs);
    m.traffic
        .groups
        .select_where(|g| g.dst.join("/") == "fqdn-1.example/-");
    quiet(&mut m, key('A'));
    assert_eq!(view(&m).focus.app_text(), "shop/prod", "a pair: its source");
    quiet(&mut m, kc(KeyCode::Esc));

    m.traffic.view = TrafficView::Mesh;
    m.traffic
        .mesh
        .select_where(|r| r.src.join("/") == "unmanaged-nas/-");
    quiet(&mut m, key('A'));
    assert_eq!(
        view(&m).focus.app_text(),
        "unmanaged-nas/-",
        "a mesh row: its source"
    );
    quiet(&mut m, kc(KeyCode::Esc));

    let mut m = with_policy(with_workloads(
        crate::tui::test_support::model_with_rich_flows(),
    ));
    m.traffic.view = TrafficView::Treemap;
    let tm = &m.traffic.treemap;
    let label = tm.tile(tm.current().expect("a tile")).label.clone();
    let cmds = update(&mut m, key('A'));
    assert_eq!(m.overlay, Overlay::App);
    assert_eq!(
        view(&m).focus.app_text(),
        label,
        "a treemap tile: its source group"
    );
    assert!(requests(&cmds) <= 2);
    assert_eq!(requests(&cmds), view(&m).requests);
}

#[test]
fn a_traffic_answer_under_the_reload_prompt_rebuilds_the_peers() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    quiet(&mut m, key('r'));
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    let answer = one_flow_answer(&m);
    let _ = update(&mut m, answer);
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::App);
    assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (1, 1));
}

#[test]
fn a_reused_view_drops_policy_rows_of_the_other_scope() {
    let mut m = everything();
    quiet(&mut m, key('A'));
    assert_eq!(view(&m).policy_scope, PolicyScope::Draft);
    quiet(&mut m, kc(KeyCode::Esc));
    let cmds = update(&mut m, Msg::Command(Command::Active));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::LoadRuleSets {
            scope: PolicyScope::Active,
            ..
        }]
    ));
    m.screen = Screen::Traffic;
    quiet(&mut m, key('A')); // the active load is on its way: not doubled
    let v = view(&m);
    assert_eq!(v.policy_scope, PolicyScope::Active);
    assert!(
        matches!(v.policy, Loadable::Loading),
        "no draft rows: {:?}",
        v.policy
    );
    let mut rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    rs.truncate(1);
    let _ = update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Active,
            result: Ok((rs, 0, false)),
        },
    );
    assert!(matches!(&view(&m).policy, Loadable::Loaded(rows) if rows.len() == 1));
}

#[test]
fn the_reload_prompt_counts_what_it_will_send() {
    let mut m = model_with_flows();
    let _ = update(&mut m, key('A')); // workloads and rulesets on their way
    quiet(&mut m, key('r'));
    let Overlay::Confirm { reason, .. } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(
        reason,
        "Reload the workloads of shop/prod (the rulesets are on their way): 1 request? (y/n)"
    );
    let cmds = update(&mut m, key('y'));
    assert_eq!(requests(&cmds), 1);
    assert_eq!(view(&m).requests, 3);
}

#[test]
fn enter_on_a_workload_before_its_list_opens_it_when_the_list_arrives() {
    let mut m = model_with_flows();
    let _ = update(&mut m, key('A'));
    let seq = view(&m).seq;
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: false,
            }),
        },
    );
    quiet(&mut m, kc(KeyCode::BackTab)); // Workloads
    quiet(&mut m, key('j'));
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert!(
        matches!(cmds.as_slice(), [Cmd::LoadWorkloads(_)]),
        "{cmds:?}"
    );
    assert_eq!(m.screen, Screen::Workloads);
    assert!(!m.workloads.show_detail);
    let _ = update(
        &mut m,
        Msg::WorkloadsLoaded(Ok((fixture_workloads(), 0, false))),
    );
    assert!(m.workloads.show_detail);
    assert_eq!(
        m.workloads
            .table
            .selected_item()
            .map(|r| r.workload.href.as_str()),
        Some("/orgs/5/workloads/aaaa-2")
    );
    assert_eq!(m.workloads.open_on_load, None);
}

#[test]
fn a_late_answer_never_reopens_a_closed_overlay() {
    let mut m = model_with_flows();
    let _ = update(&mut m, key('A'));
    let seq = view(&m).seq;
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: false,
            }),
        },
    );
    quiet(&mut m, rulesets_answer(false));
    assert_eq!(m.overlay, Overlay::None, "the answers do not reopen it");
}

#[test]
fn a_reopens_the_same_app_from_its_view_without_requests() {
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let cmds = update(&mut m, key('A'));
    assert_eq!(requests(&cmds), 2);
    let seq = view(&m).seq;
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: false,
            }),
        },
    );
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j')); // unmanaged-nas/- icmp, blocked
    let _ = update(&mut m, key('w'));
    assert!(matches!(m.overlay, Overlay::Form(_)), "{:?}", m.overlay);
    m.overlay = Overlay::None; // the form cancelled
    quiet(&mut m, key('A'));
    assert_eq!(m.overlay, Overlay::App);
    let v = view(&m);
    assert_eq!(v.seq, seq, "the same view");
    assert_eq!(v.requests, 2, "nothing more sent");
    assert!(matches!(&v.workloads, Loadable::Loaded((rows, _)) if rows.len() == 2));
    assert!(
        matches!(v.policy, Loadable::Loading),
        "the rulesets are still on their way"
    );
    assert_eq!(v.cursor[0], 2, "the cursor is kept");
}

#[test]
fn a_failed_workloads_request_shows_the_error_and_r_retries() {
    let mut m = model_with_flows();
    let _ = update(&mut m, key('A'));
    let seq = view(&m).seq;
    quiet(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq,
            result: Err("HTTP 500".into()),
        },
    );
    assert_eq!(view(&m).workloads, Loadable::Failed("HTTP 500".into()));
    assert_eq!(view(&m).inbound.len(), 3, "the other panels stay");
}

#[test]
fn a_rulesets_load_in_flight_is_not_doubled() {
    let mut m = with_workloads(model_with_flows());
    m.policy.loading = true;
    let cmds = update(&mut m, key('A'));
    assert_eq!(
        requests(&cmds),
        0,
        "workloads are loaded; the rulesets are on their way"
    );
    assert!(matches!(view(&m).policy, Loadable::Loading));
    assert!(!view(&m).policy_sent);
}

#[test]
fn a_name_keyed_app_requests_nothing_for_workloads_or_policy() {
    let mut m = model_with_flows();
    // Flows row 2: unmanaged-nas -> shop/prod; A takes the source.
    m.traffic
        .flows
        .select_where(|r| r.flow.src.display_name() == "unmanaged-nas");
    let cmds = update(&mut m, key('A'));
    assert_eq!(requests(&cmds), 0);
    let v = view(&m);
    assert_eq!(v.focus.app_text(), "unmanaged-nas/-");
    assert_eq!(v.outbound.len(), 1, "its one blocked flow");
    assert!(
        matches!(&v.workloads, Loadable::Unavailable(why) if why.contains("not a label group"))
    );
    assert!(matches!(&v.policy, Loadable::Unavailable(_)));
}

/// #90: `:app`, `:gen` and `:enforce` take the app in any case and work on
/// the PCE's spelling (the flows are found); two labels that differ only in
/// case are a status line, nothing sent.
#[test]
fn app_by_name_ignores_case_and_refuses_an_ambiguous_label() {
    let mut m = everything();
    let cmds = update(&mut m, Msg::Command(Command::App(Some("SHOP/Prod".into()))));
    assert_eq!(m.overlay, Overlay::App, "{}", m.status.text);
    assert_eq!(requests(&cmds), 0, "workloads and rulesets are loaded");
    assert_eq!(view(&m).focus.app_text(), "shop/prod");
    assert!(!view(&m).inbound.is_empty() || !view(&m).outbound.is_empty());
    quiet(&mut m, kc(KeyCode::Esc));
    update(&mut m, Msg::Command(Command::Gen(Some("SHOP/Prod".into()))));
    assert!(m.gen_view.is_some(), "{}", m.status.text);
    m.gen_view = None;
    m.overlay = Overlay::None;
    update(
        &mut m,
        Msg::Command(Command::Enforce(Some("SHOP/Prod".into()))),
    );
    assert!(m.overlay == Overlay::Enforce, "{}", m.status.text);
    m.overlay = Overlay::None;
    // Two labels of one key differing in case.
    let label = |n: u32, k: &str, v: &str| -> pce::models::Label {
        serde_json::from_value(serde_json::json!({
            "href": format!("/orgs/5/labels/{n}"), "key": k, "value": v
        }))
        .unwrap()
    };
    let idx = pce::NameIndex::new(
        &[
            label(1, "app", "Web"),
            label(2, "app", "web"),
            label(3, "env", "prod"),
        ],
        &[],
        &[],
        &[],
    );
    if let crate::tui::model::Connection::Connected { index, .. } = &mut m.connection {
        *index = Some(std::sync::Arc::new(idx));
    }
    let msg = "app=WEB matches app=Web and app=web; write one exactly; nothing sent";
    for c in [
        Command::App(Some("WEB/prod".into())),
        Command::Gen(Some("WEB/prod".into())),
        Command::Enforce(Some("WEB/prod".into())),
    ] {
        m.status = StatusLine::info("");
        quiet(&mut m, Msg::Command(c));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, msg);
    }
}

#[test]
fn app_by_name_reaches_apps_without_flows_and_names_the_closest_otherwise() {
    let mut m = everything();
    m.traffic.data.as_mut().unwrap().flows.clear();
    m.traffic.rebuild_views();
    quiet(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    assert_eq!(m.overlay, Overlay::App, "its labels exist");
    assert!(view(&m).inbound.is_empty() && view(&m).outbound.is_empty());
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(&mut m, Msg::Command(Command::App(Some("shop/prd".into()))));
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        m.status.text.starts_with("unknown app \"shop/prd\""),
        "{}",
        m.status.text
    );
    quiet(&mut m, Msg::Command(Command::App(None)));
    assert!(
        matches!(&m.overlay, Overlay::AppPicker(p) if p.purpose == crate::tui::app_picker::AppPickerPurpose::Overlay)
    );
}
