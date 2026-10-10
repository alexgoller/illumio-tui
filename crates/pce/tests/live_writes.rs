//! Opt-in, destructive on purpose and therefore doubly gated: `PCE_LIVE_WRITES=1`
//! plus the usual PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET (optionally
//! PCE_PORT, PCE_TLS_VERIFY). Creates one label `illumio-tui-smoke-<ts>` and
//! deletes it again: 4 API calls. Run it only against a PCE you designate
//! as non-production. Never in CI.
use pce::client::{ClientConfig, PceClient};
use pce::write::{IfMissing, LabelDraft, MemoryAudit, Outcome, WriteContext, WriteGuard};
use pce::{Credentials, Pce};

#[tokio::test]
async fn live_label_create_and_delete() {
    if std::env::var("PCE_LIVE_WRITES").as_deref() != Ok("1") {
        eprintln!("PCE_LIVE_WRITES != 1, skipping");
        return;
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
