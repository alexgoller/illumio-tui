use super::*;

#[test]
fn complete_for_judges_the_result_actually_used() {
    use crate::tui::test_support::model_with_flows;
    use pce::traffic::{FocusDirection, parse_app};
    let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
        .with_direction(FocusDirection::In);
    let mut m = model_with_flows();
    assert_eq!(m.traffic.complete_for(&shop), Ok(()));
    m.traffic.data.as_mut().unwrap().incomplete = true;
    assert_eq!(
        m.traffic.complete_for(&shop),
        Err(Gap::Region { focused: false })
    );
    m.traffic.data.as_mut().unwrap().incomplete = false;
    m.traffic.data.as_mut().unwrap().truncated = true;
    assert_eq!(
        m.traffic.complete_for(&shop),
        Err(Gap::Capped {
            cap: 1000,
            focused: false
        })
    );
    // A focused result for the app, itself truncated: partial_for
    // calls it whole, complete_for does not.
    let data = TrafficData {
        flows: m.traffic.loaded_flows().to_vec(),
        truncated: true,
        incomplete: false,
        elapsed: Duration::ZERO,
        cache_hit: false,
        fetched_at: Instant::now(),
        filter: m.traffic.filter.clone(),
        range: None,
        hrefs: Vec::new(),
        draft: None,
    };
    m.traffic.apply_focused(shop.clone(), 1, data);
    assert!(!m.traffic.partial_for(&shop));
    assert_eq!(
        m.traffic.complete_for(&shop),
        Err(Gap::Capped {
            cap: 1000,
            focused: true
        })
    );
    // The out side is not covered by an `in` result: the unfocused one counts.
    let out = shop.clone().with_direction(FocusDirection::Out);
    assert_eq!(
        m.traffic.complete_for(&out),
        Err(Gap::Capped {
            cap: 1000,
            focused: false
        })
    );
    let empty = crate::tui::test_support::model_fixture();
    assert_eq!(empty.traffic.complete_for(&shop), Err(Gap::NoData));
    // A focused `in` result as the only result: the out side is not loaded.
    let mut only = crate::tui::test_support::model_fixture();
    let data = TrafficData {
        flows: m.traffic.loaded_flows().to_vec(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::ZERO,
        cache_hit: false,
        fetched_at: Instant::now(),
        filter: m.traffic.filter.clone(),
        range: None,
        hrefs: Vec::new(),
        draft: None,
    };
    only.traffic.apply_focused(shop.clone(), 1, data);
    assert!(only.traffic.unfocused.is_none());
    assert_eq!(only.traffic.complete_for(&out), Err(Gap::Uncovered));
    assert!(
        only.traffic.partial_for(&out),
        "nothing covers the out side"
    );
}

fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

#[test]
fn events_window_is_whole_minutes_and_open_ended_so_the_cache_can_hit() {
    let edit = EventsFilterEdit {
        severity: " err ".into(),
        event_type: "agent".into(),
        actor: String::new(),
        window: "24h".into(),
    };
    let a = edit.to_filter(at("2026-10-10T08:00:07Z")).unwrap();
    let b = edit.to_filter(at("2026-10-10T08:00:52Z")).unwrap();
    assert_eq!(a.to_query(), b.to_query(), "same minute, same query");
    assert_eq!(
        a.to_query(),
        vec![
            ("severity", "err".to_string()),
            ("timestamp[gte]", "2026-10-09T08:00:00Z".to_string()),
        ],
        "severity trimmed, `until` left open, the local type not sent"
    );
    let c = edit.to_filter(at("2026-10-10T08:01:00Z")).unwrap();
    assert_ne!(
        a.to_query(),
        c.to_query(),
        "the next minute moves the window"
    );
}

#[test]
fn events_empty_severity_is_none_and_a_bad_window_is_an_error() {
    let mut edit = EventsFilterEdit {
        severity: "  ".into(),
        event_type: String::new(),
        actor: String::new(),
        window: "1h".into(),
    };
    assert!(
        edit.to_filter(at("2026-10-10T08:00:00Z"))
            .unwrap()
            .severity
            .is_none()
    );
    edit.window = "soon".into();
    assert!(
        edit.to_filter(at("2026-10-10T08:00:00Z"))
            .unwrap_err()
            .starts_with("window:")
    );
}

#[test]
fn an_event_without_a_timestamp_renders_blank() {
    let e: Event = serde_json::from_str(r#"{"href":"/orgs/5/events/x"}"#).unwrap();
    for c in event_columns() {
        assert!((c.cell)(&e).len() < 5, "{}", c.title);
    }
    assert_eq!((event_columns()[0].cell)(&e), "");
}

#[test]
fn rules_are_found_by_href_inline_or_in_the_deny_fallback() {
    let mut p = PolicyState::new(60);
    p.raw = serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    let (r, rs) = p
        .find_rule("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2")
        .unwrap();
    assert_eq!(r.description.as_deref(), Some("corp to web"));
    assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/100");
    let (d, _) = p
        .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
        .unwrap();
    assert!(d.override_);
    assert!(
        p.find_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
            .is_none()
    );
    let fetched: DenyRule = serde_json::from_value(serde_json::json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 23, "proto": 6}]
    }))
    .unwrap();
    p.fallback_deny.insert(
        "/orgs/5/sec_policy/draft/rule_sets/101".into(),
        vec![fetched],
    );
    let (_, rs) = p
        .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9")
        .unwrap();
    assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/101");
}

