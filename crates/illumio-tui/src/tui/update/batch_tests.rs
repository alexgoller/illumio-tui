use super::*;
use crate::tui::model::{Model, Overlay, StatusKind};
use crate::tui::test_support::{batch_plan, model_with_flows, with_draft_loaded};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::write::{ObjectKind, StepResult, Written};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn written(href: &str) -> StepResult {
    StepResult::Written(Written {
        href: href.into(),
        pending: true,
        status: 201,
        object: serde_json::json!({"href": href}),
        dropped_fields: Vec::new(),
        warnings: Vec::new(),
        audit_failed: None,
    })
}

fn kinds() -> Vec<ObjectKind> {
    vec![ObjectKind::IpList, ObjectKind::RuleSet, ObjectKind::Rule]
}

fn summaries() -> Vec<String> {
    vec![
        "create ip_list shop-prod peer 203.0.113.7".into(),
        "create rule_set RF-crm-prod".into(),
        "create rule in rule_set RF-crm-prod".into(),
    ]
}

/// Writes on, the draft rulesets loaded, the generator open for
/// shop/prod and planning.
fn armed() -> Model {
    let mut m = crate::tui::test_support::with_policy(model_with_flows());
    m.writes = crate::tui::model::WritesState::on();
    m.writes.in_flight = true; // the plan is on its way
    let focus = pce::traffic::Focus::new(
        pce::traffic::parse_app("shop/prod"),
        vec!["app".into(), "env".into()],
    );
    let mut v = crate::tui::gen_view::GenView::new(focus, "24h".into(), 1);
    v.planning = true;
    m.gen_view = Some(v);
    m.overlay = Overlay::Generator;
    m
}

/// `m` after its plan landed and `y` sent it: the confirm closed.
fn sent(mut m: Model) -> Model {
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    update(&mut m, key('y'));
    assert!(m.batch_sending && m.overlay == Overlay::None);
    m
}

#[test]
fn a_planned_batch_opens_its_confirm_with_every_request() {
    let mut m = armed();
    assert_eq!(
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan()))),
        Vec::new()
    );
    assert!(!m.writes.in_flight);
    let Overlay::ConfirmBatch { batch, scroll } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(*scroll, 0);
    assert_eq!(
        batch.summary(),
        "3 requests: 1 ip_list, 1 rule_set, 1 rule · planned with 2 reads"
    );
    assert_eq!(
        m.status.text,
        "review the requests: y sends them all, n cancels"
    );
}

#[test]
fn a_batch_landing_over_another_overlay_is_discarded() {
    let mut m = armed();
    m.overlay = Overlay::Prompt { input: "y".into() };
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    assert_eq!(m.overlay, Overlay::Prompt { input: "y".into() });
    assert_eq!(m.status.text, "requests discarded, nothing sent");
    assert!(!m.writes.in_flight);
}

#[test]
fn y_sends_exactly_one_execute_batch_and_n_sends_nothing() {
    let mut m = armed();
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    let cmds = update(&mut m, key('y'));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Cmd::ExecuteBatch(b) if b.steps.len() == 3));
    assert!(m.writes.in_flight);
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "sending 3 requests …");
    let mut m = armed();
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    assert_eq!(update(&mut m, key('n')), Vec::new());
    assert_eq!(m.overlay, Overlay::Generator, "back to the generator");
    assert_eq!(m.status.text, "cancelled, nothing sent");
}

#[test]
fn j_scrolls_the_confirm_and_stops_at_the_end() {
    let mut m = armed();
    m.size = (80, 12);
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    for _ in 0..100 {
        update(&mut m, key('j'));
    }
    let Overlay::ConfirmBatch { batch, scroll } = &m.overlay else {
        panic!()
    };
    let area = ratatui::layout::Rect::new(0, 0, 80, 12);
    assert_eq!(usize::from(*scroll), confirm_batch_last_scroll(batch, area));
    assert!(*scroll > 0);
}

