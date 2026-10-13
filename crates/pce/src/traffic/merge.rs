//! Folds the flows of another time slice into the loaded ones, so the same
//! connection seen in two slices stays one row (as one wider query would
//! have returned it).
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::models::PolicyDecision;
use std::collections::HashMap;

/// What makes two flows the same row: both sides (workload href, else IP),
/// the service, the process and user, the decision and the direction.
pub(crate) type FlowKey = (
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

pub(crate) fn flow_key(f: &ResolvedFlow) -> FlowKey {
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

fn add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        _ => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

/// The earlier (`later == false`) or later of two timestamps; one that does
/// not parse loses to one that does.
fn pick(a: Option<String>, b: Option<String>, later: bool) -> Option<String> {
    let ts = |s: &Option<String>| s.as_deref().and_then(|s| s.parse::<jiff::Timestamp>().ok());
    match (ts(&a), ts(&b)) {
        (Some(x), Some(y)) if (later && y > x) || (!later && y < x) => b,
        (Some(_), Some(_)) => a,
        (None, Some(_)) => b,
        _ => a.or(b),
    }
}

/// Appends `more` to `existing`, folding each flow into an existing one with
/// the same key: connections and bytes add up (a missing count counts as
/// none), the earliest `first_detected` and latest `last_detected` win, the
/// existing row's rules and other fields stay. New flows keep their order.
pub fn merge_flows(existing: &mut Vec<ResolvedFlow>, more: Vec<ResolvedFlow>) {
    let mut at: HashMap<FlowKey, usize> = HashMap::with_capacity(existing.len());
    for (i, f) in existing.iter().enumerate() {
        at.entry(flow_key(f)).or_insert(i);
    }
    for f in more {
        let key = flow_key(&f);
        match at.get(&key) {
            Some(&i) => {
                let e = &mut existing[i];
                e.num_connections += f.num_connections;
                e.bytes_in = add(e.bytes_in, f.bytes_in);
                e.bytes_out = add(e.bytes_out, f.bytes_out);
                e.first_detected = pick(e.first_detected.take(), f.first_detected, false);
                e.last_detected = pick(e.last_detected.take(), f.last_detected, true);
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
            first_detected: Some("2026-10-09T10:00:00Z".into()),
            last_detected: Some("2026-10-09T11:00:00Z".into()),
            rules: vec!["/orgs/1/rule_sets/1/sec_rules/1".into()],
            bytes_in: None,
            bytes_out: Some(100),
        }
    }

    #[test]
    fn identical_flows_fold_into_one_row() {
        let mut loaded = vec![flow("/w/1", 443, 420), flow("/w/2", 443, 5)];
        let mut older = flow("/w/1", 443, 380);
        older.first_detected = Some("2026-10-08T01:00:00Z".into());
        older.last_detected = Some("2026-10-08T02:00:00Z".into());
        older.bytes_in = Some(7);
        older.bytes_out = None;
        older.rules = vec!["other".into()];
        merge_flows(&mut loaded, vec![older]);
        assert_eq!(loaded.len(), 2);
        let f = &loaded[0];
        assert_eq!(f.num_connections, 800);
        assert_eq!(f.bytes_in, Some(7), "None + Some = Some");
        assert_eq!(f.bytes_out, Some(100));
        assert_eq!(f.first_detected.as_deref(), Some("2026-10-08T01:00:00Z"));
        assert_eq!(f.last_detected.as_deref(), Some("2026-10-09T11:00:00Z"));
        assert_eq!(
            f.rules,
            vec!["/orgs/1/rule_sets/1/sec_rules/1"],
            "existing rules kept"
        );
        assert_eq!(loaded[1].num_connections, 5);
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
        let n = variants.len();
        let mut loaded = vec![base];
        merge_flows(&mut loaded, variants);
        assert_eq!(loaded.len(), 1 + n);
        assert!(loaded.iter().all(|f| f.num_connections == 1));
    }

    #[test]
    fn a_side_is_its_href_else_its_ip() {
        // Same href, different IP: one workload, one row.
        let mut loaded = vec![flow("/w/1", 443, 1)];
        let mut moved = flow("/w/1", 443, 2);
        moved.src.ip = Some("10.9.9.9".into());
        merge_flows(&mut loaded, vec![moved]);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].num_connections, 3);
    }

    #[test]
    fn duplicates_within_the_new_slice_fold_too_and_order_is_kept() {
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
            vec![("/w/1".into(), 1), ("/w/2".into(), 5), ("/w/3".into(), 1)]
        );
        let mut none = vec![];
        merge_flows(&mut none, vec![]);
        assert!(none.is_empty());
    }

    #[test]
    fn unparseable_or_missing_times_lose() {
        assert_eq!(
            pick(None, Some("2026-10-08T01:00:00Z".into()), false).as_deref(),
            Some("2026-10-08T01:00:00Z")
        );
        assert_eq!(
            pick(
                Some("junk".into()),
                Some("2026-10-08T01:00:00Z".into()),
                true
            )
            .as_deref(),
            Some("2026-10-08T01:00:00Z")
        );
        assert_eq!(
            pick(Some("2026-10-08T01:00:00Z".into()), None, true).as_deref(),
            Some("2026-10-08T01:00:00Z")
        );
        assert_eq!(pick(None, None, true), None);
    }
}
