//! Every file under fixtures/recorded/ must still deserialise into its
//! model. Re-record with `cargo xtask record` when the PCE API changes.
use pce::models::{
    DenyRule, Event, Flow, IpList, Label, LabelGroup, PendingChange, RuleSet, Service, Workload,
    parse_pending,
};
use std::path::Path;

fn read(name: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/recorded")
        .join(name);
    let text = std::fs::read_to_string(&path).ok();
    if text.is_none() {
        eprintln!("skipping {name}: not recorded");
    }
    text
}

fn check<T: serde::de::DeserializeOwned>(name: &str) {
    let Some(text) = read(name) else { return };
    let parsed: Vec<T> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    if parsed.is_empty() {
        eprintln!("note: {name} is empty on the recorded PCE");
    }
}

#[test]
fn recorded_fixtures_still_parse() {
    check::<Label>("labels.json");
    check::<LabelGroup>("label_groups.json");
    check::<IpList>("ip_lists.json");
    check::<Service>("services.json");
    check::<Workload>("workloads.json");
    check::<RuleSet>("rule_sets.json");
    check::<DenyRule>("deny_rules.json");
    check::<Flow>("traffic_flows.json");
    check::<Event>("events.json");
}

/// `sec_policy/pending` is a list on some PCEs and a map on others; both
/// must flatten through `parse_pending`.
#[test]
fn recorded_pending_changes_still_flatten() {
    let Some(text) = read("pending.json") else {
        return;
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("pending.json: {e}"));
    let changes: Vec<PendingChange> = parse_pending(value);
    if changes.is_empty() {
        eprintln!("note: pending.json has no changes on the recorded PCE");
    }
    for c in &changes {
        assert!(c.href.starts_with("/orgs/"), "{c:?}");
        assert!(!c.object_type.is_empty(), "{c:?}");
    }
}

/// What `bulk_update` planning reads from a workload: an href, each label's
/// key and value inline (no name index needed), and enforcement and
/// visibility values it knows.
#[test]
fn recorded_workloads_carry_what_bulk_relies_on() {
    use pce::write::{BulkTarget, EnforcementMode, VisibilityLevel};
    let Some(text) = read("workloads.json") else {
        return;
    };
    let ws: Vec<Workload> = serde_json::from_str(&text).unwrap();
    let idx = pce::NameIndex::default();
    for w in &ws {
        assert!(w.href.starts_with("/orgs/"), "{}", w.href);
        if let Some(m) = &w.enforcement_mode {
            assert!(EnforcementMode::parse(m).is_some(), "enforcement {m}");
        }
        if let Some(v) = &w.visibility_level {
            assert!(VisibilityLevel::parse(v).is_some(), "visibility {v}");
        }
        let t = BulkTarget::of(w, &idx);
        assert!(
            t.labels
                .iter()
                .all(|l| !l.key.is_empty() && !l.value.is_empty()),
            "{}: a label without its key inline: {:?}",
            w.href,
            t.labels
        );
    }
}

fn pce_for(server: &wiremock::MockServer) -> pce::Pce {
    use pce::client::{ClientConfig, PceClient};
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut cfg = ClientConfig::new(
        url.host_str().unwrap(),
        5,
        pce::Credentials::new("key", "secret"),
    );
    cfg.scheme = "http".into();
    cfg.port = url.port().unwrap();
    cfg.backoff_base = std::time::Duration::from_millis(1);
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    pce::Pce::new(PceClient::new(cfg).unwrap())
}

/// The PCE's `X-Total-Count` is the size of the whole collection, whatever
/// the filters (demo100 answered 206 to a one-host `hostname=` read). Only
/// an unfiltered read may call itself truncated from it; a filtered read is
/// truncated only when it fills the row cap. Pinned on the recorded rows.
#[tokio::test]
async fn x_total_count_is_the_collection_size_not_the_match_count() {
    use pce::CacheMode;
    use pce::models::WorkloadFilter;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let Some(text) = read("workloads.json") else {
        return;
    };
    let rows: serde_json::Value = serde_json::from_str(&text).unwrap();
    let all = rows.as_array().unwrap().clone();
    let one = serde_json::Value::Array(all[..1].to_vec());
    let host = all[0]["hostname"].as_str().unwrap().to_string();
    let total = (all.len() + 181).to_string();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("hostname", host.as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(&one)
                .insert_header("X-Total-Count", total.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param_is_missing("hostname"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(&rows)
                .insert_header("X-Total-Count", total.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);

    let filtered = WorkloadFilter {
        hostname: Some(host.clone()),
        ..WorkloadFilter::default()
    };
    let r = p.workloads(&filtered, CacheMode::Bypass).await.unwrap();
    assert_eq!(r.value.len(), 1);
    assert!(
        !r.truncated,
        "a filtered read below the cap is complete, whatever X-Total-Count says"
    );

    let r = p
        .workloads(&WorkloadFilter::default(), CacheMode::Bypass)
        .await
        .unwrap();
    assert_eq!(r.value.len(), all.len());
    assert!(
        r.truncated,
        "an unfiltered read with X-Total-Count above its rows is cut"
    );
    server.verify().await;
}
