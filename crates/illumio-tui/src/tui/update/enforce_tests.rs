//! `:readiness` and the enforcement overlay's keys (#55, #56).
use super::*;
use crate::tui::enforce::tests::{all_allowed, loaded, write_move};
use crate::tui::enforce::{
    ENFORCE_WORD, EnforceOutcome, EnforceView, Missing, RollbackOutcome, WatchOutcome,
};
use crate::tui::model::WritesState;
use crate::tui::test_support::{model_with_window, with_draft_loaded};
use pce::models::PolicyDecision;
use pce::write::bulk::{BulkStep, ItemResult, ItemStatus, StepStatus};
use pce::write::{BulkReport, ObjectKind as WriteKind};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn typed(m: &mut Model, s: &str) {
    for c in s.chars() {
        quiet(m, key(c));
    }
}

#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

fn command(m: &mut Model, s: &str) -> Vec<Cmd> {
    update(m, Msg::Command(commands::parse(s).unwrap()))
}

fn ev(m: &Model) -> &EnforceView {
    m.enforce_view.as_ref().expect("the overlay's state")
}

/// `m` with its records in a temp dir and writes on, the overlay open on
/// shop/prod (no request).
fn on_shop(mut m: Model, dir: &std::path::Path) -> Model {
    m.records_dir = Some(dir.to_path_buf());
    m.writes = WritesState::on();
    assert_eq!(command(&mut m, "enforce shop/prod"), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::Enforce);
    m
}

fn shop_hrefs() -> Vec<String> {
    vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]
}

fn report(items: &[(&str, ItemStatus)]) -> BulkReport {
    BulkReport {
        target: "demo (pce.example, org 5)".into(),
        change: "enforcement full".into(),
        refused: None,
        reads: 1,
        reread_skipped: None,
        steps: vec![BulkStep {
            method: WriteMethod::Put,
            path: "/orgs/5/workloads/bulk_update".into(),
            summary: "bulk update".into(),
            status: StepStatus::Written,
            href: None,
            http_status: Some(200),
            error: None,
            audit_failed: None,
        }],
        items: items
            .iter()
            .map(|(h, s)| ItemResult {
                href: (*h).into(),
                name: "host-2.example".into(),
                change: "visibility_only → full".into(),
                status: *s,
                error: None,
            })
            .collect(),
        labels_changed: false,
        labels_created: false,
        failure: None,
    }
}

#[test]
fn readiness_costs_nothing_when_everything_is_loaded() {
    let mut m = loaded();
    assert_eq!(command(&mut m, "readiness"), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::Readiness);
    let v = m.readiness_view.as_ref().unwrap();
    assert_eq!(v.rows.len(), 8);
    assert_eq!(v.requests, 0);
    assert!(m.status.text.ends_with("0 req"), "{}", m.status.text);
    // Keys within it send nothing.
    for c in ['j', 'k', 's', 's'] {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, key('/'));
    typed(&mut m, "shop");
    quiet(&mut m, kc(KeyCode::Enter));
    let v = m.readiness_view.as_ref().unwrap();
    assert_eq!(v.shown().len(), 2);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn readiness_loads_what_is_missing_once_and_counts_it() {
    let mut m = with_draft_loaded(model_with_window("24h"));
    let cmds = command(&mut m, "readiness");
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadWorkloads(CacheMode::Use),
            Cmd::LoadPending(CacheMode::Use),
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use
            },
        ]
    );
    assert_eq!(m.readiness_view.as_ref().unwrap().requests, 3);
    // Reopened while they are on their way: nothing more.
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(command(&mut m, "readiness"), Vec::<Cmd>::new());
    // Without traffic: nothing opens.
    let mut empty = crate::tui::test_support::model_fixture();
    assert_eq!(command(&mut empty, "readiness"), Vec::<Cmd>::new());
    assert_eq!(empty.overlay, Overlay::None);
    assert!(empty.status.text.starts_with("no traffic loaded"));
}

