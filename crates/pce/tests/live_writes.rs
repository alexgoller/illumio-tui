//! Opt-in, destructive on purpose and therefore doubly gated:
//! `PCE_LIVE_WRITES=1` plus the usual PCE_HOST, PCE_ORG_ID, API_KEY,
//! API_SECRET (optionally PCE_PORT, PCE_TLS_VERIFY). Without it every test
//! prints a skip line and passes. Never in CI; run them only against a PCE
//! you designate as non-production, one at a time
//! (`--test-threads=1`), e.g. through `scripts/live-smoke.sh`.
//!
//! Every test writes objects named `illumio-tui-smoke-<ts>` and removes or
//! restores what it changed, also when an assertion fails: the test body
//! returns its first failed check instead of panicking, the cleanup runs,
//! and only then does the test fail. Each prints the response facts the
//! code relies on (HTTP status, the shape of the answer) and bounds its API
//! calls. Nothing is ever provisioned: a provision is planned, never sent.
//!
//! - labels: create, update, delete. The PCE accepts only its defined label
//!   dimensions, so the label is made under the existing key
//!   [`LABEL_KEY`] with a unique value.
//! - services, IP lists, unmanaged workloads: create, update, delete.
//! - a ruleset with one rule: create, update, toggle `enabled`, plan a
//!   provision, delete.
//! - a batch: IP list, ruleset and a rule naming both by placeholder.
//! - a draft revert of an IP list the test created (only that href).
//! - bulk on ONE designated managed workload, `PCE_LIVE_WORKLOAD=<hostname>`
//!   (skipped unless exactly one workload has that hostname): visibility
//!   level and back, enforcement `visibility_only` and back, one label to
//!   another existing value of its key and back; the original state is
//!   restored and checked by a fresh read.
use pce::client::{ClientConfig, PceClient};
use pce::models::{Workload, WorkloadFilter};
use pce::write::{
    ActorKind, ActorRef, Batch, BatchOrigin, BatchStep, BulkChange, BulkLabel, BulkTarget,
    EnforcementMode, IfMissing, InterfaceSpec, IpListDraft, IpListPatch, IpRangeSpec, ItemStatus,
    LabelDraft, LabelEdit, LabelSet, LabelsPatch, MemoryAudit, ObjectKind, Outcome, Plan, PortSpec,
    ProvisionRequest, ProvisionSubset, REMOVED, RevertRequest, RuleDraft, RulePatch, RuleSetDraft,
    RuleSetPatch, ServiceDraft, ServicePatch, ServiceRef, StepResult, VisibilityLevel,
    WorkloadDraft, WorkloadPatch, WriteContext, WriteError, WriteGuard, Writer, Written, plan_bulk,
    provision_counts, step_placeholder,
};
use pce::{CacheMode, Credentials, NameIndex, Pce, PceError};
use serde_json::Value;
use std::sync::Arc;

/// A label dimension every demo PCE defines; the PCE refuses labels of
/// keys it does not know (406 `label_key_not_supported`).
const LABEL_KEY: &str = "role";

type R<T> = Result<T, String>;

/// The PCE, an in-memory audit and a guard from the environment, or `None`
/// (with a skip line) unless `PCE_LIVE_WRITES=1`.
fn live() -> Option<(Pce, Arc<MemoryAudit>, WriteGuard)> {
    if std::env::var("PCE_LIVE_WRITES").as_deref() != Ok("1") {
        eprintln!("PCE_LIVE_WRITES != 1, skipping");
        return None;
    }
    let host = std::env::var("PCE_HOST").expect("PCE_HOST");
    let org: u64 = std::env::var("PCE_ORG_ID")
        .expect("PCE_ORG_ID")
        .parse()
        .unwrap();
    let creds = Credentials::new(
        std::env::var("API_KEY").expect("API_KEY"),
        std::env::var("API_SECRET").expect("API_SECRET"),
    );
    let mut cfg = ClientConfig::new(host.clone(), org, creds);
    if let Ok(p) = std::env::var("PCE_PORT") {
        cfg.port = p.parse().unwrap();
    }
    if let Ok(v) = std::env::var("PCE_TLS_VERIFY") {
        cfg.tls_verify = !matches!(v.to_ascii_lowercase().as_str(), "false" | "0" | "no");
    }
    let pce = Pce::new(PceClient::new(cfg).unwrap());
    let audit = MemoryAudit::new();
    let guard = WriteGuard::new(
        true,
        Box::new(audit.clone()),
        WriteContext {
            profile: "live".into(),
            host,
            org_id: org,
            source: "test".into(),
        },
    )
    .unwrap();
    Some((pce, audit, guard))
}

