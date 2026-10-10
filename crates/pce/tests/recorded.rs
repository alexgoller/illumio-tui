//! Every file under fixtures/recorded/ must still deserialise into its
//! model. Re-record with `cargo xtask record` when the PCE API changes.
use pce::models::{DenyRule, Flow, IpList, Label, LabelGroup, RuleSet, Service, Workload};
use std::path::Path;

fn check<T: serde::de::DeserializeOwned>(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/recorded")
        .join(name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipping {name}: not recorded");
        return;
    };
    let parsed: Vec<T> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!parsed.is_empty(), "{name} is empty; re-record");
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
}