#[test]
fn d_offers_the_draft_through_its_own_confirm_and_comes_back() {
    let mut m = loaded();
    // Draft loaded: D never clears it from here.
    quiet(&mut m, Msg::Command(Command::Readiness));
    quiet(&mut m, key('D'));
    assert!(m.traffic.draft_loaded());
    assert!(m.status.text.starts_with("draft decisions are loaded"));
    // Not evaluated: the result has no query of this session: D asks.
    let mut m = crate::tui::test_support::with_policy(crate::tui::test_support::with_workloads(
        model_with_window("24h"),
    ));
    m.policy.pending = Some(Vec::new());
    quiet(&mut m, Msg::Command(Command::Readiness));
    assert!(
        m.readiness_view
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .all(|r| r.verdict == pce::readiness::Verdict::NotEvaluated)
    );
    quiet(&mut m, key('D'));
    assert!(matches!(
        m.overlay,
        Overlay::Confirm {
            action: PendingAction::RunDraft { .. },
            ..
        }
    ));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunDraft { .. }]),
        "{cmds:?}"
    );
    assert_eq!(m.overlay, Overlay::Readiness, "back to the list");
    // `n` comes back too.
    m.traffic.cancel_draft_run();
    quiet(&mut m, key('D'));
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::Readiness);
}

#[test]
fn enter_opens_the_app_and_esc_returns_to_the_list_without_a_request() {
    let mut m = loaded();
    quiet(&mut m, Msg::Command(Command::Readiness));
    // shop/prod sorts first (most would-block).
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Enforce);
    assert_eq!(ev(&m).app_text(), "shop/prod");
    assert_eq!(ev(&m).requests, 0);
    for c in ['2', '3', '4', '5', '1', 'j', 'k'] {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, kc(KeyCode::Tab));
    assert_eq!(ev(&m).step, crate::tui::enforce::Step::Rules);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Readiness);
}

#[test]
fn enforce_finds_the_app_by_name_or_offers_the_picker() {
    let mut m = loaded();
    quiet(
        &mut m,
        Msg::Command(Command::Enforce(Some("shop/prdo".into()))),
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        m.status.text.contains("closest: shop/prod"),
        "{}",
        m.status.text
    );
    quiet(&mut m, Msg::Command(Command::Enforce(None)));
    let Overlay::AppPicker(p) = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert_eq!(p.purpose, AppPickerPurpose::Enforce);
    typed(&mut m, "shop/prod");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Enforce);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn e_in_the_app_overlay_opens_it_and_esc_goes_back() {
    let mut m = loaded();
    quiet(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    assert_eq!(m.overlay, Overlay::App);
    quiet(&mut m, key('e'));
    assert_eq!(m.overlay, Overlay::Enforce);
    quiet(&mut m, key('?'));
    assert_eq!(
        crate::tui::help::HelpContext::of(&m),
        crate::tui::help::HelpContext::Enforce
    );
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::App);
}

#[test]
fn a_not_ready_app_is_refused_until_enforce_is_typed() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('4'));
    // Writes off: nothing, not even the prompt.
    m.writes = WritesState::off("writes off; :writes on to enable");
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(ev(&m).typing.is_none());
    m.writes = WritesState::on();
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(ev(&m).typing.as_deref(), Some(""));
    assert!(m.status.text.contains("is not ready (needs rules)"));
    // Letters are the word here, not keys: `r`, `e` do nothing else.
    typed(&mut m, "enf");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Enforce, "a wrong word sends nothing");
    assert!(m.status.text.starts_with("type enforce"));
    typed(&mut m, "orce");
    quiet(&mut m, kc(KeyCode::Enter));
    let Overlay::ConfirmBulk(c) = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert_eq!(c.plan.hrefs(), vec!["/orgs/5/workloads/aaaa-2".to_string()]);
    let send = m.enforce_send.as_ref().expect("planned by the overlay");
    assert!(send.readiness.forced);
    assert_eq!(send.readiness.verdict, "needs_rules");
    assert_eq!(send.target.text(), "full");
    // `Esc` on the confirm: back to the overlay, the plan forgotten.
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce);
    assert!(m.enforce_send.is_none());
    // Esc on the prompt cancels it.
    quiet(&mut m, kc(KeyCode::Enter));
    quiet(&mut m, kc(KeyCode::Esc));
    assert!(ev(&m).typing.is_none());
}

