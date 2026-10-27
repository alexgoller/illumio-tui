use super::*;
use crate::tui::model::{AppState, Model, Overlay, RowKind};
use crate::tui::test_support::model_with_rich_flows;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::traffic::{Decisions, Direction};

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

fn on_apps() -> Model {
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    m
}

fn headers(m: &Model) -> Vec<String> {
    m.traffic
        .apps
        .rows()
        .iter()
        .filter(|r| r.is_header())
        .map(|r| r.app.join("/"))
        .collect()
}

#[test]
fn x_and_capital_x_cycle_the_state_and_d_the_decision_filter_without_requests() {
    let mut m = on_apps();
    assert_eq!(m.traffic.apps_state.by, By::Pairs);
    assert!(m.traffic.apps.is_empty(), "pairs builds no sections");
    let mut seen = Vec::new();
    for _ in 0..4 {
        quiet(&mut m, key('x'));
        seen.push(m.traffic.apps_state.by);
    }
    assert_eq!(seen, vec![By::In, By::Out, By::Both, By::Pairs]);
    let mut seen = Vec::new();
    for _ in 0..4 {
        quiet(&mut m, key('X'));
        seen.push(m.traffic.apps_state.by);
    }
    assert_eq!(seen, vec![By::Both, By::Out, By::In, By::Pairs]);
    quiet(&mut m, key('x'));
    let all = m.traffic.apps.len();
    let mut seen = Vec::new();
    for _ in 0..3 {
        quiet(&mut m, key('d'));
        seen.push((m.traffic.apps_state.decisions, m.traffic.apps.len()));
    }
    assert_eq!(seen[0].0, Decisions::Blocked);
    assert_eq!(seen[1].0, Decisions::BlockedOnly);
    assert_eq!(seen[2], (Decisions::All, all));
    assert!(seen[1].1 < seen[0].1 && seen[0].1 < all, "{seen:?}");
    assert_eq!(m.status.text, "decision all");
}

#[test]
fn enter_on_a_header_drills_to_the_app_and_on_a_peer_row_to_its_flows() {
    let mut m = on_apps();
    quiet(&mut m, key('x'));
    quiet(&mut m, key('d')); // blocked
    let first = m.traffic.apps.selected_item().unwrap().clone();
    let RowKind::Header { totals, .. } = &first.kind else {
        panic!("{first:?}")
    };
    let expected = totals[0].1.flows as usize;
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(
        m.traffic.drill,
        Some(Drill::App {
            app: first.app.clone(),
            direction: Some(Direction::In),
            decisions: Decisions::Blocked,
        })
    );
    assert_eq!(m.traffic.flows.len(), expected);
    assert!(
        m.traffic
            .drill
            .as_ref()
            .unwrap()
            .title()
            .ends_with(" · blocked")
    );
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    assert_eq!(
        m.traffic.apps_state.by,
        By::In,
        "the state outlives the drill"
    );
    quiet(&mut m, key('j'));
    let row = m.traffic.apps.selected_item().unwrap().clone();
    let (direction, peer) = row.peer().unwrap();
    let (flows, peer_key, service) = (peer.flows, peer.peer.clone(), peer.service.clone());
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        m.traffic.drill,
        Some(Drill::Peer {
            app: row.app.clone(),
            direction,
            peer: peer_key,
            service,
            decisions: Decisions::Blocked,
        })
    );
    assert_eq!(m.traffic.flows.len() as u64, flows);
    assert!(
        m.traffic
            .flows
            .rows()
            .iter()
            .all(|r| r.flow.policy_decision != PolicyDecision::Allowed)
    );
}

#[test]
fn both_header_drills_to_either_side() {
    let mut m = on_apps();
    m.traffic.apps_state.by = By::Both;
    m.traffic.rebuild_views();
    let app = m.traffic.apps.selected_item().unwrap().app.clone();
    quiet(&mut m, kc(KeyCode::Enter));
    let Some(Drill::App { direction, .. }) = &m.traffic.drill else {
        panic!("{:?}", m.traffic.drill)
    };
    assert_eq!(*direction, None);
    let keys = m.traffic.effective_keys();
    let n = m
        .traffic
        .loaded_flows()
        .iter()
        .filter(|f| {
            pce::traffic::group_key(&f.src, &keys) == app
                || pce::traffic::group_key(&f.dst, &keys) == app
        })
        .count();
    assert_eq!(m.traffic.flows.len(), n);
}

