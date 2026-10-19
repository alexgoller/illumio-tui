use super::*;
use crate::tui::model::{Model, Overlay, PendingAction, TrafficView};
use crate::tui::msg::FocusedOutcome;
use crate::tui::test_support::{model_fixture, model_with_flows, model_with_rich_flows};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::traffic::FocusDirection;
use std::time::Duration;

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

fn focus_cmd(name: &str) -> Msg {
    Msg::Command(Command::Focus(Some(name.into())))
}

fn focused(m: &Model) -> Option<(String, FocusDirection)> {
    m.traffic
        .focus
        .as_ref()
        .map(|f| (f.app_text(), f.direction))
}

#[test]
fn x_and_esc_cycle_and_clear_the_focus_on_every_view_without_requests() {
    for view in [
        TrafficView::Flows,
        TrafficView::Aggregate,
        TrafficView::Mesh,
        TrafficView::Blocked,
        TrafficView::Treemap,
    ] {
        let mut m = model_with_rich_flows();
        m.traffic.view = view;
        quiet(&mut m, focus_cmd("crm/prod"));
        assert_eq!(
            focused(&m),
            Some(("crm/prod".into(), FocusDirection::Both)),
            "{view:?}"
        );
        quiet(&mut m, key('X'));
        assert_eq!(focused(&m).unwrap().1, FocusDirection::In, "{view:?}");
        let t = &m.traffic;
        let crm = |k: &Vec<String>| k.join("/") == "crm/prod";
        match view {
            TrafficView::Flows => {
                assert_eq!(t.flows.len(), 7);
                assert!(t.flows.rows().iter().all(|r| &*r.dst_group == "crm/prod"));
            }
            TrafficView::Aggregate => {
                assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| crm(&g.dst)))
            }
            TrafficView::Mesh => {
                assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| crm(&r.dst)))
            }
            TrafficView::Blocked => {
                assert!(!t.blocked.is_empty() && t.blocked.rows().iter().all(|r| crm(&r.app)))
            }
            TrafficView::Treemap => {
                assert!(!t.treemap.tiles.is_empty() && t.treemap.groups.iter().all(|g| crm(&g.dst)))
            }
        }
        quiet(&mut m, key('X'));
        assert_eq!(focused(&m).unwrap().1, FocusDirection::Out);
        quiet(&mut m, key('X'));
        assert_eq!(focused(&m).unwrap().1, FocusDirection::Both);
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(focused(&m), None, "{view:?}");
        assert_eq!(m.traffic.flows.len(), 30, "{view:?}: every flow again");
        assert_eq!(m.status.text, "focus cleared");
    }
}

#[test]
fn without_a_focus_x_and_esc_keep_their_old_meaning() {
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    quiet(&mut m, key('X'));
    assert_eq!(m.traffic.apps_state.by, By::Both, "X steps back from pairs");
    m.traffic.view = TrafficView::Flows;
    m.traffic.drill = Some(Drill::Pair(
        vec!["crm".into(), "prod".into()],
        vec!["crm".into(), "prod".into()],
        Decisions::All,
    ));
    m.traffic.rebuild_views();
    quiet(&mut m, focus_cmd("crm/prod"));
    quiet(&mut m, kc(KeyCode::Esc));
    assert!(
        m.traffic.drill.is_some(),
        "the first Esc clears the focus only"
    );
    quiet(&mut m, kc(KeyCode::Esc));
    assert!(
        m.traffic.drill.is_none(),
        "then Esc clears the drill as before"
    );
}

#[test]
fn f_on_a_row_takes_its_source_then_its_destination() {
    let mut m = model_with_flows();
    m.traffic
        .flows
        .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
    quiet(&mut m, key('F'));
    assert_eq!(
        focused(&m),
        Some(("shop/prod".into(), FocusDirection::Both))
    );
    assert_eq!(m.status.text, "focus shop/prod · both · 4 flows");
    m.traffic
        .flows
        .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
    quiet(&mut m, key('F'));
    assert_eq!(focused(&m).unwrap().0, "fqdn-1.example/-");
    quiet(&mut m, kc(KeyCode::Esc));
    m.traffic.view = TrafficView::Blocked;
    m.traffic.blocked.select_where(|r| !r.is_header());
    quiet(&mut m, key('F'));
    assert_eq!(
        focused(&m).unwrap().0,
        "shop/prod",
        "Blocked: the destination first"
    );
}