#[test]
fn a_ready_app_goes_to_the_bulk_confirm_and_y_sends_it_with_a_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(all_allowed(), dir.path());
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(
        matches!(m.overlay, Overlay::ConfirmBulk(_)),
        "no word asked"
    );
    assert!(!m.enforce_send.as_ref().unwrap().readiness.forced);
    let cmds = update(&mut m, key('y'));
    let [Cmd::EnforceApp { plan, send }] = cmds.as_slice() else {
        panic!("{cmds:?}");
    };
    assert_eq!(plan.hrefs(), vec!["/orgs/5/workloads/aaaa-2".to_string()]);
    assert_eq!(send.dir, dir.path());
    assert_eq!(send.app, "shop/prod");
    assert!(m.writes.in_flight && m.batch_sending);
    assert_eq!(m.overlay, Overlay::None, "while it is sent");
    // The answer: the record (written by the runtime) is read, the
    // overlay comes back on Watch, the workloads reload.
    let path = write_move(dir.path(), "pce.example", "2026-10-19T10:00:00Z");
    let cmds = update(
        &mut m,
        Msg::EnforceDone(Box::new(EnforceOutcome {
            app: "shop/prod".into(),
            report: report(&[("/orgs/5/workloads/aaaa-2", ItemStatus::Updated)]),
            record: Some(path.clone()),
            record_error: None,
        })),
    );
    assert_eq!(cmds, vec![Cmd::LoadWorkloads(CacheMode::Bypass)]);
    assert_eq!(m.overlay, Overlay::Enforce);
    assert_eq!(ev(&m).step, crate::tui::enforce::Step::Watch);
    assert_eq!(ev(&m).record.as_ref().map(|(p, _)| p), Some(&path));
    assert!(
        m.status.text.contains("record shop_prod-"),
        "{}",
        m.status.text
    );
    assert!(!m.writes.in_flight);
}

#[test]
fn a_record_that_could_not_be_updated_opens_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(all_allowed(), dir.path());
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    update(&mut m, key('y'));
    update(
        &mut m,
        Msg::EnforceDone(Box::new(EnforceOutcome {
            app: "shop/prod".into(),
            report: report(&[("/orgs/5/workloads/aaaa-2", ItemStatus::Updated)]),
            record: Some(dir.path().join("x.json")),
            record_error: Some(format!(
                "CHANGE RECORD NOT UPDATED: x.json …\nsaved as /state/x.unsaved.json\n\n{}",
                "{\n  \"app\": \"shop/prod\",\n".repeat(200)
            )),
        })),
    );
    // The whole record, in the scrollable report viewer, before the send's report.
    assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
    let lines = m.batch_report.as_ref().unwrap();
    assert!(lines[0].starts_with("CHANGE RECORD NOT UPDATED"));
    assert!(lines.len() > 400, "nothing cut: {}", lines.len());
    assert!(
        lines.iter().any(|l| l.starts_with("sent to")),
        "the send's report follows"
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    quiet(&mut m, kc(KeyCode::Down));
    assert_eq!(m.overlay, Overlay::BatchReport { scroll: 1 });
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce, "back once read");
}

