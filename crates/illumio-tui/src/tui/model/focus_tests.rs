use super::*;
use crate::tui::test_support::{model_with_flows, model_with_rich_flows};
use pce::traffic::{Focus, FocusDirection};

fn app(s: &str) -> Vec<String> {
    s.split('/').map(str::to_string).collect()
}

fn keys() -> Vec<String> {
    vec!["app".into(), "env".into()]
}

fn data_of(t: &TrafficState, n: usize) -> TrafficData {
    TrafficData {
        flows: t.loaded_flows()[..n].to_vec(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::from_millis(900),
        cache_hit: false,
        fetched_at: Instant::now(),
        filter: t.filter.clone(),
        range: None,
        hrefs: Vec::new(),
        draft: None,
    }
}

#[test]
fn the_focus_narrows_every_view_and_clearing_brings_them_back() {
    let mut m = model_with_rich_flows();
    let t = &mut m.traffic;
    assert_eq!(t.flows.len(), 30);
    let crm = app("crm/prod");
    t.set_focus(Focus::new(crm.clone(), keys()).with_direction(FocusDirection::In));
    assert_eq!(t.flows.len(), 7);
    assert!(t.flows.rows().iter().all(|r| &*r.dst_group == "crm/prod"));
    assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| g.dst == crm));
    assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| r.dst == crm));
    assert!(t.volume_state.unfolded.is_empty());
    t.fold_all_volume();
    assert!(
        t.volume
            .rows()
            .iter()
            .all(|r| r.dst().is_none_or(|d| d == crm.as_slice()))
    );
    assert!(
        !t.blocked.is_empty() && t.blocked.rows().iter().all(|r| r.app == crm),
        "flow 6 is blocked into crm/prod"
    );
    assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Out));
    assert_eq!(t.flows.len(), 4);
    assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Both));
    assert_eq!(
        t.flows.len(),
        9,
        "flows 3 and 4 are crm/prod to itself: once"
    );
    assert!(t.clear_focus());
    assert_eq!(t.flows.len(), 30);
    assert!(!t.clear_focus(), "nothing left to clear");
    assert_eq!(t.cycle_focus_direction(), None);
}

#[test]
fn the_decision_filter_and_by_service_work_on_top_of_the_focus() {
    let mut m = model_with_rich_flows();
    let t = &mut m.traffic;
    t.set_focus(Focus::new(app("crm/prod"), keys()));
    t.apps_state.decisions = Decisions::BlockedOnly;
    t.apps_state.by_service = true;
    t.rebuild_pairs();
    let rows: Vec<String> = t
        .services
        .rows()
        .iter()
        .map(|g| {
            format!(
                "{}>{} {}",
                g.src.join("/"),
                g.dst.join("/"),
                g.service.clone().unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec!["crm/dev>crm/prod 3306/tcp"],
        "the one blocked flow of crm/prod"
    );
}

#[test]
fn f_takes_the_source_then_the_destination_and_in_sections_the_destination_first() {
    let mut m = model_with_flows();
    let t = &mut m.traffic;
    t.flows
        .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
    assert_eq!(
        t.row_apps(),
        vec![app("shop/prod"), app("fqdn-1.example/-")]
    );
    assert_eq!(t.next_focus_app(), Some(app("shop/prod")));
    t.set_focus(Focus::new(app("shop/prod"), keys()));
    t.flows
        .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
    assert_eq!(
        t.next_focus_app(),
        Some(app("fqdn-1.example/-")),
        "a second F switches sides"
    );
    t.flows.select_where(|r| r.flow.num_connections == 310);
    assert_eq!(
        t.row_apps(),
        vec![app("shop/prod")],
        "a flow to itself has one app"
    );
    t.view = TrafficView::Blocked;
    t.blocked.select_where(|r| !r.is_header());
    assert_eq!(
        t.row_apps()[0],
        app("shop/prod"),
        "Blocked is in: destination first"
    );
    t.blocked.select_where(|r| r.is_header());
    assert_eq!(t.row_apps(), vec![app("shop/prod")], "a header is its app");
    t.view = TrafficView::Mesh;
    t.mesh
        .select_where(|r| r.dst.join("/") == "fqdn-1.example/-");
    assert_eq!(
        t.row_apps(),
        vec![app("shop/prod"), app("fqdn-1.example/-")],
        "a Mesh row: the source, then the destination"
    );
    t.mesh.select_where(|r| r.src == r.dst);
    assert_eq!(t.row_apps(), vec![app("shop/prod")], "to itself: one app");
    t.flows.set_rows(Vec::new());
    t.view = TrafficView::Flows;
    assert!(t.row_apps().is_empty());
    assert_eq!(t.next_focus_app(), None);
    assert_eq!(t.loaded_apps(), vec!["shop/prod"]);
}

#[test]
fn a_focus_on_a_truncated_result_is_partial_until_its_focused_query_is_loaded() {
    let mut m = model_with_flows();
    let t = &mut m.traffic;
    t.data.as_mut().unwrap().truncated = true;
    let f = Focus::new(app("shop/prod"), keys());
    t.set_focus(f.clone());
    assert!(t.focus_partial());
    assert_eq!(t.focus_requery(), Some(f.clone()));
    assert_eq!(
        t.focus_meta().as_deref(),
        Some("focus is partial: r re-queries shop/prod · both")
    );
    let focused = data_of(t, 2);
    t.apply_focused(f.clone(), 2, focused);
    assert!(!t.focus_partial());
    assert_eq!(
        t.focus_meta().as_deref(),
        Some("focus shop/prod · both · 2 queries")
    );
    assert_eq!(t.loaded_flows().len(), 2);
    assert_eq!(
        t.focus_requery(),
        Some(f.clone()),
        "r refreshes the focused result"
    );
    assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::In));
    assert!(!t.focus_partial(), "a both result covers in");
    assert!(t.clear_focus());
    assert_eq!(t.loaded_flows().len(), 4, "the unfocused result is back");
    assert!(t.data.as_ref().unwrap().truncated);
    assert!(t.unfocused.is_none() && t.focus_result.is_none());
}

