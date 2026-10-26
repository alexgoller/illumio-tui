//! Compare mode on the Traffic screen (#77): `c` and its prompt, the cost
//! guard, the `Δ` column and the header's counts per view, `n`, `C`, `r`
//! keeping the baseline, a filter or window change ending it, truncation
//! and `B`, the Treemap. Every request is the user's: no `Cmd` where none
//! should be.
use super::*;
use crate::tui::model::{Model, Overlay, PendingAction, ShowChanges, StatusKind};
use crate::tui::msg::BaselineOutcome;
use crate::tui::test_support::{
    FIXTURE_END, baseline_flows_rich, compare_with, fixture_flows_rich, model_with_window,
    render_to_string,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::Timestamp;
use pce::traffic::Change;
use std::time::Duration;

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn code(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
}

/// The rich fixture over the last day before `FIXTURE_END`.
fn model() -> Model {
    model_with_window("1d")
}

fn compared() -> Model {
    let mut m = model();
    compare_with(&mut m, baseline_flows_rich(), false);
    m
}

fn prompt(m: &Model) -> String {
    match &m.overlay {
        Overlay::Baseline { input } => input.clone(),
        o => panic!("no baseline prompt: {o:?}"),
    }
}

#[test]
fn c_opens_the_prompt_at_7d_and_ctrl_n_p_step_through_the_windows() {
    let mut m = crate::tui::test_support::model_fixture();
    quiet(&mut m, key('c'));
    assert_eq!(m.status.text, "nothing to compare: r runs the query first");
    assert_eq!(m.overlay, Overlay::None);

    let mut m = model();
    quiet(&mut m, key('c'));
    assert_eq!(prompt(&m), "7d");
    quiet(&mut m, ctrl('n'));
    assert_eq!(prompt(&m), "2w");
    quiet(&mut m, ctrl('p'));
    quiet(&mut m, ctrl('p'));
    assert_eq!(prompt(&m), "3d");
    quiet(&mut m, ctrl('u'));
    assert_eq!(prompt(&m), "");
    for c in "30d".chars() {
        quiet(&mut m, key(c));
    }
    assert_eq!(prompt(&m), "30d");
    quiet(&mut m, code(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.traffic.compare.is_none(), "Esc asks nothing");
}

#[test]
fn enter_runs_the_baseline_over_exactly_the_window_before_the_result() {
    let mut m = model();
    quiet(&mut m, key('c'));
    let cmds = update(&mut m, code(KeyCode::Enter));
    let end = ts(FIXTURE_END);
    let start = ts("2026-10-09T08:00:00Z");
    let mut filter = m.traffic.filter.clone();
    filter.window = "7d".into();
    assert_eq!(
        cmds,
        vec![Cmd::RunBaseline {
            seq: m.traffic.seq,
            filter,
            start: ts("2026-10-02T08:00:00Z"),
            end: start,
            cancel: tokio_util::sync::CancellationToken::new(),
        }]
    );
    assert_ne!(start, end);
    assert!(m.traffic.is_baseline_running());
    assert!(
        m.traffic.data.is_some(),
        "the result on screen stays while it runs"
    );
    assert_eq!(
        m.traffic.compare_meta().as_deref(),
        Some("vs 7d before · baseline running")
    );
    // A bad length asks nothing.
    let mut m = model();
    quiet(&mut m, key('c'));
    quiet(&mut m, ctrl('u'));
    quiet(&mut m, key('x'));
    quiet(&mut m, code(KeyCode::Enter));
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(m.traffic.compare.is_none());
}

#[test]
fn a_baseline_above_7d_asks_first_and_y_runs_it() {
    let mut m = model();
    quiet(&mut m, key('c'));
    quiet(&mut m, ctrl('u'));
    for c in "30d".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, code(KeyCode::Enter));
    let Overlay::Confirm { reason, action } = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert!(reason.contains("window 30d exceeds 7d"), "{reason}");
    assert!(
        reason.contains("Run the 30d baseline (1 query)"),
        "{reason}"
    );
    assert!(matches!(action, PendingAction::RunBaseline { .. }));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(&cmds[..], [Cmd::RunBaseline { start, filter, .. }]
            if *start == ts("2026-09-09T08:00:00Z") && filter.window == "30d"),
        "{cmds:?}"
    );
    // `n` declines: nothing sent.
    let mut m = model();
    quiet(&mut m, key('c'));
    quiet(&mut m, ctrl('u'));
    for c in "90d".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, code(KeyCode::Enter));
    quiet(&mut m, key('n'));
    assert!(!m.traffic.is_running());
}

