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
        Some("vs 7d before 10-09 08:00 · 1 new · 1 changed · 1 gone")
    );
    assert_eq!(
        m.status.text,
        "vs 7d before 10-09 08:00 · 1 new · 1 changed · 1 gone"
    );
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("vs 7d before 10-09 08:00 · 1 new · 1 changed · 1 gone"),
        "{out}"
    );
    // The gone flow comes last, below the screen's rows.
    assert!(out.contains("CHG") && out.contains("NEW"), "{out}");
    // Every other view has its own counts.
    for view in ["App-to-app", "Mesh", "Blocked"] {
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view.title(), view);
        let meta = m.traffic.compare_meta().unwrap();
        assert!(
            meta.starts_with("vs 7d before 10-09 08:00 · "),
            "{view}: {meta}"
        );
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
        Some("vs 7d before 10-09 08:00 · no Δ here")
    );
    let out = render_to_string(&m, 160, 48);
    assert!(!out.contains("GONE") && !out.contains("CHG"), "{out}");
}

#[test]
fn app_to_app_pairs_by_service_and_sections_compare_too() {
    let mut m = compared();
    quiet(&mut m, key('v'));
    // The plain pair from its service keys: 5432 the same, 9999 gone.
    let pair = m
        .traffic
        .groups
        .rows()
        .iter()
        .find(|g| g.src.join("/") == "shop/prod" && g.dst.join("/") == "shop/prod")
        .and_then(|g| g.change)
        .unwrap();
    assert_eq!(
        pair.change,
        Change::Changed,
        "a gone service of a pair still talking"
    );
    // #79: the header counts the rows shown: pairs here (the pair with a
    // gone service is one changed row, no gone one) ...
    let pairs = m.traffic.view_counts().unwrap();
    assert_eq!(
        (pairs.new, pairs.changed, pairs.gone),
        (0, 2, 0),
        "{pairs:?}"
    );
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
    // ... and service rows by service.
    let services = m.traffic.view_counts().unwrap();
    assert_eq!(services.gone, 1, "{services:?}");
    assert_ne!(services, pairs);
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

#[test]
fn blocked_judges_a_key_on_every_decision_so_a_decision_change_is_chg() {
    // 8443 crm/prod → erp/prod: allowed then, potentially blocked now.
    let mut m = compared();
    for _ in 0..3 {
        quiet(&mut m, key('v'));
    }
    assert_eq!(m.traffic.view, TrafficView::Blocked);
    let row = m
        .traffic
        .blocked
        .rows()
        .iter()
        .filter_map(|r| r.peer().map(|(_, p)| p))
        .find(|p| p.service == "8443/tcp" && p.peer.join("/") == "crm/prod")
        .unwrap();
    assert_eq!(row.change.unwrap().change, Change::Changed);
    assert_eq!(row.change.unwrap().short_text(), "allowed→pot.blocked");
    // App-to-app `d` blocked: the same.
    let mut m = compared();
    quiet(&mut m, key('v'));
    quiet(&mut m, key('o'));
    quiet(&mut m, key('d'));
    let row = m
        .traffic
        .services
        .rows()
        .iter()
        .find(|g| g.service.as_deref() == Some("8443/tcp") && g.src.join("/") == "crm/prod")
        .unwrap();
    assert_eq!(row.change.unwrap().text(), "allowed→potentially_blocked");
}

#[test]
fn fixed_by_draft_compares_unfiltered_and_shows_no_gone_row() {
    let mut m = crate::tui::test_support::with_draft_loaded(model());
    compare_with(&mut m, baseline_flows_rich(), false);
    quiet(&mut m, key('v'));
    quiet(&mut m, key('o'));
    for _ in 0..4 {
        if m.traffic.apps_state.decisions == pce::traffic::Decisions::FixedByDraft {
            break;
        }
        quiet(&mut m, key('d'));
    }
    assert_eq!(
        m.traffic.apps_state.decisions,
        pce::traffic::Decisions::FixedByDraft
    );
    let rows = m.traffic.services.rows();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|g| g.change.is_some_and(|c| c.change != Change::Gone)),
        "the baseline carries no draft: nothing passes as gone"
    );
    assert!(
        rows.iter()
            .any(|g| g.change.is_some_and(|c| c.change != Change::New)),
        "keys are judged unfiltered, not all new"
    );
}

