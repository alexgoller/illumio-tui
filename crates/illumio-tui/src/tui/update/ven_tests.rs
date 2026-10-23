//! #72: the VEN health filter `v`, the `:ven` fleet overlay and its
//! buckets. Everything works on the loaded workloads: no request.
use super::*;
use crate::tui::commands;
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{model_fixture, model_with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::models::VenHealth;

/// Sends `code` and asserts it asked for nothing.
#[track_caller]
fn quiet(m: &mut Model, code: KeyCode) {
    let cmds = update(m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    assert_eq!(cmds, Vec::<Cmd>::new());
}

#[track_caller]
fn command(m: &mut Model, s: &str) {
    let cmds = update(m, Msg::Command(commands::parse(s).unwrap()));
    assert_eq!(cmds, Vec::<Cmd>::new());
}

fn on_workloads() -> Model {
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m
}

fn shown(m: &Model) -> Vec<String> {
    m.workloads
        .table
        .visible()
        .iter()
        .map(|&i| m.workloads.table.rows()[i].workload.display_name())
        .collect()
}

#[test]
fn v_cycles_warning_error_offline_all_and_c_clears_it_without_a_request() {
    let mut m = on_workloads();
    quiet(&mut m, KeyCode::Char('v'));
    assert_eq!(
        m.workloads.filters.health.single(),
        Some(VenHealth::Warning)
    );
    assert_eq!(shown(&m), ["host-4.example"]);
    let line = m.workloads.filter_line().unwrap();
    assert!(line.contains("health warning · 1 of 4"), "{line}");
    quiet(&mut m, KeyCode::Char('v'));
    assert_eq!(m.workloads.filters.health.single(), Some(VenHealth::Error));
    assert!(shown(&m).is_empty());
    quiet(&mut m, KeyCode::Char('v'));
    assert_eq!(shown(&m), ["host-2.example"]);
    quiet(&mut m, KeyCode::Char('v'));
    assert!(m.workloads.filters.health.is_empty(), "back to all");
    assert_eq!(shown(&m).len(), 4);
    quiet(&mut m, KeyCode::Char('v'));
    quiet(&mut m, KeyCode::Char('c'));
    assert!(m.workloads.filters.is_empty());
    assert_eq!(shown(&m).len(), 4);
}

#[test]
fn ven_opens_the_fleet_and_enter_filters_to_a_bucket_without_a_request() {
    let mut m = on_workloads();
    m.screen = Screen::Traffic;
    command(&mut m, "ven");
    assert_eq!(m.screen, Screen::Workloads, "the list it filters");
    assert_eq!(m.overlay, Overlay::Ven { selected: 0 });
    // Buckets: 5 health states, then the versions (newest first).
    for _ in 0..6 {
        quiet(&mut m, KeyCode::Char('j'));
    }
    quiet(&mut m, KeyCode::Enter);
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.workloads.filters.ven_version.as_deref(),
        Some("22.5.10-1")
    );
    assert_eq!(shown(&m), ["host-2.example"]);
    assert!(m.status.text.contains("ven 22.5.10-1"), "{:?}", m.status);
    assert!(m.status.text.contains("no request"), "{:?}", m.status);
    // A message bucket (the last), on top of the version.
    command(&mut m, "ven");
    quiet(&mut m, KeyCode::End);
    quiet(&mut m, KeyCode::Enter);
    assert_eq!(
        m.workloads.filters.message,
        Some(("warning", "policy_sync: policy out of date".to_string()))
    );
    assert_eq!(shown(&m), ["host-2.example"]);
    let line = m.workloads.filter_line().unwrap();
    assert!(
        line.contains("ven 22.5.10-1 · warning message policy_sync: policy out of date"),
        "{line}"
    );
    // A health bucket: the warning VEN has a different version, so none.
    command(&mut m, "ven");
    quiet(&mut m, KeyCode::Char('j'));
    quiet(&mut m, KeyCode::Enter);
    assert_eq!(
        m.workloads.filters.health.single(),
        Some(VenHealth::Warning)
    );
    assert!(shown(&m).is_empty());
    quiet(&mut m, KeyCode::Char('c'));
    assert!(m.workloads.filters.is_empty(), "c clears every VEN filter");
    // Esc closes and changes nothing.
    command(&mut m, "ven");
    quiet(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.workloads.filters.is_empty());
}

#[test]
fn the_enforcement_bucket_sets_the_e_filter() {
    let mut m = on_workloads();
    command(&mut m, "ven");
    let lines = crate::tui::ven_view::lines(&m.workloads.ven);
    let i = (0..20)
        .find(|&i| {
            crate::tui::ven_view::bucket_at(&lines, i)
                == Some(&crate::tui::ven_view::VenBucket::Enforcement(
                    "unmanaged".into(),
                ))
        })
        .unwrap();
    m.overlay = Overlay::Ven { selected: i };
    quiet(&mut m, KeyCode::Enter);
    assert_eq!(m.workloads.filters.enforcement, Some("unmanaged"));
    assert_eq!(shown(&m), ["unmanaged-nas"]);
}

#[test]
fn ven_without_a_loaded_list_says_so_and_never_loads_one() {
    let mut m = model_fixture();
    command(&mut m, "ven");
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        m.status.text.contains("no workloads loaded"),
        "{:?}",
        m.status
    );
}

