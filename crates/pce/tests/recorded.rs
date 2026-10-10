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
