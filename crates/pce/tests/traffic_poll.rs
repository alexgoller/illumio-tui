use pce::client::{ClientConfig, PceClient};
use pce::models::{ActorMatch, Flow};
use pce::traffic::TrafficQueryBuilder;
use pce::traffic::poll::{PollConfig, QueryProgress, run_traffic_query};
use pce::{Credentials, PceError};
use serde_json::json;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QUERY: &str = "/api/v2/orgs/5/traffic_flows/async_queries";
const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

fn client_for(server: &MockServer) -> PceClient {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut cfg = ClientConfig::new(
        url.host_str().unwrap(),
        5,
        Credentials::new("key", "secret"),
    );
    cfg.scheme = "http".into();
    cfg.port = url.port().unwrap();
    cfg.backoff_base = Duration::from_millis(1);
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    PceClient::new(cfg).unwrap()
}

fn fast() -> PollConfig {
    PollConfig {
        initial: Duration::from_millis(1),
        factor: 1.5,
        cap: Duration::from_millis(5),
        grace: Duration::from_millis(200),
        ..Default::default()
    }
}

fn query(max: u32) -> pce::models::TrafficQuery {
    TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .source(vec![ActorMatch::IpAddress {
        ip_address: "10.0.1.10".into(),
    }])
    .max_results(max)
    .build()
    .unwrap()
}

async fn mount_submit(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(QUERY))
        .and(header("Prefer", "respond-async"))
        .and(body_partial_json(json!({"sources": {"include": [[{"ip_address": "10.0.1.10"}]]}, "query_name": "illumio-tui"})))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_download(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn submits_polls_and_downloads() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"href": QHREF, "status": "working"})),
        )
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"href": QHREF, "status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4, "flows_count": 4})))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = run_traffic_query(
        &c,
        &query(1000),
        &fast(),
        &CancellationToken::new(),
        Some(&tx),
    )
    .await
    .unwrap();
    assert_eq!(r.flows.len(), 4);
    assert_eq!(r.flows_count, Some(4));
    assert!(!r.truncated && !r.incomplete);
    assert_eq!(r.query_href, QHREF);
    assert_eq!(c.stats().calls, 5, "POST + 3 polls + download");
    let mut seen = Vec::new();
    while let Ok(p) = rx.try_recv() {
        seen.push(p);
    }
    assert!(matches!(
        seen.first(),
        Some(QueryProgress::Submitted { .. })
    ));
    assert!(matches!(seen.last(), Some(QueryProgress::Done { rows: 4 })));
    assert!(
        seen.iter()
            .any(|p| matches!(p, QueryProgress::Polling { status, .. } if status == "working"))
    );
}

#[tokio::test]
async fn result_as_string_href_and_truncation() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": format!("{QHREF}/download"), "matches_count": 8, "flows_count": 4})))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(4), &fast(), &CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(r.flows.len(), 4);
    assert!(r.truncated, "flows_count == max_results");
}

/// #23: demo100's one-day status. `matches_count` above the rows is the
/// PCE collapsing aggregate versions, not more data.
#[tokio::test]
async fn matches_above_rows_is_not_truncation_and_a_silent_region_is_incomplete() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "result": format!("{QHREF}/download"),
            "matches_count": 6,
            "flows_count": 4,
            "regions": [
                {"pce_fqdn": "a.example", "responded": true, "flows_count": 4, "matches_count": 6},
                {"pce_fqdn": "b.example", "responded": false, "flows_count": 0, "matches_count": 0}
            ]
        })))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(10_000), &fast(), &CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!((r.flows.len(), r.flows_count), (4, Some(4)));
    assert!(!r.truncated, "4 of a 10000 cap");
    assert!(r.incomplete, "region b did not respond");
}

#[tokio::test]
async fn completed_without_result_is_retried_within_grace() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed"})))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}}),
        ))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(1000), &fast(), &CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(r.flows.len(), 4);
}

#[tokio::test]
async fn completed_without_result_past_grace_is_an_error() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed"})))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let cfg = PollConfig {
        grace: Duration::from_millis(20),
        ..fast()
    };
    let r = run_traffic_query(&c, &query(1000), &cfg, &CancellationToken::new(), None).await;
    assert!(
        matches!(r, Err(PceError::Decode(ref m)) if m.contains("no result")),
        "{r:?}"
    );
}