#[test]
fn the_compare_ended_note_stays_while_the_query_runs_and_with_its_result() {
    let mut m = compared();
    let mut f = m.traffic.filter.clone();
    f.src = "app=shop".into();
    let _ = update(&mut m, Msg::ApplyFilter(f.clone()));
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::QueryProgress {
            seq,
            progress: pce::traffic::QueryProgress::Polling {
                status: "running".into(),
                elapsed: Duration::from_secs(1),
            },
        },
    );
    assert!(
        m.status.text.starts_with("query running"),
        "{}",
        m.status.text
    );
    assert!(
        m.status
            .text
            .ends_with("· filter changed: c compares again")
    );
    quiet(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Ok(crate::tui::msg::QueryOutcome {
                flows: fixture_flows_rich(),
                truncated: false,
                incomplete: false,
                elapsed: Duration::from_millis(10),
                hit: false,
                filter: f,
                bounds: Some((ts("2026-10-09T08:00:00Z"), ts(FIXTURE_END))),
                hrefs: Vec::new(),
            }),
        },
    );
    assert!(
        m.status
            .text
            .ends_with("· filter changed: c compares again"),
        "{}",
        m.status.text
    );
    assert!(!m.traffic.compare_ended, "said once with the result");
    // A running baseline names itself.
    let mut m = model();
    quiet(&mut m, key('c'));
    let _ = update(&mut m, code(KeyCode::Enter));
    let seq = m.traffic.seq;
    quiet(
        &mut m,
        Msg::QueryProgress {
            seq,
            progress: pce::traffic::QueryProgress::Submitted { href: "/q".into() },
        },
    );
    assert!(
        m.status.text.starts_with("baseline query running"),
        "{}",
        m.status.text
    );
}

#[test]
fn headers_count_recent_rows_and_a_section_of_gone_rows_reads_gone() {
    // A gone flow from a source no recent flow has: its own section.
    let mut base = baseline_flows_rich();
    let mut lone = base[3].clone();
    lone.src = pce::traffic::ResolvedEndpoint {
        ip: Some("10.9.9.9".into()),
        ..Default::default()
    };
    base.push(lone);
    let plain = model();
    let mut m = model();
    compare_with(&mut m, base, false);
    // Flows by src app: shop/prod's header counts its recent flows only.
    let header = |m: &Model, app: &str| -> String {
        m.traffic
            .flows_grouped
            .rows()
            .iter()
            .filter_map(|r| r.header_line())
            .find(|h| h.contains(&format!(" {app}  ")))
            .unwrap_or_default()
    };
    let mut p = plain;
    quiet(&mut p, key('x'));
    quiet(&mut m, key('x'));
    let shop = header(&m, "shop/prod");
    assert!(!shop.is_empty());
    assert_eq!(
        shop,
        header(&p, "shop/prod"),
        "the gone 9999 flow is not counted"
    );
    assert!(
        header(&m, "10.9.9.9/-").ends_with("gone"),
        "{}",
        header(&m, "10.9.9.9/-")
    );
    // App-to-app `out`: the same section reads `out · gone`.
    quiet(&mut m, key('v'));
    quiet(&mut m, key('x'));
    quiet(&mut m, key('x'));
    assert_eq!(m.traffic.apps_state.by, crate::tui::model::By::Out);
    let line = m
        .traffic
        .apps
        .rows()
        .iter()
        .filter_map(|r| r.header_line())
        .find(|h| h.contains("10.9.9.9"))
        .unwrap();
    assert!(line.ends_with("out · gone"), "{line}");
}

#[test]
fn a_mark_on_the_draft_decision_squelches_the_key_on_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    // with_draft: the allowed 3306 flow at index 3 is blocked in draft.
    let marks = r#"
[[mark]]
id = "m-sqdraft"
action = "squelch"
src = "any"
dst = "any"
service = "3306/tcp"
draft_decision = "blocked"
comment = "noise in draft"
"#;
    let mut m = crate::tui::test_support::model_with_marks(dir.path(), marks);
    m.traffic.filter.window = "1d".into();
    if let Some(d) = m.traffic.data.as_mut() {
        d.filter = m.traffic.filter.clone();
        d.range = Some(crate::tui::model::Range {
            start: ts("2026-10-09T08:00:00Z"),
            end: ts(FIXTURE_END),
        });
    }
    let mut m = crate::tui::test_support::with_draft_loaded(m);
    let hidden = m.traffic.squelched_hidden();
    assert!(hidden >= 1, "the draft mark hides a recent record");
    compare_with(&mut m, baseline_flows_rich(), false);
    let gone: Vec<Option<u16>> = m
        .traffic
        .flows
        .rows()
        .iter()
        .filter(|r| r.change.is_some_and(|c| c.change == Change::Gone))
        .map(|r| r.port)
        .collect();
    assert_eq!(gone, vec![Some(9999)], "the squelched 3306 key is not gone");
}

