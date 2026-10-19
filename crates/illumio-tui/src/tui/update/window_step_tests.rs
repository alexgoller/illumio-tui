use super::*;
use crate::tui::model::{Model, Overlay, PendingAction, Range};
use crate::tui::msg::QueryOutcome;
use crate::tui::test_support::{FIXTURE_END, model_with_flows, model_with_window};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::{Span, Timestamp};
use pce::traffic::ResolvedFlow;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

fn end() -> Timestamp {
    FIXTURE_END.parse().unwrap()
}

fn ago(hours: i64) -> Timestamp {
    end().checked_sub(Span::new().hours(hours)).unwrap()
}

fn with_window(m: &Model, w: &str) -> TrafficFilter {
    let mut f = m.traffic.filter.clone();
    f.window = w.into();
    f
}

fn query_cmd(m: &Model, w: &str) -> Cmd {
    Cmd::RunQuery {
        seq: m.traffic.seq,
        filter: with_window(m, w),
        mode: CacheMode::Use,
        cancel: CancellationToken::new(),
    }
}

/// The answer to the query in flight for the window `hours` back.
fn query_done(m: &Model, flows: Vec<ResolvedFlow>, hours: i64) -> Msg {
    Msg::QueryDone {
        seq: m.traffic.seq,
        result: Ok(QueryOutcome {
            flows,
            truncated: false,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: Some((ago(hours), end())),
            hrefs: Vec::new(),
        }),
    }
}

fn range(m: &Model) -> Range {
    m.traffic.data.as_ref().unwrap().range.unwrap()
}

#[test]
fn app_to_app_state_survives_plus_and_minus() {
    use crate::tui::model::By;
    let mut m = model_with_window("1d");
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('x'));
    quiet(&mut m, key('x'));
    quiet(&mut m, key('d'));
    let st = m.traffic.apps_state.clone();
    assert_eq!(st.by, By::Out);
    let cmds = update(&mut m, key('+'));
    assert_eq!(cmds, vec![query_cmd(&m, "2d")]);
    let flows = m.traffic.loaded_flows().to_vec();
    let done = query_done(&m, flows, 48);
    quiet(&mut m, done);
    assert_eq!(m.traffic.apps_state, st);
    update(&mut m, key('-'));
    assert_eq!(m.traffic.apps_state, st);
}

/// Explorer returns the records that overlap the window, with lifetime
/// counters: an older slice cannot be added to the loaded rows, so `+`
/// runs the wider window as one query whose answer replaces them.
#[test]
fn plus_reruns_the_whole_wider_window_and_the_answer_replaces_the_rows() {
    let mut m = model_with_window("1d");
    let cmds = update(&mut m, key('+'));
    assert_eq!(cmds, vec![query_cmd(&m, "2d")]);
    assert_eq!(m.traffic.filter.window, "2d");
    quiet(&mut m, key('+')); // running: no second request
    let few = m.traffic.loaded_flows()[..2].to_vec();
    let done = query_done(&m, few, 48);
    quiet(&mut m, done);
    assert_eq!(m.traffic.loaded_flows().len(), 2, "replaced, not merged");
    assert_eq!(
        range(&m),
        Range {
            start: ago(48),
            end: end()
        }
    );
    let out = crate::tui::test_support::render_to_string(&m, 80, 24);
    assert!(out.contains("window 2d · cap 1000"), "{out}");
    assert!(!out.contains("slice"), "{out}");
}