#[test]
fn f_without_a_row_opens_the_picker_or_clears_and_the_picker_focuses() {
    let mut m = model_with_flows();
    m.traffic.flows.set_rows(Vec::new());
    quiet(&mut m, key('F'));
    let Overlay::AppPicker(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(p.apps, vec!["shop/prod"]);
    quiet(&mut m, key('s'));
    quiet(&mut m, key('q'));
    quiet(&mut m, kc(KeyCode::Backspace));
    assert!(!m.quit, "q is typed into the picker");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(focused(&m).unwrap().0, "shop/prod");
    m.traffic.flows.set_rows(Vec::new());
    quiet(&mut m, key('F'));
    assert_eq!(focused(&m), None, "F on nothing with a focus set clears it");
    quiet(&mut m, Msg::Command(Command::Focus(None)));
    assert!(matches!(m.overlay, Overlay::AppPicker(_)));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(focused(&m), None);
}

#[test]
fn focus_names_an_unknown_app_and_its_closest_match_and_sends_nothing() {
    let mut m = model_with_rich_flows();
    quiet(&mut m, focus_cmd("crm/prd"));
    assert_eq!(focused(&m), None);
    assert_eq!(
        m.status.text,
        "unknown app \"crm/prd\"; closest: crm/prod; nothing sent"
    );
    let mut m = model_fixture();
    quiet(&mut m, focus_cmd("crm/prod"));
    assert_eq!(m.status.text, "no result to focus yet: r runs the query");
    quiet(&mut m, key('F'));
    assert_eq!(m.status.text, "no result to focus yet: r runs the query");
}

#[test]
fn focus_from_another_screen_shows_the_traffic_screen() {
    let mut m = model_with_rich_flows();
    m.screen = Screen::Workloads;
    quiet(&mut m, focus_cmd("crm/prod"));
    assert_eq!(m.screen, Screen::Traffic);
}

#[test]
fn g_clears_the_focus_and_says_so() {
    let mut m = model_with_rich_flows();
    quiet(&mut m, focus_cmd("crm/prod"));
    apply_group_keys(&mut m, vec!["app".into()], false);
    assert_eq!(focused(&m), None);
    assert_eq!(
        m.status.text,
        "grouped by app · focus cleared: the app is not defined under the new keys"
    );
    assert_eq!(m.traffic.flows.len(), 30);
    apply_group_keys(&mut m, vec!["app".into(), "env".into()], false);
    assert_eq!(
        m.status.text, "grouped by app,env",
        "no focus, nothing to say"
    );
}

fn focused_data(m: &Model, n: usize) -> TrafficData {
    TrafficData {
        flows: m.traffic.loaded_flows()[..n].to_vec(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::from_millis(900),
        cache_hit: false,
        fetched_at: Instant::now(),
        filter: m.traffic.filter.clone(),
        range: None,
        hrefs: Vec::new(),
        draft: None,
    }
}

#[test]
fn a_truncated_result_offers_r_and_r_sends_one_query_per_side() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(&mut m, focus_cmd("shop/prod"));
    assert_eq!(
        m.status.text,
        "focus shop/prod · both · 4 flows · the result is truncated: r re-queries shop/prod (2 queries)"
    );
    let cmds = update(&mut m, key('r'));
    let [
        Cmd::RunFocused {
            seq,
            filter,
            focus,
            mode,
            ..
        },
    ] = cmds.as_slice()
    else {
        panic!("{cmds:?}")
    };
    assert_eq!(*seq, m.traffic.seq);
    assert_eq!(filter, &m.traffic.filter);
    assert_eq!(focus.app_text(), "shop/prod");
    assert_eq!(*mode, CacheMode::Use);
    assert_eq!(
        m.status.text,
        "focused query running: shop/prod · 2 queries"
    );
    let flows = crate::tui::test_support::fixture_flows();
    let outcome = QueryOutcome {
        flows: flows[..3].to_vec(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::from_millis(900),
        hit: false,
        filter: m.traffic.filter.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::FocusedDone {
            seq,
            result: Ok(FocusedOutcome {
                outcome,
                focus: focus.clone(),
                queries: 2,
            }),
        },
    );
    assert!(!m.traffic.is_running());
    assert_eq!(m.traffic.loaded_flows().len(), 3);
    assert_eq!(
        m.traffic.focus_meta().as_deref(),
        Some("focus shop/prod · both · 2 queries")
    );
    assert_eq!(m.status.text, "3 flows for shop/prod in 0.9s · 2 queries");
    let cmds = update(&mut m, key('r'));
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::RunFocused {
                mode: CacheMode::Bypass,
                ..
            }]
        ),
        "r on the focused result refreshes it past the cache: {cmds:?}"
    );
    m.traffic.cancel_query();
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(focused(&m), None);
    assert_eq!(
        m.traffic.loaded_flows().len(),
        4,
        "the unfocused result is back"
    );
    assert!(m.traffic.data.as_ref().unwrap().truncated);
}

