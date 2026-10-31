use super::*;

#[test]
fn v_cycles_views_and_g_sets_group_keys_without_requests() {
    use crate::tui::model::{Overlay, TrafficView};
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert!(update(&mut m, key('v')).is_empty());
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    assert!(
        m.traffic.groups.len() >= 3,
        "groups built from data: {}",
        m.traffic.groups.len()
    );
    assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
    assert!(update(&mut m, key('g')).is_empty());
    assert!(matches!(m.overlay, Overlay::GroupKeys { .. }));
    for _ in 0..7 {
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            )
            .is_empty()
        );
    }
    for c in "role".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .is_empty()
    );
    assert_eq!(m.traffic.group_keys, vec!["role"]);
    assert!(
        m.traffic.groups.rows().iter().any(|g| g.src == vec!["web"]),
        "{:?}",
        m.traffic.groups.rows()
    );
    assert!(update(&mut m, key('v')).is_empty());
    assert_eq!(m.traffic.view, TrafficView::Mesh);
    assert!(update(&mut m, key('v')).is_empty());
    assert_eq!(m.traffic.view, TrafficView::Blocked);
    let headers = |m: &Model| {
        m.traffic
            .blocked
            .rows()
            .iter()
            .filter(|r| r.is_header())
            .count()
    };
    assert_eq!(
        (headers(&m), m.traffic.blocked.len()),
        (2, 4),
        "two destination roles, one blocked source each"
    );
    assert!(update(&mut m, key('v')).is_empty());
    assert_eq!(m.traffic.view, TrafficView::Volume);
    assert!(update(&mut m, key('v')).is_empty());
    assert_eq!(m.traffic.view, TrafficView::Flows);
}

#[test]
fn g_opens_the_picker_and_enter_regroups_every_view_without_requests() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    assert_eq!(
        m.traffic.group_keys,
        vec!["app", "env"],
        "the profile's scope keys"
    );
    assert_eq!(m.traffic.flows.rows()[0].src_group.as_ref(), "shop/prod");
    quiet(&mut m, key('g'));
    let Overlay::GroupKeys(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(listed, vec!["app", "env", "role", "datacenter", "loc"]);
    assert_eq!(p.entries[3].seen, Some(11));
    for msg in [key('j'), key('j'), key(' '), enter()] {
        assert!(update(&mut m, msg).is_empty(), "the picker never requests");
    }
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.traffic.group_keys, vec!["app", "env", "role"]);
    assert_eq!(m.status.text, "grouped by app,env,role");
    assert_eq!(
        m.traffic.flows.rows()[0].src_group.as_ref(),
        "shop/prod/web"
    );
    assert_eq!(m.traffic.flows.rows()[0].dst_group.as_ref(), "shop/prod/db");
    assert!(m.traffic.groups.rows().iter().all(|g| g.src.len() == 3));
    assert!(m.traffic.blocked.rows().iter().all(|b| b.app.len() == 3));
    // A non-workload endpoint shows `-` in every slot.
    let corp = m
        .traffic
        .flows
        .rows()
        .iter()
        .find(|r| r.src.ip_lists == vec!["Corp"])
        .unwrap();
    assert_eq!(corp.src_group.as_ref(), "-/-/-");
}

#[test]
fn all_labels_groups_by_every_seen_key_until_a_profile_switch() {
    use crate::tui::model::{Limits, ProfileInfo};
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('g'));
    for _ in 0..5 {
        quiet(&mut m, key('j'));
    }
    quiet(&mut m, key(' '));
    quiet(&mut m, enter());
    assert!(m.traffic.all_keys);
    assert_eq!(
        m.traffic.effective_keys(),
        vec!["app", "env", "role", "datacenter", "loc"],
        "scope keys, tier key, then alphabetical"
    );
    assert!(m.traffic.groups.rows().iter().all(|g| g.src.len() == 5));
    assert_eq!(
        m.traffic.flows.rows()[4].src_group.as_ref(),
        "crm/prod/web/fra1/us"
    );
    // Reopening shows `all labels` checked and no key.
    quiet(&mut m, key('g'));
    let Overlay::GroupKeys(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert!(p.all && !p.is_checked("app"));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        update(
            &mut m,
            Msg::ProfileSwitched(Ok((
                ProfileInfo {
                    name: "prod".into(),
                    host: "prod.example".into(),
                    org_id: 9,
                },
                Limits {
                    auto_refresh_min_seconds: 60,
                    traffic_max_results: 500,
                    traffic_default_window: "1h".into(),
                    traffic_auto_backfill: false,
                    scope_keys: vec!["site".into()],
                    tier_key: "tier".into(),
                    ignore_keys: Vec::new(),
                    ruleset_name: "RF-{site}".into(),
                    consolidate_min_peers: 5,
                    event_exclude_types: Vec::new(),
                    exposure_services: Ok(pce::traffic::default_services()),
                },
            ))),
        ),
        vec![Cmd::Connect]
    );
    assert_eq!(
        m.traffic.group_keys,
        vec!["site"],
        "the new profile's label model"
    );
    assert!(!m.traffic.all_keys);
}

#[test]
fn all_labels_lists_the_profile_scope_and_tier_keys_first_then_alphabetical() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.set_label_model(vec!["bu".into(), "app".into()], "tier".into(), Vec::new());
    // The flows carry `bu` and `tier` in place of `loc` and `role`.
    if let Some(d) = m.traffic.data.as_mut() {
        for f in &mut d.flows {
            for l in f.src.labels.iter_mut().chain(f.dst.labels.iter_mut()) {
                match l.key.as_str() {
                    "loc" => l.key = "bu".into(),
                    "role" => l.key = "tier".into(),
                    _ => {}
                }
            }
        }
    }
    quiet(&mut m, key('g'));
    let Overlay::GroupKeys(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    for _ in 0..p.all_row() {
        quiet(&mut m, key('j'));
    }
    quiet(&mut m, key(' '));
    quiet(&mut m, enter());
    assert!(m.traffic.all_keys);
    assert_eq!(
        m.traffic.effective_keys(),
        vec!["bu", "app", "tier", "datacenter", "env"]
    );
}

#[test]
fn ignored_keys_stay_out_of_the_picker_and_all_labels_but_can_be_typed() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.set_label_model(
        vec!["app".into(), "env".into()],
        "role".into(),
        vec!["datacenter".into()],
    );
    quiet(&mut m, key('g'));
    let Overlay::GroupKeys(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(listed, vec!["app", "env", "role", "loc"]);
    for _ in 0..p.all_row() {
        quiet(&mut m, key('j'));
    }
    quiet(&mut m, key(' '));
    quiet(&mut m, enter());
    assert!(m.traffic.all_keys);
    assert_eq!(
        m.traffic.effective_keys(),
        vec!["app", "env", "role", "loc"]
    );
    // Typed by hand, an ignored key still groups.
    quiet(&mut m, key('g'));
    let Overlay::GroupKeys(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let to_text = p.text_row() - p.cursor;
    for _ in 0..to_text {
        quiet(&mut m, key('j'));
    }
    for c in "datacenter".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, enter());
    assert!(!m.traffic.all_keys);
    assert_eq!(m.traffic.effective_keys(), vec!["datacenter"]);
    assert!(
        m.traffic
            .flows
            .rows()
            .iter()
            .any(|r| &*r.src_group == "fra1" || &*r.dst_group == "fra1"),
        "grouping by an ignored key still reads its values"
    );
}

#[test]
fn the_flow_detail_hides_ignored_keys() {
    use crate::tui::test_support::{model_with_flows, render_to_string};
    let mut m = model_with_flows();
    m.set_label_model(vec!["app".into()], "role".into(), vec!["env".into()]);
    m.traffic.show_detail = true;
    // Stacked (80 columns): the labels line.
    let screen = render_to_string(&m, 80, 30);
    let line = screen
        .lines()
        .find(|l| l.contains("src labels"))
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(line.contains("app=shop role=web"), "{line}");
    assert!(!line.contains("env="), "{line}");
    // In columns (160 columns): one label per line, no `env`.
    let screen = render_to_string(&m, 160, 50);
    assert!(screen.contains("│role  web"), "{screen}");
    assert!(screen.contains("│app   shop"), "{screen}");
    assert!(!screen.contains("env   prod"), "{screen}");
}

