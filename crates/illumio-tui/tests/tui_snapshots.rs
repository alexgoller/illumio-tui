use illumio_tui::tui::model::{Connection, Screen};
use illumio_tui::tui::test_support::{
    model_fixture, model_with_flows, model_with_objects, model_with_policy, model_with_window,
    model_with_workloads, render_styled, render_to_string, with_pairing_profiles, with_policy,
    with_workloads,
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
    assert!(
        out.contains("Keys · Flows"),
        "the context in the title: {out}"
    );
    assert!(out.contains("FLOWS"), "its group first: {out}");
    assert!(out.contains("flat · by src app"), "{out}");
    assert!(out.contains("SECTIONS"), "{out}");
    assert!(out.contains("Tab next · S-Tab prev · a all"), "{out}");
    insta::assert_snapshot!("help_overlay_80x24", out);
    m.help_scroll = help_last_scroll(&m, (80, 24));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("EVERYWHERE"), "the last group: {out}");
    assert!(out.contains("close overlay"), "last line visible: {out}");
    assert!(out.contains("send / cancel a shown request"), "{out}");
    assert!(out.contains("more: Commands"), "the other groups: {out}");
    assert!(
        !out.contains("this help"),
        "no ? line on a screen's page: {out}"
    );
    assert!(out.contains("command prompt (Tab → Commands)"), "{out}");
    insta::assert_snapshot!("help_overlay_end_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "help_overlay_end_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn help_overlay_flows_100x40() {
    use illumio_tui::tui::help::HelpPage;
    use illumio_tui::tui::model::Overlay;
    let mut m = model_with_flows();
    m.overlay = Overlay::Help;
    insta::assert_snapshot!("help_overlay_flows_100x40", render_to_string(&m, 100, 40));
    // Tab: the next group alone; a: every group.
    m.help_page = HelpPage::Group(illumio_tui::tui::help::HelpTopic::AppToApp);
    let out = render_to_string(&m, 100, 40);
    assert!(out.contains("Keys · App-to-app"), "{out}");
    assert!(out.contains("next: Blocked · prev: Flows"), "{out}");
    assert!(!out.contains("FLOWS"), "{out}");
    m.help_page = HelpPage::All;
    let out = render_to_string(&m, 100, 40);
    assert!(out.contains("Keys · all"), "{out}");
    assert!(out.contains("a back"), "{out}");
}

/// No terminal size panics the help, in every context and on its pages,
/// scrolled to the top and to the end: the help alone at every width and
/// height (below the chrome's minimum too), the whole screen at the widths
/// where the layout changes.
#[test]
fn help_renders_at_every_terminal_size() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::help::{HelpContext, HelpPage, HelpTopic};
    use illumio_tui::tui::model::{Overlay, TrafficView};
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support as ts;
    use illumio_tui::tui::ui::widgets::overlay::{help_last_scroll, render_help};
    let mut models = Vec::new();
    for view in [
        TrafficView::Flows,
        TrafficView::Aggregate,
        TrafficView::Mesh,
        TrafficView::Blocked,
        TrafficView::Treemap,
    ] {
        let mut m = ts::model_with_rich_flows();
        m.traffic.view = view;
        models.push(m);
    }
    for (mut m, screen) in [
        (ts::model_with_policy(), Screen::Policy),
        (ts::model_with_objects(), Screen::Objects),
        (ts::model_with_workloads(), Screen::Workloads),
        (ts::model_with_events(), Screen::Events),
    ] {
        m.screen = screen;
        models.push(m);
    }
    let mut grouped = ts::model_with_many_workloads();
    grouped.screen = Screen::Workloads;
    gen_key(&mut grouped, 'x');
    models.push(grouped);
    let mut detail = ts::model_with_events();
    detail.screen = Screen::Events;
    illumio_tui::tui::update(
        &mut detail,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    models.push(detail);
    models.push(app_overlay_model());
    models.push(generator_model());
    let mut pending = ts::model_with_policy();
    pending.screen = Screen::Policy;
    pending.overlay = Overlay::Pending;
    models.push(pending);
    let dir = tempfile::tempdir().unwrap();
    let mut marks = ts::model_with_marks(dir.path(), ts::RICH_MARKS);
    illumio_tui::tui::update(&mut marks, Msg::Command(Command::Marks(None)));
    models.push(marks);
    let mut contexts = Vec::new();
    for m in &mut models {
        gen_key(m, '?');
        assert_eq!(m.overlay, Overlay::Help);
        contexts.push(HelpContext::of(m));
    }
    for c in [
        HelpContext::EventDetail,
        HelpContext::AppOverlay,
        HelpContext::Generator,
        HelpContext::Pending,
        HelpContext::Marks,
    ] {
        assert!(contexts.contains(&c), "{c:?} in {contexts:?}");
    }
    let pages = [
        HelpPage::Context,
        HelpPage::Group(HelpTopic::Commands),
        HelpPage::All,
    ];
    // The widths where the layout changes: the chrome's minimum, the tab
    // row, 80 columns, the help box's widest.
    let boundaries = [20, 59, 60, 61, 68, 69, 70, 80, 96, 97, 130];
    for w in 20..=130u16 {
        let edge = boundaries.contains(&w);
        // Every width: the context page; at the edges every page, short
        // and tall, and the whole screen.
        let heights: &[u16] = if edge { &[1, 3, 16, 24, 40] } else { &[24] };
        for &h in heights {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            for m in &mut models {
                for page in pages {
                    if !edge && page != HelpPage::Context {
                        continue;
                    }
                    m.help_page = page;
                    let last = help_last_scroll(m, (w, h));
                    for scroll in if last == 0 { vec![0] } else { vec![0, last] } {
                        m.help_scroll = scroll;
                        term.draw(|f| render_help(m, f, f.area())).unwrap();
                        if edge && page == HelpPage::Context && matches!(h, 16 | 24) {
                            let _ = render_to_string(m, w, h);
                        }
                    }
                }
            }
        }
    }
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
    // `x` groups the workloads: delete is `X` there; the bulk keys lead.
    assert!(
        last(&m).contains("Space mark  L labels  M enforce  n/e/X"),
        "{}",
        last(&m)
    );
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
fn traffic_filter_editor_with_exclusions_80x24() {
    use illumio_tui::tui::filter::FilterEditor;
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.traffic.filter.src = "app=shop | env=prod, !app=monitoring".into();
    m.traffic.filter.service = "!22/tcp".into();
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("svc !22/tcp"),
        "the bar shows exclusions: {out}"
    );
    m.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&m.traffic.filter));
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("a ! term leaves its match out of the whole field, not one | group"),
        "{out}"
    );
    insta::assert_snapshot!("traffic_filter_editor_with_exclusions_80x24", out);
}

#[test]
fn the_filter_bar_shows_a_narrowed_transmission_only() {
    let mut m = model_fixture();
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("window 24h · cap 1000 · unbounded ╮"),
        "every kind: not shown: {out}"
    );
    m.traffic.filter.transmission = vec![pce::models::Transmission::Unicast];
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("window 24h · cap 1000 · unicast · unbounded ╮"),
        "in the filter bar's title, whole at 80 columns: {out}"
    );
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

/// A truncated `1d` result names its cap and claims no time span; the
/// status says what `B` does.
#[test]
fn traffic_truncated_header_80x24() {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::{Msg, QueryOutcome};
    use illumio_tui::tui::test_support::{FIXTURE_END, fixture_flows_rich};
    use illumio_tui::tui::update;
    let mut m = model_fixture();
    let end: jiff::Timestamp = FIXTURE_END.parse().unwrap();
    let flows = fixture_flows_rich();
    let n = flows.len();
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
                incomplete: false,
                elapsed: std::time::Duration::from_millis(900),
                hit: false,
                filter,
                bounds: Some((start, end)),
                hrefs: Vec::new(),
            }),
        },
    );
    let text = format!("{n} flows, capped at {n}; B raises the cap");
    assert_eq!(m.status.text, text);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains(&text), "{out}");
    assert!(!out.contains("covers"), "{out}");
    insta::assert_snapshot!("traffic_truncated_header_80x24", out);
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
fn app_to_app_sections_by_name_80x24() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Aggregate;
    gen_key(&mut m, 'x');
    gen_key(&mut m, 's');
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("in · by app A→Z"), "{out}");
    insta::assert_snapshot!("traffic_app_sections_by_name_80x24", out);
}

#[test]
fn flows_by_app_80x24() {
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    gen_key(&mut m, 'x');
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("by src app"), "{out}");
    // The cursor stays on the flat table's first flow, now under shop/prod.
    assert!(out.contains("▾ shop/prod  6 flows · 1004 conns"), "{out}");
    insta::assert_snapshot!("traffic_flows_by_src_app_80x24", out);
    // By destination app, by flows, the header's detail open.
    gen_key(&mut m, 'x');
    gen_key(&mut m, 's');
    gen_key(&mut m, 's');
    m.traffic.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("by dst app · by flows ↓"), "{out}");
    insta::assert_snapshot!("traffic_flows_by_dst_app_80x24", out);
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
fn workloads_filtered_80x24_and_140x30() {
    use illumio_tui::tui::model::Screen;
    use illumio_tui::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m.workloads.filters.unlabeled = true;
    m.workloads.rebuild(&m.limits.key_order());
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("filter unlabeled (app, env, role) · 2 of 4"),
        "{out}"
    );
    assert!(out.contains("missing"), "the column shows at 80: {out}");
    assert!(out.contains("app, env, role"), "{out}");
    assert!(!out.contains("E/m/o/u"), "no room at 80: {out}");
    insta::assert_snapshot!("workloads_unlabeled_80x24", out);
    m.workloads.filters.unlabeled = false;
    m.workloads.filters.offline = true;
    m.workloads.truncated = true;
    m.workloads.rebuild(&m.limits.key_order());
    let out = render_to_string(&m, 140, 30);
    assert!(
        out.contains("offline · 1 of 4 · partial: the list is truncated"),
        "{out}"
    );
    assert!(!out.contains("missing"), "u off: {out}");
    assert!(
        out.contains("h/H/v health  :ven  E/m/o/u  c clear"),
        "{out}"
    );
    insta::assert_snapshot!("workloads_offline_truncated_140x30", out);
    m.workloads.filters.offline = false;
    m.workloads.filters.enforcement = Some("idle");
    m.workloads.rebuild(&m.limits.key_order());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("no workloads match the filters"), "{out}");
}

