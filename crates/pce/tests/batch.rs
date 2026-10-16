//! The generator's write batch against a mock PCE: planning reads, step
//! order, placeholders, stopping, audit. The helpers repeat those of
//! write_contract.rs (test files share no code without a module).
use pce::client::{ClientConfig, PceClient};
use pce::generate::{GenInput, PeerKind, Proposal, Stop, propose, rows};
use pce::models::{Flow, IpList, Label, LabelGroup, PolicyScope, RuleSet, Service};
use pce::traffic::{Focus, parse_app, resolve_flows};
use pce::write::{
    AuditEntry, AuditSink, Batch, BatchInput, BatchOrigin, BatchStep, MemoryAudit, ObjectKind,
    Plan, StepResult, WriteContext, WriteError, WriteGuard, WriteRequest,
};
use pce::{CacheMode, Credentials, NameIndex, Pce, WriteMethod};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn pce_for(server: &MockServer) -> Pce {
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
    cfg.timeout = Duration::from_millis(500);
    Pce::new(PceClient::new(cfg).unwrap())
}

fn guard_with(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(
        true,
        Box::new(audit),
        WriteContext {
            profile: "lab".into(),
            host: "pce.example".into(),
            org_id: 5,
            source: "test".into(),
        },
    )
    .unwrap()
}

fn ctx(profile: &str) -> WriteContext {
    WriteContext {
        profile: profile.into(),
        host: "pce.example".into(),
        org_id: 5,
        source: "test".into(),
    }
}

fn origin() -> BatchOrigin {
    BatchOrigin::of(&ctx("lab"))
}

fn index() -> NameIndex {
    let mut labels: Vec<Label> =
        serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
    let rich: Vec<Label> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_labels_rich.json")).unwrap();
    labels.extend(rich);
    let groups: Vec<LabelGroup> =
        serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
    let lists: Vec<IpList> =
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
    let services: Vec<Service> =
        serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
    NameIndex::new(&labels, &groups, &lists, &services)
}

fn rule_sets() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
}

/// shop/prod from the rich fixture, outbound on, the IP peer
/// 203.0.113.7 checked: rules into Shop-Prod (100), a new IP list and a
/// new ruleset RF-crm-prod.
fn proposal() -> Proposal {
    let idx = index();
    let raw: Vec<Flow> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows_rich.json")).unwrap();
    let flows = resolve_flows(&raw, &idx);
    let focus = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
    let mut r = rows(&flows, &focus, "role", &idx);
    for row in r.iter_mut() {
        if matches!(&row.peer, PeerKind::Ip { ip, .. } if ip == "203.0.113.7") {
            row.checked = true;
        }
    }
    let drafts: Vec<RuleSet> = serde_json::from_value(rule_sets()).unwrap();
    let input = GenInput {
        flows: &flows,
        focus: &focus,
        tier_key: "role",
        drafts: &drafts,
        idx: &idx,
        ruleset_name: "RF-{app}-{env}",
        window: "24h",
        outbound: true,
        picked: None,
    };
    propose(&r, Stop::AppPorts, &input)
}

fn input() -> BatchInput {
    BatchInput {
        proposal: proposal(),
        ruleset_name: "RF-{app}-{env}".into(),
        picked: None,
    }
}

async fn mount_reads(server: &MockServer, rule_sets: Value, ip_lists: Value) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ip_lists))
        .mount(server)
        .await;
}

fn ip_lists() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap()
}

fn shape(b: &Batch) -> Vec<String> {
    b.steps
        .iter()
        .map(|s| match &s.plan {
            Plan::Send { request, .. } => {
                format!("{} {} {:?}", request.method.as_str(), request.path, s.needs)
            }
            Plan::Noop { existing_href, .. } => format!("reuse {existing_href}"),
        })
        .collect()
}