#[test]
fn minus_during_a_cap_query_puts_the_loaded_filter_back() {
    let mut m = model_with_window("1d");
    m.limits.traffic_max_results = 20_000;
    assert_eq!(update(&mut m, key(']')).len(), 1);
    assert_eq!(m.traffic.filter.max_results, 2000);
    quiet(&mut m, key('-'));
    assert!(!m.traffic.is_running());
    assert_eq!(m.traffic.filter.max_results, 1000, "the unrun cap is gone");
    assert_eq!(m.traffic.filter.window, "12h");
    assert_eq!(m.traffic.data.as_ref().unwrap().filter, m.traffic.filter);
    // The same for an editor query with another source.
    let mut m = model_with_window("1d");
    let mut f = m.traffic.filter.clone();
    f.src = "app=shop".into();
    assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
    quiet(&mut m, key('-'));
    assert_eq!(m.traffic.filter.src, "");
    assert_eq!(m.traffic.filter.window, "12h");
}

#[test]
fn minus_sends_nothing_and_drops_exactly_the_older_flows() {
    let mut m = model_with_window("2d");
    let loaded = m.traffic.loaded_flows().len();
    if let Some(d) = m.traffic.data.as_mut() {
        for f in &mut d.flows[..3] {
            f.last_detected = Some("2026-10-09T01:00:00Z".into()); // before end - 1d
        }
        d.flows[3].last_detected = Some("2026-10-09T08:00:00Z".into()); // exactly the start
        d.flows[4].last_detected = None;
    }
    m.traffic.rebuild_views();
    quiet(&mut m, key('-'));
    assert_eq!(m.traffic.filter.window, "1d");
    let d = m.traffic.data.as_ref().unwrap();
    assert_eq!(d.filter.window, "1d");
    assert_eq!(d.flows.len(), loaded - 3);
    assert_eq!(m.traffic.flows.rows().len(), loaded - 3, "views rebuilt");
    assert!(
        d.flows.iter().any(|f| f.last_detected.is_none()),
        "undated kept"
    );
    assert_eq!(range(&m).start, ago(24));
    assert_eq!(m.status.text, "window 1d · 3 older flows dropped");
}

#[test]
fn a_late_answer_after_f_window_or_minus_is_ignored() {
    let mut m = model_with_window("1d");
    let loaded = m.traffic.loaded_flows().len();
    update(&mut m, key('+'));
    let late = query_done(&m, Vec::new(), 48);
    // `:window` (the editor's Enter takes the same path) starts a new query.
    let f = with_window(&m, "6h");
    assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
    quiet(&mut m, late);
    assert_eq!(m.traffic.loaded_flows().len(), loaded);
    assert_eq!(range(&m).start, ago(24));

    let mut m = model_with_window("1d");
    update(&mut m, key('+'));
    let late = query_done(&m, Vec::new(), 48);
    quiet(&mut m, key('-'));
    assert!(!m.traffic.is_running(), "the query was cancelled");
    assert_eq!(m.traffic.filter.window, "1d");
    quiet(&mut m, late);
    assert_eq!(m.traffic.data.as_ref().unwrap().filter.window, "1d");
    assert_eq!(range(&m).start, ago(24), "the 2d answer was dropped");
}

#[test]
fn plus_past_the_threshold_confirms_the_whole_window() {
    let mut m = model_with_window("1w");
    quiet(&mut m, key('+'));
    match &m.overlay {
        Overlay::Confirm {
            reason,
            action: PendingAction::RunQuery(f),
        } => {
            assert!(reason.contains("window 2w exceeds 7d"), "{reason}");
            assert_eq!(f.window, "2w");
        }
        o => panic!("{o:?}"),
    }
    quiet(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert_eq!(m.traffic.filter.window, "1w", "nothing changed");
    quiet(&mut m, key('+'));
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::None);
    quiet(&mut m, key('+'));
    let cmds = update(&mut m, key('y'));
    assert_eq!(cmds, vec![query_cmd(&m, "2w")]);
}

#[test]
fn plus_without_a_known_range_runs_one_full_query() {
    let mut m = model_with_flows(); // no bounds
    let cmds = update(&mut m, key('+'));
    assert_eq!(
        cmds,
        vec![Cmd::RunQuery {
            seq: m.traffic.seq,
            filter: with_window(&m, "2d"),
            mode: CacheMode::Use,
            cancel: CancellationToken::new(),
        }]
    );
    quiet(&mut m, key('+')); // running: no second request
    assert_eq!(m.status.text, "query already running");
}

