use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures::{StreamExt, stream};
use illumio_tui::tui::app::{Runtime, run_loop};
use illumio_tui::tui::model::{Connection, Model, ProfileInfo, WritesState};
use illumio_tui::tui::msg::Cmd;
use pce::client::{ClientConfig, PceClient};
use pce::write::{MemoryAudit, WriteContext, WriteGuard};
use pce::{Credentials, Pce};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn pce_for(server: &MockServer) -> Arc<Pce> {
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
    let mut pce = Pce::new(PceClient::new(cfg).unwrap());
    // The 1 s default first poll would eat most of the tests' sleep margins.
    pce.set_poll_config(pce::traffic::PollConfig {
        initial: Duration::from_millis(10),
        ..pce::traffic::PollConfig::default()
    });
    Arc::new(pce)
}

fn model() -> Model {
    Model::new_connecting(
        ProfileInfo {
            name: "t".into(),
            host: "h".into(),
            org_id: 5,
        },
        "dark".into(),
        60,
        1000,
        "24h".into(),
        WritesState::default(),
    )
}

async fn mount_objects(server: &MockServer) {
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
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn startup_connects_loads_index_and_q_quits() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .expect(1)
        .mount(&server)
        .await;
    mount_objects(&server).await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), model());
    rt.execute(Cmd::Connect);
    // Give the two startup tasks time, then press q.
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let mut draws = 0usize;
    let model = run_loop(&mut rt, Box::pin(events), &mut |_m| draws += 1).await;
    assert!(model.quit);
    assert!(
        matches!(
            model.connection,
            Connection::Connected { index: Some(_), .. }
        ),
        "{:?}",
        model.connection
    );
    assert_eq!(model.stats.calls, 5, "health + four lists");
    assert!(draws >= 3, "redraw after messages, got {draws}");
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn failed_health_leaves_a_disconnected_model() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), model());
    rt.execute(Cmd::Connect);
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    match model.connection {
        Connection::Disconnected { error } => assert!(error.contains("401"), "{error}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn end_of_event_stream_ends_the_loop() {
    let server = MockServer::start().await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), model());
    let model = run_loop(&mut rt, Box::pin(stream::empty()), &mut |_| {}).await;
    assert!(!model.quit);
}

#[tokio::test]
async fn run_query_command_executes_against_the_pce_and_reports_flows() {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::Msg;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, tx) = Runtime::new(pce, m);
    let mut f = TrafficFilter::defaults("1h", 50);
    f.src = "app=shop".into();
    tx.send(Msg::ApplyFilter(f)).unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    let data = model.traffic.data.as_ref().expect("query result");
    assert_eq!(data.flows.len(), 4);
    assert_eq!(data.flows[0].src.display_name(), "host-1.example");
    assert!(!model.traffic.is_running());
    assert_eq!(model.busy, 0);
    assert!(
        model.stats.calls >= 7,
        "4 lists + POST + poll + download, got {}",
        model.stats.calls
    );
}

#[tokio::test]
async fn bang_terms_go_into_explorers_exclude_lists() {
    use illumio_tui::tui::filter::TrafficFilter;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let pce = pce_for(&server);
    let mut f = TrafficFilter::defaults("1h", 50);
    f.src = "app=shop | 10.0.0.1, !env=prod".into();
    f.dst = "!ipl:corp, !fqdn:x.example".into();
    f.service = "tcp, !22/tcp | !udp".into();
    let q = f.to_query(&pce, pce::CacheMode::Use).await.unwrap();
    let v = serde_json::to_value(&q).unwrap();
    assert_eq!(
        v["sources"]["include"],
        json!([[{"label": {"href": "/orgs/5/labels/3"}}], [{"ip_address": "10.0.0.1"}]])
    );
    assert_eq!(
        v["sources"]["exclude"],
        json!([{"label": {"href": "/orgs/5/labels/4"}}]),
        "flat, whichever | group the ! was in"
    );
    assert_eq!(
        v["destinations"]["include"],
        json!([[]]),
        "only exclusions: every destination but those"
    );
    assert_eq!(
        v["destinations"]["exclude"],
        json!([
            {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
            {"fqdn": "x.example"}
        ])
    );
    assert_eq!(v["services"]["include"], json!([{"proto": 6}]));
    assert_eq!(
        v["services"]["exclude"],
        json!([{"port": 22, "proto": 6}, {"proto": 17}])
    );
}

#[tokio::test]
async fn refresh_only_reruns_the_traffic_query() {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::Msg;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, tx) = Runtime::new(pce, m);
    let mut f = TrafficFilter::defaults("1h", 50);
    f.src = "app=shop".into();
    tx.send(Msg::ApplyFilter(f)).unwrap();
    let events = stream::iter([(1200u64, KeyCode::Char('r')), (1200u64, KeyCode::Char('q'))]).then(
        |(ms, code)| async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
        },
    );
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.traffic.data.as_ref().map(|d| d.flows.len()), Some(4));
    assert_eq!(
        model.stats.calls,
        4 + 3 + 3,
        "4 lists once, then POST + poll + download twice"
    );
}

#[tokio::test]
async fn tick_period_follows_busy_and_auto_state() {
    use illumio_tui::tui::app::{BUSY_TICK, IDLE_TICK};
    let server = MockServer::start().await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), model());
    assert_eq!(rt.tick_period(), None);
    rt.model.traffic.auto.enabled = true;
    assert_eq!(rt.tick_period(), Some(IDLE_TICK));
    rt.model.busy = 1;
    assert_eq!(rt.tick_period(), Some(BUSY_TICK));
}

/// Runs one initial query, then a far-future Tick; `auto` presses `a` first.
/// Returns the number of async-query POSTs the PCE saw.
async fn posts_after_far_future_tick(auto: bool) -> usize {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::Msg;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, tx) = Runtime::new(pce, m);
    let mut f = TrafficFilter::defaults("1h", 50);
    f.src = "app=shop".into();
    tx.send(Msg::ApplyFilter(f)).unwrap();
    // Ticks are injected with a far-future clock: the real interval is >= 60 s.
    let tick_tx = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let _ = tick_tx.send(Msg::Tick(
            std::time::Instant::now() + Duration::from_secs(3600),
        ));
    });
    let keys: Vec<(u64, Option<KeyCode>)> = if auto {
        vec![
            (1800, Some(KeyCode::Char('a'))),
            (3700, Some(KeyCode::Char('q'))),
        ]
    } else {
        vec![(4500, Some(KeyCode::Char('q')))]
    };
    let events = stream::iter(keys).then(|(ms, code)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(code.unwrap(), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.traffic.auto.enabled, auto);
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .count()
}

#[tokio::test]
async fn auto_refresh_off_never_requeries_on_tick() {
    assert_eq!(posts_after_far_future_tick(false).await, 1);
}

#[tokio::test]
async fn auto_refresh_on_requeries_exactly_once_when_due() {
    assert_eq!(posts_after_far_future_tick(true).await, 2);
}

#[tokio::test]
async fn load_rule_sets_command_round_trips() {
    use illumio_tui::tui::msg::Msg;
    use pce::models::PolicyScope;
    let server = MockServer::start().await;
    mount_objects(&server).await;
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
    // Neither the deny-rule fallback nor the pending list is needed here.
    for p in [
        "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules",
        "/api/v2/orgs/5/sec_policy/pending",
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .expect(0)
            .mount(&server)
            .await;
    }
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, tx) = Runtime::new(pce, m);
    tx.send(Msg::SwitchScreen(illumio_tui::tui::model::Screen::Policy))
        .unwrap();
    // Leave the screen and come back: re-entry must not request anything.
    let events = stream::iter([(600u64, '1'), (150, '2'), (300, 'q')]).then(|(ms, c)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.screen, illumio_tui::tui::model::Screen::Policy);
    assert_eq!(model.policy.scope, PolicyScope::Draft);
    assert_eq!(model.policy.rulesets.len(), 2);
    assert_eq!(model.policy.rules.len(), 4);
    assert_eq!(model.busy, 0);
}

/// Starts a connected runtime, presses `keys` (each after `ms`), and returns the model.
async fn objects_session(server: &MockServer, keys: &[(u64, char)]) -> Model {
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .expect(1)
        .mount(server)
        .await;
    let (mut rt, _tx) = Runtime::new(pce_for(server), model());
    rt.execute(Cmd::Connect);
    let events = stream::iter(keys.to_vec()).then(|(ms, c)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    });
    run_loop(&mut rt, Box::pin(events), &mut |_| {}).await
}

