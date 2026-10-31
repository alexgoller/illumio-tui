use super::*;
use crate::tui::model::{Model, Screen, StatusKind};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::model_with_workloads;
use crate::tui::wl_groups::{GroupOrder, WlGroupRow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn code(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

/// Sends `msg` and asserts it asked for nothing: everything here is
/// local to the loaded list.
#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

fn on_workloads() -> Model {
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m
}

fn headers(m: &Model) -> Vec<(String, bool)> {
    m.workloads
        .sections
        .rows()
        .iter()
        .filter_map(|r| match r {
            WlGroupRow::Header { group, folded } => Some((group.title.clone(), *folded)),
            WlGroupRow::Member { .. } => None,
        })
        .collect()
}

fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        quiet(m, key(c));
    }
}

#[test]
fn x_groups_by_the_traffic_keys_and_back_without_a_request() {
    let mut m = on_workloads();
    quiet(&mut m, key('x'));
    assert!(m.workloads.grouped);
    assert_eq!(m.workloads.group_keys, ["app", "env"]);
    assert_eq!(
        headers(&m),
        [
            ("shop | prod".to_string(), false),
            ("(no app) | (no env)".to_string(), false)
        ]
    );
    assert_eq!(m.workloads.sections.rows().len(), 6);
    // The cursor stays on its workload: host-1 is the first member.
    assert_eq!(
        m.workloads.selected_row().unwrap().workload.href,
        "/orgs/5/workloads/aaaa-1"
    );
    quiet(&mut m, key('x'));
    assert!(!m.workloads.grouped);
    assert!(
        m.workloads.sections.rows().is_empty(),
        "nothing kept while flat"
    );
    assert_eq!(
        m.workloads.selected_row().unwrap().workload.href,
        "/orgs/5/workloads/aaaa-1"
    );
}

#[test]
fn g_keys_regroup_the_workloads() {
    let mut m = on_workloads();
    quiet(&mut m, key('x'));
    apply_group_keys(&mut m, vec!["role".into()], false);
    assert_eq!(m.workloads.group_keys, ["role"]);
    let titles: Vec<String> = headers(&m).into_iter().map(|(t, _)| t).collect();
    assert_eq!(titles, ["(no role)", "db", "web"]);
}

#[test]
fn space_folds_z_folds_all_and_s_orders_the_groups() {
    let mut m = on_workloads();
    quiet(&mut m, key('x'));
    quiet(&mut m, key('j')); // host-1 under shop | prod
    // On a workload Space marks it (#47); on the header it folds.
    quiet(&mut m, key(' '));
    assert_eq!(headers(&m)[0], ("shop | prod".to_string(), false));
    assert_eq!(m.workloads.marked.len(), 1);
    m.workloads.sections.home(); // the shop | prod header
    quiet(&mut m, key(' '));
    assert_eq!(headers(&m)[0], ("shop | prod".to_string(), true));
    assert!(m.workloads.selected_group().is_some(), "on its header");
    assert_eq!(m.workloads.sections.rows().len(), 4);
    quiet(&mut m, key(' '));
    assert_eq!(m.workloads.sections.rows().len(), 6);
    quiet(&mut m, key('z'));
    assert!(headers(&m).iter().all(|(_, f)| *f));
    assert_eq!(m.workloads.sections.rows().len(), 2);
    quiet(&mut m, key('z'));
    assert!(headers(&m).iter().all(|(_, f)| !*f));
    // `s` orders the groups, not the table; the cursor keeps its header.
    let sort = m.workloads.table.sort();
    quiet(&mut m, key('s'));
    assert_eq!(m.workloads.group_order, GroupOrder::Name);
    assert_eq!(m.workloads.table.sort(), sort);
    assert_eq!(m.status.text, "groups by name A→Z");
    quiet(&mut m, key('s'));
    assert_eq!(m.workloads.group_order, GroupOrder::Unhealthy);
    assert_eq!(
        m.workloads.selected_group().unwrap().title,
        "shop | prod",
        "the cursor follows its header"
    );
    // Flat, `s` sorts the table again.
    quiet(&mut m, key('x'));
    quiet(&mut m, key('s'));
    assert_eq!(m.workloads.table.sort(), Some((0, false)));
}

