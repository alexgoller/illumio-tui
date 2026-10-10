use pce::client::{ClientConfig, PceClient};
use pce::write::{
    IfMissing, MemoryAudit, ObjectKind, Outcome, Plan, WriteContext, WriteError, WriteGuard,
    WriteRefused, WriteRequest,
};
use pce::{CacheMode, Credentials, Pce, PceError, WriteMethod};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_json, method, path};
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
    cfg.timeout = Duration::from_millis(500);
    Pce::new(PceClient::new(cfg).unwrap())
}

pub fn ctx() -> WriteContext {
    WriteContext {
        profile: "lab".into(),
        host: "pce.example".into(),
        org_id: 5,
        source: "test".into(),
    }
}

pub fn guard_with(audit: Arc<MemoryAudit>) -> WriteGuard {
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

#[test]
fn guard_refuses_without_profile_or_flag() {
    let a = MemoryAudit::new();
    assert!(matches!(
        WriteGuard::new(false, true, Box::new(a.clone()), ctx()).unwrap_err(),
        WriteRefused::ProfileDenies
    ));
    assert!(matches!(
        WriteGuard::new(true, false, Box::new(a.clone()), ctx()).unwrap_err(),
        WriteRefused::FlagMissing
    ));
    assert!(WriteGuard::new(true, true, Box::new(a), ctx()).is_ok());
}

#[tokio::test]
async fn execute_posts_once_audits_and_invalidates_objects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(body_json(json!({"key": "app", "value": "web"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    pce.labels(CacheMode::Use).await.unwrap(); // warm the cache
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({"key": "app", "value": "web"})),
            ObjectKind::Label,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.href, "/orgs/5/labels/9");
    assert_eq!(w.status, 201);
    assert!(!w.pending, "labels are not provisionable");
    assert_eq!(w.object["key"], "app");
    assert!(w.dropped_fields.is_empty());
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].method, WriteMethod::Post);
    assert_eq!(entries[0].path, "/orgs/5/labels");
    assert_eq!(entries[0].status, Some(201));
    assert_eq!(entries[0].href.as_deref(), Some("/orgs/5/labels/9"));
    assert_eq!(entries[0].profile, "lab");
    assert_eq!(entries[0].source, "test");
    assert!(entries[0].phase.is_none());
    // The objects cache was invalidated: the next read hits the PCE again.
    let c = pce.labels(CacheMode::Use).await.unwrap();
    assert!(!c.hit);
}

#[tokio::test]
async fn execute_refetches_after_204_and_marks_policy_objects_pending() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "Corp",
            "ip_ranges": [{"from_ip": "10.0.0.0/8"}], "fqdns": [], "update_type": "update"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Put,
            "/orgs/5/sec_policy/draft/ip_lists/3",
            Some(json!({"name": "Corp", "ip_ranges": [{"from_ip": "10.0.0.0/8"}], "fqdns": []})),
            ObjectKind::IpList,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.status, 204);
    assert!(w.pending);
    assert_eq!(w.object["name"], "Corp");
    assert_eq!(w.href, "/orgs/5/sec_policy/draft/ip_lists/3");
    assert_eq!(audit.entries()[0].status, Some(204));
}

#[tokio::test]
async fn execute_reports_fields_the_pce_dropped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "PG", "service_ports": []
        })))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/sec_policy/draft/services",
            Some(json!({"name": "PG", "service_ports": [{"port": 5432, "proto": 6}]})),
            ObjectKind::Service,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.dropped_fields, vec!["service_ports".to_string()]);
}

#[tokio::test]
async fn delete_returns_the_request_href_and_no_object() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/labels/9"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Delete,
            "/orgs/5/labels/9",
            None,
            ObjectKind::Label,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.href, "/orgs/5/labels/9");
    assert!(w.object.is_null());
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "no re-fetch after delete"
    );
    assert_eq!(audit.entries()[0].method, WriteMethod::Delete);
}

#[tokio::test]
async fn rejected_write_is_audited_with_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(
            ResponseTemplate::new(406)
                .set_body_json(json!([{"token": "label_in_use", "message": "Label is in use"}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, WriteError::Pce(PceError::Rejected { status: 406, .. })),
        "{err:?}"
    );
    assert_eq!(err.kind(), "rejected");
    let e = &audit.entries()[0];
    assert_eq!(e.status, Some(406));
    assert!(e.error.as_deref().unwrap().contains("label_in_use"));
}

#[tokio::test]
async fn gateway_status_on_a_write_is_reported_as_unanswered() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, WriteError::Unanswered(_)), "{err:?}");
    assert!(err.to_string().contains("write may have applied"));
    let e = &audit.entries()[0];
    assert_eq!(e.phase.as_deref(), Some("sent"));
    assert_eq!(e.status, Some(502));
}

#[tokio::test]
async fn timeout_on_a_write_is_unanswered_and_audited_as_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_secs(3)))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, WriteError::Unanswered(PceError::Timeout(_))),
        "{err:?}"
    );
    assert_eq!(err.kind(), "write_unanswered");
    let e = &audit.entries()[0];
    assert_eq!(e.phase.as_deref(), Some("sent"));
    assert!(e.status.is_none());
    assert!(e.error.is_some());
}

