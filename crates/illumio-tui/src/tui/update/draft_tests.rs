use super::*;
use crate::tui::model::{Model, Overlay, PendingAction, TrafficView};
use crate::tui::msg::{DraftFailure, DraftOutcome, FocusedOutcome};
use crate::tui::test_support::{model_with_rich_flows, with_draft};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::traffic::{Decisions, Focus, FocusDirection};
use std::time::Duration;

const HREF: &str = "/orgs/1/traffic_flows/async_queries/q-1";

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn esc() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

/// The rich fixture as the answer of our own query `HREF`.
fn model() -> Model {
    let mut m = model_with_rich_flows();
    m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
    m
}

/// The draft answer for the result on screen of `m`.
fn answer(m: &Model, seq: u64, hit: bool) -> Msg {
    let d = m.traffic.data.as_ref().unwrap();
    Msg::DraftDone {
        seq,
        result: Ok(DraftOutcome {
            outcome: QueryOutcome {
                flows: with_draft(d.flows.clone()),
                truncated: d.truncated,
                incomplete: false,
                elapsed: Duration::from_millis(700),
                hit,
                filter: d.filter.clone(),
                bounds: None,
                hrefs: d.hrefs.clone(),
            },
            focus: m.traffic.focus_result.as_ref().map(|r| r.focus.clone()),
            age: if hit {
                Duration::from_secs(40)
            } else {
                Duration::ZERO
            },
        }),
    }
}

/// `D` on `m`: the one `Cmd::RunDraft` it sends.
fn press_d(m: &mut Model) -> (u64, Vec<String>, Option<Focus>, CancellationToken) {
    let cmds = update(m, key('D'));
    let [
        Cmd::RunDraft {
            seq,
            hrefs,
            focus,
            cancel,
            ..
        },
    ] = cmds.as_slice()
    else {
        panic!("{cmds:?}")
    };
    (*seq, hrefs.clone(), focus.clone(), cancel.clone())
}

use tokio_util::sync::CancellationToken;

#[test]
fn draft_decision_marks_go_active_with_draft_decisions_and_wait_again_without() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = crate::tui::test_support::model_with_marks(
        dir.path(),
        "[[mark]]\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ndecision = [\"blocked\", \"potentially_blocked\"]\ndraft_decision = \"allowed\"\ncomment = \"fixed by the pending policy\"\n",
    );
    m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
    let counts = |m: &Model| {
        let mv = m.traffic.marked.as_ref().unwrap();
        (mv.active, mv.waiting)
    };
    assert_eq!(counts(&m), (0, 1));
    assert!(
        m.traffic.flows.marker.is_none(),
        "nothing active, no gutter"
    );
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    assert!(update(&mut m, msg).is_empty());
    assert_eq!(counts(&m), (1, 0), "draft decisions loaded: active");
    let revisit = m
        .traffic
        .flows
        .rows()
        .iter()
        .filter(|r| {
            r.mark
                .is_some_and(|h| h.action == pce::marks::Action::Revisit)
        })
        .count();
    assert!(revisit > 0, "the flows the draft fixes are marked");
    assert!(m.traffic.flows.marker.is_some());
    assert!(m.traffic.marks_meta(TrafficView::Flows).is_none());
    assert!(m.traffic.clear_draft());
    assert_eq!(counts(&m), (0, 1), "draft decisions gone: waiting again");
    assert!(m.traffic.flows.rows().iter().all(|r| r.mark.is_none()));
    assert_eq!(
        m.traffic.marks_meta(TrafficView::Flows).as_deref(),
        Some("1 mark waits for the draft decision (D)")
    );
}