#[test]
fn the_flow_and_blocked_columns_fit_80_columns_with_the_group_columns() {
    use crate::tui::ui::widgets::table::visible_columns;
    let row_width = |cols: &[(usize, u16, bool)]| -> u16 {
        cols.iter().map(|c| c.1).sum::<u16>() + cols.len() as u16 - 1 + 2
    };
    let flows = flow_columns();
    let at80 = visible_columns(&flows, 80);
    let titles: Vec<&str> = at80.iter().map(|c| flows[c.0].title).collect();
    assert_eq!(
        titles,
        vec![
            "src",
            "src group",
            "dst",
            "dst group",
            "service",
            "conns",
            "decision"
        ]
    );
    assert!(
        row_width(&at80) <= 78,
        "inside the border at 80: {}",
        row_width(&at80)
    );
    assert!(row_width(&visible_columns(&flows, 120)) <= 118);
    let at160 = visible_columns(&flows, 160);
    assert_eq!(flows[at160[0].0].title, "last seen");
    assert!(row_width(&at160) <= 158);
    let blocked = blocked_columns();
    let titles: Vec<&str> = blocked.iter().map(|c| c.title).collect();
    assert_eq!(
        titles,
        vec!["src", "dst name", "service", "flows", "conns", "last seen"]
    );
    // A fill column (width 0) gets at least 8 cells.
    let fill_width = |cols: &[(usize, u16, bool)]| -> u16 {
        cols.iter()
            .map(|c| if c.1 == 0 { 8 } else { c.1 })
            .sum::<u16>()
            + cols.len() as u16
            - 1
            + 2
    };
    let at80 = visible_columns(&blocked, 80);
    assert!(fill_width(&at80) <= 78 - 16, "room for the sources at 80");
    let apps = app_columns();
    let at80 = visible_columns(&apps, 80);
    let titles: Vec<&str> = at80.iter().map(|c| apps[c.0].title).collect();
    assert_eq!(
        titles,
        vec!["peer", "service", "flows", "conns", "bytes", "decision"]
    );
    assert!(fill_width(&at80) <= 78 - 16, "room for the peer at 80");
    let at160 = visible_columns(&apps, 160);
    assert_eq!(apps[at160[1].0].title, "hosts");
}

fn rich_sections(direction: Direction, decisions: Decisions, arrows: bool) -> Vec<SectionRow> {
    let flows = crate::tui::test_support::fixture_flows_rich();
    let keys = vec!["app".to_string(), "env".to_string()];
    let sections = if arrows {
        app_sections_both(&flows, &keys, decisions)
    } else {
        app_sections(&flows, &keys, direction, decisions)
    };
    section_rows(&sections, &BTreeSet::new(), arrows)
}