#[test]
fn space_folds_a_section_and_z_folds_or_unfolds_them_all() {
    let mut m = on_apps();
    quiet(&mut m, key('x'));
    let all = m.traffic.apps.len();
    let apps = headers(&m);
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j'));
    quiet(&mut m, key(' '));
    let sel = m.traffic.apps.selected_item().unwrap();
    assert!(sel.is_header(), "the cursor lands on the folded header");
    assert!(m.traffic.apps_state.folded.contains(&sel.app));
    assert!(m.traffic.apps.len() < all);
    assert_eq!(headers(&m), apps, "every section keeps its header");
    quiet(&mut m, key(' '));
    assert_eq!(m.traffic.apps.len(), all);
    quiet(&mut m, key('z'));
    assert_eq!(m.traffic.apps.len(), apps.len(), "headers only");
    quiet(&mut m, key('z'));
    assert_eq!(m.traffic.apps.len(), all);
    quiet(&mut m, key('s'));
    assert_eq!(
        m.traffic.apps.sort(),
        None,
        "s orders the sections, no column"
    );
    assert_eq!(m.status.text, "by app A→Z");
}

#[test]
fn search_keeps_a_section_when_any_of_its_rows_matches() {
    let mut m = on_apps();
    quiet(&mut m, key('x'));
    quiet(&mut m, key('/'));
    for c in "53/udp".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, kc(KeyCode::Enter));
    let shown: Vec<_> = m
        .traffic
        .apps
        .visible()
        .iter()
        .map(|&i| m.traffic.apps.rows()[i].clone())
        .collect();
    assert!(!shown.is_empty());
    assert!(shown.iter().any(|r| r.is_header()));
    assert!(shown.len() < m.traffic.apps.rows().len());
    let apps: std::collections::BTreeSet<_> = shown.iter().map(|r| r.app.clone()).collect();
    for app in &apps {
        let whole = m
            .traffic
            .apps
            .rows()
            .iter()
            .filter(|r| &r.app == app)
            .count();
        let kept = shown.iter().filter(|r| &r.app == app).count();
        assert_eq!(whole, kept, "the whole section stays");
    }
}

#[test]
fn the_state_survives_g_and_r_and_a_profile_switch_resets_it() {
    let mut m = on_apps();
    quiet(&mut m, key('x'));
    quiet(&mut m, key('d'));
    quiet(&mut m, key('z'));
    quiet(&mut m, key('s'));
    let st = m.traffic.apps_state.clone();
    assert!(!st.folded.is_empty());
    assert_eq!(st.order, pce::traffic::SectionOrder::Name);
    quiet(&mut m, key('g'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.traffic.apps_state, st, "g keeps it");
    assert!(!m.traffic.apps.is_empty());
    let cmds = update(&mut m, key('r'));
    let filter = m.traffic.filter.clone();
    assert_eq!(
        cmds,
        vec![Cmd::RunQuery {
            seq: m.traffic.seq,
            filter: filter.clone(),
            mode: pce::CacheMode::Bypass,
            cancel: tokio_util::sync::CancellationToken::new(),
            slot: "main",
        }],
        "r runs the query"
    );
    let flows = crate::tui::test_support::fixture_flows_rich();
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(crate::tui::msg::QueryOutcome {
                flows,
                truncated: false,
                incomplete: false,
                elapsed: std::time::Duration::from_secs(1),
                hit: false,
                filter,
                bounds: None,
                hrefs: Vec::new(),
            }),
        },
    );
    assert_eq!(m.traffic.apps_state, st, "r keeps it");
    assert_eq!(m.traffic.apps.len(), headers(&m).len(), "still folded");
    m.reset_data();
    assert_eq!(
        m.traffic.apps_state,
        AppState::default(),
        ":profile resets it"
    );
}

#[test]
fn w_on_a_peer_row_opens_the_rule_form_and_on_a_header_explains() {
    let mut m = on_apps();
    m.writes = crate::tui::model::WritesState::on();
    quiet(&mut m, key('x'));
    quiet(&mut m, key('d'));
    quiet(&mut m, key('w'));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.status.text,
        "select a peer row (a header holds a whole app)"
    );
    quiet(&mut m, key('j'));
    assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.title, "Allow rule from flow");
}

#[test]
fn blocked_ignores_x_and_d_and_says_why() {
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Blocked;
    let before = (m.traffic.apps_state.clone(), m.traffic.blocked.len());
    for c in ['x', 'X', 'd'] {
        quiet(&mut m, key(c));
        assert_eq!(
            m.status.text,
            "Blocked is App-to-app with in · blocked; press v until App-to-app"
        );
        assert_eq!(
            (m.traffic.apps_state.clone(), m.traffic.blocked.len()),
            before
        );
    }
    quiet(&mut m, key('z'));
    assert!(m.traffic.blocked.rows().iter().all(|r| r.is_header()));
    assert!(m.traffic.apps_state.folded.is_empty(), "its own folds");
    quiet(&mut m, key('z'));
    quiet(&mut m, key('j'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(matches!(
        m.traffic.drill,
        Some(Drill::Peer {
            direction: Direction::In,
            decisions: Decisions::Blocked,
            ..
        })
    ));
}