#[test]
fn the_summary_is_of_every_loaded_workload_whatever_the_filters() {
    let mut m = on_workloads();
    quiet(&mut m, KeyCode::Char('o'));
    assert_eq!(shown(&m).len(), 1);
    let s = &m.workloads.ven;
    assert_eq!((s.workloads, s.vens), (4, 3));
    assert_eq!(s.count(VenHealth::Healthy), 1);
    assert_eq!(s.newest.as_deref(), Some("23.2.31-3113"));
}

/// Picks the `:ven` bucket `want` and presses Enter.
fn pick(m: &mut Model, want: &crate::tui::ven_view::VenBucket) {
    command(m, "ven");
    let lines = crate::tui::ven_view::lines(&m.workloads.ven);
    let i = (0..40)
        .find(|&i| crate::tui::ven_view::bucket_at(&lines, i) == Some(want))
        .unwrap();
    m.overlay = Overlay::Ven { selected: i };
    quiet(m, KeyCode::Enter);
}

#[test]
fn health_is_one_dimension_a_bucket_or_v_replaces_o_and_h_and_back() {
    use crate::tui::ven_view::VenBucket;
    // `o` on, then the warning bucket: the warning VEN, not an empty list.
    let mut m = on_workloads();
    quiet(&mut m, KeyCode::Char('o'));
    pick(&mut m, &VenBucket::Health(VenHealth::Warning));
    assert!(!m.workloads.filters.offline);
    assert_eq!(shown(&m), ["host-4.example"]);
    let line = m.workloads.filter_line().unwrap();
    assert!(line.starts_with("health warning · 1 of 4"), "{line}");
    // `h` on, then the healthy bucket.
    quiet(&mut m, KeyCode::Char('c'));
    quiet(&mut m, KeyCode::Char('h'));
    pick(&mut m, &VenBucket::Health(VenHealth::Healthy));
    assert!(!m.workloads.unhealthy_only);
    assert_eq!(shown(&m), ["host-1.example"]);
    // `v` replaces `o` and `h` the same way.
    quiet(&mut m, KeyCode::Char('c'));
    quiet(&mut m, KeyCode::Char('o'));
    quiet(&mut m, KeyCode::Char('h'));
    assert!(
        m.workloads.unhealthy_only && !m.workloads.filters.offline,
        "h replaced o"
    );
    quiet(&mut m, KeyCode::Char('v'));
    assert!(!m.workloads.unhealthy_only && !m.workloads.filters.offline);
    assert_eq!(
        m.workloads.filter_line().unwrap(),
        "health warning · 1 of 4"
    );
    // And back: `o` or `h` drop the health set.
    quiet(&mut m, KeyCode::Char('o'));
    assert!(m.workloads.filters.health.is_empty());
    assert_eq!(m.workloads.filter_line().unwrap(), "offline · 1 of 4");
    quiet(&mut m, KeyCode::Char('v'));
    quiet(&mut m, KeyCode::Char('h'));
    assert!(m.workloads.filters.health.is_empty() && !m.workloads.filters.offline);
    assert_eq!(m.workloads.filter_line().unwrap(), "unhealthy · 2 of 4");
    // A version bucket leaves `o` alone.
    quiet(&mut m, KeyCode::Char('c'));
    quiet(&mut m, KeyCode::Char('o'));
    pick(&mut m, &VenBucket::Version("22.5.10-1".into()));
    assert!(m.workloads.filters.offline);
    assert_eq!(shown(&m), ["host-2.example"]);
}

#[test]
fn a_message_bucket_keeps_its_severity() {
    let mut m = on_workloads();
    // The same text as an error on another VEN: a bucket of its own.
    let mut w = m.workloads.raw[1].clone();
    w.href = "/orgs/5/workloads/err".into();
    w.hostname = Some("err.example".into());
    let st = w.agent.as_mut().unwrap()["status"].as_object_mut().unwrap();
    st.insert(
        "agent_health_errors".into(),
        serde_json::json!({"errors": [{"type": "policy_sync", "message": "policy out of date"}], "warnings": []}),
    );
    st.insert("agent_health".into(), serde_json::json!([]));
    m.workloads.raw.push(w);
    m.workloads.rebuild(&m.limits.key_order());
    let text = "policy_sync: policy out of date".to_string();
    pick(
        &mut m,
        &crate::tui::ven_view::VenBucket::Message("error", text.clone()),
    );
    assert_eq!(shown(&m), ["err.example"]);
    quiet(&mut m, KeyCode::Char('c'));
    pick(
        &mut m,
        &crate::tui::ven_view::VenBucket::Message("warning", text),
    );
    assert_eq!(shown(&m), ["host-2.example"]);
}