#[tokio::test]
async fn opening_objects_after_startup_costs_zero_requests() {
    let server = MockServer::start().await;
    mount_objects(&server).await; // each list: expect(1), the startup load only
    let model = objects_session(&server, &[(500, '3'), (300, 'q')]).await;
    assert_eq!(
        model.stats.calls, 5,
        "health + four lists, nothing for the screen"
    );
    assert!(model.objects.loaded(), "{:?}", model.objects.ages);
    assert_eq!(model.objects.labels.len(), 6);
    assert_eq!(model.objects.groups.len(), 1);
    assert_eq!(model.objects.services.len(), 3);
    assert!(model.objects.loading.iter().all(|l| !l));
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn r_on_objects_bypasses_the_cache_for_the_current_kind_only() {
    let server = MockServer::start().await;
    for (p, body, n) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
            2,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
            1,
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(n)
            .mount(&server)
            .await;
    }
    let model = objects_session(&server, &[(500, '3'), (300, 'r'), (400, 'q')]).await;
    assert_eq!(
        model.stats.calls, 6,
        "five at startup plus the labels refresh"
    );
    assert_eq!(model.objects.labels.len(), 6);
    assert_eq!(model.busy, 0);
}

async fn mount_workloads(server: &MockServer, times: u64) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn opening_workloads_loads_the_list_once_and_reopening_costs_nothing() {
    // The name index does not include workloads, so the first visit is the one
    // request for them; leaving and coming back must not repeat it.
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_workloads(&server, 1).await;
    let model = objects_session(
        &server,
        &[
            (500, '4'),
            (300, '1'),
            (100, '4'),
            (100, '3'),
            (100, '4'),
            (300, 'q'),
        ],
    )
    .await;
    assert_eq!(
        model.stats.calls, 6,
        "health + four lists + one workloads list"
    );
    assert_eq!(model.workloads.table.len(), 4);
    assert!(!model.workloads.loading);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn r_on_workloads_bypasses_the_cache_exactly_once() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_workloads(&server, 2).await; // open (Use) + r (Bypass)
    let model = objects_session(&server, &[(500, '4'), (400, 'r'), (400, 'q')]).await;
    assert_eq!(model.stats.calls, 7);
    assert_eq!(model.workloads.table.len(), 4);
    assert_eq!(model.busy, 0);
}

async fn mount_events(server: &MockServer, times: u64) {
    use wiremock::matchers::query_param;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param("max_results", "500"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn opening_events_is_one_request_and_reentry_costs_nothing() {
    // Events are not part of the startup index, so the first visit is the one
    // real request; leaving and coming back must not repeat it.
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_events(&server, 1).await;
    let model = objects_session(
        &server,
        &[
            (500, '5'),
            (300, '1'),
            (100, '5'),
            (100, '3'),
            (100, '5'),
            (300, 'q'),
        ],
    )
    .await;
    assert_eq!(
        model.stats.calls, 6,
        "health + four lists + one events list"
    );
    assert_eq!(model.events.table.len(), 4);
    assert!(model.events.window.is_some());
    assert!(!model.events.loading);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn r_on_events_bypasses_the_cache_exactly_once() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_events(&server, 2).await; // open (Use) + r (Bypass)
    let model = objects_session(&server, &[(500, '5'), (400, 'r'), (400, 'q')]).await;
    assert_eq!(model.stats.calls, 7);
    assert_eq!(model.events.table.len(), 4);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn load_events_command_round_trips() {
    use illumio_tui::tui::msg::Msg;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_events(&server, 1).await;
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, tx) = Runtime::new(pce, m);
    tx.send(Msg::SwitchScreen(illumio_tui::tui::model::Screen::Events))
        .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.events.table.len(), 4);
    assert!(model.events.window.is_some());
    assert_eq!(model.busy, 0);
}

fn two_profile_config(dir: &std::path::Path, b: &MockServer) -> std::path::PathBuf {
    let ub = url::Url::parse(&b.uri()).unwrap();
    let cfg = dir.join("config.toml");
    std::fs::write(
        &cfg,
        format!(
            "[profiles.a]\nhost = \"a.invalid\"\norg_id = 1\napi_key = \"k\"\napi_secret = \"s\"\n\n[profiles.b]\nscheme = \"http\"\nhost = \"{}\"\nport = {}\norg_id = 5\napi_key = \"k\"\napi_secret = \"s\"\n[profiles.b.limits]\ntraffic_max_results = 500\ntraffic_default_window = \"1h\"\n",
            ub.host_str().unwrap(),
            ub.port().unwrap()
        ),
    )
    .unwrap();
    cfg
}

#[tokio::test]
async fn switch_profile_rebuilds_the_connection_from_config() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::msg::Msg;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), model(), global);
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.profile.name, "b");
    assert_eq!(model.profile.org_id, 5);
    assert_eq!(model.limits.traffic_max_results, 500);
    assert_eq!(model.traffic.filter.max_results, 500);
    assert_eq!(model.traffic.filter.window, "1h");
    assert!(
        matches!(
            model.connection,
            Connection::Connected { index: Some(_), .. }
        ),
        "{:?}",
        model.connection
    );
    assert_eq!(model.stats.calls, 5, "health + four lists, nothing else");
    assert_eq!(model.busy, 0);
    assert!(
        a.received_requests().await.unwrap_or_default().is_empty(),
        "the old PCE sees nothing"
    );
    assert_eq!(
        b.received_requests().await.unwrap().len(),
        5,
        "the new PCE sees exactly the startup sequence"
    );
}

#[tokio::test]
async fn failed_switch_issues_no_request_and_keeps_the_model() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::msg::Msg;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), model(), global);
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "nope".into(),
    )))
    .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.profile.name, "t");
    assert!(model.status.text.contains("nope"), "{}", model.status.text);
    assert_eq!(model.busy, 0);
    assert!(a.received_requests().await.unwrap_or_default().is_empty());
    assert!(b.received_requests().await.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn results_from_the_old_pce_are_ignored_after_a_switch() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::msg::Msg;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(401).set_delay(Duration::from_millis(400)))
        .mount(&a)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), model(), global);
    rt.execute(Cmd::Connect); // slow, failing health on the old PCE
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(900)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert!(
        matches!(
            model.connection,
            Connection::Connected { index: Some(_), .. }
        ),
        "the late 401 from the old PCE must not disconnect: {:?}",
        model.connection
    );
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn a_label_usage_answer_from_the_old_pce_is_dropped_after_a_switch() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::model::PceUsage;
    use illumio_tui::tui::msg::Msg;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/labels_usage.json"))
                .insert_header("Content-Type", "application/json")
                .set_delay(Duration::from_millis(400)),
        )
        .expect(1)
        .mount(&a)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), model(), global);
    // `u` on the old PCE, answered slowly; the switch lands first.
    rt.execute(Cmd::LoadLabelUsage(pce::CacheMode::Use));
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(900)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.profile.name, "b");
    assert_eq!(
        model.objects.pce_usage,
        PceUsage::Unknown,
        "the old PCE's flags must not land in the new profile"
    );
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn unparsable_env_file_fails_the_switch_without_echoing_it() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::msg::Msg;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let env = dir.path().join("bad.env");
    std::fs::write(&env, "API_SECRET=abc def!\n").unwrap();
    let global = GlobalArgs {
        config: Some(cfg),
        env_file: Some(env),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), model(), global);
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.profile.name, "t", "model unchanged");
    assert!(
        model.status.text.contains("bad line"),
        "{}",
        model.status.text
    );
    assert!(
        model.status.text.contains("parse error at index"),
        "{}",
        model.status.text
    );
    assert!(
        !model.status.text.contains("abc def"),
        "{}",
        model.status.text
    );
    assert_eq!(model.busy, 0);
    assert!(a.received_requests().await.unwrap_or_default().is_empty());
    assert!(b.received_requests().await.unwrap_or_default().is_empty());
}

/// `:profile` builds the client off the event loop: a slow `api_secret_cmd`
/// must not freeze the screen. A Resize sent while the command still runs is
/// drawn before the switch lands.
#[cfg(unix)]
#[tokio::test]
async fn a_slow_profile_switch_does_not_block_the_event_loop() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::msg::Msg;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let ub = url::Url::parse(&b.uri()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(
        &cfg,
        format!(
            "[profiles.a]\nhost = \"a.invalid\"\norg_id = 1\napi_key = \"k\"\napi_secret = \"s\"\n\n[profiles.slow]\nscheme = \"http\"\nhost = \"{}\"\nport = {}\norg_id = 5\napi_key = \"k\"\napi_secret_cmd = \"sleep 1; echo s\"\n",
            ub.host_str().unwrap(),
            ub.port().unwrap()
        ),
    )
    .unwrap();
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), model(), global);
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "slow".into(),
    )))
    .unwrap();
    let events = stream::iter([
        (200u64, Event::Resize(99, 33)),
        (
            1600u64,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        ),
    ])
    .then(|(ms, e)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        e
    });
    let mut resized_before_switch = false;
    let model = run_loop(&mut rt, Box::pin(events), &mut |m| {
        if m.size == (99, 33) && m.profile.name == "t" {
            resized_before_switch = true;
        }
    })
    .await;
    assert!(
        resized_before_switch,
        "the Resize was handled while the switch was running"
    );
    assert_eq!(model.profile.name, "slow", "the switch still lands");
    assert_eq!(model.stats.calls, 5, "health + four lists on the new PCE");
    assert_eq!(model.busy, 0);
    assert!(a.received_requests().await.unwrap_or_default().is_empty());
}

