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

mod ip_lists {
    use super::*;
    use pce::write::{IpListDraft, IpListPatch, IpRangeSpec};

    fn corp() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "Corp", "description": "corp nets",
            "ip_ranges": [{"from_ip": "10.0.0.0/8"}, {"from_ip": "192.168.1.1", "to_ip": "192.168.1.9", "exclusion": true}],
            "fqdns": [{"fqdn": "*.corp.example"}]
        })
    }

    #[test]
    fn draft_validates_and_builds_the_body() {
        let d = IpListDraft {
            name: "Corp".into(),
            description: None,
            ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            fqdns: vec!["*.corp.example".into()],
        };
        assert!(d.validate().is_ok());
        let empty = IpListDraft {
            name: " ".into(),
            description: None,
            ranges: vec![],
            fqdns: vec![],
        };
        let errs = empty.validate().unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
    }

    #[tokio::test]
    async fn create_plan_body_uses_pce_field_names() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = IpListDraft {
            name: "Corp".into(),
            description: Some("corp nets".into()),
            ranges: vec![
                IpRangeSpec::parse("10.0.0.0/8", false).unwrap(),
                IpRangeSpec::parse("192.168.1.1-192.168.1.9", true).unwrap(),
            ],
            fqdns: vec!["*.corp.example".into()],
        };
        let plan = pce
            .writer(&guard)
            .plan_create_ip_list(&d, IfMissing::Create)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/ip_lists");
        assert_eq!(
            req.body,
            Some(json!({
                "name": "Corp", "description": "corp nets",
                "ip_ranges": [{"from_ip": "10.0.0.0/8"}, {"from_ip": "192.168.1.1", "to_ip": "192.168.1.9", "exclusion": true}],
                "fqdns": [{"fqdn": "*.corp.example"}]
            }))
        );
        assert_eq!(req.summary, "create ip_list Corp");
    }

    #[tokio::test]
    async fn if_missing_matches_names_exactly_and_case_sensitively() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([corp()])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = |name: &str| IpListDraft {
            name: name.into(),
            description: None,
            ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            fqdns: vec![],
        };
        let plan = pce
            .writer(&guard)
            .plan_create_ip_list(&d("Corp"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(matches!(plan, Plan::Noop { .. }));
        let plan = pce
            .writer(&guard)
            .plan_create_ip_list(&d("corp"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some(), "case differs: not the same list");
    }

    #[test]
    fn patch_applies_adds_and_removes_to_the_current_list() {
        let current: pce::models::IpList = serde_json::from_value(corp()).unwrap();
        let patch = IpListPatch {
            name: None,
            description: Some("updated".into()),
            add_ranges: vec![IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
            remove_ranges: vec![IpRangeSpec::parse("192.168.1.1-192.168.1.9", true).unwrap()],
            add_fqdns: vec!["db.corp.example".into()],
            remove_fqdns: vec!["*.corp.example".into()],
        };
        let d = patch.apply(&current).unwrap();
        assert_eq!(d.name, "Corp");
        assert_eq!(d.description.as_deref(), Some("updated"));
        assert_eq!(
            d.ranges
                .iter()
                .map(IpRangeSpec::display)
                .collect::<Vec<_>>(),
            vec!["10.0.0.0/8", "10.2.0.0/16"]
        );
        assert_eq!(d.fqdns, vec!["db.corp.example"]);
        // Adding a duplicate is a no-op, not a second entry.
        let dup = IpListPatch {
            add_ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            ..IpListPatch::default()
        };
        assert_eq!(dup.apply(&current).unwrap().ranges.len(), 2);
    }

    #[test]
    fn remove_range_matches_a_cidr() {
        let current: pce::models::IpList = serde_json::from_value(corp()).unwrap();
        let patch = IpListPatch {
            remove_ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            ..IpListPatch::default()
        };
        let d = patch.apply(&current).unwrap();
        assert_eq!(d.ranges.len(), 1);
        assert_eq!(d.ranges[0].display(), "!192.168.1.1-192.168.1.9");
    }

    #[test]
    fn removing_something_absent_or_the_last_range_is_refused() {
        let current: pce::models::IpList = serde_json::from_value(corp()).unwrap();
        let patch = IpListPatch {
            remove_ranges: vec![IpRangeSpec::parse("172.16.0.1", false).unwrap()],
            ..IpListPatch::default()
        };
        let errs = patch.apply(&current).unwrap_err();
        assert!(errs[0].contains("172.16.0.1"), "{errs:?}");
        let patch = IpListPatch {
            remove_fqdns: vec!["nope".into()],
            ..IpListPatch::default()
        };
        assert!(patch.apply(&current).is_err());
        // Remove everything: the draft no longer validates.
        let patch = IpListPatch {
            remove_ranges: vec![
                IpRangeSpec::parse("10.0.0.0/8", false).unwrap(),
                IpRangeSpec::parse("192.168.1.1-192.168.1.9", true).unwrap(),
            ],
            remove_fqdns: vec!["*.corp.example".into()],
            ..IpListPatch::default()
        };
        let errs = patch.apply(&current).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("at least one")), "{errs:?}");
        assert!(IpListPatch::default().is_empty());
    }

    #[tokio::test]
    async fn update_plan_fetches_current_and_puts_the_full_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(corp()))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = IpListPatch {
            add_fqdns: vec!["db.corp.example".into()],
            ..IpListPatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_ip_list("/orgs/5/sec_policy/active/ip_lists/3", &patch)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(
            req.path, "/orgs/5/sec_policy/draft/ip_lists/3",
            "active is rewritten to draft"
        );
        let body = req.body.clone().unwrap();
        assert_eq!(body["name"], "Corp");
        assert_eq!(body["ip_ranges"].as_array().unwrap().len(), 2);
        assert_eq!(
            body["fqdns"],
            json!([{"fqdn": "*.corp.example"}, {"fqdn": "db.corp.example"}])
        );
        let err = pce
            .writer(&guard)
            .plan_update_ip_list(
                "/orgs/5/sec_policy/draft/ip_lists/3",
                &IpListPatch::default(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, WriteError::Invalid(_)),
            "an empty patch is refused: {err:?}"
        );
    }

    #[tokio::test]
    async fn delete_plan() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_delete_ip_list("/orgs/5/sec_policy/draft/ip_lists/3")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/ip_lists/3");
        assert!(
            pce.writer(&guard)
                .plan_delete_ip_list("/orgs/5/labels/3")
                .is_err()
        );
    }

    #[tokio::test]
    async fn update_keeps_range_descriptions_and_remove_ignores_them() {
        let server = MockServer::start().await;
        let mut l = corp();
        l["ip_ranges"][0]["description"] = json!("DMZ");
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(l.clone()))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = IpListPatch {
            add_fqdns: vec!["db.corp.example".into()],
            ..IpListPatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_ip_list("/orgs/5/sec_policy/draft/ip_lists/3", &patch)
            .await
            .unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body["ip_ranges"][0],
            json!({"from_ip": "10.0.0.0/8", "description": "DMZ"})
        );
        let current: pce::models::IpList = serde_json::from_value(l).unwrap();
        let patch = IpListPatch {
            remove_ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            ..IpListPatch::default()
        };
        assert_eq!(patch.apply(&current).unwrap().ranges.len(), 1);
    }
}