#[tokio::test]
async fn failed_status_reports_message() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "failed", "message": "too many flows"})),
        )
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(1000), &fast(), &CancellationToken::new(), None).await;
    assert!(
        matches!(r, Err(PceError::QueryFailed(ref m)) if m == "too many flows"),
        "{r:?}"
    );
}

#[tokio::test]
async fn cancellation_stops_polling() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "working"})))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let cancel = CancellationToken::new();
    let cfg = PollConfig {
        initial: Duration::from_secs(30),
        ..fast()
    };
    let child = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        child.cancel();
    });
    let r = run_traffic_query(&c, &query(1000), &cfg, &cancel, None).await;
    assert!(matches!(r, Err(PceError::Cancelled)));
    assert_eq!(c.stats().calls, 1, "only the POST was sent");
}

#[tokio::test]
async fn cancellation_interrupts_an_in_flight_request() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "working"}))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;
    let c = client_for(&server);
    let cancel = CancellationToken::new();
    let child = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        child.cancel();
    });
    let started = std::time::Instant::now();
    let r = run_traffic_query(&c, &query(1000), &fast(), &cancel, None).await;
    assert!(matches!(r, Err(PceError::Cancelled)), "{r:?}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancel must not wait for the 5 s response: {:?}",
        started.elapsed()
    );
    assert_eq!(c.stats().calls, 2, "POST plus the interrupted status GET");
}

#[test]
fn fixture_rows_keep_unknown_fields() {
    let flows: Vec<Flow> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    assert_eq!(flows.len(), 4);
    assert!(flows[0].extra.is_empty());
    let with_extra: Flow = serde_json::from_str(
        r#"{"src":{},"dst":{},"service":{},"policy_decision":"allowed","new_field":{"a":1}}"#,
    )
    .unwrap();
    assert_eq!(with_extra.extra["new_field"]["a"], 1);
}

/// The download has its own timeout: a slow body past the client's
/// per-request timeout still arrives.
#[tokio::test]
async fn the_download_outlasts_the_client_timeout() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "completed", "result": format!("{QHREF}/download"), "flows_count": 4}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json")
                .set_delay(Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut cfg = ClientConfig::new(
        url.host_str().unwrap(),
        5,
        Credentials::new("key", "secret"),
    );
    cfg.scheme = "http".into();
    cfg.port = url.port().unwrap();
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    cfg.timeout = Duration::from_millis(150);
    let c = PceClient::new(cfg).unwrap();
    let slow = PollConfig {
        download_timeout: Duration::from_secs(5),
        ..fast()
    };
    let r = run_traffic_query(&c, &query(1000), &slow, &CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(
        r.flows.len(),
        4,
        "a 400 ms body under a 150 ms client timeout"
    );
}

// ---- the draft re-evaluation (`update_rules`) ----

const RULES: &str = "/api/v2/orgs/5/traffic_flows/async_queries/q-1/update_rules";

/// The fixture's flows with a draft decision each: the blocked one is
/// allowed in draft, the rest keep their decision.
fn draft_download() -> String {
    let mut flows: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    for f in flows.as_array_mut().unwrap() {
        let reported = f["policy_decision"].as_str().unwrap().to_string();
        let draft = if reported == "blocked" {
            "allowed"
        } else {
            reported.as_str()
        };
        f["draft_policy_decision"] = json!(draft);
        f["rules"] = json!([{"essential_service_rule": "dhcp"}]);
        f["enforcement_boundaries"] = json!([]);
        f["override_deny_rules"] = json!([]);
    }
    flows.to_string()
}

fn completed(extra: serde_json::Value) -> serde_json::Value {
    let mut v = json!({"href": QHREF, "status": "completed",
        "result": {"href": format!("{QHREF}/download")}, "flows_count": 4});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

/// The status sequence of a draft run: completed without rules (until the
/// PUT), then `rules: working` once, then `rules: completed`.
async fn mount_draft_statuses(server: &MockServer, before_put: u64) {
    if before_put > 0 {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{QHREF}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(completed(json!({}))))
            .up_to_n_times(before_put)
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completed(json!({"rules": "working"}))),
        )
        .up_to_n_times(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completed(json!({"rules": "completed"}))),
        )
        .mount(server)
        .await;
}

