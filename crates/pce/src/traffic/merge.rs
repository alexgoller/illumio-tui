//! The one rule for putting Explorer results together. Explorer returns
//! stored aggregate records whose lifetime overlaps the window, each with
//! lifetime counters, so the same record can come back from two queries
//! (both sides of a focus, two windows). A record is its flow plus its
//! `first_detected`; two copies of it are versions, and the later one
//! supersedes the other. Counts are never added up.
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::models::PolicyDecision;
use std::collections::HashMap;

/// What makes two flows the same connection for display: both sides
/// (workload href, else IP), the service, the process and user, the
/// decision and the direction. Not a merge key: one connection has a new
/// record each time the PCE starts one (`record_key`).
pub type FlowKey = (
    Option<String>,
    Option<String>,
    Option<u16>,
    String,
    Option<String>,
    Option<String>,
    PolicyDecision,
    Option<String>,
);

fn side(e: &ResolvedEndpoint) -> Option<String> {
    e.workload_href.clone().or_else(|| e.ip.clone())
}

/// The flow identity without `first_detected`.
pub fn flow_key(f: &ResolvedFlow) -> FlowKey {
    (
        side(&f.src),
        side(&f.dst),
        f.port,
        f.proto.clone(),
        f.process_name.clone(),
        f.user_name.clone(),
        f.policy_decision,
        f.flow_direction.clone(),
    )
}

/// One stored aggregate record: both sides by workload href and IP, the
/// service, process, user, decision, direction and `first_detected`.
pub type RecordKey = (
    (Option<String>, Option<String>),
    (Option<String>, Option<String>),
    Option<u16>,
    String,
    Option<String>,
    Option<String>,
    PolicyDecision,
    Option<String>,
    Option<String>,
);

pub fn record_key(f: &ResolvedFlow) -> RecordKey {
    let ep = |e: &ResolvedEndpoint| (e.workload_href.clone(), e.ip.clone());
    (
        ep(&f.src),
        ep(&f.dst),
        f.port,
        f.proto.clone(),
        f.process_name.clone(),
        f.user_name.clone(),
        f.policy_decision,
        f.flow_direction.clone(),
        f.first_detected.clone(),
    )
}

fn last_seen(f: &ResolvedFlow) -> Option<jiff::Timestamp> {
    f.last_detected.as_deref()?.parse().ok()
}

/// Whether `new` is a later version of the same record than `old`: the
/// later `last_detected` (one that does not parse loses), on a tie the
/// larger `num_connections`. A full tie keeps `old`.
fn supersedes(new: &ResolvedFlow, old: &ResolvedFlow) -> bool {
    (last_seen(new), new.num_connections) > (last_seen(old), old.num_connections)
}

