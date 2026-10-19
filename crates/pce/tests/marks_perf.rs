//! The marks' speed budget: 200 000 flows against 200 marks, classified
//! and partitioned, in under 300 ms. Release only (`#[ignore]`); CI runs
//! it on Ubuntu with `cargo test --release -p pce --test marks_perf --
//! --ignored --nocapture`, which prints the time.
use pce::marks::{Action, Mark, MarkSet, ServiceSpec, parse_side, partition};
use pce::models::PolicyDecision;
use pce::resolve::ResolvedLabel;
use pce::traffic::{ResolvedEndpoint, ResolvedFlow};
use std::time::Instant;

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
            num_connections: 1,
            policy_decision: PolicyDecision::ALL[i % 3],
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            side_rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        })
        .collect()
}

/// 200 marks of every side kind, most with a service, a few broad ones.
fn marks() -> MarkSet {
    let marks = (0..200)
        .map(|i| {
            let (src, dst) = match i % 5 {
                0 => (format!("app=app{} env=prod", i % 50), "any".to_string()),
                1 => (
                    "any".to_string(),
                    format!("app=app{} role=role{}", i % 50, i % 7),
                ),
                2 => (format!("ip:10.{}.0.0/16", i % 200), "any".to_string()),
                3 => ("ipl:corp".to_string(), format!("app=app{}", i % 50)),
                _ => (
                    format!("app=app{}", i % 50),
                    format!("app=app{}", (i + 1) % 50),
                ),
            };
            Mark {
                id: format!("m-{i:06}"),
                action: Action::ALL[i % 6],
                src: parse_side(&src).unwrap(),
                dst: parse_side(&dst).unwrap(),
                services: if i % 4 == 0 {
                    Vec::new()
                } else {
                    vec![ServiceSpec::parse(&format!("{}/tcp", 1000 + i * 13)).unwrap()]
                },
                decisions: if i % 6 == 3 {
                    vec![PolicyDecision::Blocked]
                } else {
                    Vec::new()
                },
                draft_decisions: Vec::new(),
                scope: None,
                expires: None,
                comment: "perf".into(),
                created: None,
                created_on: None,
            }
        })
        .collect();
    MarkSet { marks }
}

#[test]
#[ignore = "release only: cargo test --release -p pce --test marks_perf -- --ignored"]
fn two_hundred_thousand_flows_against_two_hundred_marks_in_under_300_ms() {
    let flows = flows(200_000);
    let set = marks();
    let now = "2026-10-17T12:00:00Z".parse().unwrap();
    let active = set.active(None, now, false);
    assert_eq!(active.len(), 200);
    let start = Instant::now();
    let p = partition(&flows, &active, false);
    let took = start.elapsed();
    println!(
        "marks_perf: 200k flows x 200 marks partitioned in {took:?} ({} hits)",
        p.hits
    );
    assert!(p.hits > 0, "the marks match something");
    // 300 ms on a developer machine; shared CI runners are slower and noisy,
    // so CI sets MARKS_PERF_BUDGET_MS to catch only gross regressions.
    let budget: u128 = std::env::var("MARKS_PERF_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    assert!(
        took.as_millis() < budget,
        "200k flows x 200 marks took {took:?} (budget {budget} ms)"
    );
}