#[test]
fn enter_on_a_header_narrows_and_on_a_workload_opens_the_detail() {
    let mut m = on_workloads();
    quiet(&mut m, key('x'));
    quiet(&mut m, code(KeyCode::End)); // host-4, the last member
    quiet(&mut m, code(KeyCode::Enter));
    assert!(m.workloads.show_detail);
    quiet(&mut m, code(KeyCode::Enter));
    assert!(!m.workloads.show_detail);
    // The (no app) | (no env) header: missing labels narrow as key=-.
    quiet(&mut m, key('k'));
    quiet(&mut m, key('k'));
    assert_eq!(
        m.workloads.selected_group().unwrap().title,
        "(no app) | (no env)"
    );
    quiet(&mut m, code(KeyCode::Enter));
    assert_eq!(m.workloads.labels.to_string(), "app=- env=-");
    assert_eq!(m.workloads.table.len(), 2);
    assert_eq!(headers(&m).len(), 1);
    assert!(
        m.workloads
            .filter_line()
            .unwrap()
            .starts_with("labels app=- env=- · 2 of 4")
    );
    // Another group's header replaces those keys, other terms stay.
    quiet(&mut m, key('c'));
    assert!(m.workloads.labels.is_empty(), "c clears the label filter");
    apply_workload_labels(&mut m, "!role=db").unwrap();
    // (no app) has two workloads now, shop one: shop's header is second.
    quiet(&mut m, code(KeyCode::End));
    quiet(&mut m, key('k'));
    quiet(&mut m, code(KeyCode::Enter));
    assert_eq!(m.workloads.labels.to_string(), "!role=db app=shop env=prod");
    assert_eq!(m.workloads.table.len(), 1);
}

#[test]
fn search_filters_the_sections_and_filters_keep_working_grouped() {
    let mut m = on_workloads();
    quiet(&mut m, key('x'));
    quiet(&mut m, key('/'));
    type_text(&mut m, "host-4");
    assert!(m.workloads.table.is_searching());
    assert_eq!(headers(&m).len(), 1);
    assert_eq!(headers(&m)[0].0, "(no app) | (no env)");
    quiet(&mut m, code(KeyCode::Esc));
    assert_eq!(headers(&m).len(), 2);
    // `m` (managed only), then `h` keep their sections current.
    quiet(&mut m, key('m'));
    let members: usize = m.workloads.groups.iter().map(|g| g.count()).sum();
    assert_eq!(members, 3);
    quiet(&mut m, key('c'));
    assert_eq!(m.workloads.sections.rows().len(), 6);
}