#[test]
fn esc_keeps_the_keys_and_the_drill_and_applying_clears_the_drill() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('v'));
    update(&mut m, enter()); // crm/prod -> crm/prod
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(m.traffic.flows.len(), 2);
    quiet(&mut m, key('g'));
    update(&mut m, key(' ')); // app off
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
    assert!(m.traffic.drill.is_some());
    quiet(&mut m, key('g'));
    quiet(&mut m, enter());
    assert!(m.traffic.drill.is_none(), "applying drops the drill-down");
    assert_eq!(m.traffic.flows.len(), 30);
    // An empty text falls back to the profile's scope keys.
    quiet(&mut m, key('g'));
    for _ in 0..7 {
        quiet(&mut m, kc(KeyCode::Backspace));
    }
    quiet(&mut m, enter());
    assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
    // Unknown keys are allowed and show `-`.
    quiet(&mut m, key('g'));
    for c in ",nope".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, enter());
    assert_eq!(m.traffic.flows.rows()[0].src_group.as_ref(), "shop/prod/-");
}

#[test]
fn w_on_a_blocked_row_drafts_from_the_first_flow_behind_it() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = writes_on(model_with_rich_flows());
    m.traffic.view = TrafficView::Blocked;
    let row = m.traffic.blocked.selected_item().unwrap().clone();
    assert!(row.is_header(), "{row:?}");
    assert!(update(&mut m, key('w')).is_empty(), "no request");
    assert_eq!(m.overlay, Overlay::None, "a header holds a whole app");
    assert_eq!(
        m.status.text,
        "select a peer row (a header holds a whole app)"
    );
    quiet(&mut m, key('j'));
    let (_, peer) = m.traffic.blocked.selected_item().unwrap().peer().unwrap();
    let service = peer.service.clone();
    assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.title, "Allow rule from flow");
    assert_eq!(f.value("Services"), service);
    quiet(&mut m, kc(KeyCode::Esc));
    m.traffic.blocked.set_rows(Vec::new());
    quiet(&mut m, key('w'));
    assert_eq!(m.status.text, "no row selected");
}

#[test]
fn mesh_sits_between_app_to_app_and_blocked_and_u_p_regroup_without_requests() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    let mut seen = Vec::new();
    for _ in 0..5 {
        assert!(update(&mut m, key('v')).is_empty());
        seen.push(m.traffic.view);
    }
    assert_eq!(
        seen,
        vec![
            TrafficView::Aggregate,
            TrafficView::Mesh,
            TrafficView::Blocked,
            TrafficView::Volume,
            TrafficView::Flows
        ]
    );
    quiet(&mut m, key('v'));
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Mesh);
    assert_eq!(m.traffic.mesh.len(), 30);
    quiet(&mut m, key('j'));
    quiet(&mut m, key('s'));
    assert!(update(&mut m, key('u')).is_empty(), "no request");
    assert!(!m.traffic.mesh_cols.user);
    assert_eq!(m.traffic.mesh.len(), 29);
    assert_eq!(m.traffic.mesh.selected_pos(), 0, "back on the first row");
    assert_eq!(m.traffic.mesh.sort(), None, "a new table");
    assert!(update(&mut m, key('p')).is_empty());
    assert_eq!(m.traffic.mesh.len(), 28);
    assert_eq!(m.traffic.mesh.columns[0].title, "src");
    quiet(&mut m, key('u'));
    quiet(&mut m, key('p'));
    assert_eq!(m.traffic.mesh.len(), 30);
    // u and p mean nothing on the other views.
    quiet(&mut m, key('v'));
    assert!(update(&mut m, key('u')).is_empty());
    assert!(m.traffic.mesh_cols.user);
}

#[test]
fn enter_on_a_mesh_row_drills_to_its_flows_and_esc_clears() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Mesh;
    quiet(&mut m, key('u'));
    quiet(&mut m, key('p'));
    let row = m
        .traffic
        .mesh
        .rows()
        .iter()
        .position(|r| r.flows == 3)
        .unwrap();
    for _ in 0..row {
        quiet(&mut m, key('j'));
    }
    assert!(update(&mut m, enter()).is_empty());
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(
        m.traffic.flows.len(),
        3,
        "java, batch and python3 to shop-db-p"
    );
    assert!(m.traffic.flows.rows().iter().all(|r| r.port == Some(5432)));
    assert!(matches!(m.traffic.drill, Some(Drill::Mesh { .. })));
    assert_eq!(
        m.traffic.drill.as_ref().unwrap().title(),
        "shop/prod → shop/prod 5432/tcp"
    );
    assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
    assert!(m.traffic.drill.is_none());
    assert_eq!(m.traffic.flows.len(), 30);
}

#[test]
fn w_on_a_mesh_row_drafts_from_its_first_blocked_flow() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = writes_on(model_with_rich_flows());
    m.traffic.view = TrafficView::Mesh;
    assert!(update(&mut m, key('w')).is_empty());
    assert_eq!(m.status.text, "this flow is allowed; nothing to add");
    let pos = m
        .traffic
        .mesh
        .rows()
        .iter()
        .position(|r| r.blocked > 0)
        .unwrap();
    for _ in 0..pos {
        quiet(&mut m, key('j'));
    }
    assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.title, "Allow rule from flow");
}

/// The rich fixture on App-to-app `pairs` with `o` on.
fn on_pairs_by_service(m: Model) -> Model {
    let mut m = m;
    m.traffic.view = TrafficView::Aggregate;
    assert!(update(&mut m, key('o')).is_empty(), "no request");
    m
}

#[test]
fn o_groups_the_pairs_by_service_without_a_request_and_holds() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = on_pairs_by_service(model_with_rich_flows());
    let t = &m.traffic;
    assert!(t.apps_state.by_service);
    let titles: Vec<&str> = t.pairs_table().columns.iter().map(|c| c.title).collect();
    assert_eq!(
        titles,
        [
            "src", "dst", "service", "flows", "conns", "bytes", "decision", "allow.c", "pot.c",
            "block.c"
        ]
    );
    let tuples = t.services.rows();
    assert!(tuples.len() > t.groups.len(), "some pair has two services");
    assert!(tuples.iter().all(|g| g.service.is_some()));
    let conns = |rows: &[pce::traffic::FlowGroup]| rows.iter().map(|g| g.connections).sum::<u64>();
    assert_eq!(conns(tuples), conns(t.groups.rows()));
    assert!(m.status.text.starts_with("pairs · by service"));
    assert!(
        t.groups.rows().iter().all(|g| g.service.is_none()),
        "the pairs stay"
    );

    // s and / act on the table on screen.
    quiet(&mut m, key('s'));
    assert!(m.traffic.services.sort().is_some());
    assert!(m.traffic.groups.sort().is_none());
    quiet(&mut m, key('/'));
    assert!(m.traffic.is_searching());
    for c in "5432".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, enter());
    let shown = m.traffic.services.visible().len();
    assert!(
        shown > 0 && shown < m.traffic.services.rows().len(),
        "{shown}"
    );

    // Held across x and a regroup; in, out and both are already per service.
    quiet(&mut m, key('x'));
    assert!(update(&mut m, key('o')).is_empty());
    assert!(
        m.traffic.apps_state.by_service,
        "o does nothing outside pairs"
    );
    assert_eq!(m.status.text, "o: pairs only (in/out/both are per service)");
    for _ in 0..3 {
        quiet(&mut m, key('x'));
    }
    assert_eq!(m.traffic.apps_state.by, By::Pairs);
    m.traffic.rebuild_views();
    assert!(!m.traffic.services.is_empty());

    assert!(update(&mut m, key('o')).is_empty());
    assert!(!m.traffic.apps_state.by_service);
    assert!(m.traffic.services.is_empty());
    assert_eq!(
        m.traffic.pairs_table().columns[2].title,
        "flows",
        "today's pairs"
    );
}

#[test]
fn enter_on_a_pair_by_service_drills_to_that_service_only() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = on_pairs_by_service(model_with_rich_flows());
    let pos = m
        .traffic
        .services
        .rows()
        .iter()
        .position(|g| g.flows > 1)
        .unwrap();
    for _ in 0..pos {
        quiet(&mut m, key('j'));
    }
    let row = m.traffic.services.selected_item().unwrap().clone();
    let service = row.service.clone().unwrap();
    assert!(update(&mut m, enter()).is_empty(), "no request");
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(m.traffic.flows.len() as u64, row.flows);
    assert!(
        m.traffic
            .flows
            .rows()
            .iter()
            .all(|r| r.flow.service_text() == service)
    );
    assert_eq!(
        m.traffic.drill.as_ref().unwrap().title(),
        format!("{} → {} {service}", row.src.join("/"), row.dst.join("/"))
    );
}