#[test]
fn a_failed_plan_sends_nothing_and_says_why() {
    let mut m = armed();
    assert_eq!(
        update(
            &mut m,
            Msg::BatchPlanned(Err("too many draft rulesets".into()))
        ),
        Vec::new()
    );
    assert_eq!(m.status.text, "plan: too many draft rulesets");
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn a_partly_failed_batch_reports_each_step_and_reloads_once() {
    let mut m = sent(with_draft_loaded(armed()));
    // A known query: the draft is evaluated again without asking.
    m.traffic.data.as_mut().unwrap().hrefs = vec!["/orgs/5/traffic_flows/async_queries/q-1".into()];
    let results = vec![
        written("/orgs/5/sec_policy/draft/ip_lists/77"),
        StepResult::Failed(pce::write::WriteError::invalid("name in use")),
        StepResult::NotSent,
    ];
    let cmds = update(
        &mut m,
        Msg::BatchDone {
            kinds: kinds(),
            summaries: summaries(),
            target: "demo (pce.example, org 5)".into(),
            results,
        },
    );
    assert_eq!(
        m.status.text.split(" · ").next().unwrap(),
        "wrote 1 of 3 (pending); step 2 failed: name in use; 1 not sent"
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(!m.writes.in_flight);
    let count = |f: fn(&Cmd) -> bool| cmds.iter().filter(|c| f(c)).count();
    assert_eq!(
        count(|c| matches!(c, Cmd::LoadRuleSets { .. })),
        1,
        "{cmds:?}"
    );
    assert_eq!(
        count(|c| matches!(c, Cmd::LoadIpLists(CacheMode::Bypass))),
        1
    );
    assert_eq!(
        count(|c| matches!(c, Cmd::RunDraft { .. })),
        1,
        "one redraft per batch"
    );
    // The report stays up: which object went where, what did not.
    assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
    assert_eq!(
        m.batch_report.as_deref().unwrap(),
        [
            "sent to demo (pce.example, org 5)",
            "1/3 create ip_list shop-prod peer 203.0.113.7",
            "    written /orgs/5/sec_policy/draft/ip_lists/77",
            "2/3 create rule_set RF-crm-prod",
            "    FAILED: name in use",
            "3/3 create rule in rule_set RF-crm-prod",
            "    not sent",
            "",
            "These are draft changes, not provisioned: p on Policy lists them, R reverts the marked ones.",
            "Planning the batch again resumes it: what was written then exists.",
        ]
    );
    assert_eq!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        ),
        Vec::new()
    );
    assert_eq!(m.overlay, Overlay::Generator, "back to the generator");
    on_command(&mut m, crate::tui::commands::Command::Batch);
    assert_eq!(
        m.overlay,
        Overlay::BatchReport { scroll: 0 },
        ":batch reopens it"
    );
}

fn all_written() -> Vec<StepResult> {
    vec![
        written("/orgs/5/sec_policy/draft/ip_lists/77"),
        written("/orgs/5/sec_policy/draft/rule_sets/200"),
        written("/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"),
    ]
}

fn done(m: &mut Model, results: Vec<StepResult>) -> Vec<Cmd> {
    update(
        m,
        Msg::BatchDone {
            kinds: kinds(),
            summaries: summaries(),
            target: "demo (pce.example, org 5)".into(),
            results,
        },
    )
}

#[test]
fn quitting_waits_while_a_batch_is_sent() {
    let mut m = armed();
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    update(&mut m, key('y'));
    assert!(m.batch_sending);
    let ctrl_c = || Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(update(&mut m, ctrl_c()), Vec::new());
    assert_eq!(update(&mut m, key('q')), Vec::new());
    assert_eq!(
        on_command(&mut m, crate::tui::commands::Command::Quit),
        Vec::new()
    );
    assert!(!m.quit);
    assert_eq!(m.status.text, "batch sending: wait for it to finish");
    done(&mut m, all_written());
    assert!(!m.batch_sending);
    assert_eq!(update(&mut m, ctrl_c()), vec![Cmd::Quit]);
    assert!(m.quit);
}

#[test]
fn nothing_can_switch_or_clear_the_write_guard_while_a_batch_is_sent() {
    use crate::tui::commands::Command;
    let mut m = armed();
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    update(&mut m, key('y'));
    assert_eq!(
        on_command(&mut m, Command::Profile("prod-b".into())),
        Vec::new()
    );
    assert_eq!(m.status.text, "batch sending: wait for it to finish");
    assert!(!m.switching);
    assert_eq!(on_command(&mut m, Command::Writes(Some(false))), Vec::new());
    assert_eq!(m.status.text, "batch sending: wait for it to finish");
    // A plan landing now (from anywhere) leaves the batch's guard alone.
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    assert!(m.writes.in_flight, "still the batch's");
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(
        m.status.text,
        "plan discarded: a batch is being sent, nothing sent"
    );
    update(&mut m, Msg::Planned(Err("x".into())));
    assert!(m.writes.in_flight);
    update(&mut m, Msg::BatchProgress { done: 2, of: 3 });
    assert_eq!(m.status.text, "sending: 2 of 3 answered …");
    done(&mut m, all_written());
    assert!(!m.writes.in_flight && !m.batch_sending);
    assert_eq!(
        on_command(&mut m, Command::Profile("prod-b".into())),
        vec![Cmd::SwitchProfile("prod-b".into())]
    );
}