fn delta(cols: &[crate::tui::table::Column<impl Sized>]) -> bool {
    cols.first().is_some_and(|c| c.title == "Δ")
}

#[test]
fn every_view_but_the_treemap_gains_the_delta_and_the_header_its_counts() {
    let mut m = compared();
    let t = &m.traffic;
    assert!(delta(&t.flows.columns));
    assert!(delta(&t.groups.columns));
    assert!(delta(&t.mesh.columns));
    assert!(delta(&t.apps.columns));
    assert!(delta(&t.blocked.columns));
    // Flows: the first flow is new, the 8443 one changed, 9999 gone, the
    // rest the same.
    let tag = |p: u16| {
        t.flows
            .rows()
            .iter()
            .find(|r| r.port == Some(p))
            .and_then(|r| r.change)
            .map(|c| c.change)
    };
    assert_eq!(tag(8443), Some(Change::Changed));
    assert_eq!(tag(9999), Some(Change::Gone));
    let c = t.compare.as_ref().unwrap().counts.flows;
    assert_eq!((c.new, c.changed, c.gone), (1, 1, 1));
    assert_eq!(
        t.compare_meta().as_deref(),
        Some("vs 7d before · 1 new · 1 changed · 1 gone")
    );
    assert_eq!(m.status.text, "vs 7d before · 1 new · 1 changed · 1 gone");
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("vs 7d before · 1 new · 1 changed · 1 gone"),
        "{out}"
    );
    // The gone flow comes last, below the screen's rows.
    assert!(out.contains("CHG") && out.contains("NEW"), "{out}");
    // Every other view has its own counts.
    for view in ["App-to-app", "Mesh", "Blocked"] {
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view.title(), view);
        let meta = m.traffic.compare_meta().unwrap();
        assert!(meta.starts_with("vs 7d before · "), "{view}: {meta}");
        let counts = m.traffic.view_counts().unwrap();
        assert!(
            counts.new + counts.changed + counts.gone >= 1,
            "{view}: {counts:?}"
        );
        let out = render_to_string(&m, 160, 48);
        assert!(out.contains(" Δ "), "{view}: {out}");
    }
    // The Treemap: no Δ.
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Treemap);
    assert_eq!(
        m.traffic.compare_meta().as_deref(),
        Some("vs 7d before · no Δ here")
    );
    let out = render_to_string(&m, 160, 48);
    assert!(!out.contains("GONE") && !out.contains("CHG"), "{out}");
}

#[test]
fn app_to_app_pairs_by_service_and_sections_compare_too() {
    let mut m = compared();
    quiet(&mut m, key('v'));
    let gone_pair = m
        .traffic
        .groups
        .rows()
        .iter()
        .find(|g| g.change.is_some_and(|c| c.change == Change::Gone));
    assert!(gone_pair.is_none(), "9999 shares its pair with 5432: same");
    quiet(&mut m, key('o'));
    let gone: Vec<String> = m
        .traffic
        .services
        .rows()
        .iter()
        .filter(|g| g.change.is_some_and(|c| c.change == Change::Gone))
        .filter_map(|g| g.service.clone())
        .collect();
    assert_eq!(gone, vec!["9999/tcp".to_string()]);
    quiet(&mut m, key('o'));
    quiet(&mut m, key('x'));
    let peers: Vec<(String, Change)> = m
        .traffic
        .apps
        .rows()
        .iter()
        .filter_map(|r| {
            r.peer()
                .map(|(_, p)| (p.service.clone(), p.change.unwrap().change))
        })
        .collect();
    assert!(
        peers.contains(&("9999/tcp".into(), Change::Gone)),
        "{peers:?}"
    );
}