fn flows_of(rows: &[pce::traffic::FlowGroup]) -> u64 {
    rows.iter().map(|g| g.flows).sum()
}

#[test]
fn d_filters_the_pairs_and_by_service_without_a_request_and_spares_volume() {
    use crate::tui::test_support::{model_with_rich_flows, render_to_string};
    use pce::traffic::Decisions;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Aggregate;
    let volume = m.traffic.volume.rows().to_vec();
    assert_eq!(flows_of(m.traffic.groups.rows()), 30);
    let allowed = m
        .traffic
        .groups
        .rows()
        .iter()
        .find(|g| g.blocked + g.potentially_blocked == 0)
        .expect("a pair with only allowed flows")
        .clone();

    assert!(update(&mut m, key('d')).is_empty(), "no request");
    assert_eq!(m.status.text, "decision blocked");
    let t = &m.traffic;
    assert_eq!(t.apps_state.decisions, Decisions::Blocked);
    let rows = t.groups.rows();
    assert_eq!(flows_of(rows), 14);
    assert!(rows.iter().all(|g| g.allowed == 0 && g.flows > 0));
    assert!(
        rows.iter().any(|g| g.potentially_blocked > 0),
        "pot.blocked kept"
    );
    assert!(
        !rows
            .iter()
            .any(|g| g.src == allowed.src && g.dst == allowed.dst),
        "an allowed-only pair disappears"
    );
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("pairs · blocked · keys"), "{out}");

    // By service: the same filter, held across o.
    quiet(&mut m, key('o'));
    let rows = m.traffic.services.rows();
    assert_eq!(flows_of(rows), 14);
    assert!(rows.iter().all(|g| g.allowed == 0));
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("pairs · by service · blocked · keys"), "{out}");

    assert!(update(&mut m, key('d')).is_empty(), "no request");
    assert_eq!(m.status.text, "decision blocked only");
    for rows in [m.traffic.services.rows(), m.traffic.groups.rows()] {
        assert_eq!(flows_of(rows), 7);
        assert!(
            rows.iter()
                .all(|g| g.allowed == 0 && g.potentially_blocked == 0)
        );
    }
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("by service · blocked only"), "{out}");

    quiet(&mut m, key('d'));
    assert_eq!(flows_of(m.traffic.services.rows()), 30);
    assert_eq!(flows_of(m.traffic.groups.rows()), 30);
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("pairs · by service · keys"), "{out}");

    // Volume kept every flow throughout, and a regroup keeps it so.
    quiet(&mut m, key('d'));
    m.traffic.rebuild_views();
    assert_eq!(flows_of(m.traffic.groups.rows()), 14);
    assert_eq!(m.traffic.volume.rows(), volume.as_slice());
    let flows: u64 = volume.iter().map(|r| r.node.group.flows).sum();
    assert_eq!(flows, 30);
}

#[test]
fn enter_on_a_filtered_pair_drills_to_the_flows_it_counts() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Aggregate;
    let mixed: Vec<(Vec<String>, Vec<String>)> = m
        .traffic
        .groups
        .rows()
        .iter()
        .filter(|g| g.allowed > 0 && g.blocked + g.potentially_blocked > 0)
        .map(|g| (g.src.clone(), g.dst.clone()))
        .collect();
    assert!(!mixed.is_empty(), "a pair with allowed and blocked flows");
    quiet(&mut m, key('d'));
    let pos = m
        .traffic
        .groups
        .visible()
        .iter()
        .position(|&i| {
            let g = &m.traffic.groups.rows()[i];
            mixed.contains(&(g.src.clone(), g.dst.clone()))
        })
        .unwrap();
    for _ in 0..pos {
        quiet(&mut m, key('j'));
    }
    let row = m.traffic.groups.selected_item().unwrap().clone();
    assert!(update(&mut m, enter()).is_empty(), "no request");
    assert_eq!(m.traffic.flows.len() as u64, row.flows);
    assert!(
        m.traffic
            .flows
            .rows()
            .iter()
            .all(|r| r.flow.policy_decision != PolicyDecision::Allowed)
    );
    assert_eq!(
        m.traffic.drill.as_ref().unwrap().title(),
        format!("{} → {} · blocked", row.src.join("/"), row.dst.join("/"))
    );

    // By service under blocked only.
    quiet(&mut m, kc(KeyCode::Esc));
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('o'));
    quiet(&mut m, key('d'));
    let row = m.traffic.services.selected_item().unwrap().clone();
    assert!(update(&mut m, enter()).is_empty(), "no request");
    assert_eq!(m.traffic.flows.len() as u64, row.flows);
    assert!(
        m.traffic
            .flows
            .rows()
            .iter()
            .all(|r| r.flow.policy_decision == PolicyDecision::Blocked)
    );
    assert!(
        m.traffic
            .drill
            .as_ref()
            .unwrap()
            .title()
            .ends_with(" · blocked only")
    );
}

#[test]
fn w_on_a_pair_by_service_drafts_under_every_decision_filter() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = on_pairs_by_service(writes_on(model_with_rich_flows()));
    for _ in 0..3 {
        let pred = labelled_blocked(&m);
        let target = m
            .traffic
            .services
            .rows()
            .iter()
            .find(|g| pred(g))
            .unwrap()
            .clone();
        drop(pred);
        select_service_pair(&mut m, |g| *g == target);
        let (_, _, services, row) = w_form_of_row(&mut m);
        assert_eq!(Some(services), row.service);
        quiet(&mut m, key('d'));
    }
}

/// Moves the cursor to the first by-service row `pred` accepts.
fn select_service_pair(m: &mut Model, pred: impl Fn(&pce::traffic::FlowGroup) -> bool) {
    m.traffic.services.home();
    let pos = (0..m.traffic.services.len())
        .find(|&i| pred(&m.traffic.services.rows()[m.traffic.services.visible()[i]]))
        .expect("a matching row");
    for _ in 0..pos {
        quiet(m, key('j'));
    }
}

/// A blocked row both of whose groups are labelled workloads under
/// the keys in force (no `-` slot, no host-name fallback).
fn labelled_blocked(m: &Model) -> impl Fn(&pce::traffic::FlowGroup) -> bool + '_ {
    let keys = m.traffic.effective_keys();
    move |g| {
        g.blocked + g.potentially_blocked > 0
            && m.traffic.loaded_flows().iter().any(|f| {
                g.contains(f, &keys)
                    && keys
                        .iter()
                        .all(|k| f.src.label_value(k).is_some() && f.dst.label_value(k).is_some())
            })
    }
}

/// `app=shop, env=prod`: a group under `keys` as actor tokens.
fn group_tokens(keys: &[String], group: &[String]) -> String {
    keys.iter()
        .zip(group)
        .filter(|(_, v)| *v != "-")
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `w` on the selected by-service row: the form's Providers,
/// Consumers and Services, and the row.
fn w_form_of_row(m: &mut Model) -> (String, String, String, pce::traffic::FlowGroup) {
    let row = m.traffic.services.selected_item().unwrap().clone();
    // The draft rulesets, once: none while that load is on its way.
    let loading = m.policy.loading;
    let want = if loading {
        Vec::new()
    } else {
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use,
        }]
    };
    assert_eq!(update(m, key('w')), want);
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?} {}", m.overlay, m.status.text)
    };
    assert_eq!(f.title, "Allow rule from flow");
    let out = (
        f.value("Providers").to_string(),
        f.value("Consumers").to_string(),
        f.value("Services").to_string(),
        row,
    );
    quiet(m, kc(KeyCode::Esc));
    out
}