async fn sent(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

async fn mount_created(server: &MockServer, p: &str, href: &str, times: u64) {
    Mock::given(method("POST"))
        .and(path(format!("/api/v2{p}")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"href": href})))
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn planning_reads_twice_orders_the_steps_and_sends_nothing() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(
        shape(&b),
        vec![
            "POST /orgs/5/sec_policy/draft/ip_lists []",
            "POST /orgs/5/sec_policy/draft/rule_sets []",
            "POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules []",
            "POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules []",
            "POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules [0]",
            "POST {step:1}/sec_rules [1]",
        ]
    );
    assert_eq!(b.reads, 2);
    assert_eq!(
        b.summary(),
        "6 requests: 1 ip_list, 1 rule_set, 4 rules · planned with 2 reads"
    );
    let refreshed = b.refreshed.as_ref().expect("the refreshed proposal");
    assert_eq!(refreshed.counts().create, 4, "what the steps create");
    assert!(b.drafts.is_some(), "the fresh read it was planned against");
    let lines = b.lines();
    assert!(
        lines.contains(
            &"6/6 POST <href of step 2: create rule_set RF-crm-prod>/sec_rules".to_string()
        ),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("<href of step 1: create ip_list shop-prod peer 203.0.113.7>"))
    );
    assert_eq!(
        sent(&server).await,
        vec![
            "GET /api/v2/orgs/5/sec_policy/draft/rule_sets",
            "GET /api/v2/orgs/5/sec_policy/draft/ip_lists"
        ],
        "the two fresh reads, no write"
    );
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn executing_puts_each_created_href_in_and_audits_every_step() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets",
        "/orgs/5/sec_policy/draft/rule_sets/200",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
        3,
    )
    .await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        ))
        .and(body_partial_json(json!({"unscoped_consumers": true})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(
                json!({"href": "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    // Warm both caches: the writes must drop them.
    pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap();
    let warm = pce
        .rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap();
    assert!(warm.hit);
    let out = w.execute_batch(b).await;
    assert_eq!(out.len(), 6);
    assert!(
        out.iter().all(|r| matches!(r, StepResult::Written(_))),
        "{out:?}"
    );
    let entries = audit.entries();
    assert_eq!(entries.len(), 6, "one audit entry per step sent");
    let ip_rule = &entries[4];
    assert_eq!(
        ip_rule.body.as_ref().unwrap()["providers"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/77"}}]),
        "the IP list step's href, put in before sending"
    );
    assert_eq!(
        entries[5].path,
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules"
    );
    assert!(
        entries.iter().all(|e| !e.path.contains("{step:")
            && !e
                .body
                .as_ref()
                .is_some_and(|b| b.to_string().contains("{step:"))),
        "no placeholder reached the wire"
    );
    assert!(
        !pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
            .await
            .unwrap()
            .hit,
        "policy: dropped"
    );
    server.verify().await;
}

#[tokio::test]
async fn an_existing_ip_list_is_reused_through_its_noop_href() {
    let server = MockServer::start().await;
    let mut lists = ip_lists();
    lists.as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/ip_lists/55", "name": "shop-prod peer 203.0.113.7",
        "ip_ranges": [{"from_ip": "203.0.113.7"}]
    }));
    mount_reads(&server, rule_sets(), lists).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets",
        "/orgs/5/sec_policy/draft/rule_sets/200",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
        3,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1",
        1,
    )
    .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    assert_eq!(shape(&b)[0], "reuse /orgs/5/sec_policy/draft/ip_lists/55");
    assert!(
        b.summary().ends_with("· reuses 1 object"),
        "{}",
        b.summary()
    );
    let out = w.execute_batch(b).await;
    assert!(
        matches!(&out[0], StepResult::Skipped { existing_href } if existing_href.ends_with("/55"))
    );
    let entries = audit.entries();
    assert_eq!(entries.len(), 5, "the reused list is not sent");
    assert_eq!(
        entries[3].body.as_ref().unwrap()["providers"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/55"}}])
    );
    server.verify().await;
}

#[tokio::test]
async fn the_first_failure_stops_the_batch_and_the_rest_is_not_sent() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([{"token": "name_in_use"}])))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let out = w.execute_batch(b).await;
    assert!(matches!(out[0], StepResult::Written(_)));
    assert!(
        matches!(out[1], StepResult::Failed(WriteError::Pce(_))),
        "{:?}",
        out[1]
    );
    assert!(out[2..].iter().all(|r| matches!(r, StepResult::NotSent)));
    assert_eq!(audit.entries().len(), 2, "the failure is audited too");
    server.verify().await;
}

