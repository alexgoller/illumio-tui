use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures::{StreamExt, stream};
use illumio_tui::tui::app::{Runtime, run_loop};
use illumio_tui::tui::model::{Connection, Model, ProfileInfo};
use illumio_tui::tui::msg::Cmd;
use pce::client::{ClientConfig, PceClient};
use pce::{Credentials, Pce};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
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
    Arc::new(Pce::new(PceClient::new(cfg).unwrap()))
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
    let pce = pce_for(&server);
    let mut m = model();
    m.connection = Connection::Connected {
        index: Some(pce.name_index(pce::CacheMode::Use).await.unwrap()),
        index_age_secs: 0,
    };
    let (mut rt, tx) = Runtime::new(pce, m);
    tx.send(Msg::SwitchScreen(illumio_tui::tui::model::Screen::Policy))
        .unwrap();
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
    });
    let model = run_loop(&mut rt, Box::pin(events), &mut |_| {}).await;
    assert_eq!(model.policy.scope, PolicyScope::Draft);
    assert_eq!(model.policy.rulesets.len(), 2);
    assert_eq!(model.policy.rules.len(), 4);
    assert_eq!(model.busy, 0);
}