#[test]
fn blocked_cells_name_one_host_or_count_them() {
    let rows = rich_sections(Direction::In, Decisions::Blocked, false);
    let cols = blocked_columns();
    let header = rows
        .iter()
        .find(|r| r.is_header() && r.app == vec!["shop", "prod"])
        .unwrap();
    assert_eq!(
        header.header_line().unwrap(),
        "▾ shop/prod  in · 3 groups · 24 conns · 2 blocked · 1 pot.blocked"
    );
    assert!(
        cols.iter()
            .all(|c| (c.cell)(header).is_empty() || c.title == "src")
    );
    let peers: Vec<Vec<String>> = rows
        .iter()
        .filter(|r| !r.is_header() && r.app == vec!["shop", "prod"])
        .map(|r| cols.iter().map(|c| (c.cell)(r)).collect())
        .collect();
    assert_eq!(peers.len(), 3);
    assert!(
        peers
            .iter()
            .all(|p| p[1] == "shop-db-p.example" && p[2] == "5432/tcp")
    );
    let srcs: BTreeSet<&str> = peers.iter().map(|p| p[0].as_str()).collect();
    assert_eq!(
        srcs,
        BTreeSet::from(["  hr/prod", "  shop/dev", "  ipl:Corp/-"]),
        "indented sources"
    );
    let mut many = rows
        .iter()
        .find(|r| !r.is_header() && r.app == vec!["shop", "prod"])
        .unwrap()
        .clone();
    if let RowKind::Peer { row, .. } = &mut many.kind {
        row.app_hosts = 8;
        row.app_names = vec!["a".into(), "b".into()];
    }
    assert_eq!((cols[1].cell)(&many), "8 hosts");
}

#[test]
fn both_rows_mark_the_direction_and_share_one_header_per_app() {
    let rows = rich_sections(Direction::In, Decisions::All, true);
    let header = &rows[0];
    let line = header.header_line().unwrap();
    assert_eq!(
        line,
        "▾ shop/prod  ← 1545 conns · 2 blocked · 1 pot.blocked  → 1004 conns"
    );
    let peers: Vec<String> = rows
        .iter()
        .skip(1)
        .take_while(|r| !r.is_header())
        .map(|r| (app_columns()[0].cell)(r))
        .collect();
    assert!(peers.iter().any(|p| p.starts_with("  ← ")), "{peers:?}");
    assert!(peers.iter().any(|p| p.starts_with("  → ")), "{peers:?}");
    let first_out = peers.iter().position(|p| p.starts_with("  → ")).unwrap();
    assert!(
        peers[first_out..].iter().all(|p| p.starts_with("  → ")),
        "inbound rows, then outbound"
    );
    let headers = rows.iter().filter(|r| r.is_header()).count();
    let apps: BTreeSet<&Vec<String>> = rows.iter().map(|r| &r.app).collect();
    assert_eq!(headers, apps.len());
}

#[test]
fn a_folded_section_keeps_its_header_and_search_keeps_whole_sections() {
    let flows = crate::tui::test_support::fixture_flows_rich();
    let keys = vec!["app".to_string(), "env".to_string()];
    let sections = app_sections(&flows, &keys, Direction::In, Decisions::All);
    let folded = BTreeSet::from([vec!["crm".to_string(), "prod".to_string()]]);
    let rows = section_rows(&sections, &folded, false);
    assert!(rows[0].header_line().unwrap().starts_with("▹ crm/prod"));
    assert!(rows[1].is_header(), "no peers under a folded header");
    let mut t = TableView::new(app_columns()).with_matcher(section_matches);
    t.set_rows(section_rows(&sections, &BTreeSet::new(), false));
    let all = t.len();
    t.start_search();
    for c in "api.partner".chars() {
        t.search_push(c);
    }
    assert!(t.len() < all && !t.is_empty());
    let shown: Vec<&SectionRow> = t.visible().iter().map(|&i| &t.rows()[i]).collect();
    let app = &shown[0].app;
    assert!(shown[0].is_header());
    assert!(shown.iter().all(|r| &r.app == app), "one whole section");
    assert!(shown.len() > 2, "with its rows that do not match too");
}