#[test]
fn a_batch_planned_on_another_profile_or_during_a_switch_is_discarded() {
    let mut m = armed();
    m.profile.name = "prod-b".into();
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    assert_eq!(m.overlay, Overlay::Generator, "no confirm");
    assert_eq!(
        m.status.text,
        "requests discarded: planned on another profile, nothing sent"
    );
    let mut m = armed();
    m.switching = true;
    update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
    assert_eq!(m.overlay, Overlay::Generator, "no confirm");
    let mut m = armed();
    let mut b = batch_plan();
    b.origin = None;
    update(&mut m, Msg::BatchPlanned(Ok(b)));
    assert_eq!(m.overlay, Overlay::Generator, "no origin: never confirmed");
}

#[test]
fn a_clean_batch_keeps_its_report_for_batch_and_an_open_overlay_stays() {
    let mut m = sent(armed());
    done(&mut m, all_written());
    assert_eq!(m.overlay, Overlay::Generator, "the generator reopens");
    assert!(
        m.status.text.contains(":batch shows each step"),
        "{}",
        m.status.text
    );
    assert!(m.batch_report.is_some());
    // Help opened while the batch was sent is not closed by its answer.
    let mut m = sent(armed());
    m.overlay = Overlay::Help;
    done(
        &mut m,
        vec![
            StepResult::Failed(pce::write::WriteError::invalid("bad")),
            StepResult::NotSent,
            StepResult::NotSent,
        ],
    );
    assert_eq!(m.overlay, Overlay::Help);
    assert!(m.status.text.contains(":batch shows each step"));
}

#[test]
fn a_step_written_but_not_audited_stops_and_says_so() {
    let mut m = sent(armed());
    let StepResult::Written(mut w) = written("/orgs/5/sec_policy/draft/ip_lists/77") else {
        unreachable!()
    };
    let why = "audit entry could not be written to audit.jsonl: disk full".to_string();
    w.warnings = vec![why.clone()];
    w.audit_failed = Some(why);
    done(
        &mut m,
        vec![
            StepResult::Written(w),
            StepResult::NotSent,
            StepResult::NotSent,
        ],
    );
    assert!(
        m.status.text.starts_with(
            "wrote 1 of 3 (pending); step 1 written but not audited (audit entry could not be written to audit.jsonl: disk full): batch stopped; 2 not sent"
        ),
        "{}",
        m.status.text
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
    assert!(m.batch_report.as_ref().unwrap()[2].contains("NOT AUDITED"));
}

#[test]
fn an_unanswered_step_says_it_may_have_applied_and_reloads() {
    let mut m = armed();
    let results = vec![
        StepResult::Failed(pce::write::WriteError::Unanswered(pce::PceError::Timeout(
            std::time::Duration::from_secs(30),
        ))),
        StepResult::NotSent,
        StepResult::NotSent,
    ];
    let cmds = update(
        &mut m,
        Msg::BatchDone {
            kinds: kinds(),
            summaries: summaries(),
            target: "demo (pce.example, org 5)".into(),
            results,
        },
    );
    assert!(
        m.status
            .text
            .starts_with("wrote 0 of 3 (pending); step 1 not answered: it may have applied; reloading; 2 not sent"),
        "{}",
        m.status.text
    );
    assert!(cmds.iter().any(|c| matches!(c, Cmd::LoadRuleSets { .. })));
}

#[test]
fn what_the_pce_dropped_from_a_written_step_is_a_warning() {
    let mut m = armed();
    let StepResult::Written(mut w) = written("/orgs/5/sec_policy/draft/ip_lists/77") else {
        unreachable!()
    };
    w.dropped_fields = vec!["fqdns".into()];
    let results = vec![
        StepResult::Written(w),
        written("/orgs/5/sec_policy/draft/rule_sets/200"),
        written("/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"),
    ];
    update(
        &mut m,
        Msg::BatchDone {
            kinds: kinds(),
            summaries: summaries(),
            target: "demo (pce.example, org 5)".into(),
            results,
        },
    );
    assert_eq!(
        m.status.text.split(" · ").next().unwrap(),
        "wrote 3 of 3 (pending); step 1: the PCE dropped fqdns"
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
}

#[test]
fn a_batch_that_failed_first_reloads_nothing() {
    let mut m = armed();
    let results = vec![
        StepResult::Failed(pce::write::WriteError::invalid("bad")),
        StepResult::NotSent,
        StepResult::NotSent,
    ];
    let cmds = update(
        &mut m,
        Msg::BatchDone {
            kinds: kinds(),
            summaries: summaries(),
            target: "demo (pce.example, org 5)".into(),
            results,
        },
    );
    assert_eq!(cmds, Vec::new());
}
