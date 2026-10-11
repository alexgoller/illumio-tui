//! Policy writes: rulesets, rules, deny rules, provisioning, managed
//! labels. The helpers repeat those of write_contract.rs (test files share
//! no code without a module).
#![allow(dead_code)]
use pce::client::{ClientConfig, PceClient};
use pce::models::PolicyScope;
use pce::write::{
    MemoryAudit, ObjectKind, Outcome, Plan, WriteContext, WriteError, WriteGuard, WriteRequest,
};
use pce::{CacheMode, Credentials, Pce, WriteMethod};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
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

fn ctx() -> WriteContext {
    WriteContext {
        profile: "lab".into(),
        host: "pce.example".into(),
        org_id: 5,
        source: "test".into(),
    }
}

fn guard_with(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(true, true, Box::new(audit), ctx()).unwrap()
}

fn send_plan(method: WriteMethod, path: &str, body: Option<Value>, kind: ObjectKind) -> Plan {
    Plan::Send {
        request: WriteRequest {
            method,
            path: path.into(),
            body,
            summary: "test".into(),
        },
        kind,
    }
}

/// The messages of an `Invalid`, joined; panics on any other error.
fn invalid(err: WriteError) -> String {
    match err {
        WriteError::Invalid(m) => m.join("; "),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn rule_sets() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test]
async fn execute_of_a_rule_set_plan_invalidates_policy_and_marks_pending() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"href": "/orgs/5/sec_policy/draft/rule_sets/102", "name": "New"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap(); // warm the cache
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/sec_policy/draft/rule_sets",
            Some(json!({"name": "New"})),
            ObjectKind::RuleSet,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert!(w.pending, "rulesets wait for provisioning");
    assert_eq!(w.href, "/orgs/5/sec_policy/draft/rule_sets/102");
    let again = pce
        .rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap();
    assert!(!again.hit, "policy: was invalidated");
    assert_eq!(audit.entries().len(), 1);
}
