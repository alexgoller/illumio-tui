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
async fn get_retries_after_503_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c.get::<Value>("/orgs/5/x", &[]).await.unwrap();
    assert_eq!(r.body["ok"], true);
    assert_eq!(c.stats().calls, 2, "503 then 200");
}

/// A `Retry-After` pause is client-wide: a request on another path that
/// starts while the pause is in force waits for it too. The paths are hit
/// concurrently because the throttled GET itself only returns after the
/// pause has elapsed.
#[tokio::test]
async fn retry_after_pauses_the_whole_client() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"x": true})))
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/y"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"y": true})))
        .expect(1)
        .mount(&server)
        .await;
    let c = client_for(&server);
    let started = tokio::time::Instant::now();
    let (x, y_done) = tokio::join!(c.get::<Value>("/orgs/5/x", &[]), async {
        // Let the 429 land first, then ask for an unrelated path.
        tokio::time::sleep(Duration::from_millis(100)).await;
        c.get::<Value>("/orgs/5/y", &[]).await.unwrap();
        started.elapsed()
    });
    assert_eq!(x.unwrap().body["x"], true);
    assert!(
        y_done >= Duration::from_millis(900),
        "y returned after {y_done:?}, before the 1 s pause ended"
    );
    assert_eq!(c.stats().calls, 3, "x, x again, y");
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

use pce::client::WriteMethod;

#[tokio::test]
async fn write_sends_json_body_once_and_decodes_201() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(header("Authorization", "Basic a2V5OnNlY3JldA=="))
        .and(header("Content-Type", "application/json"))
        .and(wiremock::matchers::body_json(
            json!({"key": "app", "value": "web"}),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c
        .write(
            WriteMethod::Post,
            &c.org_path("labels"),
            Some(&json!({"key": "app", "value": "web"})),
        )
        .await
        .unwrap();
    assert_eq!(r.status, 201);
    assert_eq!(r.body["href"], "/orgs/5/labels/9");
    assert_eq!(c.stats().calls, 1);
}

#[tokio::test]
async fn write_treats_204_as_null_body() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/labels/9"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/labels/9"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c
        .write(
            WriteMethod::Put,
            "/orgs/5/labels/9",
            Some(&json!({"value": "x"})),
        )
        .await
        .unwrap();
    assert_eq!(r.status, 204);
    assert!(r.body.is_null());
    let r = c
        .write(WriteMethod::Delete, "/orgs/5/labels/9", None)
        .await
        .unwrap();
    assert_eq!(r.status, 204);
    assert!(r.body.is_null());
}

#[tokio::test]
async fn write_maps_validation_statuses_to_rejected() {
    let server = MockServer::start().await;
    for (p, status) in [("a", 400), ("b", 406), ("c", 409), ("d", 422)] {
        Mock::given(method("POST"))
            .and(path(format!("/api/v2/orgs/5/{p}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(
                json!([{"token": "name_must_be_unique", "message": "Name must be unique"}]),
            ))
            .mount(&server)
            .await;
    }
    let c = client_for(&server);
    for (p, status) in [("a", 400u16), ("b", 406), ("c", 409), ("d", 422)] {
        let err = c
            .write(WriteMethod::Post, &c.org_path(p), Some(&json!({})))
            .await
            .unwrap_err();
        match err {
            PceError::Rejected { status: s, body } => {
                assert_eq!(s, status);
                assert_eq!(body, "name_must_be_unique: Name must be unique");
            }
            other => panic!("{p}: expected Rejected, got {other:?}"),
        }
        assert_eq!(err_kind(&c, p).await, "rejected");
    }
}

async fn err_kind(c: &PceClient, p: &str) -> &'static str {
    c.write(WriteMethod::Post, &c.org_path(p), Some(&json!({})))
        .await
        .unwrap_err()
        .kind()
}

#[tokio::test]
async fn write_is_never_retried_on_429_or_503() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/x"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/y"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let c = client_for(&server);
    let err = c
        .write(WriteMethod::Put, "/orgs/5/x", Some(&json!({})))
        .await
        .unwrap_err();
    assert!(matches!(err, PceError::RateLimited { .. }), "{err:?}");
    // A write 429 sets no client pause (nothing is retried); the DELETE is sent once.
    let err = c
        .write(WriteMethod::Delete, "/orgs/5/y", None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, PceError::Server { status: 503, .. }),
        "{err:?}"
    );
    assert_eq!(c.stats().calls, 2);
}
