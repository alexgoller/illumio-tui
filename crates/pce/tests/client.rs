use pce::client::{ClientConfig, PceClient};
use pce::{Credentials, PceError};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn get_sends_basic_auth_accept_and_org_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(header("Authorization", "Basic a2V5OnNlY3JldA=="))
        .and(header("Accept", "application/json"))
        .and(query_param("max_results", "10"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/labels/1"}]))
                .insert_header("X-Total-Count", "42"),
        )
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c
        .get::<Value>(&c.org_path("labels"), &[("max_results", "10".into())])
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.total_count, Some(42));
    assert_eq!(r.body[0]["href"], "/orgs/5/labels/1");
    let s = c.stats();
    assert_eq!(s.calls, 1);
    assert!(s.bytes > 10);
}

#[tokio::test]
async fn health_path_has_no_org() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c.get::<Value>("/health", &[]).await.unwrap();
    assert_eq!(r.body["status"], "normal");
}

#[tokio::test]
async fn status_codes_map_to_typed_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/a"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/b"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/c"))
        .respond_with(ResponseTemplate::new(406).set_body_json(
            json!([{"token": "invalid_uri", "message": "Invalid traffic filter type"}]),
        ))
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/d"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let c = client_for(&server);
    assert!(matches!(
        c.get::<Value>("/orgs/5/a", &[]).await,
        Err(PceError::Auth { status: 401 })
    ));
    assert!(matches!(
        c.get::<Value>("/orgs/5/b", &[]).await,
        Err(PceError::NotFound { .. })
    ));
    match c.get::<Value>("/orgs/5/c", &[]).await {
        Err(PceError::InvalidFilter(m)) => {
            assert_eq!(m, "invalid_uri: Invalid traffic filter type")
        }
        other => panic!("{other:?}"),
    }
    match c.get::<Value>("/orgs/5/d", &[]).await {
        Err(PceError::Server { status: 500, body }) => assert_eq!(body, "boom"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn get_retries_after_429_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c.get::<Value>("/orgs/5/x", &[]).await.unwrap();
    assert_eq!(r.body["ok"], true);
    assert_eq!(c.stats().calls, 3);
}

#[tokio::test]
async fn get_gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .mount(&server)
        .await;
    let c = client_for(&server);
    match c.get::<Value>("/orgs/5/x", &[]).await {
        Err(PceError::RateLimited { retry_after }) => assert_eq!(retry_after, Some(Duration::ZERO)),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.stats().calls, 5, "1 try + 4 retries");
}

#[tokio::test]
async fn post_is_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(header("Prefer", "respond-async"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c
        .post::<Value, Value>(
            "/orgs/5/traffic_flows/async_queries",
            &json!({"a": 1}),
            &[("Prefer", "respond-async")],
        )
        .await;
    assert!(matches!(r, Err(PceError::Server { status: 503, .. })));
    assert_eq!(c.stats().calls, 1);
}

#[tokio::test]
async fn retry_after_http_date_is_ignored() {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(
        "Retry-After",
        "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
    );
    assert_eq!(pce::client::parse_retry_after(&h), None);
    h.insert("Retry-After", "12".parse().unwrap());
    assert_eq!(
        pce::client::parse_retry_after(&h),
        Some(Duration::from_secs(12))
    );
}
