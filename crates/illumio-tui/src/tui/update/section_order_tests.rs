use super::*;
use crate::tui::model::{FlowsBy, Model, RowKind, SectionRow};
use crate::tui::test_support::model_with_rich_flows;
use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::traffic::{Decisions, Direction, SectionOrder, group_key};

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

/// Each section of `rows`: its app, its header's (flows, conns), and
/// the rows under it.
fn sections(rows: &[SectionRow]) -> Vec<(String, u64, u64, Vec<&SectionRow>)> {
    let mut out: Vec<(String, u64, u64, Vec<&SectionRow>)> = Vec::new();
    for r in rows {
        match &r.kind {
            RowKind::Header { totals, .. } => {
                let flows = totals.iter().map(|(_, t)| t.flows).sum();
                let conns = totals.iter().map(|(_, t)| t.connections).sum();
                out.push((r.app.join("/"), flows, conns, Vec::new()));
            }
            _ => out.last_mut().unwrap().3.push(r),
        }
    }
    out
}

fn by_name(names: &[String]) -> Vec<String> {
    let mut v = names.to_vec();
    v.sort_by_key(|n| n.to_lowercase());
    v
}

#[test]
fn s_cycles_app_to_app_sections_by_conns_name_flows_and_keeps_their_rows() {
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('v'));
    for x in 0..3 {
        quiet(&mut m, key('x'));
        let both = x == 2;
        let start = sections(m.traffic.apps.rows())
            .into_iter()
            .map(|(a, f, c, rows)| (a, f, c, rows.into_iter().cloned().collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        assert!(
            start.windows(2).all(|w| w[0].2 >= w[1].2),
            "by conns at first"
        );
        let mut seen = Vec::new();
        for _ in 0..3 {
            quiet(&mut m, key('s'));
            seen.push(m.traffic.apps_state.order);
            let now = sections(m.traffic.apps.rows());
            assert_eq!(now.len(), start.len());
            for (app, f, c, rows) in &now {
                let was = start.iter().find(|s| &s.0 == app).unwrap();
                assert_eq!((was.1, was.2), (*f, *c));
                assert!(
                    was.3.iter().eq(rows.iter().copied()),
                    "{app}: the rows keep their order (both: {both})"
                );
            }
            let names: Vec<String> = now.iter().map(|s| s.0.clone()).collect();
            match m.traffic.apps_state.order {
                SectionOrder::Name => assert_eq!(names, by_name(&names)),
                SectionOrder::Flows => assert!(now.windows(2).all(|w| w[0].1 >= w[1].1)),
                SectionOrder::Connections => {
                    let start_names: Vec<String> = start.iter().map(|s| s.0.clone()).collect();
                    assert_eq!(names, start_names, "back to the start");
                }
            }
            assert_eq!(m.traffic.apps.sort(), None);
            assert_eq!(m.status.text, m.traffic.apps_state.order.label());
        }
        assert_eq!(
            seen,
            vec![
                SectionOrder::Name,
                SectionOrder::Flows,
                SectionOrder::Connections
            ]
        );
    }
}

#[test]
fn s_keeps_the_cursor_on_its_section_and_pairs_keep_the_table_sort() {
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('v'));
    quiet(&mut m, key('s'));
    assert_eq!(
        m.traffic.groups.sort(),
        Some((0, false)),
        "pairs: a column sort"
    );
    assert_eq!(m.traffic.apps_state.order, SectionOrder::Connections);
    quiet(&mut m, key('x'));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j'));
    let app = m.traffic.apps.selected_item().unwrap().app.clone();
    quiet(&mut m, key('s'));
    let sel = m.traffic.apps.selected_item().unwrap();
    assert!(sel.is_header() && sel.app == app, "on its app's header");
}

#[test]
fn blocked_keeps_its_order() {
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Blocked;
    let before = m.traffic.blocked.rows().to_vec();
    quiet(&mut m, key('s'));
    assert_eq!(m.traffic.blocked.rows(), before.as_slice());
    assert_eq!(m.status.text, "sections keep their order (by connections)");
}

