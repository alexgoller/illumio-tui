use super::*;
use crate::tui::test_support::*;
use pce::enforcement::{RecordWorkload, afters, read_record, write_record};
use pce::models::DraftDecision;
use pce::traffic::parse_app;

/// The rich fixture, its draft decisions, the rulesets, the workloads and
/// the pending list (Global-Infra and Corp) loaded.
pub(crate) fn loaded() -> Model {
    let mut m = with_draft_loaded(with_policy(with_workloads(model_with_window("24h"))));
    m.policy.pending = Some(pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../../../fixtures/pending_list.json")).unwrap(),
    ));
    m
}

/// `loaded` with every flow allowed by the draft and nothing pending:
/// shop/prod is ready.
pub(crate) fn all_allowed() -> Model {
    let mut m = loaded();
    if let Some(d) = m.traffic.data.as_mut() {
        for f in &mut d.flows {
            f.draft_policy_decision = Some(DraftDecision::Allowed);
        }
        d.draft = Some(pce::traffic::DraftSummary::of(&d.flows));
    }
    m.policy.pending = Some(Vec::new());
    m.traffic.rebuild_views();
    m
}

fn shop() -> GroupKey {
    parse_app("shop/prod")
}

fn view_of(m: &Model) -> EnforceView {
    let mut v = EnforceView::new(shop(), 1);
    v.set_readiness(compute(m, &[shop()], true).pop());
    v
}

#[test]
fn steps_wrap_both_ways() {
    assert_eq!(Step::Readiness.prev(), Step::Watch);
    assert_eq!(Step::Watch.next(), Step::Readiness);
    let mut s = Step::Readiness;
    let mut seen = Vec::new();
    for _ in 0..5 {
        seen.push(s);
        s = s.next();
    }
    assert_eq!(seen, Step::ALL);
}

#[test]
fn readiness_reads_only_what_the_model_holds() {
    let m = loaded();
    let rows = compute(&m, &[], false);
    assert_eq!(rows.len(), 8, "every app of the result");
    let r = rows.iter().find(|r| r.app == "shop/prod").unwrap();
    assert_eq!(r.verdict, Verdict::NeedsRules);
    assert_eq!(r.workloads.total, 2);
    assert_eq!(r.pending.count, 1, "Global-Infra covers every app");
    assert!(r.gaps.is_none(), "the list asks for no gaps");
    assert!(!Missing::of(&m).any());
    // Without the draft decisions: not evaluated everywhere.
    let plain = with_policy(with_workloads(model_with_window("24h")));
    assert!(
        compute(&plain, &[], false)
            .iter()
            .all(|r| r.verdict == Verdict::NotEvaluated)
    );
    let missing = Missing::of(&plain);
    assert!(missing.pending && !missing.workloads && !missing.rulesets);
    assert_eq!(missing.text(), "pending changes");
    // No name index: nothing judged.
    let mut none = loaded();
    none.connection = crate::tui::model::Connection::Connecting;
    assert!(compute(&none, &[], false).is_empty());
}

#[test]
fn the_steps_say_what_blocks_the_move() {
    let m = loaded();
    let v = view_of(&m);
    let ok = Missing::default();
    assert_eq!(v.step_state(Step::Readiness, &ok), "needs rules");
    assert_eq!(v.step_state(Step::Rules, &ok), "3 gaps");
    assert_eq!(v.step_state(Step::Provision, &ok), "1 pending");
    assert_eq!(v.step_state(Step::Enforce, &ok), "blocked");
    assert_eq!(v.step_state(Step::Watch, &ok), "no record");
    assert_eq!(v.step_len(Step::Rules), 3);
    // Not evaluated: the rules wait for `D`.
    let plain = with_policy(with_workloads(model_with_window("24h")));
    let v = view_of(&plain);
    assert_eq!(v.step_state(Step::Rules, &ok), "D first");
    assert_eq!(v.step_state(Step::Enforce, &ok), "blocked");
    // Ready, but something still loading: not offered.
    let m = all_allowed();
    let v = view_of(&m);
    assert_eq!(v.readiness.as_ref().unwrap().verdict, Verdict::Ready);
    assert_eq!(v.step_state(Step::Enforce, &ok), "ready");
    let loading = Missing {
        workloads: true,
        ..Missing::default()
    };
    assert_eq!(v.step_state(Step::Enforce, &loading), "blocked");
    assert_eq!(v.step_state(Step::Readiness, &loading), "loading…");
}

#[test]
fn step_four_cursor_picks_the_mode_and_v_the_level() {
    let mut v = view_of(&all_allowed());
    v.step = Step::Enforce;
    assert_eq!(v.mode, EnforcementMode::Full);
    v.move_cursor(1);
    assert_eq!(v.mode, EnforcementMode::Selective);
    v.move_cursor(5);
    assert_eq!(v.mode, EnforcementMode::Selective, "clamped");
    v.move_cursor(-1);
    assert_eq!(v.mode, EnforcementMode::Full);
    assert_eq!(v.visibility_text(), "keep each workload's level");
    v.visibility = 2;
    assert_eq!(v.target().text(), "full · flow_drops");
}

#[test]
fn the_list_sorts_searches_and_keeps_the_cursor_on_its_app() {
    let m = loaded();
    let mut v = ReadinessView::default();
    v.set_rows(compute(&m, &[], false));
    assert_eq!(v.counts_text(), "8 apps · 8 needs rules");
    // By verdict, then most would-block first.
    assert_eq!(v.shown()[0].app, "shop/prod");
    v.sort = ReadinessSort::App;
    assert_eq!(v.shown()[0].app, "crm/dev");
    v.cursor = 7;
    assert_eq!(v.selected().unwrap().app, "shop/prod");
    v.set_rows(compute(&m, &[], false));
    assert_eq!(v.selected().unwrap().app, "shop/prod", "kept on its app");
    v.search = "HR".into();
    assert_eq!(v.shown().len(), 2);
    v.move_cursor(9);
    assert_eq!(v.cursor, 1);
}