#[test]
fn the_ladder_ends_send_nothing() {
    let mut m = model_with_window("90d");
    quiet(&mut m, key('+'));
    assert_eq!(m.status.text, "window already at 90d");
    let mut m = model_with_window("1h");
    quiet(&mut m, key('-'));
    assert_eq!(m.status.text, "window already at 1h");
    let mut m = model_with_window("1d");
    m.traffic.filter.max_results = 200_000;
    quiet(&mut m, key(']'));
    assert_eq!(m.status.text, "cap already at the maximum");
    m.traffic.filter.max_results = 500;
    quiet(&mut m, key('['));
    assert_eq!(m.status.text, "cap already at the minimum");
}

#[test]
fn brackets_rerun_the_whole_query_once_with_the_next_cap() {
    let mut m = model_with_window("1d");
    m.limits.traffic_max_results = 20_000;
    let mut f = m.traffic.filter.clone();
    f.max_results = 2000;
    let cmds = update(&mut m, key(']'));
    assert_eq!(
        cmds,
        vec![Cmd::RunQuery {
            seq: m.traffic.seq,
            filter: f,
            mode: CacheMode::Use,
            cancel: CancellationToken::new(),
        }]
    );
    assert_eq!(m.traffic.filter.max_results, 2000);
    quiet(&mut m, key(']')); // running
    let mut m = model_with_window("1d");
    let mut f = m.traffic.filter.clone();
    f.max_results = 500;
    let cmds = update(&mut m, key('['));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Cmd::RunQuery { filter, .. } if *filter == f));
}

#[test]
fn raising_the_cap_past_the_threshold_confirms() {
    let mut m = model_with_window("1d");
    m.limits.traffic_max_results = 20_000;
    m.traffic.filter.max_results = 10_000;
    quiet(&mut m, key(']'));
    assert!(
        matches!(&m.overlay, Overlay::Confirm { reason, .. } if reason.contains("cap 20000 exceeds 10000")),
        "{:?}",
        m.overlay
    );
    quiet(&mut m, key('n'));
    assert_eq!(m.traffic.filter.max_results, 10_000, "n keeps the cap");
    quiet(&mut m, key(']'));
    quiet(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    quiet(&mut m, key(']'));
    let cmds = update(&mut m, key('y'));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Cmd::RunQuery { filter, .. } if filter.max_results == 20_000));
}

/// The profile's cap is only where `]` starts: it steps on to 200000,
/// every rung past 10000 behind the `y`.
#[test]
fn the_cap_steps_past_the_profile_value_to_the_hard_maximum() {
    let mut m = model_with_window("1d");
    m.limits.traffic_max_results = 3000;
    m.traffic.filter.max_results = 3000;
    let mut seen = Vec::new();
    loop {
        let cmds = update(&mut m, key(']'));
        if let Overlay::Confirm { reason, .. } = &m.overlay {
            assert!(reason.contains("exceeds 10000"), "{reason}");
            assert!(cmds.is_empty());
            assert_eq!(update(&mut m, key('y')).len(), 1, "y runs it");
        } else if cmds.is_empty() {
            break;
        }
        let cap = m.traffic.filter.max_results;
        seen.push(cap);
        m.traffic.query = QueryState::Idle;
    }
    assert_eq!(
        seen,
        [5000, 10_000, 25_000, 50_000, 100_000, 200_000],
        "{}",
        m.status.text
    );
    assert_eq!(m.status.text, "cap already at the maximum");
    m.traffic.filter.max_results = 5000;
    m.traffic.query = QueryState::Idle;
    assert_eq!(update(&mut m, key('[')).len(), 1);
    assert_eq!(m.traffic.filter.max_results, 3000, "the profile's own rung");
}