/// A focused result on screen: `B` and `]` re-run the focused query at
/// the next cap, one query per side, asking above 10000; the answer
/// says how `B` goes on.
#[test]
fn leaving_a_focus_puts_the_unfocused_cap_back() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    let unfocused_cap = m.traffic.data.as_ref().unwrap().filter.max_results;
    quiet(&mut m, focus_cmd("shop/prod"));
    let focus = m.traffic.focus.clone().unwrap();
    let mut data = focused_data(&m, 2);
    data.truncated = true;
    m.traffic.apply_focused(focus, 2, data);
    let _ = update(&mut m, key('B')); // a focused cap step in flight
    assert_ne!(m.traffic.filter.max_results, unfocused_cap);
    quiet(&mut m, kc(KeyCode::Esc)); // cancels it, the focus goes
    assert_eq!(m.traffic.filter.max_results, unfocused_cap);
    // `B` now offers the unfocused result's next rung, not "r first".
    let cmds = update(&mut m, key('B'));
    assert!(
        !m.status.text.contains("nothing to backfill"),
        "{}",
        m.status.text
    );
    assert!(
        !cmds.is_empty() || matches!(m.overlay, Overlay::Confirm { .. }),
        "{cmds:?}"
    );
}

#[test]
fn b_and_brackets_raise_the_cap_of_a_focused_result() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(&mut m, focus_cmd("shop/prod"));
    let focus = m.traffic.focus.clone().unwrap();
    let mut data = focused_data(&m, 2);
    data.truncated = true;
    m.traffic.apply_focused(focus.clone(), 2, data);
    let cmds = update(&mut m, key('B'));
    let [
        Cmd::RunFocused {
            filter,
            focus: f,
            mode,
            ..
        },
    ] = cmds.as_slice()
    else {
        panic!("{cmds:?}")
    };
    assert_eq!((filter.max_results, *mode), (10_000, CacheMode::Use));
    assert_eq!(f, &focus);
    assert_eq!(m.traffic.filter.max_results, 10_000);
    assert!(m.traffic.focus_result.is_some(), "still focused");
    let outcome = QueryOutcome {
        flows: m.traffic.loaded_flows().to_vec(),
        truncated: true,
        incomplete: false,
        elapsed: Duration::from_millis(900),
        hit: false,
        filter: filter.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::FocusedDone {
            seq,
            result: Ok(FocusedOutcome {
                outcome,
                focus: focus.clone(),
                queries: 2,
            }),
        },
    );
    assert_eq!(
        m.status.text,
        "2 flows for shop/prod · 2 queries, capped at 10000; B raises the cap"
    );
    let out = crate::tui::test_support::render_to_string(&m, 160, 40);
    assert!(out.contains("capped at 10000; B raises the cap"), "{out}");
    // Above 10000 the next rung asks, naming the queries; n changes nothing.
    quiet(&mut m, key(']'));
    match &m.overlay {
        Overlay::Confirm {
            reason,
            action: PendingAction::RunFocused(f, _, CacheMode::Use),
        } => {
            assert_eq!(f.max_results, 25_000);
            assert!(
                reason.starts_with(
                    "cap 25000 exceeds 10000. Re-run shop/prod at cap 25000 (2 queries"
                ),
                "{reason}"
            );
        }
        o => panic!("{o:?}"),
    }
    quiet(&mut m, key('n'));
    assert_eq!(m.traffic.filter.max_results, 10_000);
    quiet(&mut m, key('B'));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunFocused { filter, .. }] if filter.max_results == 25_000),
        "{cmds:?}"
    );
}