fn smoke_name() -> String {
    format!("illumio-tui-smoke-{}", jiff::Timestamp::now().as_second())
}

fn ensure(ok: bool, msg: impl FnOnce() -> String) -> R<()> {
    if ok { Ok(()) } else { Err(msg()) }
}

/// Sends `plan` and prints what came back.
async fn send(w: &Writer<'_>, plan: Result<Plan, WriteError>, what: &str) -> R<Written> {
    let plan = plan.map_err(|e| format!("{what}: plan: {e}"))?;
    match w.execute(plan).await.map_err(|e| format!("{what}: {e}"))? {
        Outcome::Written(x) => {
            eprintln!(
                "{what}: HTTP {} href={} dropped={:?} warnings={:?}",
                x.status, x.href, x.dropped_fields, x.warnings
            );
            Ok(x)
        }
        Outcome::Skipped {
            existing_href,
            reason,
        } => Err(format!("{what}: skipped ({reason}): {existing_href}")),
    }
}

/// A delete answers 204 with no body.
fn deleted(d: &Written, what: &str) -> R<()> {
    ensure(d.status == 204 && d.object.is_null(), || {
        format!("{what}: expected 204 and no object, got {}", d.status)
    })
}

/// After a delete or revert, the href is gone: a 404, or an object the PCE
/// marks deleted (workloads are kept as `deleted: true` for a while).
async fn gone(w: &Writer<'_>, href: &str) -> R<()> {
    match w.fetch(href).await {
        Err(WriteError::Pce(PceError::NotFound { .. })) => {
            eprintln!("re-read {href}: 404");
            Ok(())
        }
        Ok(v) if v.get("deleted") == Some(&Value::Bool(true)) => {
            eprintln!("re-read {href}: 200 with deleted=true");
            Ok(())
        }
        Ok(v) => Err(format!("{href} still there after delete: {v}")),
        Err(e) => Err(format!("re-read {href}: {e}")),
    }
}

/// Deletes what a failed test left behind; returns what it could not.
async fn cleanup(w: &Writer<'_>, left: &[(String, &str)]) -> Vec<String> {
    let mut failed = Vec::new();
    for (href, kind) in left {
        let plan = match *kind {
            "label" => w.plan_delete_label(href),
            "service" => w.plan_delete_service(href),
            "ip_list" => w.plan_delete_ip_list(href),
            "rule_set" => w.plan_delete_rule_set(href),
            "workload" => w.plan_delete_workload(href).await,
            _ => unreachable!("{kind}"),
        };
        if let Err(e) = send(w, plan, &format!("cleanup {kind}")).await {
            failed.push(format!("LEFT ON THE PCE: {kind} {href}: {e}"));
        }
    }
    failed
}

/// Fails the test with the body's error and anything cleanup left behind.
fn finish(body: R<()>, left: Vec<String>) {
    for l in &left {
        eprintln!("{l}");
    }
    if let Err(e) = body {
        let more = if left.is_empty() {
            ""
        } else {
            " (and cleanup failed, see above)"
        };
        panic!("{e}{more}");
    }
    assert!(left.is_empty(), "cleanup failed: {left:?}");
}

#[tokio::test]
async fn live_label_create_update_and_delete() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let value = smoke_name();
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let draft = LabelDraft::new(LABEL_KEY, &value);
        let plan = w.plan_create_label(&draft, IfMissing::SkipExisting).await;
        let c = send(&w, plan, "create label").await?;
        left.push((c.href.clone(), "label"));
        ensure(c.status == 201, || format!("create: {}", c.status))?;
        ensure(c.href.contains("/labels/"), || c.href.clone())?;
        ensure(
            c.object["key"] == LABEL_KEY && c.object["value"] == value.as_str(),
            || format!("create echoed {}", c.object),
        )?;

        let new_value = format!("{value}-upd");
        let u = send(&w, w.plan_update_label(&c.href, &new_value), "update label").await?;
        ensure(matches!(u.status, 200 | 204), || {
            format!("update: {}", u.status)
        })?;
        ensure(u.object["value"] == new_value.as_str(), || {
            format!("update: object {}", u.object)
        })?;

        let d = send(&w, w.plan_delete_label(&c.href), "delete label").await?;
        left.clear();
        deleted(&d, "delete label")
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(audit.entries().len(), 3, "create, update, delete");
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 6, "{} calls", pce.stats().calls);
    finish(body, failed);
}