#[test]
fn workloads_grouped_120x40_and_80x24() {
    use illumio_tui::tui::test_support as ts;
    let mut m = ts::model_with_many_workloads();
    m.screen = Screen::Workloads;
    gen_key(&mut m, 'x');
    let out = render_to_string(&m, 120, 40);
    assert!(out.contains("Workloads by app | env"), "{out}");
    assert!(out.contains("▾ crm | prod"), "{out}");
    assert!(out.contains("▾ (no app) | (no env)"), "{out}");
    assert!(out.contains("by size ↓ · 6 groups · 14"), "{out}");
    insta::assert_snapshot!("workloads_grouped_120x40", out);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("▾ crm | prod"), "{out}");
    insta::assert_snapshot!("workloads_grouped_80x24", out);
    // Folded, ordered by name, narrowed by a label filter with an
    // unknown value: the filter line names it, dimmed.
    gen_key(&mut m, 'z');
    gen_key(&mut m, 's');
    illumio_tui::tui::update(
        &mut m,
        illumio_tui::tui::msg::Msg::Command(illumio_tui::tui::commands::Command::Filter(
            "app=crm,shop,nope !env=dev".into(),
        )),
    );
    let out = render_to_string(&m, 120, 40);
    assert!(
        out.contains("filter labels app=crm,shop,nope !env=dev · 7 of 14"),
        "{out}"
    );
    assert!(out.contains("no match: unknown value app=nope"), "{out}");
    assert!(out.contains("▹ crm | prod"), "{out}");
    insta::assert_snapshot!("workloads_grouped_folded_filtered_120x40", out);
}

#[test]
fn workloads_label_filter_editor_80x24_and_120x40() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::test_support as ts;
    let mut m = ts::model_with_many_workloads();
    m.screen = Screen::Workloads;
    gen_key(&mut m, 'f');
    for c in "app=shop,crm !role=db en".chars() {
        gen_key(&mut m, c);
    }
    // `en` → `env=`, then into its values.
    for _ in 0..2 {
        illumio_tui::tui::update(
            &mut m,
            illumio_tui::tui::msg::Msg::Key(KeyEvent::new(
                KeyCode::Char('n'),
                KeyModifiers::CONTROL,
            )),
        );
    }
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Label filter"), "{out}");
    assert!(out.contains("app=shop,crm !role=db env=prod"), "{out}");
    assert!(out.contains("env=-"), "the strip offers no label: {out}");
    insta::assert_snapshot!("workloads_label_filter_editor_80x24", out);
    let out = render_to_string(&m, 120, 40);
    insta::assert_snapshot!("workloads_label_filter_editor_120x40", out);
}

/// The grouped Workloads view, its fold, detail pane, filter line and
/// label filter editor never panic at any terminal size.
#[test]
fn grouped_workloads_render_at_every_terminal_size() {
    use illumio_tui::tui::test_support as ts;
    let grouped = || {
        let mut m = ts::model_with_many_workloads();
        m.screen = Screen::Workloads;
        gen_key(&mut m, 'x');
        m
    };
    let mut models = vec![grouped()];
    let mut detail = grouped();
    gen_key(&mut detail, 'j');
    detail.workloads.show_detail = true;
    models.push(detail);
    let mut header_detail = grouped();
    header_detail.workloads.show_detail = true;
    models.push(header_detail);
    let mut folded = grouped();
    gen_key(&mut folded, 'z');
    models.push(folded);
    let mut filtered = grouped();
    illumio_tui::tui::update(
        &mut filtered,
        illumio_tui::tui::msg::Msg::Command(illumio_tui::tui::commands::Command::Filter(
            "app=crm,nope os=linux".into(),
        )),
    );
    models.push(filtered);
    let mut editor = grouped();
    gen_key(&mut editor, 'f');
    for c in "app=shop,crm !role=db".chars() {
        gen_key(&mut editor, c);
    }
    models.push(editor);
    for m in &models {
        for w in 20..=130 {
            for h in [6, 10, 16, 24, 40] {
                let out = render_to_string(m, w, h);
                assert!(!out.is_empty(), "{w}x{h}");
            }
        }
    }
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
        raw: true,
    };
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("\"event_type\": \"agent.tampering\""), "{out}");
    insta::assert_snapshot!("events_detail_160x48", out);
    // At 80x24 the box holds 22 JSON lines and its note says so.
    let small = render_to_string(&m, 80, 24);
    assert!(small.contains("lines 1-22/"), "{small}");
    assert!(small.contains("^d/^u"), "{small}");
    // Scrolled past the end: the view clamps so the box stays full and the
    // last line is visible.
    if let Overlay::EventDetail { scroll, .. } = &mut m.overlay {
        *scroll = 1000;
    }
    let end = render_to_string(&m, 80, 24);
    assert!(end.contains("lines "), "{end}");
    assert!(!end.contains("lines 1-22"), "{end}");
    assert!(
        end.contains("\n│}"),
        "last line (closing brace) visible: {end}"
    );
    insta::assert_snapshot!("events_detail_end_80x24", end);
}

#[test]
fn event_detail_change_diffs_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::{fixture_change_events, model_with_events};
    use illumio_tui::tui::update::update;
    let mut m = with_policy(model_with_events());
    m.screen = Screen::Events;
    let events = fixture_change_events();
    let names = [
        "event_diff_rule_update_80x24",
        "event_diff_ruleset_create_80x24",
        "event_diff_workload_labels_80x24",
        "event_diff_rule_delete_80x24",
        "event_diff_auth_failure_80x24",
    ];
    for (event, name) in events.iter().zip(names) {
        m.overlay = Overlay::EventDetail {
            event: event.clone(),
            scroll: 0,
            raw: false,
        };
        let out = render_to_string(&m, 80, 24);
        assert!(out.contains("J json"), "{out}");
        assert!(!out.contains("/orgs/5/labels/"), "label hrefs named: {out}");
        insta::assert_snapshot!(name, out);
    }
    // J shows the raw JSON from the top; J again goes back to the diff.
    m.overlay = Overlay::EventDetail {
        event: events[0].clone(),
        scroll: 3,
        raw: false,
    };
    let j = || Msg::Key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT));
    update(&mut m, j());
    assert!(matches!(
        m.overlay,
        Overlay::EventDetail {
            raw: true,
            scroll: 0,
            ..
        }
    ));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("\"event_type\": \"sec_rule.update\""), "{out}");
    assert!(out.contains("J diff"), "{out}");
    insta::assert_snapshot!("event_diff_raw_toggle_80x24", out);
    update(&mut m, j());
    assert!(matches!(m.overlay, Overlay::EventDetail { raw: false, .. }));
    // A long create (a sec_rule as the PCE logs it) scrolls; the meta says where.
    let create: pce::models::Event = serde_json::from_value(serde_json::json!({
        "href": "/orgs/5/events/c9",
        "timestamp": "2026-10-10T07:45:00.000Z",
        "event_type": "sec_rule.create",
        "severity": "info",
        "status": "success",
        "created_by": {"user": {"href": "/users/1", "username": "ops@example.com"}},
        "resource_changes": [{
            "change_type": "create",
            "changes": {
                "consumers": {"created": [{"actors": "ams"}, {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]},
                "providers": {"created": [{"label": {"href": "/orgs/5/labels/1"}}, {"label": {"href": "/orgs/5/labels/4"}}]},
                "ingress_services": {"created": [{"href": "/orgs/5/sec_policy/draft/services/32"}, {"port": 8443, "proto": 6}]},
                "enabled": {"after": true, "before": null},
                "log_flow": {"after": false, "before": null},
                "machine_auth": {"after": false, "before": null},
                "network_type": {"after": 2, "before": null},
                "resolve_labels": {"after": {"consumers": ["workloads"], "providers": ["workloads"]}, "before": null},
                "rule_list": {"after": {"href": "/orgs/5/sec_policy/draft/rule_sets/100"}, "before": null},
                "sec_connect": {"after": false, "before": null},
                "stateless": {"after": false, "before": null},
                "type": {"after": "allow", "before": null},
                "unscoped_consumers": {"after": false, "before": null},
                "use_workload_subnets": {"after": "none", "before": null}
            },
            "resource": {"sec_rule": {
                "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/21",
                "rule_set": {"href": "/orgs/5/sec_policy/draft/rule_sets/100", "name": "Shop-Prod"}
            }}
        }],
        "action": {"api_endpoint": "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules", "api_method": "POST",
                   "http_status_code": 201, "info": {"api_key_id": "k-1"}, "src_ip": "192.0.2.10"}
    }))
    .unwrap();
    m.overlay = Overlay::EventDetail {
        event: create,
        scroll: 3,
        raw: false,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("lines 4-25/25"), "{out}");
    assert!(out.contains("rule_list: Shop-Prod"), "{out}");
    insta::assert_snapshot!("event_diff_long_scrolled_80x24", out);
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
    let mut e = illumio_tui::tui::events_editor::EventsEditor::new([
        "err".into(),
        "agent".into(),
        String::new(),
        "24h".into(),
    ]);
    e.active = 1;
    m.overlay = Overlay::EventsEditor(e);
    insta::assert_snapshot!("events_editor_80x24", render_to_string(&m, 80, 24));
    m.overlay = Overlay::None;
    m.set_theme("light");
    insta::assert_snapshot!("light_events_160x48", render_styled(&m, 160, 48));
}