fn grouped(by: char) -> Model {
    let mut m = model_with_rich_flows();
    assert_eq!(m.traffic.view, TrafficView::Flows);
    for c in by.to_string().chars() {
        quiet(&mut m, key(c));
    }
    // The cursor on the first header (`x` keeps it on its flow).
    m.traffic.flows_grouped.home();
    m
}

#[test]
fn x_and_capital_x_cycle_flat_by_src_by_dst_without_requests() {
    let mut m = model_with_rich_flows();
    assert!(
        m.traffic.flows_grouped.is_empty(),
        "flat builds no sections"
    );
    let mut seen = Vec::new();
    for _ in 0..3 {
        quiet(&mut m, key('x'));
        seen.push(m.traffic.flows_state.by);
    }
    assert_eq!(seen, vec![FlowsBy::Src, FlowsBy::Dst, FlowsBy::Flat]);
    let mut seen = Vec::new();
    for _ in 0..3 {
        quiet(&mut m, key('X'));
        seen.push(m.traffic.flows_state.by);
    }
    assert_eq!(seen, vec![FlowsBy::Dst, FlowsBy::Src, FlowsBy::Flat]);
    assert!(m.traffic.flows_grouped.is_empty());
    assert_eq!(
        m.traffic.apps_state.by,
        crate::tui::model::By::Pairs,
        "App-to-app untouched"
    );
}

#[test]
fn by_src_and_by_dst_put_every_flow_under_its_app_once() {
    for (c, dir) in [('x', Direction::Out), ('X', Direction::In)] {
        let m = grouped(c);
        let t = &m.traffic;
        let keys = t.effective_keys();
        let secs = sections(t.flows_grouped.rows());
        assert!(secs.windows(2).all(|w| w[0].2 >= w[1].2), "by conns");
        let mut n = 0;
        for (app, flows, conns, rows) in &secs {
            assert_eq!(rows.len() as u64, *flows, "{app}");
            let sum: u64 = rows.iter().map(|r| r.flow().unwrap().num_connections).sum();
            assert_eq!(sum, *conns, "{app}");
            for r in rows {
                let f = &r.flow().unwrap().flow;
                let side = match dir {
                    Direction::Out => &f.src,
                    Direction::In => &f.dst,
                };
                assert_eq!(group_key(side, &keys).join("/"), *app);
            }
            n += rows.len();
        }
        assert_eq!(n, t.flows.rows().len(), "every flow once");
        // The headers count flows, not groups.
        let first = t.flows_grouped.rows()[0].header_line().unwrap();
        assert!(
            first.contains(" flows · ") && !first.contains("group"),
            "{first}"
        );
    }
}

#[test]
fn a_sections_flows_follow_the_flat_tables_sort() {
    let mut m = model_with_rich_flows();
    // conns, descending: column 6 of the flat table.
    m.traffic.flows.sort_by(6, true);
    quiet(&mut m, key('x'));
    for (app, _, _, rows) in sections(m.traffic.flows_grouped.rows()) {
        let conns: Vec<u64> = rows
            .iter()
            .map(|r| r.flow().unwrap().num_connections)
            .collect();
        assert!(conns.windows(2).all(|w| w[0] >= w[1]), "{app}: {conns:?}");
    }
    assert_eq!(
        m.traffic.flows_grouped.sort(),
        None,
        "no sort across sections"
    );
    quiet(&mut m, key('s'));
    assert_eq!(m.traffic.flows_state.order, SectionOrder::Name);
    assert_eq!(
        m.traffic.flows.sort(),
        Some((6, true)),
        "s orders the sections"
    );
    let names: Vec<String> = sections(m.traffic.flows_grouped.rows())
        .into_iter()
        .map(|s| s.0)
        .collect();
    assert_eq!(names, by_name(&names));
    quiet(&mut m, key('s'));
    let secs = sections(m.traffic.flows_grouped.rows());
    assert!(secs.windows(2).all(|w| w[0].1 >= w[1].1), "by flows");
}