#[test]
fn d_with_a_known_query_sends_one_draft_run_and_no_new_query() {
    let mut m = model();
    let (seq, hrefs, focus, _) = press_d(&mut m);
    assert_eq!(hrefs, vec![HREF.to_string()], "the PUT goes to our query");
    assert_eq!(focus, None);
    assert!(m.traffic.is_draft_running());
    assert!(m.status.text.contains("Esc cancels"), "{}", m.status.text);
    assert!(
        update(&mut m, key('D')).is_empty(),
        "a second D while it runs sends nothing"
    );
    let msg = answer(&m, seq, false);
    assert!(update(&mut m, msg).is_empty());
    assert!(m.traffic.draft_loaded());
    let s = m.traffic.data.as_ref().unwrap().draft.unwrap();
    assert!(s.fixed > 0 && s.newly_blocked > 0);
    assert_eq!(
        m.status.text,
        format!(
            "draft: {} blocked flows fixed · {} would be blocked · D clears",
            s.fixed, s.newly_blocked
        )
    );
    assert_eq!(m.traffic.flows.columns.last().unwrap().title, "draft");
    assert_eq!(m.traffic.groups.columns.last().unwrap().title, "draft");
    assert_eq!(m.traffic.apps.columns.last().unwrap().title, "draft");
    let row = m
        .traffic
        .flows
        .rows()
        .iter()
        .find(|r| r.flow.fixed_by_draft())
        .unwrap();
    let cell = row.flow.draft_text();
    assert!(cell.ends_with(" → allowed"), "{cell}");
}

#[test]
fn d_again_clears_the_draft_with_no_request() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    m.traffic.apps_state.decisions = Decisions::FixedByDraft;
    assert!(update(&mut m, key('D')).is_empty());
    assert!(!m.traffic.draft_loaded());
    assert!(
        m.traffic
            .loaded_flows()
            .iter()
            .all(|f| f.draft_policy_decision.is_none())
    );
    assert_eq!(m.traffic.flows.columns.last().unwrap().title, "decision");
    assert_eq!(
        m.traffic.apps_state.decisions,
        Decisions::All,
        "fixed by draft goes with the draft"
    );
    assert!(m.status.text.contains("no request"), "{}", m.status.text);
}

#[test]
fn d_without_a_known_query_asks_then_runs_it_again_with_the_draft() {
    let mut m = model_with_rich_flows(); // no hrefs: a result we cannot re-evaluate
    assert!(update(&mut m, key('D')).is_empty());
    let Overlay::Confirm { reason, action } = m.overlay.clone() else {
        panic!("{:?}", m.overlay)
    };
    assert!(
        reason.contains("1 POST, 1 PUT, polls and 1 download"),
        "{reason}"
    );
    assert!(matches!(&action, PendingAction::RunDraft { hrefs, .. } if hrefs.is_empty()));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunDraft { hrefs, .. }] if hrefs.is_empty()),
        "{cmds:?}"
    );
    let mut m = model_with_rich_flows();
    update(&mut m, key('D'));
    assert!(update(&mut m, key('n')).is_empty(), "n sends nothing");
    assert!(!m.traffic.is_running());
}

#[test]
fn d_past_a_cost_threshold_asks_first() {
    let mut m = model();
    let d = m.traffic.data.as_mut().unwrap();
    d.filter.max_results = 25_000;
    m.traffic.filter.max_results = 25_000;
    assert!(update(&mut m, key('D')).is_empty());
    let Overlay::Confirm { reason, .. } = m.overlay.clone() else {
        panic!("{:?}", m.overlay)
    };
    assert!(
        reason.contains("1 status check, 1 PUT, polls and 1 download"),
        "{reason}"
    );
    assert!(matches!(
        update(&mut m, key('y')).as_slice(),
        [Cmd::RunDraft { .. }]
    ));
}

#[test]
fn esc_cancels_a_draft_run_and_its_late_answer_is_dropped() {
    let mut m = model();
    let (seq, _, _, cancel) = press_d(&mut m);
    assert!(update(&mut m, esc()).is_empty());
    assert!(cancel.is_cancelled());
    assert!(!m.traffic.is_running());
    assert_eq!(m.status.text, "draft re-evaluation cancelled");
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    assert!(!m.traffic.draft_loaded(), "the late answer is dropped");
}

