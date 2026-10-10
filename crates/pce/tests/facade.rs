use pce::client::{ClientConfig, PceClient};
use pce::{CacheMode, Credentials, Pce};
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn pce_for(server: &MockServer) -> Pce {
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
    Pce::new(PceClient::new(cfg).unwrap())
}

#[tokio::test]
async fn health_is_not_cached_and_has_no_org() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"status": "normal"}])))
        .expect(2)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    p.health().await.unwrap();
    p.health().await.unwrap();
    assert_eq!(p.stats().calls, 2);
}

#[tokio::test]
async fn cached_list_hits_cache_and_reports_truncation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(query_param("max_results", "10000"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/labels/1", "key": "app", "value": "web"}]))
                .insert_header("X-Total-Count", "25000"),
        )
        .expect(3)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let first = p.labels(CacheMode::Use).await.unwrap();
    assert!(!first.hit);
    assert!(first.truncated, "25000 total but 1 row returned");
    let second = p.labels(CacheMode::Use).await.unwrap();
    assert!(second.hit);
    assert_eq!(p.stats().calls, 1);
    let third = p.labels(CacheMode::Bypass).await.unwrap();
    assert!(!third.hit);
    assert_eq!(p.stats().calls, 2);
    p.clear_cache();
    let fourth = p.labels(CacheMode::Use).await.unwrap();
    assert!(!fourth.hit, "clear_cache forces a fetch");
    assert_eq!(p.stats().calls, 3);
}

#[tokio::test]
async fn object_lists_are_cached_separately() {
    let server = MockServer::start().await;
    for (p, body) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let p = pce_for(&server);
    assert_eq!(p.labels(CacheMode::Use).await.unwrap().value.len(), 6);
    assert_eq!(p.label_groups(CacheMode::Use).await.unwrap().value.len(), 1);
    assert_eq!(p.ip_lists(CacheMode::Use).await.unwrap().value.len(), 2);
    assert_eq!(p.services(CacheMode::Use).await.unwrap().value.len(), 3);
    assert!(p.services(CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.stats().calls, 4);
}

#[tokio::test]
async fn workloads_pass_filters_and_cache_per_filter() {
    use pce::models::WorkloadFilter;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("hostname", "host-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let f = WorkloadFilter {
        hostname: Some("host-1".into()),
        ..Default::default()
    };
    assert_eq!(
        p.workloads(&f, CacheMode::Use).await.unwrap().value.len(),
        3
    );
    assert!(p.workloads(&f, CacheMode::Use).await.unwrap().hit);
    assert_eq!(
        p.workloads(&WorkloadFilter::default(), CacheMode::Use)
            .await
            .unwrap()
            .value
            .len(),
        0
    );
    assert_eq!(p.stats().calls, 2);
}