#[test]
fn w_on_a_pair_by_service_drafts_the_rows_groups_and_service() {
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = writes_on(model_with_rich_flows());
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('w'));
    assert_eq!(
        m.status.text, "select a flow in the Flows view (Enter on a pair)",
        "plain pairs hold several services"
    );
    let mut m = on_pairs_by_service(m);

    // The default keys: the row's app/env groups, not one flow's tier.
    let keys = m.traffic.effective_keys();
    let pred = labelled_blocked(&m);
    let target = m
        .traffic
        .services
        .rows()
        .iter()
        .find(|g| pred(g))
        .unwrap()
        .clone();
    drop(pred);
    select_service_pair(&mut m, |g| *g == target);
    let (providers, consumers, services, row) = w_form_of_row(&mut m);
    assert_eq!(providers, group_tokens(&keys, &row.dst));
    assert_eq!(consumers, group_tokens(&keys, &row.src));
    assert_eq!(Some(services), row.service);

    // A row whose blocked flows differ in tier still gets its groups.
    let keep = |f: &pce::traffic::ResolvedFlow| {
        row.contains(f, &keys)
            && matches!(
                f.policy_decision,
                PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
            )
    };
    let mut odd = m
        .traffic
        .loaded_flows()
        .iter()
        .find(|f| keep(f))
        .unwrap()
        .clone();
    for side in [&mut odd.src, &mut odd.dst] {
        for l in side.labels.iter_mut().filter(|l| l.key == "role") {
            l.value = "batch-tier".into();
            l.href = "/orgs/5/labels/9999".into();
        }
    }
    m.traffic.data.as_mut().unwrap().flows.insert(0, odd);
    m.traffic.rebuild_views();
    select_service_pair(&mut m, |g| {
        g.src == row.src && g.dst == row.dst && g.service == row.service
    });
    let (p2, c2, _, _) = w_form_of_row(&mut m);
    assert_eq!((p2, c2), (providers, consumers), "group level, not tier");

    // A non-scope `g` key: the row's loc groups.
    m.traffic.group_keys = vec!["loc".to_string()];
    m.traffic.rebuild_views();
    let loc = vec!["loc".to_string()];
    assert_eq!(m.traffic.effective_keys(), loc);
    let pred = labelled_blocked(&m);
    let target = m
        .traffic
        .services
        .rows()
        .iter()
        .find(|g| pred(g))
        .unwrap()
        .clone();
    drop(pred);
    select_service_pair(&mut m, |g| *g == target);
    let (providers, consumers, services, row) = w_form_of_row(&mut m);
    assert!(providers.starts_with("loc="), "{providers}");
    assert_eq!(providers, group_tokens(&loc, &row.dst));
    assert_eq!(consumers, group_tokens(&loc, &row.src));
    assert_eq!(Some(services), row.service);

    // Nothing blocked behind the row: nothing to add.
    select_service_pair(&mut m, |g| g.blocked + g.potentially_blocked == 0);
    assert!(update(&mut m, key('w')).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.status.text,
        "every flow of this row is allowed; nothing to add"
    );
}

#[test]
fn export_of_pairs_by_service_is_traffic_groups_services_csv() {
    use crate::cli::output::TableRows;
    use crate::cli::traffic::ServiceGroups;
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = on_pairs_by_service(model_with_rich_flows());
    let (_, _, headers, rows) = export_cmd(&mut m, "export p.csv");
    let cli = ServiceGroups(m.traffic.services.rows().to_vec());
    assert_eq!(
        headers,
        cli.full_headers(),
        "the CLI's --services --csv header"
    );
    assert_eq!(
        headers,
        [
            "src",
            "dst",
            "service",
            "flows",
            "conns",
            "bytes",
            "allowed",
            "pot.blocked",
            "blocked",
            "allowed_conns",
            "pot_blocked_conns",
            "blocked_conns",
            "unknown_conns"
        ]
    );
    assert_eq!(rows.len(), m.traffic.services.len());
    let first = m.traffic.services.selected_item().unwrap();
    assert_eq!(rows[0][2], first.service.clone().unwrap());
    assert_eq!(rows[0][5], first.bytes.to_string(), "bytes whole");
    let mixed = m
        .traffic
        .services
        .rows()
        .iter()
        .find(|g| g.blocked > 0 && g.allowed + g.potentially_blocked > 0)
        .or_else(|| m.traffic.services.rows().iter().find(|g| g.blocked > 0))
        .unwrap();
    let line = rows
        .iter()
        .find(|r| {
            r[0] == mixed.src.join("/")
                && r[1] == mixed.dst.join("/")
                && Some(&r[2]) == mixed.service.as_ref()
        })
        .unwrap();
    assert_eq!(line[8], mixed.blocked.to_string(), "blocked counted");
    assert_eq!(line[6], mixed.allowed.to_string());

    assert_eq!(line[11], mixed.conns.blocked_conns.to_string());

    // On screen: the compact decision column.
    let titles: Vec<&str> = m.traffic.services.columns.iter().map(|c| c.title).collect();
    assert!(titles.contains(&"decision"));
}

#[test]
fn export_of_pairs_is_traffic_groups_csv() {
    use crate::cli::output::TableRows;
    use crate::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Aggregate;
    m.traffic.rebuild_views();
    let (_, _, headers, rows) = export_cmd(&mut m, "export p.csv");
    let shown: Vec<pce::traffic::FlowGroup> = m
        .traffic
        .groups
        .visible()
        .iter()
        .map(|&i| m.traffic.groups.rows()[i].clone())
        .collect();
    assert_eq!(headers, shown.full_headers(), "the CLI's --csv header");
    assert_eq!(rows, shown.full_rows(), "the CLI's --csv rows");
    assert_eq!(
        headers,
        [
            "src",
            "dst",
            "flows",
            "conns",
            "bytes",
            "allowed",
            "pot.blocked",
            "blocked",
            "allowed_conns",
            "pot_blocked_conns",
            "blocked_conns",
            "unknown_conns"
        ]
    );
    assert!(!rows.is_empty());
}

fn on_volume(w: u16, h: u16) -> Model {
    let mut m = crate::tui::test_support::model_with_rich_flows();
    quiet(&mut m, Msg::Resize(w, h));
    for _ in 0..4 {
        quiet(&mut m, key('v'));
    }
    assert_eq!(m.traffic.view, TrafficView::Volume);
    m
}

/// The Volume rows as text: a source by its name, a destination `→ dst`.
fn volume_rows(m: &Model) -> Vec<String> {
    let t = &m.traffic.volume;
    t.visible()
        .iter()
        .map(|&i| {
            let r = &t.rows()[i];
            match r.dst() {
                Some(d) => format!("→ {}", d.join("/")),
                None => r.name(),
            }
        })
        .collect()
}

fn selected(m: &Model) -> String {
    let r = m.traffic.volume.selected_item().expect("a row");
    match r.dst() {
        Some(d) => format!("→ {}", d.join("/")),
        None => r.name(),
    }
}

#[test]
fn volume_lists_every_source_heaviest_first_and_v_comes_back_to_flows() {
    let mut m = on_volume(80, 24);
    let t = &m.traffic.volume;
    assert_eq!(t.len(), 9, "every source, folded; nothing in `other`");
    assert!(t.rows().iter().all(|r| r.is_source()));
    assert_eq!(volume_rows(&m)[0], "ipl:Corp/-");
    let conns: Vec<u64> = t.rows().iter().map(|r| r.value()).collect();
    assert!(conns.windows(2).all(|w| w[0] >= w[1]), "{conns:?}");
    assert_eq!(conns.iter().sum::<u64>(), 4_109);
    assert!(
        t.rows().iter().all(|r| r.scale == 1_301),
        "scaled to the largest"
    );
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Flows);
    // No flows: no rows, and the keys still cost nothing.
    let mut empty = model_fixture();
    empty.traffic.view = TrafficView::Volume;
    for msg in [
        key('l'),
        key('h'),
        enter(),
        key('E'),
        key('b'),
        key('j'),
        key('s'),
        kc(KeyCode::Esc),
    ] {
        quiet(&mut empty, msg);
    }
    assert!(empty.traffic.volume.is_empty());
}

#[test]
fn enter_unfolds_a_source_h_folds_it_and_enter_on_a_destination_drills() {
    let mut m = on_volume(80, 24);
    m.traffic
        .volume
        .select_where(|r| r.is_source() && r.name() == "shop/prod");
    quiet(&mut m, enter());
    assert_eq!(selected(&m), "shop/prod", "the cursor stays on the source");
    let rows = volume_rows(&m);
    let at = rows.iter().position(|r| r == "shop/prod").unwrap();
    assert_eq!(
        rows[at + 1..at + 4],
        ["→ shop/prod", "→ crm/prod", "→ api.partner.example/-"]
    );
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j'));
    assert_eq!(selected(&m), "→ crm/prod");
    // `h` on a destination folds its source, the cursor back on it.
    quiet(&mut m, key('h'));
    assert_eq!(selected(&m), "shop/prod");
    assert_eq!(volume_rows(&m).len(), 9);
    quiet(&mut m, kc(KeyCode::Right));
    assert_eq!(volume_rows(&m).len(), 12);
    quiet(&mut m, key('l'));
    assert_eq!(volume_rows(&m).len(), 12, "l unfolds only");
    quiet(&mut m, kc(KeyCode::Left));
    assert_eq!(volume_rows(&m).len(), 9);
    quiet(&mut m, enter());
    quiet(&mut m, enter());
    assert_eq!(volume_rows(&m).len(), 9, "Enter on a source toggles");
    quiet(&mut m, enter());
    quiet(&mut m, key('j'));
    quiet(&mut m, enter()); // the largest destination
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(
        m.traffic.drill,
        Some(Drill::Pair(
            vec!["shop".into(), "prod".into()],
            vec!["shop".into(), "prod".into()],
            Decisions::All
        ))
    );
    assert_eq!(m.traffic.flows.len(), 4);
    // Back on Volume the source is still unfolded.
    for _ in 0..4 {
        quiet(&mut m, key('v'));
    }
    assert_eq!(m.traffic.view, TrafficView::Volume);
    assert_eq!(volume_rows(&m).len(), 12);
}

