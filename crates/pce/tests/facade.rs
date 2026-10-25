use pce::client::{ClientConfig, PceClient};
use pce::{CacheMode, Credentials, KeyOrder, Pce};
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
    let p = pce_for(&server).with_key_order(KeyOrder::new(&["env".into()], "role"));
    let idx = p.name_index(CacheMode::Use).await.unwrap();
    assert_eq!(idx.label_text("/orgs/5/labels/3"), "app=shop");
    let again = p.name_index(CacheMode::Use).await.unwrap();
    assert_eq!(
        again.label_keys(),
        vec!["env", "role", "app", "loc"],
        "the index is built with the Pce's key order"
    );
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
async fn a_filtered_read_ignores_the_collection_total() {
    use pce::models::WorkloadFilter;
    // The PCE's X-Total-Count is the whole collection's size, whatever the
    // filters: a one-host read is complete below the row cap.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("hostname", "web-8"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/workloads/1", "hostname": "web-8"}]))
                .insert_header("X-Total-Count", "206"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("hostname", "web"))
        .and(query_param("max_results", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/workloads/1", "hostname": "web-8"}]))
                .insert_header("X-Total-Count", "206"),
        )
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let one = WorkloadFilter {
        hostname: Some("web-8".into()),
        ..Default::default()
    };
    assert!(!p.workloads(&one, CacheMode::Use).await.unwrap().truncated);
    let capped = WorkloadFilter {
        hostname: Some("web".into()),
        max_results: Some(1),
        ..Default::default()
    };
    assert!(
        p.workloads(&capped, CacheMode::Use)
            .await
            .unwrap()
            .truncated,
        "the row cap was reached: more may match"
    );
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
        ..Default::default()
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
        ..Default::default()
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

#[tokio::test]
async fn events_are_truncated_only_when_the_row_cap_is_reached() {
    use pce::models::EventFilter;
    // `/events` reports every retained event in X-Total-Count, not only the
    // window's (#37): 4 rows of a 500 cap are the whole window.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param("max_results", "500"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json")
                .insert_header("X-Total-Count", "9000"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param("max_results", "4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json")
                .insert_header("X-Total-Count", "9000"),
        )
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let all = p
        .events(&EventFilter::default(), CacheMode::Use)
        .await
        .unwrap();
    assert_eq!(all.value.len(), 4);
    assert!(!all.truncated, "4 of a 500 cap: the window is complete");
    let capped = EventFilter {
        max_results: Some(4),
        ..Default::default()
    };
    let c = p.events(&capped, CacheMode::Use).await.unwrap();
    assert!(c.truncated, "the cap was reached: more may exist");
}

#[tokio::test]
async fn active_lists_are_cached_under_objects_and_dropped_by_a_write() {
    let server = MockServer::start().await;
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/active/ip_lists",
            json!([{"href": "/orgs/5/sec_policy/active/ip_lists/20", "name": "Any", "ip_ranges": [{"from_ip": "0.0.0.0/0"}]}]),
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/services",
            json!([{"href": "/orgs/5/sec_policy/active/services/32", "name": "Web", "service_ports": [{"port": 443, "proto": 6}]}]),
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/label_groups",
            json!([{"href": "/orgs/5/sec_policy/active/label_groups/10", "name": "All prod envs", "key": "env", "labels": [{"href": "/orgs/5/labels/4"}]}]),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(query_param("max_results", "10000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(2)
            .mount(&server)
            .await;
    }
    let p = pce_for(&server);
    let lists = p.active_ip_lists(CacheMode::Use).await.unwrap();
    assert!(!lists.hit);
    assert_eq!(lists.value[0].name, "Any");
    assert!(p.active_ip_lists(CacheMode::Use).await.unwrap().hit);
    let services = p.active_services(CacheMode::Use).await.unwrap();
    assert_eq!(services.value[0].service_ports[0].port, Some(443));
    assert!(p.active_services(CacheMode::Use).await.unwrap().hit);
    let groups = p.active_label_groups(CacheMode::Use).await.unwrap();
    assert_eq!(groups.value[0].labels.len(), 1);
    assert!(p.active_label_groups(CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.stats().calls, 3, "one GET per list");
    // `Writer::execute` invalidates the kind's prefix; for IP lists and
    // services that is `objects:`, which must cover the active lists too.
    for prefix in pce::write::ObjectKind::IpList.cache_prefixes() {
        p.invalidate(prefix);
    }
    assert!(!p.active_ip_lists(CacheMode::Use).await.unwrap().hit);
    assert!(!p.active_services(CacheMode::Use).await.unwrap().hit);
    assert!(!p.active_label_groups(CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.stats().calls, 6);
}

fn fast_poll(p: &mut Pce) {
    p.set_poll_config(pce::traffic::poll::PollConfig {
        initial: Duration::from_millis(1),
        factor: 1.5,
        cap: Duration::from_millis(5),
        grace: Duration::from_millis(100),
        ..Default::default()
    });
}

/// A POST matching `body` answered with query `name`, its status and its
/// download (`flows`); exactly `expect` POSTs.
async fn mount_query(
    server: &MockServer,
    name: &str,
    body: serde_json::Value,
    flows: &str,
    expect: u64,
) {
    use wiremock::matchers::body_partial_json;
    let href = format!("/orgs/5/traffic_flows/async_queries/{name}");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(body))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})),
        )
        .expect(expect)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"status": "completed", "result": format!("{href}/download")}),
            ),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(flows.to_string())
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

