use illumio_tui::tui::model::{Connection, Screen};
use illumio_tui::tui::test_support::{
    model_fixture, model_with_flows, model_with_objects, model_with_policy, model_with_workloads,
    render_styled, render_to_string,
};

#[test]
fn chrome_dark_80x24() {
    let m = model_fixture();
    insta::assert_snapshot!("chrome_dark_80x24", render_to_string(&m, 80, 24));
}

#[test]
fn chrome_dark_160x48() {
    let m = model_fixture();
    insta::assert_snapshot!("chrome_dark_160x48", render_to_string(&m, 160, 48));
}

#[test]
fn chrome_dark_styled_80x24() {
    let m = model_fixture();
    insta::assert_snapshot!("chrome_dark_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn chrome_light_styled_80x24() {
    let mut m = model_fixture();
    m.set_theme("light");
    insta::assert_snapshot!("chrome_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn too_small_terminal_shows_notice_and_does_not_panic() {
    let m = model_fixture();
    let out = render_to_string(&m, 40, 10);
    assert!(out.contains("too small"), "{out}");
    let _ = render_to_string(&m, 1, 1);
    let _ = render_to_string(&m, 0, 0);
}

#[test]
fn disconnected_state_shows_error_in_body() {
    let mut m = model_fixture();
    m.connection = Connection::Disconnected {
        error: "authentication failed (HTTP 401); check API key, secret and org id".into(),
    };
    m.screen = Screen::Traffic;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("HTTP 401"), "{out}");
    assert!(out.contains(":profile"), "hint to switch profile: {out}");
}

#[test]
fn table_widget_renders_header_sort_and_selection() {
    use illumio_tui::tui::table::{Column, SortKey, TableView};
    use illumio_tui::tui::ui::widgets::panel::panel;
    use illumio_tui::tui::ui::widgets::table::render;
    use ratatui::{Terminal, backend::TestBackend, style::Style};
    let m = model_fixture();
    let mut v: TableView<(String, u64)> = TableView::new(vec![
        Column {
            title: "name",
            width: 12,
            cell: |r: &(String, u64)| r.0.clone(),
            key: |r: &(String, u64)| SortKey::Text(r.0.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |r: &(String, u64)| r.1.to_string(),
            key: |r: &(String, u64)| SortKey::Num(r.1),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]);
    v.set_rows(
        (0..30)
            .map(|i| (format!("host-{i}"), (30 - i) as u64 * 7))
            .collect(),
    );
    v.sort_by(1, true);
    v.select_next();
    v.select_next();
    v.ensure_visible(6);
    let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
    term.draw(|f| {
        render(
            f,
            f.area(),
            &v,
            &m.theme,
            panel("Hosts", "30 rows", &m.theme, true),
            &|_| Style::default(),
        )
    })
    .unwrap();
    let buf = term.backend().buffer();
    let text: String = (0..10)
        .map(|y| {
            (0..40)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
                + "\n"
        })
        .collect();
    insta::assert_snapshot!("table_widget_40x10", text);
}

#[test]
fn help_overlay_80x24() {
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::ui::widgets::overlay::help_last_scroll;
    let mut m = model_fixture();
    m.overlay = Overlay::Help;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("1 2 3 4 5"), "first page: {out}");
    assert!(out.contains("move in the current table"), "{out}");
    assert!(out.contains("j/k scroll"), "{out}");
    insta::assert_snapshot!("help_overlay_80x24", out);
    m.help_scroll = help_last_scroll(24);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("close overlay"), "last line visible: {out}");
    assert!(out.contains("send / cancel a shown request"), "{out}");
    insta::assert_snapshot!("help_overlay_end_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "help_overlay_end_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn status_hint_lists_the_edit_keys_only_with_writes() {
    use illumio_tui::tui::model::WritesState;
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    let last = |m: &illumio_tui::tui::Model| {
        render_to_string(m, 80, 24)
            .lines()
            .last()
            .unwrap_or("")
            .to_string()
    };
    assert!(!last(&m).contains("n:new"), "{}", last(&m));
    m.writes = WritesState::on();
    assert!(last(&m).contains("n:new e:edit x:del"), "{}", last(&m));
    m.screen = Screen::Workloads;
    assert!(last(&m).contains("n:new e:edit x:del"), "{}", last(&m));
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut m);
    assert!(last(&m).contains("n:new e:edit x:del"), "{}", last(&m));
    m.screen = Screen::Traffic;
    assert!(!last(&m).contains("n:new"), "{}", last(&m));
}

#[test]
fn prompt_overlay_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::Prompt {
        input: "filter app=web".into(),
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains(":filter app=web"), "{out}");
    insta::assert_snapshot!("prompt_overlay_80x24", out);
}

#[test]
fn traffic_filter_bar_and_editor_80x24() {
    use illumio_tui::tui::filter::FilterEditor;
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.traffic.filter.src = "app=shop,env=prod".into();
    m.traffic.filter.service = "5432/tcp".into();
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("app=shop,env=prod"), "{out}");
    assert!(out.contains("cap 1000"), "{out}");
    insta::assert_snapshot!("traffic_filter_bar_80x24", out);
    m.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&m.traffic.filter));
    insta::assert_snapshot!("traffic_filter_editor_80x24", render_to_string(&m, 80, 24));
}

#[test]
fn traffic_filter_editor_completing_src_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::filter::FilterEditor;
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::update;
    let mut m = model_fixture();
    let mut e = FilterEditor::from_filter(&m.traffic.filter);
    e.active = 1;
    e.values[1] = "env=prod, ".into();
    m.overlay = Overlay::FilterEditor(e);
    let ctrl_n = || Msg::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(update(&mut m, ctrl_n()).is_empty());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("env=prod, app="), "{out}");
    assert!(out.contains("app=  loc=  role=  ip:  ipl:"), "{out}");
    insta::assert_snapshot!("traffic_filter_editor_completing_src_80x24", out);
}