#[test]
fn a_new_query_a_cap_step_or_a_profile_switch_drops_the_draft_answer() {
    // A new query (`]` steps the cap and re-runs).
    let mut m = model();
    let (seq, _, _, cancel) = press_d(&mut m);
    let cmds = update(&mut m, key(']'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]),
        "{cmds:?}"
    );
    assert!(cancel.is_cancelled());
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    assert!(!m.traffic.draft_loaded());
    // A profile switch.
    let mut m = model();
    let (seq, _, _, cancel) = press_d(&mut m);
    m.reset_data();
    assert!(cancel.is_cancelled());
    let mut m2 = model();
    let msg = answer(&m2, seq, false);
    update(&mut m, msg);
    assert!(!m.traffic.draft_loaded());
    // The answer is checked against the result too, past the seq.
    let (seq, ..) = press_d(&mut m2);
    m2.traffic.data.as_mut().unwrap().filter.window = "2h".into();
    let msg = Msg::DraftDone {
        seq,
        result: Ok(DraftOutcome {
            outcome: QueryOutcome {
                flows: Vec::new(),
                truncated: false,
                incomplete: false,
                elapsed: Duration::ZERO,
                hit: false,
                filter: model().traffic.filter.clone(),
                bounds: None,
                hrefs: vec![HREF.into()],
            },
            focus: None,
            age: Duration::ZERO,
        }),
    };
    update(&mut m2, msg);
    assert!(!m2.traffic.draft_loaded());
    assert_eq!(m2.status.text, "draft answer dropped: the result changed");
}

#[test]
fn a_focus_change_cancels_a_draft_run() {
    let mut m = model();
    let (_, _, _, cancel) = press_d(&mut m);
    update(
        &mut m,
        Msg::Command(Command::Focus(Some("crm/prod".into()))),
    );
    assert!(cancel.is_cancelled(), "setting a focus");
    let mut m = model();
    update(
        &mut m,
        Msg::Command(Command::Focus(Some("crm/prod".into()))),
    );
    let (_, _, _, cancel) = press_d(&mut m);
    update(&mut m, key('X'));
    assert!(cancel.is_cancelled(), "its direction");
    let (_, _, _, cancel) = press_d(&mut m);
    update(&mut m, esc());
    assert!(cancel.is_cancelled(), "Esc first cancels the run");
    assert!(m.traffic.focus.is_some(), "and leaves the focus");
}

#[test]
fn a_focused_result_is_re_evaluated_by_its_hrefs_per_side() {
    let mut m = model();
    let focus = Focus::new(
        vec!["crm".into(), "prod".into()],
        vec!["app".into(), "env".into()],
    );
    update(
        &mut m,
        Msg::Command(Command::Focus(Some("crm/prod".into()))),
    );
    let cmds = update(&mut m, key('r'));
    let seq = match cmds.as_slice() {
        [Cmd::RunFocused { seq, .. }] => *seq,
        // A complete result needs no re-query: set the focused one directly.
        _ => m.traffic.seq,
    };
    let d = m.traffic.data.as_ref().unwrap();
    let hrefs = vec!["/q/in".to_string(), "/q/out".to_string()];
    let outcome = QueryOutcome {
        flows: d.flows[..10].to_vec(),
        truncated: false,
        incomplete: false,
        elapsed: Duration::ZERO,
        hit: false,
        filter: m.traffic.filter.clone(),
        bounds: None,
        hrefs: hrefs.clone(),
    };
    m.traffic.seq = seq;
    update(
        &mut m,
        Msg::FocusedDone {
            seq,
            result: Ok(FocusedOutcome {
                outcome,
                focus: focus.clone().with_direction(FocusDirection::Both),
                queries: 2,
            }),
        },
    );
    assert!(m.traffic.focus_result.is_some());
    let (seq, sent, f, _) = press_d(&mut m);
    assert_eq!(sent, hrefs);
    assert_eq!(f.map(|f| f.app_text()), Some("crm/prod".into()));
    let msg = answer(&m, seq, true);
    update(&mut m, msg);
    assert!(m.traffic.draft_loaded());
    assert!(m.status.text.ends_with("(cached 40s)"), "{}", m.status.text);
}