fn fixture_index() -> pce::NameIndex {
    let labels: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
    pce::NameIndex::new(&labels, &[], &[], &[])
}

fn shop_labels() -> serde_json::Value {
    json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}])
}

#[tokio::test]
async fn focused_traffic_runs_one_query_per_side_in_its_own_slot() {
    use pce::traffic::{Focus, FocusDirection, TrafficQueryBuilder};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    let flows = include_str!("../../../fixtures/traffic_flows.json");
    let labels = shop_labels();
    mount_query(
        &server,
        "q-in",
        json!({"destinations": {"include": [labels.clone()]}}),
        flows,
        1,
    )
    .await;
    mount_query(
        &server,
        "q-out",
        json!({"sources": {"include": [labels]}}),
        flows,
        1,
    )
    .await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let idx = fixture_index();
    let base = TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .max_results(500)
    .build()
    .unwrap();
    let both = Focus::new(
        vec!["shop".into(), "prod".into()],
        vec!["app".into(), "env".into()],
    );
    let cancel = CancellationToken::new();
    let r = p
        .focused_traffic("focus", &both, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert_eq!(r.query_count(), 2);
    assert_eq!(
        r.flows.len(),
        4,
        "flow 0 came back from both queries and counts once"
    );
    assert!(!r.hit);
    let again = p
        .focused_traffic("focus", &both, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(again.hit, "both slots answer from the cache");
    let inbound = both.clone().with_direction(FocusDirection::In);
    let r = p
        .focused_traffic(
            "focus",
            &inbound,
            &base,
            &idx,
            CacheMode::Use,
            &cancel,
            None,
        )
        .await
        .unwrap();
    assert!(r.hit, "the in query is the one `both` ran first");
    assert_eq!(
        r.flows.len(),
        3,
        "Focus::keeps narrows the in query's answer"
    );
    let nope = Focus::new(
        vec!["nope".into(), "prod".into()],
        vec!["app".into(), "env".into()],
    );
    let e = p
        .focused_traffic("focus", &nope, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, pce::PceError::Invalid(m) if m.starts_with("unknown app \"nope/prod\"")),
        "{e:?}"
    );
}

#[tokio::test]
async fn traffic_slots_do_not_evict_each_other() {
    use pce::traffic::{Focus, TrafficQueryBuilder};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    let flows = include_str!("../../../fixtures/traffic_flows.json");
    let labels = shop_labels();
    mount_query(&server, "q-main", json!({"max_results": 501}), "[]", 1).await;
    mount_query(
        &server,
        "q-in",
        json!({"destinations": {"include": [labels.clone()]}}),
        flows,
        1,
    )
    .await;
    mount_query(
        &server,
        "q-out",
        json!({"sources": {"include": [labels]}}),
        flows,
        1,
    )
    .await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let idx = fixture_index();
    let bounds = || {
        TrafficQueryBuilder::between(
            "2026-10-09T00:00:00Z".parse().unwrap(),
            "2026-10-10T00:00:00Z".parse().unwrap(),
        )
    };
    let main = bounds().max_results(501).build().unwrap();
    let base = bounds().max_results(500).build().unwrap();
    let focus = Focus::new(
        vec!["shop".into(), "prod".into()],
        vec!["app".into(), "env".into()],
    );
    let cancel = CancellationToken::new();
    assert!(
        !p.traffic(&main, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit
    );
    let f = p
        .focused_traffic("focus", &focus, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(!f.hit);
    assert!(
        p.traffic(&main, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit,
        "the focused queries left the main slot alone"
    );
    assert!(
        p.focused_traffic("focus", &focus, &base, &idx, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit,
        "and the main query left the focused slots alone"
    );
}

#[tokio::test]
async fn labels_with_usage_asks_for_usage_and_caches_apart_from_the_plain_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(query_param("usage", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/labels_usage.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/labels.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let with = p.labels_with_usage(CacheMode::Use).await.unwrap();
    assert!(with.value.iter().all(|l| l.usage.is_some()));
    assert!(p.labels_with_usage(CacheMode::Use).await.unwrap().hit);
    let plain = p.labels(CacheMode::Use).await.unwrap();
    assert!(!plain.hit, "the plain list is its own cache entry");
    assert!(plain.value.iter().all(|l| l.usage.is_none()));
    assert_eq!(p.stats().calls, 2);
}

#[tokio::test]
async fn any_write_drops_the_label_usage_but_not_the_plain_labels() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(query_param("usage", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/labels_usage.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/labels.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    p.labels(CacheMode::Use).await.unwrap();
    p.labels_with_usage(CacheMode::Use).await.unwrap();
    // `Writer::execute` drops the kind's prefixes: a workload write and a
    // rule write each leave the plain labels cached but not their usage.
    for kind in [
        pce::write::ObjectKind::Workload,
        pce::write::ObjectKind::Rule,
    ] {
        for prefix in kind.cache_prefixes() {
            p.invalidate(prefix);
        }
        assert!(
            !p.labels_with_usage(CacheMode::Use).await.unwrap().hit,
            "{kind:?}"
        );
        assert!(p.labels(CacheMode::Use).await.unwrap().hit, "{kind:?}");
    }
    assert_eq!(p.stats().calls, 4);
}

/// Every `METHOD path` the server saw, in order.
async fn seen(server: &MockServer) -> Vec<String> {
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

/// `update_rules` on any query of `names`, answered 204; their statuses
/// report the rules completed.
async fn mount_update_rules(server: &MockServer, names: &[&str]) {
    for name in names {
        let href = format!("/orgs/5/traffic_flows/async_queries/{name}");
        Mock::given(method("PUT"))
            .and(path(format!("/api/v2{href}/update_rules")))
            .respond_with(ResponseTemplate::new(204))
            .mount(server)
            .await;
    }
}

/// A status that reports the rules completed: mounted before
/// `mount_query`, so it answers first.
async fn mount_rules_completed(server: &MockServer, name: &str) {
    let href = format!("/orgs/5/traffic_flows/async_queries/{name}");
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}")))
        .respond_with(Ticking::new(json!({
            "status": "completed", "rules": "completed",
            "result": format!("{href}/download")
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn the_draft_result_has_its_own_cache_slot_tied_to_its_query() {
    use pce::traffic::TrafficQueryBuilder;
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    mount_rules_completed(&server, "q-main").await;
    mount_query(
        &server,
        "q-main",
        json!({"max_results": 700}),
        include_str!("../../../fixtures/traffic_flows.json"),
        1,
    )
    .await;
    mount_update_rules(&server, &["q-main", "q-gone"]).await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let q = TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .max_results(700)
    .build()
    .unwrap();
    let cancel = CancellationToken::new();
    let plain = p.traffic(&q, CacheMode::Use, &cancel, None).await.unwrap();
    let href = plain.value.query_href.clone();
    let before = seen(&server).await.len();
    let draft = p
        .traffic_draft(&q, Some(&href), CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(!draft.hit);
    assert_eq!(
        seen(&server).await[before..],
        [
            format!("GET {href}"),
            format!("PUT {href}/update_rules"),
            format!("GET {href}"),
            format!("GET {href}/download"),
        ],
        "no POST: the known query's status, then it is re-evaluated"
    );
    let calls = p.stats().calls;
    let again = p
        .traffic_draft(&q, Some(&href), CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(again.hit, "the draft slot answers");
    assert!(
        p.traffic(&q, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit,
        "the reported result stays cached beside it"
    );
    assert_eq!(p.stats().calls, calls, "no request for either");
    // Another run of the same body (a new href): its own draft, not the old one.
    let other = "/orgs/5/traffic_flows/async_queries/q-gone";
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{other}")))
        .respond_with(Ticking::new(json!({
            "status": "completed", "rules": "completed",
            "result": format!("{other}/download")
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{other}/download")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let r = p
        .traffic_draft(&q, Some(other), CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(!r.hit && r.value.flows.is_empty());
}

/// A write that drops the draft slot while a draft run is on its way: the
/// run's answer (read before the write) is not stored, so the run after
/// the write asks the PCE again instead of serving it.
#[tokio::test]
async fn a_draft_answer_overtaken_by_a_write_is_not_cached() {
    use pce::traffic::TrafficQueryBuilder;
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    let href = "/orgs/5/traffic_flows/async_queries/q-race";
    mount_rules_completed(&server, "q-race").await;
    mount_update_rules(&server, &["q-race"]).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json")
                .set_delay(Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let q = TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .max_results(700)
    .build()
    .unwrap();
    let cancel = CancellationToken::new();
    let (run, ()) = tokio::join!(
        p.traffic_draft(&q, Some(href), CacheMode::Use, &cancel, None),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            p.invalidate("draft:");
        }
    );
    assert!(!run.unwrap().hit);
    let puts = |seen: &[String]| seen.iter().filter(|s| s.starts_with("PUT")).count();
    assert_eq!(puts(&seen(&server).await), 1);
    let again = p
        .traffic_draft(&q, Some(href), CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(!again.hit, "the stale answer was not stored");
    assert_eq!(puts(&seen(&server).await), 2, "asked again");
    let third = p
        .traffic_draft(&q, Some(href), CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(third.hit, "an answer no write overtook is cached");
}

#[tokio::test]
async fn a_focused_draft_re_evaluates_each_side_without_a_post() {
    use pce::traffic::{Focus, TrafficQueryBuilder};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    let flows = include_str!("../../../fixtures/traffic_flows.json");
    let labels = shop_labels();
    mount_rules_completed(&server, "q-in").await;
    mount_rules_completed(&server, "q-out").await;
    mount_query(
        &server,
        "q-in",
        json!({"destinations": {"include": [labels.clone()]}}),
        flows,
        1,
    )
    .await;
    mount_query(
        &server,
        "q-out",
        json!({"sources": {"include": [labels]}}),
        flows,
        1,
    )
    .await;
    mount_update_rules(&server, &["q-in", "q-out"]).await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let idx = fixture_index();
    let base = TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .max_results(500)
    .build()
    .unwrap();
    let both = Focus::new(
        vec!["shop".into(), "prod".into()],
        vec!["app".into(), "env".into()],
    );
    let cancel = CancellationToken::new();
    let r = p
        .focused_traffic("focus", &both, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert_eq!(
        r.hrefs,
        vec![
            "/orgs/5/traffic_flows/async_queries/q-in",
            "/orgs/5/traffic_flows/async_queries/q-out"
        ]
    );
    let before = seen(&server).await.len();
    let d = p
        .focused_traffic_eval(
            "focus",
            &both,
            &base,
            &idx,
            Some(&r.hrefs),
            CacheMode::Use,
            &cancel,
            None,
        )
        .await
        .unwrap();
    assert_eq!(d.hrefs, r.hrefs);
    let after = seen(&server).await;
    let sent = &after[before..];
    assert_eq!(sent.iter().filter(|s| s.starts_with("PUT")).count(), 2);
    assert!(!sent.iter().any(|s| s.starts_with("POST")), "{sent:?}");
}

/// A status answer whose `updated_at` moves on every call, as the PCE's
/// does while it works: a draft `completed` after the PUT is then new.
struct Ticking {
    body: serde_json::Value,
    n: std::sync::atomic::AtomicU64,
}

impl Ticking {
    fn new(body: serde_json::Value) -> Self {
        Self {
            body,
            n: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl wiremock::Respond for Ticking {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut body = self.body.clone();
        body["updated_at"] = json!(format!("2026-10-14T00:00:{:02}Z", n % 60));
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// What an observer was told: (list, rows, complete).
#[derive(Default)]
struct Seen(std::sync::Mutex<Vec<(&'static str, usize, bool)>>);

impl pce::ListObserver for Seen {
    fn labels(&self, v: &[pce::models::Label], complete: bool) {
        self.0.lock().unwrap().push(("labels", v.len(), complete));
    }
    fn services(&self, v: &[pce::models::Service], complete: bool) {
        self.0.lock().unwrap().push(("services", v.len(), complete));
    }
    fn workloads(&self, v: &[pce::models::Workload], complete: bool) {
        self.0
            .lock()
            .unwrap()
            .push(("workloads", v.len(), complete));
    }
}

#[tokio::test]
async fn the_observer_hears_of_fresh_reads_only_and_whether_they_are_whole() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/labels/1", "key": "app", "value": "web"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(
                    json!([{"href": "/orgs/5/sec_policy/draft/services/1", "name": "SSH"}]),
                )
                .insert_header("X-Total-Count", "40"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/workloads/1", "hostname": "web1"}])),
        )
        .mount(&server)
        .await;
    let seen = std::sync::Arc::new(Seen::default());
    let p = pce_for(&server).with_observer(seen.clone());
    p.labels(CacheMode::Use).await.unwrap();
    p.labels(CacheMode::Use).await.unwrap();
    p.services(CacheMode::Use).await.unwrap();
    p.workloads(&Default::default(), CacheMode::Use)
        .await
        .unwrap();
    let filtered = pce::models::WorkloadFilter {
        hostname: Some("web".into()),
        ..Default::default()
    };
    p.workloads(&filtered, CacheMode::Use).await.unwrap();
    // A failed read tells nothing.
    assert!(p.ip_lists(CacheMode::Use).await.is_err());
    assert_eq!(
        *seen.0.lock().unwrap(),
        [
            ("labels", 1, true),
            ("services", 1, false),
            ("workloads", 1, true),
            ("workloads", 1, false),
        ],
        "a cache hit is not told; a truncated or filtered read is a part"
    );
}

#[tokio::test]
async fn label_workload_probe_reads_one_row_filtered_by_the_label_once() {
    // The X-Total-Count (the whole collection) says nothing of the label:
    // one row is "at least one", none an exact zero.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", r#"[["/orgs/5/labels/4"]]"#))
        .and(query_param("max_results", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/workloads/1"}]))
                .insert_header("X-Total-Count", "206"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", r#"[["/orgs/5/labels/5"]]"#))
        .and(query_param("max_results", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .insert_header("X-Total-Count", "206"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let one = p
        .label_workload_probe("/orgs/5/labels/4", CacheMode::Use)
        .await
        .unwrap();
    assert_eq!(
        (one.value.count, one.value.exact, one.hit),
        (1, false, false)
    );
    let again = p
        .label_workload_probe("/orgs/5/labels/4", CacheMode::Use)
        .await
        .unwrap();
    assert!(again.hit, "cached: no second request");
    let none = p
        .label_workload_probe("/orgs/5/labels/5", CacheMode::Use)
        .await
        .unwrap();
    assert_eq!((none.value.count, none.value.exact), (0, true));
    assert_eq!(p.stats().calls, 2);
    server.verify().await;
}
