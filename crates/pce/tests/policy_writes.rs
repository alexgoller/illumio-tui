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
mod rules {
    use super::*;
    use pce::models::ResolveLabelsAs;
    use pce::write::{
        ActorKind, ActorRef, DenyRuleDraft, DenyRulePatch, PortSpec, RuleDraft, RulePatch,
        ServiceRef,
    };
    use wiremock::matchers::body_json;

    const RULE_1: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1";

    fn label(id: u32) -> ActorRef {
        ActorRef::new(ActorKind::Label, format!("/orgs/5/labels/{id}"))
    }

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    fn mixed() -> RuleDraft {
        RuleDraft {
            description: Some("web tier".into()),
            enabled: true,
            providers: vec![
                label(1),
                ActorRef::new(
                    ActorKind::LabelGroup,
                    "/orgs/5/sec_policy/draft/label_groups/10",
                ),
            ],
            consumers: vec![
                ActorRef::all(),
                ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/20").excluded(),
            ],
            services: vec![
                ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into()),
                port("443/tcp"),
                port("8000-8099/tcp"),
            ],
            unscoped_consumers: false,
            resolve_labels_as: Some(ResolveLabelsAs {
                providers: vec!["workloads".into()],
                consumers: vec!["workloads".into()],
            }),
        }
    }

    fn mixed_body() -> Value {
        json!({
            "enabled": true, "description": "web tier",
            "providers": [
                {"label": {"href": "/orgs/5/labels/1"}},
                {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}
            ],
            "consumers": [
                {"actors": "ams"},
                {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}, "exclusion": true}
            ],
            "ingress_services": [
                {"href": "/orgs/5/sec_policy/draft/services/31"},
                {"port": 443, "proto": 6},
                {"port": 8000, "to_port": 8099, "proto": 6}
            ],
            "unscoped_consumers": false,
            "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
        })
    }

    /// Rule 1 with fields the models do not name, an extra key on a
    /// provider and an inline ICMP service the typed model cannot hold.
    fn rule_1() -> Value {
        json!({
            "href": RULE_1, "enabled": true, "description": "web to db",
            "providers": [{"label": {"href": "/orgs/5/labels/2"}, "exclusion": false, "future_flag": "x"}],
            "consumers": [{"label": {"href": "/orgs/5/labels/1"}}],
            "ingress_services": [
                {"href": "/orgs/5/sec_policy/draft/services/31"},
                {"proto": 1, "icmp_type": 8, "icmp_code": 0}
            ],
            "sec_connect": false, "egress_services": [], "use_workload_subnets": ["providers"]
        })
    }

    async fn mount_rule_1(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{RULE_1}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_1()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn create_posts_under_the_ruleset_with_mixed_actors_and_sends_once() {
        let server = MockServer::start().await;
        let mut created = mixed_body();
        created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
        Mock::given(method("POST"))
            .and(path(
                "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
            ))
            .and(body_json(mixed_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(created))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let audit = MemoryAudit::new();
        let guard = guard_with(audit.clone());
        let w = pce.writer(&guard);
        let plan = w
            .plan_create_rule("/orgs/5/sec_policy/active/rule_sets/100", &mixed())
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules");
        assert_eq!(req.body, Some(mixed_body()));
        assert_eq!(
            req.summary,
            "create rule in rule_set /orgs/5/sec_policy/draft/rule_sets/100"
        );
        assert_eq!(request_count(&server).await, 0, "planning sends nothing");
        let Outcome::Written(out) = w.execute(plan).await.unwrap() else {
            panic!("expected Written")
        };
        assert_eq!(
            out.href,
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9"
        );
        assert!(out.pending);
        assert!(out.dropped_fields.is_empty());
        assert_eq!(audit.entries().len(), 1);
    }

    const INLINE_REFUSAL: &str = "inline rule services take tcp or udp (optionally with a port, for example 443/tcp or tcp); for icmp and other protocols use a service object (service:<name> or an href)";

    #[tokio::test]
    async fn create_refuses_inline_icmp_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let rs = "/orgs/5/sec_policy/draft/rule_sets/100";
        for bad in ["icmp", "icmpv6"] {
            let draft = RuleDraft {
                services: vec![port(bad)],
                ..mixed()
            };
            assert_eq!(
                invalid(w.plan_create_rule(rs, &draft).unwrap_err()),
                INLINE_REFUSAL,
                "{bad}"
            );
        }
        let deny = DenyRuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::all()],
            services: vec![port("icmp")],
            override_: false,
        };
        assert_eq!(
            invalid(w.plan_create_deny_rule(rs, &deny).unwrap_err()),
            INLINE_REFUSAL
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn create_still_accepts_tcp_udp_ports_and_icmp_service_objects() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let draft = RuleDraft {
            services: vec![
                port("443/tcp"),
                port("8000-8100/udp"),
                port("tcp"),
                ServiceRef::Href("/orgs/5/sec_policy/draft/services/77".into()),
            ],
            ..mixed()
        };
        let plan = w
            .plan_create_rule("/orgs/5/sec_policy/draft/rule_sets/100", &draft)
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body.as_ref().unwrap()["ingress_services"],
            json!([
                {"port": 443, "proto": 6},
                {"port": 8000, "to_port": 8100, "proto": 17},
                {"proto": 6},
                {"href": "/orgs/5/sec_policy/draft/services/77"}
            ])
        );
    }

    #[tokio::test]
    async fn update_refuses_adding_inline_icmp_before_any_request() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let patch = RulePatch {
            add_services: vec![port("icmp")],
            ..RulePatch::default()
        };
        assert_eq!(
            invalid(w.plan_update_rule(RULE_1, &patch).await.unwrap_err()),
            INLINE_REFUSAL
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn create_validation_errors_send_nothing() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let empty = RuleDraft {
            providers: vec![],
            consumers: vec![],
            services: vec![],
            resolve_labels_as: None,
            ..mixed()
        };
        let rs = "/orgs/5/sec_policy/draft/rule_sets/100";
        assert_eq!(
            invalid(w.plan_create_rule(rs, &empty).unwrap_err()),
            "a rule needs at least one provider; a rule needs at least one consumer; a rule needs at least one service"
        );
        let excluded_all = RuleDraft {
            consumers: vec![ActorRef::all().excluded()],
            ..mixed()
        };
        assert_eq!(
            invalid(w.plan_create_rule(rs, &excluded_all).unwrap_err()),
            "all cannot be excluded"
        );
        let m = invalid(w.plan_create_rule(RULE_1, &mixed()).unwrap_err());
        assert!(m.contains("is not a rule_sets href"), "{m}");
        let deny = DenyRuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![],
            services: vec![port("22/tcp")],
            override_: false,
        };
        assert_eq!(
            invalid(w.plan_create_deny_rule(rs, &deny).unwrap_err()),
            "a rule needs at least one consumer"
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn update_add_provider_keeps_extras_and_sends_providers_whole() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            add_providers: vec![label(4)],
            ..RulePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule(
                "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/1",
                &patch,
            )
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, RULE_1);
        assert_eq!(
            req.body,
            Some(json!({"providers": [
                {"label": {"href": "/orgs/5/labels/2"}, "exclusion": false, "future_flag": "x"},
                {"label": {"href": "/orgs/5/labels/4"}}
            ]}))
        );
        assert_eq!(req.summary, format!("update rule {RULE_1}"));
        assert_eq!(request_count(&server).await, 1, "one GET of the rule");
    }

    #[tokio::test]
    async fn description_only_patch_sends_only_the_description() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            description: Some("web to db, v2".into()),
            ..RulePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule(RULE_1, &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"description": "web to db, v2"}))
        );
    }

    #[tokio::test]
    async fn removing_an_absent_consumer_is_invalid_and_nothing_is_sent() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            remove_consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/99",
            )],
            ..RulePatch::default()
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_update_rule(RULE_1, &patch)
                .await
                .unwrap_err(),
        );
        assert_eq!(
            m,
            "ip_list /orgs/5/sec_policy/draft/ip_lists/99 is not among the consumers"
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test]
    async fn empty_patches_and_wrong_href_shapes_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        assert_eq!(
            invalid(
                w.plan_update_rule(RULE_1, &RulePatch::default())
                    .await
                    .unwrap_err()
            ),
            "nothing to change"
        );
        let patch = RulePatch {
            description: Some("x".into()),
            ..RulePatch::default()
        };
        for bad in [
            "/orgs/5/sec_policy/draft/rule_sets/100",
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/7",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/../1",
            "/orgs/7/sec_policy/draft/rule_sets/100/sec_rules/1",
        ] {
            invalid(w.plan_update_rule(bad, &patch).await.unwrap_err());
            invalid(w.plan_delete_rule(bad).unwrap_err());
        }
        let deny_patch = DenyRulePatch {
            override_: Some(true),
            ..DenyRulePatch::default()
        };
        let m = invalid(
            w.plan_update_deny_rule(RULE_1, &deny_patch)
                .await
                .unwrap_err(),
        );
        assert!(
            m.contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>/deny_rules/<id>"),
            "{m}"
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn deny_rule_with_override_create_update_and_delete() {
        let server = MockServer::start().await;
        let deny_rules: Value =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        Mock::given(method("GET"))
            .and(path(
                "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(deny_rules[1].clone()))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let draft = DenyRuleDraft {
            description: Some("no telnet".into()),
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/20",
            )],
            services: vec![port("23/tcp")],
            override_: true,
        };
        let plan = w
            .plan_create_deny_rule("/orgs/5/sec_policy/draft/rule_sets/100", &draft)
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(
            req.path,
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules"
        );
        assert_eq!(
            req.body,
            Some(json!({
                "enabled": true, "description": "no telnet",
                "providers": [{"actors": "ams"}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}],
                "ingress_services": [{"port": 23, "proto": 6}],
                "override": true
            }))
        );
        let patch = DenyRulePatch {
            override_: Some(false),
            ..DenyRulePatch::default()
        };
        let plan = w
            .plan_update_deny_rule(
                "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
                &patch,
            )
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.body, Some(json!({"override": false})));
        assert_eq!(
            req.summary,
            "update deny_rule /orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8"
        );
        let plan = w
            .plan_delete_deny_rule("/orgs/5/sec_policy/active/rule_sets/100/deny_rules/8")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(
            req.path,
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8"
        );
        assert!(matches!(
            plan,
            Plan::Send {
                kind: ObjectKind::DenyRule,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn delete_of_a_nested_rule_href() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_delete_rule("/orgs/5/sec_policy/active/rule_sets/100/sec_rules/1")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, RULE_1);
        assert!(req.body.is_none());
        assert_eq!(req.summary, format!("delete rule {RULE_1}"));
        assert!(matches!(
            plan,
            Plan::Send {
                kind: ObjectKind::Rule,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_rule_that_already_matches_is_a_noop() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            enabled: Some(true),
            add_consumers: vec![label(1)],
            ..RulePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule(RULE_1, &patch)
            .await
            .unwrap();
        let Plan::Noop {
            existing_href,
            reason,
        } = plan
        else {
            panic!("expected Noop")
        };
        assert_eq!(existing_href, RULE_1);
        assert_eq!(reason, "rule already matches");
    }
}

mod provision {
    use super::*;
    use pce::write::{ProvisionRequest, ProvisionSubset, provision_counts};
    use std::collections::BTreeMap;
    use wiremock::matchers::body_json;

    fn all() -> ProvisionRequest {
        ProvisionRequest {
            description: None,
            subset: ProvisionSubset::All,
        }
    }

    fn hrefs(h: &[&str]) -> ProvisionRequest {
        ProvisionRequest {
            description: None,
            subset: ProvisionSubset::Hrefs(h.iter().map(|s| s.to_string()).collect()),
        }
    }

    async fn mount_pending(server: &MockServer, body: Value) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    fn fixture_body() -> Value {
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}]
            }
        })
    }

    #[tokio::test]
    async fn all_groups_every_pending_href_by_type_from_both_shapes() {
        for fixture in [
            include_str!("../../../fixtures/pending_dict.json"),
            include_str!("../../../fixtures/pending_list.json"),
        ] {
            let server = MockServer::start().await;
            mount_pending(&server, serde_json::from_str(fixture).unwrap()).await;
            let pce = pce_for(&server);
            let guard = guard_with(MemoryAudit::new());
            let plan = pce.writer(&guard).plan_provision(&all()).await.unwrap();
            let req = plan.request().unwrap();
            assert_eq!(req.method, WriteMethod::Post);
            assert_eq!(req.path, "/orgs/5/sec_policy");
            assert_eq!(req.body, Some(fixture_body()));
            assert_eq!(req.summary, "provision 1 ip_list, 1 rule_set");
            assert_eq!(
                provision_counts(req.body.as_ref().unwrap()),
                BTreeMap::from([("ip_lists".to_string(), 1), ("rule_sets".to_string(), 1)])
            );
            assert!(matches!(
                plan,
                Plan::Send {
                    kind: ObjectKind::Provision,
                    ..
                }
            ));
            assert_eq!(request_count(&server).await, 1, "one pending GET");
        }
    }

    #[tokio::test]
    async fn rule_hrefs_map_to_their_ruleset_once() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "change_type": "update"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "change_type": "create"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/100", "change_type": "update", "name": "Shop-Prod"},
                {"href": "/orgs/5/sec_policy/draft/services/31", "change_type": "update", "name": "PostgreSQL"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let req_all = ProvisionRequest {
            description: Some("change 42".into()),
            subset: ProvisionSubset::All,
        };
        let plan = pce.writer(&guard).plan_provision(&req_all).await.unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body,
            json!({
                "update_description": "change 42",
                "change_subset": {
                    "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/100"}],
                    "services": [{"href": "/orgs/5/sec_policy/draft/services/31"}]
                }
            })
        );
        let text = body.to_string();
        assert!(
            !text.contains("sec_rules") && !text.contains("deny_rules"),
            "{text}"
        );
        // A requested rule href provisions its ruleset.
        let plan = pce
            .writer(&guard)
            .plan_provision(&hrefs(&[
                "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/1",
            ]))
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body.as_ref().unwrap()["change_subset"],
            json!({"rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/100"}]})
        );
    }

    #[tokio::test]
    async fn explicit_hrefs_must_be_pending() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w
            .plan_provision(&hrefs(&["/orgs/5/sec_policy/active/ip_lists/21"]))
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({
                "update_description": "illumio-tui provision",
                "change_subset": {"ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}]}
            }))
        );
        assert_eq!(plan.request().unwrap().summary, "provision 1 ip_list");
        let m = invalid(
            w.plan_provision(&hrefs(&["/orgs/5/sec_policy/draft/services/31"]))
                .await
                .unwrap_err(),
        );
        assert_eq!(
            m,
            "/orgs/5/sec_policy/draft/services/31 has no pending changes"
        );
    }

    #[tokio::test]
    async fn bad_hrefs_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        for bad in [
            "/orgs/5/labels/1",
            "/orgs/5/rule_sets/100",
            "/orgs/5/sec_policy/draft/rule_sets/../ip_lists/21",
            "/orgs/5/sec_policy/draft/workloads/1",
            "/orgs/5/sec_policy/draft/labels/1",
            "/orgs/7/sec_policy/draft/ip_lists/21",
        ] {
            invalid(w.plan_provision(&hrefs(&[bad])).await.unwrap_err());
        }
        assert_eq!(
            invalid(w.plan_provision(&hrefs(&[])).await.unwrap_err()),
            "nothing to provision: name at least one href, or provision all"
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn nothing_pending_is_a_noop_and_sends_nothing() {
        for empty in [json!([]), json!({"rule_sets": [], "ip_lists": []})] {
            let server = MockServer::start().await;
            mount_pending(&server, empty).await;
            let pce = pce_for(&server);
            let audit = MemoryAudit::new();
            let guard = guard_with(audit.clone());
            let w = pce.writer(&guard);
            let plan = w.plan_provision(&all()).await.unwrap();
            let Plan::Noop {
                existing_href,
                reason,
            } = &plan
            else {
                panic!("expected Noop")
            };
            assert_eq!(existing_href, "/orgs/5/sec_policy/pending");
            assert_eq!(reason, "nothing pending");
            assert!(matches!(
                w.execute(plan).await.unwrap(),
                Outcome::Skipped { .. }
            ));
            assert!(audit.entries().is_empty());
            assert_eq!(request_count(&server).await, 1);
        }
    }

    #[tokio::test]
    async fn every_pending_type_is_grouped_by_its_object_type() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/virtual_services/7", "change_type": "update"},
                {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w.plan_provision(&all()).await.unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body["change_subset"],
            json!({
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "virtual_services": [{"href": "/orgs/5/sec_policy/draft/virtual_services/7"}]
            }),
            "--all means all: every pending type is listed under its own key"
        );
        let plan = w
            .plan_provision(&hrefs(&["/orgs/5/sec_policy/draft/ip_lists/21"]))
            .await
            .unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body["change_subset"],
            json!({"ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}]}),
            "explicit hrefs ignore unrelated pending entries"
        );
        let m = invalid(
            w.plan_provision(&hrefs(&["/orgs/5/sec_policy/draft/rule_sets/..//1"]))
                .await
                .unwrap_err(),
        );
        assert!(m.contains("rule_sets"), "{m}");
    }

    #[tokio::test]
    async fn id_less_singletons_are_grouped_and_accepted() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/firewall_settings", "change_type": "update"},
                {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w.plan_provision(&all()).await.unwrap();
        assert_eq!(
            plan.request().unwrap().body.as_ref().unwrap()["change_subset"],
            json!({
                "firewall_settings": [{"href": "/orgs/5/sec_policy/draft/firewall_settings"}],
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}]
            })
        );
        assert_eq!(request_count(&server).await, 1);
        let plan = w
            .plan_provision(&hrefs(&["/orgs/5/sec_policy/active/firewall_settings/"]))
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body.as_ref().unwrap()["change_subset"],
            json!({"firewall_settings": [{"href": "/orgs/5/sec_policy/draft/firewall_settings"}]})
        );
    }

    #[tokio::test]
    async fn bad_singleton_hrefs_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        for bad in [
            "/orgs/6/sec_policy/draft/firewall_settings",
            "/orgs/5/sec_policy/draft/../firewall_settings",
            "/orgs/5/sec_policy/draft/workloads",
            "/orgs/5/sec_policy/pending/firewall_settings",
        ] {
            invalid(w.plan_provision(&hrefs(&[bad])).await.unwrap_err());
        }
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn execute_provision_posts_once_and_invalidates_policy_and_objects() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            serde_json::from_str(include_str!("../../../fixtures/pending_dict.json")).unwrap(),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy"))
            .and(body_json(fixture_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "href": "/orgs/5/sec_policy/12", "version": 12,
                "commit_message": "illumio-tui provision"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
            .await
            .unwrap();
        pce.ip_lists(CacheMode::Use).await.unwrap();
        let audit = MemoryAudit::new();
        let guard = guard_with(audit.clone());
        let w = pce.writer(&guard);
        let plan = w.plan_provision(&all()).await.unwrap();
        let Outcome::Written(out) = w.execute(plan).await.unwrap() else {
            panic!("expected Written")
        };
        assert_eq!(out.href, "/orgs/5/sec_policy/12");
        assert!(!out.pending, "a provision is not pending");
        assert_eq!(out.status, 201);
        assert_eq!(out.object["version"], 12);
        assert!(out.dropped_fields.is_empty(), "{:?}", out.dropped_fields);
        let entries = audit.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, WriteMethod::Post);
        assert_eq!(entries[0].path, "/orgs/5/sec_policy");
        assert_eq!(entries[0].body, Some(fixture_body()));
        assert_eq!(entries[0].href.as_deref(), Some("/orgs/5/sec_policy/12"));
        assert!(
            !pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
                .await
                .unwrap()
                .hit,
            "policy: invalidated"
        );
        assert!(
            !pce.ip_lists(CacheMode::Use).await.unwrap().hit,
            "objects: invalidated"
        );
    }
}