#[tokio::test]
async fn live_service_create_update_and_delete() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let draft = ServiceDraft {
            name: smoke_name(),
            description: Some("illumio-tui live smoke; safe to delete".into()),
            ports: vec![PortSpec::parse("65001/tcp").unwrap()],
        };
        let plan = w.plan_create_service(&draft, IfMissing::Create).await;
        let c = send(&w, plan, "create service").await?;
        left.push((c.href.clone(), "service"));
        ensure(c.status == 201, || format!("create: {}", c.status))?;
        ensure(c.pending, || "a service is a draft object".into())?;
        ensure(c.href.contains("/sec_policy/draft/services/"), || {
            c.href.clone()
        })?;
        ensure(c.dropped_fields.is_empty(), || {
            format!("dropped: {:?}", c.dropped_fields)
        })?;

        let patch = ServicePatch {
            description: Some("updated by the smoke".into()),
            add_ports: vec![PortSpec::parse("65002/udp").unwrap()],
            ..ServicePatch::default()
        };
        let plan = w.plan_update_service(&c.href, &patch).await;
        let u = send(&w, plan, "update service").await?;
        ensure(matches!(u.status, 200 | 204), || {
            format!("update: {}", u.status)
        })?;
        let ports = u.object["service_ports"].as_array().map_or(0, Vec::len);
        ensure(ports == 2, || format!("update: ports {}", u.object))?;

        let d = send(&w, w.plan_delete_service(&c.href), "delete service").await?;
        left.clear();
        deleted(&d, "delete service")?;
        gone(&w, &c.href).await
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(audit.entries().len(), 3, "create, update, delete");
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 8, "{} calls", pce.stats().calls);
    finish(body, failed);
}

#[tokio::test]
async fn live_ip_list_create_update_and_delete() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let draft = IpListDraft {
            name: smoke_name(),
            description: Some("illumio-tui live smoke; safe to delete".into()),
            ranges: vec![IpRangeSpec::parse("192.0.2.1", false).unwrap()],
            fqdns: vec![],
        };
        let plan = w.plan_create_ip_list(&draft, IfMissing::Create).await;
        let c = send(&w, plan, "create ip_list").await?;
        left.push((c.href.clone(), "ip_list"));
        ensure(c.status == 201, || format!("create: {}", c.status))?;
        ensure(c.href.contains("/sec_policy/draft/ip_lists/"), || {
            c.href.clone()
        })?;
        ensure(c.dropped_fields.is_empty(), || {
            format!("dropped: {:?}", c.dropped_fields)
        })?;

        let patch = IpListPatch {
            description: Some("updated by the smoke".into()),
            add_ranges: vec![IpRangeSpec::parse("198.51.100.0/24", false).unwrap()],
            ..IpListPatch::default()
        };
        let plan = w.plan_update_ip_list(&c.href, &patch).await;
        let u = send(&w, plan, "update ip_list").await?;
        ensure(matches!(u.status, 200 | 204), || {
            format!("update: {}", u.status)
        })?;
        let ranges = u.object["ip_ranges"].as_array().map_or(0, Vec::len);
        ensure(ranges == 2, || format!("update: ranges {}", u.object))?;

        let d = send(&w, w.plan_delete_ip_list(&c.href), "delete ip_list").await?;
        left.clear();
        deleted(&d, "delete ip_list")?;
        gone(&w, &c.href).await
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(audit.entries().len(), 3, "create, update, delete");
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 8, "{} calls", pce.stats().calls);
    finish(body, failed);
}