#[test]
fn traffic_flows_80x24() {
    let m = model_with_flows();
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("host-1.example"), "{out}");
    assert!(
        out.contains("icmp"),
        "ICMP flow renders without a port: {out}"
    );
    assert!(out.contains("4 flows"), "{out}");
    insta::assert_snapshot!("traffic_flows_80x24", out);
}

#[test]
fn traffic_flows_160x48_with_detail() {
    let mut m = model_with_flows();
    m.traffic.show_detail = true;
    m.traffic.flows.select_next(); // the Corp -> host-1 potentially blocked flow
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("Corp"), "{out}");
    assert!(out.contains("potentially_blocked"), "{out}");
    insta::assert_snapshot!("traffic_flows_160x48_detail", out);
}

#[test]
fn traffic_merged_slices_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::msg::{Msg, QueryOutcome};
    use illumio_tui::tui::test_support::model_with_window;
    use illumio_tui::tui::update;
    let mut m = model_with_window("1d");
    let cmds = update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE)),
    );
    let Some(illumio_tui::tui::Cmd::RunSlice { start, end, .. }) = cmds.first().cloned() else {
        panic!("{cmds:?}");
    };
    let older = m.traffic.loaded_flows()[..3]
        .iter()
        .cloned()
        .map(|mut f| {
            f.last_detected = Some("2026-10-08T12:00:00Z".into());
            f
        })
        .collect();
    let seq = m.traffic.seq;
    let filter = m.traffic.filter.clone();
    update(
        &mut m,
        Msg::SliceDone {
            seq,
            result: Ok(QueryOutcome {
                flows: older,
                truncated: false,
                matches_count: None,
                elapsed: std::time::Duration::from_millis(900),
                hit: false,
                filter,
                bounds: Some((start, end)),
            }),
        },
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("window 2d · cap 1000 · 2 slices"), "{out}");
    insta::assert_snapshot!("traffic_merged_slices_80x24", out);
}

/// A truncated `1d` result whose flows span 4 minutes: the Result header
/// says how much it covers.
#[test]
fn traffic_coverage_header_80x24() {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::{Msg, QueryOutcome};
    use illumio_tui::tui::test_support::{FIXTURE_END, fixture_flows_rich};
    use illumio_tui::tui::update;
    let mut m = model_fixture();
    let end: jiff::Timestamp = FIXTURE_END.parse().unwrap();
    let mut flows = fixture_flows_rich();
    let n = flows.len();
    for (i, f) in flows.iter_mut().enumerate() {
        let t = end
            .checked_sub(jiff::Span::new().seconds((i * 240 / (n - 1)) as i64))
            .unwrap();
        f.last_detected = Some(pce::traffic::format_timestamp(t));
    }
    let filter = TrafficFilter::defaults("1d", n as u32);
    update(&mut m, Msg::ApplyFilter(filter.clone()));
    let seq = m.traffic.seq;
    let start = end.checked_sub(jiff::Span::new().hours(24)).unwrap();
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(QueryOutcome {
                flows,
                truncated: true,
                matches_count: None,
                elapsed: std::time::Duration::from_millis(900),
                hit: false,
                filter,
                bounds: Some((start, end)),
            }),
        },
    );
    assert!(
        m.status.text.starts_with(&format!(
            "cap {n} covers ~4 minutes of 1d: ] raises the cap"
        )),
        "{}",
        m.status.text
    );
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains(&format!("{n} flows · covers 4m of 1d · truncated")),
        "{out}"
    );
    assert!(
        out.lines().last().unwrap().contains("B backfills"),
        "the warning outranks the key hints at 80 columns: {out}"
    );
    insta::assert_snapshot!("traffic_coverage_header_80x24", out);
}

#[test]
fn traffic_flows_truncated_shows_warning() {
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("⚠ truncated"), "{out}");
}

#[test]
fn traffic_aggregate_and_blocked_80x24() {
    use illumio_tui::tui::model::TrafficView;
    let mut m = model_with_flows();
    m.traffic.view = TrafficView::Aggregate;
    m.traffic.rebuild_views();
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("shop/prod"), "{out}");
    insta::assert_snapshot!("traffic_aggregate_80x24", out);
    m.traffic.view = TrafficView::Blocked;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("8443/tcp"), "{out}");
    insta::assert_snapshot!("traffic_blocked_80x24", out);
}

#[test]
fn app_to_app_pairs_by_service_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::{Msg, update};
    let mut m = model_with_flows();
    m.traffic.view = TrafficView::Aggregate;
    let o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE);
    assert!(update(&mut m, Msg::Key(o)).is_empty());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("pairs · by service"), "{out}");
    assert!(out.contains("5432/tcp") && out.contains("icmp"), "{out}");
    insta::assert_snapshot!("traffic_pairs_by_service_80x24", out);
}

#[test]
fn app_to_app_sections_in_and_both_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::{By, TrafficView};
    use illumio_tui::tui::test_support::model_with_rich_flows;
    use illumio_tui::tui::{Msg, update};
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Aggregate;
    let x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(update(&mut m, Msg::Key(x)).is_empty());
    assert_eq!(m.traffic.apps_state.by, By::In);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("in · keys app,env"), "{out}");
    assert!(
        out.contains("▾ crm/prod  in · 6 groups · 1601 conns"),
        "{out}"
    );
    insta::assert_snapshot!("traffic_app_sections_in_80x24", out);
    m.traffic.apps_state.by = By::Both;
    m.traffic.rebuild_views();
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("▾ shop/prod  ← 1545 conns · 2 blocked · 1 pot.blocked  → 1004 conns"),
        "{out}"
    );
    assert!(out.contains("    → crm/prod"), "{out}");
    insta::assert_snapshot!("traffic_app_sections_both_80x24", out);
}