#[test]
fn a_dropped_query_asks_to_run_it_again_and_a_refusal_keeps_the_plain_result() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    update(
        &mut m,
        Msg::DraftDone {
            seq,
            result: Err(DraftFailure::Gone("not found: q-1".into())),
        },
    );
    let Overlay::Confirm { reason, action } = m.overlay.clone() else {
        panic!("{:?}", m.overlay)
    };
    assert!(
        reason.starts_with("The PCE no longer holds this query"),
        "{reason}"
    );
    assert!(matches!(action, PendingAction::RunDraft { hrefs, .. } if hrefs.is_empty()));
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let n = m.traffic.loaded_flows().len();
    update(
        &mut m,
        Msg::DraftDone {
            seq,
            result: Err(DraftFailure::Failed("the PCE refused it".into())),
        },
    );
    assert!(!m.traffic.is_running() && !m.traffic.draft_loaded());
    assert_eq!(m.traffic.loaded_flows().len(), n, "the plain result stays");
    assert_eq!(m.status.text, "draft: the PCE refused it");
}

/// The answer of a successful rule delete.
fn written() -> Msg {
    Msg::WriteDone {
        kind: pce::write::ObjectKind::Rule,
        method: Some(pce::WriteMethod::Delete),
        result: Ok(pce::write::Outcome::Written(pce::write::Written {
            href: "/orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2".into(),
            pending: true,
            status: 204,
            object: serde_json::Value::Null,
            dropped_fields: vec![],
            warnings: vec![],
            audit_failed: None,
        })),
    }
}

/// The `Cmd::RunDraft`s among `cmds`: their hrefs.
fn redrafts(cmds: &[Cmd]) -> Vec<Vec<String>> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::RunDraft { hrefs, .. } => Some(hrefs.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_write_under_draft_decisions_evaluates_them_again_once() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    assert!(m.traffic.draft_loaded());
    m.writes.in_flight = true;
    let cmds = update(&mut m, written());
    assert_eq!(redrafts(&cmds), vec![vec![HREF.to_string()]], "{cmds:?}");
    assert!(!m.traffic.draft_loaded() && m.traffic.is_draft_running());
    assert_eq!(
        m.status.text,
        "deleted /orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2 (pending) · draft re-evaluation running · Esc cancels"
    );
    // A run in flight at the write: cancelled and started once more.
    let mut m = model();
    let (.., cancel) = press_d(&mut m);
    m.writes.in_flight = true;
    let cmds = update(&mut m, written());
    assert!(cancel.is_cancelled(), "the stale run is cancelled");
    assert_eq!(redrafts(&cmds).len(), 1, "{cmds:?}");
    assert!(m.traffic.is_draft_running());
}

#[test]
fn an_unrelated_write_without_draft_decisions_asks_nothing() {
    let mut m = model();
    m.writes.in_flight = true;
    let cmds = update(&mut m, written());
    assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
    assert!(!m.traffic.is_running());
    assert_eq!(
        m.status.text,
        "deleted /orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2 (pending)"
    );
}

#[test]
fn a_failed_write_keeps_the_draft_and_asks_nothing() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    m.writes.in_flight = true;
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Delete),
            result: Err(pce::write::WriteError::invalid("refused")),
        },
    );
    assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
    assert!(m.traffic.draft_loaded() && !m.traffic.is_running());
}

#[test]
fn a_write_drops_the_draft_and_the_next_d_asks_the_pce() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    assert!(m.traffic.draft_loaded());
    // No query of this session behind it (a cached result): `D` would
    // ask first, so the write only drops the draft.
    m.traffic.data.as_mut().unwrap().hrefs.clear();
    m.writes.in_flight = true;
    let cmds = update(&mut m, written());
    assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
    assert!(!m.traffic.draft_loaded() && !m.traffic.is_running());
    assert!(
        m.status.text.ends_with("draft cleared: D re-evaluates"),
        "{}",
        m.status.text
    );
    m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
    let (_, hrefs, ..) = press_d(&mut m);
    assert_eq!(hrefs, vec![HREF.to_string()], "D runs again, not a clear");
}

#[test]
fn a_draft_only_on_the_result_kept_aside_is_dropped_not_evaluated() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    // The drafted result kept aside, a plain one on screen.
    m.traffic.unfocused = m.traffic.data.take();
    m.traffic.data = model().traffic.data;
    m.writes.in_flight = true;
    let cmds = update(&mut m, written());
    assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
    assert!(m.traffic.unfocused.as_ref().unwrap().draft.is_none());
    assert!(
        m.status.text.ends_with("draft cleared: D re-evaluates"),
        "{}",
        m.status.text
    );
}