#[test]
fn space_and_z_fold_flows_sections_and_the_folds_survive_a_rebuild() {
    let mut m = grouped('x');
    let all = m.traffic.flows_grouped.len();
    let headers = sections(m.traffic.flows_grouped.rows()).len();
    quiet(&mut m, key('j'));
    quiet(&mut m, key(' '));
    let sel = m.traffic.flows_grouped.selected_item().unwrap().clone();
    assert!(sel.is_header());
    assert!(m.traffic.flows_state.folded().contains(&sel.app));
    assert!(m.traffic.apps_state.folded.is_empty(), "its own folds");
    assert!(m.traffic.flows_grouped.len() < all);
    m.traffic.rebuild_views();
    assert!(m.traffic.flows_grouped.len() < all, "kept");
    quiet(&mut m, key(' '));
    assert_eq!(m.traffic.flows_grouped.len(), all);
    quiet(&mut m, key('z'));
    assert_eq!(m.traffic.flows_grouped.len(), headers, "headers only");
    quiet(&mut m, key('z'));
    assert_eq!(m.traffic.flows_grouped.len(), all);
}

#[test]
fn enter_on_a_flow_opens_the_detail_and_on_a_header_drills_to_the_app() {
    let mut m = grouped('x');
    quiet(&mut m, key('j'));
    assert!(m.traffic.selected_flow().is_some());
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(m.traffic.show_detail);
    assert_eq!(m.traffic.drill, None, "a flow row opens its detail");
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(!m.traffic.show_detail);
    quiet(&mut m, key('k'));
    let app = m.traffic.flows_grouped.selected_item().unwrap().app.clone();
    let (_, flows, _, _) = sections(m.traffic.flows_grouped.rows())[0].clone();
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        m.traffic.drill,
        Some(Drill::App {
            app: app.clone(),
            direction: Some(Direction::Out),
            decisions: Decisions::All,
        })
    );
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(
        m.traffic.flows.rows().len() as u64,
        flows,
        "the app's flows"
    );
    let secs = sections(m.traffic.flows_grouped.rows());
    assert_eq!(secs.len(), 1);
    assert_eq!(secs[0].0, app.join("/"));
    assert!(m.traffic.flows_grouped.selected_item().unwrap().is_header());
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.traffic.drill, None);
    assert!(sections(m.traffic.flows_grouped.rows()).len() > 1);
}

#[test]
fn f_w_and_m_read_the_grouped_row() {
    let mut m = grouped('x');
    m.writes = crate::tui::model::WritesState::on();
    let app = m.traffic.flows_grouped.selected_item().unwrap().app.clone();
    assert_eq!(m.traffic.row_apps(), vec![app.clone()], "a header: its app");
    quiet(&mut m, key('w'));
    assert_eq!(
        m.status.text,
        "select a flow row (a header holds a whole app)"
    );
    quiet(&mut m, key('j'));
    let f = m.traffic.selected_flow().unwrap().flow.clone();
    let keys = m.traffic.effective_keys();
    let mut want = vec![group_key(&f.src, &keys), group_key(&f.dst, &keys)];
    want.dedup();
    assert_eq!(m.traffic.row_apps(), want, "a flow: its two sides");
    quiet(&mut m, key('k'));
    quiet(&mut m, key('F'));
    assert_eq!(m.traffic.focus.as_ref().map(|f| f.app.clone()), Some(app));
}

#[test]
fn search_in_flows_sections_keeps_whole_sections_once_per_section() {
    let mut m = grouped('x');
    quiet(&mut m, key('/'));
    for c in "shop-db-p".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, kc(KeyCode::Enter));
    let t = &m.traffic;
    let shown: Vec<SectionRow> = t
        .flows_grouped
        .visible()
        .iter()
        .map(|&i| t.flows_grouped.rows()[i].clone())
        .collect();
    let secs = sections(&shown);
    assert!(!secs.is_empty());
    for (app, flows, _, rows) in &secs {
        assert_eq!(rows.len() as u64, *flows, "{app}: the whole section");
        assert!(
            rows.iter()
                .any(|r| r.flow().unwrap().dst.display_name() == "shop-db-p.example"),
            "{app}"
        );
    }
    assert!(secs.len() < sections(t.flows_grouped.rows()).len());
}