#[test]
fn e_unfolds_every_source_or_folds_them_all() {
    let mut m = on_volume(80, 24);
    quiet(&mut m, key('j'));
    let src = selected(&m);
    quiet(&mut m, key('E'));
    let pairs = m.traffic.volume_state.tree_pairs().count();
    assert_eq!(volume_rows(&m).len(), 9 + pairs);
    assert_eq!(selected(&m), src, "the cursor stays on its source");
    assert!(m.status.text.is_empty() || !m.status.text.contains("explain"));
    quiet(&mut m, key('E'));
    assert_eq!(volume_rows(&m).len(), 9);
    // One unfolded: E unfolds the rest.
    quiet(&mut m, enter());
    quiet(&mut m, key('E'));
    assert_eq!(volume_rows(&m).len(), 9 + pairs);
    assert!(m.overlay == Overlay::None, "E is no explain here");
}

#[test]
fn volume_scrolls_with_the_table_keys_and_the_wheel_and_keeps_its_order() {
    use crossterm::event::MouseEventKind;
    let mut m = on_volume(80, 24);
    quiet(&mut m, key('E'));
    let n = m.traffic.volume.len();
    assert!(n > 13, "more rows than the panel holds: {n}");
    quiet(&mut m, kc(KeyCode::End));
    assert_eq!(m.traffic.volume.selected_pos(), n - 1);
    assert!(m.traffic.volume.offset() > 0, "scrolled");
    quiet(&mut m, kc(KeyCode::Home));
    assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 12)).is_empty());
    assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 12)).is_empty());
    assert_eq!(m.traffic.volume.selected_pos(), 2);
    let before = volume_rows(&m);
    quiet(&mut m, key('s'));
    assert_eq!(volume_rows(&m), before, "no sort: the tree keeps its order");
    assert!(
        m.status.text.contains("keeps its order"),
        "{}",
        m.status.text
    );
}

#[test]
fn search_on_volume_keeps_a_source_whose_destination_matches() {
    let mut m = on_volume(80, 24);
    quiet(&mut m, key('/'));
    for c in "partner".chars() {
        quiet(&mut m, key(c));
    }
    assert!(m.traffic.is_searching());
    quiet(&mut m, enter());
    let rows = volume_rows(&m);
    assert!(rows.contains(&"shop/prod".to_string()), "{rows:?}");
    assert!(rows.len() < 9, "{rows:?}");
    // Unfolded, its destinations come along.
    m.traffic.volume.select_where(|r| r.name() == "shop/prod");
    quiet(&mut m, enter());
    assert!(volume_rows(&m).contains(&"→ api.partner.example/-".to_string()));
    assert!(volume_rows(&m).contains(&"→ crm/prod".to_string()));
    quiet(&mut m, key('/'));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(volume_rows(&m).len(), 12);
}

#[test]
fn b_weighs_by_bytes_or_says_there_are_none() {
    use pce::traffic::volume::Weight;
    let mut m = on_volume(80, 24);
    quiet(&mut m, key('j'));
    let src = selected(&m);
    quiet(&mut m, key('b'));
    assert_eq!(m.traffic.volume_state.weight, Weight::Bytes);
    assert_eq!(m.traffic.volume.rows()[0].value(), 8_350_000);
    assert_eq!(selected(&m), src, "the cursor follows its row");
    assert_eq!(m.traffic.volume.columns[2].title, "bytes");
    quiet(&mut m, key('b'));
    assert_eq!(m.traffic.volume_state.weight, Weight::Connections);
    if let Some(d) = m.traffic.data.as_mut() {
        for f in &mut d.flows {
            f.bytes_in = None;
            f.bytes_out = None;
        }
    }
    assert!(m.traffic.volume_state.has_bytes);
    m.traffic.rebuild_views();
    assert!(
        !m.traffic.volume_state.has_bytes,
        "the meta greys `b bytes`"
    );
    quiet(&mut m, key('b'));
    assert_eq!(m.status.text, "no byte counts in this result");
    assert_eq!(m.traffic.volume_state.weight, Weight::Connections);
}

#[test]
fn the_picked_keys_reach_the_mesh_and_volume() {
    let mut m = on_volume(80, 24);
    for msg in [key('g'), key('j'), key('j'), key(' '), enter()] {
        quiet(&mut m, msg);
    }
    assert_eq!(m.traffic.group_keys, vec!["app", "env", "role"]);
    assert!(m.traffic.mesh.rows().iter().all(|r| r.src.len() == 3));
    assert_eq!(volume_rows(&m)[0], "ipl:Corp/-/-");
    assert!(
        volume_rows(&m).iter().any(|r| r == "shop/prod/web"),
        "sources by app, env and role"
    );
}

#[test]
fn a_new_result_drops_the_drill_and_keeps_the_unfolded_sources() {
    let mut m = on_volume(80, 24);
    quiet(&mut m, enter());
    quiet(&mut m, key('j'));
    quiet(&mut m, enter());
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert!(m.traffic.drill.is_some());
    let outcome = crate::tui::msg::QueryOutcome {
        flows: crate::tui::test_support::fixture_flows_rich(),
        truncated: false,
        incomplete: false,
        elapsed: std::time::Duration::from_secs(1),
        hit: false,
        filter: m.traffic.filter.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(outcome),
        },
    );
    assert_eq!(m.traffic.drill, None, "a new result drops the drill");
    assert_eq!(m.traffic.volume_state.unfolded.len(), 1);
}

#[test]
fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
    use crate::tui::model::TrafficView;
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    update(&mut m, key('v'));
    // first group is the 310-connection shop/prod -> shop/prod pair
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert!(m.traffic.drill.is_some());
    assert_eq!(m.traffic.flows.len(), 1);
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(m.traffic.drill.is_none());
    assert_eq!(m.traffic.flows.len(), 4);
}

#[test]
fn query_done_rebuilds_all_views() {
    use crate::tui::filter::TrafficFilter;
    use crate::tui::test_support::fixture_flows;
    let mut m = model_fixture();
    let f = TrafficFilter::defaults("1h", 50);
    update(&mut m, Msg::ApplyFilter(f.clone()));
    let outcome = crate::tui::msg::QueryOutcome {
        flows: fixture_flows(),
        truncated: false,
        incomplete: false,
        elapsed: std::time::Duration::from_secs(1),
        hit: false,
        filter: f,
        bounds: None,
        hrefs: Vec::new(),
    };
    update(
        &mut m,
        Msg::QueryDone {
            seq: 1,
            result: Ok(outcome),
        },
    );
    assert!(!m.traffic.groups.is_empty());
    assert_eq!(
        m.traffic.blocked.len(),
        3,
        "the shop/prod header and its two blocked sources"
    );
}

/// A cap past 10000 needs a `y` each run: `a` will not turn auto-refresh
/// on for it, and a tick turns it off instead of running unasked.
#[test]
fn auto_refresh_never_runs_a_cap_that_needs_a_confirm() {
    use crate::tui::model::QueryState;
    use crate::tui::test_support::model_with_flows;
    use std::time::{Duration, Instant};
    let mut m = model_with_flows();
    m.traffic.filter.max_results = 10_000;
    assert!(update(&mut m, key(']')).is_empty());
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunQuery { filter, .. }] if filter.max_results == 25_000),
        "{cmds:?}"
    );
    m.traffic.query = QueryState::Idle;
    let off = "auto off: cap 25000 needs a confirm each run";
    assert!(update(&mut m, key('a')).is_empty());
    assert!(!m.traffic.auto.enabled);
    assert_eq!(m.status.text, off);
    m.traffic.auto.enabled = true; // on from before the raise
    m.status = StatusLine::info("");
    let cmds = update(
        &mut m,
        Msg::Tick(Instant::now() + Duration::from_secs(3600)),
    );
    assert_eq!(cmds, Vec::<Cmd>::new());
    assert!(!m.traffic.auto.enabled);
    assert_eq!(m.status.text, off);
}