#[test]
fn app_to_app_hints_x_o_and_d_where_the_status_line_has_room() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    let out = render_to_string(&m, 160, 48);
    assert!(!out.contains("x mode"), "{out}");
    m.traffic.view = TrafficView::Aggregate;
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("]/[ cap  x mode  o service  d decision"),
        "{out}"
    );
    let out = render_to_string(&m, 80, 24);
    assert!(!out.contains("x mode"), "no room at 80: {out}");
}

#[test]
fn policy_screen_80x24_and_160x48() {
    use illumio_tui::tui::model::Screen;
    use illumio_tui::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Shop-Prod"), "{out}");
    assert!(
        out.contains("override_deny") || out.contains("override"),
        "{out}"
    );
    insta::assert_snapshot!("policy_80x24", out);
    insta::assert_snapshot!("policy_160x48", render_to_string(&m, 160, 48));
}

#[test]
fn policy_pending_overlay_80x24() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.policy.pending = Some(pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
    ));
    m.overlay = Overlay::Pending;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Global-Infra"), "{out}");
    insta::assert_snapshot!("policy_pending_80x24", out);
}

#[test]
fn policy_rule_detail_overlay_80x24() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let rule = m.policy.rules.rows()[0].view.clone(); // the override_deny rule sorts first
    m.overlay = Overlay::RuleDetail(rule);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("override_deny"), "{out}");
    assert!(
        out.contains("press i to load workloads (1 request)"),
        "{out}"
    );
    insta::assert_snapshot!("policy_rule_detail_80x24", out);
}

#[test]
fn objects_labels_key_filter_and_hidden_count_80x24() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::picker::KeyPicker;
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    m.set_label_model(
        vec!["app".into(), "env".into()],
        "role".into(),
        vec!["loc".into()],
    );
    m.objects.label_keys = vec!["role".into(), "env".into()];
    m.objects.rebuild_labels(&m.limits.key_order());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("key role,env · 3 of 6 · 1 hidden"), "{out}");
    assert!(!out.contains("shop"), "{out}");
    insta::assert_snapshot!("objects_labels_key_filter_80x24", out);
    let seen = vec![
        ("app".to_string(), 2),
        ("env".to_string(), 1),
        ("role".to_string(), 2),
        ("loc".to_string(), 1),
    ];
    m.overlay = Overlay::LabelKeys(KeyPicker::new(
        &m.limits.scope_keys,
        &m.limits.tier_key,
        &seen,
        &m.objects.label_keys,
        false,
    ));
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("Label keys") && out.contains("(1 label)"),
        "{out}"
    );
    insta::assert_snapshot!("objects_label_keys_picker_80x24", out);
}

#[test]
fn objects_screen_tabs_80x24() {
    use illumio_tui::tui::model::{ObjectKind, Screen};
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("role") && out.contains("web"), "{out}");
    assert!(out.contains("cached 3s ago"), "{out}");
    insta::assert_snapshot!("objects_labels_80x24", out);
    m.objects.show_detail = true;
    m.objects.kind = ObjectKind::Services;
    insta::assert_snapshot!("objects_services_160x48", render_to_string(&m, 160, 48));
}

#[test]
fn objects_group_detail_resolves_members_80x24() {
    use illumio_tui::tui::model::{ObjectKind, Screen};
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    m.objects.kind = ObjectKind::LabelGroups;
    m.objects.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("env=prod"), "{out}");
    insta::assert_snapshot!("objects_groups_detail_80x24", out);
}

#[test]
fn workloads_screen_80x24_and_detail_160x48() {
    use illumio_tui::tui::model::Screen;
    use illumio_tui::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("host-1.example"), "{out}");
    assert!(out.contains("healthy"), "{out}");
    insta::assert_snapshot!("workloads_80x24", out);
    m.workloads.show_detail = true;
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("eth0"), "{out}");
    insta::assert_snapshot!("workloads_160x48_detail", out);
}

#[test]
fn error_overlay_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::Error(format!(
        "rulesets: {}",
        "request to the PCE failed: connection reset by peer while reading the response body; \
         the load balancer closed the connection after 30 seconds, retry with a narrower filter or try again later"
    ));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Error"), "{out}");
    assert!(out.contains("Esc"), "{out}");
    insta::assert_snapshot!("error_overlay_80x24", out);
}

#[test]
fn light_theme_styled_screens_160x48() {
    let mut m = model_with_flows();
    m.set_theme("light");
    insta::assert_snapshot!("light_traffic_160x48", render_styled(&m, 160, 48));
    let mut m = model_with_policy();
    m.set_theme("light");
    m.screen = Screen::Policy;
    insta::assert_snapshot!("light_policy_160x48", render_styled(&m, 160, 48));
    let mut m = model_with_objects();
    m.set_theme("light");
    m.screen = Screen::Objects;
    insta::assert_snapshot!("light_objects_160x48", render_styled(&m, 160, 48));
    let mut m = model_with_workloads();
    m.set_theme("light");
    m.screen = Screen::Workloads;
    insta::assert_snapshot!("light_workloads_160x48", render_styled(&m, 160, 48));
}

#[test]
fn dark_and_light_differ_in_style_not_text() {
    let dark = model_with_flows();
    let mut light = model_with_flows();
    light.set_theme("light");
    assert_eq!(
        render_to_string(&dark, 80, 24),
        render_to_string(&light, 80, 24)
    );
    assert_ne!(render_styled(&dark, 80, 24), render_styled(&light, 80, 24));
}