#[test]
fn events_hidden_types_capped_summary_and_type_completion() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_events;
    use illumio_tui::tui::{Msg, update};
    let mut m = model_with_events();
    m.screen = Screen::Events;
    m.events.truncated = true;
    m.events.filter.event_type = "!user.login, !system.*".into();
    m.events.filter.severity = "warning+".into();
    m.events.rebuild();
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("hiding 2 types · warning+ · 1 of 4 · capped at 4"),
        "{out}"
    );
    insta::assert_snapshot!("events_hidden_capped_80x24", out);
    m.overlay = Overlay::EventTypes {
        selected: 1,
        actors: false,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("hidden"), "{out}");
    assert!(out.contains("(severity)"), "{out}");
    insta::assert_snapshot!("event_types_80x24", out);
    // The Actors view (Tab): the actor filter hides one, and the types
    // filter and severity floor empty the others.
    m.events.filter.actor = "!=system, !=alex@ryte.de, !=bob@ryte.de".into();
    m.events.rebuild();
    m.overlay = Overlay::EventTypes {
        selected: 0,
        actors: true,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("Event actors") && out.contains("(type)"),
        "{out}"
    );
    // The typed actor terms are clipped in the header.
    assert!(out.contains("actor !=system, !=alex@ryte.d… ·"), "{out}");
    insta::assert_snapshot!("event_actors_80x24", out);
    m.events.filter.actor.clear();
    m.events.rebuild();
    // The Type field completes the loaded types after a `!`.
    m.overlay = Overlay::None;
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
    );
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    for c in ", !a".chars() {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
        );
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("!agent.tampering"), "{out}");
    insta::assert_snapshot!("events_editor_completing_80x24", out);
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
    assert!(out.contains("Scope*"), "{out}");
    assert!(out.contains("iplist:Corp wherever they are"), "{out}");
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
    use illumio_tui::tui::edit::{
        EditIntent, explain_unknown_ruleset, form_for, new_ruleset_offer,
    };
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
    form.error = Some(explain_unknown_ruleset(&form, e, &m.limits.scope_keys));
    m.overlay = Overlay::Form(form);
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("Ctrl-R: create it, scoped app=shop, env=prod"),
        "{out}"
    );
    assert!(out.contains("Ctrl-R new ruleset"), "{out}");
    assert!(out.contains("similar: Shop-Prod"), "{out}");
    assert!(out.contains("Ctrl-R creates it scoped"), "{out}");
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
    // Stacked below 92 columns.
    let out = render_to_string(&m, 91, 40);
    assert!(
        out.contains("src labels app=shop env=prod role=web"),
        "{out}"
    );
    assert!(
        out.contains("dst labels app=shop env=prod role=db"),
        "{out}"
    );
    assert!(out.contains("bytes in 120.0K"), "{out}");
    // In columns from 92.
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("│role  web"), "{out}");
    assert!(out.contains("│ role  db"), "{out}");
    assert!(out.contains("bytes in 120.0K out 4.0K"), "{out}");
    m.traffic.flows.select_next(); // Corp: an IP list, no labels, no bytes
    let out = render_to_string(&m, 91, 40);
    assert!(out.contains("src labels -"), "{out}");
    assert!(out.contains("ip lists: Corp"), "{out}");
    assert!(!out.contains("bytes in"), "{out}");
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("│ip lists"), "{out}");
    assert!(out.contains("│Corp "), "{out}");
    assert!(!out.contains("bytes in"), "{out}");
}

/// The detail pane's rows (the lines between `╭ Detail` and its bottom
/// border) of a render.
fn detail_pane(out: &str) -> Vec<String> {
    let mut pane: Vec<String> = Vec::new();
    for l in out.lines().skip_while(|l| !l.starts_with("╭ Detail")) {
        pane.push(l.to_string());
        if l.starts_with('╰') {
            break;
        }
    }
    pane
}

/// The character column `needle` starts at in `line`.
fn col_of(line: &str, needle: &str) -> usize {
    let i = line
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} in {line}"));
    line[..i].chars().count()
}

/// From 92 columns (and a tall enough body) the open Flows detail is three
/// columns: SOURCE and DESTINATION alike, SERVICE narrower, and the values
/// sit under their headings.
#[test]
fn flow_detail_in_columns_120x40() {
    let mut m = model_with_flows();
    m.traffic.show_detail = true;
    let out = render_to_string(&m, 120, 40);
    let pane = detail_pane(&out);
    assert_eq!(
        pane.len(),
        10,
        "border, headings, 6 rows, footer, border: {out}"
    );
    let head = &pane[1];
    let (src, dst, svc) = (
        col_of(head, "SOURCE"),
        col_of(head, "DESTINATION"),
        col_of(head, "SERVICE"),
    );
    assert!(src < dst && dst < svc, "{head}");
    assert_eq!(dst - src, svc - dst, "equal endpoint columns: {head}");
    assert!(120 - svc < svc - dst, "a narrower service column: {head}");
    let at = |row: usize, needle: &str| col_of(&pane[row], needle);
    assert_eq!(at(2, "host-1.example"), src);
    assert_eq!(at(2, "host-2.example"), dst);
    assert_eq!(at(2, "5432/tcp"), svc);
    assert_eq!(at(3, "10.0.1.10"), src);
    assert_eq!(at(3, "10.0.2.20"), dst);
    assert_eq!(at(3, "process  postgres"), svc);
    assert_eq!(at(4, "app   shop"), src);
    assert_eq!(at(4, "user     postgres"), svc);
    assert_eq!(at(5, "allowed"), svc);
    assert_eq!(at(6, "role  web"), src);
    assert_eq!(at(6, "role  db"), dst);
    assert!(
        pane[8]
            .contains("seen 2026-10-07 08:00 → 10-10 07:31 · bytes in 120.0K out 4.0K · rules n/a"),
        "{out}"
    );
    insta::assert_snapshot!("traffic_flow_detail_columns_120x40", pane.join("\n"));
    // An IP-list source: its lists where labels would be.
    m.traffic.flows.select_next();
    let out = render_to_string(&m, 120, 40);
    let pane = detail_pane(&out);
    assert_eq!(col_of(&pane[2], "10.1.4.9"), src, "{out}");
    assert_eq!(col_of(&pane[3], "ip lists"), src, "{out}");
    assert_eq!(col_of(&pane[4], "Corp"), src, "{out}");
    assert_eq!(col_of(&pane[3], "potentially_blocked"), svc, "{out}");
    insta::assert_snapshot!(
        "traffic_flow_detail_columns_ip_list_120x40",
        pane.join("\n")
    );
}

/// Narrow or short terminals keep the stacked detail.
#[test]
fn flow_detail_stays_stacked_when_narrow_or_short() {
    let mut m = model_with_flows();
    m.traffic.show_detail = true;
    for (w, h) in [(80, 40), (91, 40), (120, 30)] {
        let out = render_to_string(&m, w, h);
        assert!(!out.contains("SOURCE"), "{w}x{h}: {out}");
        assert!(out.contains("src labels"), "{w}x{h}: {out}");
        assert_eq!(detail_pane(&out).len(), 8, "{w}x{h}: {out}");
    }
    insta::assert_snapshot!(
        "traffic_flow_detail_stacked_80x40",
        detail_pane(&render_to_string(&m, 80, 40)).join("\n")
    );
}

/// The Flows detail at every width and the heights around the columns'
/// threshold: plain, drafted, marked (the mark line), grouped (a section
/// header), a long-labelled flow and the IP-list one. No panics, and the
/// columns only where they fit.
#[test]
fn flow_detail_renders_at_every_terminal_size() {
    use illumio_tui::tui::test_support as ts;
    let dir = tempfile::tempdir().unwrap();
    let mut models = vec![model_with_flows()];
    let mut ip = model_with_flows();
    ip.traffic.flows.select_next();
    models.push(ip);
    models.push(ts::with_draft_loaded(ts::model_with_rich_flows()));
    models.push(ts::model_with_marks(dir.path(), ts::RICH_MARKS));
    for keys in ["x", "xj"] {
        let mut g = ts::model_with_rich_flows();
        for c in keys.chars() {
            gen_key(&mut g, c);
        }
        models.push(g);
    }
    for m in &mut models {
        m.traffic.show_detail = true;
    }
    for m in &models {
        // A section header keeps its two lines; a flow gets the columns.
        let on_flow = m.traffic.selected_flow().is_some();
        for w in 20..=130 {
            for h in [6, 10, 22, 23, 30, 31, 32, 40] {
                let out = render_to_string(m, w, h);
                let columns = out.contains("│SOURCE");
                if w < 92 || h < 31 {
                    assert!(!columns, "{w}x{h}: {out}");
                } else {
                    assert_eq!(columns, on_flow, "{w}x{h}: {out}");
                }
            }
        }
    }
}