#[test]
fn mesh_columns_follow_the_toggles_and_fit_80_columns() {
    use crate::tui::ui::widgets::table::visible_columns;
    let titles = |cols: MeshColumns| -> Vec<&'static str> {
        mesh_columns(cols).iter().map(|c| c.title).collect()
    };
    assert_eq!(
        titles(MESH_ALL),
        vec![
            "user", "process", "src", "dst", "service", "dst proc", "conns", "decision"
        ]
    );
    assert_eq!(
        titles(MeshColumns::default()),
        vec!["src", "dst", "service", "dst proc", "conns", "decision"]
    );
    let cols = mesh_columns(MESH_ALL);
    let at80 = visible_columns(&cols, 80);
    let width: u16 = at80.iter().map(|c| c.1).sum::<u16>() + at80.len() as u16 - 1 + 2;
    assert!(width <= 78, "{width}");
    assert!(
        !at80.iter().any(|c| cols[c.0].title == "dst proc"),
        "from 120 columns"
    );
    let at160 = visible_columns(&cols, 160);
    let width: u16 = at160.iter().map(|c| c.1).sum::<u16>() + at160.len() as u16 - 1 + 2;
    assert!(width <= 158, "{width}");
    let rows = mesh(
        &crate::tui::test_support::fixture_flows_rich(),
        &["app".to_string(), "env".to_string()],
        MESH_ALL,
    );
    let pb = rows
        .iter()
        .find(|r| r.dominant() == pce::models::PolicyDecision::PotentiallyBlocked)
        .unwrap();
    assert_eq!((cols[7].cell)(pb), "pot.blocked");
    assert_eq!((cols[7].narrow.unwrap())(pb), "pot.blk");
    let icmp = rows.iter().find(|r| r.proto == "icmp").unwrap();
    assert_eq!((cols[1].cell)(icmp), "-", "no process");
}

#[test]
fn workload_labels_column_follows_the_key_order_and_sorts_by_it() {
    let mut m = crate::tui::test_support::model_with_workloads();
    m.set_label_model(vec!["env".into()], "role".into(), Vec::new());
    // host-1 sorts first alphabetically (`app=aaa`), host-2 by the order (`role=db`).
    m.workloads.raw[0].labels[1].value = Some("aaa".into());
    m.workloads.rebuild(&m.limits.key_order());
    let t = &m.workloads.table;
    let col = t.columns.iter().position(|c| c.title == "labels").unwrap();
    assert_eq!(t.cell(&t.rows()[0], col), "env=prod role=web app=aaa");
    assert_eq!(t.rows()[0].labels, "env=prod role=web app=aaa");
    m.workloads.table.sort_by(col, false);
    let t = &m.workloads.table;
    let sorted: Vec<&str> = t
        .visible()
        .iter()
        .map(|&i| t.rows()[i].labels.as_str())
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        sorted,
        vec!["env=prod role=db app=shop", "env=prod role=web app=aaa"]
    );
}

fn shown_hosts(m: &Model) -> Vec<String> {
    let t = &m.workloads.table;
    t.visible()
        .iter()
        .map(|&i| t.rows()[i].workload.display_name())
        .collect()
}

#[test]
fn workload_filters_combine_and_name_themselves_with_the_count() {
    let mut m = crate::tui::test_support::model_with_workloads();
    let order = m.limits.key_order();
    let w = &mut m.workloads;
    assert_eq!(w.filter_line(), None, "nothing active: no line");
    // E: all → full → selective → visibility_only → idle → unmanaged → all.
    let mut seen = Vec::new();
    for _ in 0..6 {
        w.filters.next_enforcement();
        seen.push(w.filters.enforcement);
    }
    assert_eq!(
        seen,
        [
            Some("full"),
            Some("selective"),
            Some("visibility_only"),
            Some("idle"),
            Some("unmanaged"),
            None
        ]
    );
    // m: all → managed → unmanaged → all.
    let mut seen = Vec::new();
    for _ in 0..3 {
        w.filters.next_managed();
        seen.push(w.filters.managed);
    }
    assert_eq!(seen, [Some(true), Some(false), None]);

    w.filters.enforcement = Some("unmanaged");
    w.rebuild(&order);
    assert_eq!(shown_hosts(&m), ["unmanaged-nas"], "effective: no VEN");
    let w = &mut m.workloads;
    w.filters.enforcement = Some("idle");
    w.rebuild(&order);
    assert!(
        shown_hosts(&m).is_empty(),
        "the nas reports idle, but has no VEN"
    );

    let w = &mut m.workloads;
    w.filters = WorkloadFilters {
        unlabeled: true,
        ..Default::default()
    };
    w.rebuild(&order);
    assert_eq!(shown_hosts(&m), ["unmanaged-nas", "host-4.example"]);
    let w = &mut m.workloads;
    w.filters.managed = Some(true);
    w.rebuild(&order);
    assert_eq!(shown_hosts(&m), ["host-4.example"]);
    assert_eq!(
        m.workloads.filter_line().as_deref(),
        Some("managed · unlabeled (app, env, role) · 1 of 4")
    );

    let w = &mut m.workloads;
    w.filters = WorkloadFilters {
        offline: true,
        ..Default::default()
    };
    w.unhealthy_only = true;
    w.rebuild(&order);
    assert_eq!(shown_hosts(&m), ["host-2.example"]);
    assert_eq!(
        m.workloads.filter_line().as_deref(),
        Some("unhealthy · offline · 1 of 4")
    );
    // Search narrows the shown count too.
    m.workloads.table.start_search();
    m.workloads.table.search_push('z');
    m.workloads.table.search_commit();
    assert_eq!(
        m.workloads.filter_line().as_deref(),
        Some("unhealthy · offline · 0 of 4")
    );
    m.workloads.truncated = true;
    assert_eq!(
        m.workloads.filter_line().as_deref(),
        Some("unhealthy · offline · 0 of 4 · partial: the list is truncated")
    );
    m.workloads.unhealthy_only = false;
    m.workloads.filters = WorkloadFilters {
        enforcement: Some("visibility_only"),
        managed: Some(false),
        ..Default::default()
    };
    m.workloads.table.search_clear();
    m.workloads.rebuild(&order);
    assert_eq!(
        m.workloads.filter_line().as_deref(),
        Some("enforcement visibility_only · unmanaged · 0 of 4 · partial: the list is truncated")
    );
}