/// A result that may miss the app's flows: `ready` is not trusted, the
/// move is typed through and recorded as forced.
#[test]
fn a_ready_verdict_on_a_partial_result_is_typed_through() {
    type Cause = Box<dyn Fn(&mut Model)>;
    let causes: Vec<(&str, Cause)> = vec![
        (
            "the traffic filter narrows",
            Box::new(|m: &mut Model| {
                let d = m.traffic.data.as_mut().unwrap();
                d.filter.decisions = vec![PolicyDecision::Allowed];
            }),
        ),
        (
            "hit its cap",
            Box::new(|m: &mut Model| m.traffic.data.as_mut().unwrap().truncated = true),
        ),
        (
            "did not answer",
            Box::new(|m: &mut Model| m.traffic.data.as_mut().unwrap().incomplete = true),
        ),
        (
            "focused on crm/prod",
            Box::new(|m: &mut Model| {
                let keys = vec!["app".to_string(), "env".to_string()];
                let d = crate::tui::model::TrafficData {
                    flows: m.traffic.loaded_flows().to_vec(),
                    truncated: false,
                    incomplete: false,
                    elapsed: std::time::Duration::ZERO,
                    cache_hit: false,
                    fetched_at: std::time::Instant::now(),
                    filter: m.traffic.filter.clone(),
                    range: None,
                    hrefs: Vec::new(),
                    draft: m.traffic.data.as_ref().unwrap().draft,
                };
                m.traffic.apply_focused(
                    Focus::new(pce::traffic::parse_app("crm/prod"), keys),
                    1,
                    d,
                );
            }),
        ),
        (
            "no flow of the app",
            Box::new(|m: &mut Model| {
                let d = m.traffic.data.as_mut().unwrap();
                d.flows.retain(|f| {
                    let k = vec!["app".to_string(), "env".to_string()];
                    pce::traffic::group_key(&f.src, &k) != pce::traffic::parse_app("shop/prod")
                        && pce::traffic::group_key(&f.dst, &k)
                            != pce::traffic::parse_app("shop/prod")
                });
            }),
        ),
    ];
    for (why, make) in causes {
        let dir = tempfile::tempdir().unwrap();
        let mut base = all_allowed();
        make(&mut base);
        base.traffic.rebuild_views();
        let mut m = on_shop(base, dir.path());
        let v = ev(&m);
        assert!(
            v.partial.iter().any(|p| p.contains(why)),
            "{why}: {:?}",
            v.partial
        );
        assert_eq!(
            v.step_state(crate::tui::enforce::Step::Enforce, &Missing::default()),
            if v.readiness.as_ref().unwrap().verdict.allows_enforcement() {
                "partial"
            } else {
                "blocked"
            },
            "{why}"
        );
        quiet(&mut m, key('4'));
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(ev(&m).typing.as_deref(), Some(""), "{why}: the word first");
        typed(&mut m, ENFORCE_WORD);
        quiet(&mut m, kc(KeyCode::Enter));
        assert!(matches!(m.overlay, Overlay::ConfirmBulk(_)), "{why}");
        assert!(m.enforce_send.as_ref().unwrap().readiness.forced, "{why}");
    }
}

#[test]
fn the_typed_move_checks_again_what_readiness_reads() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    typed(&mut m, ENFORCE_WORD);
    // The pending list goes (a write dropped it) while the word is typed.
    m.policy.pending = None;
    m.policy.pending_loading = true;
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Enforce);
    assert!(m.enforce_send.is_none());
    assert!(
        m.status.text.starts_with("readiness is not complete"),
        "{}",
        m.status.text
    );
}

#[test]
fn answers_land_only_in_the_overlay_that_asked() {
    let dir = tempfile::tempdir().unwrap();
    write_move(dir.path(), "pce.example", "2026-10-19T10:00:00Z");
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('5'));
    update(&mut m, key('R'));
    let seq = ev(&m).seq;
    // Closed meanwhile: the re-read's answer is dropped, reopening is clean.
    quiet(&mut m, kc(KeyCode::Esc));
    let wl = m.workloads.raw.clone();
    quiet(
        &mut m,
        Msg::RollbackRead {
            seq,
            result: Ok((wl.clone(), false)),
        },
    );
    quiet(
        &mut m,
        Msg::Command(Command::Enforce(Some("shop/prod".into()))),
    );
    assert!(ev(&m).rollback.is_none() && !ev(&m).rollback_reading);
    // A move of another app does not touch this overlay.
    m.writes.in_flight = true;
    let before = ev(&m).step;
    update(
        &mut m,
        Msg::EnforceDone(Box::new(EnforceOutcome {
            app: "crm/prod".into(),
            report: report(&[("/orgs/5/workloads/aaaa-2", ItemStatus::Updated)]),
            record: Some(dir.path().join("crm_prod-x.json")),
            record_error: None,
        })),
    );
    assert_eq!(ev(&m).step, before);
}