#[test]
fn auto_refresh_respects_interval_running_and_focus() {
    use crate::tui::test_support::model_with_flows;
    use std::time::{Duration, Instant};
    let mut m = model_with_flows();
    let t0 = Instant::now();
    assert!(update(&mut m, Msg::Tick(t0)).is_empty(), "off by default");
    update(&mut m, key('a'));
    assert!(m.traffic.auto.enabled);
    assert!(m.status.text.contains("60s"), "{}", m.status.text);
    // first tick after enabling: not due, last_run is the data's fetch time, 3 s ago.
    assert!(update(&mut m, Msg::Tick(t0)).is_empty());
    let later = t0 + Duration::from_secs(61);
    let cmds = update(&mut m, Msg::Tick(later));
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::RunQuery {
                mode: pce::CacheMode::Bypass,
                ..
            }]
        ),
        "{cmds:?}"
    );
    // while running, no second run
    assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(120))).is_empty());
    let seq = m.traffic.seq;
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Err("boom".into()),
        },
    );
    assert!(!m.traffic.auto.enabled, "a failed tick query disarms it");
    update(&mut m, key('a'));
    assert!(m.traffic.auto.enabled, "`a` re-arms");
    // unfocused: no run even when due
    update(&mut m, Msg::Focus(false));
    assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(200))).is_empty());
    update(&mut m, Msg::Focus(true));
    assert!(!update(&mut m, Msg::Tick(later + Duration::from_secs(200))).is_empty());
    let seq = m.traffic.seq;
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Err("boom".into()),
        },
    );
    assert!(!m.traffic.auto.enabled);
    assert_eq!(m.status.text, "auto-refresh off: boom");
    assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(1000))).is_empty());
}

#[test]
fn auto_refresh_interval_never_below_profile_minimum() {
    use crate::tui::model::AutoRefresh;
    let a = AutoRefresh::new(10);
    assert_eq!(a.interval, std::time::Duration::from_secs(60));
    let b = AutoRefresh::new(300);
    assert_eq!(b.interval, std::time::Duration::from_secs(300));
}

#[test]
fn auto_refresh_without_data_does_nothing() {
    use std::time::{Duration, Instant};
    let mut m = model_fixture();
    update(&mut m, key('a'));
    assert!(
        update(&mut m, Msg::Tick(Instant::now() + Duration::from_secs(600))).is_empty(),
        "nothing to refresh until a query ran"
    );
}

#[test]
fn result_panel_shows_countdown_only_while_auto_is_enabled() {
    use crate::tui::test_support::{model_with_flows, render_to_string};
    let mut m = model_with_flows();
    assert!(!render_to_string(&m, 120, 30).contains("auto in"));
    update(&mut m, key('a'));
    assert!(render_to_string(&m, 120, 30).contains("auto in 5"));
}

#[test]
fn auto_refresh_is_held_back_by_overlay_and_other_screens() {
    use crate::tui::test_support::model_with_flows;
    use std::time::{Duration, Instant};
    let mut m = model_with_flows();
    update(&mut m, key('a'));
    let due = Instant::now() + Duration::from_secs(600);
    m.overlay = Overlay::Help;
    assert!(update(&mut m, Msg::Tick(due)).is_empty());
    m.overlay = Overlay::None;
    m.screen = Screen::Policy;
    assert!(update(&mut m, Msg::Tick(due)).is_empty());
}

#[test]
fn search_captures_keys_on_aggregate_mesh_and_blocked_views() {
    use crate::tui::test_support::model_with_flows;
    for presses in [1, 2, 3, 4] {
        let mut m = model_with_flows();
        for _ in 0..presses {
            quiet(&mut m, key('v'));
        }
        quiet(&mut m, key('/'));
        for c in ['q', ':', '?'] {
            quiet(&mut m, key(c));
        }
        assert_eq!(m.screen, Screen::Traffic);
        assert!(!m.quit);
        assert_eq!(m.overlay, Overlay::None);
        let table_search = |m: &Model| match m.traffic.view {
            TrafficView::Aggregate => m.traffic.groups.search_text().to_string(),
            TrafficView::Mesh => m.traffic.mesh.search_text().to_string(),
            TrafficView::Blocked => m.traffic.blocked.search_text().to_string(),
            TrafficView::Volume => m.traffic.volume.search_text().to_string(),
            TrafficView::Flows | TrafficView::Exposure => {
                unreachable!("v moved off Flows")
            }
        };
        assert_eq!(table_search(&m), "q:?");
        quiet(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(!m.traffic.is_searching());
        assert_eq!(table_search(&m), "");
    }
}

#[test]
fn search_captures_global_keys() {
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    update(&mut m, key('/'));
    for c in ['1', '0', '.', 'q', ':', '?'] {
        update(&mut m, key(c));
    }
    assert_eq!(m.screen, Screen::Traffic);
    assert!(!m.quit);
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.traffic.flows.search_text(), "10.q:?");
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(!m.traffic.flows.is_searching());
    assert_eq!(m.traffic.flows.search_text(), "");
    update(&mut m, key('q'));
    assert!(m.quit);
}

#[test]
fn flow_table_navigation_sort_search_and_detail_never_emit_commands() {
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    assert_eq!(m.traffic.flows.len(), 4);
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        )
        .is_empty()
    );
    assert_eq!(m.traffic.flows.selected_pos(), 1);
    assert!(update(&mut m, key('j')).is_empty());
    assert_eq!(m.traffic.flows.selected_pos(), 2);
    assert!(update(&mut m, key('k')).is_empty());
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        )
        .is_empty()
    );
    assert_eq!(m.traffic.flows.selected_pos(), 3);
    assert!(update(&mut m, key('s')).is_empty());
    assert_eq!(m.traffic.flows.sort(), Some((0, false)));
    assert!(update(&mut m, key('/')).is_empty());
    assert!(m.traffic.flows.is_searching());
    for c in "nas".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert_eq!(m.traffic.flows.len(), 1);
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .is_empty()
    );
    assert!(!m.traffic.flows.is_searching());
    assert!(update(&mut m, key('/')).is_empty());
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        )
        .is_empty()
    );
    assert_eq!(m.traffic.flows.len(), 4, "Esc clears the search");
    assert!(!m.traffic.show_detail);
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .is_empty()
    );
    assert!(m.traffic.show_detail);
}

#[test]
fn query_done_fills_the_flow_table() {
    use crate::tui::filter::TrafficFilter;
    use crate::tui::test_support::fixture_flows;
    let mut m = model_fixture();
    let f = TrafficFilter::defaults("1h", 50);
    update(&mut m, Msg::ApplyFilter(f.clone()));
    let outcome = crate::tui::msg::QueryOutcome {
        flows: fixture_flows(),
        truncated: false,
        incomplete: false,
        elapsed: std::time::Duration::from_secs(1),
        hit: false,
        filter: f,
        bounds: None,
        hrefs: Vec::new(),
    };
    update(
        &mut m,
        Msg::QueryDone {
            seq: 1,
            result: Ok(outcome),
        },
    );
    assert_eq!(m.traffic.flows.len(), 4);
    assert_eq!(m.traffic.flows.selected_pos(), 0);
}

#[test]
fn q_and_ctrl_c_quit() {
    let mut m = model_fixture();
    assert_eq!(update(&mut m, key('q')), vec![Cmd::Quit]);
    assert!(m.quit);
    let mut m = model_fixture();
    let cmds = update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
    );
    assert_eq!(cmds, vec![Cmd::Quit]);
}

#[test]
fn number_keys_switch_screens_policy_objects_and_workloads_load() {
    let mut m = model_fixture();
    assert_eq!(m.screen, Screen::Traffic);
    // entering Policy for the first time loads its rulesets (see the policy tests)
    assert_eq!(update(&mut m, key('2')).len(), 1);
    assert_eq!(m.screen, Screen::Policy);
    // entering Objects for the first time loads all three kinds (cache hits)
    assert_eq!(update(&mut m, key('3')).len(), 3);
    assert_eq!(m.screen, Screen::Objects);
    // entering Workloads for the first time loads the list once (a cache hit)
    assert_eq!(
        update(&mut m, key('4')),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
    );
    assert_eq!(m.screen, Screen::Workloads);
    // there is no sixth screen: 6 does nothing
    assert!(update(&mut m, key('6')).is_empty());
    assert_eq!(m.screen, Screen::Workloads);
    assert!(update(&mut m, key('1')).is_empty());
    assert_eq!(m.screen, Screen::Traffic);
}