#[test]
fn a_focused_result_of_another_app_goes_back_first_and_window_keys_go_back_too() {
    let mut m = model_with_flows();
    let t = &mut m.traffic;
    t.data.as_mut().unwrap().truncated = true;
    let f = Focus::new(app("shop/prod"), keys()).with_direction(FocusDirection::In);
    let focused = data_of(t, 1);
    t.apply_focused(f, 1, focused);
    assert_eq!(t.loaded_flows().len(), 1);
    t.set_focus(Focus::new(app("unmanaged-nas/-"), keys()));
    assert!(
        t.unfocused.is_none(),
        "another app: the unfocused result is back first"
    );
    assert_eq!(t.loaded_flows().len(), 4);
    assert!(t.focus_partial(), "still truncated: partial");
    assert!(!t.back_to_unfocused(), "nothing focused to go back from");
    let f = Focus::new(app("shop/prod"), keys());
    let focused = data_of(t, 2);
    t.apply_focused(f, 2, focused);
    assert!(t.back_to_unfocused());
    assert_eq!(t.loaded_flows().len(), 4);
    assert!(t.focus.is_some(), "the focus stays");
    assert!(t.drop_focus(), "drop_focus reports the focus it dropped");
    assert!(t.focus.is_none() && t.unfocused.is_none());
}

#[test]
fn queries_text_counts() {
    assert_eq!(queries_text(1), "1 query");
    assert_eq!(queries_text(2), "2 queries");
}

#[test]
fn the_focused_flows_are_computed_once_per_focus_or_data_change() {
    let mut m = model_with_rich_flows();
    let t = &mut m.traffic;
    assert!(
        std::ptr::eq(t.view_flows().as_ptr(), t.loaded_flows().as_ptr()),
        "no focus: the loaded flows, borrowed"
    );
    t.set_focus(Focus::new(app("crm/prod"), keys()).with_direction(FocusDirection::In));
    assert_eq!(t.view_flows().len(), 7, "focus set");
    let cached = t.view_flows().as_ptr();
    t.rebuild_pairs();
    assert_eq!(
        t.view_flows().as_ptr(),
        cached,
        "a rebuild of one view reuses it"
    );
    t.cycle_focus_direction();
    assert_eq!(t.view_flows().len(), 4, "X: recomputed");
    let more = t.loaded_flows()[..3].to_vec();
    t.data.as_mut().unwrap().flows = more;
    t.rebuild_views();
    assert_eq!(
        t.view_flows().len(),
        t.loaded_flows()
            .iter()
            .filter(|f| t.focus.as_ref().unwrap().keeps(f))
            .count(),
        "new data: recomputed"
    );
    assert!(t.drop_focus());
    assert_eq!(
        t.view_flows().len(),
        3,
        "dropped without a rebuild: every flow"
    );
    let data = data_of(t, 3);
    t.apply_focused(Focus::new(app("crm/prod"), keys()), 2, data);
    assert!(t.restore_unfocused());
    assert_eq!(
        t.view_flows().len(),
        t.loaded_flows()
            .iter()
            .filter(|f| t.focus.as_ref().unwrap().keeps(f))
            .count(),
        "the unfocused result back: recomputed without a rebuild"
    );
}

#[test]
fn a_truncated_result_names_its_cap_and_claims_no_span() {
    let mut m = model_with_flows();
    let d = m.traffic.data.as_mut().unwrap();
    let n = d.flows.len();
    assert_eq!(d.count_text(), format!("{n} flows"));
    d.truncated = true;
    d.filter.max_results = 1000;
    assert_eq!(
        d.count_text(),
        format!("{n} flows, capped at 1000; B raises the cap")
    );
}

#[test]
fn f_on_a_peer_row_of_both_sections_takes_its_sections_app_first() {
    let mut m = model_with_flows();
    let t = &mut m.traffic;
    t.view = TrafficView::Aggregate;
    t.apps_state.by = By::Both;
    t.rebuild_sections();
    let peer = |d: Direction, text: &'static str| {
        move |r: &SectionRow| {
            r.peer()
                .is_some_and(|(rd, p)| rd == d && p.peer_text() == text)
        }
    };
    t.apps.select_where(peer(Direction::In, "unmanaged-nas/-"));
    assert_eq!(
        t.row_apps(),
        vec![app("shop/prod"), app("unmanaged-nas/-")],
        "an inbound section: its app (the destination) first"
    );
    t.apps
        .select_where(peer(Direction::Out, "fqdn-1.example/-"));
    assert_eq!(
        t.row_apps(),
        vec![app("shop/prod"), app("fqdn-1.example/-")],
        "an outbound section: its app (the source) first"
    );
}