#[test]
fn workload_missing_column_names_the_absent_model_keys() {
    let mut m = crate::tui::test_support::model_with_workloads();
    m.set_label_model(vec!["env".into()], "role".into(), Vec::new());
    m.workloads.raw[0]
        .labels
        .retain(|l| l.key.as_deref() != Some("role"));
    m.workloads.rebuild(&m.limits.key_order());
    let t = &m.workloads.table;
    let col = t.columns.iter().position(|c| c.title == "missing").unwrap();
    let cells: Vec<String> = t.rows().iter().map(|r| t.cell(r, col)).collect();
    assert_eq!(cells, ["role", "", "env, role", "env, role"]);
    // `:export` joins them as every list cell: `; `.
    assert_eq!(t.full_cells(&t.rows()[2])[col], "env; role");
    // Hidden at ordinary widths unless `u` is on.
    assert!(t.columns[col].hide_below > 180);
    m.workloads.filters.unlabeled = true;
    m.workloads.rebuild(&m.limits.key_order());
    assert_eq!(m.workloads.table.columns[col].hide_below, 0);
}

/// 10k workloads, grouped, with a label filter and a search: a
/// rebuild and a regrouping per search key stay one pass each (well
/// under the generous bound even unoptimised; a guard against a
/// quadratic step, not a timing gate).
#[test]
fn grouped_workloads_at_10k_rebuild_and_search_quickly() {
    let mut m = crate::tui::test_support::model_with_workloads();
    let base = m.workloads.raw.clone();
    m.workloads.raw = (0..10_000)
        .map(|i| {
            let mut w = base[i % 2].clone();
            w.href = format!("/w/{i}");
            w.hostname = Some(format!("host-{i}.example"));
            w.labels[1].value = Some(format!("app{}", i % 300));
            w
        })
        .collect();
    let order = m.limits.key_order();
    let start = std::time::Instant::now();
    m.workloads.toggle_grouped(vec!["app".into(), "env".into()]);
    m.workloads.rebuild(&order);
    assert_eq!(m.workloads.groups.len(), 300);
    let labels = LabelFilter::parse("app=app1,app2,app3 !role=db").unwrap();
    m.workloads.set_labels(labels, None, &order);
    // Odd workloads are role=db: app1 and app3 are all excluded.
    assert_eq!(m.workloads.groups.len(), 1);
    m.workloads.set_labels(LabelFilter::default(), None, &order);
    m.workloads.table.start_search();
    for c in "host-99".chars() {
        m.workloads.table.search_push(c);
        m.workloads.regroup();
    }
    let took = start.elapsed();
    assert!(m.workloads.table.len() < 200);
    assert!(took < Duration::from_secs(20), "{took:?}");
}