#[test]
fn startup_connect_flow() {
    let mut m = Model::new_connecting(
        model_fixture().profile,
        model_fixture().theme_name.clone(),
        60,
        1000,
        "24h".into(),
        crate::tui::model::WritesState::default(),
    );
    assert!(matches!(m.connection, Connection::Connecting));
    let cmds = update(&mut m, Msg::Connected);
    assert_eq!(cmds, vec![Cmd::LoadIndex(pce::CacheMode::Use)]);
    assert_eq!(
        m.status.text,
        "connected; loading labels, label groups, IP lists, services"
    );
    let cmds = update(
        &mut m,
        Msg::ConnectFailed("authentication failed (HTTP 401)".into()),
    );
    assert!(cmds.is_empty());
    match &m.connection {
        Connection::Disconnected { error } => assert!(error.contains("401")),
        other => panic!("{other:?}"),
    }
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn resize_and_focus_update_model() {
    let mut m = model_fixture();
    update(&mut m, Msg::Resize(120, 40));
    assert_eq!(m.size, (120, 40));
    update(&mut m, Msg::Focus(false));
    assert!(!m.focused);
}

/// Types `:text` and Enter; what the command asked for.
fn colon(m: &mut Model, text: &str) -> Vec<Cmd> {
    let mut cmds = update(m, key(':'));
    for c in text.chars() {
        cmds.extend(update(m, key(c)));
    }
    cmds.extend(update(
        m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    ));
    cmds
}

/// The one `Cmd::Export` `:text` asked for: `(path, force, headers, rows)`.
#[track_caller]
fn export_cmd(m: &mut Model, text: &str) -> (String, bool, Vec<&'static str>, Vec<Vec<String>>) {
    let mut cmds = colon(m, text);
    assert_eq!(cmds.len(), 1, "one Cmd, nothing for the PCE: {cmds:?}");
    match cmds.remove(0) {
        Cmd::Export {
            path,
            force,
            headers,
            rows,
        } => (path, force, headers, rows),
        other => panic!("not an export: {other:?}"),
    }
}

#[test]
fn export_asks_the_runtime_to_write_the_table_on_screen() {
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    let (path, force, headers, rows) = export_cmd(&mut m, "export ~/flows.csv");
    assert_eq!((path.as_str(), force), ("~/flows.csv", false));
    assert_eq!(
        headers,
        [
            "last seen",
            "src",
            "src group",
            "dst",
            "dst group",
            "service",
            "conns",
            "decision"
        ]
    );
    assert_eq!(rows.len(), 4);
    // Every column, the hidden-below-140 `last seen` whole.
    assert_eq!(rows[0][..2], ["2026-10-10T07:31:02Z", "host-1.example"]);

    // The search applies; `!` forces.
    m.traffic.flows.start_search();
    for c in "icmp".chars() {
        m.traffic.flows.search_push(c);
    }
    m.traffic.flows.search_commit();
    let (_, force, _, rows) = export_cmd(&mut m, "export! out.csv");
    assert!(force);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains(&"icmp".to_string()), "{rows:?}");

    // The runtime's answer lands in the status line.
    assert!(update(&mut m, Msg::Exported(Ok("wrote 1 row to out.csv".into()))).is_empty());
    assert_eq!(m.status.kind, StatusKind::Info);
    assert_eq!(m.status.text, "wrote 1 row to out.csv");
    assert!(update(&mut m, Msg::Exported(Err("out.csv exists".into()))).is_empty());
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn export_of_sections_is_every_peer_as_the_cli_prints_them_folded_or_not() {
    use crate::cli::traffic::SECTION_CSV_HEADERS;
    use crate::tui::model::TrafficView;
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    while m.traffic.view != TrafficView::Blocked {
        update(&mut m, key('v'));
    }
    let (_, _, headers, open) = export_cmd(&mut m, "export b.csv");
    assert_eq!(headers, SECTION_CSV_HEADERS);
    assert_eq!(
        open[0],
        [
            "shop/prod",
            "in",
            "ipl:Corp/-",
            "1",
            "10.1.4.9",
            "1",
            "host-1.example",
            "8443/tcp",
            "1",
            "12",
            "0",
            "2026-10-10T07:30:58Z",
            "potentially_blocked",
            "0",
            "12",
            "0",
            "0"
        ]
    );
    update(&mut m, key('z'));
    let shown = m.traffic.section_table().unwrap();
    assert!(
        (0..shown.len()).all(|i| shown.rows()[shown.visible()[i]].is_header()),
        "every section folded"
    );
    let (_, _, _, folded) = export_cmd(&mut m, "export b.csv");
    assert_eq!(folded, open, "folding hides nothing from the export");

    // Volume: the tree as shown, its sources and unfolded destinations.
    update(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Volume);
    let (_, _, headers, rows) = export_cmd(&mut m, "export v.csv");
    assert_eq!(
        headers,
        [
            "src",
            "dst",
            "conns",
            "allowed",
            "pot_blocked",
            "blocked",
            "unknown"
        ]
    );
    assert_eq!(rows.len(), m.traffic.volume.len());
    assert!(rows.iter().all(|r| r[1].is_empty()), "folded: sources only");
    let total: u64 = rows.iter().map(|r| r[2].parse::<u64>().unwrap()).sum();
    let split: u64 = rows
        .iter()
        .map(|r| (3..7).map(|i| r[i].parse::<u64>().unwrap()).sum::<u64>())
        .sum();
    assert_eq!(total, split, "the decisions add up to the weight");
    update(&mut m, enter());
    let (_, _, _, rows) = export_cmd(&mut m, "export v.csv");
    assert_eq!(rows.len(), m.traffic.volume.len());
    assert!(
        rows.iter().any(|r| !r[1].is_empty()),
        "the unfolded destinations"
    );
}

#[test]
fn export_of_workloads_matches_workloads_csv() {
    use crate::cli::output::TableRows;
    use crate::cli::workloads::WorkloadRow;
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    let (_, _, headers, rows) = export_cmd(&mut m, "export w.csv");
    let now = m.workloads.now.unwrap();
    let order = m.limits.key_order();
    let cli: Vec<WorkloadRow> = m
        .workloads
        .raw
        .iter()
        .cloned()
        .map(|w| WorkloadRow::new(w, now, &order))
        .collect();
    let (cli_headers, cli_rows) = (cli.full_headers(), cli.full_rows());
    // Both in the fetched order (the table is unsorted).
    assert_eq!(rows.len(), cli_rows.len());
    let mut shared = 0;
    for (i, h) in headers.iter().enumerate() {
        let Some(j) = cli_headers.iter().position(|c| c == h) else {
            continue;
        };
        shared += 1;
        for (row, cli_row) in rows.iter().zip(&cli_rows) {
            assert_eq!(row[i], cli_row[j], "{h} of {}", cli_row[j]);
        }
    }
    assert!(headers.contains(&"heartbeat_age_secs"), "{headers:?}");
    assert!(shared >= 10, "{headers:?}");
    let labels = headers.iter().position(|h| *h == "labels").unwrap();
    assert_eq!(rows[0][labels], "app=shop; env=prod; role=web");
}

#[test]
fn colon_opens_prompt_enter_runs_command_and_errors_show_in_status() {
    use crate::tui::model::Overlay;
    let mut m = model_fixture();
    update(&mut m, key(':'));
    assert!(matches!(m.overlay, Overlay::Prompt { .. }));
    for c in "policy".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(matches!(m.overlay, Overlay::None));
    assert_eq!(m.screen, Screen::Policy);

    update(&mut m, key(':'));
    for c in "bogus".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(
        m.status.text.contains("unknown command"),
        "{}",
        m.status.text
    );
    assert_eq!(m.screen, Screen::Policy, "a bad command changes nothing");

    update(&mut m, key(':'));
    update(&mut m, key('x'));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
    );
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(matches!(m.overlay, Overlay::None));
}