#[tokio::test]
async fn an_unanswered_step_stops_the_batch() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .respond_with(ResponseTemplate::new(504))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let out = w.execute_batch(b).await;
    assert!(
        matches!(out[0], StepResult::Failed(WriteError::Unanswered(_))),
        "{:?}",
        out[0]
    );
    assert!(out[1..].iter().all(|r| matches!(r, StepResult::NotSent)));
    assert_eq!(audit.entries()[0].phase.as_deref(), Some("sent"));
    server.verify().await;
}

#[tokio::test]
async fn a_placeholder_naming_a_step_it_does_not_wait_for_is_refused_before_sending() {
    let server = MockServer::start().await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let step = |path: &str, body: Option<Value>, needs: Vec<usize>| BatchStep {
        plan: Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: path.into(),
                body,
                summary: "test".into(),
                objects: Vec::new(),
            },
            kind: ObjectKind::Rule,
        },
        kind: ObjectKind::Rule,
        needs,
        deny_notes: Vec::new(),
        notes: Vec::new(),
        expect: None,
    };
    let b = Batch {
        steps: vec![
            step("{step:0}/sec_rules", None, vec![0]),
            step(
                "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules",
                Some(json!({"x": "{step:0}"})),
                vec![],
            ),
        ],
        reads: 0,
        origin: Some(origin()),
        refreshed: None,
        drafts: None,
        note: None,
    };
    let out = pce.writer(&guard).execute_batch(b).await;
    let StepResult::Failed(WriteError::Invalid(m)) = &out[0] else {
        panic!("{out:?}")
    };
    assert!(
        m[0].contains("step 1 names step 1, which it does not wait for"),
        "{m:?}"
    );
    assert!(matches!(out[1], StepResult::NotSent));
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing sent"
    );
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn several_ruleset_creates_take_one_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let d = |name: &str| pce::write::RuleSetDraft {
        name: name.into(),
        description: None,
        enabled: true,
        scopes: vec![],
    };
    let plans = pce
        .writer(&guard)
        .plan_create_rule_sets_if_missing(&[d("RF-crm-prod"), d("RF-hr-prod"), d("SHOP-prod")])
        .await
        .unwrap();
    assert!(plans[0].request().is_some());
    assert!(plans[1].request().is_some());
    assert!(
        matches!(&plans[2], Plan::Noop { existing_href, .. } if existing_href.ends_with("/100")),
        "found by name, any case"
    );
    server.verify().await;
}

#[tokio::test]
async fn the_fresh_read_dedupes_again_and_a_truncated_one_is_refused() {
    // A rule someone added since: intra-app is in the draft now, the rest
    // still to write.
    let server = MockServer::start().await;
    let mut sets = rule_sets();
    sets[0]["rules"].as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50", "enabled": true,
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 5432, "proto": 6}], "unscoped_consumers": false
    }));
    mount_reads(&server, sets, ip_lists()).await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(b.sends(), 5, "the intra-app rule exists now: not created");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(rule_sets())
                .insert_header("X-Total-Count", "5000"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let err = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("too many draft rulesets"), "{err}");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "no IP list read after it"
    );
}

#[tokio::test]
async fn a_ruleset_named_like_the_new_one_but_scoped_elsewhere_is_refused() {
    let server = MockServer::start().await;
    let mut sets = rule_sets();
    sets.as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/300", "name": "RF-crm-prod", "enabled": true,
        "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}]], "rules": []
    }));
    mount_reads(&server, sets, ip_lists()).await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let err = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap_err();
    // The fresh read gives crm/prod no target (`refresh`), so the batch
    // is refused with the target's reason before anything else is read.
    assert_eq!(
        err.to_string(),
        "ruleset RF-crm-prod exists with another scope: no ruleset for crm/prod to create"
    );
    assert_eq!(
        sent(&server).await,
        vec!["GET /api/v2/orgs/5/sec_policy/draft/rule_sets"],
        "one read, no IP list read, no write"
    );
}

#[tokio::test]
async fn an_ip_list_read_that_is_truncated_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(ip_lists())
                .insert_header("X-Total-Count", "5000"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let err = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("too many IP lists"), "{err}");
}