#[test]
fn workload_visibility_is_a_dash_without_a_ven() {
    let mut m = crate::tui::test_support::model_with_workloads();
    for w in &mut m.workloads.raw {
        w.visibility_level = Some("flow_summary".into());
    }
    m.workloads.rebuild(&m.limits.key_order());
    let t = &m.workloads.table;
    let col = t
        .columns
        .iter()
        .position(|c| c.title == "visibility")
        .unwrap();
    let cells: Vec<String> = t.rows().iter().map(|r| t.cell(r, col)).collect();
    let nas = t
        .rows()
        .iter()
        .position(|r| r.workload.is_unmanaged())
        .unwrap();
    assert_eq!(cells[nas], "-");
    assert_eq!(cells[0], "flow_summary");
}

#[test]
fn workload_labels_column_hides_ignored_keys() {
    let mut m = crate::tui::test_support::model_with_workloads();
    m.set_label_model(vec!["app".into()], "role".into(), vec!["env".into()]);
    m.workloads.rebuild(&m.limits.key_order());
    let t = &m.workloads.table;
    let col = t.columns.iter().position(|c| c.title == "labels").unwrap();
    assert_eq!(t.cell(&t.rows()[0], col), "app=shop role=web");
    assert!(
        m.workloads.raw[0]
            .labels
            .iter()
            .any(|l| l.key.as_deref() == Some("env")),
        "the workload itself keeps the label"
    );
}

#[test]
fn limits_from_config_carry_the_ignored_keys() {
    let file: crate::config::ConfigFile = toml::from_str(
        r#"
[profiles.default]
host = "h"
org_id = 1
api_key = "k"
api_secret = "s"

[profiles.default.labels]
ignore_keys = ["os", "app"]
"#,
    )
    .unwrap();
    let cfg = crate::config::resolve(
        &file,
        &crate::config::Overrides::default(),
        &Default::default(),
    )
    .unwrap();
    let limits = Limits::from_config(&cfg);
    assert_eq!(limits.ignore_keys, vec!["os"], "app is a scope key");
    assert_eq!(limits.ruleset_name, "RF-{app}-{env}");
    assert_eq!(limits.key_order().ignored(), vec!["os"]);
    let mut m = crate::tui::test_support::model_fixture();
    m.limits = limits;
    m.reset_data();
    assert!(!m.traffic.key_order.shows("os"));
}

#[test]
fn the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env() {
    let mut m = crate::tui::test_support::model_fixture();
    assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
    assert_eq!(m.traffic.key_order.first(), vec!["app", "env", "role"]);
    m.set_label_model(
        vec!["site".into(), "zone".into()],
        "tier".into(),
        vec!["os".into()],
    );
    assert_eq!(m.traffic.group_keys, vec!["site", "zone"]);
    assert_eq!(m.limits.tier_key, "tier");
    assert_eq!(m.traffic.key_order.first(), vec!["site", "zone", "tier"]);
    assert_eq!(m.traffic.key_order.ignored(), vec!["os"]);
    m.reset_data();
    assert_eq!(
        m.traffic.key_order.first(),
        vec!["site", "zone", "tier"],
        "a profile switch's reset keeps the profile's order"
    );
    assert_eq!(m.traffic.key_order.ignored(), vec!["os"]);
    m.set_label_model(Vec::new(), "role".into(), Vec::new());
    assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
    assert_eq!(m.traffic.key_order.first(), vec!["role"]);
    assert!(
        m.limits.scope_keys.is_empty(),
        "the profile value itself is kept"
    );
}