/// The test model with writes on (the runtime guard is given separately).
fn writes_model() -> Model {
    let mut m = model();
    m.writes = WritesState::on();
    m
}

/// A guard like the one `run` builds, auditing into memory.
fn tui_guard(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(
        true,
        Box::new(audit),
        WriteContext {
            profile: "t".into(),
            host: "h".into(),
            org_id: 5,
            source: "tui".into(),
        },
    )
    .unwrap()
}

/// One key no screen or overlay uses, after `ms`: lets queued tasks finish.
fn wait(ms: u64) -> impl futures::Stream<Item = Event> {
    stream::once(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE))
    })
}

#[tokio::test]
async fn plan_write_needs_the_runtime_writer_and_sends_nothing() {
    use illumio_tui::tui::edit::WriteIntent;
    use illumio_tui::tui::model::Overlay;
    use pce::write::LabelDraft;
    let server = MockServer::start().await;
    let intent = WriteIntent::CreateLabel(LabelDraft::new("app", "web2"));
    // No writer: refused, although the model says writes are on.
    let (mut rt, _tx) = Runtime::new(pce_for(&server), writes_model());
    rt.execute(Cmd::PlanWrite(intent.clone()));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert_eq!(
        model.status.text, "write: writes are not enabled",
        "{:?}",
        model.status
    );
    assert_eq!(model.overlay, Overlay::None);
    assert_eq!(model.busy, 0);
    // With a writer: the confirm overlay shows the POST; planning a label sends nothing.
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(Cmd::PlanWrite(intent));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    match &model.overlay {
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            let req = plan.request().expect("a request");
            assert_eq!(req.method, pce::WriteMethod::Post);
            assert_eq!(req.path, "/orgs/5/labels");
            assert_eq!(*kind, pce::write::ObjectKind::Label);
            assert_eq!(*scroll, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(model.busy, 0);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "planning a label makes no request"
    );
    assert!(audit.entries().is_empty(), "nothing sent, nothing audited");
}

/// The four index lists, each expected `n` times (startup plus reloads).
async fn mount_lists(server: &MockServer, labels: u64, ip_lists: u64, services: u64) {
    for (p, body, n) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
            labels,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
            ip_lists,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
            services,
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(n)
            .mount(server)
            .await;
    }
}

/// Each key after its delay in ms.
fn press(keys: Vec<(u64, KeyCode)>) -> impl futures::Stream<Item = Event> {
    stream::iter(keys).then(|(ms, code)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    })
}

fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

/// Typing `text` into a form, 20 ms per character.
fn typed(text: &str) -> Vec<(u64, KeyCode)> {
    text.chars().map(|c| (20, KeyCode::Char(c))).collect()
}

/// Connects (health is mounted here), presses `keys`, returns the model.
async fn key_session(
    server: &MockServer,
    model: Model,
    writer: Option<WriteGuard>,
    keys: Vec<(u64, KeyCode)>,
) -> Model {
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .expect(1)
        .mount(server)
        .await;
    let pce = pce_for(server);
    let (mut rt, _tx) = match writer {
        Some(g) => Runtime::with_writer(pce, model, g),
        None => Runtime::new(pce, model),
    };
    rt.execute(Cmd::Connect);
    run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await
}

/// Methods the mock PCE received other than GET.
async fn writes_seen(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.method.to_string())
        .filter(|m| m != "GET")
        .collect()
}