#[test]
fn a_cancelled_first_baseline_ends_compare_and_an_incomplete_one_warns() {
    let mut m = model();
    quiet(&mut m, key('c'));
    let _ = update(&mut m, code(KeyCode::Enter));
    assert!(m.traffic.compare.is_some());
    // Another cap of the same filter (`:load`): the baseline run goes.
    let mut f = m.traffic.filter.clone();
    f.max_results = 2000;
    let _ = update(&mut m, Msg::ApplyFilter(f));
    assert!(
        m.traffic.compare.is_none(),
        "never `baseline running` forever"
    );

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
    quiet(
        &mut m,
        Msg::BaselineDone {
            seq,
            result: Ok(BaselineOutcome {
                flows: baseline_flows_rich(),
                truncated: false,
                incomplete: true,
                hit: false,
                filter,
                start,
                end,
            }),
        },
    );
    assert_eq!(
        m.traffic.compare_warning().as_deref(),
        Some("baseline incomplete: a PCE region did not respond")
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
}

#[test]
fn the_result_line_counts_the_rows_the_tables_show_in_compare_mode() {
    let dir = tempfile::tempdir().unwrap();
    // Matches the 8443 flow's baseline record only (allowed then,
    // potentially blocked now): its key leaves both sides.
    let marks = r#"
[[mark]]
id = "m-sq8443"
action = "squelch"
src = "any"
dst = "any"
service = "8443/tcp"
decision = "allowed"
comment = "allowed then"
"#;
    let mut m = crate::tui::test_support::model_with_marks(dir.path(), marks);
    m.traffic.filter.window = "1d".into();
    if let Some(d) = m.traffic.data.as_mut() {
        d.filter = m.traffic.filter.clone();
        d.range = Some(crate::tui::model::Range {
            start: ts("2026-10-09T08:00:00Z"),
            end: ts(FIXTURE_END),
        });
    }
    m.traffic.rebuild_views();
    let n = fixture_flows_rich().len();
    assert_eq!(m.traffic.squelched_hidden(), 0, "no recent record matches");
    compare_with(&mut m, baseline_flows_rich(), false);
    assert_eq!(m.traffic.squelched_hidden(), 1);
    let recent_rows = m
        .traffic
        .flows
        .rows()
        .iter()
        .filter(|r| r.change.is_some_and(|c| c.change != Change::Gone))
        .count();
    assert_eq!(recent_rows, n - 1, "the table without the hidden key");
    assert_eq!(m.traffic.shown_flows().len(), n - 1);
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains(&format!("{} of {n} flows (1 squelched)", n - 1)),
        "{out}"
    );
    // Compare off: as before.
    quiet(&mut m, key('C'));
    assert_eq!(m.traffic.squelched_hidden(), 0);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains(&format!("│{n} flows ")), "{out}");
}

/// The rows of the view on screen with `c`, as its table holds them.
fn shown_with(m: &Model, c: Change) -> u64 {
    let t = &m.traffic;
    let is = |x: Option<&pce::traffic::RowChange>| x.is_some_and(|x| x.change == c);
    let n = match t.view {
        TrafficView::Flows => t
            .flows
            .rows()
            .iter()
            .filter(|r| is(r.change.as_ref()))
            .count(),
        TrafficView::Aggregate if t.apps_state.by != crate::tui::model::By::Pairs => {
            t.apps.rows().iter().filter(|r| is(r.change())).count()
        }
        TrafficView::Aggregate => t
            .pairs_table()
            .rows()
            .iter()
            .filter(|r| is(r.change.as_ref()))
            .count(),
        TrafficView::Mesh => t
            .mesh
            .rows()
            .iter()
            .filter(|r| is(r.change.as_ref()))
            .count(),
        TrafficView::Blocked => t.blocked.rows().iter().filter(|r| is(r.change())).count(),
        TrafficView::Treemap => 0,
    };
    n as u64
}

/// #79: the header counts the rows the view shows, in every view, App-to-app
/// with `o` off (pair rows) and on (service rows), under every `n`.
#[test]
fn the_header_counts_equal_the_rows_each_view_shows_under_every_n() {
    let mut m = compared();
    let mut seen = Vec::new();
    for step in 0..6 {
        match step {
            0 => {}
            // App-to-app pairs, then by service, then the sections.
            1 => quiet(&mut m, key('v')),
            2 => quiet(&mut m, key('o')),
            3 => {
                quiet(&mut m, key('o'));
                quiet(&mut m, key('x'));
            }
            _ => quiet(&mut m, key('v')),
        }
        let what = format!(
            "{} by={:?} o={}",
            m.traffic.view.title(),
            m.traffic.apps_state.by,
            m.traffic.apps_state.by_service
        );
        let counts = m.traffic.view_counts().unwrap();
        for c in [Change::New, Change::Changed, Change::Gone] {
            assert_eq!(counts.of(c), shown_with(&m, c), "{what}: {c:?} (n all)");
        }
        // Each `n` narrows the rows to what its count says.
        for _ in 0..4 {
            quiet(&mut m, key('n'));
            let show = m.traffic.compare.as_ref().unwrap().show;
            assert_eq!(
                m.traffic.view_counts().unwrap(),
                counts,
                "{what}: n keeps them"
            );
            for c in [Change::New, Change::Changed, Change::Gone] {
                let want = if show == ShowChanges::All || show.set().contains(c) {
                    counts.of(c)
                } else {
                    0
                };
                assert_eq!(shown_with(&m, c), want, "{what}: {c:?} under {show:?}");
            }
        }
        seen.push(what);
    }
    assert_eq!(seen.len(), 6, "{seen:?}");
}