#[test]
fn enter_on_a_flow_keeps_the_cursor_on_screen_with_the_mark_line() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    m.size = (80, 24);
    quiet(&mut m, key('x'));
    for _ in 0..3 {
        quiet(&mut m, key('j'));
    }
    assert!(m.traffic.selected_flow().is_some());
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(m.traffic.show_detail);
    let page = traffic_page(&m);
    let t = &m.traffic.flows_grouped;
    assert!(
        t.selected_pos() >= t.offset() && t.selected_pos() < t.offset() + page,
        "row {} not in {}..{}",
        t.selected_pos(),
        t.offset(),
        t.offset() + page
    );
}

#[test]
fn the_flows_sections_are_built_only_while_the_flows_view_shows() {
    let mut m = grouped('x');
    let n = m.traffic.flows_grouped.len();
    quiet(&mut m, key('v'));
    m.traffic.rebuild_views();
    assert!(
        m.traffic.flows_grouped.is_empty(),
        "App-to-app on screen: none"
    );
    // App-to-app's own keys leave them alone.
    quiet(&mut m, key('x'));
    quiet(&mut m, key(' '));
    assert!(m.traffic.flows_grouped.is_empty());
    for _ in 0..4 {
        quiet(&mut m, key('v'));
    }
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(
        m.traffic.flows_grouped.len(),
        n,
        "built again on the way back"
    );
}

#[test]
fn a_flat_search_and_sort_carry_into_the_sections_and_the_meta() {
    let mut m = model_with_rich_flows();
    quiet(&mut m, key('/'));
    for c in "5432".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, kc(KeyCode::Enter));
    let kept = m.traffic.flows.len();
    assert!(kept > 0 && kept < 30);
    quiet(&mut m, key('x'));
    let secs = sections(m.traffic.flows_grouped.rows());
    assert_eq!(
        secs.iter().map(|s| s.3.len()).sum::<usize>(),
        kept,
        "the flat search applies"
    );
    m.traffic.flows.sort_by(6, true);
    m.traffic.rebuild_views();
    let out = crate::tui::test_support::render_to_string(&m, 120, 30);
    assert!(
        out.contains("by src app · sort conns ↓ · flows /5432"),
        "{out}"
    );
}

#[test]
fn x_keeps_the_cursor_on_its_flow_and_folds_are_per_grouping() {
    let mut m = model_with_rich_flows();
    for _ in 0..5 {
        quiet(&mut m, key('j'));
    }
    let flow = m.traffic.selected_flow().unwrap().flow.clone();
    quiet(&mut m, key('x'));
    assert!(std::sync::Arc::ptr_eq(
        &m.traffic.selected_flow().unwrap().flow,
        &flow
    ));
    quiet(&mut m, key('x'));
    assert!(std::sync::Arc::ptr_eq(
        &m.traffic.selected_flow().unwrap().flow,
        &flow
    ));
    // Fold by destination app; by source app has its own folds.
    quiet(&mut m, key(' '));
    let dst_folds = m.traffic.flows_state.folded_dst.clone();
    assert_eq!(dst_folds.len(), 1);
    quiet(&mut m, key('X'));
    assert_eq!(m.traffic.flows_state.by, FlowsBy::Src);
    assert!(m.traffic.flows_state.folded().is_empty());
    assert!(
        m.traffic
            .flows_grouped
            .rows()
            .iter()
            .all(|r| !matches!(r.kind, RowKind::Header { folded: true, .. }))
    );
    quiet(&mut m, key('x'));
    assert_eq!(m.traffic.flows_state.folded(), &dst_folds, "kept");
    // Back to flat: the cursor still on a flow of the table.
    quiet(&mut m, key('x'));
    assert_eq!(m.traffic.flows_state.by, FlowsBy::Flat);
    assert!(m.traffic.selected_flow().is_some());
}

#[test]
fn f_on_a_flow_by_dst_app_focuses_the_sections_app_first() {
    let mut m = grouped('X');
    quiet(&mut m, key('j'));
    let r = m.traffic.flows_grouped.selected_item().unwrap().clone();
    let f = &r.flow().unwrap().flow;
    let keys = m.traffic.effective_keys();
    assert_eq!(group_key(&f.dst, &keys), r.app);
    assert_eq!(m.traffic.row_apps()[0], r.app);
    quiet(&mut m, key('F'));
    assert_eq!(m.traffic.focus.as_ref().map(|x| x.app.clone()), Some(r.app));
}