/// The stacked detail cuts its rows instead of wrapping them, so a long
/// `seen` row never pushes the `mark` line out of the pane.
#[test]
fn stacked_flow_detail_keeps_the_mark_line_80x30() {
    use illumio_tui::tui::test_support as ts;
    let dir = tempfile::tempdir().unwrap();
    let mut m = ts::model_with_marks(dir.path(), ts::RICH_MARKS);
    m.traffic.show_detail = true;
    while m.traffic.selected_flow().is_some_and(|f| f.mark.is_none()) {
        m.traffic.flows.select_next();
    }
    assert!(m.traffic.selected_flow().is_some(), "a marked flow");
    let out = render_to_string(&m, 80, 30);
    let pane = detail_pane(&out);
    assert_eq!(pane.len(), 9, "8 rows and the mark line: {out}");
    assert!(pane[7].starts_with("│mark       "), "{out}");
    let seen = pane[6].trim_end_matches('│').trim_end();
    assert!(seen.starts_with("│seen       "), "{out}");
    assert!(
        seen.ends_with('…') || seen.contains("· rules "),
        "cut or whole: {seen}"
    );
    insta::assert_snapshot!("traffic_flow_detail_stacked_mark_80x30", pane.join("\n"));
}

/// In columns the table is shorter; `j` past a page keeps the cursor on
/// screen (scrolling and drawing use the same layout), marks or not.
#[test]
fn the_flow_table_scrolls_with_the_columns_detail_open() {
    use illumio_tui::tui::test_support as ts;
    let dir = tempfile::tempdir().unwrap();
    for mut m in [
        ts::model_with_rich_flows(),
        ts::model_with_marks(dir.path(), ts::RICH_MARKS),
    ] {
        m.traffic.show_detail = true;
        m.size = (120, 40);
        let n = m.traffic.flows.len();
        for _ in 0..n - 1 {
            gen_key(&mut m, 'j');
            let out = render_to_string(&m, 120, 40);
            assert!(out.contains("│SOURCE"), "{out}");
            let table: String = out
                .lines()
                .take_while(|l| !l.starts_with("╭ Detail"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(table.contains("│▸"), "the cursor is on screen: {out}");
        }
        assert!(n > 20, "more than a page: {n}");
    }
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
fn open_app(m: &mut illumio_tui::tui::model::Model, app: &str) {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    illumio_tui::tui::update(m, Msg::Command(Command::App(Some(app.into()))));
}

/// The rich flows in a known 24h window, workloads and rulesets loaded:
/// every panel filled, nothing sent.
fn app_overlay_model() -> illumio_tui::tui::model::Model {
    let mut m = with_policy(with_workloads(model_with_window("24h")));
    open_app(&mut m, "shop/prod");
    m
}

#[test]
fn app_overlay_80x24() {
    let m = app_overlay_model();
    let out = render_to_string(&m, 80, 24);
    for s in [
        "shop/prod · in+out · last 24h",
        "0 req · cached",
        "Workloads 2",
        "enforced 1 · visibility 1",
        "VENs  1 active · 1 stale",
        "Inbound",
        "Outbound",
        "RS Shop-Prod",
        "host-1.example",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    assert!(
        !out.contains("Blocked over time") && !out.contains("approx"),
        "the timeline panel is gone: {out}"
    );
    insta::assert_snapshot!("app_overlay_80x24", out);
}

#[test]
fn app_overlay_120x40() {
    let m = app_overlay_model();
    insta::assert_snapshot!("app_overlay_120x40", render_to_string(&m, 120, 40));
}

#[test]
fn app_overlay_light_styled_80x24() {
    let mut m = app_overlay_model();
    m.set_theme("light");
    insta::assert_snapshot!("app_overlay_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn app_overlay_loading_80x24() {
    let mut m = model_with_flows();
    open_app(&mut m, "shop/prod");
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("2 req") && out.contains("loading"), "{out}");
    insta::assert_snapshot!("app_overlay_loading_80x24", out);
}

#[test]
fn app_overlay_no_flows_80x24() {
    let mut m = with_policy(with_workloads(model_with_window("24h")));
    m.traffic.data.as_mut().unwrap().flows.clear();
    m.traffic.rebuild_views();
    open_app(&mut m, "shop/prod");
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("no flows in the window"), "{out}");
    assert!(
        out.contains("host-1.example"),
        "workloads still load: {out}"
    );
    insta::assert_snapshot!("app_overlay_no_flows_80x24", out);
}

#[test]
fn app_overlay_workloads_error_80x24() {
    use illumio_tui::tui::msg::Msg;
    let mut m = with_policy(model_with_window("24h"));
    open_app(&mut m, "shop/prod");
    let seq = m.app_view.as_ref().unwrap().seq;
    illumio_tui::tui::update(
        &mut m,
        Msg::AppWorkloadsLoaded {
            seq,
            result: Err("HTTP 500".into()),
        },
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("HTTP 500 · r retries"), "{out}");
    assert!(
        out.contains("RS Shop-Prod"),
        "the policy panel still renders: {out}"
    );
    insta::assert_snapshot!("app_overlay_workloads_error_80x24", out);
}

#[test]
fn traffic_draft_flows_pairs_and_sections() {
    use illumio_tui::tui::model::{By, TrafficView};
    use illumio_tui::tui::test_support::{model_with_rich_flows, with_draft_loaded};
    let mut m = with_draft_loaded(model_with_rich_flows());
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("draft ✓"), "{out}");
    assert!(out.contains("→ allowed"), "{out}");
    insta::assert_snapshot!("traffic_draft_flows_160x48", out);
    // At 120 columns the draft values are whole, not cut with `…`.
    let out = render_to_string(&m, 120, 36);
    assert!(out.contains("→ allowed"), "{out}");
    assert!(out.contains("→ block"), "{out}");
    assert!(
        !out.contains("→ allowed…") && !out.contains("allowed →"),
        "{out}"
    );
    assert!(
        out.lines()
            .filter(|l| l.contains("→ "))
            .all(|l| !l.contains('…') || !l.split("→ ").nth(1).unwrap_or("").contains('…')),
        "{out}"
    );
    insta::assert_snapshot!("traffic_draft_flows_120x36", out);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("→ block"), "{out}");
    insta::assert_snapshot!("traffic_draft_flows_80x24", out);
    m.traffic.view = TrafficView::Aggregate;
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains("would allow"), "{out}");
    insta::assert_snapshot!("traffic_draft_pairs_120x30", out);
    insta::assert_snapshot!("traffic_draft_pairs_80x24", render_to_string(&m, 80, 24));
    m.traffic.apps_state.by = By::In;
    m.traffic.rebuild_sections();
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains("draft allows"), "{out}");
    insta::assert_snapshot!("traffic_draft_sections_in_120x30", out);
    insta::assert_snapshot!(
        "traffic_draft_sections_in_80x24",
        render_to_string(&m, 80, 24)
    );
    m.traffic.view = TrafficView::Mesh;
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains("would block"), "{out}");
    insta::assert_snapshot!("traffic_draft_mesh_120x30", out);
}

#[test]
fn app_overlay_draft_120x40() {
    use illumio_tui::tui::test_support::with_draft_loaded;
    let mut m = with_draft_loaded(with_policy(with_workloads(model_with_window("24h"))));
    open_app(&mut m, "shop/prod");
    let out = render_to_string(&m, 120, 40);
    assert!(out.contains("draft: "), "{out}");
    assert!(out.contains("blocked flows fixed"), "{out}");
    insta::assert_snapshot!("app_overlay_draft_120x40", out);
}

/// `w` in the app overlay with writes off: the refusal outranks the
/// overlay's hints, whole, at 80 and 120 columns (a cut refusal reads as
/// a key that does nothing).
#[test]
fn app_overlay_w_with_writes_off_shows_the_whole_refusal() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::WRITES_OFF;
    use illumio_tui::tui::msg::Msg;
    let key = |c| Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    let mut m = app_overlay_model();
    assert!(!m.writes.enabled, "the fixture starts with writes off");
    for c in ['j', 'j', 'j', 'w'] {
        illumio_tui::tui::update(&mut m, key(c));
    }
    assert_eq!(m.status.text, WRITES_OFF);
    for (w, h) in [(80, 24), (120, 30)] {
        let out = render_to_string(&m, w, h);
        let last = out.lines().last().unwrap_or_default();
        assert!(last.contains(WRITES_OFF), "{w}x{h}: {last:?}");
        assert!(last.contains("Esc back"), "{w}x{h}: {last:?}");
    }
    insta::assert_snapshot!("app_overlay_writes_off_80x24", render_to_string(&m, 80, 24));
    // An info line still shares the row with every hint.
    illumio_tui::tui::update(&mut m, key('k'));
    m.status = illumio_tui::tui::model::StatusLine::info("shop/prod · 0 req");
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.lines().last().unwrap_or_default().contains("w rule"),
        "{out}"
    );
}

#[test]
fn confirm_batch_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::ConfirmBatch {
        batch: illumio_tui::tui::test_support::batch_plan(),
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Send these requests?",
        "3 requests: 1 ip_list, 1 rule_set, 1 rule · planned with 2 reads",
        "1/3 POST /orgs/5/sec_policy/draft/ip_lists",
        "more · j/k scroll · y send all · n cancel",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("confirm_batch_80x24", out);
}

#[test]
fn confirm_batch_spells_placeholders_120x48() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::ConfirmBatch {
        batch: illumio_tui::tui::test_support::batch_plan(),
        scroll: 0,
    };
    let out = render_to_string(&m, 120, 48);
    assert!(
        out.contains("<href of step 2: create rule_set RF-crm-prod>/sec_rules"),
        "{out}"
    );
    assert!(!out.contains("{step:"), "{out}");
    insta::assert_snapshot!("confirm_batch_120x48", out);
}

