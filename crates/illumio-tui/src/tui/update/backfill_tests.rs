use super::*;
use crate::tui::model::{Model, Overlay, PendingAction, StatusKind};
use crate::tui::msg::QueryOutcome;
use crate::tui::test_support::{FIXTURE_END, fixture_flows_rich, model_fixture, render_to_string};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::{Span, Timestamp};
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

fn cap(m: &Model) -> u32 {
    m.traffic.filter.max_results
}

/// The answer to the query in flight (or to a `1d` query at `cap`
/// started here): the rich fixture, `truncated` or not.
fn answer(m: &mut Model, cap: u32, truncated: bool) -> Vec<Cmd> {
    if !m.traffic.is_running() {
        let f = TrafficFilter::defaults("1d", cap);
        assert_eq!(update(m, Msg::ApplyFilter(f)).len(), 1);
    }
    let filter = m.traffic.filter.clone();
    assert_eq!(filter.max_results, cap, "the query in flight");
    update(
        m,
        Msg::QueryDone {
            seq: m.traffic.seq,
            result: Ok(QueryOutcome {
                flows: fixture_flows_rich(),
                truncated,
                incomplete: false,
                elapsed: Duration::from_millis(900),
                hit: false,
                filter,
                bounds: Some((end().checked_sub(Span::new().hours(24)).unwrap(), end())),
                hrefs: Vec::new(),
            }),
        },
    )
}

fn run_at(m: &Model, cap: u32) -> Cmd {
    let mut filter = m.traffic.filter.clone();
    filter.max_results = cap;
    Cmd::RunQuery {
        seq: m.traffic.seq,
        filter,
        mode: CacheMode::Use,
        cancel: CancellationToken::new(),
    }
}

fn n() -> usize {
    fixture_flows_rich().len()
}

#[test]
fn nothing_to_backfill_without_a_result_or_on_a_complete_one() {
    let mut m = model_fixture();
    quiet(&mut m, key('B'));
    assert_eq!(m.status.text, "nothing to backfill: r runs the query first");
    let mut m = model_fixture();
    quiet_answer(&mut m, 1000, false);
    quiet(&mut m, key('B'));
    assert_eq!(m.status.text, "nothing to backfill: the result is complete");
    assert_eq!(m.overlay, Overlay::None);
}

fn quiet_answer(m: &mut Model, cap: u32, truncated: bool) {
    assert_eq!(answer(m, cap, truncated), Vec::<Cmd>::new());
}

/// A truncated result names its cap and claims no time span; `B`
/// re-runs the window at 10000 without asking (below the threshold
/// nothing is asked), and the answer replaces the result.
#[test]
fn b_below_10000_reruns_the_window_at_10000_and_replaces_the_result() {
    let mut m = model_fixture();
    quiet_answer(&mut m, 120, true);
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(
        m.status.text,
        format!("{} flows, capped at 120; B raises the cap", n())
    );
    let out = render_to_string(&m, 120, 30);
    assert!(
        out.contains(&format!("{} flows, capped at 120; B raises the cap", n())),
        "{out}"
    );
    assert!(!out.contains("covers"), "no time span: {out}");
    let cmds = update(&mut m, key('B'));
    assert_eq!(m.overlay, Overlay::None, "nothing asked up to 10000");
    assert_eq!(cmds, vec![run_at(&m, 10_000)]);
    assert_eq!(cap(&m), 10_000);
    quiet(&mut m, key('B')); // running: no second request
    assert_eq!(m.status.text, "query already running");
    quiet_answer(&mut m, 10_000, false);
    let d = m.traffic.data.as_ref().unwrap();
    assert!(!d.truncated);
    assert_eq!(d.filter.max_results, 10_000);
    assert_eq!(d.flows.len(), n(), "replaced, never merged or summed");
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains(&format!("{} flows · ", n())), "{out}");
    assert!(!out.contains("capped"), "{out}");
}