#[tokio::test]
async fn live_unmanaged_workload_create_update_and_delete() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let name = smoke_name();
        let draft = WorkloadDraft {
            hostname: Some(format!("{name}.invalid")),
            name,
            description: Some("illumio-tui live smoke; safe to delete".into()),
            interfaces: vec![InterfaceSpec::parse("192.0.2.10", "eth0".into()).unwrap()],
            labels: vec![],
        };
        let plan = w.plan_create_workload(&draft, IfMissing::Create).await;
        let c = send(&w, plan, "create workload").await?;
        left.push((c.href.clone(), "workload"));
        ensure(c.status == 201, || format!("create: {}", c.status))?;
        ensure(!c.pending, || "a workload is not a draft object".into())?;
        ensure(c.object["managed"] == false, || {
            format!("create: managed {}", c.object["managed"])
        })?;

        let patch = WorkloadPatch {
            description: Some("updated by the smoke".into()),
            add_addresses: vec!["192.0.2.11".into()],
            ..WorkloadPatch::default()
        };
        let plan = w
            .plan_update_workload(&c.href, &patch, &NameIndex::default())
            .await;
        let u = send(&w, plan, "update workload").await?;
        ensure(matches!(u.status, 200 | 204), || {
            format!("update: {}", u.status)
        })?;
        let n = u.object["interfaces"].as_array().map_or(0, Vec::len);
        ensure(n == 2, || format!("update: interfaces {}", u.object))?;

        let plan = w.plan_delete_workload(&c.href).await;
        let d = send(&w, plan, "delete workload").await?;
        left.clear();
        deleted(&d, "delete workload")?;
        gone(&w, &c.href).await
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(audit.entries().len(), 3, "create, update, delete");
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 10, "{} calls", pce.stats().calls);
    finish(body, failed);
}

#[tokio::test]
async fn live_ruleset_and_rule_lifecycle() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let draft = RuleSetDraft {
            name: smoke_name(),
            description: Some("illumio-tui live smoke; safe to delete".into()),
            enabled: false,
            scopes: vec![],
        };
        let plan = w
            .plan_create_rule_set(&draft, IfMissing::SkipExisting)
            .await;
        let rs = send(&w, plan, "create rule_set").await?;
        left.push((rs.href.clone(), "rule_set"));
        ensure(rs.pending && rs.status == 201, || {
            format!("create rule_set: {}", rs.status)
        })?;

        let rule = RuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::all()],
            services: vec![ServiceRef::Port(PortSpec::parse("443/tcp").unwrap())],
            unscoped_consumers: false,
            resolve_labels_as: None,
        };
        let r = send(&w, w.plan_create_rule(&rs.href, &rule), "create rule").await?;
        ensure(
            r.href.starts_with(&format!("{}/sec_rules/", rs.href)),
            || r.href.clone(),
        )?;

        let off = RulePatch {
            description: Some("updated by the smoke".into()),
            enabled: Some(false),
            ..RulePatch::default()
        };
        let plan = w.plan_update_rule(&r.href, &off).await;
        let u = send(&w, plan, "disable rule").await?;
        ensure(u.object["enabled"] == false, || {
            format!("disable: {}", u.object)
        })?;
        let on = RulePatch {
            enabled: Some(true),
            ..RulePatch::default()
        };
        let plan = w.plan_update_rule(&r.href, &on).await;
        let u = send(&w, plan, "enable rule").await?;
        ensure(u.object["enabled"] == true, || {
            format!("enable: {}", u.object)
        })?;

        let enable = RuleSetPatch {
            enabled: Some(true),
            ..RuleSetPatch::default()
        };
        let plan = w.plan_update_rule_set(&rs.href, &enable).await;
        let u = send(&w, plan, "enable rule_set").await?;
        ensure(u.object["enabled"] == true, || {
            format!("enable rule_set: {}", u.object)
        })?;

        // Provisioning is planned, never sent: the smoke must not change
        // active policy.
        let plan = w
            .plan_provision(&ProvisionRequest {
                description: None,
                subset: ProvisionSubset::Hrefs(vec![rs.href.clone()]),
            })
            .await
            .map_err(|e| format!("plan provision: {e}"))?;
        let body = plan.request().and_then(|r| r.body.clone());
        ensure(
            body.as_ref()
                .is_some_and(|b| b.to_string().contains(&rs.href)),
            || format!("provision body {body:?}"),
        )?;

        let d = send(&w, w.plan_delete_rule(&r.href), "delete rule").await?;
        deleted(&d, "delete rule")?;
        let d = send(&w, w.plan_delete_rule_set(&rs.href), "delete rule_set").await?;
        left.clear();
        deleted(&d, "delete rule_set")
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(
            audit.entries().len(),
            7,
            "create, create, 3 updates, delete, delete"
        );
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 18, "{} calls", pce.stats().calls);
    finish(body, failed);
}