#[tokio::test]
async fn noop_plan_sends_nothing_and_logs_nothing() {
    let server = MockServer::start().await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(Plan::Noop {
            existing_href: "/orgs/5/labels/1".into(),
            reason: "label app=web exists".into(),
        })
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Skipped { .. }));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn unwritable_audit_refuses_before_sending() {
    struct Broken;
    impl pce::write::AuditSink for Broken {
        fn check_writable(&self) -> std::io::Result<()> {
            Err(std::io::Error::other("disk full"))
        }
        fn append(&self, _: &pce::write::AuditEntry) -> std::io::Result<()> {
            Ok(())
        }
        fn location(&self) -> String {
            "broken".into()
        }
    }
    let server = MockServer::start().await;
    let pce = pce_for(&server);
    let err = WriteGuard::new(true, true, Box::new(Broken), ctx()).unwrap_err();
    assert!(matches!(err, WriteRefused::AuditUnwritable(_)), "{err:?}");
    assert!(server.received_requests().await.unwrap().is_empty());
    let _ = IfMissing::SkipExisting; // used by later tasks; keeps the import honest
    let _ = pce;
}

#[test]
fn draft_href_rewrites_active_and_refuses_other_scopes() {
    use pce::write::draft_href;
    assert_eq!(
        draft_href("/orgs/5/sec_policy/active/ip_lists/3").unwrap(),
        "/orgs/5/sec_policy/draft/ip_lists/3"
    );
    assert_eq!(draft_href("/orgs/5/labels/1").unwrap(), "/orgs/5/labels/1");
    assert!(matches!(
        draft_href("/orgs/5/sec_policy/42/ip_lists/3").unwrap_err(),
        WriteError::Invalid(_)
    ));
}

#[tokio::test]
async fn failed_refetch_after_204_is_a_warning_not_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Put,
            "/orgs/5/sec_policy/draft/ip_lists/3",
            Some(json!({"name": "Corp"})),
            ObjectKind::IpList,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert!(w.object.is_null());
    assert!(
        w.warnings[0].contains("write applied; re-fetch of"),
        "{:?}",
        w.warnings
    );
    assert_eq!(audit.entries()[0].status, Some(204));
}

#[tokio::test]
async fn non_json_2xx_body_is_unanswered() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(201).set_body_string("not json"))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, WriteError::Unanswered(_)), "{err:?}");
    assert_eq!(audit.entries()[0].phase.as_deref(), Some("sent"));
}

struct FailingAppend;
impl pce::write::AuditSink for FailingAppend {
    fn check_writable(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn append(&self, _: &pce::write::AuditEntry) -> std::io::Result<()> {
        Err(std::io::Error::other("disk full"))
    }
    fn location(&self) -> String {
        "failing".into()
    }
}

#[tokio::test]
async fn audit_append_failure_is_a_warning_on_success_and_audit_failed_on_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"href": "/orgs/5/labels/9"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([{"token": "x"}])))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = WriteGuard::new(true, true, Box::new(FailingAppend), ctx()).unwrap();
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert!(
        w.warnings[0].contains("audit entry could not be written to failing"),
        "{:?}",
        w.warnings
    );
    let err = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/sec_policy/draft/services",
            Some(json!({})),
            ObjectKind::Service,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, WriteError::AuditFailed { .. }), "{err:?}");
    assert_eq!(err.kind(), "rejected");
}

#[tokio::test]
async fn auth_failure_is_audited_with_its_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let _ = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap_err();
    assert_eq!(audit.entries()[0].status, Some(401));
}

#[tokio::test]
async fn post_without_href_does_not_refetch_the_collection() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({})),
            ObjectKind::Label,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(
        w.warnings.iter().any(|m| m.contains("no href")),
        "{:?}",
        w.warnings
    );
}

mod labels {
    use super::*;
    use pce::write::LabelDraft;

    #[test]
    fn label_draft_validates_and_trims() {
        let d = LabelDraft::new(" app ", " web ");
        assert_eq!(d.display(), "app=web");
        assert!(d.validate().is_ok());
        let errs = LabelDraft::new("", " ").validate().unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
    }

    #[tokio::test]
    async fn create_plan_without_if_missing_makes_no_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "web"), IfMissing::Create)
            .await
            .unwrap();
        let req = plan.request().expect("send plan").clone();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/labels");
        assert_eq!(req.body, Some(json!({"key": "app", "value": "web"})));
        assert_eq!(req.summary, "create label app=web");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn if_missing_bypasses_the_cache_and_skips_an_exact_match() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"href": "/orgs/5/labels/1", "key": "app", "value": "web"},
                {"href": "/orgs/5/labels/2", "key": "app", "value": "webshop"}
            ])))
            .expect(3)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        pce.labels(CacheMode::Use).await.unwrap(); // warm: the writer must not use this
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "web"), IfMissing::SkipExisting)
            .await
            .unwrap();
        match plan {
            Plan::Noop { existing_href, .. } => assert_eq!(existing_href, "/orgs/5/labels/1"),
            other => panic!("expected Noop, got {other:?}"),
        }
        // "web" is a substring of "webshop": only the exact match counts.
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "we"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some());
    }

    #[tokio::test]
    async fn if_missing_ignores_deleted_labels() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"href": "/orgs/5/labels/1", "key": "app", "value": "web", "deleted": true}
            ])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "web"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(
            plan.request().is_some(),
            "a deleted label does not count as existing"
        );
    }

    #[tokio::test]
    async fn update_and_delete_plans() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let up = w.plan_update_label("/orgs/5/labels/9", "prod").unwrap();
        let req = up.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/labels/9");
        assert_eq!(req.body, Some(json!({"value": "prod"})));
        let del = w.plan_delete_label("/orgs/5/labels/9").unwrap();
        let req = del.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.body, None);
        assert_eq!(req.summary, "delete label /orgs/5/labels/9");
        assert!(matches!(
            w.plan_delete_label("/orgs/5/sec_policy/draft/ip_lists/1")
                .unwrap_err(),
            WriteError::Invalid(_)
        ));
        assert!(matches!(
            w.plan_update_label("/orgs/5/labels/9", " ").unwrap_err(),
            WriteError::Invalid(_)
        ));
    }
}