/// Above 10000 every rung asks first, even though the last result
/// proved there is more; `B` climbs one rung per `y`.
#[test]
fn above_10000_b_asks_for_each_rung() {
    let mut m = model_fixture();
    quiet_answer(&mut m, 10_000, true);
    quiet(&mut m, key('B'));
    match &m.overlay {
        Overlay::Confirm {
            reason,
            action: PendingAction::RunQuery(f),
        } => {
            assert_eq!(
                reason,
                &format!(
                    "cap 25000 exceeds 10000. Re-run the window at cap 25000 (1 query, replaces the {} flows capped at 10000)? y/n",
                    n()
                )
            );
            assert_eq!(f.max_results, 25_000);
        }
        o => panic!("{o:?}"),
    }
    quiet(&mut m, key('n'));
    assert_eq!(cap(&m), 10_000, "n keeps the cap");
    quiet(&mut m, key('B'));
    let cmds = update(&mut m, key('y'));
    assert_eq!(cmds, vec![run_at(&m, 25_000)]);
    quiet_answer(&mut m, 25_000, true);
    quiet(&mut m, key('B'));
    assert!(
        matches!(&m.overlay, Overlay::Confirm { action: PendingAction::RunQuery(f), .. } if f.max_results == 50_000),
        "{:?}",
        m.overlay
    );
    // At the hard maximum there is no rung left.
    let mut m = model_fixture();
    quiet_answer(&mut m, 200_000, true);
    quiet(&mut m, key('B'));
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.status.text,
        "cap already at the maximum (200000): narrow the filter"
    );
}

#[test]
fn the_opt_in_raises_to_10000_without_b_and_never_above() {
    let mut m = model_fixture();
    m.limits.traffic_auto_backfill = true;
    let cmds = answer(&mut m, 120, true);
    assert_eq!(cmds, vec![run_at(&m, 10_000)]);
    assert_eq!(
        m.status.text,
        format!(
            "{} flows, capped at 120: re-running at cap 10000 (profile traffic_auto_backfill)",
            n()
        )
    );
    // Truncated again at 10000: the opt-in stops, B asks as usual.
    quiet_answer(&mut m, 10_000, true);
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.status.text.contains("capped at 10000; B raises the cap"));
    // A complete result is left alone.
    let mut m = model_fixture();
    m.limits.traffic_auto_backfill = true;
    quiet_answer(&mut m, 120, false);
}

/// The opt-in never overrides a cap the user stepped down to.
#[test]
fn lowering_the_cap_under_the_opt_in_stays_at_the_lower_cap() {
    let mut m = model_fixture();
    m.limits.traffic_auto_backfill = true;
    m.limits.traffic_max_results = 10_000;
    quiet_answer(&mut m, 10_000, true);
    let cmds = update(&mut m, key('['));
    assert_eq!(cmds, vec![run_at(&m, 5000)], "exactly one query");
    quiet_answer(&mut m, 5000, true);
    assert_eq!(cap(&m), 5000);
    assert!(!m.traffic.is_running());
    assert!(m.status.text.contains("capped at 5000; B raises the cap"));
    // `r` on the same filter does not raise it either.
    assert_eq!(update(&mut m, key('r')).len(), 1);
    quiet_answer(&mut m, 5000, true);
    assert_eq!(cap(&m), 5000);
}

#[test]
fn a_late_answer_to_b_is_dropped_after_another_query() {
    let mut m = model_fixture();
    quiet_answer(&mut m, 120, true);
    assert_eq!(update(&mut m, key('B')).len(), 1);
    let stale = m.traffic.seq;
    let f = TrafficFilter::defaults("6h", 500);
    assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
    quiet(
        &mut m,
        Msg::QueryDone {
            seq: stale,
            result: Ok(QueryOutcome {
                flows: Vec::new(),
                truncated: false,
                incomplete: false,
                elapsed: Duration::ZERO,
                hit: false,
                filter: TrafficFilter::defaults("1d", 10_000),
                bounds: None,
                hrefs: Vec::new(),
            }),
        },
    );
    let d = m.traffic.data.as_ref().unwrap();
    assert_eq!((d.flows.len(), d.filter.max_results), (n(), 120));
    assert!(
        m.traffic.is_running(),
        "the 6h query is still the one in flight"
    );
}

#[test]
fn an_incomplete_result_says_so_in_the_status_and_the_header() {
    let mut m = model_fixture();
    let f = TrafficFilter::defaults("1d", 1000);
    assert_eq!(update(&mut m, Msg::ApplyFilter(f.clone())).len(), 1);
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(QueryOutcome {
                flows: fixture_flows_rich(),
                truncated: false,
                incomplete: true,
                elapsed: Duration::ZERO,
                hit: false,
                filter: f,
                bounds: None,
                hrefs: Vec::new(),
            }),
        },
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(
        m.status.text,
        format!("{} flows · incomplete: a PCE region did not respond", n())
    );
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains("incomplete"), "{out}");
}