#[test]
fn a_command_or_a_screen_switch_keeps_the_overlay_from_coming_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('2'));
    update(&mut m, key('G'));
    assert_eq!(m.overlay, Overlay::Generator);
    assert_eq!(m.enforce_back, Some(Overlay::Enforce));
    update(
        &mut m,
        Msg::Command(Command::Screen(crate::tui::model::Screen::Policy)),
    );
    assert!(m.enforce_back.is_none());
    m.overlay = Overlay::None;
    quiet(&mut m, Msg::Tick(std::time::Instant::now()));
    assert_eq!(m.overlay, Overlay::None);
    // A screen key too.
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('2'));
    update(&mut m, key('G'));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce, "back from the generator");
    update(&mut m, key('G'));
    m.overlay = Overlay::None;
    update(
        &mut m,
        Msg::SwitchScreen(crate::tui::model::Screen::Workloads),
    );
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn reopening_after_the_help_asks_for_nothing_and_a_replaced_watch_is_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    // A write dropped the pending list: only a write's answer asks again.
    m.policy.pending = None;
    quiet(&mut m, key('?'));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce);
    // The watch: a second start cancels the first.
    let now = jiff::Timestamp::now();
    let at = jiff::Timestamp::from_second(now.as_second() - 600).unwrap();
    write_move(dir.path(), "pce.example", &at.to_string());
    m.policy.pending = Some(Vec::new());
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(
        &mut m,
        Msg::Command(Command::Enforce(Some("shop/prod".into()))),
    );
    quiet(&mut m, key('5'));
    let first = update(&mut m, key('r'));
    let [Cmd::WatchBlocked { cancel, .. }] = first.as_slice() else {
        panic!("{first:?}");
    };
    let second = start_watch(&mut m);
    assert!(cancel.is_cancelled(), "the first query is cancelled");
    assert!(matches!(second.as_slice(), [Cmd::WatchBlocked { .. }]));
}

#[test]
fn the_move_waits_for_what_readiness_reads() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(all_allowed(), dir.path());
    m.policy.pending = None;
    m.policy.pending_loading = true;
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Enforce);
    assert!(m.status.text.starts_with("readiness is not complete"));
    // No place for records: refused before the confirm.
    let mut m = on_shop(all_allowed(), dir.path());
    m.records_dir = None;
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::Enforce);
    assert!(m.status.text.contains("change records; nothing sent"));
}

#[test]
fn step_two_opens_the_rule_form_the_mark_and_the_generator_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('2'));
    // `w`: the rule form for the first gap's flow.
    update(&mut m, key('w'));
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert!(matches!(f.intent, EditIntent::RuleFromFlow(_)));
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce);
    // `m`: accept-blocked for the gap.
    quiet(&mut m, key('m'));
    let Overlay::MarkForm(mf) = &m.overlay else {
        panic!("{:?}", m.overlay);
    };
    assert_eq!(mf.form.value("Action"), "accept-blocked");
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce);
    // `G`: the generator for the app.
    update(&mut m, key('G'));
    assert_eq!(m.overlay, Overlay::Generator);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Enforce);
    // The keys are step 2's only.
    quiet(&mut m, key('1'));
    quiet(&mut m, key('w'));
    assert_eq!(m.overlay, Overlay::Enforce);
}

#[test]
fn step_three_provisions_the_apps_pending_changes_through_the_preview() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('3'));
    let cmds = update(&mut m, key('P'));
    assert_eq!(
        cmds,
        vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
            description: Some(PROVISION_DESCRIPTION.into()),
            subset: ProvisionSubset::Hrefs(vec!["/orgs/5/sec_policy/draft/ip_lists/21".into()]),
        }))]
    );
    assert_eq!(m.overlay, Overlay::None);
    // The preview opens over nothing; its cancel comes back.
    update(
        &mut m,
        Msg::Planned(Ok((
            crate::tui::test_support::provision_plan(),
            WriteKind::Provision,
        ))),
    );
    assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::Enforce);
}