/// A batch whose rule names the IP list and the ruleset created by the
/// steps before it, through placeholders.
#[tokio::test]
async fn live_batch_with_placeholders() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let org = pce.client().org_id();
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let name = smoke_name();
        let ipl = w
            .plan_create_ip_list(
                &IpListDraft {
                    name: name.clone(),
                    description: Some("illumio-tui live smoke; safe to delete".into()),
                    ranges: vec![IpRangeSpec::parse("192.0.2.2", false).unwrap()],
                    fqdns: vec![],
                },
                IfMissing::Create,
            )
            .await
            .map_err(|e| e.to_string())?;
        let rs = w
            .plan_create_rule_set(
                &RuleSetDraft {
                    name,
                    description: Some("illumio-tui live smoke; safe to delete".into()),
                    enabled: false,
                    scopes: vec![],
                },
                IfMissing::Create,
            )
            .await
            .map_err(|e| e.to_string())?;
        let rule = RuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::new(ActorKind::IpList, step_placeholder(0))],
            services: vec![ServiceRef::Port(PortSpec::parse("443/tcp").unwrap())],
            unscoped_consumers: false,
            resolve_labels_as: None,
        };
        // Planned against a stand-in ruleset, then pointed at step 1.
        let stand_in = format!("/orgs/{org}/sec_policy/draft/rule_sets/1");
        let Plan::Send { mut request, kind } = w
            .plan_create_rule(&stand_in, &rule)
            .map_err(|e| e.to_string())?
        else {
            return Err("rule create planned no request".into());
        };
        request.path = format!("{}/sec_rules", step_placeholder(1));
        let batch = Batch {
            steps: vec![
                BatchStep::new(ipl, ObjectKind::IpList, vec![]),
                BatchStep::new(rs, ObjectKind::RuleSet, vec![]),
                BatchStep::new(Plan::Send { request, kind }, ObjectKind::Rule, vec![0, 1]),
            ],
            origin: Some(BatchOrigin::of(guard.context())),
            ..Batch::default()
        };
        let out = w.execute_batch(batch).await;
        let href = |i: usize| match out.get(i) {
            Some(StepResult::Written(x)) => Some(x.href.clone()),
            _ => None,
        };
        // Ruleset first: deleting it removes the rule naming the list.
        if let Some(h) = href(1) {
            left.push((h, "rule_set"));
        }
        if let Some(h) = href(0) {
            left.push((h, "ip_list"));
        }
        for (i, r) in out.iter().enumerate() {
            match r {
                StepResult::Written(x) => {
                    eprintln!("batch step {}: HTTP {} href={}", i + 1, x.status, x.href)
                }
                other => eprintln!("batch step {}: {other:?}", i + 1),
            }
        }
        let (Some(ipl), Some(rs), Some(StepResult::Written(rule))) = (href(0), href(1), out.get(2))
        else {
            return Err(format!("batch did not write every step: {out:?}"));
        };
        ensure(rule.href.starts_with(&format!("{rs}/sec_rules/")), || {
            rule.href.clone()
        })?;
        let consumers = rule.object["consumers"].to_string();
        ensure(consumers.contains(&ipl), || {
            format!("rule consumers {consumers} do not name {ipl}")
        })?;

        let d = send(&w, w.plan_delete_rule_set(&rs), "delete rule_set").await?;
        left.retain(|(h, _)| *h != rs);
        deleted(&d, "delete rule_set")?;
        let d = send(&w, w.plan_delete_ip_list(&ipl), "delete ip_list").await?;
        left.clear();
        deleted(&d, "delete ip_list")
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(audit.entries().len(), 5, "3 creates, 2 deletes");
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 10, "{} calls", pce.stats().calls);
    finish(body, failed);
}