#[test]
fn batch_report_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.batch_report = Some(vec![
        "1/3 create ip_list shop-prod peer 203.0.113.7".into(),
        "    written /orgs/5/sec_policy/draft/ip_lists/77".into(),
        "2/3 create rule_set RF-crm-prod".into(),
        "    FAILED: name in use".into(),
        "3/3 create rule in rule_set RF-crm-prod".into(),
        "    not sent".into(),
        String::new(),
        "These are draft changes, not provisioned: p on Policy lists them, R reverts the marked ones.".into(),
    ]);
    m.overlay = Overlay::BatchReport { scroll: 0 };
    let out = render_to_string(&m, 80, 24);
    for s in ["Batch result", "FAILED: name in use", "Esc closes"] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("batch_report_80x24", out);
}

/// The rich flows in a known 24h window, the rulesets loaded, the
/// generator open for shop/prod: nothing sent.
fn generator_model() -> illumio_tui::tui::model::Model {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = with_policy(model_with_window("24h"));
    // The index the rich flows were resolved with: every label has its name.
    m.connection = Connection::Connected {
        index: Some(illumio_tui::tui::test_support::rich_index()),
        index_age_secs: 0,
    };
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Gen(Some("shop/prod".into()))));
    m
}

/// `generator_model` with fan-in from 3 peers (5432/tcp: hr/prod,
/// shop/dev, Corp) and the cursor on its line.
fn generator_fan_model() -> illumio_tui::tui::model::Model {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = with_policy(model_with_window("24h"));
    m.connection = Connection::Connected {
        index: Some(illumio_tui::tui::test_support::rich_index()),
        index_age_secs: 0,
    };
    m.limits.consolidate_min_peers = 3;
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Gen(Some("shop/prod".into()))));
    m
}

fn gen_key(m: &mut illumio_tui::tui::model::Model, c: char) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    illumio_tui::tui::update(
        m,
        illumio_tui::tui::msg::Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
    );
}

#[test]
fn generator_80x24() {
    let m = generator_model();
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Generate · shop/prod · last 24h · 8 flows · complete",
        "0 req · cached",
        "[app·ports]",
        "target  Shop-Prod (draft, 2 rules)",
        "Inbound → Shop-Prod",
        "[x] intra-app",
        "⚠ env mismatch",
        "Outbound (o: off)",
        "2 rows hidden: o shows them",
        "2 rules (2 new · 0 add services · 0 exist) · covers 5 of 8 flows · 3 not",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_80x24", out);
}

#[test]
fn generator_outbound_role_ports_120x40() {
    let mut m = generator_model();
    gen_key(&mut m, 'o');
    gen_key(&mut m, 'l');
    let out = render_to_string(&m, 120, 40);
    for s in [
        "[role·ports]",
        "Outbound (o: on) · IP peers → Shop-Prod",
        "→ RF-crm-prod",
        "⚠ needs an IP list",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_outbound_role_ports_120x40", out);
}

#[test]
fn generator_rule_list_120x40() {
    let mut m = generator_model();
    gen_key(&mut m, 'p');
    let out = render_to_string(&m, 120, 40);
    assert!(
        out.contains("1. create · Shop-Prod: all workloads → all workloads on 5432/tcp"),
        "{out}"
    );
    assert!(out.contains("not covered: 3 unchecked"), "{out}");
    insta::assert_snapshot!("generator_rule_list_120x40", out);
}

#[test]
fn generator_incomplete_80x24() {
    let mut m = with_policy(model_with_window("24h"));
    m.traffic.data.as_mut().unwrap().truncated = true;
    {
        use illumio_tui::tui::commands::Command;
        use illumio_tui::tui::msg::Msg;
        illumio_tui::tui::update(&mut m, Msg::Command(Command::Gen(Some("shop/prod".into()))));
    }
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("incomplete: r re-queries shop/prod (1 query)"),
        "{out}"
    );
    insta::assert_snapshot!("generator_incomplete_80x24", out);
}

#[test]
fn generator_outbound_80x24() {
    let mut m = generator_model();
    gen_key(&mut m, 'o');
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Outbound (o: on) · IP peers → Shop-Prod",
        "→ RF-crm-prod",
        "⚠ needs an IP list",
        "10 flows",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_outbound_80x24", out);
}

#[test]
fn generator_rule_list_80x24() {
    let mut m = generator_model();
    gen_key(&mut m, 'p');
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("1. create · Shop-Prod"), "{out}");
    assert!(out.contains("j/k scroll · Esc back to the rows"), "{out}");
    insta::assert_snapshot!("generator_rule_list_80x24", out);
}

/// A rule an override deny blocks, and one without a ruleset: the rows
/// behind them say so.
#[test]
fn generator_override_deny_and_no_target_80x24() {
    let mut m = generator_model();
    {
        let v = m.gen_view.as_mut().unwrap();
        let p = v.proposal.as_mut().unwrap();
        p.rules[0].denied_flows = 2;
        p.rules[0].denied_by = vec!["deny-db".into()];
        p.targets[0] = pce::generate::Target::None("no ruleset can be named".into());
    }
    let out = render_to_string(&m, 80, 24);
    for s in ["⚠ override deny: 2", "⚠ no target"] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_override_deny_and_no_target_80x24", out);
}

/// Every row checked at role·ports with outbound on: more rules than the
/// 80x24 body holds; `p` scrolled to its end shows the last line.
#[test]
fn generator_rule_list_scrolled_to_the_end_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut m = generator_model();
    gen_key(&mut m, 'o');
    gen_key(&mut m, 'l');
    {
        let v = m.gen_view.as_mut().unwrap();
        for r in v.rows.iter_mut().filter(|r| r.checkable) {
            r.checked = true;
        }
    }
    gen_key(&mut m, '='); // any key that re-derives
    gen_key(&mut m, 'p');
    for _ in 0..100 {
        gen_key(&mut m, 'j');
    }
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("not covered:"), "{out}");
    assert!(out.contains("more above"), "{out}");
    insta::assert_snapshot!("generator_rule_list_scrolled_to_the_end_80x24", out);
    illumio_tui::tui::update(
        &mut m,
        illumio_tui::tui::msg::Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
}

/// More rows than the 80x24 body holds: the list follows the cursor and
/// says what is above and below.
#[test]
fn generator_rows_scroll_with_the_cursor_80x24() {
    let mut m = generator_model();
    gen_key(&mut m, 'o');
    {
        // Twelve more inbound IP peers.
        let v = m.gen_view.as_mut().unwrap();
        let ip = v
            .rows
            .iter()
            .find(|r| matches!(r.peer, pce::generate::PeerKind::IpList { .. }))
            .cloned()
            .unwrap();
        for n in 0..12 {
            let mut r = ip.clone();
            r.peer = pce::generate::PeerKind::Ip {
                ip: format!("10.9.9.{n}"),
            };
            v.rows.insert(4, r);
        }
        v.section = pce::generate::Side::Inbound;
        v.cursor = [0, 0];
    }
    let top = render_to_string(&m, 80, 24);
    assert!(top.contains("more below"), "{top}");
    for _ in 0..16 {
        gen_key(&mut m, 'j');
    }
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("more above"), "{out}");
    assert!(
        out.contains("Outbound (o: on)"),
        "the outbound title comes along:\n{out}"
    );
    insta::assert_snapshot!("generator_rows_scroll_with_the_cursor_80x24", out);
}

/// Flags that do not fit at 80 columns: the first and how many more.
#[test]
fn generator_flags_that_do_not_fit_say_how_many_more_80x24() {
    let mut m = generator_model();
    {
        let v = m.gen_view.as_mut().unwrap();
        let p = v.proposal.as_mut().unwrap();
        p.rules[0].denied_flows = 2;
        p.targets[0] = pce::generate::Target::None("no ruleset can be named".into());
    }
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("⚠ override deny: 2 +1"), "{out}");
    let wide = render_to_string(&m, 120, 40);
    assert!(wide.contains("⚠ override deny: 2 ⚠ no target"), "{wide}");
}

