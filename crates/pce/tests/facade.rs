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
async fn name_index_loads_four_lists_once() {
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
    let idx = p.name_index(CacheMode::Use).await.unwrap();
    assert_eq!(idx.label_text("/orgs/5/labels/3"), "app=shop");
    let again = p.name_index(CacheMode::Use).await.unwrap();
    assert_eq!(again.label_keys().len(), 4);
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
        4
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

#[tokio::test]
async fn policy_reads_use_scope_and_cache() {
    use pce::models::PolicyScope;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/active/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/deny_rules.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/pending_dict.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    assert_eq!(
        p.rule_sets(PolicyScope::Draft, CacheMode::Use)
            .await
            .unwrap()
            .value
            .len(),
        2
    );
    assert_eq!(
        p.rule_sets(PolicyScope::Active, CacheMode::Use)
            .await
            .unwrap()
            .value
            .len(),
        0
    );
    assert!(
        p.rule_sets(PolicyScope::Draft, CacheMode::Use)
            .await
            .unwrap()
            .hit
    );
    assert_eq!(
        p.deny_rules("/orgs/5/sec_policy/draft/rule_sets/100", CacheMode::Use)
            .await
            .unwrap()
            .value
            .len(),
        2
    );
    assert_eq!(
        p.pending_changes(CacheMode::Use).await.unwrap().value.len(),
        2
    );
    assert_eq!(p.stats().calls, 4);
}

#[tokio::test]
async fn traffic_is_cached_by_query_and_filters_resolve() {
    use pce::traffic::{FilterExpr, TrafficQueryBuilder, poll::PollConfig};
    use tokio_util::sync::CancellationToken;
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
        (
            "/api/v2/orgs/5/workloads",
            include_str!("../../../fixtures/workloads.json"),
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
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(ResponseTemplate::new(202).set_body_json(
            json!({"href": "/orgs/5/traffic_flows/async_queries/q-9", "status": "queued"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries/q-9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": "/orgs/5/traffic_flows/async_queries/q-9/download"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/traffic_flows/async_queries/q-9/download",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut p = pce_for(&server);
    p.set_poll_config(PollConfig {
        initial: Duration::from_millis(1),
        factor: 1.5,
        cap: Duration::from_millis(5),
        grace: Duration::from_millis(100),
    });
    let groups = vec![vec![
        FilterExpr::parse("app=shop").unwrap(),
        FilterExpr::parse("wl:host-1.example").unwrap(),
    ]];
    let actors = p
        .resolve_filter_groups(&groups, CacheMode::Use)
        .await
        .unwrap();
    assert_eq!(actors[0].len(), 2);
    let no_wl = p
        .resolve_filter_groups(
            &[vec![FilterExpr::parse("env=prod").unwrap()]],
            CacheMode::Use,
        )
        .await
        .unwrap();
    assert_eq!(no_wl[0].len(), 1);
    let q = TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .source(actors[0].clone())
    .build()
    .unwrap();
    let r1 = p
        .traffic(&q, CacheMode::Use, &CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(!r1.hit);
    assert_eq!(r1.value.flows.len(), 4);
    let r2 = p
        .traffic(&q, CacheMode::Use, &CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(r2.hit);
    assert_eq!(
        p.stats().calls,
        5 + 3,
        "4 object lists + workloads once, then POST + poll + download once"
    );
}

#[tokio::test]
async fn only_the_latest_traffic_result_is_cached() {
    use pce::models::ActorMatch;
    use pce::traffic::{TrafficQueryBuilder, poll::PollConfig};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(ResponseTemplate::new(202).set_body_json(
            json!({"href": "/orgs/5/traffic_flows/async_queries/q-9", "status": "queued"}),
        ))
        .expect(4) // q1, q2, q1 again, q2 again: each a miss
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries/q-9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": "/orgs/5/traffic_flows/async_queries/q-9/download"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/traffic_flows/async_queries/q-9/download",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let mut p = pce_for(&server);
    p.set_poll_config(PollConfig {
        initial: Duration::from_millis(1),
        factor: 1.5,
        cap: Duration::from_millis(5),
        grace: Duration::from_millis(100),
    });
    let bounds = || {
        TrafficQueryBuilder::between(
            "2026-10-09T00:00:00Z".parse().unwrap(),
            "2026-10-10T00:00:00Z".parse().unwrap(),
        )
    };
    let q1 = bounds()
        .source(vec![ActorMatch::IpAddress {
            ip_address: "10.0.0.1".into(),
        }])
        .build()
        .unwrap();
    let q2 = bounds()
        .source(vec![ActorMatch::IpAddress {
            ip_address: "10.0.0.2".into(),
        }])
        .build()
        .unwrap();
    let cancel = CancellationToken::new();
    assert!(
        !p.traffic(&q1, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit
    );
    assert!(
        p.traffic(&q1, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit
    );
    let after_first = p.stats().calls;
    assert!(
        !p.traffic(&q2, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit
    );
    let after_second = p.stats().calls;
    assert!(after_second > after_first);
    assert!(
        !p.traffic(&q1, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit,
        "the first query was evicted when the second ran"
    );
    assert!(p.stats().calls > after_second);
    assert!(
        !p.traffic(&q2, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit,
        "and the second is gone again now"
    );
}

#[tokio::test]
async fn events_are_cached_briefly_and_default_to_500_rows() {
    use pce::models::EventFilter;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param("severity", "err"))
        .and(query_param("max_results", "500"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let f = EventFilter {
        severity: Some("err".into()),
        ..Default::default()
    };
    assert_eq!(p.events(&f, CacheMode::Use).await.unwrap().value.len(), 4);
    assert!(p.events(&f, CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.ttls().events, Duration::from_secs(60));
}