#[test]
fn a_write_off_the_traffic_screen_says_where_esc_cancels() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    m.screen = crate::tui::model::Screen::Policy;
    m.writes.in_flight = true;
    let cmds = update(&mut m, written());
    assert_eq!(redrafts(&cmds).len(), 1, "{cmds:?}");
    assert!(
        m.status
            .text
            .ends_with(" · draft re-evaluation running · Esc on Traffic cancels"),
        "{}",
        m.status.text
    );
}

#[test]
fn a_provision_evaluates_the_draft_and_keeps_its_own_report_for_the_pending_list() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    m.policy.provision_counts = Some("rule_sets=1".into());
    m.writes.in_flight = true;
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Provision,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: "/orgs/1/sec_policy/7".into(),
                pending: false,
                status: 201,
                object: serde_json::Value::Null,
                dropped_fields: vec![],
                warnings: vec![],
                audit_failed: None,
            })),
        },
    );
    assert_eq!(redrafts(&cmds).len(), 1, "{cmds:?}");
    assert_eq!(
        m.policy.provisioned.as_deref(),
        Some("provisioned /orgs/1/sec_policy/7: rule_sets=1")
    );
    assert!(m.status.text.contains("draft re-evaluation running"));
}

#[test]
fn an_unanswered_write_reloads_but_evaluates_nothing() {
    let mut m = model();
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    m.policy.write_from_flow = true;
    m.writes.in_flight = true;
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Delete),
            result: Err(pce::write::WriteError::Unanswered(pce::PceError::Timeout(
                Duration::from_secs(30),
            ))),
        },
    );
    assert!(!cmds.is_empty(), "it reloads");
    assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
    assert!(!m.policy.write_from_flow, "taken");
}

#[test]
fn a_draft_re_run_takes_the_new_time_range() {
    let mut m = model_with_rich_flows(); // no hrefs: D re-runs the query
    update(&mut m, key('D'));
    let cmds = update(&mut m, key('y'));
    let [Cmd::RunDraft { seq, .. }] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    let end: jiff::Timestamp = "2026-10-11T00:00:00Z".parse().unwrap();
    let start: jiff::Timestamp = "2026-10-10T00:00:00Z".parse().unwrap();
    let d = m.traffic.data.as_ref().unwrap();
    let msg = Msg::DraftDone {
        seq: *seq,
        result: Ok(DraftOutcome {
            outcome: QueryOutcome {
                flows: with_draft(d.flows.clone()),
                truncated: false,
                incomplete: false,
                elapsed: Duration::ZERO,
                hit: false,
                filter: d.filter.clone(),
                bounds: Some((start, end)),
                hrefs: vec!["/orgs/1/traffic_flows/async_queries/new".into()],
            },
            focus: None,
            age: Duration::ZERO,
        }),
    };
    update(&mut m, msg);
    let r = m.traffic.data.as_ref().unwrap().range.unwrap();
    assert_eq!((r.start, r.end), (start, end));
}

#[test]
fn the_decision_filter_offers_fixed_by_draft_only_with_draft_data() {
    let mut m = model();
    m.traffic.view = TrafficView::Aggregate;
    for want in [Decisions::Blocked, Decisions::BlockedOnly, Decisions::All] {
        update(&mut m, key('d'));
        assert_eq!(m.traffic.apps_state.decisions, want);
    }
    let (seq, ..) = press_d(&mut m);
    let msg = answer(&m, seq, false);
    update(&mut m, msg);
    for _ in 0..3 {
        update(&mut m, key('d'));
    }
    assert_eq!(m.traffic.apps_state.decisions, Decisions::FixedByDraft);
    assert_eq!(m.status.text, "decision fixed by draft");
    let fixed = m.traffic.data.as_ref().unwrap().draft.unwrap().fixed;
    let total: u64 = m.traffic.groups.rows().iter().map(|g| g.flows).sum();
    assert_eq!(total, fixed, "the pairs count only the fixed flows");
    update(&mut m, key('d'));
    assert_eq!(m.traffic.apps_state.decisions, Decisions::All);
}
