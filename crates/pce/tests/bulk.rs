//! Bulk workload writes against a mock PCE: the re-read, chunking, label
//! creation, per-workload outcomes, stopping, refusals and the audit.
use pce::client::{ClientConfig, PceClient};
use pce::models::{Workload, WorkloadFilter};
use pce::write::bulk::{ItemStatus, StepStatus};
use pce::write::{
    BatchOrigin, BulkChange, BulkLabel, BulkTarget, EnforcementMode, LabelDraft, LabelEdit,
    LabelSet, MemoryAudit, WriteContext, WriteGuard, plan_bulk,
};
use pce::{CacheMode, Credentials, NameIndex, Pce};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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

fn ctx(profile: &str) -> WriteContext {
    WriteContext {
        profile: profile.into(),
        host: "pce.example".into(),
        org_id: 5,
        source: "test".into(),
    }
}

fn guard(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(true, Box::new(audit), ctx("lab")).unwrap()
}

fn origin() -> Option<BatchOrigin> {
    Some(BatchOrigin::of(&ctx("lab")))
}

fn wl(n: usize, mode: &str, labels: Value) -> Value {
    json!({
        "href": format!("/orgs/5/workloads/w{n}"),
        "hostname": format!("host-{n}"),
        "managed": true,
        "enforcement_mode": mode,
        "visibility_level": "flow_summary",
        "labels": labels,
    })
}

fn targets(list: &Value) -> Vec<BulkTarget> {
    let ws: Vec<Workload> = serde_json::from_value(list.clone()).unwrap();
    let idx = NameIndex::default();
    ws.iter().map(|w| BulkTarget::of(w, &idx)).collect()
}

async fn mount_list(server: &MockServer, list: &Value) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list.clone()))
        .mount(server)
        .await;
}

/// Answers every chunk with `updated` for each workload it carries.
fn all_updated(req: &Request) -> ResponseTemplate {
    let sent: Value = serde_json::from_slice(&req.body).unwrap();
    let answer: Vec<Value> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"href": i["href"], "status": "updated"}))
        .collect();
    ResponseTemplate::new(200).set_body_json(answer)
}

async fn requests(server: &MockServer, m: &str, p: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == m && r.url.path() == p)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

const BULK: &str = "/api/v2/orgs/5/workloads/bulk_update";