mod services {
    use super::*;
    use pce::write::{PortSpec, ServiceDraft, ServicePatch};

    fn pg() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "PostgreSQL",
            "service_ports": [{"port": 5432, "proto": 6}]
        })
    }

    #[test]
    fn draft_validates_and_sends_only_service_ports() {
        let d = ServiceDraft {
            name: "PG".into(),
            description: None,
            ports: vec![],
        };
        assert!(d.validate().is_err(), "a service needs a port");
        let d = ServiceDraft {
            name: "PG".into(),
            description: Some("db".into()),
            ports: vec![
                PortSpec::parse("5432/tcp").unwrap(),
                PortSpec::parse("icmp").unwrap(),
            ],
        };
        assert!(d.validate().is_ok());
    }

    #[tokio::test]
    async fn create_plan_body() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = ServiceDraft {
            name: "PostgreSQL".into(),
            description: Some("db".into()),
            ports: vec![
                PortSpec::parse("5432/tcp").unwrap(),
                PortSpec::parse("8000-8100/udp").unwrap(),
            ],
        };
        let plan = pce
            .writer(&guard)
            .plan_create_service(&d, IfMissing::Create)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/services");
        let body = req.body.clone().unwrap();
        assert_eq!(
            body,
            json!({"name": "PostgreSQL", "description": "db",
                   "service_ports": [{"port": 5432, "proto": 6}, {"port": 8000, "to_port": 8100, "proto": 17}]})
        );
        assert!(
            body.get("windows_services").is_none(),
            "exactly one port list key"
        );
        assert_eq!(req.summary, "create service PostgreSQL");
    }

    #[tokio::test]
    async fn if_missing_skips_exact_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([pg()])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = |n: &str| ServiceDraft {
            name: n.into(),
            description: None,
            ports: vec![PortSpec::parse("1/tcp").unwrap()],
        };
        assert!(matches!(
            pce.writer(&guard)
                .plan_create_service(&d("PostgreSQL"), IfMissing::SkipExisting)
                .await
                .unwrap(),
            Plan::Noop { .. }
        ));
        assert!(
            pce.writer(&guard)
                .plan_create_service(&d("Postgres"), IfMissing::SkipExisting)
                .await
                .unwrap()
                .request()
                .is_some()
        );
    }

    #[test]
    fn patch_adds_and_removes_ports() {
        let current: pce::models::Service = serde_json::from_value(pg()).unwrap();
        let patch = ServicePatch {
            name: Some("PG".into()),
            description: None,
            add_ports: vec![
                PortSpec::parse("5433/tcp").unwrap(),
                PortSpec::parse("5432/tcp").unwrap(),
            ],
            remove_ports: vec![],
        };
        let d = patch.apply(&current).unwrap();
        assert_eq!(d.name, "PG");
        assert_eq!(
            d.ports.iter().map(PortSpec::display).collect::<Vec<_>>(),
            vec!["5432/tcp", "5433/tcp"]
        );
        let patch = ServicePatch {
            remove_ports: vec![PortSpec::parse("5432/tcp").unwrap()],
            ..ServicePatch::default()
        };
        let errs = patch.apply(&current).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("at least one")),
            "removing the last port: {errs:?}"
        );
        let patch = ServicePatch {
            remove_ports: vec![PortSpec::parse("99/tcp").unwrap()],
            ..ServicePatch::default()
        };
        assert!(patch.apply(&current).unwrap_err()[0].contains("99/tcp"));
    }

    #[tokio::test]
    async fn update_and_delete_plans() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pg()))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = ServicePatch {
            add_ports: vec![PortSpec::parse("5433/tcp").unwrap()],
            ..ServicePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_service("/orgs/5/sec_policy/draft/services/7", &patch)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(
            req.body.clone().unwrap()["service_ports"],
            json!([{"port": 5432, "proto": 6}, {"port": 5433, "proto": 6}])
        );
        let del = pce
            .writer(&guard)
            .plan_delete_service("/orgs/5/sec_policy/active/services/7")
            .unwrap();
        assert_eq!(
            del.request().unwrap().path,
            "/orgs/5/sec_policy/draft/services/7"
        );
    }

    async fn refuses_unpreservable(body: Value) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = ServicePatch {
            add_ports: vec![PortSpec::parse("5433/tcp").unwrap()],
            ..ServicePatch::default()
        };
        let err = pce
            .writer(&guard)
            .plan_update_service("/orgs/5/sec_policy/draft/services/7", &patch)
            .await
            .unwrap_err();
        assert!(matches!(err, WriteError::Invalid(_)), "{err:?}");
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1, "only the GET");
        assert_eq!(reqs[0].method.as_str(), "GET");
    }

    #[tokio::test]
    async fn update_refuses_windows_services() {
        refuses_unpreservable(json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "W",
            "service_ports": [{"port": 5432, "proto": 6}],
            "windows_services": [{"service_name": "spooler"}]
        }))
        .await;
    }

    #[tokio::test]
    async fn update_refuses_icmp_type_or_code() {
        refuses_unpreservable(json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "I",
            "service_ports": [{"proto": 1, "icmp_type": 8, "icmp_code": 0}]
        }))
        .await;
    }

    #[tokio::test]
    async fn update_refuses_a_port_without_proto() {
        refuses_unpreservable(json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "P",
            "service_ports": [{"port": 5432, "proto": 6}, {"port": 1}]
        }))
        .await;
    }

    #[tokio::test]
    async fn null_icmp_and_windows_fields_do_not_refuse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "href": "/orgs/5/sec_policy/draft/services/7", "name": "N",
                "service_ports": [{"port": 5432, "proto": 6, "icmp_type": null, "icmp_code": null}],
                "windows_services": null
            })))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = ServicePatch {
            description: Some("d".into()),
            ..ServicePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_service("/orgs/5/sec_policy/draft/services/7", &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body.clone().unwrap()["service_ports"],
            json!([{"port": 5432, "proto": 6}])
        );
    }
}