#[tokio::test]
async fn an_ip_list_of_the_name_with_other_addresses_is_not_reused() {
    let server = MockServer::start().await;
    let mut lists = ip_lists();
    lists.as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/ip_lists/55", "name": "shop-prod peer 203.0.113.7",
        "ip_ranges": [{"from_ip": "0.0.0.0/0"}]
    }));
    mount_reads(&server, rule_sets(), lists).await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(
            "ip_list shop-prod peer 203.0.113.7 exists (/orgs/5/sec_policy/draft/ip_lists/55) but holds other addresses"
        ),
        "{err}"
    );
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn added_services_cost_one_get_of_the_rule_and_send_the_list_whole() {
    let server = MockServer::start().await;
    let rule = json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50", "enabled": true,
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 443, "proto": 6}], "unscoped_consumers": false
    });
    let mut sets = rule_sets();
    sets[0]["rules"] = json!([rule.clone()]);
    mount_reads(&server, sets, ip_lists()).await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(b.reads, 3);
    assert!(
        b.summary()
            .ends_with("planned with 3 reads · +1 GET to check rules before updating"),
        "the GET before the PUT is counted: {}",
        b.summary()
    );
    let put = b
        .steps
        .iter()
        .find_map(|s| s.plan.request().filter(|r| r.method == WriteMethod::Put))
        .expect("an update");
    assert_eq!(
        put.path,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50"
    );
    assert_eq!(
        put.body,
        Some(json!({"ingress_services": [{"port": 443, "proto": 6}, {"port": 5432, "proto": 6}]}))
    );
    assert_eq!(
        shape(&b)[2],
        "PUT /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50 []",
        "updates before creates"
    );
    server.verify().await;
}

#[tokio::test]
async fn a_created_href_is_checked_before_a_later_step_carries_it() {
    let server = MockServer::start().await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/9/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let post = |path: &str, body: Value, kind: ObjectKind, needs: Vec<usize>| BatchStep {
        plan: Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: path.into(),
                body: Some(body),
                summary: "test".into(),
                objects: Vec::new(),
            },
            kind,
        },
        kind,
        needs,
        deny_notes: Vec::new(),
        notes: Vec::new(),
        expect: None,
    };
    let b = Batch {
        steps: vec![
            post(
                "/orgs/5/sec_policy/draft/ip_lists",
                json!({"name": "x", "ip_ranges": [{"from_ip": "203.0.113.7"}]}),
                ObjectKind::IpList,
                vec![],
            ),
            post(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
                json!({"providers": [{"ip_list": {"href": "{step:0}"}}]}),
                ObjectKind::Rule,
                vec![0],
            ),
        ],
        reads: 0,
        origin: Some(origin()),
        refreshed: None,
        drafts: None,
        note: None,
    };
    let out = pce.writer(&guard).execute_batch(b).await;
    assert!(matches!(out[0], StepResult::Written(_)));
    let StepResult::Failed(WriteError::Invalid(m)) = &out[1] else {
        panic!("{out:?}")
    };
    assert!(m[0].contains("belongs to org 9"), "{m:?}");
    assert_eq!(audit.entries().len(), 1, "the rule was not sent");
    server.verify().await;
}

/// The shop/prod proposal over a draft whose intra-app rule (`ams` to
/// `ams`) already allows 443/tcp: the intra-app 5432/tcp becomes an update
/// of rule 50.
fn sets_with_rule_50() -> (Value, Value) {
    let rule = json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50", "enabled": true,
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 443, "proto": 6}], "unscoped_consumers": false
    });
    let mut sets = rule_sets();
    sets[0]["rules"] = json!([rule.clone()]);
    (sets, rule)
}

