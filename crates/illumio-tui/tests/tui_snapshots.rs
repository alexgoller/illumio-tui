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
    let rule = m.policy.rules.rows()[0].clone(); // the override_deny rule sorts first
    m.overlay = Overlay::RuleDetail(rule);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("override_deny"), "{out}");
    insta::assert_snapshot!("policy_rule_detail_80x24", out);
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
    m.objects.kind = ObjectKind::IpLists;
    m.objects.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("0.0.0.0/0"), "{out}");
    insta::assert_snapshot!("objects_iplists_detail_80x24", out);
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
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
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