#[test]
fn a_stale_focused_answer_is_dropped() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(&mut m, focus_cmd("shop/prod"));
    let _ = update(&mut m, key('r'));
    let stale = m.traffic.seq;
    m.traffic.cancel_query();
    let outcome = QueryOutcome {
        flows: Vec::new(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::ZERO,
        hit: false,
        filter: m.traffic.filter.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    let focus = m.traffic.focus.clone().unwrap();
    quiet(
        &mut m,
        Msg::FocusedDone {
            seq: stale,
            result: Ok(FocusedOutcome {
                outcome,
                focus,
                queries: 2,
            }),
        },
    );
    assert!(m.traffic.focus_result.is_none());
    assert_eq!(m.traffic.loaded_flows().len(), 4);
}

#[test]
fn an_expensive_focused_query_asks_first_and_names_the_count() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    m.traffic.filter.max_results = 20_000;
    quiet(&mut m, focus_cmd("shop/prod"));
    quiet(&mut m, key('X'));
    quiet(&mut m, key('r'));
    let Overlay::Confirm { reason, .. } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(
        reason,
        "cap 20000 exceeds 10000. Run 1 query for shop/prod anyway? (y/n)"
    );
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunFocused { focus, .. }] if focus.direction == FocusDirection::In),
        "{cmds:?}"
    );
}

#[test]
fn a_group_without_labels_is_not_re_queried() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    m.traffic.view = TrafficView::Blocked;
    let nas = |r: &crate::tui::model::SectionRow| {
        r.peer()
            .is_some_and(|(_, p)| p.peer_text() == "unmanaged-nas/-")
    };
    m.traffic.blocked.select_where(nas);
    quiet(&mut m, key('F'));
    m.traffic.blocked.select_where(nas);
    quiet(&mut m, key('F'));
    assert_eq!(focused(&m).unwrap().0, "unmanaged-nas/-");
    quiet(&mut m, key('r'));
    assert_eq!(
        m.status.text,
        "unmanaged-nas/- is not a label group: it cannot be re-queried by its labels; Esc clears the focus, then r reruns the whole query"
    );
}

#[test]
fn r_without_a_partial_focus_reruns_the_unfocused_query() {
    let mut m = model_with_flows();
    quiet(&mut m, focus_cmd("shop/prod"));
    let cmds = update(&mut m, key('r'));
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::RunQuery {
                mode: CacheMode::Bypass,
                ..
            }]
        ),
        "{cmds:?}"
    );
}

#[test]
fn window_and_cap_keys_work_on_the_unfocused_result() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(&mut m, focus_cmd("shop/prod"));
    let f = m.traffic.focus.clone().unwrap();
    let data = focused_data(&m, 2);
    m.traffic.apply_focused(f, 2, data);
    let _ = update(&mut m, key('-'));
    assert!(m.traffic.unfocused.is_none() && m.traffic.focus_result.is_none());
    assert_eq!(m.traffic.loaded_flows().len(), 4);
    assert!(
        m.traffic.focus.is_some(),
        "the focus stays and narrows the unfocused result"
    );
    // A new unfocused result also drops a focused one kept aside.
    let f = m.traffic.focus.clone().unwrap();
    let data = focused_data(&m, 1);
    m.traffic.apply_focused(f, 2, data);
    let seq = m.traffic.seq;
    let outcome = QueryOutcome {
        flows: crate::tui::test_support::fixture_flows(),
        truncated: true,
        incomplete: false,
        elapsed: Duration::from_millis(100),
        hit: false,
        filter: m.traffic.filter.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    let _ = update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(outcome),
        },
    );
    assert!(m.traffic.unfocused.is_none() && m.traffic.focus_result.is_none());
    assert!(
        m.traffic.focus_partial(),
        "the new result is truncated: partial again"
    );
}

/// A focused run in flight on the rich fixture (truncated), focused on
/// crm/prod: its seq and focus.
fn in_flight() -> (Model, u64, pce::traffic::Focus) {
    let mut m = model_with_rich_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(&mut m, focus_cmd("crm/prod"));
    let cmds = update(&mut m, key('r'));
    let [Cmd::RunFocused { seq, focus, .. }] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    assert!(m.traffic.is_running());
    assert_eq!(m.traffic.running_focus(), Some(focus));
    (m, *seq, focus.clone())
}

