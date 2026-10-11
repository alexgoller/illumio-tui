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

mod rule_sets {
    use super::*;
    use pce::write::{ActorKind, ActorRef, IfMissing, RuleSetDraft, RuleSetPatch};

    fn label(id: u32) -> ActorRef {
        ActorRef::new(ActorKind::Label, format!("/orgs/5/labels/{id}"))
    }

    fn group() -> ActorRef {
        ActorRef::new(
            ActorKind::LabelGroup,
            "/orgs/5/sec_policy/draft/label_groups/10",
        )
    }

    fn named(name: &str) -> RuleSetDraft {
        RuleSetDraft {
            name: name.into(),
            description: None,
            enabled: true,
            scopes: vec![],
        }
    }

    async fn mount_list(server: &MockServer, body: Value) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    async fn mount_shop_prod(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()[0].clone()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn create_body_has_two_scopes_and_unscoped_sends_one_empty_scope() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = RuleSetDraft {
            name: " Web-Tier ".into(),
            description: Some("web".into()),
            enabled: true,
            scopes: vec![vec![label(3), label(4)], vec![group()]],
        };
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&d, IfMissing::Create)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets");
        assert_eq!(
            req.body,
            Some(json!({
                "name": "Web-Tier", "description": "web", "enabled": true,
                "scopes": [
                    [{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}],
                    [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]
                ]
            }))
        );
        assert_eq!(req.summary, "create rule_set Web-Tier");
        let unscoped = RuleSetDraft {
            enabled: false,
            ..named("Global")
        };
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&unscoped, IfMissing::Create)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"name": "Global", "enabled": false, "scopes": [[]]}))
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn a_scope_with_an_ip_list_or_an_empty_name_is_refused_locally() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = RuleSetDraft {
            scopes: vec![vec![
                label(3),
                ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/21"),
            ]],
            ..named("X")
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_create_rule_set(&d, IfMissing::SkipExisting)
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "scopes take labels and label groups only");
        let m = invalid(
            pce.writer(&guard)
                .plan_create_rule_set(&named("  "), IfMissing::SkipExisting)
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "ruleset name must not be empty");
        let patch = RuleSetPatch {
            set_scopes: Some(vec![vec![ActorRef::all()]]),
            ..RuleSetPatch::default()
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "scopes take labels and label groups only");
        assert_eq!(
            request_count(&server).await,
            0,
            "refused before any request"
        );
    }

    #[tokio::test]
    async fn if_missing_matches_names_case_insensitively_and_skips_pending_deletes() {
        let server = MockServer::start().await;
        mount_list(&server, rule_sets()).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&named("shop-prod"), IfMissing::SkipExisting)
            .await
            .unwrap();
        let Plan::Noop {
            existing_href,
            reason,
        } = plan
        else {
            panic!("expected Noop")
        };
        assert_eq!(existing_href, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(reason, "rule_set shop-prod exists");
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&named("Web-Tier"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some());
        assert_eq!(
            request_count(&server).await,
            2,
            "one GET per lookup, never cached"
        );
    }

    #[tokio::test]
    async fn if_missing_refuses_ambiguity() {
        let server = MockServer::start().await;
        mount_list(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/rule_sets/200", "name": "Dup"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/201", "name": "dup"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/202", "name": "Gone", "update_type": "delete"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let m = invalid(
            pce.writer(&guard)
                .plan_create_rule_set(&named("DUP"), IfMissing::SkipExisting)
                .await
                .unwrap_err(),
        );
        assert!(
            m.contains("/orgs/5/sec_policy/draft/rule_sets/200")
                && m.contains("/orgs/5/sec_policy/draft/rule_sets/201"),
            "{m}"
        );
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&named("gone"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some(), "a pending delete does not count");
    }

    #[tokio::test]
    async fn update_sends_only_changed_keys_and_never_rules() {
        let server = MockServer::start().await;
        mount_shop_prod(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let patch = RuleSetPatch {
            name: Some("Shop-Prod".into()),
            description: Some("Ringfence v2".into()),
            enabled: Some(true),
            set_scopes: None,
        };
        let plan = w
            .plan_update_rule_set("/orgs/5/sec_policy/active/rule_sets/100", &patch)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(req.body, Some(json!({"description": "Ringfence v2"})));
        assert_eq!(req.summary, "update rule_set Shop-Prod");
        let patch = RuleSetPatch {
            set_scopes: Some(vec![vec![label(5)]]),
            ..RuleSetPatch::default()
        };
        let plan = w
            .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"scopes": [[{"label": {"href": "/orgs/5/labels/5"}}]]}))
        );
        let patch = RuleSetPatch {
            enabled: Some(false),
            set_scopes: Some(vec![]),
            ..RuleSetPatch::default()
        };
        let plan = w
            .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"enabled": false, "scopes": [[]]}))
        );
        assert_eq!(request_count(&server).await, 3, "one GET per plan");
    }

    #[tokio::test]
    async fn an_update_that_changes_nothing_is_a_noop_and_an_empty_patch_is_refused() {
        let server = MockServer::start().await;
        mount_shop_prod(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RuleSetPatch {
            name: Some("Shop-Prod".into()),
            enabled: Some(true),
            ..RuleSetPatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
            .await
            .unwrap();
        let Plan::Noop {
            existing_href,
            reason,
        } = plan
        else {
            panic!("expected Noop")
        };
        assert_eq!(existing_href, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(reason, "rule_set Shop-Prod already matches");
        let m = invalid(
            pce.writer(&guard)
                .plan_update_rule_set(
                    "/orgs/5/sec_policy/draft/rule_sets/100",
                    &RuleSetPatch::default(),
                )
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "nothing to change");
        assert_eq!(request_count(&server).await, 1);
    }

    #[tokio::test]
    async fn delete_and_href_shapes() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w
            .plan_delete_rule_set("/orgs/5/sec_policy/active/rule_sets/100")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert!(req.body.is_none());
        assert_eq!(
            req.summary,
            "delete rule_set /orgs/5/sec_policy/draft/rule_sets/100"
        );
        let patch = RuleSetPatch {
            description: Some("x".into()),
            ..RuleSetPatch::default()
        };
        for bad in [
            "/orgs/5/rule_sets/100",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
            "/orgs/5/sec_policy/draft/ip_lists/21",
            "/orgs/5/sec_policy/draft/rule_sets/../labels/1",
        ] {
            invalid(w.plan_delete_rule_set(bad).unwrap_err());
            invalid(w.plan_update_rule_set(bad, &patch).await.unwrap_err());
        }
        assert_eq!(request_count(&server).await, 0);
    }
}