async fn mount_creates_before_the_update(server: &MockServer) {
    mount_created(
        server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    mount_created(
        server,
        "/orgs/5/sec_policy/draft/rule_sets",
        "/orgs/5/sec_policy/draft/rule_sets/200",
        1,
    )
    .await;
}

#[tokio::test]
async fn an_update_whose_rule_changed_since_planning_stops_the_batch() {
    let server = MockServer::start().await;
    let (sets, rule) = sets_with_rule_50();
    mount_reads(&server, sets, ip_lists()).await;
    let rule_path = "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50";
    Mock::given(method("GET"))
        .and(path(rule_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule.clone()))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    // A colleague adds udp/53 while the confirm is open.
    let mut changed = rule;
    changed["ingress_services"] = json!([{"port": 443, "proto": 6}, {"port": 53, "proto": 17}]);
    Mock::given(method("GET"))
        .and(path(rule_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(changed))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(rule_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    mount_creates_before_the_update(&server).await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    assert_eq!(b.steps[2].plan.request().unwrap().method, WriteMethod::Put);
    let out = w.execute_batch(b).await;
    let StepResult::Failed(WriteError::Invalid(m)) = &out[2] else {
        panic!("{out:?}")
    };
    assert!(
        m[0].contains("step 3: rule /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50 changed since planning (ingress_services)"),
        "{m:?}"
    );
    assert!(out[3..].iter().all(|r| matches!(r, StepResult::NotSent)));
    assert_eq!(audit.entries().len(), 2, "the PUT was not sent");
    server.verify().await;
}

#[tokio::test]
async fn an_unchanged_rule_is_put_after_one_more_get() {
    let server = MockServer::start().await;
    let (sets, rule) = sets_with_rule_50();
    mount_reads(&server, sets, ip_lists()).await;
    let rule_path = "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50";
    Mock::given(method("GET"))
        .and(path(rule_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(rule_path))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50"}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    mount_creates_before_the_update(&server).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
        2,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1",
        1,
    )
    .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let out = w.execute_batch(b).await;
    assert!(
        out.iter().all(|r| matches!(r, StepResult::Written(_))),
        "{out:?}"
    );
    server.verify().await;
}

#[tokio::test]
async fn services_two_rules_add_to_one_rule_go_in_one_update() {
    let server = MockServer::start().await;
    let (sets, rule) = sets_with_rule_50();
    mount_reads(&server, sets, ip_lists()).await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule))
        .expect(1)
        .mount(&server)
        .await;
    let mut input = input();
    let intra = input
        .proposal
        .rules
        .iter()
        .find(|r| {
            r.services
                == vec![pce::write::ServiceRef::Port(
                    pce::write::PortSpec::parse("5432/tcp").unwrap(),
                )]
        })
        .expect("the intra-app rule")
        .clone();
    let mut redis = intra;
    redis.services = vec![pce::write::ServiceRef::Port(
        pce::write::PortSpec::parse("6379/tcp").unwrap(),
    )];
    input.proposal.rules.push(redis);
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let b = pce
        .writer(&guard)
        .plan_batch(input, &index())
        .await
        .unwrap();
    let puts: Vec<&WriteRequest> = b
        .steps
        .iter()
        .filter_map(|s| s.plan.request().filter(|r| r.method == WriteMethod::Put))
        .collect();
    assert_eq!(puts.len(), 1, "one update for rule 50");
    assert_eq!(
        puts[0].body,
        Some(json!({"ingress_services": [
            {"port": 443, "proto": 6}, {"port": 5432, "proto": 6}, {"port": 6379, "proto": 6}
        ]}))
    );
    assert_eq!(b.reads, 3, "one GET of rule 50");
    server.verify().await;
}

#[tokio::test]
async fn a_batch_planned_on_another_profile_is_not_sent() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    let pce = pce_for(&server);
    let b = pce
        .writer(&guard_with(MemoryAudit::new()))
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(b.origin, Some(origin()));
    let audit = MemoryAudit::new();
    let other = WriteGuard::new(true, Box::new(audit.clone()), ctx("other")).unwrap();
    let out = pce.writer(&other).execute_batch(b).await;
    let StepResult::Failed(WriteError::Invalid(m)) = &out[0] else {
        panic!("{out:?}")
    };
    assert!(
        m[0].contains(
            "planned on lab (pce.example, org 5), this session writes to other (pce.example, org 5): nothing sent"
        ),
        "{m:?}"
    );
    assert!(out[1..].iter().all(|r| matches!(r, StepResult::NotSent)));
    assert_eq!(sent(&server).await.len(), 2, "the plan's two reads only");
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn a_placeholder_in_free_text_is_sent_as_typed() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets",
        "/orgs/5/sec_policy/draft/rule_sets/200",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
        3,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1",
        1,
    )
    .await;
    // A label value that reads like a placeholder ends up in a description.
    let mut input = input();
    for r in input.proposal.rules.iter_mut() {
        r.description = "from app {step:5} and {step:0}".into();
    }
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input, &index()).await.unwrap();
    assert!(
        b.lines()
            .iter()
            .any(|l| l.contains("from app {step:5} and {step:0}")),
        "the confirm shows the text as it is sent"
    );
    let out = w.execute_batch(b).await;
    assert!(
        out.iter().all(|r| matches!(r, StepResult::Written(_))),
        "{out:?}"
    );
    let entries = audit.entries();
    let ip_rule = entries[4].body.as_ref().unwrap();
    assert_eq!(ip_rule["description"], "from app {step:5} and {step:0}");
    assert_eq!(
        ip_rule["providers"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/77"}}])
    );
    server.verify().await;
}