#[tokio::test]
async fn writes_disabled_editing_keys_send_nothing() {
    use illumio_tui::tui::model::{Overlay, StatusKind};
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let keys = vec![
        (500, ch('3')),
        (50, ch('n')),
        (50, ch('e')),
        (50, ch('x')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('e')),
        (50, KeyCode::Tab),
        (300, ch('e')),
        (50, ch('x')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(model.screen, illumio_tui::tui::model::Screen::Objects);
    assert_eq!(
        model.objects.kind,
        illumio_tui::tui::model::ObjectKind::IpLists
    );
    assert_eq!(model.stats.calls, 5, "health + four lists");
    assert_eq!(
        server.received_requests().await.unwrap().len() as u64,
        model.stats.calls,
        "the IP lists tab reads the cache; the refused e and x send nothing"
    );
    assert!(writes_seen(&server).await.is_empty());
    assert_eq!(model.status.text, "writes are not enabled");
    assert_eq!(model.status.kind, StatusKind::Warn);
    assert_eq!(model.overlay, Overlay::None);
}

#[tokio::test]
async fn create_label_posts_once_audits_as_tui_and_reloads_labels_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 2, 1, 1).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(body_json(json!({"key": "app", "value": "web2"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web2"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('3')), (50, ch('n'))];
    keys.extend(typed("app"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("web2"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(model.status.text, "created /orgs/5/labels/9");
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + POST + one labels reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "tui");
    assert_eq!(entries[0].method, pce::WriteMethod::Post);
    assert_eq!(entries[0].status, Some(201));
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn update_ip_list_puts_only_the_changed_ranges() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await;
    let corp = json!({
        "href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp", "description": "Office ranges",
        "ip_ranges": [{"from_ip": "10.1.0.0", "to_ip": "10.1.255.255", "exclusion": false}],
        "fqdns": [{"fqdn": "*.corp.example"}], "update_type": "create"
    });
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(corp.clone()))
        .expect(1)
        .mount(&server)
        .await;
    // A real PCE answers 204 here and the writer re-fetches (one more GET);
    // answering with the object keeps the count to the plan's GET.
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/21"))
        .and(body_json(json!({"ip_ranges": [
            {"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"},
            {"from_ip": "10.2.0.0/16"}
        ]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(corp))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![
        (500, ch('3')),
        (100, KeyCode::BackTab),
        (200, ch('j')),
        (50, ch('e')),
        (20, KeyCode::Tab),
        (20, KeyCode::Tab),
    ];
    keys.extend(typed(", 10.2.0.0/16"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "updated /orgs/5/sec_policy/draft/ip_lists/21 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + plan GET + PUT + one IP lists reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["PUT"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn delete_service_rejected_shows_the_body_and_does_not_reload() {
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_lists(&server, 1, 1, 1).await; // services: the startup load only
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/services/30"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([
            {"token": "object_in_use", "message": "service is referenced by a rule"}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('3')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('x')), // All Services
        (300, ch('y')),
        (400, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    match &model.overlay {
        Overlay::Error(t) => assert!(
            t.contains("object_in_use: service is referenced by a rule"),
            "{t}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        writes_seen(&server).await,
        vec!["DELETE"],
        "sent once, never retried"
    );
    assert_eq!(
        model.stats.calls, 6,
        "health + four lists + DELETE, no reload"
    );
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, Some(406));
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn cancelled_confirm_sends_nothing() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('3')), (50, ch('n'))];
    keys.extend(typed("app"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("web2"));
    keys.extend([(50, KeyCode::Enter), (300, ch('n')), (200, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(model.status.text, "cancelled, nothing sent");
    assert!(writes_seen(&server).await.is_empty());
    assert!(audit.entries().is_empty());
    assert_eq!(model.stats.calls, 5);
}

#[tokio::test]
async fn a_y_typed_ahead_into_the_prompt_never_confirms_a_late_plan() {
    use illumio_tui::tui::model::{Overlay, StatusKind};
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(Cmd::Connect);
    let key = |c: KeyCode| Event::Key(KeyEvent::new(c, KeyModifiers::NONE));
    // Services, then `x`, `:` and `y` in one burst: a ready stream never
    // yields, so all three keys land before the spawned plan can answer.
    let keys = press(vec![(500, ch('3')), (50, KeyCode::Tab), (50, KeyCode::Tab)])
        .chain(stream::iter([key(ch('x')), key(ch(':')), key(ch('y'))]))
        .chain(wait(300)) // the plan lands here, on the open prompt
        .chain(press(vec![(0, ch('y')), (200, KeyCode::F(12))]));
    let model = run_loop(&mut rt, Box::pin(keys), &mut |_| {}).await;
    assert_eq!(
        model.overlay,
        Overlay::Prompt { input: "yy".into() },
        "both y went to the prompt"
    );
    assert_eq!(model.status.kind, StatusKind::Warn);
    assert_eq!(model.status.text, "request discarded, nothing sent");
    assert!(writes_seen(&server).await.is_empty(), "nothing sent");
    assert!(audit.entries().is_empty(), "nothing audited");
    assert_eq!(model.stats.calls, 5, "health + four lists");
    assert!(!model.writes.in_flight);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn switching_to_a_profile_without_writes_drops_the_guard_and_the_marker() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::model::{Overlay, StatusKind};
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::render_to_string;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), writes_model(), global);
    rt.writer = Some(Arc::new(tui_guard(MemoryAudit::new())));
    let header = |m: &Model| {
        render_to_string(m, 80, 24)
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    assert!(header(&rt.model).contains("✎ writes"));
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let keys = vec![(700, ch('3')), (100, ch('n')), (200, KeyCode::F(12))];
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(model.profile.name, "b");
    assert!(rt.writer.is_none(), "the old profile's guard is gone");
    assert!(!model.writes.enabled);
    assert!(!header(&model).contains("writes"), "{}", header(&model));
    assert_eq!(model.status.kind, StatusKind::Warn);
    assert_eq!(model.status.text, model.writes.refusal());
    assert_eq!(model.overlay, Overlay::None);
    assert_eq!(
        model.stats.calls, 5,
        "health + four lists; n made no request"
    );
    assert!(a.received_requests().await.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn pending_diff_fetches_the_active_list_once() {
    use illumio_tui::tui::diff::{DiffKind, Sign};
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await; // the draft list: the startup load only
    for (p, body, n) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json").to_string(),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/pending",
            include_str!("../../../fixtures/pending_list.json").to_string(),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/ip_lists",
            json!([{"href": "/orgs/5/sec_policy/active/ip_lists/20", "name": "Any (0.0.0.0/0 and ::/0)",
                    "ip_ranges": [{"from_ip": "0.0.0.0/0"}, {"from_ip": "::/0"}]}])
            .to_string(),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules",
            "[]".to_string(),
            0,
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(n)
            .mount(&server)
            .await;
    }
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, ch('j')), // Corp, a pending create
        (50, KeyCode::Enter),
        (300, KeyCode::Esc),
        (50, KeyCode::Enter),
        (300, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    match &model.overlay {
        Overlay::Diff { diff, scroll } => {
            assert_eq!(diff.name, "Corp");
            assert_eq!(diff.change, DiffKind::Created);
            // Every line is added, except a note that deny rules were not compared.
            assert!(
                diff.lines
                    .iter()
                    .all(|l| l.sign == Sign::Added || l.text.starts_with("deny rules not compared")),
                "{diff:?}"
            );
            assert_eq!(*scroll, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + pending + one active list; the second Enter is free"
    );
    assert_eq!(model.busy, 0);
}

/// The draft rulesets, expected `n` times (the Policy screen plus reloads);
/// the first ruleset has inline deny rules, so no fallback is asked for.
async fn mount_rule_sets(server: &MockServer, n: u64) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(n)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .expect(0)
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_ruleset_posts_once_and_reloads_the_rulesets_once() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    let body = json!({
        "name": "Lab",
        "enabled": true,
        "scopes": [[{"label": {"href": "/orgs/5/labels/1"}}]]
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/102");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('2')), (300, ch('n'))];
    keys.extend(typed("Lab"));
    keys.extend([(20, KeyCode::Tab), (20, KeyCode::Tab), (20, KeyCode::Tab)]);
    keys.extend(typed("role=web"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/102 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + POST + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "tui");
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn create_rule_from_policy_posts_once_and_reloads_once() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    let body = json!({
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/2"}}],
        "consumers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "ingress_services": [{"port": 443, "proto": 6}],
        "unscoped_consumers": false,
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        ))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('2')), (300, KeyCode::Tab), (50, ch('n'))];
    keys.extend(typed("role=db"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("role=web"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("443/tcp"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + POST + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn edit_rule_puts_only_the_changed_services() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    let rule = json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "enabled": true,
        "description": "web to db",
        "providers": [{"label": {"href": "/orgs/5/labels/2"}}],
        "consumers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "ingress_services": [{"href": "/orgs/5/sec_policy/draft/services/31"}],
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]},
        "unscoped_consumers": false, "sec_connect": false, "stateless": false,
        "machine_auth": false, "network_type": "brn", "update_type": null
    });
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let mut updated = rule;
    updated["ingress_services"] = json!([
        {"href": "/orgs/5/sec_policy/draft/services/31"},
        {"port": 443, "proto": 6}
    ]);
    Mock::given(method("PUT"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
        ))
        .and(body_json(json!({"ingress_services": [
            {"href": "/orgs/5/sec_policy/draft/services/31"},
            {"port": 443, "proto": 6}
        ]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(updated))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![
        (500, ch('2')),
        (300, KeyCode::Tab),
        (50, ch('j')), // allow rule 1 (the override deny rule sorts first)
        (50, ch('e')),
        (20, KeyCode::Tab),
        (20, KeyCode::Tab),
    ];
    keys.extend(typed(", 443/tcp"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "updated /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1 (pending)"
    );
    assert_eq!(
        model.stats.calls, 9,
        "health + four lists + rulesets + the plan's rule GET + PUT + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["PUT"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn delete_deny_rule_sends_one_delete() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    Mock::given(method("DELETE"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('2')),
        (300, KeyCode::Tab),
        (50, ch('x')), // row 0: the override deny rule 8
        (300, ch('y')),
        (400, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "deleted /orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + DELETE + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["DELETE"]);
    assert_eq!(audit.entries()[0].method, pce::WriteMethod::Delete);
    assert_eq!(model.busy, 0);
}

/// Three pending entries (served the same on every GET, expected
/// `pending_gets` times) and the provision POST with exactly `body`.
async fn mount_provision(server: &MockServer, pending_gets: u64, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"href": "/orgs/5/sec_policy/draft/rule_sets/101", "change_type": "create", "name": "Global-Infra"},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create", "name": "Corp"},
            {"href": "/orgs/5/sec_policy/draft/services/32", "change_type": "update", "name": "Web"}
        ])))
        .expect(pending_gets)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "href": "/orgs/5/sec_policy/7",
            "commit_message": "illumio-tui provision"
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn provision_marked_posts_exactly_the_marked_hrefs() {
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await; // the screen + the reload
    // Opening the list, the plan's fresh read, the reload.
    mount_provision(
        &server,
        3,
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}],
                "services": [{"href": "/orgs/5/sec_policy/draft/services/32"}]
            }
        }),
    )
    .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, ch(' ')), // Global-Infra
        (50, ch('j')),
        (50, ch('j')),
        (50, ch(' ')), // Web
        (50, ch('P')),
        (300, ch('y')),
        (500, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "provisioned /orgs/5/sec_policy/7: rule_sets=1, services=1; 3 pending changes left"
    );
    assert_eq!(
        model.overlay,
        Overlay::Pending,
        "the list reloads into its overlay"
    );
    assert_eq!(
        model.stats.calls, 11,
        "health + four lists + rulesets + pending + the plan's pending read + POST + rulesets + pending"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "/orgs/5/sec_policy");
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn provision_all_lists_every_pending_entry() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    mount_provision(
        &server,
        3,
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}],
                "services": [{"href": "/orgs/5/sec_policy/draft/services/32"}]
            }
        }),
    )
    .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, ch('P')),
        (300, ch('y')),
        (500, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "provisioned /orgs/5/sec_policy/7: ip_lists=1, rule_sets=1, services=1; 3 pending changes left"
    );
    assert_eq!(model.stats.calls, 11);
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn ruleset_diff_fetches_the_active_rulesets_once() {
    use illumio_tui::tui::diff::{DiffKind, Sign};
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 1).await; // the screen; the diff reads it from the cache
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/pending",
            include_str!("../../../fixtures/pending_list.json"),
        ),
        ("/api/v2/orgs/5/sec_policy/active/rule_sets", "[]"),
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
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, KeyCode::Enter), // Global-Infra, a pending create
        (300, KeyCode::Esc),
        (50, KeyCode::Enter),
        (300, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    match &model.overlay {
        Overlay::Diff { diff, scroll } => {
            assert_eq!(diff.name, "Global-Infra");
            assert_eq!(diff.change, DiffKind::Created);
            // Every line is added, except a note that deny rules were not compared.
            assert!(
                diff.lines
                    .iter()
                    .all(|l| l.sign == Sign::Added || l.text.starts_with("deny rules not compared")),
                "{diff:?}"
            );
            assert!(
                diff.lines
                    .iter()
                    .any(|l| l.text == "allow all ← group:All prod envs All Services"),
                "{diff:?}"
            );
            assert_eq!(*scroll, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + pending + one active rulesets GET; the second Enter is free"
    );
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn w_creates_a_rule_in_the_named_ruleset() {
    use illumio_tui::tui::test_support::model_with_flows;
    let server = MockServer::start().await;
    mount_rule_sets(&server, 2).await; // the form's drafts (the name lookup reuses them) + the reload
    let body = json!({
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
        "ingress_services": [{"port": 8443, "proto": 6}],
        "unscoped_consumers": true,
        "description": "allow 10.1.4.9 to host-1.example on 8443/tcp",
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        ))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    // Connected with the fixture index and the fixture flows already shown.
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let (mut rt, _tx) = Runtime::with_writer(pce_for(&server), m, tui_guard(audit.clone()));
    // The potentially blocked flow; the drafts arrive and pick Shop-Prod.
    let mut keys = vec![(100, ch('j')), (50, ch('w'))];
    keys.extend([(300, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9 (pending)"
    );
    assert_eq!(
        model.stats.calls, 3,
        "one rulesets GET for the form (the name reuses it), the POST, one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

/// `w` on the potentially blocked flow, `New-App` typed (unknown), `Ctrl-R`,
/// `y` on the ruleset's confirm, then `last` on the rule's. Returns the
/// model, the requests of the confirms shown (method, path, body) and the
/// audit entries.
async fn new_ruleset_session(
    server: &MockServer,
    rule_sets_gets: u64,
    last: char,
) -> (Model, Vec<(String, String, serde_json::Value)>, usize) {
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::model_with_flows;
    mount_rule_sets(server, rule_sets_gets).await; // the form's drafts + Ctrl-R's fresh check + one reload per write
    let ruleset = json!({
        "name": "New-App",
        "enabled": true,
        "scopes": [[
            {"label": {"href": "/orgs/5/labels/3"}},
            {"label": {"href": "/orgs/5/labels/4"}}
        ]]
    });
    let mut created = ruleset.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/102");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .and(body_json(ruleset))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(server)
        .await;
    let rule = json!({
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
        "ingress_services": [{"port": 8443, "proto": 6}],
        "unscoped_consumers": true,
        "description": "allow 10.1.4.9 to host-1.example on 8443/tcp",
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = rule.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/102/sec_rules/1");
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/102/sec_rules",
        ))
        .and(body_json(rule))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(if last == 'y' { 1 } else { 0 })
        .mount(server)
        .await;
    let audit = MemoryAudit::new();
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let (mut rt, _tx) = Runtime::with_writer(pce_for(server), m, tui_guard(audit.clone()));
    let plain = |ms: u64, code: KeyCode| (ms, KeyEvent::new(code, KeyModifiers::NONE));
    let mut keys = vec![plain(100, ch('j')), plain(50, ch('w'))];
    // The drafts arrive and pick Shop-Prod: cleared, a new name typed.
    keys.push((300, KeyEvent::new(ch('u'), KeyModifiers::CONTROL)));
    keys.extend(typed("New-App").into_iter().map(|(ms, c)| plain(ms, c)));

    keys.extend([
        plain(50, KeyCode::Enter),
        (300, KeyEvent::new(ch('r'), KeyModifiers::CONTROL)),
        plain(300, ch('y')),
        plain(400, ch(last)),
        plain(400, KeyCode::F(12)),
    ]);
    let events = stream::iter(keys).then(|(ms, k)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(k)
    });
    let mut confirms = Vec::new();
    let model = run_loop(&mut rt, Box::pin(events), &mut |m| {
        if let Overlay::ConfirmWrite { plan, .. } = &m.overlay
            && let Some(r) = plan.request()
        {
            let seen = (
                format!("{:?}", r.method).to_uppercase(),
                r.path.clone(),
                r.body.clone().unwrap_or_default(),
            );
            if confirms.last() != Some(&seen) {
                confirms.push(seen);
            }
        }
    })
    .await;
    (model, confirms, audit.entries().len())
}

#[tokio::test]
async fn ctrl_r_creates_the_scoped_ruleset_then_the_rule_two_confirms_two_posts() {
    let server = MockServer::start().await;
    let (model, confirms, audited) = new_ruleset_session(&server, 4, 'y').await;
    assert_eq!(
        confirms
            .iter()
            .map(|(m, p, _)| format!("{m} {p}"))
            .collect::<Vec<_>>(),
        vec![
            "POST /orgs/5/sec_policy/draft/rule_sets",
            "POST /orgs/5/sec_policy/draft/rule_sets/102/sec_rules",
        ]
    );
    assert!(confirms[0].2["scopes"].is_array(), "{:?}", confirms[0].2);
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/102/sec_rules/1 (pending)"
    );
    assert_eq!(
        model.stats.calls, 6,
        "the form's drafts (the name check reuses them), Ctrl-R's fresh check, two POSTs, one rulesets reload after each"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST", "POST"]);
    assert_eq!(audited, 2);
    assert!(model.policy.rule_after_ruleset.is_none());
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn cancelling_the_rule_after_ctrl_r_sends_only_the_ruleset() {
    let server = MockServer::start().await;
    let (model, confirms, audited) = new_ruleset_session(&server, 3, 'n').await;
    assert_eq!(confirms.len(), 2, "the rule's confirm was shown");
    assert_eq!(model.status.text, "cancelled, nothing sent");
    assert_eq!(
        model.stats.calls, 4,
        "the form's drafts, Ctrl-R's fresh check, the ruleset POST, its reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audited, 1);
    assert!(model.policy.rule_after_ruleset.is_none());
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn w_on_an_allowed_flow_sends_nothing() {
    use illumio_tui::tui::test_support::model_with_flows;
    let server = MockServer::start().await;
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let (mut rt, _tx) = Runtime::with_writer(pce_for(&server), m, tui_guard(MemoryAudit::new()));
    let keys = vec![(100, ch('w')), (200, KeyCode::F(12))]; // row 0 is allowed
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(model.status.text, "this flow is allowed; nothing to add");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

/// One fresh session on the Policy screen: `keys`, then a probe `F12`. Returns
/// the model and how many requests the mock server itself received (which must
/// equal the client's own count).
async fn policy_session(keys: &[(u64, KeyCode)]) -> (Model, usize) {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/pending_list.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let mut all = vec![(500, ch('2'))];
    all.extend_from_slice(keys);
    all.push((200, KeyCode::F(12)));
    let model = key_session(&server, model(), None, all).await;
    let seen = server.received_requests().await.unwrap().len();
    assert!(writes_seen(&server).await.is_empty());
    assert_eq!(seen as u64, model.stats.calls, "server and client agree");
    (model, seen)
}

#[tokio::test]
async fn writes_disabled_policy_and_traffic_keys_send_nothing() {
    use illumio_tui::tui::model::{Overlay, StatusKind};
    let refused = "writes are not enabled";
    // (keys before the refused key, the refused key, overlay left open)
    let tab = (300, KeyCode::Tab);
    let p = (400, ch('p'));
    type Keys = Vec<(u64, KeyCode)>;
    let cases: Vec<(&str, Keys, (u64, KeyCode), Overlay)> = vec![
        ("rulesets n", vec![], (300, ch('n')), Overlay::None),
        ("rulesets e", vec![], (300, ch('e')), Overlay::None),
        ("rulesets x", vec![], (300, ch('x')), Overlay::None),
        ("rules n", vec![tab], (300, ch('n')), Overlay::None),
        ("rules N", vec![tab], (300, ch('N')), Overlay::None),
        ("rules e", vec![tab], (300, ch('e')), Overlay::None),
        ("rules x", vec![tab], (300, ch('x')), Overlay::None),
        ("pending P", vec![p], (300, ch('P')), Overlay::Pending),
    ];
    for (name, before, key, overlay) in cases {
        // Baseline: the same session without the refused key.
        let (base, base_calls) = policy_session(&before).await;
        assert_ne!(base.status.text, refused, "{name}: baseline");
        let mut keys = before.clone();
        keys.push(key);
        let (m, calls) = policy_session(&keys).await;
        assert_eq!(m.status.text, refused, "{name}");
        assert_eq!(m.status.kind, StatusKind::Warn, "{name}");
        assert_eq!(m.overlay, overlay, "{name}");
        assert_eq!(calls, base_calls, "{name}: the key cost a request");
    }
    // Traffic `w` on a selected row of the flows fixture, loaded model-side
    // (no query request); the refusal is the writes-off path.
    use illumio_tui::tui::test_support::model_with_flows;
    let server = MockServer::start().await;
    let m = model_with_flows();
    assert!(!m.writes.enabled, "writes stay off");
    let reason = m.writes.reason.clone().expect("the fixture says why");
    let (mut rt, _tx) = Runtime::new(pce_for(&server), m);
    let keys = vec![(100, ch('j')), (100, ch('w')), (200, KeyCode::F(12))];
    let m = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(m.status.text, reason);
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(m.overlay, Overlay::None);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn opening_the_ip_lists_tab_costs_nothing_after_startup_and_r_bypasses_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await; // IP lists: the startup load and the r
    let keys = vec![
        (500, ch('3')),
        (100, KeyCode::BackTab),
        (300, ch('1')),
        (100, ch('3')),
        (300, ch('r')),
        (400, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(model.stats.calls, 6, "health + four lists + the r refresh");
    assert_eq!(model.objects.ip_lists.table.len(), 2);
    assert!(!model.objects.ip_lists.loading);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn create_ip_list_on_the_objects_ip_lists_tab_posts_once_and_reloads_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await; // IP lists: startup + the reload
    let body = json!({"name": "Lab", "ip_ranges": [{"from_ip": "10.9.0.0/16"}], "fqdns": []});
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/ip_lists/22");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('3')), (100, KeyCode::BackTab), (200, ch('n'))];
    keys.extend(typed("Lab"));
    keys.extend([(20, KeyCode::Tab), (20, KeyCode::Tab)]);
    keys.extend(typed("10.9.0.0/16"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text, "created /orgs/5/sec_policy/draft/ip_lists/22 (pending)",
        "the reload keeps the write's report"
    );
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + POST + one IP lists reload; opening the screen was a cache hit"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(
        model.objects.ip_lists.table.len(),
        2,
        "the mock list did not change"
    );
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn i_on_a_rule_detail_loads_workloads_once_and_a_second_i_is_free() {
    use illumio_tui::tui::impact::rule_detail_rows;
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 1).await;
    mount_workloads(&server, 1).await;
    let keys = vec![
        (500, ch('2')),
        (300, KeyCode::Tab),
        (50, KeyCode::Enter), // the override deny rule sorts first
        (50, ch('i')),
        (400, ch('i')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + rulesets + one workloads read"
    );
    let Overlay::RuleDetail(rule) = &model.overlay else {
        panic!("{:?}", model.overlay)
    };
    assert_eq!(
        rule_detail_rows(&model, &rule.href)[0],
        ("impact", "1 provider · 2 consumers".to_string())
    );
    assert_eq!(model.policy.rules.rows()[0].pc, "1/2");
    assert_eq!(
        model.status.text,
        "4 workloads already loaded; r on Workloads (4) refreshes them"
    );
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn w_confirm_shows_the_impact_after_i_and_y_still_posts_once() {
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{fixture_index, model_with_flows};
    let server = MockServer::start().await;
    mount_rule_sets(&server, 2).await; // the name lookup + the reload
    mount_workloads(&server, 1).await;
    let body = json!({
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
        "ingress_services": [{"port": 8443, "proto": 6}],
        "unscoped_consumers": true,
        "description": "allow 10.1.4.9 to host-1.example on 8443/tcp",
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        ))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    // The fixture flows on screen and the draft rulesets loaded: the w form
    // pre-fills Shop-Prod, and the confirm finds its scope.
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let raw = serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
    m.policy.set_rule_sets(raw, &fixture_index(), None);
    m.policy.loaded_at = Some(std::time::Instant::now());
    let (mut rt, _tx) = Runtime::with_writer(pce_for(&server), m, tui_guard(audit.clone()));
    let keys = vec![
        (100, ch('j')), // the potentially blocked flow
        (50, ch('w')),
        (50, KeyCode::Enter),
        (300, ch('i')),
        (300, ch('y')),
        (400, KeyCode::F(12)),
    ];
    let mut lines: Vec<String> = Vec::new();
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |m| {
        if let (Overlay::ConfirmWrite { .. }, Some(l)) = (&m.overlay, &m.policy.confirm_impact)
            && lines.last() != Some(l)
        {
            lines.push(l.clone());
        }
    })
    .await;
    assert_eq!(
        lines.first().map(String::as_str),
        Some("impact: press i to load workloads (1 request)")
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("impact: 1 provider · 0 consumers")
    );
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9 (pending)"
    );
    assert_eq!(
        model.stats.calls, 4,
        "the rulesets GET for the name, one workloads read, the POST, one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn a_late_revert_plan_after_a_profile_switch_is_dropped() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::edit::WriteIntent;
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::msg::Msg;
    use pce::write::RevertRequest;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    // The old PCE answers the planner's pending read late.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create", "name": "Corp"}]))
                .set_delay(Duration::from_millis(400)),
        )
        .expect(1)
        .mount(&a)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args_and_writer(
        pce_for(&a),
        writes_model(),
        global,
        tui_guard(MemoryAudit::new()),
    );
    rt.execute(Cmd::PlanWrite(WriteIntent::Revert(RevertRequest {
        all: false,
        targets: vec!["/orgs/5/sec_policy/draft/ip_lists/21".into()],
    })));
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let model = run_loop(&mut rt, Box::pin(wait(900)), &mut |_| {}).await;
    assert!(
        !matches!(model.overlay, Overlay::ConfirmWrite { .. }),
        "the old PCE's plan must not open a confirm: {:?}",
        model.overlay
    );
    assert_eq!(model.profile.name, "b");
    assert!(!model.writes.in_flight);
    assert_eq!(model.busy, 0);
    assert!(writes_seen(&a).await.is_empty());
    assert!(writes_seen(&b).await.is_empty());
    assert!(
        b.received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .all(|r| r.url.path() != "/api/v2/orgs/5/sec_policy/pending"),
        "nothing re-planned against the new PCE"
    );
}

#[tokio::test]
async fn r_on_a_partial_focus_sends_one_query_per_side() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_flows;
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let labels =
        json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]);
    for (side, body) in [
        ("in", json!({"destinations": {"include": [labels.clone()]}})),
        ("out", json!({"sources": {"include": [labels.clone()]}})),
    ] {
        let href = format!("/orgs/5/traffic_flows/async_queries/q-{side}");
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
            .and(body_partial_json(body))
            .respond_with(
                ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{href}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{href}/download")}, "matches_count": 4})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{href}/download")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                    .insert_header("Content-Type", "application/json"),
            )
            .mount(&server)
            .await;
    }
    let pce = pce_for(&server);
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    let (mut rt, tx) = Runtime::new(pce, m);
    tx.send(Msg::Command(Command::Focus(Some("shop/prod".into()))))
        .unwrap();
    let model = run_loop(
        &mut rt,
        Box::pin(press(vec![(100, ch('r')), (1500, ch('q'))])),
        &mut |_| {},
    )
    .await;
    let r = model
        .traffic
        .focus_result
        .as_ref()
        .expect("a focused result");
    assert_eq!(r.queries, 2);
    assert!(
        model.traffic.unfocused.is_some(),
        "the unfocused result is kept aside"
    );
    assert_eq!(model.traffic.loaded_flows().len(), 4, "flow 0 once");
    assert_eq!(model.busy, 0);
    assert_eq!(
        model.stats.calls,
        4 + 3 + 3,
        "4 lists for the filter, then POST + poll + download per side"
    );
}
#[tokio::test]
async fn opening_the_app_overlay_reads_workloads_and_rulesets_once() {
    use illumio_tui::tui::app_view::Loadable;
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_flows;
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param(
            "labels",
            r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#,
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
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
    // The Policy screen's own deny-rule fallback may ask for the selected ruleset's.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let (mut rt, tx) = Runtime::new(pce, model_with_flows());
    tx.send(Msg::Command(Command::App(Some("shop/prod".into()))))
        .unwrap();
    let model = run_loop(&mut rt, Box::pin(press(vec![(600, ch('q'))])), &mut |_| {}).await;
    let v = model.app_view.as_ref().expect("the overlay");
    assert_eq!(v.requests, 2);
    assert!(matches!(&v.workloads, Loadable::Loaded((rows, _)) if rows.len() == 2));
    assert!(matches!(&v.policy, Loadable::Loaded(rows) if rows.len() == 2));
    assert_eq!(model.busy, 0);
}

/// `D` end to end: the query once, then `D` re-evaluates it by its href
/// (one PUT, a poll, one download; no POST), `D` clears with no request,
/// and `D` again answers from the draft slot with no request. The PUT is
/// not a write: nothing in the write audit, even with writes armed.
#[tokio::test]
async fn d_re_evaluates_our_query_once_and_then_answers_from_the_cache() {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::Msg;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-7";
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(Ticking::new(json!({
            "status": "completed", "rules": "completed",
            "result": {"href": format!("{QHREF}/download")}, "flows_count": 4
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .and(wiremock::matchers::query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let mut flows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    for f in &mut flows {
        f["draft_policy_decision"] = json!("allowed");
    }
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(serde_json::to_string(&flows).unwrap())
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let mut m = writes_model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let audit = MemoryAudit::new();
    let (mut rt, tx) = Runtime::with_writer(pce, m, tui_guard(audit.clone()));
    tx.send(Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)))
        .unwrap();
    let mut seen = Vec::new();
    let keys = press(vec![
        (800, ch('D')),
        (800, ch('D')),
        (100, ch('D')),
        (300, ch('q')),
    ]);
    let model = run_loop(&mut rt, Box::pin(keys), &mut |m| {
        if let Some(d) = &m.traffic.data {
            seen.push(d.draft.is_some());
        }
    })
    .await;
    let data = model.traffic.data.as_ref().unwrap();
    assert!(data.draft.is_some(), "the last D loaded the draft again");
    assert_eq!(
        data.draft.unwrap().fixed,
        2,
        "the blocked and pot.blocked flows"
    );
    assert!(seen.contains(&false) && seen.contains(&true));
    let sent: Vec<String> = server
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
        .filter(|s| s.contains("traffic_flows"))
        .collect();
    let posts = sent.iter().filter(|s| s.starts_with("POST")).count();
    let puts = sent.iter().filter(|s| s.starts_with("PUT")).count();
    let downloads = sent.iter().filter(|s| s.ends_with("/download")).count();
    assert_eq!((posts, puts, downloads), (1, 1, 2), "{sent:?}");
    assert!(audit.entries().is_empty(), "update_rules is not a write");
    assert_eq!(model.busy, 0);
}

/// The query, `D` (one PUT), then a rule delete: the write dropped the
/// draft on screen and the cached draft, and evaluates the result again
/// on its own, once (a second PUT on the same query, no new POST).
#[tokio::test]
async fn a_rule_write_re_evaluates_the_draft_on_screen_once() {
    use illumio_tui::tui::filter::TrafficFilter;
    use illumio_tui::tui::msg::Msg;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-8";
    const RULE: &str = "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2";
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(Ticking::new(json!({
            "status": "completed", "rules": "completed",
            "result": {"href": format!("{QHREF}/download")}, "flows_count": 4
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/v2{RULE}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let mut m = writes_model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let audit = MemoryAudit::new();
    let guard = tui_guard(audit.clone());
    let plan = pce.writer(&guard).plan_delete_rule(RULE).unwrap();
    let (mut rt, tx) = Runtime::with_writer(pce, m, tui_guard(audit.clone()));
    tx.send(Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)))
        .unwrap();
    let first = run_loop(
        &mut rt,
        Box::pin(press(vec![(800, ch('D')), (800, KeyCode::F(12))])),
        &mut |_| {},
    )
    .await;
    assert!(first.traffic.draft_loaded(), "the first D loaded the draft");
    rt.model = first;
    rt.execute(Cmd::ExecuteWrite {
        plan,
        kind: pce::write::ObjectKind::Rule,
    });
    let model = run_loop(
        &mut rt,
        Box::pin(press(vec![(600, KeyCode::F(12)), (800, ch('q'))])),
        &mut |m| {
            if m.status.text.contains("deleted") {
                assert!(!m.traffic.draft_loaded(), "the write dropped the draft");
            }
        },
    )
    .await;
    assert!(model.traffic.draft_loaded(), "the write loaded it again");
    let sent: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.method.to_string())
        .filter(|m| m != "GET")
        .collect();
    assert_eq!(sent, vec!["POST", "PUT", "DELETE", "PUT"]);
    assert_eq!(audit.entries().len(), 1, "only the rule delete is audited");
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

#[tokio::test]
async fn execute_batch_sends_every_step_in_order_with_the_created_hrefs() {
    use illumio_tui::tui::test_support::batch_plan;
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    for (p, href) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            "/orgs/5/sec_policy/draft/ip_lists/77",
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            "/orgs/5/sec_policy/draft/rule_sets/200",
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"href": href})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        ))
        .and(body_partial_json(
            json!({"providers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/77"}}]}),
        ))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(
                json!({"href": "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    // One reload each afterwards: the rulesets, the IP lists (one was written).
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
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
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    // Planned on this runtime's profile (the guard's context).
    let mut batch = batch_plan();
    batch.origin = Some(pce::write::BatchOrigin {
        profile: "t".into(),
        host: "h".into(),
        org_id: 5,
    });
    rt.execute(Cmd::ExecuteBatch(batch));
    let model = run_loop(&mut rt, Box::pin(wait(500)), &mut |_| {}).await;
    assert!(
        model.status.text.starts_with("wrote 3 of 3 (pending)"),
        "{}",
        model.status.text
    );
    assert_eq!(audit.entries().len(), 3);
    assert!(audit.entries().iter().all(|e| e.source == "tui"));
    assert_eq!(
        model.batch_report.as_ref().unwrap()[0],
        "sent to t (h, org 5)",
        "the report names where it went"
    );
    assert_eq!(model.busy, 0);
    server.verify().await;
}

#[tokio::test]
async fn plan_batch_needs_the_runtime_writer_and_sends_nothing() {
    use illumio_tui::tui::test_support::fixture_index;
    let server = MockServer::start().await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), writes_model());
    let input = pce::write::BatchInput {
        proposal: pce::generate::Proposal {
            scopes: Vec::new(),
            targets: Vec::new(),
            ip_lists: Vec::new(),
            rules: Vec::new(),
            coverage: pce::generate::Coverage::default(),
        },
        ruleset_name: "RF-{app}-{env}".into(),
        picked: None,
    };
    rt.execute(Cmd::PlanBatch {
        input: Box::new(input),
        index: fixture_index(),
    });
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert_eq!(model.status.text, "plan: writes are not enabled");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

/// `4`, `V`, `M` to full, `Enter`, the count, `y`: one list load, then on
/// `y` the re-read (one GET per target: two), one `bulk_update`, one
/// reload; nothing before `y`.
#[tokio::test]
async fn bulk_enforcement_from_keys_re_reads_sends_once_and_reloads_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 1, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(2)
        .mount(&server)
        .await;
    // Two targets: the re-read is one GET each, not the list.
    let fixture: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    for w in fixture.iter().filter(|w| {
        ["/orgs/5/workloads/aaaa-2", "/orgs/5/workloads/aaaa-4"]
            .contains(&w["href"].as_str().unwrap())
    }) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{}", w["href"].as_str().unwrap())))
            .respond_with(ResponseTemplate::new(200).set_body_json(w.clone()))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/workloads/bulk_update"))
        .and(body_json(json!([
            {"href": "/orgs/5/workloads/aaaa-2", "enforcement_mode": "full"},
            {"href": "/orgs/5/workloads/aaaa-4", "enforcement_mode": "full"},
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"href": "/orgs/5/workloads/aaaa-2", "status": "updated"},
            {"href": "/orgs/5/workloads/aaaa-4", "status": "updated"},
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![
        (400, ch('4')),
        (300, ch('V')),
        (20, ch('M')),
        (20, ch('j')),
        (20, ch('j')),
        (20, KeyCode::Enter),
        // Refused before the count, and with a wrong one.
        (20, ch('y')),
        (20, ch('1')),
        (20, ch('y')),
        (20, KeyCode::Backspace),
        (20, ch('2')),
    ];
    keys.push((20, ch('y')));
    keys.push((600, KeyCode::F(12)));
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert!(
        model.status.text.starts_with("updated 2 of 2"),
        "{}",
        model.status.text
    );
    assert!(
        model.workloads.marked.is_empty(),
        "a sent change clears the marks"
    );
    assert!(!model.batch_sending);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "tui");
    assert_eq!(entries[0].path, "/orgs/5/workloads/bulk_update");
    assert_eq!(
        entries[0].items.as_ref().unwrap().as_array().unwrap().len(),
        2
    );
    assert!(
        model.batch_report.as_ref().unwrap()[0].starts_with("sent to t (h, org 5)"),
        "{:?}",
        model.batch_report
    );
    assert_eq!(model.busy, 0);
    server.verify().await;
}

/// The runtime without a guard: the bulk change is refused, nothing read
/// or sent, and the answer still arrives.
#[tokio::test]
async fn bulk_without_the_runtime_writer_sends_nothing() {
    let server = MockServer::start().await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), writes_model());
    let ws: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    let idx = pce::NameIndex::default();
    let targets: Vec<pce::write::BulkTarget> = ws
        .iter()
        .map(|w| pce::write::BulkTarget::of(w, &idx))
        .collect();
    let plan = pce::write::plan_bulk(
        &targets,
        &pce::write::BulkChange::enforce(pce::write::EnforcementMode::Idle, None),
        pce::models::WorkloadFilter::default(),
        Some(pce::write::BatchOrigin {
            profile: "t".into(),
            host: "h".into(),
            org_id: 5,
        }),
    )
    .unwrap();
    rt.execute(Cmd::ExecuteBulk(Box::new(plan)));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert!(
        model.status.text.starts_with("nothing sent: writes"),
        "{}",
        model.status.text
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
    assert_eq!(model.busy, 0);
}

/// A move of shop/prod's `aaaa-2` to full from the enforcement overlay,
/// with its change record (`Cmd::EnforceApp`).
fn enforce_cmd(dir: &std::path::Path) -> Cmd {
    use illumio_tui::tui::enforce::EnforceSend;
    use pce::enforcement::{ModeState, RecordOrigin, RecordReadiness};
    let ws: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    let idx = pce::NameIndex::default();
    let targets: Vec<pce::write::BulkTarget> = ws
        .iter()
        .filter(|w| w.href == "/orgs/5/workloads/aaaa-2")
        .map(|w| pce::write::BulkTarget::of(w, &idx))
        .collect();
    let plan = pce::write::plan_bulk(
        &targets,
        &pce::write::BulkChange::enforce(pce::write::EnforcementMode::Full, None),
        pce::models::WorkloadFilter::default(),
        Some(pce::write::BatchOrigin {
            profile: "t".into(),
            host: "h".into(),
            org_id: 5,
        }),
    )
    .unwrap();
    Cmd::EnforceApp {
        plan: Box::new(plan),
        send: Box::new(EnforceSend {
            app: "shop/prod".into(),
            target: ModeState::target(pce::write::EnforcementMode::Full, None),
            readiness: RecordReadiness {
                verdict: "ready".into(),
                coverage: Some(1.0),
                window: "7d".into(),
                flows: 40,
                forced: false,
            },
            dir: dir.to_path_buf(),
            origin: RecordOrigin {
                profile: "t".into(),
                pce: "h".into(),
                org_id: 5,
            },
        }),
    }
}

/// The record is written before the send (every workload pending) and
/// rewritten with the results after it; the re-read, one `bulk_update`,
/// one reload.
#[tokio::test]
async fn enforce_app_writes_the_record_around_the_bulk_send() {
    let server = MockServer::start().await;
    let fixture: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    let w2 = fixture
        .iter()
        .find(|w| w["href"] == "/orgs/5/workloads/aaaa-2")
        .unwrap()
        .clone();
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads/aaaa-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(w2))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/workloads/bulk_update"))
        .and(body_json(json!([
            {"href": "/orgs/5/workloads/aaaa-2", "enforcement_mode": "full"},
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"href": "/orgs/5/workloads/aaaa-2", "status": "updated"},
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let audit = MemoryAudit::new();
    let mut m = writes_model();
    m.records_dir = Some(dir.path().to_path_buf());
    let (mut rt, _tx) = Runtime::with_writer(pce_for(&server), m, tui_guard(audit.clone()));
    rt.execute(enforce_cmd(dir.path()));
    let model = run_loop(&mut rt, Box::pin(wait(600)), &mut |_| {}).await;
    assert!(
        model.status.text.starts_with("updated 1 of 1")
            && model.status.text.contains("record shop_prod-"),
        "{}",
        model.status.text
    );
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        files.len(),
        1,
        "one record, no temp or probe file left: {files:?}"
    );
    let rec = pce::enforcement::read_record(&files[0]).unwrap();
    assert_eq!(rec.app, "shop/prod");
    assert_eq!(rec.workloads[0].result, "updated");
    assert_eq!(
        rec.workloads[0].before.mode.as_deref(),
        Some("visibility_only")
    );
    assert_eq!(rec.readiness.as_ref().unwrap().verdict, "ready");
    assert_eq!(audit.entries().len(), 1);
    assert!(!model.writes.in_flight && !model.batch_sending);
    assert_eq!(model.busy, 0);
    server.verify().await;
}

/// A records directory that cannot be written: refused before any
/// request, nothing sent, no record.
#[tokio::test]
async fn enforce_app_without_a_writable_record_place_sends_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(enforce_cmd(&file));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert!(
        model
            .status
            .text
            .starts_with("nothing sent: change records cannot be written"),
        "{}",
        model.status.text
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
    assert!(audit.entries().is_empty());
    assert_eq!(model.busy, 0);
}

/// A rollback of `(href, mode now, mode restored)`s as one group each.
fn rollback_cmd(dir: &std::path::Path, groups: &[(&str, &str, &str)]) -> Cmd {
    use illumio_tui::tui::enforce::RollbackSend;
    use pce::enforcement::{ModeState, RecordOrigin};
    let ws: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    let idx = pce::NameIndex::default();
    let plans = groups
        .iter()
        .map(|(href, now, to)| {
            let mut w = ws.iter().find(|w| w.href == *href).unwrap().clone();
            w.enforcement_mode = Some((*now).into());
            let mode = pce::write::EnforcementMode::parse(to).unwrap();
            let plan = pce::write::plan_bulk(
                &[pce::write::BulkTarget::of(&w, &idx)],
                &pce::write::BulkChange::enforce(mode, None),
                pce::models::WorkloadFilter::default(),
                Some(pce::write::BatchOrigin {
                    profile: "t".into(),
                    host: "h".into(),
                    org_id: 5,
                }),
            )
            .unwrap();
            (ModeState::target(mode, None), plan)
        })
        .collect();
    Cmd::ExecuteRollback(Box::new(RollbackSend {
        app: "shop/prod".into(),
        record_path: dir.join("shop_prod-20261019T100000.000Z.json"),
        plans,
        dir: dir.to_path_buf(),
        origin: RecordOrigin {
            profile: "t".into(),
            pce: "h".into(),
            org_id: 5,
        },
    }))
}

/// Without the runtime's guard, or without a place for its record: the
/// rollback is refused before any request, nothing written.
#[tokio::test]
async fn rollback_sends_nothing_without_the_guard_or_a_writable_record_place() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut rt, _tx) = Runtime::new(pce_for(&server), writes_model());
    rt.execute(rollback_cmd(
        dir.path(),
        &[("/orgs/5/workloads/aaaa-2", "full", "visibility_only")],
    ));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert!(
        model
            .status
            .text
            .starts_with("rollback: nothing sent: writes off"),
        "{}",
        model.status.text
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no record"
    );
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(rollback_cmd(
        &file,
        &[("/orgs/5/workloads/aaaa-2", "full", "visibility_only")],
    ));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert!(
        model
            .status
            .text
            .contains("nothing sent: change records cannot be written"),
        "{}",
        model.status.text
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
    assert!(audit.entries().is_empty());
    assert_eq!(model.busy, 0);
}

/// A group that fails stops the rollback: the next is not sent; the
/// rollback record names the original and each workload's result.
#[tokio::test]
async fn rollback_stops_after_a_failed_group_and_records_the_results() {
    let server = MockServer::start().await;
    let fixture: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    let mut w2 = fixture
        .iter()
        .find(|w| w["href"] == "/orgs/5/workloads/aaaa-2")
        .unwrap()
        .clone();
    w2["enforcement_mode"] = json!("full");
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads/aaaa-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(w2))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads/aaaa-4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/workloads/bulk_update"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!([{"token": "invalid"}])))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(rollback_cmd(
        dir.path(),
        &[
            ("/orgs/5/workloads/aaaa-2", "full", "visibility_only"),
            ("/orgs/5/workloads/aaaa-4", "full", "selective"),
        ],
    ));
    let model = run_loop(&mut rt, Box::pin(wait(600)), &mut |_| {}).await;
    assert!(
        model.status.text.contains("an earlier group failed"),
        "{}",
        model.status.text
    );
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let rec = pce::enforcement::read_record(&files[0]).unwrap();
    assert!(
        rec.rollback_of
            .as_deref()
            .unwrap()
            .ends_with("shop_prod-20261019T100000.000Z.json")
    );
    let results: Vec<(&str, &str)> = rec
        .workloads
        .iter()
        .map(|w| (w.href.as_str(), w.result.as_str()))
        .collect();
    assert_eq!(
        results,
        vec![
            ("/orgs/5/workloads/aaaa-2", "failed"),
            ("/orgs/5/workloads/aaaa-4", "not_sent"),
        ]
    );
    assert!(!model.writes.in_flight && !model.batch_sending);
    assert_eq!(model.busy, 0);
    server.verify().await;
}

/// `r` on Watch: blocked unicast flows only, the app on either side
/// (`or`), past the cache in its own slot: a second `r` asks again.
#[tokio::test]
async fn the_watch_query_asks_for_blocked_unicast_flows_past_the_cache_each_time() {
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    const QHREF: &str = "/orgs/5/traffic_flows/async_queries/w-1";
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(json!({
            "policy_decisions": ["blocked"],
            "sources_destinations_query_op": "or",
            "destinations": {"exclude": [{"transmission": "broadcast"}, {"transmission": "multicast"}]},
        })))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 0})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, _tx) = Runtime::new(pce, m);
    for seq in [1, 2] {
        rt.execute(Cmd::WatchBlocked {
            seq,
            app: pce::traffic::parse_app("shop/prod"),
            keys: vec!["app".into(), "env".into()],
            since: jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1),
            max_results: 500,
            cancel: tokio_util::sync::CancellationToken::new(),
        });
    }
    let model = run_loop(&mut rt, Box::pin(wait(1500)), &mut |_| {}).await;
    assert!(
        model.traffic.data.is_none(),
        "the Traffic result is not touched"
    );
    assert_eq!(model.busy, 0);
    server.verify().await;
}
