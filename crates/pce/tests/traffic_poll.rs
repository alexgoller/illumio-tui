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