/// An audit log that takes nothing: the disk is full.
struct FullDisk;

impl AuditSink for FullDisk {
    fn check_writable(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn append(&self, _: &AuditEntry) -> std::io::Result<()> {
        Err(std::io::Error::other("no space left on device"))
    }
    fn location(&self) -> String {
        "audit.jsonl".into()
    }
}

#[tokio::test]
async fn a_write_whose_audit_entry_failed_stops_the_batch() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = WriteGuard::new(true, Box::new(FullDisk), ctx("lab")).unwrap();
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let out = w.execute_batch(b).await;
    let StepResult::Written(first) = &out[0] else {
        panic!("{out:?}")
    };
    assert!(
        first
            .audit_failed
            .as_deref()
            .is_some_and(|e| e.contains("no space left on device")),
        "{first:?}"
    );
    assert!(out[1..].iter().all(|r| matches!(r, StepResult::NotSent)));
    server.verify().await;
}

#[tokio::test]
async fn rules_an_override_deny_blocks_are_named_in_the_confirm() {
    let server = MockServer::start().await;
    let mut sets = rule_sets();
    sets[0]["deny_rules"] = json!([{
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "enabled": true,
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 5432, "proto": 6}], "override": true
    }]);
    mount_reads(&server, sets, ip_lists()).await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert!(b.denied_rules() >= 1, "{:#?}", b.lines());
    assert!(
        b.summary().contains("blocked by override denies"),
        "{}",
        b.summary()
    );
    let lines = b.lines();
    assert!(
        lines.iter().any(|l| l.starts_with("  ! blocked: ")
            && l.ends_with("by override deny /orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn a_checked_list_row_names_its_list_creates_none_and_the_confirm_lists_its_ips() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let mut input = input();
    let idx = index();
    let raw: Vec<Flow> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows_rich.json")).unwrap();
    let flows = resolve_flows(&raw, &idx);
    let focus = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
    let mut r = rows(&flows, &focus, "role", &idx);
    for row in r.iter_mut() {
        if matches!(&row.peer, PeerKind::IpList { name, .. } if name == "Corp") {
            row.checked = true;
        }
    }
    let drafts: Vec<RuleSet> = serde_json::from_value(rule_sets()).unwrap();
    let gen_input = GenInput {
        flows: &flows,
        focus: &focus,
        tier_key: "role",
        drafts: &drafts,
        idx: &idx,
        ruleset_name: "RF-{app}-{env}",
        window: "24h",
        outbound: false,
        picked: None,
    };
    input.proposal = propose(&r, Stop::AppPorts, &gen_input);
    assert!(input.proposal.ip_lists.is_empty());
    let b = pce.writer(&guard).plan_batch(input, &idx).await.unwrap();
    assert_eq!(b.reads, 1, "no IP list read: none is created");
    assert!(
        b.steps
            .iter()
            .all(|s| s.kind != pce::write::ObjectKind::IpList),
        "{:#?}",
        b.lines()
    );
    let lines = b.lines();
    assert!(
        lines.contains(&"  · ipl:Corp: 1 IP seen: 10.1.4.9".to_string()),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn each_step_is_handed_over_as_it_finishes() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([{"token": "name_in_use"}])))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let mut seen: Vec<(usize, &'static str)> = Vec::new();
    w.execute_batch_each(b, &mut |i, r| {
        seen.push((
            i,
            match r {
                StepResult::Written(_) => "written",
                StepResult::Skipped { .. } => "skipped",
                StepResult::Failed(_) => "failed",
                StepResult::NotSent => "not sent",
            },
        ))
    })
    .await;
    assert_eq!(
        seen,
        vec![
            (0, "written"),
            (1, "failed"),
            (2, "not sent"),
            (3, "not sent"),
            (4, "not sent"),
            (5, "not sent")
        ]
    );
    server.verify().await;
}