#[tokio::test]
async fn two_hundred_fifty_workloads_go_in_three_chunks_after_one_re_read() {
    let server = MockServer::start().await;
    let list = Value::Array(
        (0..250)
            .map(|n| wl(n, "visibility_only", json!([])))
            .collect(),
    );
    mount_list(&server, &list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .expect(3)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let mut plan = plan_bulk(
        &targets(&list),
        &BulkChange::enforce(EnforcementMode::Full, None),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    plan.confirmed = Some(250);
    let mut seen = Vec::new();
    let report = pce
        .writer(&g)
        .execute_bulk(plan, &mut |d, of| seen.push((d, of)))
        .await;
    assert_eq!(report.refused, None);
    assert_eq!(report.reads, 1);
    assert_eq!(report.updated(), 250);
    assert!(!report.stopped(), "{:?}", report.lines());
    let bodies = requests(&server, "PUT", BULK).await;
    let sizes: Vec<usize> = bodies.iter().map(|b| b.as_array().unwrap().len()).collect();
    assert_eq!(sizes, vec![100, 100, 50]);
    assert_eq!(
        bodies[0][0],
        json!({"href": "/orgs/5/workloads/w0", "enforcement_mode": "full"}),
        "only the changed fields"
    );
    assert_eq!(
        requests(&server, "GET", "/api/v2/orgs/5/workloads")
            .await
            .len(),
        1
    );
    assert_eq!(seen, vec![(1, 3), (2, 3), (3, 3)]);
    // One audit entry per request, each listing its workloads' outcomes.
    let entries = audit.entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[2].items.as_ref().unwrap().as_array().unwrap().len(),
        50
    );
    assert_eq!(
        entries[0].items.as_ref().unwrap()[0],
        json!({"href": "/orgs/5/workloads/w0", "status": "updated"})
    );
    assert!(
        entries
            .iter()
            .all(|e| e.path == "/orgs/5/workloads/bulk_update")
    );
    server.verify().await;
}

#[tokio::test]
async fn a_chunk_with_a_failure_stops_the_send() {
    let server = MockServer::start().await;
    let list = Value::Array(
        (0..250)
            .map(|n| wl(n, "visibility_only", json!([])))
            .collect(),
    );
    mount_list(&server, &list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(|req: &Request| {
            let sent: Value = serde_json::from_slice(&req.body).unwrap();
            let answer: Vec<Value> = sent
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    if i == 7 {
                        json!({"href": x["href"], "status": "validation_failure", "errors": [{"message": "bad"}]})
                    } else {
                        json!({"href": x["href"], "status": "updated"})
                    }
                })
                .collect();
            ResponseTemplate::new(200).set_body_json(answer)
        })
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let plan = plan_bulk(
        &targets(&list),
        &BulkChange::enforce(EnforcementMode::Idle, None),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert_eq!(report.updated(), 99);
    let failed: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Failed)
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].href, "/orgs/5/workloads/w7");
    assert_eq!(failed[0].error.as_deref(), Some("validation_failure: bad"));
    assert_eq!(
        report
            .items
            .iter()
            .filter(|i| i.status == ItemStatus::NotSent)
            .count(),
        150
    );
    assert_eq!(report.steps[0].status, StepStatus::Partial);
    assert_eq!(report.steps[1].status, StepStatus::NotSent);
    assert!(report.stopped());
    assert!(
        report
            .status_text()
            .starts_with("updated 99 of 250 · 1 failed · 150 not sent")
    );
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(
        audit.entries()[0].items.as_ref().unwrap()[7]["status"],
        "failed"
    );
    server.verify().await;
}