async fn mount_put(server: &MockServer, limit: &str, status: u16) {
    Mock::given(method("PUT"))
        .and(path(RULES))
        .and(wiremock::matchers::query_param(
            "label_based_rules",
            "false",
        ))
        .and(wiremock::matchers::query_param("offset", "0"))
        .and(wiremock::matchers::query_param("limit", limit))
        .respond_with(ResponseTemplate::new(status))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_draft_download(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(draft_download())
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(server)
        .await;
}

/// `METHOD path` of every request the server saw, in order.
async fn sequence(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r.method,
                r.url.path().trim_start_matches("/api/v2")
            )
        })
        .collect()
}

#[tokio::test]
async fn a_draft_run_posts_polls_puts_polls_and_downloads_once() {
    use pce::models::DraftDecision;
    use pce::traffic::run_traffic_query_draft;
    let server = MockServer::start().await;
    mount_submit(&server).await;
    mount_draft_statuses(&server, 1).await;
    mount_put(&server, "2500", 202).await;
    mount_draft_download(&server).await;
    let c = client_for(&server);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = run_traffic_query_draft(
        &c,
        &query(2500),
        &fast(),
        &CancellationToken::new(),
        Some(&tx),
    )
    .await
    .unwrap();
    assert_eq!(r.query_href, QHREF);
    let draft: Vec<_> = r
        .flows
        .iter()
        .map(|f| f.draft_policy_decision.clone())
        .collect();
    assert!(draft.iter().all(Option::is_some), "{draft:?}");
    assert!(draft.contains(&Some(DraftDecision::Allowed)));
    assert_eq!(
        sequence(&server).await,
        vec![
            "POST /orgs/5/traffic_flows/async_queries".to_string(),
            format!("GET {QHREF}"),
            format!("PUT {QHREF}/update_rules"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
        ]
    );
    assert_eq!(c.stats().calls, 6, "POST, poll, PUT, 2 polls, download");
    let mut seen = Vec::new();
    while let Ok(p) = rx.try_recv() {
        seen.push(p);
    }
    assert!(
        seen.iter()
            .any(|p| matches!(p, QueryProgress::Evaluating { rules, .. } if rules == "working"))
    );
}

#[tokio::test]
async fn re_evaluating_a_known_query_sends_no_post() {
    use pce::traffic::evaluate_draft;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    mount_draft_statuses(&server, 0).await;
    mount_put(&server, "10000", 204).await;
    mount_draft_download(&server).await;
    let c = client_for(&server);
    let r = evaluate_draft(
        &c,
        QHREF,
        &query(10_000),
        &fast(),
        &CancellationToken::new(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.flows.len(), 4);
    assert!(r.flows.iter().all(|f| f.draft_policy_decision.is_some()));
    assert_eq!(
        sequence(&server).await,
        vec![
            format!("GET {QHREF}"),
            format!("PUT {QHREF}/update_rules"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
        ],
        "the status before the PUT, then polls until the rules complete"
    );
}

#[tokio::test]
async fn a_query_the_pce_dropped_is_not_found_and_a_refusal_is_clear() {
    use pce::traffic::{evaluate_draft, run_traffic_query_draft};
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(RULES))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = evaluate_draft(
        &c,
        QHREF,
        &query(10),
        &fast(),
        &CancellationToken::new(),
        None,
    )
    .await;
    assert!(matches!(r, Err(PceError::NotFound { .. })), "{r:?}");
    assert_eq!(c.stats().calls, 1, "the PUT only");

    // On a query just submitted, a 404 is a PCE without update_rules.
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(completed(json!({}))))
        .mount(&server)
        .await;
    let r = run_traffic_query_draft(&c, &query(10), &fast(), &CancellationToken::new(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(r, PceError::QueryFailed(ref m) if m.contains("update_rules")),
        "{r}"
    );
}

#[tokio::test]
async fn a_pce_that_never_reports_rules_fails_after_the_grace() {
    use pce::traffic::evaluate_draft;
    let server = MockServer::start().await;
    mount_put(&server, "10", 202).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(completed(json!({}))))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let cfg = PollConfig {
        grace: Duration::from_millis(20),
        ..fast()
    };
    let r = evaluate_draft(&c, QHREF, &query(10), &cfg, &CancellationToken::new(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(r, PceError::QueryFailed(ref m) if m.contains("did not start")),
        "{r}"
    );
}

#[tokio::test]
async fn failed_rules_are_an_error_and_cancel_stops_the_draft_polls() {
    use pce::traffic::evaluate_draft;
    let server = MockServer::start().await;
    mount_put(&server, "10", 202).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completed(json!({"rules": "failed"}))),
        )
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = evaluate_draft(
        &c,
        QHREF,
        &query(10),
        &fast(),
        &CancellationToken::new(),
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(r, PceError::QueryFailed(ref m) if m.contains("failed")),
        "{r}"
    );

    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completed(json!({"rules": "working"}))),
        )
        .mount(&server)
        .await;
    let c = client_for(&server);
    let cancel = CancellationToken::new();
    let child = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        child.cancel();
    });
    let cfg = PollConfig {
        initial: Duration::from_secs(30),
        ..fast()
    };
    let r = evaluate_draft(&c, QHREF, &query(10), &cfg, &cancel, None).await;
    assert!(matches!(r, Err(PceError::Cancelled)), "{r:?}");
    assert_eq!(
        c.stats().calls,
        2,
        "the status before the PUT and the PUT, no poll after the cancel"
    );
}