/// Reverts a draft-only IP list the test created, naming only its href:
/// no other pending change is touched; the list disappears.
#[tokio::test]
async fn live_revert_of_own_draft_change() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let mut left: Vec<(String, &str)> = Vec::new();
    let body: R<()> = async {
        let draft = IpListDraft {
            name: smoke_name(),
            description: Some("illumio-tui live smoke; safe to delete".into()),
            ranges: vec![IpRangeSpec::parse("192.0.2.3", false).unwrap()],
            fqdns: vec![],
        };
        let plan = w.plan_create_ip_list(&draft, IfMissing::Create).await;
        let c = send(&w, plan, "create ip_list").await?;
        left.push((c.href.clone(), "ip_list"));

        let plan = w
            .plan_revert(&RevertRequest {
                all: false,
                targets: vec![c.href.clone()],
            })
            .await
            .map_err(|e| format!("plan revert: {e}"))?;
        let req = plan
            .request()
            .ok_or_else(|| format!("revert planned nothing: {plan:?}"))?
            .clone();
        let body = req.body.clone().unwrap_or(Value::Null);
        eprintln!("revert: {:?} {} body={body}", req.method, req.path);
        let counts = provision_counts(&body);
        ensure(
            counts.len() == 1 && counts.get("ip_lists") == Some(&1),
            || format!("revert names more than the smoke list: {counts:?}"),
        )?;
        ensure(body.to_string().contains(&c.href), || body.to_string())?;
        ensure(
            req.objects.len() == 1 && req.objects[0].contains(REMOVED),
            || format!("revert objects {:?}", req.objects),
        )?;
        let r = send(&w, Ok(plan), "revert").await?;
        ensure((200..300).contains(&r.status), || {
            format!("revert: {}", r.status)
        })?;
        eprintln!("revert answer: {}", r.object);
        gone(&w, &c.href).await?;
        left.clear();
        Ok(())
    }
    .await;
    let failed = cleanup(&w, &left).await;
    if body.is_ok() {
        assert_eq!(audit.entries().len(), 2, "create, revert");
    }
    eprintln!("api calls={}", pce.stats().calls);
    assert!(pce.stats().calls <= 6, "{} calls", pce.stats().calls);
    finish(body, failed);
}

async fn fetch_workload(w: &Writer<'_>, href: &str) -> R<Workload> {
    let v = w
        .fetch(href)
        .await
        .map_err(|e| format!("GET {href}: {e}"))?;
    serde_json::from_value(v).map_err(|e| format!("decode {href}: {e}"))
}

fn state(w: &Workload) -> String {
    let mut labels: Vec<String> = w
        .labels
        .iter()
        .map(|l| match (&l.key, &l.value) {
            (Some(k), Some(v)) => format!("{k}={v}"),
            _ => l.href.clone(),
        })
        .collect();
    labels.sort();
    format!(
        "enforcement={:?} visibility={:?} labels=[{}]",
        w.enforcement_mode,
        w.visibility_level,
        labels.join(" ")
    )
}

fn label_hrefs(w: &Workload) -> Vec<String> {
    let mut v: Vec<String> = w.labels.iter().map(|l| l.href.clone()).collect();
    v.sort();
    v
}

/// One bulk change to `now` through `execute_bulk` (re-read, PUT), then a
/// fresh read: the workload as it is afterwards.
async fn bulk(
    w: &Writer<'_>,
    now: &Workload,
    change: BulkChange,
    idx: &NameIndex,
    origin: &BatchOrigin,
    what: &str,
) -> R<Workload> {
    let target = BulkTarget::of(now, idx);
    let plan = plan_bulk(
        &[target],
        &change,
        WorkloadFilter::default(),
        Some(origin.clone()),
    )
    .map_err(|e| format!("{what}: plan: {e}"))?;
    ensure(plan.items.len() == 1, || {
        format!("{what}: planned nothing: {:?}", plan.skipped)
    })?;
    let report = w.execute_bulk(plan, &mut |_, _| {}).await;
    for s in &report.steps {
        eprintln!(
            "{what}: {:?} {} HTTP {:?} step {:?} {}",
            s.method,
            s.path,
            s.http_status,
            s.status,
            s.error.as_deref().unwrap_or("")
        );
    }
    for i in &report.items {
        eprintln!("{what}: item {} {} {:?}", i.href, i.change, i.status);
    }
    ensure(report.failure.is_none() && !report.stopped(), || {
        format!("{what}: {}", report.status_text())
    })?;
    ensure(
        report.reads == 1 && report.updated() == 1 && report.steps.len() == 1,
        || format!("{what}: {}", report.status_text()),
    )?;
    ensure(
        report.items.iter().all(|i| i.status == ItemStatus::Updated),
        || format!("{what}: {}", report.status_text()),
    )?;
    let after = fetch_workload(w, &now.href).await?;
    eprintln!("{what}: now {}", state(&after));
    Ok(after)
}