/// The late answer of the run `in_flight` started.
fn late(m: &mut Model, seq: u64, focus: pce::traffic::Focus) {
    let outcome = QueryOutcome {
        flows: m.traffic.loaded_flows()[..2].to_vec(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::from_millis(900),
        hit: false,
        filter: m.traffic.filter.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    quiet(
        m,
        Msg::FocusedDone {
            seq,
            result: Ok(FocusedOutcome {
                outcome,
                focus,
                queries: 2,
            }),
        },
    );
    assert!(
        m.traffic.focus_result.is_none(),
        "the late answer is dropped"
    );
    assert!(m.traffic.unfocused.is_none());
    assert_eq!(
        m.traffic.loaded_flows().len(),
        30,
        "the unfocused result stays"
    );
}

#[test]
fn esc_during_a_focused_run_cancels_it_and_its_answer_never_revives_the_focus() {
    let (mut m, seq, focus) = in_flight();
    quiet(&mut m, kc(KeyCode::Esc));
    assert!(!m.traffic.is_running(), "Esc cancelled the focused run");
    assert_eq!(focused(&m), None);
    late(&mut m, seq, focus);
    assert_eq!(focused(&m), None);
    assert_eq!(m.traffic.flows.len(), 30);
}

#[test]
fn another_app_during_a_focused_run_cancels_it() {
    let (mut m, seq, focus) = in_flight();
    quiet(&mut m, focus_cmd("shop/prod"));
    assert!(!m.traffic.is_running());
    late(&mut m, seq, focus);
    assert_eq!(
        focused(&m),
        Some(("shop/prod".into(), FocusDirection::Both))
    );
}

#[test]
fn x_during_a_focused_run_cancels_it() {
    let (mut m, seq, focus) = in_flight();
    quiet(&mut m, key('X'));
    assert!(!m.traffic.is_running());
    late(&mut m, seq, focus);
    assert_eq!(focused(&m), Some(("crm/prod".into(), FocusDirection::In)));
    assert_eq!(
        m.traffic.flows.len(),
        7,
        "still narrowed by the new direction"
    );
}

#[test]
fn g_during_a_focused_run_cancels_it() {
    let (mut m, seq, focus) = in_flight();
    apply_group_keys(&mut m, vec!["app".into()], false);
    assert!(!m.traffic.is_running());
    late(&mut m, seq, focus);
    assert_eq!(focused(&m), None, "the old keys' focus is not revived");
}

#[test]
fn a_focused_answer_for_another_focus_is_dropped_even_on_the_current_seq() {
    let (mut m, seq, focus) = in_flight();
    // Bypass the cancel: the outcome guard alone must hold.
    m.traffic.focus = Some(focus.clone().with_direction(FocusDirection::Out));
    late(&mut m, seq, focus);
    assert!(!m.traffic.is_running());
    assert_eq!(focused(&m).unwrap().1, FocusDirection::Out);
}

#[test]
fn f_on_an_app_to_app_pair_takes_the_source_then_the_destination() {
    let mut m = model_with_flows();
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    assert_eq!(m.traffic.apps_state.by, By::Pairs);
    let fqdn = |g: &pce::traffic::FlowGroup| g.dst.join("/") == "fqdn-1.example/-";
    m.traffic.groups.select_where(fqdn);
    quiet(&mut m, key('F'));
    assert_eq!(focused(&m).unwrap().0, "shop/prod");
    m.traffic.groups.select_where(fqdn);
    quiet(&mut m, key('F'));
    assert_eq!(
        focused(&m).unwrap().0,
        "fqdn-1.example/-",
        "F again: the other side"
    );
}

#[test]
fn f_on_a_mesh_row_takes_the_source_then_the_destination() {
    let mut m = model_with_flows();
    m.traffic.view = TrafficView::Mesh;
    let nas = |r: &pce::traffic::MeshRow| r.src.join("/") == "unmanaged-nas/-";
    m.traffic.mesh.select_where(nas);
    quiet(&mut m, key('F'));
    assert_eq!(focused(&m).unwrap().0, "unmanaged-nas/-");
    m.traffic.mesh.select_where(nas);
    quiet(&mut m, key('F'));
    assert_eq!(focused(&m).unwrap().0, "shop/prod");
}

#[test]
fn f_on_a_treemap_tile_takes_its_source_group() {
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Treemap;
    let tm = &m.traffic.treemap;
    let target = tm.current().expect("a tile under the cursor");
    assert!(target.inner.is_none() && tm.zoom.is_none());
    let label = tm.tile(target).label.clone();
    assert!(!tm.tile(target).is_other(), "{label}");
    quiet(&mut m, key('F'));
    assert_eq!(
        focused(&m).unwrap().0,
        label,
        "the outer tile is a source group"
    );
    assert!(
        m.traffic
            .treemap
            .groups
            .iter()
            .all(|g| g.src.join("/") == label || g.dst.join("/") == label),
        "the treemap is narrowed to it"
    );
}