#[test]
fn generator_fan_in_off_80x24() {
    let m = generator_fan_model();
    let out = render_to_string(&m, 80, 24);
    for s in [
        "⚠ 5432/tcp from 3 peers · c consolidates: off",
        "c consolidates",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_fan_in_off_80x24", out);
}

#[test]
fn generator_fan_in_any_80x24() {
    let mut m = generator_fan_model();
    gen_key(&mut m, 'c');
    let out = render_to_string(&m, 80, 24);
    for s in [
        "c consolidates: Any · 1 rule",
        "also allows: shop/dev",
        "⚠ consolidated",
        "covers 7 of 8 flows",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_fan_in_any_80x24", out);
}

#[test]
fn generator_fan_in_all_workloads_rule_list_80x24() {
    let mut m = generator_fan_model();
    gen_key(&mut m, 'c');
    gen_key(&mut m, 'c');
    gen_key(&mut m, 'p');
    let out = render_to_string(&m, 80, 24);
    for s in [
        "unscoped",
        "consolidates 5432/tcp from 2 peers",
        "every VEN",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("generator_fan_in_all_workloads_rule_list_80x24", out);
}

/// Marks at the standard size: the Flows view with its gutter and the
/// detail's mark line, App-to-app's pairs.
#[test]
fn traffic_marks_80x24() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    insta::assert_snapshot!("traffic_marks_flows_80x24", render_to_string(&m, 80, 24));
    m.traffic.view = TrafficView::Aggregate;
    insta::assert_snapshot!("traffic_marks_pairs_80x24", render_to_string(&m, 80, 24));
    // The light palette: the gutter's colours, the red critical name
    // cell, the dimmed quiet rows.
    m.set_theme("light");
    insta::assert_snapshot!(
        "traffic_marks_pairs_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn traffic_marks_pairs_and_blocked_120x34() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    m.traffic.view = TrafficView::Aggregate;
    insta::assert_snapshot!("traffic_marks_pairs_120x34", render_to_string(&m, 120, 34));
    m.traffic.view = TrafficView::Blocked;
    insta::assert_snapshot!(
        "traffic_marks_blocked_120x34",
        render_to_string(&m, 120, 34)
    );
    m.traffic.toggle_squelched();
    m.traffic.view = TrafficView::Aggregate;
    insta::assert_snapshot!(
        "traffic_marks_pairs_shown_120x34",
        render_to_string(&m, 120, 34)
    );
}

#[test]
fn mark_form_from_blocked_100x30() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    use illumio_tui::tui::{Msg, update};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    m.traffic.view = TrafficView::Blocked;
    let key = |c| Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    update(&mut m, key('j'));
    update(&mut m, key('m'));
    for c in "CHG-1182, owner confirmed".chars() {
        update(&mut m, key(c));
    }
    insta::assert_snapshot!(
        "mark_form_from_blocked_100x30",
        render_to_string(&m, 100, 30)
    );
}

#[test]
fn marks_overlay_120x30() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    use illumio_tui::tui::{Msg, update};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    update(&mut m, Msg::Command(Command::Marks(None)));
    insta::assert_snapshot!("marks_overlay_120x30", render_to_string(&m, 120, 30));
}

/// No terminal size panics the generator: every width from 20 to 130 and a
/// few heights, in its main states (rows, outbound at a role stop, a fan-in
/// line off and switched on, the rule list).
#[test]
fn generator_renders_at_every_terminal_size() {
    let states: [(&dyn Fn() -> illumio_tui::tui::model::Model, &str); 5] = [
        (&generator_model, ""),
        (&generator_model, "ol"),
        (&generator_fan_model, ""),
        (&generator_fan_model, "c"),
        (&generator_model, "p"),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (make, keys, marks) in states
        .into_iter()
        .map(|(make, keys)| (make, keys, false))
        .chain([(&generator_model as &dyn Fn() -> _, "", true)])
    {
        let mut m = make();
        if marks {
            // The header's squelched count (the rules include them).
            std::fs::write(
                dir.path().join("marks.toml"),
                illumio_tui::tui::test_support::RICH_MARKS,
            )
            .unwrap();
            m.traffic.marks = illumio_tui::marks::MarksStore::new(dir.path().join("marks.toml"));
            open_gen_again(&mut m);
            assert!(gen_header(&m).contains("squelched (rules include them)"));
        }
        for c in keys.chars() {
            gen_key(&mut m, c);
        }
        for w in 20..=130 {
            for h in [6, 10, 24, 40] {
                let _ = render_to_string(&m, w, h);
            }
        }
    }
}

/// `:gen` again for the generator's app (its marks read afresh).
fn open_gen_again(m: &mut illumio_tui::tui::model::Model) {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    illumio_tui::tui::update(m, Msg::Command(Command::Gen(Some("shop/prod".into()))));
}

fn gen_header(m: &illumio_tui::tui::model::Model) -> String {
    m.gen_view
        .as_ref()
        .map(|v| v.header_title())
        .unwrap_or_default()
}

/// No terminal size panics any screen, traffic view or the app overlay:
/// widths 20 to 130, a short and a normal height.
#[test]
fn screens_render_at_every_terminal_size() {
    use illumio_tui::tui::test_support as ts;
    let dir = tempfile::tempdir().unwrap(); // marks.toml for the marked models
    let views = "vvvvv"; // Flows → App-to-app → Mesh → Blocked → Treemap → Flows
    let mut models: Vec<illumio_tui::tui::model::Model> = vec![
        ts::model_with_policy(),
        ts::model_with_objects(),
        ts::model_with_ip_lists(),
        ts::model_with_workloads(),
        ts::model_with_events(),
        app_overlay_model(),
    ];
    // The app overlay with marks (the gutter, the panel titles), and `M`.
    for keys in ["", "M"] {
        let mut a = with_policy(with_workloads(ts::model_with_marks(
            dir.path(),
            ts::RICH_MARKS,
        )));
        open_app(&mut a, "shop/prod");
        for c in keys.chars() {
            gen_key(&mut a, c);
        }
        assert!(a.app_view.as_ref().is_some_and(|v| v.marks_on));
        models.push(a);
    }
    for i in 0..views.len() {
        let mut m = ts::model_with_rich_flows();
        for c in views[..i].chars() {
            gen_key(&mut m, c);
        }
        models.push(m);
        let mut x = ts::model_with_rich_flows();
        gen_key(&mut x, 'v');
        gen_key(&mut x, 'x');
        models.push(x);
        // Sections in another order (`s`), Flows by app (`x`, `X`), a
        // folded section and a header's detail.
        for keys in ["vxs", "vxxxss", "x", "Xss", "xjz", "xz"] {
            let mut g = ts::model_with_rich_flows();
            for c in views[..i].chars().chain(keys.chars()) {
                gen_key(&mut g, c);
            }
            g.traffic.show_detail = true;
            models.push(g);
        }
        // The same views with marks: the gutter, the metas, `M`, the
        // sections and the Flows detail's mark line.
        for keys in ["", "x", "M", "xM", "xs", "Xj"] {
            let mut mk = ts::model_with_marks(dir.path(), ts::RICH_MARKS);
            for c in views[..i].chars() {
                gen_key(&mut mk, c);
            }
            for c in keys.chars() {
                gen_key(&mut mk, c);
            }
            mk.traffic.show_detail = true;
            models.push(mk);
        }
        // The mark form on the view's row: as opened, with a long comment,
        // and with a bad field (the error, no preview).
        for (back, typed) in [
            (0, ""),
            (0, "a long reason that runs past the value column, CHG-1182"),
            (5, "x"),
        ] {
            let mut mf = ts::model_with_marks(dir.path(), ts::RICH_MARKS);
            for c in views[..i].chars() {
                gen_key(&mut mf, c);
            }
            gen_key(&mut mf, 'm');
            for _ in 0..back {
                illumio_tui::tui::update(
                    &mut mf,
                    illumio_tui::tui::msg::Msg::Key(crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::BackTab,
                        crossterm::event::KeyModifiers::SHIFT,
                    )),
                );
            }
            for c in typed.chars() {
                gen_key(&mut mf, c);
            }
            // The treemap (the fifth view) drills first: no form there.
            match &mf.overlay {
                illumio_tui::tui::model::Overlay::MarkForm(f) => {
                    assert_eq!(f.form.error.is_some(), back > 0, "{:?}", f.form.error);
                }
                o => assert_eq!(i, 4, "no form in view {i}: {o:?}"),
            }
            models.push(mf);
        }
    }
    // The mark form from the app overlay.
    let mut am = with_policy(with_workloads(ts::model_with_marks(
        dir.path(),
        ts::RICH_MARKS,
    )));
    open_app(&mut am, "shop/prod");
    gen_key(&mut am, 'm');
    assert!(matches!(
        am.overlay,
        illumio_tui::tui::model::Overlay::MarkForm(_)
    ));
    models.push(am);
    // `:marks`: the list, its delete confirm, the extend and edit forms,
    // an invalid file's problems, and the list with no traffic loaded.
    for keys in ["", "x", "\n"] {
        let mut mm = ts::model_with_marks(dir.path(), ts::RICH_MARKS);
        illumio_tui::tui::update(
            &mut mm,
            illumio_tui::tui::msg::Msg::Command(illumio_tui::tui::commands::Command::Marks(None)),
        );
        for c in keys.chars() {
            let code = match c {
                '\n' => crossterm::event::KeyCode::Enter,
                c => crossterm::event::KeyCode::Char(c),
            };
            illumio_tui::tui::update(
                &mut mm,
                illumio_tui::tui::msg::Msg::Key(crossterm::event::KeyEvent::new(
                    code,
                    crossterm::event::KeyModifiers::NONE,
                )),
            );
        }
        assert!(!matches!(
            mm.overlay,
            illumio_tui::tui::model::Overlay::None
        ));
        models.push(mm);
    }
    let bad = tempfile::tempdir().unwrap();
    let mut inv = ts::model_with_marks(
        bad.path(),
        "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\n",
    );
    gen_key(&mut inv, 'm');
    assert!(matches!(
        inv.overlay,
        illumio_tui::tui::model::Overlay::Marks
    ));
    models.push(inv);
    // `e` on a mark that expires: the extend form (the comment alone).
    let mut ext = ts::model_with_marks(
        bad.path(),
        "[[mark]]\nid = \"m-soon01\"\naction = \"squelch\"\nsrc = { ip_list = \"Corp\" }\ndst = \"any\"\nexpires = \"2099-01-01T23:59:59Z\"\ncomment = \"scanner\"\n",
    );
    illumio_tui::tui::update(
        &mut ext,
        illumio_tui::tui::msg::Msg::Command(illumio_tui::tui::commands::Command::Marks(None)),
    );
    gen_key(&mut ext, 'e');
    assert!(
        matches!(
            &ext.overlay,
            illumio_tui::tui::model::Overlay::MarkForm(f) if f.ctx.extend
        ),
        "{:?} {:?}",
        ext.status,
        ext.traffic.marks.error()
    );
    models.push(ext);
    let mut bare = ts::model_fixture();
    std::fs::write(bad.path().join("bare.toml"), ts::RICH_MARKS).unwrap();
    bare.traffic.marks = illumio_tui::marks::MarksStore::new(bad.path().join("bare.toml"));
    illumio_tui::tui::update(
        &mut bare,
        illumio_tui::tui::msg::Msg::Command(illumio_tui::tui::commands::Command::Marks(None)),
    );
    models.push(bare);
    for m in &models {
        for w in 20..=130 {
            for h in [6, 24] {
                let _ = render_to_string(m, w, h);
            }
        }
    }
}

/// Workloads with writes on, every workload marked (`V`), nothing sent.
fn bulk_model() -> illumio_tui::tui::model::Model {
    use illumio_tui::tui::model::{Screen, WritesState};
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m.writes = WritesState::on();
    bulk_key(&mut m, KeyCodeAlias::Char('V'));
    m
}

type KeyCodeAlias = crossterm::event::KeyCode;

fn bulk_key(m: &mut illumio_tui::tui::model::Model, c: KeyCodeAlias) {
    use crossterm::event::{KeyEvent, KeyModifiers};
    use illumio_tui::tui::msg::Msg;
    let cmds = illumio_tui::tui::update(m, Msg::Key(KeyEvent::new(c, KeyModifiers::NONE)));
    assert!(cmds.is_empty(), "{cmds:?}");
}

/// `M` moved to full, then Enter: the confirm with the typed count.
fn bulk_confirm_model() -> illumio_tui::tui::model::Model {
    let mut m = bulk_model();
    bulk_key(&mut m, KeyCodeAlias::Char('M'));
    bulk_key(&mut m, KeyCodeAlias::Char('j'));
    bulk_key(&mut m, KeyCodeAlias::Char('j'));
    bulk_key(&mut m, KeyCodeAlias::Enter);
    m
}

#[test]
fn bulk_marked_rows_80x24() {
    let mut m = bulk_model();
    bulk_key(&mut m, KeyCodeAlias::Char('j'));
    bulk_key(&mut m, KeyCodeAlias::Char(' ')); // unmark host-2
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("3 marked"), "{out}");
    assert!(out.contains("● host-1.example"), "{out}");
    assert!(!out.contains("● host-2.example"), "{out}");
    insta::assert_snapshot!("bulk_marked_rows_80x24", out);
    // Grouped (`x`): the same marks on the members.
    bulk_key(&mut m, KeyCodeAlias::Char('x'));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("● host-1.example"), "{out}");
    assert!(out.contains("3 marked"), "{out}");
}