/// The same change sent as the bare `bulk_update` request, to print the
/// PCE's answer as it is: a list of `{href, status}` per workload.
async fn bulk_raw(
    w: &Writer<'_>,
    now: &Workload,
    change: BulkChange,
    idx: &NameIndex,
    what: &str,
) -> R<Workload> {
    let plan = plan_bulk(
        &[BulkTarget::of(now, idx)],
        &change,
        WorkloadFilter::default(),
        None,
    )
    .map_err(|e| format!("{what}: plan: {e}"))?;
    let mut reqs = plan.requests(w.pce().client().org_id());
    ensure(reqs.len() == 1, || {
        format!("{what}: {} requests", reqs.len())
    })?;
    let request = reqs.remove(0);
    let x = send(
        w,
        Ok(Plan::Send {
            request,
            kind: ObjectKind::WorkloadBulk,
        }),
        what,
    )
    .await?;
    eprintln!("{what}: answer body {}", x.object);
    let items = x.object.as_array().cloned().unwrap_or_default();
    ensure(items.len() == 1, || {
        format!("{what}: expected a one-item list, got {}", x.object)
    })?;
    ensure(items[0]["href"] == now.href.as_str(), || {
        format!("{what}: item href {}", items[0])
    })?;
    let token = items[0]["status"].as_str().unwrap_or("");
    ensure(matches!(token, "updated" | "success"), || {
        format!("{what}: status token {:?}", items[0]["status"])
    })?;
    let after = fetch_workload(w, &now.href).await?;
    eprintln!("{what}: now {}", state(&after));
    Ok(after)
}

/// Puts `orig`'s enforcement, visibility and labels back where they
/// differ, then reads it fresh.
async fn restore(
    w: &Writer<'_>,
    orig: &Workload,
    idx: &NameIndex,
    origin: &BatchOrigin,
) -> R<Workload> {
    let mut now = fetch_workload(w, &orig.href).await?;
    let mut touched = false;
    if now.enforcement_mode != orig.enforcement_mode
        || now.visibility_level != orig.visibility_level
    {
        let mode = orig
            .enforcement_mode
            .as_deref()
            .and_then(EnforcementMode::parse)
            .ok_or("original enforcement unknown")?;
        let vis = orig
            .visibility_level
            .as_deref()
            .and_then(VisibilityLevel::parse);
        let change = BulkChange::enforce(mode, vis);
        now = bulk(w, &now, change, idx, origin, "restore enforcement").await?;
        touched = true;
    }
    if label_hrefs(&now) != label_hrefs(orig) {
        let patch = LabelsPatch {
            set_labels: Some(orig.labels.iter().map(|l| l.href.clone()).collect()),
            ..LabelsPatch::default()
        };
        let plan = w.plan_relabel_workload(&orig.href, &patch, idx).await;
        send(w, plan, "restore labels").await?;
        touched = true;
    }
    if touched {
        fetch_workload(w, &orig.href).await
    } else {
        Ok(now)
    }
}