#[test]
fn n_cycles_all_new_changed_gone_and_c_upper_leaves() {
    let mut m = compared();
    let n_rows = |m: &Model| m.traffic.flows.rows().len();
    let all = n_rows(&m);
    assert_eq!(all, fixture_flows_rich().len() + 1);
    for (show, rows) in [
        (ShowChanges::New, 1),
        (ShowChanges::Changed, 1),
        (ShowChanges::Gone, 1),
        (ShowChanges::All, all),
    ] {
        quiet(&mut m, key('n'));
        assert_eq!(m.traffic.compare.as_ref().unwrap().show, show);
        assert_eq!(n_rows(&m), rows, "{show:?}");
    }
    quiet(&mut m, key('n'));
    assert!(m.traffic.compare_meta().unwrap().ends_with("· new only"));
    quiet(&mut m, key('C'));
    assert!(m.traffic.compare.is_none());
    assert_eq!(m.status.text, "compare off");
    assert!(!delta(&m.traffic.flows.columns));
    assert_eq!(n_rows(&m), fixture_flows_rich().len());
    quiet(&mut m, key('n'));
    assert_eq!(m.status.text, "n: compare first (c)");
    quiet(&mut m, key('C'));
    assert_eq!(m.status.text, "not comparing: c compares with a baseline");
}

#[test]
fn r_reruns_the_recent_query_and_keeps_the_baseline() {
    let mut m = compared();
    let cmds = update(&mut m, key('r'));
    assert!(
        matches!(
            &cmds[..],
            [Cmd::RunQuery {
                mode: CacheMode::Bypass,
                ..
            }]
        ),
        "the recent query only: {cmds:?}"
    );
    let filter = m.traffic.filter.clone();
    let end = ts("2026-10-10T09:00:00Z");
    let seq = m.traffic.seq;
    let none = update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(crate::tui::msg::QueryOutcome {
                flows: fixture_flows_rich(),
                truncated: false,
                incomplete: false,
                elapsed: Duration::from_millis(10),
                hit: false,
                filter,
                bounds: Some((ts("2026-10-09T09:00:00Z"), end)),
                hrefs: Vec::new(),
            }),
        },
    );
    assert_eq!(none, Vec::<Cmd>::new(), "no baseline query");
    let cmp = m.traffic.compare.as_ref().unwrap();
    assert!(cmp.baseline.is_some(), "the cached baseline stays");
    assert!(delta(&m.traffic.flows.columns));
}

#[test]
fn a_filter_or_window_change_ends_compare_mode() {
    let mut m = compared();
    let mut f = m.traffic.filter.clone();
    f.src = "app=shop".into();
    let cmds = update(&mut m, Msg::ApplyFilter(f));
    assert!(matches!(&cmds[..], [Cmd::RunQuery { .. }]), "{cmds:?}");
    assert!(m.traffic.compare.is_none());
    assert_eq!(m.status.text, "filter changed: c compares again");

    // `-`: the window narrows locally, compare mode ends; no request.
    let mut m = compared();
    m.traffic.filter.window = "1d".into();
    quiet(&mut m, key('-'));
    assert!(m.traffic.compare.is_none());
    assert!(
        m.status
            .text
            .ends_with("· filter changed: c compares again"),
        "{}",
        m.status.text
    );
    assert!(!delta(&m.traffic.flows.columns));

    // `+`: another window, through ApplyFilter.
    let mut m = compared();
    let cmds = update(&mut m, key('+'));
    assert!(matches!(&cmds[..], [Cmd::RunQuery { .. }]), "{cmds:?}");
    assert!(m.traffic.compare.is_none());

    // A cap step keeps it: the same filter.
    let mut m = compared();
    let cmds = update(&mut m, key(']'));
    assert!(matches!(&cmds[..], [Cmd::RunQuery { .. }]), "{cmds:?}");
    assert!(m.traffic.compare.is_some());
}

#[test]
fn a_truncated_baseline_warns_and_b_backfills_it() {
    let mut m = model();
    compare_with(&mut m, baseline_flows_rich(), true);
    assert_eq!(
        m.traffic.compare_warning().as_deref(),
        Some("baseline truncated: new/gone over-report")
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert!(
        m.status
            .text
            .contains("baseline truncated: new/gone over-report")
    );
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("baseline truncated: new/gone over-report"),
        "{out}"
    );
    // `B`: the recent result is complete, so the baseline climbs.
    let cap = m.traffic.filter.max_results;
    let cmds = update(&mut m, key('B'));
    let next = pce::traffic::backfill_cap(cap).unwrap();
    assert!(
        matches!(&cmds[..], [Cmd::RunBaseline { filter, start, .. }]
            if filter.max_results == next && *start == ts("2026-10-02T08:00:00Z")),
        "{cmds:?}"
    );
    assert!(
        m.traffic.compare.as_ref().unwrap().baseline.is_some(),
        "the old baseline stays shown until the answer"
    );
    // Above 10000 it asks.
    let mut m = model();
    compare_with(&mut m, baseline_flows_rich(), true);
    if let Some(b) = m.traffic.compare.as_mut().and_then(|c| c.baseline.as_mut()) {
        b.filter.max_results = 10_000;
    }
    quiet(&mut m, key('B'));
    let Overlay::Confirm { reason, .. } = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert!(
        reason.contains("Re-run the baseline at cap 25000"),
        "{reason}"
    );
}