/// Appends `more` to `existing`, one row per record (`record_key`): a copy
/// of a record already present replaces it in place when it is a later
/// version (`supersedes`), else it is dropped. Nothing is summed. New
/// records keep their order.
pub fn merge_flows(existing: &mut Vec<ResolvedFlow>, more: Vec<ResolvedFlow>) {
    let mut at: HashMap<RecordKey, usize> = HashMap::with_capacity(existing.len());
    for (i, f) in existing.iter().enumerate() {
        at.entry(record_key(f)).or_insert(i);
    }
    for f in more {
        let key = record_key(&f);
        match at.get(&key) {
            Some(&i) => {
                if supersedes(&f, &existing[i]) {
                    existing[i] = f;
                }
            }
            None => {
                at.insert(key, existing.len());
                existing.push(f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(href: Option<&str>, ip: &str) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(ip.into()),
            workload_href: href.map(str::to_string),
            ..Default::default()
        }
    }

    fn flow(src: &str, port: u16, conns: u64) -> ResolvedFlow {
        ResolvedFlow {
            src: ep(Some(src), "10.0.0.1"),
            dst: ep(None, "10.0.0.2"),
            port: Some(port),
            proto: "tcp".into(),
            process_name: Some("nginx".into()),
            user_name: None,
            num_connections: conns,
            policy_decision: PolicyDecision::Allowed,
            draft_policy_decision: None,
            flow_direction: Some("outbound".into()),
            first_detected: Some("2026-10-13T00:00:00Z".into()),
            last_detected: Some("2026-10-13T11:00:00Z".into()),
            rules: vec!["/orgs/1/rule_sets/1/sec_rules/1".into()],
            bytes_in: None,
            bytes_out: Some(100),
        }
    }

    fn conns(rows: &[ResolvedFlow]) -> Vec<u64> {
        rows.iter().map(|f| f.num_connections).collect()
    }

    #[test]
    fn the_same_record_twice_is_one_row_and_never_summed() {
        // demo100: an hour inside a day returned the same records with the
        // same lifetime counters as the day itself.
        let mut loaded = vec![flow("/w/1", 443, 420), flow("/w/2", 443, 5)];
        merge_flows(
            &mut loaded,
            vec![flow("/w/1", 443, 420), flow("/w/2", 443, 5)],
        );
        assert_eq!(conns(&loaded), vec![420, 5]);
    }

    #[test]
    fn the_later_version_of_a_record_wins_wherever_it_comes_from() {
        // The PCE can return an older version of an aggregate: the same
        // first_detected, an earlier last_detected and a lower count.
        let mut older = flow("/w/1", 443, 380);
        older.last_detected = Some("2026-10-13T09:00:00Z".into());
        older.rules = vec!["older".into()];
        let newer = flow("/w/1", 443, 420);

        let mut loaded = vec![older.clone(), flow("/w/2", 443, 5)];
        merge_flows(&mut loaded, vec![newer.clone()]);
        assert_eq!(conns(&loaded), vec![420, 5], "replaced in place");
        assert_eq!(loaded[0].rules, newer.rules, "the whole later copy");

        let mut loaded = vec![newer.clone()];
        merge_flows(&mut loaded, vec![older]);
        assert_eq!(conns(&loaded), vec![420], "an older copy is dropped");
    }

    #[test]
    fn a_tie_on_last_seen_keeps_the_larger_count_then_the_existing_row() {
        let mut loaded = vec![flow("/w/1", 443, 10)];
        merge_flows(&mut loaded, vec![flow("/w/1", 443, 12)]);
        assert_eq!(conns(&loaded), vec![12]);
        let mut same = flow("/w/1", 443, 12);
        same.rules = vec!["other".into()];
        merge_flows(&mut loaded, vec![same]);
        assert_eq!(loaded[0].rules, vec!["/orgs/1/rule_sets/1/sec_rules/1"]);
        // A last_detected that does not parse is older than one that does.
        let mut junk = flow("/w/1", 443, 99);
        junk.last_detected = Some("junk".into());
        merge_flows(&mut loaded, vec![junk]);
        assert_eq!(conns(&loaded), vec![12]);
    }

    #[test]
    fn another_first_detected_is_another_record() {
        // The PCE starts new records at 00:00 UTC: yesterday's record of
        // the same flow is a row of its own.
        let mut yesterday = flow("/w/1", 443, 7);
        yesterday.first_detected = Some("2026-10-12T00:00:00Z".into());
        yesterday.last_detected = Some("2026-10-12T23:59:00Z".into());
        let mut loaded = vec![flow("/w/1", 443, 420)];
        merge_flows(&mut loaded, vec![yesterday]);
        assert_eq!(conns(&loaded), vec![420, 7]);
        assert_eq!(flow_key(&loaded[0]), flow_key(&loaded[1]), "one flow");
    }

    #[test]
    fn any_key_difference_keeps_a_new_row() {
        let base = flow("/w/1", 443, 1);
        let mut variants = Vec::new();
        let mut v = base.clone();
        v.port = Some(80);
        variants.push(v);
        let mut v = base.clone();
        v.proto = "udp".into();
        variants.push(v);
        let mut v = base.clone();
        v.src = ep(Some("/w/9"), "10.0.0.1");
        variants.push(v);
        let mut v = base.clone();
        v.src.ip = Some("10.9.9.9".into());
        variants.push(v);
        let mut v = base.clone();
        v.dst = ep(None, "10.0.0.3");
        variants.push(v);
        let mut v = base.clone();
        v.process_name = None;
        variants.push(v);
        let mut v = base.clone();
        v.user_name = Some("root".into());
        variants.push(v);
        let mut v = base.clone();
        v.policy_decision = PolicyDecision::Blocked;
        variants.push(v);
        let mut v = base.clone();
        v.flow_direction = Some("inbound".into());
        variants.push(v);
        let mut v = base.clone();
        v.first_detected = None;
        variants.push(v);
        let n = variants.len();
        let mut loaded = vec![base];
        merge_flows(&mut loaded, variants);
        assert_eq!(loaded.len(), 1 + n);
        assert!(loaded.iter().all(|f| f.num_connections == 1));
    }

    #[test]
    fn duplicates_within_the_new_rows_fold_too_and_order_is_kept() {
        let mut loaded = vec![flow("/w/1", 443, 1)];
        merge_flows(
            &mut loaded,
            vec![
                flow("/w/2", 22, 1),
                flow("/w/2", 22, 4),
                flow("/w/3", 22, 1),
            ],
        );
        let rows: Vec<(String, u64)> = loaded
            .iter()
            .map(|f| (f.src.workload_href.clone().unwrap(), f.num_connections))
            .collect();
        assert_eq!(
            rows,
            vec![("/w/1".into(), 1), ("/w/2".into(), 4), ("/w/3".into(), 1)]
        );
        let mut none = vec![];
        merge_flows(&mut none, vec![]);
        assert!(none.is_empty());
    }
}