#[test]
fn bulk_labels_form_80x24() {
    let mut m = bulk_model();
    bulk_key(&mut m, KeyCodeAlias::Char('L'));
    for c in "brandnew".chars() {
        bulk_key(&mut m, KeyCodeAlias::Char(c));
    }
    bulk_key(&mut m, KeyCodeAlias::Tab);
    bulk_key(&mut m, KeyCodeAlias::Char('-'));
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Labels · 4 marked (4 workloads)",
        "now (mixed: 2)",
        "app=brandnew is no label: Ctrl-A",
        "Ctrl-A new label",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("bulk_labels_form_80x24", out);
}

#[test]
fn bulk_enforcement_picker_80x24() {
    let mut m = bulk_model();
    bulk_key(&mut m, KeyCodeAlias::Char('M'));
    bulk_key(&mut m, KeyCodeAlias::Char('j'));
    bulk_key(&mut m, KeyCodeAlias::Char('j'));
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Enforcement · 4 marked",
        "● full",
        "● keep each workload's level",
        "4 workloads: 1 selective → full · 1 visibility_only → full",
        "1 unmanaged (skipped)",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("bulk_enforcement_picker_80x24", out);
}

#[test]
fn bulk_confirm_with_typed_count_100x30() {
    let mut m = bulk_confirm_model();
    bulk_key(&mut m, KeyCodeAlias::Char('1'));
    let out = render_to_string(&m, 100, 30);
    for s in [
        "Send this change to 2 workloads?",
        "full on 2 workloads: type 2 to confirm: 1",
        "no traffic loaded: d asks the draft policy",
        "type 2, then y",
        "host-4.example",
        "selective → full",
        "PUT /orgs/5/workloads/bulk_update",
        "2 GETs, one per workload, past the cache",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("bulk_confirm_typed_count_100x30", out);
}

#[test]
fn bulk_report_80x24() {
    use illumio_tui::tui::msg::Msg;
    use pce::write::bulk::{BulkStep, ItemResult, ItemStatus, StepStatus};
    let mut m = bulk_model();
    let item = |n: &str, status, error: Option<&str>| ItemResult {
        href: format!("/orgs/5/workloads/{n}"),
        name: format!("{n}.example"),
        change: "visibility_only → full".into(),
        status,
        error: error.map(str::to_string),
    };
    let report = pce::write::BulkReport {
        target: "demo (pce.example, org 5)".into(),
        change: "enforcement full (visibility kept)".into(),
        refused: None,
        reads: 1,
        steps: vec![BulkStep {
            method: pce::WriteMethod::Put,
            path: "/orgs/5/workloads/bulk_update".into(),
            summary: "bulk update 2 workloads (1/1)".into(),
            status: StepStatus::Partial,
            href: None,
            http_status: Some(200),
            error: None,
            audit_failed: None,
        }],
        items: vec![
            item("host-1", ItemStatus::Updated, None),
            item(
                "host-2",
                ItemStatus::Failed,
                Some("validation_failure: bad mode"),
            ),
            item("host-4", ItemStatus::Changed, None),
            ItemResult {
                change: String::new(),
                ..item("host-3", ItemStatus::Skipped, Some("unmanaged"))
            },
        ],
        labels_changed: false,
        labels_created: false,
        reread_skipped: None,
        failure: Some(pce::write::bulk::BulkFailure::Partial),
    };
    let _ = illumio_tui::tui::update(&mut m, Msg::BulkDone(Box::new(report)));
    let out = render_to_string(&m, 80, 24);
    for s in [
        "sent to demo (pce.example, org 5)",
        "FAILED: validation_failure",
        "changed since planned, skipped",
        "skipped: unmanaged",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("bulk_report_80x24", out);
}

#[test]
fn bulk_overlays_survive_every_size() {
    let mut models = vec![bulk_model()];
    let mut form = bulk_model();
    bulk_key(&mut form, KeyCodeAlias::Char('L'));
    models.push(form);
    let mut picker = bulk_model();
    bulk_key(&mut picker, KeyCodeAlias::Char('M'));
    models.push(picker);
    let mut confirm = bulk_confirm_model();
    bulk_key(&mut confirm, KeyCodeAlias::Char('2'));
    models.push(confirm);
    let mut grouped = bulk_model();
    bulk_key(&mut grouped, KeyCodeAlias::Char('x'));
    models.push(grouped);
    for m in &models {
        for w in 20..=130 {
            for h in [6, 24, 40] {
                let _ = render_to_string(m, w, h);
            }
        }
    }
}

/// The fixture flows after a draft evaluation (each at its reported
/// decision; only the fqdn flow names a rule), the policy and workloads
/// loaded, the cursor on the flow to `dst`, then `E`.
fn explain_model(dst: &str) -> illumio_tui::tui::model::Model {
    use pce::models::DraftDecision;
    let mut m = with_policy(with_workloads(model_with_flows()));
    let d = m.traffic.data.as_mut().expect("flows");
    for f in &mut d.flows {
        f.draft_policy_decision = Some(DraftDecision::parse(f.policy_decision.as_str()));
        if f.dst.display_name() != "fqdn-1.example" {
            f.rules.clear();
        }
    }
    d.draft = Some(pce::traffic::DraftSummary::of(&d.flows));
    m.traffic.rebuild_views();
    m.traffic
        .flows
        .select_where(|r| r.flow.dst.display_name() == dst);
    bulk_key(&mut m, KeyCodeAlias::Char('E'));
    m
}

#[test]
fn explain_allowed_flow_80x24() {
    let m = explain_model("fqdn-1.example");
    let out = render_to_string(&m, 80, 24);
    for s in ["explain", "allowed by 1 rule (draft)", "Shop-Prod", "Esc"] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("explain_allowed_flow_80x24", out);
}

#[test]
fn explain_no_rule_100x30() {
    let m = explain_model("host-1.example");
    let out = render_to_string(&m, 100, 30);
    for s in ["no rule allows this flow", "w: write an allow rule"] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("explain_no_rule_100x30", out);
}

#[test]
fn explain_survives_every_size() {
    for m in [
        explain_model("fqdn-1.example"),
        explain_model("host-1.example"),
    ] {
        for w in 20..=130 {
            for h in [6, 24, 40] {
                let _ = render_to_string(&m, w, h);
            }
        }
    }
}

#[test]
fn explain_scrolls_to_the_selected_rule_and_keeps_its_keys_in_a_short_terminal() {
    let mut m = with_policy(model_with_flows());
    let d = m.traffic.data.as_mut().expect("flows");
    for f in &mut d.flows {
        f.draft_policy_decision = Some(pce::models::DraftDecision::Allowed);
        f.rules = (1..=8)
            .map(|i| format!("/orgs/5/sec_policy/draft/rule_sets/999/sec_rules/{i}"))
            .collect();
    }
    m.traffic.rebuild_views();
    bulk_key(&mut m, KeyCodeAlias::Char('E'));
    for _ in 0..7 {
        bulk_key(&mut m, KeyCodeAlias::Char('j'));
    }
    let out = render_to_string(&m, 80, 18);
    assert!(out.contains("sec_rules/8"), "the selected rule:\n{out}");
    assert!(
        !out.contains("sec_rules/1 "),
        "scrolled past the first:\n{out}"
    );
    assert!(out.contains("w rule · Esc"), "the key line:\n{out}");
}

/// The rich flows with their draft decisions, the workloads, the rulesets
/// and the pending list loaded; records in a fixed place (never written
/// here): readiness and the enforcement overlay cost nothing.
fn enforce_model() -> illumio_tui::tui::model::Model {
    let mut m = illumio_tui::tui::test_support::with_draft_loaded(with_policy(with_workloads(
        model_with_window("24h"),
    )));
    m.policy.pending = Some(pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
    ));
    m.records_dir = Some("/state/illumio-tui/enforcements".into());
    m
}

fn command(m: &mut illumio_tui::tui::model::Model, c: illumio_tui::tui::commands::Command) {
    let cmds = illumio_tui::tui::update(m, illumio_tui::tui::msg::Msg::Command(c));
    assert!(cmds.is_empty(), "no request: {cmds:?}");
}

#[test]
fn readiness_overlay_120x40_and_80x24() {
    use illumio_tui::tui::commands::Command;
    let mut m = enforce_model();
    command(&mut m, Command::Readiness);
    let wide = render_to_string(&m, 120, 40);
    for s in [
        "Readiness · last 24h · draft",
        "8 apps · 8 needs rules",
        "shop/prod",
        "needs rules",
        "enforcement",
        "Enter enforce",
    ] {
        assert!(wide.contains(s), "missing {s:?}:\n{wide}");
    }
    insta::assert_snapshot!("readiness_overlay_120x40", wide);
    let narrow = render_to_string(&m, 80, 24);
    assert!(narrow.contains("shop/prod"), "{narrow}");
    insta::assert_snapshot!("readiness_overlay_80x24", narrow);
    // Not evaluated: said, and `D` offered.
    let mut plain = with_policy(with_workloads(model_with_window("24h")));
    plain.policy.pending = Some(Vec::new());
    command(&mut plain, Command::Readiness);
    let out = render_to_string(&plain, 80, 24);
    assert!(out.contains("not evaluated"), "{out}");
    assert!(out.contains("D evaluates"), "{out}");
}

#[test]
fn enforce_overlay_120x40_and_80x24() {
    use illumio_tui::tui::commands::Command;
    let mut m = enforce_model();
    command(&mut m, Command::Enforce(Some("shop/prod".into())));
    let wide = render_to_string(&m, 120, 40);
    for s in [
        "Enforce · shop/prod · last 24h",
        "1 Readiness",
        "5 Watch",
        "3 gaps",
        "Coverage",
        "Warnings",
    ] {
        assert!(wide.contains(s), "missing {s:?}:\n{wide}");
    }
    insta::assert_snapshot!("enforce_overlay_120x40", wide);
    insta::assert_snapshot!("enforce_overlay_80x24", render_to_string(&m, 80, 24));
    // Step 2's gaps and step 4's gate at 80x24.
    gen_key(&mut m, '2');
    let rules = render_to_string(&m, 80, 24);
    assert!(rules.contains("ipl:Corp/-"), "{rules}");
    assert!(rules.contains("G generator"), "{rules}");
    insta::assert_snapshot!("enforce_overlay_rules_80x24", rules);
    gen_key(&mut m, '4');
    let gate = render_to_string(&m, 80, 24);
    assert!(gate.contains("type enforce"), "{gate}");
    insta::assert_snapshot!("enforce_overlay_enforce_80x24", gate);
}

#[test]
fn enforce_overlay_watch_with_a_record_120x40() {
    use illumio_tui::tui::commands::Command;
    use pce::enforcement::{ChangeRecord, ModeState, RecordWorkload, write_record};
    let dir = tempfile::tempdir().unwrap();
    let rec = ChangeRecord {
        app: "shop/prod".into(),
        profile: "demo".into(),
        pce: "pce.example".into(),
        org_id: 5,
        at: "2026-10-19T10:00:00Z".into(),
        to: Some(ModeState {
            mode: Some("full".into()),
            visibility: None,
        }),
        rollback_of: None,
        readiness: None,
        workloads: vec![RecordWorkload {
            href: "/orgs/5/workloads/aaaa-2".into(),
            hostname: "host-2.example".into(),
            before: ModeState {
                mode: Some("visibility_only".into()),
                visibility: Some("flow_summary".into()),
            },
            after: ModeState {
                mode: Some("full".into()),
                visibility: None,
            },
            result: "updated".into(),
        }],
    };
    write_record(dir.path(), &rec, "2026-10-19T10:00:00Z".parse().unwrap()).unwrap();
    let mut m = enforce_model();
    m.records_dir = Some(dir.path().into());
    command(&mut m, Command::Enforce(Some("shop/prod".into())));
    gen_key(&mut m, '5');
    let out = render_to_string(&m, 120, 40);
    for s in [
        "shop_prod-20261019T100000.000Z.json",
        "host-2.example · visibility_only · flow_summary → full · updated",
        "r blocked since the change",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("enforce_overlay_watch_120x40", out);
}

#[test]
fn ven_overlay_160x48_and_80x24_and_the_strip_narrow() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    // The cursor on the newest version.
    m.overlay = Overlay::Ven { selected: 5 };
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("VEN health"), "{out}");
    assert!(out.contains("3 VENs · 4 workloads"), "{out}");
    assert!(out.contains("newest 23.2.31-3113 · 1 behind"), "{out}");
    assert!(out.contains("▸ 23.2.31-3113"), "{out}");
    assert!(out.contains("policy_sync: policy out of date"), "{out}");
    insta::assert_snapshot!("ven_overlay_160x48", out);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("▸ 23.2.31-3113"), "scrolled to it: {out}");
    insta::assert_snapshot!("ven_overlay_80x24", out);
    // Narrow, the strip shortens its words.
    m.overlay = Overlay::None;
    m.workloads.truncated = true;
    m.workloads.rebuild(&m.limits.key_order());
    let out = render_to_string(&m, 60, 20);
    assert!(
        out.contains("VEN 3 · ok 1 · warn 1 · err 0 · off 1 · unm 1 · capped"),
        "{out}"
    );
    insta::assert_snapshot!("workloads_ven_strip_60x20", out);
}

