//! Opt-in: `PCE_LIVE=1` plus PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET
//! (and optionally PCE_PORT, PCE_TLS_VERIFY). At most 8 API calls.
use pce::client::{ClientConfig, PceClient};
use pce::traffic::TrafficQueryBuilder;
use pce::{CacheMode, Credentials, Pce};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn live_smoke() {
    if std::env::var("PCE_LIVE").as_deref() != Ok("1") {
        eprintln!("PCE_LIVE != 1, skipping");
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
    let mut cfg = ClientConfig::new(host, org, creds);
    if let Ok(p) = std::env::var("PCE_PORT") {
        cfg.port = p.parse().unwrap();
    }
    if let Ok(v) = std::env::var("PCE_TLS_VERIFY") {
        cfg.tls_verify = !matches!(v.to_ascii_lowercase().as_str(), "false" | "0" | "no");
    }
    let pce = Pce::new(PceClient::new(cfg).unwrap());

    let health = pce.health().await.expect("health");
    assert!(!health.is_null());
    let labels = pce.labels(CacheMode::Use).await.expect("labels");
    assert!(!labels.value.is_empty(), "test PCE should have labels");
    let idx = pce.name_index(CacheMode::Use).await.expect("index");
    assert!(!idx.label_keys().is_empty());

    let q = TrafficQueryBuilder::last(Duration::from_secs(3600))
        .unwrap()
        .max_results(50)
        .build()
        .unwrap();
    let r = pce
        .traffic(&q, CacheMode::Use, &CancellationToken::new(), None)
        .await
        .expect("traffic query");
    eprintln!(
        "flows={} matches={:?} truncated={} elapsed={:?}",
        r.value.flows.len(),
        r.value.matches_count,
        r.truncated,
        r.value.elapsed
    );
    let s = pce.stats();
    eprintln!("api calls={} bytes={}", s.calls, s.bytes);
    assert!(
        s.calls <= 8 + 10,
        "poll count depends on PCE speed; sanity cap"
    );
}