#[test]
fn f_edits_the_label_filter_locally() {
    let mut m = on_workloads();
    quiet(&mut m, key('f'));
    assert!(matches!(m.overlay, Overlay::LabelFilter(_)));
    type_text(&mut m, "app=shop,crm !role=db");
    quiet(&mut m, code(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.workloads.labels.to_string(), "app=shop,crm !role=db");
    assert_eq!(m.workloads.table.len(), 1, "host-1: shop, role web");
    assert_eq!(m.status.kind, StatusKind::Warn, "crm is no label");
    assert_eq!(m.workloads.label_warnings, ["unknown value app=crm"]);
    // Malformed: the error shows in the editor and the old filter holds.
    quiet(&mut m, key('f'));
    type_text(&mut m, " env=");
    quiet(&mut m, code(KeyCode::Enter));
    match &m.overlay {
        Overlay::LabelFilter(e) => assert!(e.error.as_deref().unwrap().contains("env=-")),
        o => panic!("{o:?}"),
    }
    assert_eq!(m.workloads.labels.to_string(), "app=shop,crm !role=db");
    quiet(&mut m, code(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    // An empty filter clears it.
    quiet(&mut m, key('f'));
    for _ in 0.."app=shop,crm !role=db".len() {
        quiet(&mut m, code(KeyCode::Backspace));
    }
    quiet(&mut m, code(KeyCode::Enter));
    assert!(m.workloads.labels.is_empty());
    assert_eq!(m.workloads.table.len(), 4);
}

#[test]
fn ctrl_n_completes_keys_then_values_of_the_label_filter() {
    let mut m = on_workloads();
    quiet(&mut m, key('f'));
    type_text(&mut m, "ap");
    let ctrl_n = || Msg::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    let value = |m: &Model| match &m.overlay {
        Overlay::LabelFilter(e) => e.value.clone(),
        o => panic!("{o:?}"),
    };
    quiet(&mut m, ctrl_n());
    assert_eq!(value(&m), "app=");
    quiet(&mut m, ctrl_n());
    assert_eq!(value(&m), "app=shop", "dives into the key's values");
    quiet(&mut m, key(','));
    quiet(&mut m, ctrl_n());
    assert_eq!(
        value(&m),
        "app=shop,-",
        "the listed value is left out; - last"
    );
    type_text(&mut m, " !ro");
    quiet(&mut m, ctrl_n());
    assert_eq!(value(&m), "app=shop,- !role=");
}

#[test]
fn colon_filter_sets_the_label_filter_on_workloads_only() {
    use crate::tui::commands::Command;
    let mut m = on_workloads();
    quiet(&mut m, Msg::Command(Command::Filter("env=prod".into())));
    assert_eq!(m.workloads.labels.to_string(), "env=prod");
    assert_eq!(m.workloads.table.len(), 2);
    assert_eq!(m.traffic.filter.src, "", "the traffic filter is untouched");
    quiet(&mut m, Msg::Command(Command::Filter("app=".into())));
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(m.workloads.labels.to_string(), "env=prod");
    // Elsewhere `:filter` still edits the traffic source.
    m.screen = Screen::Traffic;
    update(&mut m, Msg::Command(Command::Filter("app=shop".into())));
    assert_eq!(m.traffic.filter.src, "app=shop");
}

#[test]
fn x_on_workloads_no_longer_deletes() {
    let mut m = writes_on_workloads();
    quiet(&mut m, key('j'));
    quiet(&mut m, key('j')); // unmanaged-nas
    quiet(&mut m, key('x'));
    assert!(m.workloads.grouped);
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn shift_x_deletes_the_grouped_workload_and_refuses_a_header() {
    use crate::tui::edit::WriteIntent;
    let mut m = writes_on_workloads();
    quiet(&mut m, key('x'));
    // shop | prod, host-1, host-2, (no app) | (no env), unmanaged-nas, host-4.
    quiet(&mut m, code(KeyCode::End));
    quiet(&mut m, key('k'));
    assert_eq!(
        m.workloads.selected_row().unwrap().workload.href,
        "/orgs/5/workloads/aaaa-3"
    );
    assert_eq!(
        update(&mut m, key('X')),
        vec![Cmd::PlanWrite(WriteIntent::DeleteWorkload {
            href: "/orgs/5/workloads/aaaa-3".into(),
            name: "unmanaged-nas".into(),
        })],
        "the same plan, then the same confirm, as flat"
    );
    let mut m = writes_on_workloads();
    quiet(&mut m, key('x'));
    quiet(&mut m, code(KeyCode::Home));
    assert!(m.workloads.selected_group().is_some());
    quiet(&mut m, key('X'));
    assert_eq!(m.status.text, "no row selected");
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn label_warnings_follow_a_reload() {
    let mut m = on_workloads();
    apply_workload_labels(&mut m, "app=crm").unwrap();
    assert_eq!(m.workloads.label_warnings, ["unknown value app=crm"]);
    let mut raw = m.workloads.raw.clone();
    raw[0].labels[1].value = Some("crm".into());
    quiet(&mut m, Msg::WorkloadsLoaded(Ok((raw, 0, false))));
    assert!(m.workloads.label_warnings.is_empty());
    assert_eq!(m.workloads.table.len(), 1);
}

/// #90: the label filter ignores case, shows what was typed, and refuses a
/// name two labels share without case (the editor shows the error).
#[test]
fn the_label_filter_ignores_case_and_refuses_an_ambiguous_name() {
    let mut m = on_workloads();
    apply_workload_labels(&mut m, "app=shop").unwrap();
    let shop = m.workloads.table.len();
    assert!(shop > 0);
    apply_workload_labels(&mut m, "APP=Shop").unwrap();
    assert_eq!(m.workloads.table.len(), shop);
    assert!(
        m.workloads.label_warnings.is_empty(),
        "{:?}",
        m.workloads.label_warnings
    );
    assert_eq!(m.workloads.labels.to_string(), "APP=Shop");
    apply_workload_labels(&mut m, "!APP=SHOP").unwrap();
    assert_eq!(m.workloads.table.len(), m.workloads.raw.len() - shop);
    // Two labels of one key that differ in case.
    let label = |n: u32, v: &str| -> pce::models::Label {
        serde_json::from_value(serde_json::json!({
            "href": format!("/orgs/5/labels/{n}"), "key": "app", "value": v
        }))
        .unwrap()
    };
    let idx = pce::NameIndex::new(&[label(1, "Web"), label(2, "web")], &[], &[], &[]);
    if let crate::tui::model::Connection::Connected { index, .. } = &mut m.connection {
        *index = Some(std::sync::Arc::new(idx));
    }
    apply_workload_labels(&mut m, "APP=SHOP").unwrap();
    let before = m.workloads.labels.to_string();
    assert_eq!(
        apply_workload_labels(&mut m, "app=WEB"),
        Err("app=WEB matches app=Web and app=web; write one exactly".to_string())
    );
    assert_eq!(
        m.workloads.labels.to_string(),
        before,
        "the old filter holds"
    );
}

fn writes_on_workloads() -> Model {
    let mut m = on_workloads();
    m.writes = crate::tui::model::WritesState::on();
    m
}