#[tokio::test]
async fn a_refused_chunk_fails_its_workloads_and_stops() {
    let server = MockServer::start().await;
    let list = Value::Array((0..150).map(|n| wl(n, "idle", json!([]))).collect());
    mount_list(&server, &list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(
            ResponseTemplate::new(406)
                .set_body_json(json!([{"token": "invalid", "message": "nope"}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let plan = plan_bulk(
        &targets(&list),
        &BulkChange::enforce(EnforcementMode::VisibilityOnly, None),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert_eq!(report.steps[0].status, StepStatus::Failed);
    assert_eq!(report.steps[0].http_status, Some(406));
    assert_eq!(report.updated(), 0);
    assert_eq!(
        report
            .items
            .iter()
            .filter(|i| i.status == ItemStatus::Failed)
            .count(),
        100
    );
    assert_eq!(
        report
            .items
            .iter()
            .filter(|i| i.status == ItemStatus::NotSent)
            .count(),
        50
    );
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(audit.entries()[0].status, Some(406));
    server.verify().await;
}

#[tokio::test]
async fn the_re_read_drops_changed_and_missing_workloads() {
    let server = MockServer::start().await;
    let planned = Value::Array(
        (0..4)
            .map(|n| wl(n, "visibility_only", json!([])))
            .collect(),
    );
    // Since planning: w1 went full elsewhere, w2 got a label, w3 is gone.
    let now = json!([
        wl(0, "visibility_only", json!([])),
        wl(1, "full", json!([])),
        wl(
            2,
            "visibility_only",
            json!([{"href": "/orgs/5/labels/1", "key": "app", "value": "x"}])
        ),
    ]);
    mount_list(&server, &now).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let g = guard(MemoryAudit::new());
    let mut plan = plan_bulk(
        &targets(&planned),
        &BulkChange::enforce(EnforcementMode::Full, None),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    plan.confirmed = Some(4);
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    let bodies = requests(&server, "PUT", BULK).await;
    assert_eq!(
        bodies[0],
        json!([{"href": "/orgs/5/workloads/w0", "enforcement_mode": "full"}]),
        "only the unchanged workload is sent"
    );
    let status = |h: &str| {
        report
            .items
            .iter()
            .find(|i| i.href == h)
            .map(|i| i.status)
            .unwrap()
    };
    assert_eq!(status("/orgs/5/workloads/w0"), ItemStatus::Updated);
    assert_eq!(status("/orgs/5/workloads/w1"), ItemStatus::Changed);
    assert_eq!(status("/orgs/5/workloads/w2"), ItemStatus::Changed);
    assert_eq!(status("/orgs/5/workloads/w3"), ItemStatus::Missing);
    assert_eq!(report.dropped(), 3);
    assert!(
        report
            .lines()
            .iter()
            .any(|l| l.contains("changed since planned, skipped")),
        "{:?}",
        report.lines()
    );
    server.verify().await;
}

#[tokio::test]
async fn everything_changed_sends_nothing_after_the_re_read() {
    let server = MockServer::start().await;
    let planned = json!([wl(0, "idle", json!([]))]);
    mount_list(&server, &json!([wl(0, "full", json!([]))])).await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let plan = plan_bulk(
        &targets(&planned),
        &BulkChange::enforce(EnforcementMode::VisibilityOnly, None),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert!(report.steps.is_empty());
    assert!(requests(&server, "PUT", BULK).await.is_empty());
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn a_new_label_is_created_first_and_its_href_put_in_the_chunk() {
    let server = MockServer::start().await;
    let prod = json!({"href": "/orgs/5/labels/3", "key": "env", "value": "prod"});
    let list = json!([
        wl(
            0,
            "full",
            json!([{"href": "/orgs/5/labels/2", "key": "app", "value": "crm"}, prod])
        ),
        json!({"href": "/orgs/5/workloads/u1", "hostname": "unmanaged-1", "managed": false, "labels": []}),
    ]);
    mount_list(&server, &list).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(body_partial_json(json!({"key": "app", "value": "shop"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"href": "/orgs/5/labels/99", "key": "app", "value": "shop"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    // A cached list before: the relabel must drop it.
    pce.workloads(&WorkloadFilter::default(), CacheMode::Use)
        .await
        .unwrap();
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let plan = plan_bulk(
        &targets(&list),
        &BulkChange::relabel(LabelEdit {
            set: vec![LabelSet::Create(LabelDraft::new("app", "shop"))],
            remove: vec![],
        }),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    assert!(!plan.needs_count(), "labels: no typed count");
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert_eq!(report.updated(), 2, "{:?}", report.lines());
    assert!(report.labels_created && report.labels_changed);
    let bodies = requests(&server, "PUT", BULK).await;
    assert_eq!(
        bodies[0],
        json!([
            {"href": "/orgs/5/workloads/w0", "labels": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/99"}]},
            {"href": "/orgs/5/workloads/u1", "labels": [{"href": "/orgs/5/labels/99"}]},
        ]),
        "managed and unmanaged alike: labels only, the created href in place"
    );
    assert_eq!(audit.entries().len(), 2);
    assert_eq!(audit.entries()[0].path, "/orgs/5/labels");
    // The relabel dropped the cached list: the next read asks again.
    let before = requests(&server, "GET", "/api/v2/orgs/5/workloads")
        .await
        .len();
    pce.workloads(&WorkloadFilter::default(), CacheMode::Use)
        .await
        .unwrap();
    let after = requests(&server, "GET", "/api/v2/orgs/5/workloads")
        .await
        .len();
    assert_eq!(after, before + 1);
    server.verify().await;
}

#[tokio::test]
async fn a_failed_label_create_sends_no_chunk() {
    let server = MockServer::start().await;
    let list = json!([wl(0, "full", json!([]))]);
    mount_list(&server, &list).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([{"token": "label_exists"}])))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let g = guard(MemoryAudit::new());
    let plan = plan_bulk(
        &targets(&list),
        &BulkChange::relabel(LabelEdit {
            set: vec![LabelSet::Create(LabelDraft::new("app", "shop"))],
            remove: vec![],
        }),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert_eq!(report.steps[0].status, StepStatus::Failed);
    assert_eq!(report.steps[1].status, StepStatus::NotSent);
    assert_eq!(report.items[0].status, ItemStatus::NotSent);
    assert!(!report.labels_changed);
    assert!(requests(&server, "PUT", BULK).await.is_empty());
    server.verify().await;
}

#[tokio::test]
async fn refusals_send_and_read_nothing() {
    let server = MockServer::start().await;
    let list = Value::Array(
        (0..3)
            .map(|n| wl(n, "visibility_only", json!([])))
            .collect(),
    );
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let full = BulkChange::enforce(EnforcementMode::Full, None);
    // Full on three without the typed count; then with the wrong one.
    for confirmed in [None, Some(2)] {
        let mut plan =
            plan_bulk(&targets(&list), &full, WorkloadFilter::default(), origin()).unwrap();
        plan.confirmed = confirmed;
        let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
        assert!(
            report
                .refused
                .as_deref()
                .unwrap()
                .contains("needs that count typed"),
            "{:?}",
            report.refused
        );
        assert_eq!(report.reads, 0);
        assert!(report.items.iter().all(|i| i.status == ItemStatus::NotSent));
    }
    // Planned on another profile.
    let mut plan = plan_bulk(
        &targets(&list),
        &full,
        WorkloadFilter::default(),
        Some(BatchOrigin::of(&ctx("other"))),
    )
    .unwrap();
    plan.confirmed = Some(3);
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert!(
        report
            .refused
            .as_deref()
            .unwrap()
            .contains("planned on other")
    );
    // A label shaped href that is no workload of this org.
    let mut t = targets(&list);
    t[0].href = "/orgs/7/workloads/x".into();
    let mut plan = plan_bulk(&t, &full, WorkloadFilter::default(), origin()).unwrap();
    plan.confirmed = Some(3);
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert!(report.refused.is_some());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn labels_keep_unknown_keys_and_replace_by_key() {
    let shop = BulkLabel {
        href: "/orgs/5/labels/1".into(),
        key: "app".into(),
        value: "shop".into(),
    };
    let list = json!([wl(
        0,
        "full",
        json!([{"href": "/orgs/5/labels/2", "key": "app", "value": "crm"}, {"href": "/orgs/5/labels/77"}])
    )]);
    let plan = plan_bulk(
        &targets(&list),
        &BulkChange::relabel(LabelEdit {
            set: vec![LabelSet::Existing(shop)],
            remove: vec![pce::write::LabelRemoval {
                key: "env".into(),
                value: None,
            }],
        }),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    assert_eq!(
        plan.items[0].body["labels"],
        json!([{"href": "/orgs/5/labels/77"}, {"href": "/orgs/5/labels/1"}]),
        "a label without a known key stays"
    );
}

#[tokio::test]
async fn an_unanswered_chunk_may_have_applied_and_is_never_retried() {
    let server = MockServer::start().await;
    let list = Value::Array((0..120).map(|n| wl(n, "idle", json!([]))).collect());
    mount_list(&server, &list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let g = guard(audit.clone());
    let plan = plan_bulk(
        &targets(&list),
        &BulkChange::enforce(EnforcementMode::VisibilityOnly, None),
        WorkloadFilter::default(),
        origin(),
    )
    .unwrap();
    let report = pce.writer(&g).execute_bulk(plan, &mut |_, _| {}).await;
    assert_eq!(report.steps[0].status, StepStatus::Unanswered);
    assert_eq!(report.steps[1].status, StepStatus::NotSent);
    assert_eq!(
        report
            .items
            .iter()
            .filter(|i| i.status == ItemStatus::Unanswered)
            .count(),
        100
    );
    assert!(report.status_text().contains("it may have applied"));
    assert_eq!(audit.entries()[0].phase.as_deref(), Some("sent"));
    assert_eq!(requests(&server, "PUT", BULK).await.len(), 1, "no retry");
}