/// A repeat PUT: the first poll after it still shows the previous
/// evaluation's `completed` (same `updated_at`); only the next one, whose
/// `updated_at` moved, counts.
#[tokio::test]
async fn a_repeat_put_waits_past_the_old_completed_state() {
    use pce::traffic::evaluate_draft;
    let server = MockServer::start().await;
    let at = |t: &str| completed(json!({"rules": "completed", "updated_at": t}));
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(at("2026-10-14T10:00:00Z")))
        .up_to_n_times(2) // before the PUT, and the first poll after it
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(at("2026-10-14T10:05:00Z")))
        .mount(&server)
        .await;
    mount_put(&server, "10", 202).await;
    mount_draft_download(&server).await;
    let c = client_for(&server);
    let cfg = PollConfig {
        grace: Duration::from_secs(30),
        ..fast()
    };
    evaluate_draft(&c, QHREF, &query(10), &cfg, &CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(
        sequence(&server).await,
        vec![
            format!("GET {QHREF}"),
            format!("PUT {QHREF}/update_rules"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
        ],
        "the stale completed is polled past"
    );
}

#[tokio::test]
async fn both_polling_stages_give_up_after_max_wait() {
    use pce::traffic::evaluate_draft;
    let cfg = PollConfig {
        max_wait: Duration::from_millis(40),
        ..fast()
    };
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "working"})))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let e = run_traffic_query(&c, &query(10), &cfg, &CancellationToken::new(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, PceError::QueryFailed(m) if m.starts_with("the query timed out after")),
        "{e}"
    );

    let server = MockServer::start().await;
    mount_put(&server, "10", 202).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completed(json!({"rules": "working"}))),
        )
        .mount(&server)
        .await;
    let c = client_for(&server);
    let e = evaluate_draft(&c, QHREF, &query(10), &cfg, &CancellationToken::new(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, PceError::QueryFailed(m) if m.starts_with("the draft re-evaluation timed out after")),
        "{e}"
    );
}

#[tokio::test]
async fn an_empty_result_skips_the_draft_evaluation() {
    // demo100 answers update_rules on an empty result with 405.
    use pce::traffic::run_traffic_query_draft;
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completed(json!({"flows_count": 0}))),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(RULES))
        .respond_with(ResponseTemplate::new(405))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = run_traffic_query_draft(&c, &query(2500), &fast(), &CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(r.flows.is_empty());
    assert_eq!(
        sequence(&server).await,
        vec![
            "POST /orgs/5/traffic_flows/async_queries".to_string(),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
        ]
    );
}