#[test]
fn watch_queries_once_on_r_and_only_with_a_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('5'));
    quiet(&mut m, key('r'));
    assert!(m.status.text.starts_with("no change record"));
    let now = jiff::Timestamp::now();
    let at = jiff::Timestamp::from_second(now.as_second() - 3600).unwrap();
    write_move(dir.path(), "pce.example", &at.to_string());
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(
        &mut m,
        Msg::Command(Command::Enforce(Some("shop/prod".into()))),
    );
    let cmds = update(&mut m, key('r'));
    let [
        Cmd::WatchBlocked {
            seq, app, since, ..
        },
    ] = cmds.as_slice()
    else {
        panic!("{cmds:?}");
    };
    assert_eq!(app, &pce::traffic::parse_app("shop/prod"));
    assert_eq!(since, &at);
    assert_eq!(ev(&m).requests, 1);
    // A second `r` while it runs: nothing.
    quiet(&mut m, key('r'));
    // The answer: the blocked flows into the app, grouped.
    let mut flows: Vec<_> = m.traffic.loaded_flows().to_vec();
    for f in &mut flows {
        f.policy_decision = PolicyDecision::Blocked;
    }
    let seq = *seq;
    quiet(
        &mut m,
        Msg::WatchDone {
            seq: seq + 7,
            result: Err("stale".into()),
        },
    );
    assert!(matches!(ev(&m).watch, Watch::Running { .. }), "dropped");
    quiet(
        &mut m,
        Msg::WatchDone {
            seq,
            result: Ok(WatchOutcome {
                flows,
                truncated: false,
                incomplete: false,
                since: at,
            }),
        },
    );
    let Watch::Done(w) = &ev(&m).watch else {
        panic!("{:?}", ev(&m).watch);
    };
    assert_eq!(w.blocked.flows, 10);
    assert!(m.status.text.starts_with("watch: 10 flows"));
}

#[test]
fn a_watch_over_a_long_window_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    write_move(dir.path(), "pce.example", "2026-01-01T00:00:00Z");
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('5'));
    quiet(&mut m, key('r'));
    assert!(matches!(
        m.overlay,
        Overlay::Confirm {
            action: PendingAction::EnforceWatch,
            ..
        }
    ));
    let cmds = update(&mut m, key('y'));
    assert!(matches!(cmds.as_slice(), [Cmd::WatchBlocked { .. }]));
    assert_eq!(m.overlay, Overlay::Enforce);
}

#[test]
fn rollback_re_reads_previews_and_sends_on_y() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_move(dir.path(), "pce.example", "2026-10-19T10:00:00Z");
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('5'));
    let cmds = update(&mut m, key('R'));
    let seq = ev(&m).seq;
    assert_eq!(
        cmds,
        vec![Cmd::RollbackRead {
            seq,
            filter: pce::app::app_workload_filter(&shop_hrefs()),
            dir: dir.path().to_path_buf(),
        }]
    );
    // The workload is still as the move set it.
    let mut now = m.workloads.raw.clone();
    now[1].enforcement_mode = Some("full".into());
    quiet(
        &mut m,
        Msg::RollbackRead {
            seq,
            result: Ok((now.clone(), false)),
        },
    );
    let p = ev(&m).rollback.as_ref().expect("the preview");
    assert_eq!((p.count, p.skipped.len(), p.needs_count), (1, 0, false));
    assert_eq!(p.plans[0].0.text(), "visibility_only");
    let cmds = update(&mut m, key('y'));
    let [Cmd::ExecuteRollback(send)] = cmds.as_slice() else {
        panic!("{cmds:?}");
    };
    assert_eq!(send.record_path, path);
    assert_eq!(send.plans.len(), 1);
    assert!(ev(&m).rollback.is_none() && m.writes.in_flight);
    let cmds = update(
        &mut m,
        Msg::RollbackDone(Box::new(RollbackOutcome {
            app: "shop/prod".into(),
            reports: vec![report(&[("/orgs/5/workloads/aaaa-2", ItemStatus::Updated)])],
            record: None,
            record_error: None,
        })),
    );
    assert_eq!(cmds, vec![Cmd::LoadWorkloads(CacheMode::Bypass)]);
    assert!(!m.writes.in_flight && !m.batch_sending);
    assert!(m.status.text.starts_with("rollback: updated 1 of 1"));
    // Changed by someone else since: skipped, nothing to send.
    now[1].enforcement_mode = Some("selective".into());
    update(&mut m, key('R'));
    quiet(
        &mut m,
        Msg::RollbackRead {
            seq,
            result: Ok((now, false)),
        },
    );
    let p = ev(&m).rollback.as_ref().unwrap();
    assert_eq!((p.count, p.skipped.len()), (0, 1));
    quiet(&mut m, key('y'));
    assert!(m.status.text.starts_with("nothing to restore"));
    quiet(&mut m, kc(KeyCode::Esc));
    assert!(ev(&m).rollback.is_none());
}