#[tokio::test]
async fn live_bulk_on_designated_workload() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let Ok(host) = std::env::var("PCE_LIVE_WORKLOAD") else {
        eprintln!("PCE_LIVE_WORKLOAD unset, skipping the bulk test");
        return;
    };
    let w = pce.writer(&guard);
    let filter = WorkloadFilter {
        hostname: Some(host.clone()),
        max_results: Some(50),
        ..WorkloadFilter::default()
    };
    let found = pce
        .workloads(&filter, CacheMode::Bypass)
        .await
        .expect("workload read");
    let hits: Vec<Workload> = found
        .value
        .into_iter()
        .filter(|x| {
            x.hostname
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&host))
        })
        .collect();
    if hits.len() != 1 {
        eprintln!(
            "PCE_LIVE_WORKLOAD={host}: {} workloads have that hostname, need exactly one; skipping",
            hits.len()
        );
        return;
    }
    let orig = hits.into_iter().next().unwrap();
    if orig.managed != Some(true) {
        eprintln!("{host} is not VEN-managed; skipping the bulk test");
        return;
    }
    let (Some(mode0), Some(vis0)) = (
        orig.enforcement_mode
            .as_deref()
            .and_then(EnforcementMode::parse),
        orig.visibility_level
            .as_deref()
            .and_then(VisibilityLevel::parse),
    ) else {
        eprintln!(
            "{host}: unknown enforcement or visibility ({}); skipping",
            state(&orig)
        );
        return;
    };
    eprintln!("{host} {}: original {}", orig.href, state(&orig));
    let labels = pce.labels(CacheMode::Bypass).await.expect("labels").value;
    let idx = NameIndex::new(&labels, &[], &[], &[]);
    let origin = BatchOrigin::of(guard.context());
    let base = BulkTarget::of(&orig, &idx);

    // The label to swap: the workload's only label of a key (LABEL_KEY
    // first), and another existing value of that key.
    let mut keys: Vec<&str> = base.labels.iter().map(|l| l.key.as_str()).collect();
    keys.sort_by_key(|k| *k != LABEL_KEY);
    let swap: Option<(BulkLabel, BulkLabel)> = keys.iter().find_map(|k| {
        let mine: Vec<&BulkLabel> = base.labels.iter().filter(|l| l.key == *k).collect();
        let [cur] = mine.as_slice() else { return None };
        let other = labels.iter().find(|l| {
            l.key == *k && l.href != cur.href && l.deleted != Some(true) && !l.value.is_empty()
        })?;
        Some((
            (*cur).clone(),
            BulkLabel {
                href: other.href.clone(),
                key: other.key.clone(),
                value: other.value.clone(),
            },
        ))
    });

    let body: R<()> = async {
        // Visibility level and back (the way back sent bare, to see the
        // PCE's answer as it is).
        let vis1 = [VisibilityLevel::FlowSummary, VisibilityLevel::FlowDrops]
            .into_iter()
            .find(|v| *v != vis0)
            .unwrap();
        let change = BulkChange::enforce(mode0, Some(vis1));
        let now = bulk(&w, &orig, change, &idx, &origin, "visibility").await?;
        ensure(
            now.visibility_level.as_deref() == Some(vis1.as_str()),
            || format!("visibility not applied: {}", state(&now)),
        )?;
        let change = BulkChange::enforce(mode0, Some(vis0));
        let now = bulk_raw(&w, &now, change, &idx, "visibility back").await?;
        ensure(
            now.visibility_level.as_deref() == Some(vis0.as_str()),
            || format!("visibility not restored: {}", state(&now)),
        )?;

        // Enforcement visibility_only and back, visibility kept.
        let now = if mode0 == EnforcementMode::VisibilityOnly {
            eprintln!("{host} is already visibility_only; enforcement step skipped");
            now
        } else {
            let change = BulkChange::enforce(EnforcementMode::VisibilityOnly, None);
            let mid = bulk(&w, &now, change, &idx, &origin, "enforcement").await?;
            ensure(
                mid.enforcement_mode.as_deref() == Some("visibility_only")
                    && mid.visibility_level == orig.visibility_level,
                || format!("enforcement not applied as asked: {}", state(&mid)),
            )?;
            let change = BulkChange::enforce(mode0, Some(vis0));
            let back = bulk(&w, &mid, change, &idx, &origin, "enforcement back").await?;
            ensure(back.enforcement_mode == orig.enforcement_mode, || {
                format!("enforcement not restored: {}", state(&back))
            })?;
            back
        };

        // One label to another existing value of its key, and back.
        let Some((cur, other)) = &swap else {
            eprintln!("{host}: no label key with another value; relabel step skipped");
            return Ok(());
        };
        let set = |l: &BulkLabel| {
            BulkChange::relabel(LabelEdit {
                set: vec![LabelSet::Existing(l.clone())],
                remove: vec![],
            })
        };
        let mid = bulk(&w, &now, set(other), &idx, &origin, "relabel").await?;
        let hrefs = label_hrefs(&mid);
        ensure(
            hrefs.contains(&other.href) && !hrefs.contains(&cur.href),
            || format!("relabel not applied: {}", state(&mid)),
        )?;
        let back = bulk(&w, &mid, set(cur), &idx, &origin, "relabel back").await?;
        ensure(label_hrefs(&back) == label_hrefs(&orig), || {
            format!("labels not restored: {}", state(&back))
        })?;
        Ok(())
    }
    .await;

    // Whatever happened above: back to the original, checked fresh.
    let restored = restore(&w, &orig, &idx, &origin).await;
    eprintln!(
        "audit entries={} api calls={}",
        audit.entries().len(),
        pce.stats().calls
    );
    for e in audit.entries() {
        if let Some(items) = &e.items {
            eprintln!("audit {:?} {} items={items}", e.method, e.path);
        }
    }
    let restored = match restored {
        Ok(r) => r,
        Err(e) => panic!(
            "RESTORE FAILED for {host} ({e}); original was {}; body: {body:?}",
            state(&orig)
        ),
    };
    eprintln!("{host}: final {}", state(&restored));
    assert_eq!(restored.enforcement_mode, orig.enforcement_mode);
    assert_eq!(restored.visibility_level, orig.visibility_level);
    assert_eq!(label_hrefs(&restored), label_hrefs(&orig));
    if let Err(e) = body {
        panic!("{e} (state was restored)");
    }
    assert!(pce.stats().calls <= 24, "{} calls", pce.stats().calls);
}
