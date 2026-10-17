//! The sectioned views' speed budget at 200 000 flows: grouping the Flows
//! view by app (`x`), one search keystroke there, a fold (`Space`), the
//! section order (`s`), and App-to-app's sections not paying for the Flows
//! ones. Release only (`#[ignore]`); run it with `cargo test --release -p
//! illumio-tui --test sections_perf -- --ignored --nocapture`, which
//! prints each time. `SECTIONS_PERF_BUDGET_MS` (default 100) is the budget
//! per key.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use illumio_tui::tui::model::{Model, TrafficView};
use illumio_tui::tui::msg::Msg;
use illumio_tui::tui::{test_support, update};
use pce::models::PolicyDecision;
use pce::resolve::ResolvedLabel;
use pce::traffic::{ResolvedEndpoint, ResolvedFlow};
use std::time::{Duration, Instant};

fn endpoint(i: usize) -> ResolvedEndpoint {
    let label = |k: &str, v: String| ResolvedLabel {
        href: format!("/orgs/1/labels/{k}{v}"),
        key: k.into(),
        value: v,
    };
    ResolvedEndpoint {
        ip: Some(format!("10.{}.{}.{}", i % 200, (i / 200) % 250, i % 250)),
        hostname: Some(format!("host-{}", i % 5000)),
        workload_href: Some(format!("/orgs/1/workloads/{}", i % 5000)),
        labels: vec![
            label("app", format!("app{}", i % 50)),
            label("env", ["prod", "dev", "test"][i % 3].into()),
            label("role", format!("role{}", i % 7)),
        ],
        ..Default::default()
    }
}

fn flows(n: usize) -> Vec<ResolvedFlow> {
    (0..n)
        .map(|i| ResolvedFlow {
            src: endpoint(i),
            dst: endpoint(i * 7 + 3),
            port: Some(1000 + (i % 3000) as u16),
            proto: if i % 5 == 0 { "udp" } else { "tcp" }.into(),
            process_name: None,
            user_name: None,
            num_connections: 1 + (i % 17) as u64,
            policy_decision: PolicyDecision::ALL[i % 3],
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        })
        .collect()
}

fn key(m: &mut Model, code: KeyCode) -> Duration {
    let t = Instant::now();
    let cmds = update(m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    assert!(cmds.is_empty(), "no request: {cmds:?}");
    t.elapsed()
}

#[test]
#[ignore = "release-mode speed budget; run with --release -- --ignored"]
fn sections_stay_fast_at_200k_flows() {
    let budget = Duration::from_millis(
        std::env::var("SECTIONS_PERF_BUDGET_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100),
    );
    let mut m = test_support::model_with_rich_flows();
    m.traffic.data.as_mut().unwrap().flows = flows(200_000);
    m.traffic.rebuild_views();
    assert_eq!(m.traffic.view, TrafficView::Flows);
    let mut times: Vec<(&str, Duration)> = Vec::new();
    times.push(("Flows x (by src app)", key(&mut m, KeyCode::Char('x'))));
    assert!(m.traffic.flows_grouped.len() > 50);
    times.push(("Flows Space (fold)", key(&mut m, KeyCode::Char(' '))));
    times.push(("Flows s (order)", key(&mut m, KeyCode::Char('s'))));
    times.push(("Flows z (fold all)", key(&mut m, KeyCode::Char('z'))));
    times.push(("Flows z (unfold all)", key(&mut m, KeyCode::Char('z'))));
    key(&mut m, KeyCode::Char('/'));
    times.push(("Flows search keystroke", key(&mut m, KeyCode::Char('q'))));
    times.push(("Flows search keystroke 2", key(&mut m, KeyCode::Char('q'))));
    key(&mut m, KeyCode::Esc);
    times.push(("Flows x (by dst app)", key(&mut m, KeyCode::Char('x'))));
    key(&mut m, KeyCode::Char('v'));
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    times.push(("App-to-app x (in)", key(&mut m, KeyCode::Char('x'))));
    times.push(("App-to-app Space", key(&mut m, KeyCode::Char(' '))));
    times.push(("App-to-app s", key(&mut m, KeyCode::Char('s'))));
    key(&mut m, KeyCode::Char('/'));
    times.push((
        "App-to-app search keystroke",
        key(&mut m, KeyCode::Char('9')),
    ));
    key(&mut m, KeyCode::Esc);
    for (what, t) in &times {
        println!("{what}: {} ms", t.as_millis());
    }
    for (what, t) in &times {
        assert!(*t < budget, "{what}: {t:?} over {budget:?}");
    }
}