#[test]
fn an_already_rolled_back_record_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_move(dir.path(), "pce.example", "2026-10-19T10:00:00Z");
    let mut rb = pce::enforcement::read_record(&path).unwrap();
    rb.rollback_of = Some(path.display().to_string());
    pce::enforcement::write_record(dir.path(), &rb, jiff::Timestamp::now()).unwrap();
    let mut m = on_shop(loaded(), dir.path());
    quiet(&mut m, key('5'));
    quiet(&mut m, key('R'));
    assert!(
        m.status.text.contains("was already rolled back by"),
        "{}",
        m.status.text
    );
    // Made on another org than the profile writes to now: refused too.
    let other = tempfile::tempdir().unwrap();
    write_move(other.path(), "pce.example", "2026-10-19T10:00:00Z");
    let mut m = on_shop(loaded(), other.path());
    m.profile.org_id = 6;
    quiet(&mut m, key('5'));
    quiet(&mut m, key('R'));
    assert!(
        m.status
            .text
            .contains("this profile writes to pce.example (org 6)"),
        "{}",
        m.status.text
    );
}

#[test]
fn opening_either_overlay_with_the_data_loaded_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = loaded();
    m.records_dir = Some(dir.path().to_path_buf());
    for c in [
        Command::Readiness,
        Command::Enforce(Some("shop/prod".into())),
        Command::Enforce(Some("crm/prod".into())),
    ] {
        quiet(&mut m, Msg::Command(c));
    }
    // Data landing while it is open: re-evaluated, nothing asked.
    let rs = m.policy.raw.clone();
    quiet(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, 0, true)),
        },
    );
    assert_eq!(ev(&m).app_text(), "crm/prod");
    assert_eq!(ev(&m).requests, 0);
}

/// Readiness counts unicast flows only: `:unicast` narrows nothing.
#[test]
fn a_unicast_result_is_not_partial() {
    let dir = tempfile::tempdir().unwrap();
    let mut base = all_allowed();
    base.traffic.data.as_mut().unwrap().filter.transmission =
        vec![pce::models::Transmission::Unicast];
    let mut m = on_shop(base, dir.path());
    assert!(ev(&m).partial.is_empty(), "{:?}", ev(&m).partial);
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(ev(&m).typing.is_none(), "no word asked");
    assert!(matches!(m.overlay, Overlay::ConfirmBulk(_)));
    assert!(!m.enforce_send.as_ref().unwrap().readiness.forced);
}

/// `y` checks readiness again: what landed since the review may require
/// the typed word; a typed (forced) move goes on.
#[test]
fn y_refuses_a_move_that_needs_the_word_since_the_review() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = on_shop(all_allowed(), dir.path());
    quiet(&mut m, key('4'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert!(matches!(m.overlay, Overlay::ConfirmBulk(_)));
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(&mut m, key('y'));
    assert_eq!(m.overlay, Overlay::Enforce);
    assert!(m.enforce_send.is_none() && !m.writes.in_flight);
    assert!(
        m.status.text.contains("partial result since the review"),
        "{}",
        m.status.text
    );
    // Typed through: sent.
    quiet(&mut m, kc(KeyCode::Enter));
    typed(&mut m, ENFORCE_WORD);
    quiet(&mut m, kc(KeyCode::Enter));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::EnforceApp { .. }]),
        "{cmds:?}"
    );
}