#[test]
fn ven_overlay_scrolls_to_the_notes_after_the_last_bucket() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    // No VEN with a message, and enough versions to scroll at 80x24.
    m.workloads.raw.remove(1);
    let base = m.workloads.raw[0].clone();
    for i in 0..15 {
        let mut w = base.clone();
        w.href = format!("/orgs/5/workloads/v-{i}");
        let status = w.agent.as_mut().unwrap()["status"].as_object_mut().unwrap();
        status.insert("agent_version".into(), format!("23.1.{i}").into());
        m.workloads.raw.push(w);
    }
    m.workloads.rebuild(&m.limits.key_order());
    m.overlay = Overlay::Ven { selected: 99 };
    let out = render_to_string(&m, 80, 24);
    let messages = out.find("│messages").expect(&out);
    assert!(out[messages..].contains("│  none"), "{out}");
}

/// `W` on a label (#75): with everything loaded, and with the rulesets, the
/// pairing profiles and the workload probe still on their way.
#[test]
fn where_used_overlay_120x40_80x24_and_loading() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::Msg;
    use illumio_tui::tui::model::Screen;
    use illumio_tui::tui::update;
    let w = || Msg::Key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE));
    let mut m = with_pairing_profiles(with_workloads(with_policy(model_with_objects())));
    m.screen = Screen::Objects;
    m.objects
        .labels
        .select_where(|r| r.href == "/orgs/5/labels/4");
    update(&mut m, w());
    let out = render_to_string(&m, 120, 40);
    for s in [
        "where used: env=prod",
        "3 references",
        "Rulesets 1 · Rules 1 · Label groups 1 · Workloads 2",
        "not searched: virtual services",
        "Global-Infra · #1 · consumer · allow · pending create disabled",
        "All prod envs",
        "Enter open",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("where_used_overlay_120x40", out);
    insta::assert_snapshot!("where_used_overlay_80x24", render_to_string(&m, 80, 24));

    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    m.objects
        .labels
        .select_where(|r| r.href == "/orgs/5/labels/4");
    update(&mut m, w());
    let out = render_to_string(&m, 100, 30);
    assert!(
        out.contains("loading rulesets, pairing profiles, workload count…"),
        "{out}"
    );
    assert!(out.contains("3 reqs"), "{out}");
    insta::assert_snapshot!("where_used_overlay_loading_100x30", out);
}

/// `W` on a label a pairing profile gives (#78): the profile's row with
/// `locked`, after the rules, before the workloads.
#[test]
fn where_used_overlay_pairing_profile_120x40_80x24() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use illumio_tui::tui::Msg;
    use illumio_tui::tui::model::Screen;
    use illumio_tui::tui::update;
    let mut m = with_pairing_profiles(with_workloads(with_policy(model_with_objects())));
    m.screen = Screen::Objects;
    m.objects
        .labels
        .select_where(|r| r.href == "/orgs/5/labels/1");
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE)),
    );
    let out = render_to_string(&m, 120, 40);
    for s in [
        "where used: role=web",
        "Rules 2 · Pairing profiles 1 · Workloads 1",
        "pairing profile shop-prod · locked · disabled",
        "not searched: virtual services, enforcement boundaries, container workload profiles",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("where_used_overlay_pairing_120x40", out);
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("not searched: virtual services, boundaries, container profiles"),
        "{out}"
    );
    assert!(out.contains("shop-prod · locked · disabled"), "{out}");
    insta::assert_snapshot!("where_used_overlay_pairing_80x24", out);
}