mod managed_labels {
    use super::*;
    use pce::NameIndex;
    use pce::models::Label;
    use pce::write::{LabelRemoval, LabelsPatch, WorkloadPatch};

    /// `web01` / `host-1.example`: `managed: true`, a VEN, labels 1, 3, 4.
    fn web01() -> Value {
        let all: Value =
            serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
        all[0].clone()
    }

    fn idx() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        NameIndex::new(&labels, &[], &[], &[])
    }

    async fn mount_web01(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/aaaa-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(web01()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn relabel_of_a_managed_workload_sends_only_labels() {
        let server = MockServer::start().await;
        mount_web01(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let add = LabelsPatch {
            add_labels: vec!["/orgs/5/labels/5".into()],
            ..LabelsPatch::default()
        };
        let plan = w
            .plan_relabel_workload("/orgs/5/workloads/aaaa-1", &add, &idx())
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/workloads/aaaa-1");
        assert_eq!(
            req.body,
            Some(json!({"labels": [
                {"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"},
                {"href": "/orgs/5/labels/4"}, {"href": "/orgs/5/labels/5"}
            ]}))
        );
        assert_eq!(req.summary, "relabel workload host-1.example");
        assert!(matches!(
            plan,
            Plan::Send {
                kind: ObjectKind::Workload,
                ..
            }
        ));
        let remove = LabelsPatch {
            remove_labels: vec![LabelRemoval {
                key: "role".into(),
                value: None,
            }],
            ..LabelsPatch::default()
        };
        let plan = w
            .plan_relabel_workload("/orgs/5/workloads/aaaa-1", &remove, &idx())
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"labels": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}]}))
        );
        assert_eq!(
            request_count(&server).await,
            2,
            "one GET per plan, nothing sent"
        );
    }

    #[tokio::test]
    async fn managed_non_label_update_is_still_refused() {
        let server = MockServer::start().await;
        mount_web01(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = WorkloadPatch {
            description: Some("x".into()),
            ..WorkloadPatch::default()
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_update_workload("/orgs/5/workloads/aaaa-1", &patch, &idx())
                .await
                .unwrap_err(),
        );
        assert!(m.contains("VEN-managed"), "{m}");
    }

    #[tokio::test]
    async fn empty_or_conflicting_label_patches_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        assert_eq!(
            invalid(
                w.plan_relabel_workload(
                    "/orgs/5/workloads/aaaa-1",
                    &LabelsPatch::default(),
                    &idx()
                )
                .await
                .unwrap_err()
            ),
            "nothing to change"
        );
        let both = LabelsPatch {
            set_labels: Some(vec![]),
            add_labels: vec!["/orgs/5/labels/5".into()],
            remove_labels: vec![],
        };
        assert_eq!(
            invalid(
                w.plan_relabel_workload("/orgs/5/workloads/aaaa-1", &both, &idx())
                    .await
                    .unwrap_err()
            ),
            "set the labels or add and remove them, not both"
        );
        let add = LabelsPatch {
            add_labels: vec!["/orgs/5/labels/5".into()],
            ..LabelsPatch::default()
        };
        invalid(
            w.plan_relabel_workload("/orgs/5/labels/1", &add, &idx())
                .await
                .unwrap_err(),
        );
        assert_eq!(request_count(&server).await, 0);
    }
}