#[test]
fn help_overlay_toggles_and_swallows_keys() {
    use crate::tui::model::Overlay;
    let mut m = model_fixture();
    update(&mut m, key('?'));
    assert!(matches!(m.overlay, Overlay::Help));
    update(&mut m, key('2'));
    assert_eq!(
        m.screen,
        Screen::Traffic,
        "keys go to the overlay, not the screen"
    );
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(matches!(m.overlay, Overlay::None));
    update(&mut m, key('?'));
    update(&mut m, key('?'));
    assert!(matches!(m.overlay, Overlay::None), "? toggles");
    assert!(update(&mut m, Msg::Command(crate::tui::commands::Command::Quit)).contains(&Cmd::Quit));
}

#[test]
fn f_opens_editor_enter_applies_or_confirms() {
    use crate::tui::model::{Overlay, PendingAction};
    let mut m = model_fixture();
    update(&mut m, key('f'));
    assert!(matches!(m.overlay, Overlay::FilterEditor(_)));
    // Type a source filter into field 1.
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    for c in "app=shop".chars() {
        update(&mut m, key(c));
    }
    let msgs = update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(matches!(m.overlay, Overlay::None));
    assert_eq!(m.traffic.filter.src, "app=shop");
    assert!(
        matches!(msgs.as_slice(), [Cmd::RunQuery { .. }]),
        "an applied filter runs the query"
    );

    // A big window asks for confirmation before anything happens.
    update(&mut m, key('f'));
    // field 0 = window: clear and type 30d
    for _ in 0..3 {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
    }
    for c in "30d".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    match &m.overlay {
        Overlay::Confirm {
            reason,
            action: PendingAction::RunQuery(f),
        } => {
            assert!(reason.contains("30d"), "{reason}");
            assert_eq!(f.window, "30d");
        }
        other => panic!("{other:?}"),
    }
    update(&mut m, key('n'));
    assert!(matches!(m.overlay, Overlay::None));
    assert_eq!(m.traffic.filter.window, "24h", "declined: filter unchanged");
    update(&mut m, key('f'));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(matches!(m.overlay, Overlay::None));
}

#[test]
fn editor_shows_validation_error_and_stays_open() {
    use crate::tui::model::Overlay;
    let mut m = model_fixture();
    update(&mut m, key('f'));
    for _ in 0..3 {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
    }
    for c in "soon".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    match &m.overlay {
        Overlay::FilterEditor(e) => {
            assert!(e.error.as_deref().unwrap_or("").contains("window"))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn filter_and_window_commands_apply_without_the_editor() {
    use crate::tui::commands::Command;
    let mut m = model_fixture();
    update(&mut m, Msg::Command(Command::Filter("env=prod".into())));
    assert_eq!(m.traffic.filter.src, "env=prod");
    update(&mut m, Msg::Command(Command::Window("2h".into())));
    assert_eq!(m.traffic.filter.window, "2h");
    update(&mut m, Msg::Command(Command::Window("never".into())));
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(m.traffic.filter.window, "2h");
}

#[test]
fn unicast_toggles_the_transmission_and_runs_one_query() {
    use crate::tui::commands::Command;
    use pce::models::Transmission;
    let mut m = model_fixture();
    let cmds = update(&mut m, Msg::Command(Command::Unicast));
    assert_eq!(m.traffic.filter.transmission, vec![Transmission::Unicast]);
    let runs = cmds
        .iter()
        .filter(|c| matches!(c, Cmd::RunQuery { .. }))
        .count();
    assert_eq!(runs, 1, "one query, user-caused: {cmds:?}");
    assert!(m.traffic.filter.summary().contains("transmission unicast"));
    update(&mut m, Msg::Command(Command::Unicast));
    assert!(
        m.traffic.filter.transmission.is_empty(),
        "again: every kind"
    );
}

#[test]
fn apply_filter_runs_query_and_r_reruns_with_bypass() {
    use crate::tui::filter::TrafficFilter;
    let mut m = model_fixture();
    let f = TrafficFilter::defaults("1h", 50);
    let cmds = update(&mut m, Msg::ApplyFilter(f.clone()));
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        Cmd::RunQuery {
            seq, filter, mode, ..
        } => {
            assert_eq!(*seq, 1);
            assert_eq!(filter, &f);
            assert_eq!(*mode, pce::CacheMode::Use);
        }
        other => panic!("{other:?}"),
    }
    assert!(running(&m));
    // r while running: no-op
    let cmds = update(&mut m, key('r'));
    assert!(cmds.is_empty());
    assert!(
        m.status.text.contains("already running"),
        "{}",
        m.status.text
    );
    // finish it
    let outcome = crate::tui::msg::QueryOutcome {
        flows: vec![],
        truncated: false,
        incomplete: false,
        elapsed: std::time::Duration::from_millis(5),
        hit: false,
        filter: f.clone(),
        bounds: None,
        hrefs: Vec::new(),
    };
    update(
        &mut m,
        Msg::QueryDone {
            seq: 1,
            result: Ok(outcome),
        },
    );
    assert!(!running(&m));
    assert!(m.traffic.data.is_some());
    // r now reruns with Bypass and a new seq
    let cmds = update(&mut m, key('r'));
    match &cmds[0] {
        Cmd::RunQuery { seq, mode, .. } => {
            assert_eq!(*seq, 2);
            assert_eq!(*mode, pce::CacheMode::Bypass);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn new_filter_cancels_running_query_and_ignores_its_late_result() {
    use crate::tui::filter::TrafficFilter;
    use crate::tui::model::QueryState;
    let mut m = model_fixture();
    update(&mut m, Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)));
    let first_token = match &m.traffic.query {
        QueryState::Running { cancel, .. } => cancel.clone(),
        other => panic!("{other:?}"),
    };
    let mut f2 = TrafficFilter::defaults("1h", 50);
    f2.src = "app=shop".into();
    let cmds = update(&mut m, Msg::ApplyFilter(f2));
    assert!(first_token.is_cancelled(), "old query cancelled");
    assert!(matches!(cmds[0], Cmd::RunQuery { seq: 2, .. }));
    // The old query's late error must not disturb the new one.
    update(
        &mut m,
        Msg::QueryDone {
            seq: 1,
            result: Err("cancelled".into()),
        },
    );
    assert!(running(&m));
    assert!(!matches!(m.traffic.query, QueryState::Failed(_)));
    update(
        &mut m,
        Msg::QueryDone {
            seq: 2,
            result: Err("traffic query failed: boom".into()),
        },
    );
    assert!(matches!(&m.traffic.query, QueryState::Failed(e) if e.contains("boom")));
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn progress_updates_status_while_running() {
    use crate::tui::filter::TrafficFilter;
    use pce::traffic::QueryProgress;
    let mut m = model_fixture();
    update(&mut m, Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)));
    update(
        &mut m,
        Msg::QueryProgress {
            seq: 1,
            progress: QueryProgress::Polling {
                status: "working".into(),
                elapsed: std::time::Duration::from_secs(2),
            },
        },
    );
    assert!(m.status.text.contains("working"), "{}", m.status.text);
    update(
        &mut m,
        Msg::QueryProgress {
            seq: 7,
            progress: QueryProgress::Downloading,
        },
    );
    assert!(m.status.text.contains("working"), "stale seq ignored");
}

#[test]
fn truncated_result_warns() {
    use crate::tui::filter::TrafficFilter;
    let mut m = model_fixture();
    let f = TrafficFilter::defaults("1h", 50);
    update(&mut m, Msg::ApplyFilter(f.clone()));
    let outcome = crate::tui::msg::QueryOutcome {
        flows: vec![],
        truncated: true,
        incomplete: false,
        elapsed: std::time::Duration::from_secs(1),
        hit: false,
        filter: f,
        bounds: None,
        hrefs: Vec::new(),
    };
    update(
        &mut m,
        Msg::QueryDone {
            seq: 1,
            result: Ok(outcome),
        },
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(m.status.text, "0 flows, capped at 50; B raises the cap");
}

#[test]
fn confirm_requires_y_and_enter_is_ignored() {
    use crate::tui::model::Overlay;
    let enter = || Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let mut m = model_fixture();
    update(&mut m, key('f'));
    for _ in 0..3 {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
    }
    for c in "30d".chars() {
        update(&mut m, key(c));
    }
    assert!(update(&mut m, enter()).is_empty());
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    let cmds = update(&mut m, enter());
    assert!(cmds.is_empty(), "Enter must not confirm");
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    let cmds = update(&mut m, key('y'));
    assert!(matches!(m.overlay, Overlay::None));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Cmd::RunQuery {
            mode: pce::CacheMode::Use,
            ..
        }
    ));
    assert_eq!(m.traffic.filter.window, "30d");
}
