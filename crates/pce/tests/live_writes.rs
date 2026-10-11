//! Opt-in, destructive on purpose and therefore doubly gated: `PCE_LIVE_WRITES=1`
//! plus the usual PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET (optionally
//! PCE_PORT, PCE_TLS_VERIFY). `live_label_create_and_delete` creates one label
//! `illumio-tui-smoke-<ts>` and deletes it again (4 API calls);
//! `live_ruleset_and_rule_lifecycle` creates a disabled, unscoped draft
//! ruleset with one rule, updates the rule, plans (never sends) a provision,
//! and deletes both again (about 10 API calls; nothing reaches active
//! policy). Run them only against a PCE you designate as non-production.
//! Never in CI.
use pce::client::{ClientConfig, PceClient};
use pce::write::{
    ActorRef, IfMissing, LabelDraft, MemoryAudit, Outcome, PortSpec, ProvisionRequest,
    ProvisionSubset, RuleDraft, RulePatch, RuleSetDraft, ServiceRef, WriteContext, WriteGuard,
};
use pce::{Credentials, Pce};
use std::sync::Arc;

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

#[tokio::test]
async fn live_label_create_and_delete() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);

    let value = format!("illumio-tui-smoke-{}", jiff::Timestamp::now().as_second());
    let draft = LabelDraft::new("illumio-tui-smoke", &value);
    let plan = w
        .plan_create_label(&draft, IfMissing::SkipExisting)
        .await
        .unwrap();
    let Outcome::Written(created) = w.execute(plan).await.unwrap() else {
        panic!("smoke label already existed; clean up the PCE")
    };
    assert_eq!(created.status, 201);
    assert!(created.href.contains("/labels/"));

    let plan = w.plan_delete_label(&created.href).unwrap();
    let Outcome::Written(deleted) = w.execute(plan).await.unwrap() else {
        panic!("delete produced no result")
    };
    assert_eq!(deleted.status, 204);
    assert_eq!(audit.entries().len(), 2);
    assert!(pce.stats().calls <= 4, "{} calls", pce.stats().calls);
}

#[tokio::test]
async fn live_ruleset_and_rule_lifecycle() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let name = format!("illumio-tui-smoke-{}", jiff::Timestamp::now().as_second());
    let draft = RuleSetDraft {
        name,
        description: Some("illumio-tui live smoke; safe to delete".into()),
        enabled: false,
        scopes: vec![],
    };
    let plan = w
        .plan_create_rule_set(&draft, IfMissing::SkipExisting)
        .await
        .unwrap();
    let Outcome::Written(rs) = w.execute(plan).await.unwrap() else {
        panic!("smoke ruleset already existed; clean up the PCE")
    };
    assert!(rs.pending);

    let rule = RuleDraft {
        description: None,
        enabled: true,
        providers: vec![ActorRef::all()],
        consumers: vec![ActorRef::all()],
        services: vec![ServiceRef::Port(PortSpec::parse("443/tcp").unwrap())],
        unscoped_consumers: false,
        resolve_labels_as: None,
    };
    let Outcome::Written(r) = w
        .execute(w.plan_create_rule(&rs.href, &rule).unwrap())
        .await
        .unwrap()
    else {
        panic!("rule create produced no result")
    };
    assert!(
        r.href.starts_with(&format!("{}/sec_rules/", rs.href)),
        "{}",
        r.href
    );
    let patch = RulePatch {
        description: Some("updated by the smoke".into()),
        ..RulePatch::default()
    };
    w.execute(w.plan_update_rule(&r.href, &patch).await.unwrap())
        .await
        .unwrap();

    // Provisioning is planned, never sent: the smoke must not change
    // active policy.
    let plan = w
        .plan_provision(&ProvisionRequest {
            description: None,
            subset: ProvisionSubset::Hrefs(vec![rs.href.clone()]),
        })
        .await
        .unwrap();
    let body = plan.request().unwrap().body.clone().unwrap();
    assert!(body.to_string().contains(&rs.href), "{body}");

    w.execute(w.plan_delete_rule(&r.href).unwrap())
        .await
        .unwrap();
    w.execute(w.plan_delete_rule_set(&rs.href).unwrap())
        .await
        .unwrap();
    assert_eq!(
        audit.entries().len(),
        5,
        "create, create, update, delete, delete"
    );
}