#[test]
fn enter_on_a_header_under_a_drill_narrows_and_never_widens() {
    // A drill to one destination app, grouped by source app: a
    // header narrows to the pair.
    let mut m = model_with_rich_flows();
    let crm = vec!["crm".to_string(), "prod".to_string()];
    m.traffic.drill = Some(Drill::App {
        app: crm.clone(),
        direction: Some(Direction::In),
        decisions: Decisions::All,
    });
    m.traffic.rebuild_views();
    quiet(&mut m, key('x'));
    m.traffic.flows_grouped.home();
    let src = m.traffic.flows_grouped.selected_item().unwrap().app.clone();
    let n = sections(m.traffic.flows_grouped.rows())[0].1;
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        m.traffic.drill,
        Some(Drill::Pair(src.clone(), crm.clone(), Decisions::All))
    );
    assert_eq!(m.traffic.flows.rows().len() as u64, n);
    // One app on each side already: nothing changes.
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        m.traffic.drill,
        Some(Drill::Pair(src, crm.clone(), Decisions::All))
    );
    assert_eq!(m.status.text, "the drill holds only this app already");
    // Either side (`both`): no single drill says it; the header folds.
    m.traffic.drill = Some(Drill::App {
        app: crm.clone(),
        direction: None,
        decisions: Decisions::All,
    });
    m.traffic.rebuild_views();
    m.traffic.flows_grouped.home();
    let before = m.traffic.drill.clone();
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.traffic.drill, before, "never widened");
    assert_eq!(m.status.text, "drill active: Esc first");
    assert!(matches!(
        m.traffic.flows_grouped.selected_item().unwrap().kind,
        RowKind::Header { folded: true, .. }
    ));
}

#[test]
fn m_on_a_flows_header_marks_the_app_side() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = crate::tui::test_support::model_with_marks(dir.path(), "");
    quiet(&mut m, key('x'));
    m.traffic.flows_grouped.home();
    let app = m
        .traffic
        .flows_grouped
        .selected_item()
        .unwrap()
        .app
        .join("/");
    quiet(&mut m, key('m'));
    let crate::tui::model::Overlay::MarkForm(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.ctx.rows, crate::tui::mark_form::RowsOf::Flows);
    let value = |l: &str| {
        f.form
            .fields
            .iter()
            .find(|x| x.label == l)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value("Destination"), "any", "the app is the source");
    assert_eq!(
        value("Source"),
        "ipl:Corp",
        "{app}: the IP list by its name"
    );
    assert_eq!(value("Service"), "");
}