/// The decision cell's text and its first cell's foreground at `w`×48 on
/// the line holding `text`.
fn decision_fg(m: &Model, w: u16, text: &str) -> Option<ratatui::style::Color> {
    let mut term =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, 48)).expect("test terminal");
    term.draw(|f| crate::tui::ui::view(m, f)).expect("draw");
    let buf = term.backend().buffer();
    for y in 0..buf.area.height {
        let cells: Vec<&ratatui::buffer::Cell> =
            (0..buf.area.width).map(|x| &buf[(x, y)]).collect();
        let line: Vec<String> = cells.iter().map(|c| c.symbol().to_string()).collect();
        let want: Vec<String> = text.chars().map(|c| c.to_string()).collect();
        if let Some(at) = line.windows(want.len()).position(|w| w == want.as_slice()) {
            return cells[at].fg.into();
        }
    }
    None
}

/// #79: a CHG row's decision cell shows `baseline→recent` in the warning
/// colour, ellipsized to the column, in every view with a decision column;
/// `:export` keeps the decision.
#[test]
fn a_changed_rows_decision_cell_shows_the_transition_in_the_warning_colour() {
    let mut m = compared();
    // Changed rows only, so they are on screen.
    for _ in 0..2 {
        quiet(&mut m, key('n'));
    }
    assert_eq!(
        m.traffic.compare.as_ref().unwrap().show,
        ShowChanges::Changed
    );
    let warn = m.theme.warn;
    let mut seen = Vec::new();
    for step in 0..5 {
        match step {
            0 => {}
            1 => quiet(&mut m, key('v')),
            2 => quiet(&mut m, key('o')),
            3 => {
                quiet(&mut m, key('o'));
                quiet(&mut m, key('x'));
            }
            _ => {
                quiet(&mut m, key('v'));
                assert_eq!(m.traffic.view, TrafficView::Mesh);
            }
        }
        let t = &m.traffic;
        if step == 1 {
            // Plain pairs have per-decision counts, no decision cell.
            assert!(!t.groups.columns.iter().any(|c| c.title == "decision"));
            continue;
        }
        // (the decision cell of the first CHG row, its change)
        fn first<T>(
            v: &crate::tui::table::TableView<T>,
            change: impl Fn(&T) -> Option<&pce::traffic::RowChange>,
        ) -> (String, String, pce::traffic::RowChange) {
            let col = v
                .columns
                .iter()
                .position(|c| c.title == "decision")
                .unwrap();
            let row = v
                .rows()
                .iter()
                .find(|r| change(r).is_some_and(|c| c.change == Change::Changed))
                .expect("a changed row");
            (
                v.cell(row, col),
                v.full_cells(row)[col].clone(),
                *change(row).unwrap(),
            )
        }
        let (cell, exported, rc) = match t.view {
            TrafficView::Flows => first(&t.flows, |r| r.change.as_ref()),
            TrafficView::Aggregate if t.apps_state.by != crate::tui::model::By::Pairs => {
                first(&t.apps, |r| r.change())
            }
            TrafficView::Aggregate => first(t.pairs_table(), |r| r.change.as_ref()),
            TrafficView::Mesh => first(&t.mesh, |r| r.change.as_ref()),
            _ => unreachable!(),
        };
        let what = format!("{} step {step}", t.view.title());
        let full = format!(
            "{}→{}",
            rc.baseline_decision.unwrap().text(),
            rc.recent_decision.unwrap().text()
        )
        .replace("potentially_blocked", "pot.blocked");
        let width = if t.view == TrafficView::Flows { 19 } else { 11 };
        let want = if full.chars().count() > width {
            format!("{}…", full.chars().take(width - 1).collect::<String>())
        } else {
            full.clone()
        };
        assert_eq!(cell, want, "{what}");
        assert!(cell.contains('→'), "{what}: {cell}");
        assert!(
            !exported.contains('→'),
            "{what}: :export keeps the decision: {exported}"
        );
        assert_eq!(decision_fg(&m, 160, &cell), Some(warn), "{what}: {cell}");
        seen.push(cell);
    }
    assert_eq!(seen.len(), 4, "{seen:?}");
    // Narrow (below 120 columns): the short words, ellipsized to 8.
    assert_eq!(m.traffic.view, TrafficView::Mesh);
    assert_eq!(decision_fg(&m, 100, "allow→p…"), Some(warn));
}
