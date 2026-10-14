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
async fn a_timeline_bucket_counts_self_traffic_once_in_both_directions() {
    use pce::traffic::{Bucket, Focus, TrafficQueryBuilder};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    // Flow 0 (shop/prod to itself, 310 connections) made blocked; both
    // sides' queries return it, with the NAS flow (3, blocked).
    let mut flows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    flows[0]["policy_decision"] = json!("blocked");
    let flows = serde_json::to_string(&flows).unwrap();
    let labels = shop_labels();
    let bucket = Bucket {
        start: "2026-10-09T09:00:00Z".parse().unwrap(),
        end: "2026-10-09T10:00:00Z".parse().unwrap(),
    };
    for (name, side) in [("b-in", "destinations"), ("b-out", "sources")] {
        mount_query(
            &server,
            name,
            json!({
                side: {"include": [labels.clone()]},
                "policy_decisions": ["blocked"],
                "start_date": "2026-10-09T09:00:00Z",
                "end_date": "2026-10-09T10:00:00Z",
            }),
            &flows,
            1,
        )
        .await;
    }
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
    let c = p
        .timeline_bucket(3, &both, &base, &bucket, &idx, CacheMode::Use, &cancel)
        .await
        .unwrap();
    assert_eq!(
        c.connections, 313,
        "the self flow's 310 once, plus the NAS flow's 3"
    );
    assert_eq!((c.queries, c.hit, c.truncated), (2, false, false));
    let again = p
        .timeline_bucket(3, &both, &base, &bucket, &idx, CacheMode::Use, &cancel)
        .await
        .unwrap();
    assert!(again.hit, "a repeated bucket is answered from its slot");
    assert_eq!(again.connections, 313);
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
