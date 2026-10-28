//! #86: `I` in the pending overlay and `:impact`, the provision impact
//! overlay. It loads once per open on the user's key (through the cache),
//! reopening shows the loaded summary and sends nothing, `r` loads it
//! again past the cache; nothing runs on a timer.
use super::*;
use crate::tui::commands;
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{impact_fixture, model_fixture};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn press(m: &mut Model, code: KeyCode) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn impact(m: &mut Model) -> Vec<Cmd> {
    update(m, Msg::Command(commands::parse("impact").unwrap()))
}

/// The one load a command or key asked for, with its cache mode.
fn one_load(cmds: &[Cmd]) -> CacheMode {
    match cmds {
        [Cmd::LoadImpact(spec)] => {
            assert_eq!(spec.keys, ["app", "env"]);
            assert_eq!(spec.query.max_results, 1000);
            assert_eq!(spec.requested, None);
            spec.mode
        }
        other => panic!("expected one LoadImpact, got {other:?}"),
    }
}

#[test]
fn the_command_loads_once_and_a_reopen_sends_nothing() {
    let mut m = model_fixture();
    assert_eq!(one_load(&impact(&mut m)), CacheMode::Use);
    assert_eq!(m.overlay, Overlay::Impact);
    assert!(m.policy.impact.loading);
    // Asked again while it loads: nothing more.
    assert!(impact(&mut m).is_empty());
    update(&mut m, Msg::ImpactLoaded(Ok(impact_fixture())));
    let v = &m.policy.impact;
    assert!(!v.loading && v.loaded_at.is_some() && v.report.is_some());
    assert!(
        m.status
            .text
            .contains("1 flow newly blocked · 1 newly allowed"),
        "{}",
        m.status.text
    );
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
    // Reopened: the loaded summary, no request.
    assert!(impact(&mut m).is_empty());
    assert_eq!(m.overlay, Overlay::Impact);
    assert!(m.policy.impact.report.is_some());
}

#[test]
fn r_reloads_past_the_cache_and_a_failure_keeps_the_last_summary() {
    let mut m = model_fixture();
    impact(&mut m);
    update(&mut m, Msg::ImpactLoaded(Ok(impact_fixture())));
    assert_eq!(
        one_load(&press(&mut m, KeyCode::Char('r'))),
        CacheMode::Bypass
    );
    assert!(
        press(&mut m, KeyCode::Char('r')).is_empty(),
        "one at a time"
    );
    update(&mut m, Msg::ImpactLoaded(Err("HTTP 503".into())));
    let v = &m.policy.impact;
    assert_eq!(v.error.as_deref(), Some("HTTP 503"));
    assert!(v.report.is_some(), "the last summary stays");
    assert!(m.status.text.contains("HTTP 503"));
    assert!(v.lines()[0].1.contains("impact failed: HTTP 503"));
}

#[test]
fn i_in_the_pending_overlay_opens_it_and_esc_goes_back() {
    let mut m = model_fixture();
    m.screen = Screen::Policy;
    m.policy.pending = Some(Vec::new());
    m.overlay = Overlay::Pending;
    assert_eq!(one_load(&press(&mut m, KeyCode::Char('I'))), CacheMode::Use);
    assert_eq!(m.overlay, Overlay::Impact);
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::Pending);
    press(&mut m, KeyCode::Char('I'));
    press(&mut m, KeyCode::Char('q'));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn scrolling_stops_at_the_last_line() {
    let mut m = model_fixture();
    impact(&mut m);
    update(&mut m, Msg::ImpactLoaded(Ok(impact_fixture())));
    let last = m.policy.impact.lines().len() as u16 - 1;
    press(&mut m, KeyCode::End);
    assert_eq!(m.policy.impact.scroll, last);
    press(&mut m, KeyCode::Down);
    assert_eq!(m.policy.impact.scroll, last);
    press(&mut m, KeyCode::Home);
    assert_eq!(m.policy.impact.scroll, 0);
    press(&mut m, KeyCode::Up);
    assert_eq!(m.policy.impact.scroll, 0);
}

#[test]
fn a_window_that_needs_confirming_sends_nothing() {
    let mut m = model_fixture();
    m.limits.traffic_default_window = "30d".into();
    assert!(impact(&mut m).is_empty());
    assert_eq!(m.overlay, Overlay::Impact);
    assert!(!m.policy.impact.loading);
    let e = m.policy.impact.error.clone().unwrap();
    assert!(e.contains("--yes"), "{e}");
}

#[test]
fn the_report_lines_name_every_section() {
    let lines: Vec<String> = crate::tui::provision_view::report_lines(&impact_fixture(), 20)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    let text = lines.join("\n");
    for s in [
        "Changes · 3 pending changes",
        "Rules · 2 rulesets · 1 added · 0 removed · 1 changed",
        "    ~ allow role=web ← iplist:Corp",
        "      was allow role=web ← role=db",
        "      providers 1→1 · consumers 1→0",
        "Traffic · 1d · 4 flow records · 1 newly blocked · 1 newly allowed",
        "Newly blocked or potentially blocked · 1 pair",
        "Newly allowed · 1 pair",
        "Not counted",
    ] {
        assert!(text.contains(s), "{s:?} missing:\n{text}");
    }
    let none = crate::provision_impact::ProvisionImpact::nothing_pending();
    assert_eq!(
        crate::tui::provision_view::report_lines(&none, 20)[0].1,
        "nothing pending: provisioning would change nothing"
    );
}

#[test]
fn a_write_forgets_the_summary_and_the_next_open_loads_it_again() {
    let mut m = model_fixture();
    impact(&mut m);
    update(&mut m, Msg::ImpactLoaded(Ok(impact_fixture())));
    press(&mut m, KeyCode::Esc);
    update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                pending: true,
                status: 201,
                object: serde_json::json!({}),
                dropped_fields: Vec::new(),
                warnings: Vec::new(),
                audit_failed: None,
            })),
        },
    );
    assert!(m.policy.impact.report.is_none());
    assert_eq!(one_load(&impact(&mut m)), CacheMode::Use);
}
