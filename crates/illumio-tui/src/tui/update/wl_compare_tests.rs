//! #85: compare mode on Workloads against a snapshot (`b`, the picker,
//! `n`, `C`), the filters and grouping on both sides, VEN health
//! transitions, gone rows left alone; and `:readiness`'s trend read from
//! the files. Files only: no key here asks the PCE.
use super::*;
use crate::snapshot::SnapshotMeta;
use crate::tui::model::{Model, Overlay, Screen, StatusKind, wl_gone};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::model_with_workloads;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(m: &mut Model, code: KeyCode) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

/// Sends `code` and asserts it asked for nothing.
#[track_caller]
fn quiet(m: &mut Model, code: KeyCode) {
    assert_eq!(key(m, code), Vec::<Cmd>::new());
}

fn on_workloads() -> Model {
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m.snapshots_root = Some("/data/snapshots".into());
    m
}

use crate::tui::test_support::workloads_baseline as baseline;

fn meta(id: &str) -> SnapshotMeta {
    SnapshotMeta {
        id: id.into(),
        path: format!("/data/snapshots/default/{id}.json").into(),
        bytes: 10,
        version: Some(1),
        taken_at: None,
        label: None,
        sections: vec!["workloads"],
        problem: None,
    }
}

/// `b`, the picker on `latest`, `Enter`, the baseline loaded: compare on.
fn comparing() -> Model {
    let mut m = on_workloads();
    let cmds = key(&mut m, KeyCode::Char('b'));
    assert_eq!(
        cmds,
        [Cmd::ListBaselines {
            dir: std::path::PathBuf::from("/data/snapshots")
                .join(crate::names::dir_name(&m.profile.name))
        }]
    );
    update(
        &mut m,
        Msg::BaselinesListed(Ok(vec![
            meta("2026-10-01T000000Z"),
            meta("2026-10-10T075900Z"),
        ])),
    );
    let Overlay::SnapshotPicker { items, selected } = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert_eq!(items[0].id, "2026-10-10T075900Z", "newest first");
    assert_eq!(*selected, 0, "latest");
    let cmds = key(&mut m, KeyCode::Enter);
    assert_eq!(
        cmds,
        [Cmd::LoadWlBaseline {
            path: "/data/snapshots/default/2026-10-10T075900Z.json".into(),
            id: "2026-10-10T075900Z".into(),
        }]
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(update(&mut m, Msg::WlBaselineLoaded(Ok(baseline()))).is_empty());
    m
}

/// The rows shown: `(Δ, hostname or name, was→now)`.
fn shown(m: &Model) -> Vec<(String, String, String)> {
    let t = &m.workloads.table;
    t.visible()
        .iter()
        .map(|&i| {
            let r = &t.rows()[i];
            (
                r.change.as_ref().map_or("", |c| c.change.tag()).to_string(),
                r.workload.display_name(),
                crate::tui::model::wl_transition(r),
            )
        })
        .collect()
}

#[test]
fn b_picks_a_snapshot_and_every_row_gains_its_change() {
    let m = comparing();
    let w = &m.workloads;
    let c = w.compare.as_ref().unwrap();
    assert_eq!(
        (c.counts.new, c.counts.changed, c.counts.gone, c.counts.same),
        (1, 2, 2, 1)
    );
    assert_eq!(
        w.compare_meta().unwrap(),
        "vs snapshot 10-10 07:59 · 1 new · 2 changed · 2 gone"
    );
    assert_eq!(m.status.text, w.compare_meta().unwrap());
    let rows = shown(&m);
    assert!(rows.contains(&(
        "CHG".into(),
        "host-1.example".into(),
        "app: crm→shop · mode: visibility→full".into()
    )));
    assert!(rows.contains(&(
        "CHG".into(),
        "host-2.example".into(),
        "ven: active→stopped · health: healthy→offline".into()
    )));
    assert!(rows.contains(&("NEW".into(), "host-4.example".into(), String::new())));
    assert!(rows.contains(&("GONE".into(), "gone-1.example".into(), String::new())));
    // The columns: Δ and was→now first; the strip is the fleet loaded now.
    assert_eq!(w.table.columns[0].title, "Δ");
    assert_eq!(w.table.columns[1].title, "was→now");
    assert_eq!(w.ven.workloads, 4);
}

#[test]
fn the_picker_steps_older_and_newer_and_esc_leaves_compare_as_it_was() {
    let mut m = on_workloads();
    key(&mut m, KeyCode::Char('b'));
    update(
        &mut m,
        Msg::BaselinesListed(Ok(vec![meta("a"), meta("b"), meta("c")])),
    );
    quiet(&mut m, KeyCode::Left);
    quiet(&mut m, KeyCode::Left);
    quiet(&mut m, KeyCode::Left);
    assert!(matches!(
        m.overlay,
        Overlay::SnapshotPicker { selected: 2, .. }
    ));
    quiet(&mut m, KeyCode::Right);
    assert!(matches!(
        m.overlay,
        Overlay::SnapshotPicker { selected: 1, .. }
    ));
    quiet(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.workloads.compare.is_none());
    // None with workloads: said, no picker.
    update(&mut m, Msg::BaselinesListed(Ok(Vec::new())));
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        m.status.text.contains(":snapshot saves one"),
        "{}",
        m.status.text
    );
    // Another PCE's (the runtime refuses it): compare stays off.
    update(
        &mut m,
        Msg::WlBaselineLoaded(Err("snapshots of different PCEs are never compared".into())),
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(m.workloads.compare.is_none());
}

#[test]
fn b_needs_the_list_and_a_data_directory() {
    let mut m = on_workloads();
    m.workloads.loaded_at = None;
    quiet(&mut m, KeyCode::Char('b'));
    assert!(m.status.text.contains("r loads them"), "{}", m.status.text);
    let mut m = on_workloads();
    m.snapshots_root = None;
    quiet(&mut m, KeyCode::Char('b'));
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn n_cycles_the_changes_and_c_leaves_without_a_request() {
    let mut m = comparing();
    quiet(&mut m, KeyCode::Char('n'));
    assert_eq!(shown(&m).len(), 1);
    assert_eq!(shown(&m)[0].0, "NEW");
    assert!(m.workloads.compare_meta().unwrap().ends_with("· new only"));
    quiet(&mut m, KeyCode::Char('n'));
    assert!(shown(&m).iter().all(|r| r.0 == "CHG"));
    quiet(&mut m, KeyCode::Char('n'));
    assert!(shown(&m).iter().all(|r| r.0 == "GONE"));
    assert_eq!(shown(&m).len(), 2);
    // Counts stay those before `n`.
    assert!(
        m.workloads
            .compare_meta()
            .unwrap()
            .contains("1 new · 2 changed · 2 gone")
    );
    quiet(&mut m, KeyCode::Char('n'));
    assert_eq!(shown(&m).len(), 6);
    quiet(&mut m, KeyCode::Char('C'));
    assert!(m.workloads.compare.is_none());
    assert_eq!(m.status.text, "compare off");
    assert_eq!(shown(&m).len(), 4);
    assert_eq!(m.workloads.table.columns[0].title, "hostname");
    quiet(&mut m, KeyCode::Char('C'));
    assert!(m.status.text.contains("b compares"), "{}", m.status.text);
}

#[test]
fn the_filters_apply_to_both_sides_and_the_counts_follow_the_rows() {
    let mut m = comparing();
    crate::tui::update::apply_workload_labels(&mut m, "app=shop").unwrap();
    let rows = shown(&m);
    let names: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    // host-1 is in shop now (changed), gone-1 was (gone); gone-2 was in crm.
    assert!(names.contains(&"host-1.example"));
    assert!(names.contains(&"gone-1.example"));
    assert!(!names.contains(&"gone-2.example"));
    let c = m.workloads.compare.as_ref().unwrap().counts;
    assert_eq!(
        (c.new + c.changed + c.gone + c.same) as usize,
        rows.len(),
        "counts are the rows shown"
    );
}

#[test]
fn newly_offline_is_a_changed_row_under_the_offline_filter() {
    let mut m = comparing();
    // `v` three times: offline; `n` twice: changed.
    for _ in 0..3 {
        quiet(&mut m, KeyCode::Char('v'));
    }
    quiet(&mut m, KeyCode::Char('n'));
    quiet(&mut m, KeyCode::Char('n'));
    assert_eq!(
        shown(&m),
        [(
            "CHG".to_string(),
            "host-2.example".to_string(),
            "ven: active→stopped · health: healthy→offline".to_string()
        )]
    );
}

#[test]
fn gone_rows_are_not_marked_edited_or_queried() {
    let mut m = comparing();
    quiet(&mut m, KeyCode::Char('n'));
    quiet(&mut m, KeyCode::Char('n'));
    quiet(&mut m, KeyCode::Char('n'));
    assert!(wl_gone(m.workloads.selected_row().unwrap()));
    for c in [' ', 'e', 'X', 't', 'T', 'L', 'M'] {
        quiet(&mut m, KeyCode::Char(c));
        assert!(m.status.text.starts_with("gone since the snapshot"), "{c}");
    }
    assert!(m.workloads.marked.is_empty());
    // `V` marks every row shown but the gone ones.
    quiet(&mut m, KeyCode::Char('n'));
    quiet(&mut m, KeyCode::Char('V'));
    assert_eq!(m.workloads.marked.len(), 4);
}

#[test]
fn group_headers_count_the_changes() {
    let mut m = comparing();
    quiet(&mut m, KeyCode::Char('x'));
    let shop = m
        .workloads
        .groups
        .iter()
        .find(|g| g.title == "shop | prod")
        .unwrap();
    // host-1 (changed into shop), host-2 (changed), gone-1: two there
    // now, in the count and the health; the gone one apart.
    assert_eq!((shop.changes.changed, shop.changes.gone), (2, 1));
    assert_eq!(shop.count(), 2);
    assert_eq!(shop.health.iter().sum::<usize>(), 2);
    assert_eq!(crate::tui::wl_groups::changes_text(shop), "2 chg");
}

#[test]
fn a_reload_keeps_compare_mode_and_compares_the_new_list() {
    let mut m = comparing();
    let mut ws = m.workloads.raw.clone();
    ws.retain(|w| w.href != "/orgs/5/workloads/aaaa-4");
    let cmds = update(&mut m, Msg::WorkloadsLoaded(Ok((ws, 0, false))));
    assert!(cmds.is_empty(), "{cmds:?}");
    let c = m.workloads.compare.as_ref().expect("still comparing");
    assert_eq!(c.counts.new, 0);
}

#[test]
fn readiness_reads_the_trend_from_the_files_at_each_open_and_counts_no_request() {
    use crate::tui::test_support::{model_with_window, with_draft_loaded};
    let mut m = with_draft_loaded(model_with_window("24h"));
    m.snapshots_root = Some("/data/snapshots".into());
    // The loads readiness needs are requests (counted); the trend is a
    // file read (not counted).
    let cmds = update(
        &mut m,
        Msg::Command(crate::tui::commands::parse("readiness").unwrap()),
    );
    let trend: Vec<&Cmd> = cmds
        .iter()
        .filter(|c| matches!(c, Cmd::LoadTrend { .. }))
        .collect();
    assert_eq!(trend.len(), 1, "{cmds:?}");
    let reqs = cmds.len() - 1;
    assert!(
        m.status.text.contains(&format!("{reqs} req")),
        "{}",
        m.status.text
    );
    let t = crate::snapshot::baseline::Trend {
        snapshots: 2,
        ..Default::default()
    };
    update(&mut m, Msg::TrendLoaded(Ok(t)));
    assert_eq!(
        m.readiness_view
            .as_ref()
            .unwrap()
            .trend
            .as_ref()
            .unwrap()
            .snapshots,
        2
    );
    // Closed and opened again: the files are read again (a snapshot may
    // have been saved since), still no request.
    key(&mut m, KeyCode::Esc);
    let cmds = update(
        &mut m,
        Msg::Command(crate::tui::commands::parse("readiness").unwrap()),
    );
    assert_eq!(
        cmds,
        [Cmd::LoadTrend {
            dir: std::path::PathBuf::from("/data/snapshots")
                .join(crate::names::dir_name(&m.profile.name))
        }]
    );
}

#[test]
fn the_comparison_is_computed_once_per_list_and_filtered_after() {
    let mut m = comparing();
    // The baseline's workloads changed under it: the filters keep using
    // the comparison made for this list (it is not recomputed).
    m.workloads.compare.as_mut().unwrap().workloads.clear();
    quiet(&mut m, KeyCode::Char('v'));
    quiet(&mut m, KeyCode::Char('c'));
    assert_eq!(m.workloads.compare.as_ref().unwrap().counts.gone, 2);
    // A new list is compared again.
    let ws = m.workloads.raw.clone();
    update(&mut m, Msg::WorkloadsLoaded(Ok((ws, 0, false))));
    let c = m.workloads.compare.as_ref().unwrap();
    assert_eq!((c.counts.gone, c.counts.new), (0, 4));
}

#[test]
fn a_cut_list_judges_no_workload_gone() {
    let mut m = comparing();
    let ws = m.workloads.raw.clone();
    update(&mut m, Msg::WorkloadsLoaded(Ok((ws, 0, true))));
    assert!(shown(&m).iter().all(|r| r.0 != "GONE"), "{:?}", shown(&m));
    let meta = m.workloads.compare_meta().unwrap();
    assert!(
        meta.ends_with("1 new · 2 changed · gone not judged: list cut at 4"),
        "{meta}"
    );
}