#[test]
fn a_section_whose_flows_are_all_squelched_disappears_and_the_counts_add_up() {
    // Every flow from hr/prod squelched; erp/prod's to crm/prod only.
    let marks = r#"
[[mark]]
id = "m-hrsq"
action = "squelch"
src = { labels = { app = "hr", env = "prod" } }
dst = "any"
comment = "hr out"

[[mark]]
id = "m-erpsq"
action = "squelch"
src = { labels = { app = "erp", env = "prod" } }
dst = { labels = { app = "crm" } }
comment = "erp to crm"

[[mark]]
id = "m-crit"
action = "critical"
src = "any"
dst = { labels = { app = "shop", env = "prod", role = "db" } }
decision = "blocked"
comment = "payments db"
"#;
    let (mut m, _dir) = marked_model(marks);
    let hr_out = m
        .traffic
        .loaded_flows()
        .iter()
        .filter(|f| f.src.label_value("app") == Some("hr"))
        .count();
    assert!(hr_out > 0);
    let flat = m.traffic.marked.as_ref().unwrap().flows;
    quiet(&mut m, key('x'));
    let t = &m.traffic;
    let secs = sections(t.flows_grouped.rows());
    assert!(
        secs.iter().all(|s| s.0 != "hr/prod"),
        "every flow from hr/prod is squelched: no section"
    );
    let shown: usize = secs.iter().map(|s| s.3.len()).sum();
    let mv = t.marked.as_ref().unwrap();
    assert_eq!(mv.flows, flat, "the flat view's counts");
    assert_eq!(shown, t.flows.rows().len());
    assert_eq!(shown as u64 + mv.flows.flows, 30, "kept + squelched = all");
    assert_eq!(
        t.marks_meta(TrafficView::Flows),
        mv.flows.meta(false, true),
        "the flat view's meta"
    );
    // erp/prod keeps its other flows: a partly squelched header with
    // its hidden records.
    let erp = t
        .flows_grouped
        .rows()
        .iter()
        .find(|r| r.is_header() && r.app.join("/") == "erp/prod")
        .unwrap();
    let RowKind::Header {
        marks: Some(rm), ..
    } = &erp.kind
    else {
        panic!("{erp:?}")
    };
    assert!(rm.hidden > 0, "{rm:?}");
    assert_eq!(
        crate::tui::traffic_marks::section_marker(erp),
        Some(pce::marks::Marker::PartlySquelched(rm.hidden))
    );
    // A header sums its flows' hits; the gutter follows them.
    assert!(t.flows_grouped.marker.is_some());
    let crit = t
        .flows_grouped
        .rows()
        .iter()
        .find(|r| {
            r.flow().is_some_and(|f| {
                f.mark
                    .is_some_and(|h| h.action == pce::marks::Action::Critical)
            })
        })
        .unwrap();
    let header = t
        .flows_grouped
        .rows()
        .iter()
        .find(|r| r.is_header() && r.app == crit.app)
        .unwrap();
    assert_eq!(
        crate::tui::traffic_marks::section_marker(header),
        Some(pce::marks::Marker::Critical)
    );
    // `M`: the squelched flows back, dimmed, hr's section with them.
    quiet(&mut m, key('M'));
    let secs = sections(m.traffic.flows_grouped.rows());
    let hr = m
        .traffic
        .flows_grouped
        .rows()
        .iter()
        .find(|r| r.is_header() && r.app.join("/") == "hr/prod")
        .unwrap();
    assert_eq!(
        crate::tui::traffic_marks::section_marker(hr),
        Some(pce::marks::Marker::Squelched)
    );
    assert_eq!(secs.iter().map(|s| s.3.len()).sum::<usize>(), 30);
}

#[test]
fn export_writes_the_grouped_flows_with_a_section_column() {
    let mut m = grouped('X');
    quiet(&mut m, key('z')); // folded sections still export every flow
    let (headers, rows) = crate::tui::export::current_table(&m).unwrap();
    let (flat_headers, _) = m.traffic.flows.export();
    assert_eq!(headers[0], "section");
    assert_eq!(&headers[1..], flat_headers.as_slice());
    assert_eq!(rows.len(), m.traffic.flows.rows().len());
    let secs = sections(&m.traffic.unfolded_section_rows().unwrap())
        .into_iter()
        .map(|s| s.0)
        .collect::<Vec<_>>();
    let mut seen: Vec<String> = rows.iter().map(|r| r[0].clone()).collect();
    seen.dedup();
    assert_eq!(seen, secs, "the sections in order, each whole");
    // The dst group column matches the section (by dst app).
    let dst = headers.iter().position(|h| *h == "dst group").unwrap();
    // (A non-workload's group cell is `-/-`; its section is its name.)
    assert!(
        rows.iter()
            .filter(|r| !r[dst].starts_with('-'))
            .all(|r| r[0] == r[dst]),
        "{rows:?}"
    );
}

#[test]
fn the_flows_state_survives_g_and_a_profile_switch_resets_it() {
    let mut m = grouped('x');
    quiet(&mut m, key('s'));
    quiet(&mut m, key('z'));
    let st = m.traffic.flows_state.clone();
    quiet(&mut m, key('g'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.traffic.flows_state, st, "g keeps it");
    assert!(!m.traffic.flows_grouped.is_empty());
    m.reset_data();
    assert_eq!(
        m.traffic.flows_state,
        crate::tui::model::FlowsState::default(),
        ":profile resets it"
    );
}