#[test]
fn a_late_or_cancelled_baseline_answer_is_dropped() {
    let mut m = model();
    quiet(&mut m, key('c'));
    let cmds = update(&mut m, code(KeyCode::Enter));
    let Some(Cmd::RunBaseline {
        seq,
        filter,
        start,
        end,
        ..
    }) = cmds.into_iter().next()
    else {
        panic!()
    };
    quiet(&mut m, key('C'));
    assert!(!m.traffic.is_running(), "C cancels the run");
    quiet(
        &mut m,
        Msg::BaselineDone {
            seq,
            result: Ok(BaselineOutcome {
                flows: baseline_flows_rich(),
                truncated: false,
                incomplete: false,
                hit: true,
                filter,
                start,
                end,
            }),
        },
    );
    assert!(m.traffic.compare.is_none());
    // A failed first baseline leaves compare mode off.
    let mut m = model();
    quiet(&mut m, key('c'));
    let _ = update(&mut m, code(KeyCode::Enter));
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::BaselineDone {
            seq,
            result: Err("boom".into()),
        },
    );
    assert!(m.traffic.compare.is_none());
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn squelched_flows_are_hidden_on_both_sides_and_never_gone() {
    let dir = tempfile::tempdir().unwrap();
    let marks = r#"
[[mark]]
id = "m-sq9999"
action = "squelch"
src = "any"
dst = "any"
service = "9999/tcp"
"#;
    let mut m = crate::tui::test_support::model_with_marks(dir.path(), marks);
    let end = ts(FIXTURE_END);
    m.traffic.filter.window = "1d".into();
    if let Some(d) = m.traffic.data.as_mut() {
        d.filter = m.traffic.filter.clone();
        d.range = Some(crate::tui::model::Range {
            start: ts("2026-10-09T08:00:00Z"),
            end,
        });
    }
    m.traffic.rebuild_views();
    compare_with(&mut m, baseline_flows_rich(), false);
    let c = m.traffic.compare.as_ref().unwrap().counts.flows;
    assert_eq!(c.gone, 0, "the squelched 9999 flow is not gone");
    // Shown with `M`, it is gone again (both sides alike).
    quiet(&mut m, key('M'));
    let c = m.traffic.compare.as_ref().unwrap().counts.flows;
    assert_eq!(c.gone, 1);
}

#[test]
fn compare_refuses_a_focused_result_and_a_focused_answer_ends_it() {
    let mut m = compared();
    let focus = pce::traffic::Focus::new(
        vec!["shop".into(), "prod".into()],
        vec!["app".into(), "env".into()],
    );
    m.traffic.focus = Some(focus.clone());
    let (seq, _) = m
        .traffic
        .start_focused_query(std::time::Instant::now(), focus.clone());
    let filter = m.traffic.filter.clone();
    quiet(
        &mut m,
        Msg::FocusedDone {
            seq,
            result: Ok(crate::tui::msg::FocusedOutcome {
                outcome: crate::tui::msg::QueryOutcome {
                    flows: fixture_flows_rich(),
                    truncated: false,
                    incomplete: false,
                    elapsed: Duration::from_millis(10),
                    hit: false,
                    filter,
                    bounds: Some((ts("2026-10-09T08:00:00Z"), ts(FIXTURE_END))),
                    hrefs: Vec::new(),
                },
                focus,
                queries: 2,
            }),
        },
    );
    assert!(m.traffic.compare.is_none());
    quiet(&mut m, key('c'));
    assert!(
        m.status
            .text
            .starts_with("compare works on the unfocused result")
    );
    assert_eq!(m.overlay, Overlay::None);
}