#[test]
fn a_rollback_preview_wants_the_exact_count() {
    let mut p = RollbackPreview {
        record_path: "r.json".into(),
        record: ChangeRecord {
            app: "shop/prod".into(),
            profile: "demo".into(),
            pce: "pce.example".into(),
            org_id: 5,
            at: "2026-10-19T10:00:00Z".into(),
            to: None,
            rollback_of: None,
            readiness: None,
            workloads: Vec::new(),
        },
        plans: Vec::new(),
        skipped: Vec::new(),
        count: 3,
        needs_count: true,
        typed: String::new(),
    };
    assert!(!p.count_ok());
    p.typed = "2".into();
    assert!(!p.count_ok());
    p.typed = "3".into();
    assert!(p.count_ok());
    p.needs_count = false;
    p.typed.clear();
    assert!(p.count_ok());
}

/// A move record of shop/prod on `pce`, written into `dir`.
pub(crate) fn write_move(dir: &Path, pce: &str, at: &str) -> PathBuf {
    let rec = ChangeRecord {
        app: "shop/prod".into(),
        profile: "demo".into(),
        pce: pce.into(),
        org_id: 5,
        at: at.into(),
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
    write_record(dir, &rec, at.parse().unwrap()).unwrap()
}

#[test]
fn the_newest_record_of_this_pce_is_read_and_its_rollback_noticed() {
    let dir = tempfile::tempdir().unwrap();
    let here = RecordOrigin {
        profile: "demo".into(),
        pce: "pce.example".into(),
        org_id: 5,
    };
    let mut v = EnforceView::new(shop(), 1);
    v.load_record(Some(dir.path()), &here);
    assert!(v.record.is_none() && v.record_error.is_none());
    let old = write_move(dir.path(), "pce.example", "2026-10-18T10:00:00Z");
    let new = write_move(dir.path(), "pce.example", "2026-10-19T10:00:00Z");
    write_move(dir.path(), "other.example", "2026-10-20T10:00:00Z");
    v.load_record(Some(dir.path()), &here);
    assert_eq!(v.record.as_ref().map(|(p, _)| p), Some(&new), "not {old:?}");
    assert!(v.rolled_back_by.is_none());
    assert_eq!(v.step_state(Step::Watch, &Missing::default()), "moved");
    // Its rollback names it.
    let mut rb = read_record(&new).unwrap();
    rb.rollback_of = Some(new.display().to_string());
    let rb_path = write_record(dir.path(), &rb, jiff::Timestamp::now()).unwrap();
    v.load_record(Some(dir.path()), &here);
    assert_eq!(v.rolled_back_by.as_ref(), Some(&rb_path));
    assert_eq!(
        v.step_state(Step::Watch, &Missing::default()),
        "rolled back"
    );
    // No place for records: none.
    v.load_record(None, &here);
    assert!(v.record.is_none());
}

#[test]
fn the_record_is_written_pending_first_and_then_with_the_results() {
    let dir = tempfile::tempdir().unwrap();
    probe(dir.path()).unwrap();
    let plan = {
        let m = all_allowed();
        let idx = m.index().unwrap().clone();
        let targets: Vec<pce::write::BulkTarget> = m
            .workloads
            .raw
            .iter()
            .take(2)
            .map(|w| pce::write::BulkTarget::of(w, &idx))
            .collect();
        pce::write::plan_bulk(
            &targets,
            &pce::write::BulkChange::enforce(EnforcementMode::Full, None),
            Default::default(),
            None,
        )
        .unwrap()
    };
    let to = ModeState::target(EnforcementMode::Full, None);
    let afters = afters(&[(to.clone(), &plan)]);
    assert_eq!(afters.len(), 1, "host-1 is full already");
    let origin = RecordOrigin {
        profile: "demo".into(),
        pce: "pce.example".into(),
        org_id: 5,
    };
    let rec = ChangeRecord::of_move(
        "shop/prod",
        &origin,
        jiff::Timestamp::now(),
        Some(to),
        &afters,
        &[],
    );
    let path = record_before(dir.path(), &rec).unwrap();
    assert_eq!(read_record(&path).unwrap().workloads[0].result, "pending");
    let results = vec![ItemResult {
        href: afters[0].0.href.clone(),
        name: afters[0].0.name.clone(),
        change: afters[0].0.change.clone(),
        status: pce::write::ItemStatus::Updated,
        error: None,
    }];
    assert!(record_after(&path, &rec, &afters, &results).is_none());
    let back = read_record(&path).unwrap();
    assert_eq!(back.workloads[0].result, "updated");
    assert_eq!(
        back.workloads[0].before.mode.as_deref(),
        Some("visibility_only")
    );
    // A place that cannot take a file: refused, nothing written.
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    assert!(probe(&file).unwrap_err().contains("nothing sent"));
    // The rewrite of a record whose directory is gone: loud, with the record.
    let gone = dir.path().join("gone").join("r.json");
    let loud = record_after(&gone, &rec, &afters, &results).unwrap();
    assert!(loud.starts_with("CHANGE RECORD NOT UPDATED"), "{loud}");
    assert!(loud.contains("a rollback can still use it"), "{loud}");
    assert!(loud.contains("\"updated\""), "the whole record: {loud}");
    // Saved beside the records' directory instead.
    let saved = pce::enforcement::unsaved_path(&gone);
    assert!(loud.contains(&saved.display().to_string()), "{loud}");
    assert_eq!(read_record(&saved).unwrap().workloads[0].result, "updated");
}
