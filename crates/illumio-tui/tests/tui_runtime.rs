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
    assert_eq!(model.objects.ip_lists.len(), 2);
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
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(model.stats.calls, 5, "health + four lists");
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
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('j')),
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
    let keys = press(vec![
        (500, ch('3')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
    ])
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
            assert!(diff.lines.iter().all(|l| l.sign == Sign::Added), "{diff:?}");
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