#[test]
fn tripped_auto_refresh_keeps_its_reason_in_the_panel_meta() {
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    assert!(!render_to_string(&m, 160, 48).contains("auto off"));
    m.workloads.auto.tripped = true;
    assert!(render_to_string(&m, 160, 48).contains("auto off · failed"));
    m.workloads.auto.enabled = true;
    let on = render_to_string(&m, 160, 48);
    assert!(on.contains("auto in") && !on.contains("auto off"), "{on}");
}

#[test]
fn workloads_health_summary_and_columns_160x48() {
    use illumio_tui::tui::model::Screen;
    use illumio_tui::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m.workloads.show_detail = true;
    m.workloads.table.select_next();
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("healthy 1"), "{out}");
    assert!(out.contains("offline 1"), "{out}");
    assert!(out.contains("unmanaged 1"), "{out}");
    assert!(
        out.contains("policy out of date"),
        "warning text in detail: {out}"
    );
    assert!(out.contains("syncing"), "{out}");
    insta::assert_snapshot!("workloads_health_160x48", out);
}

#[test]
fn events_screen_80x24_and_detail_160x48() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_events;
    let mut m = model_with_events();
    m.screen = Screen::Events;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("agent.tampering"), "{out}");
    assert!(out.contains("user:user-1"), "{out}");
    insta::assert_snapshot!("events_80x24", out);
    m.overlay = Overlay::EventDetail {
        event: m.events.table.rows()[2].clone(),
        scroll: 0,
    };
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("\"event_type\": \"agent.tampering\""), "{out}");
    insta::assert_snapshot!("events_detail_160x48", out);
    // At 80x24 the box holds 22 JSON lines and its note says so.
    let small = render_to_string(&m, 80, 24);
    assert!(small.contains("lines 1-22 of"), "{small}");
    assert!(small.contains("j/k scroll"), "{small}");
    // Scrolled past the end: the view clamps so the box stays full and the
    // last line is visible.
    if let Overlay::EventDetail { scroll, .. } = &mut m.overlay {
        *scroll = 1000;
    }
    let end = render_to_string(&m, 80, 24);
    assert!(end.contains("of"), "{end}");
    assert!(!end.contains("lines 1-22"), "{end}");
    assert!(
        end.contains("\n│}"),
        "last line (closing brace) visible: {end}"
    );
    insta::assert_snapshot!("events_detail_end_80x24", end);
}

#[test]
fn events_wide_editor_and_light_styled() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_events;
    let mut m = model_with_events();
    m.screen = Screen::Events;
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("cached 5s ago · 4 rows · window 24h"), "{out}");
    assert!(out.contains("resource"), "{out}");
    m.overlay = Overlay::EventsEditor {
        values: ["err".into(), "agent".into(), "24h".into()],
        active: 1,
        error: None,
    };
    insta::assert_snapshot!("events_editor_80x24", render_to_string(&m, 80, 24));
    m.overlay = Overlay::None;
    m.set_theme("light");
    insta::assert_snapshot!("light_events_160x48", render_styled(&m, 160, 48));
}