#[test]
fn ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them() {
    let lists: Vec<IpList> =
        serde_json::from_str(include_str!("../../../../../fixtures/ip_lists.json")).unwrap();
    let mut s = IpListsState::new(60);
    let mut p = PolicyState::new(60);
    s.set_lists(lists, &p);
    assert_eq!(s.table.rows()[1].size.display(), "65.5K");
    assert_eq!(s.table.rows()[1].used_by, None, "no rulesets loaded");
    p.raw = serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    p.loaded_at = Some(Instant::now());
    s.recount(&p);
    let used: Vec<Option<usize>> = s.table.rows().iter().map(|r| r.used_by).collect();
    assert_eq!(used, vec![Some(1), Some(1)], "deny rule 7 and allow rule 2");
    let uses = p
        .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21")
        .unwrap();
    assert_eq!(uses[0].ruleset, "Shop-Prod");
    assert_eq!(uses[0].kind, RuleKind::Allow);
    p.scope = PolicyScope::Active;
    s.recount(&p);
    assert_eq!(s.table.rows()[0].used_by, None, "only draft rulesets count");
    let cols = ip_list_columns();
    let titles: Vec<&str> = cols.iter().map(|c| c.title).collect();
    assert_eq!(
        titles,
        vec!["name", "size", "ranges", "fqdns", "used by", "pending"]
    );
    let rows = s.table.rows();
    assert_eq!((cols[1].cell)(&rows[0]), "any");
    assert_eq!((cols[1].cell)(&rows[1]), "65.5K");
    assert_eq!((cols[2].cell)(&rows[0]), "2");
    assert_eq!((cols[3].cell)(&rows[1]), "1");
    assert_eq!((cols[4].cell)(&rows[0]), "-");
    assert!(
        (cols[1].key)(&rows[0]) > (cols[1].key)(&rows[1]),
        "any sorts last"
    );
    assert_eq!(Screen::ALL.len(), 5);
    assert_eq!(Screen::Events.index(), 4);
    assert_eq!(Screen::Objects.title(), "Objects");
    assert_eq!(ObjectKind::ALL.len(), 4);
    assert_eq!(ObjectKind::IpLists.title(), "IP lists");
    assert_eq!(ObjectKind::IpLists.index(), 3);
    assert_eq!(ObjectKind::Labels.prev(), ObjectKind::IpLists);
    assert_eq!(ObjectKind::Services.next(), ObjectKind::IpLists);
    assert_eq!(ObjectKind::IpLists.next(), ObjectKind::Labels);
    assert!(!ObjectKind::FETCHED.contains(&ObjectKind::IpLists));
}

#[test]
fn ip_list_uses_match_the_provider_side_and_fetched_deny_rules() {
    let mut sets: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    let deny_on_20 = sets[0]["deny_rules"][0].clone();
    let mut edge = sets[1].clone();
    edge["name"] = "Edge".into();
    edge["href"] = "/orgs/5/sec_policy/draft/rule_sets/102".into();
    edge["rules"][0]["providers"] =
        serde_json::json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]);
    sets.push(edge);
    let mut p = PolicyState::new(60);
    p.raw = serde_json::from_value(serde_json::Value::Array(sets)).unwrap();
    p.loaded_at = Some(Instant::now());
    p.fallback_deny.insert(
        "/orgs/5/sec_policy/draft/rule_sets/102".into(),
        vec![serde_json::from_value(deny_on_20).unwrap()],
    );
    let corp = p
        .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21")
        .unwrap();
    let corp: Vec<(&str, RuleKind)> = corp.iter().map(|u| (u.ruleset.as_str(), u.kind)).collect();
    assert_eq!(
        corp,
        vec![("Shop-Prod", RuleKind::Allow), ("Edge", RuleKind::Allow)],
        "Edge names the list as a provider"
    );
    let any = p
        .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/20")
        .unwrap();
    let any: Vec<(&str, RuleKind)> = any.iter().map(|u| (u.ruleset.as_str(), u.kind)).collect();
    assert_eq!(
        any,
        vec![("Shop-Prod", RuleKind::Deny), ("Edge", RuleKind::Deny)],
        "the second deny rule comes from fallback_deny"
    );
}

#[test]
fn the_rules_pane_lists_intra_then_extra_and_keeps_the_cursor_on_a_moved_rule() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    let scopes = |m: &Model| -> Vec<String> {
        m.policy
            .rules
            .rows()
            .iter()
            .map(|r| crate::tui::model::scope_cell(r).to_string())
            .collect()
    };
    assert_eq!(scopes(&m), ["", "intra", "extra", ""]);
    let corp = m.policy.raw[0].rules[1].href.clone();
    m.policy.rules.select_where(|r| r.href == corp);
    assert_eq!(m.policy.rules.selected_pos(), 2);
    // Its scope switched (and its neighbour's): it moves, the cursor with it.
    m.policy.raw[0].rules[1].unscoped_consumers = Some(false);
    m.policy.raw[0].rules[0].unscoped_consumers = Some(true);
    m.rebuild_rules();
    assert_eq!(scopes(&m), ["", "intra", "extra", ""]);
    assert_eq!(
        m.policy.rules.selected_item().map(|r| r.href.clone()),
        Some(corp)
    );
    assert_eq!(m.policy.rules.selected_pos(), 1);
}