#[test]
fn header_shows_the_writes_marker_only_when_enabled() {
    use illumio_tui::tui::model::WritesState;
    let mut m = model_fixture();
    let first = |m: &illumio_tui::tui::Model| {
        render_to_string(m, 80, 24)
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    assert!(!first(&m).contains("writes"), "{}", first(&m));
    m.writes = WritesState::on();
    assert!(
        first(&m).starts_with(" illumio-tui ✎ writes · pce.example · org 5 ●"),
        "{}",
        first(&m)
    );
    insta::assert_snapshot!("header_writes_dark_styled_80x24", render_styled(&m, 80, 24));
    m.set_theme("light");
    insta::assert_snapshot!(
        "header_writes_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
    // A host too long for the row cannot push the safety marker off it.
    m.profile.host =
        "policy-compute-engine-01.customer-production.datacenter.example.internal".into();
    assert!(first(&m).contains("✎ writes"), "{}", first(&m));
}

#[test]
fn ip_list_form_80x24_scrolls_the_active_field() {
    use illumio_tui::tui::edit::EditIntent;
    use illumio_tui::tui::form::{Field, Form};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut m);
    let mut form = Form::new(
        "New IP list",
        vec![
            Field::new("Name", "Corp", "Corp", true),
            Field::new("Description", "Office ranges", "optional", false),
            Field::new(
                "Ranges",
                "10.1.0.0-10.1.255.255, 10.2.0.0/16, !10.1.5.0/24, 192.168.0.0/16, 172.16.0.0/12",
                "10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24",
                false,
            ),
            Field::new(
                "FQDNs",
                "*.corp.example",
                "*.corp.example, api.example.com",
                false,
            ),
        ],
        EditIntent::CreateIpList,
    );
    form.active = 2;
    m.overlay = Overlay::Form(form);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("New IP list"), "{out}");
    assert!(out.contains("Name*"), "required fields are marked: {out}");
    assert!(
        out.contains("…") && out.contains("172.16.0.0/12▏"),
        "the end of the active field stays visible: {out}"
    );
    insta::assert_snapshot!("form_ip_list_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("form_ip_list_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn confirm_write_overlay_shows_method_path_and_body() {
    use illumio_tui::tui::model::WritesState;
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::{
        ip_list_delete_plan, label_post_plan, model_with_objects,
    };
    // What an admin sees: writes on, so the header carries the marker.
    let mut m = model_with_objects();
    m.writes = WritesState::on();
    m.screen = Screen::Objects;
    m.overlay = Overlay::ConfirmWrite {
        plan: label_post_plan(),
        kind: pce::write::ObjectKind::Label,
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("✎ writes"), "{out}");
    assert!(out.contains("create label app=web2"), "{out}");
    assert!(out.contains("POST /orgs/5/labels"), "{out}");
    assert!(out.contains("\"value\": \"web2\""), "{out}");
    assert!(out.contains("y send · n cancel"), "{out}");
    insta::assert_snapshot!("confirm_label_post_80x24", out);
    let mut l = model_with_objects();
    l.writes = WritesState::on();
    l.screen = Screen::Objects;
    l.set_theme("light");
    l.overlay = m.overlay.clone();
    insta::assert_snapshot!(
        "confirm_label_post_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
    let mut m = illumio_tui::tui::test_support::model_with_ip_lists();
    m.writes = WritesState::on();
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut m);
    m.overlay = Overlay::ConfirmWrite {
        plan: ip_list_delete_plan(),
        kind: pce::write::ObjectKind::IpList,
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("delete ip_list Corp"), "{out}");
    assert!(
        out.contains("DELETE /orgs/5/sec_policy/draft/ip_lists/21"),
        "{out}"
    );
    assert!(out.contains("(no body)"), "{out}");
    insta::assert_snapshot!("confirm_ip_list_delete_80x24", out);
    let mut l = illumio_tui::tui::test_support::model_with_ip_lists();
    l.writes = WritesState::on();
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut l);
    l.set_theme("light");
    l.overlay = m.overlay.clone();
    insta::assert_snapshot!(
        "confirm_ip_list_delete_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
}

#[test]
fn confirm_write_overlay_wraps_a_long_value_instead_of_clipping_it() {
    use illumio_tui::tui::model::{Overlay, Screen, WritesState};
    use illumio_tui::tui::test_support::model_with_objects;
    let description = "Office ranges for the Frankfurt, Munich and Hamburg sites, \
        agreed with the network team in the October change window; review after the \
        Q1 migration END";
    let plan = pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/sec_policy/draft/ip_lists".into(),
            body: Some(serde_json::json!({
                "name": "Corp",
                "description": description,
                "ip_ranges": [{"from_ip": "10.1.0.0/16"}]
            })),
            summary: "create ip_list Corp".into(),
            objects: Vec::new(),
        },
        kind: pce::write::ObjectKind::IpList,
    };
    let mut m = model_with_objects();
    m.writes = WritesState::on();
    m.screen = Screen::Objects;
    m.overlay = Overlay::ConfirmWrite {
        plan,
        kind: pce::write::ObjectKind::IpList,
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    // Every character of the value is drawn, across the wrapped lines.
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    let drawn: String = out
        .lines()
        .filter_map(|l| l.split('│').nth(2))
        .map(squeeze)
        .collect();
    assert!(
        drawn.contains(&squeeze(&format!("\"description\": \"{description}\","))),
        "{out}"
    );
    insta::assert_snapshot!("confirm_long_description_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "confirm_long_description_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn pending_diff_overlay_80x24() {
    use illumio_tui::tui::diff::{DiffKind, ObjectDiff, diff_ip_list};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let lists: Vec<pce::models::IpList> =
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
    let mut draft = lists[1].clone();
    draft.description = Some("Office and lab".into());
    draft
        .ip_ranges
        .push(serde_json::from_value(serde_json::json!({"from_ip": "10.2.0.0/16"})).unwrap());
    m.overlay = Overlay::Diff {
        diff: ObjectDiff {
            name: "Corp".into(),
            href: draft.href.clone(),
            change: DiffKind::Updated,
            lines: diff_ip_list(Some(&lists[1]), Some(&draft)),
        },
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("IP list Corp · updated"), "{out}");
    assert!(out.contains("+ range 10.2.0.0/16"), "{out}");
    assert!(out.contains("- description: Office ranges"), "{out}");
    insta::assert_snapshot!("pending_diff_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("pending_diff_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn rule_set_form_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::fixture_index;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let shop = m.policy.raw[0].clone();
    let idx = fixture_index();
    m.overlay = Overlay::Form(
        form_for(
            EditIntent::UpdateRuleSet(shop),
            Some(idx.as_ref()),
            m.flow_prefill(),
        )
        .expect("a form"),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Edit ruleset Shop-Prod"), "{out}");
    assert!(out.contains("app=shop, env=prod"), "{out}");
    assert!(out.contains("Enabled*"), "{out}");
    insta::assert_snapshot!("rule_set_form_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "rule_set_form_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn rule_and_deny_rule_forms_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::fixture_index;
    let idx = fixture_index();
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let shop = m.policy.raw[0].clone();
    m.overlay = Overlay::Form(
        form_for(
            EditIntent::UpdateRule(shop.rules[1].clone(), shop.href.clone()),
            Some(idx.as_ref()),
            m.flow_prefill(),
        )
        .expect("a form"),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Edit allow rule 2 in ruleset 100"), "{out}");
    assert!(out.contains("iplist:Corp"), "{out}");
    assert!(out.contains("443/tcp, 8000-8099/tcp"), "{out}");
    assert!(out.contains("Unscoped*"), "{out}");
    insta::assert_snapshot!("form_rule_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("form_rule_light_styled_80x24", render_styled(&m, 80, 24));
    let mut d = model_with_policy();
    d.screen = Screen::Policy;
    d.overlay = Overlay::Form(
        form_for(
            EditIntent::UpdateDenyRule(shop.deny_rules[1].clone(), shop.href.clone()),
            Some(idx.as_ref()),
            d.flow_prefill(),
        )
        .expect("a form"),
    );
    let out = render_to_string(&d, 80, 24);
    assert!(out.contains("Edit deny rule 8 in ruleset 100"), "{out}");
    assert!(out.contains("Override*"), "{out}");
    insta::assert_snapshot!("form_deny_rule_80x24", out);
    d.set_theme("light");
    insta::assert_snapshot!(
        "form_deny_rule_light_styled_80x24",
        render_styled(&d, 80, 24)
    );
}

#[test]
fn pending_marks_and_provision_confirm_80x24() {
    use illumio_tui::tui::model::{Overlay, Screen, WritesState};
    use illumio_tui::tui::test_support::provision_plan;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.policy.pending = Some(pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
    ));
    m.policy.pending_selected = 1;
    m.policy
        .pending_marked
        .insert("/orgs/5/sec_policy/draft/ip_lists/21".into());
    m.overlay = Overlay::Pending;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("[x] ip_lists"), "{out}");
    assert!(out.contains("[ ] rule_sets"), "{out}");
    assert!(out.contains("2/2 · 1 marked"), "{out}");
    assert!(
        out.contains("j/k select · Space mark · Enter diff · P provision · R revert · Esc"),
        "{out}"
    );
    insta::assert_snapshot!("policy_pending_marks_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "policy_pending_marks_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
    // What an admin sees after `P`: writes on, so the header carries the marker.
    let mut c = model_with_policy();
    c.writes = WritesState::on();
    c.screen = Screen::Policy;
    c.overlay = Overlay::ConfirmWrite {
        plan: provision_plan(),
        kind: pce::write::ObjectKind::Provision,
        scroll: 0,
    };
    let out = render_to_string(&c, 80, 24);
    assert!(out.contains("provision 1 ip_list, 1 rule_set"), "{out}");
    assert!(out.contains("POST /orgs/5/sec_policy"), "{out}");
    assert!(out.contains("\"change_subset\""), "{out}");
    insta::assert_snapshot!("confirm_provision_80x24", out);
    c.set_theme("light");
    insta::assert_snapshot!(
        "confirm_provision_light_styled_80x24",
        render_styled(&c, 80, 24)
    );
}

#[test]
fn confirm_revert_80x24() {
    use illumio_tui::tui::model::{Overlay, Screen, WritesState};
    use illumio_tui::tui::test_support::revert_plan;
    let mut c = model_with_policy();
    c.writes = WritesState::on();
    c.screen = Screen::Policy;
    c.overlay = Overlay::ConfirmWrite {
        plan: revert_plan(),
        kind: pce::write::ObjectKind::Revert,
        scroll: 0,
    };
    c.policy.revert_typed = "rev".into();
    let out = render_to_string(&c, 80, 24);
    assert!(out.contains("revert 2 changes:"), "{out}");
    assert!(out.contains("  ip_list \"Corp\" (removed)"), "{out}");
    assert!(
        out.contains("  rule_set \"Global-Infra\" (back to active)"),
        "{out}"
    );
    assert!(out.contains("type revert to confirm: rev"), "{out}");
    assert!(out.contains("PUT /orgs/5/sec_policy/delete"), "{out}");
    insta::assert_snapshot!("confirm_revert_80x24", out);
    c.set_theme("light");
    insta::assert_snapshot!(
        "confirm_revert_light_styled_80x24",
        render_styled(&c, 80, 24)
    );
}

#[test]
fn pending_rule_set_diff_80x24() {
    use illumio_tui::tui::diff::{DiffKind, ObjectDiff, diff_rule_set};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::{fixture_index, rule_set_diff_fixture};
    let (active, draft) = rule_set_diff_fixture();
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.overlay = Overlay::Diff {
        diff: ObjectDiff {
            name: "Shop-Prod".into(),
            href: draft.href.clone(),
            change: DiffKind::Updated,
            lines: diff_rule_set(Some(&active), Some(&draft), &fixture_index()),
        },
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("ruleset Shop-Prod · updated"), "{out}");
    assert!(
        out.contains("+ allow role=web ← iplist:Corp 443/tcp"),
        "{out}"
    );
    assert!(out.contains("- description: Ringfence"), "{out}");
    insta::assert_snapshot!("pending_rule_set_diff_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "pending_rule_set_diff_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn rule_from_flow_form_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{fixture_flows, fixture_index};
    let mut m = model_with_flows();
    let idx = fixture_index();
    m.overlay = Overlay::Form(
        form_for(
            EditIntent::RuleFromFlow(Box::new(fixture_flows()[1].clone())),
            Some(idx.as_ref()),
            m.flow_prefill(),
        )
        .expect("a form"),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Allow rule from flow"), "{out}");
    assert!(out.contains("Ruleset*"), "{out}");
    assert!(out.contains("role=web"), "{out}");
    assert!(!out.contains("app=shop, env=prod, role=web"), "{out}");
    assert!(out.contains("scope app=shop, env=prod"), "{out}");
    assert!(out.contains("iplist:Corp"), "{out}");
    assert!(out.contains("8443/tcp"), "{out}");
    assert!(out.contains("Ctrl-A all labels"), "{out}");
    insta::assert_snapshot!("form_rule_from_flow_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "form_rule_from_flow_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn rule_from_flow_form_offering_ctrl_r_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for, new_ruleset_offer};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{fixture_flows, fixture_index};
    let mut m = model_with_flows();
    let idx = fixture_index();
    let mut form = form_for(
        EditIntent::RuleFromFlow(Box::new(fixture_flows()[1].clone())),
        Some(idx.as_ref()),
        m.flow_prefill(),
    )
    .expect("a form");
    // The planner's answer to `New-App`, as the reopened form shows it.
    form.fields[0].value = "New-App".into();
    let e = r#"no ruleset named "New-App"; similar: Shop-Prod"#;
    form.offers_ruleset = new_ruleset_offer(&form, e, &m.limits.scope_keys);
    form.error = Some(e.into());
    m.overlay = Overlay::Form(form);
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("Ctrl-R: create it, scoped app=shop, env=prod"),
        "{out}"
    );
    assert!(out.contains("Ctrl-R new ruleset"), "{out}");
    assert!(out.contains("similar: Shop-Prod"), "{out}");
    insta::assert_snapshot!("form_rule_from_flow_ctrl_r_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "form_rule_from_flow_ctrl_r_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn label_form_completing_the_key_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::fixture_index;
    use illumio_tui::tui::update;
    let mut m = model_fixture();
    let idx = fixture_index();
    let form = form_for(
        EditIntent::CreateLabel,
        Some(idx.as_ref()),
        m.flow_prefill(),
    )
    .expect("a form");
    m.overlay = Overlay::Form(form);
    let ctrl_n = || Msg::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(update(&mut m, ctrl_n()).is_empty());
    assert!(update(&mut m, ctrl_n()).is_empty());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("app  env  loc  role"), "{out}");
    insta::assert_snapshot!("form_label_completing_key_80x24", out);
}

#[test]
fn objects_ip_lists_tab_80x24_and_160x48_with_detail() {
    use illumio_tui::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut m);
    m.objects.ip_lists.table.select_next(); // Corp
    m.objects.ip_lists.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    let strip = out.lines().nth(2).unwrap();
    assert!(
        strip.contains("Services") && strip.contains("IP lists"),
        "the fourth Objects tab: {strip}"
    );
    assert!(out.contains("3 Objects") && !out.contains(" 6 "), "{out}");
    assert!(out.contains("65.5K"), "{out}");
    assert!(out.contains("used by Shop-Prod · allow"), "{out}");
    insta::assert_snapshot!("objects_ip_lists_80x24", out);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("65536 IPv4"), "{out}");
    insta::assert_snapshot!("objects_ip_lists_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "objects_ip_lists_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn ip_list_detail_with_many_ranges_keeps_used_by_visible_80x24() {
    use illumio_tui::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    let ranges: Vec<_> = (0..10)
        .map(|i| serde_json::json!({"from_ip": format!("10.0.{i}.0/24")}))
        .collect();
    let many: pce::models::IpList = serde_json::from_value(serde_json::json!({
        "href": "/orgs/5/sec_policy/draft/ip_lists/20", "name": "Any (0.0.0.0/0 and ::/0)",
        "ip_ranges": ranges
    }))
    .unwrap();
    m.objects.ip_lists.set_lists(vec![many], &m.policy);
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut m);
    m.objects.ip_lists.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("used by Shop-Prod · deny"), "{out}");
    assert!(out.contains("+7 more ranges"), "{out}");
    insta::assert_snapshot!("objects_ip_list_detail_many_ranges_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "objects_ip_list_detail_many_ranges_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn the_tab_row_uses_short_titles_only_when_the_full_row_does_not_fit() {
    let m = model_fixture();
    let narrow = render_to_string(&m, 64, 24);
    let row = narrow.lines().nth(1).unwrap();
    assert!(
        row.contains("1 Traf") && row.contains("3 Obj") && row.contains("5 Ev"),
        "{row}"
    );
    for w in [80, 160] {
        let out = render_to_string(&m, w, 24);
        let row = out.lines().nth(1).unwrap();
        assert!(
            row.contains("1 Traffic") && row.contains("5 Events") && !row.contains('6'),
            "{row}"
        );
    }
}

#[test]
fn rule_detail_and_rules_pane_show_the_impact_80x24() {
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{model_with_policy, with_workloads};
    let mut m = with_workloads(model_with_policy());
    m.screen = Screen::Policy;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("P/C") && out.contains("1/2"), "{out}");
    insta::assert_snapshot!("policy_impact_80x24", out);
    let mut l = with_workloads(model_with_policy());
    l.screen = Screen::Policy;
    l.set_theme("light");
    insta::assert_snapshot!(
        "policy_impact_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
    let rule = m.policy.rules.rows()[2].view.clone(); // corp to web
    m.overlay = Overlay::RuleDetail(rule.clone());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("1 provider · 0 consumers"), "{out}");
    assert!(out.contains("Corp: 65.5K addresses"), "{out}");
    insta::assert_snapshot!("rule_detail_impact_80x24", out);
    l.overlay = Overlay::RuleDetail(rule);
    insta::assert_snapshot!(
        "rule_detail_impact_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
}

#[test]
fn confirm_rule_overlay_shows_the_impact_80x24() {
    use illumio_tui::tui::model::{Overlay, WritesState};
    use illumio_tui::tui::test_support::{model_with_policy, rule_post_plan, with_workloads};
    let mut m = with_workloads(model_with_policy());
    m.writes = WritesState::on();
    m.screen = Screen::Policy;
    m.overlay = Overlay::ConfirmWrite {
        plan: rule_post_plan(),
        kind: pce::write::ObjectKind::Rule,
        scroll: 0,
    };
    m.refresh_confirm_impact();
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("impact: 1 provider · 0 consumers"), "{out}");
    assert!(
        out.contains("POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules"),
        "{out}"
    );
    insta::assert_snapshot!("confirm_rule_impact_80x24", out);
    let mut cold = model_with_policy();
    cold.writes = WritesState::on();
    cold.screen = Screen::Policy;
    cold.overlay = m.overlay.clone();
    cold.refresh_confirm_impact();
    assert!(
        render_to_string(&cold, 80, 24).contains("impact: press i to load workloads (1 request)")
    );
    m.set_theme("light");
    insta::assert_snapshot!(
        "confirm_rule_impact_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn traffic_group_columns_picker_and_blocked_by_group_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::{Overlay, TrafficView};
    use illumio_tui::tui::test_support::model_with_rich_flows;
    use illumio_tui::tui::{Msg, update};
    let mut m = model_with_rich_flows();
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("src group") && out.contains("dst group"),
        "{out}"
    );
    assert!(out.contains("shop/prod"), "{out}");
    assert!(out.contains("g group"), "the status hint names g: {out}");
    insta::assert_snapshot!("traffic_flows_groups_80x24", out);
    let g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
    assert!(update(&mut m, Msg::Key(g)).is_empty(), "g never requests");
    assert!(
        matches!(m.overlay, Overlay::GroupKeys(_)),
        "{:?}",
        m.overlay
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Group by"), "{out}");
    assert!(out.contains("[x] app") && out.contains("[ ] role"), "{out}");
    assert!(out.contains("(seen in 11 flows)"), "{out}");
    assert!(out.contains("keys: app,env"), "{out}");
    insta::assert_snapshot!("traffic_key_picker_80x24", out);
    m.overlay = Overlay::None;
    m.traffic.view = TrafficView::Blocked;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("dst name") && out.contains("hr/prod"), "{out}");
    assert!(out.contains("in · blocked · keys app,env"), "{out}");
    insta::assert_snapshot!("traffic_blocked_groups_80x24", out);
}

#[test]
fn flow_detail_shows_both_label_sets_and_the_bytes() {
    let mut m = model_with_flows();
    m.traffic.show_detail = true; // row 0: shop/prod -> shop/prod with bytes
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("src labels app=shop env=prod role=web"),
        "{out}"
    );
    assert!(
        out.contains("dst labels app=shop env=prod role=db"),
        "{out}"
    );
    assert!(out.contains("bytes in 120.0K out 4.0K"), "{out}");
    m.traffic.flows.select_next(); // Corp: an IP list, no labels, no bytes
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("src labels -"), "{out}");
    assert!(out.contains("ip lists: Corp"), "{out}");
    assert!(!out.contains("bytes in"), "{out}");
}

#[test]
fn traffic_mesh_80x24_160x48_and_light() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Mesh;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Mesh"), "{out}");
    assert!(out.contains("u user · p process · Enter drills"), "{out}");
    assert!(
        out.contains("▸ -        -          ipl:Corp/-"),
        "an inbound flow's user is the destination's, not the IP list's: {out}"
    );
    assert!(!out.contains("www-data"), "{out}");
    assert!(!out.contains("dst proc"), "from 120 columns: {out}");
    insta::assert_snapshot!("traffic_mesh_80x24", out);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("dst proc") && out.contains("nginx"), "{out}");
    assert!(out.contains("pot.blocked"), "{out}");
    insta::assert_snapshot!("traffic_mesh_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!("light_traffic_mesh_160x48", render_styled(&m, 160, 48));
}

#[test]
fn traffic_treemap_80x24_160x48_zoomed_and_light() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    use illumio_tui::tui::update;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Treemap;
    assert!(update(&mut m, Msg::Resize(80, 24)).is_empty());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Treemap"), "{out}");
    assert!(
        out.contains("by conns · b bytes · Enter zoom · Esc out"),
        "{out}"
    );
    assert!(out.contains("ipl:Corp/-") && out.contains("▒"), "{out}");
    insta::assert_snapshot!("traffic_treemap_80x24", out);
    assert!(update(&mut m, Msg::Resize(160, 48)).is_empty());
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("702") && out.contains("821"),
        "destination weights: {out}"
    );
    insta::assert_snapshot!("traffic_treemap_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!("light_traffic_treemap_160x48", render_styled(&m, 160, 48));
    m.set_theme("dark");
    assert!(update(&mut m, Msg::Resize(80, 24)).is_empty());
    m.traffic.treemap.zoom = Some(vec!["shop".into(), "prod".into()]);
    m.traffic.relayout_treemap();
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Treemap ▸ shop/prod"), "{out}");
    assert!(
        out.contains("▒crm/prod▒") && out.contains("▒33▒"),
        "the destinations: {out}"
    );
    insta::assert_snapshot!("traffic_treemap_zoomed_80x24", out);
}

#[test]
fn an_empty_result_draws_the_treemap_like_an_empty_table() {
    use illumio_tui::tui::model::TrafficView;
    let mut m = model_fixture();
    m.traffic.view = TrafficView::Treemap;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Treemap") && out.contains("no rows"), "{out}");
}

#[test]
fn mesh_and_treemap_keep_the_truncated_marker() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    use illumio_tui::tui::update;
    let mut m = model_with_rich_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    assert!(update(&mut m, Msg::Resize(160, 48)).is_empty());
    for view in [TrafficView::Mesh, TrafficView::Treemap] {
        m.traffic.view = view;
        let out = render_to_string(&m, 160, 48);
        assert!(out.contains("⚠ truncated"), "{view:?}: {out}");
    }
}

#[test]
fn traffic_focus_badge_80x24() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = illumio_tui::tui::test_support::model_with_rich_flows();
    illumio_tui::tui::update(
        &mut m,
        Msg::Command(Command::Focus(Some("crm/prod".into()))),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("focus crm/prod · both"), "{out}");
    assert!(out.contains("9 of 30 flows"), "{out}");
    insta::assert_snapshot!("traffic_focus_badge_80x24", out);
}

#[test]
fn traffic_app_picker_80x24() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = model_with_flows();
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Focus(None)));
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("Focus on an app") && out.contains("shop/prod"),
        "{out}"
    );
    insta::assert_snapshot!("traffic_app_picker_80x24", out);
}

#[test]
fn traffic_focus_partial_120x30() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    illumio_tui::tui::update(
        &mut m,
        Msg::Command(Command::Focus(Some("shop/prod".into()))),
    );
    let out = render_to_string(&m, 120, 30);
    assert!(
        out.contains("focus is partial: r re-queries shop/prod · both"),
        "{out}"
    );
    insta::assert_snapshot!("traffic_focus_partial_120x30", out);
}
