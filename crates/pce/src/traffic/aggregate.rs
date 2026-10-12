//! Pure aggregations over resolved flows. The TUI's views and the CLI's
//! `--group` / `groups` use these (`sections` builds on them); no requests.
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::key_order::KeyOrder;
use crate::models::PolicyDecision;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The decision a group of flows is drawn in: blocked if any flow was
/// blocked, else potentially blocked if any was, else allowed if any was,
/// else unknown.
pub type Dominant = PolicyDecision;

pub fn dominant(allowed: u64, potentially_blocked: u64, blocked: u64) -> Dominant {
    if blocked > 0 {
        PolicyDecision::Blocked
    } else if potentially_blocked > 0 {
        PolicyDecision::PotentiallyBlocked
    } else if allowed > 0 {
        PolicyDecision::Allowed
    } else {
        PolicyDecision::Unknown
    }
}

/// Counts one flow's decision into the matching counter (unknown counts in none).
pub(crate) fn tally(
    decision: PolicyDecision,
    allowed: &mut u64,
    potentially_blocked: &mut u64,
    blocked: &mut u64,
) {
    match decision {
        PolicyDecision::Allowed => *allowed += 1,
        PolicyDecision::PotentiallyBlocked => *potentially_blocked += 1,
        PolicyDecision::Blocked => *blocked += 1,
        PolicyDecision::Unknown => {}
    }
}

/// `keys`, or `app,env` when empty.
pub fn keys_or_default(keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        vec!["app".into(), "env".into()]
    } else {
        keys.to_vec()
    }
}

/// `999`, `1.2K`, `3.4M`: at most six characters.
pub fn compact_count(n: u64) -> String {
    crate::iplist_size::human(u128::from(n))
}

/// The grouping values of an endpoint for `keys`: one entry per key, `-`
/// when missing or when the endpoint is not a workload.
pub fn group_values(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String> {
    if !ep.is_workload() {
        return vec!["-".to_string(); keys.len()];
    }
    keys.iter()
        .map(|k| ep.label_value(k).unwrap_or("-").to_string())
        .collect()
}

/// `shop/prod`; `-` for every slot of a non-workload.
pub fn group_text(ep: &ResolvedEndpoint, keys: &[String]) -> String {
    group_values(ep, keys).join("/")
}

/// The key an aggregation groups an endpoint under: its `group_values`
/// when it is a workload with labels; otherwise its name (or `ipl:<list>`)
/// in the first slot and `-` in the others, so IP lists, FQDNs and
/// unlabelled hosts stay apart and readable.
pub fn group_key(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String> {
    if ep.labels.is_empty() || !ep.is_workload() {
        let name = ep
            .ip_lists
            .first()
            .map(|l| format!("ipl:{l}"))
            .unwrap_or_else(|| ep.display_name());
        let mut k = vec![name];
        k.extend(std::iter::repeat_n(
            "-".to_string(),
            keys.len().saturating_sub(1),
        ));
        return k;
    }
    group_values(ep, keys)
}

/// Every label key seen in `flows`, with the number of flows carrying it
/// (on either side), sorted by [`KeyOrder::cmp_keys`]. Unresolved labels
/// (key `?`) and keys `order` does not show are left out.
pub fn keys_seen(flows: &[ResolvedFlow], order: &KeyOrder) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in flows {
        let keys: BTreeSet<&str> = f
            .src
            .labels
            .iter()
            .chain(&f.dst.labels)
            .map(|l| l.key.as_str())
            .filter(|k| *k != "?" && order.shows(k))
            .collect();
        for k in keys {
            *counts.entry(k.to_string()).or_default() += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|(a, _), (b, _)| order.cmp_keys(a, b));
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowGroup {
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub flows: u64,
    pub connections: u64,
    /// Bytes in plus out over the group's flows; 0 when none reported any.
    pub bytes: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

impl FlowGroup {
    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// Whether `f` is one of this group's flows under `keys`.
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        let keys = keys_or_default(keys);
        group_key(&f.src, &keys) == self.src && group_key(&f.dst, &keys) == self.dst
    }
}

pub fn group_by_labels(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup> {
    let keys = keys_or_default(keys);
    let mut map: BTreeMap<(Vec<String>, Vec<String>), FlowGroup> = BTreeMap::new();
    for f in flows {
        let key = (group_key(&f.src, &keys), group_key(&f.dst, &keys));
        let g = map.entry(key.clone()).or_insert_with(|| FlowGroup {
            src: key.0.clone(),
            dst: key.1.clone(),
            flows: 0,
            connections: 0,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 0,
            blocked: 0,
        });
        g.flows += 1;
        g.connections += f.num_connections;
        g.bytes += f.bytes();
        tally(
            f.policy_decision,
            &mut g.allowed,
            &mut g.potentially_blocked,
            &mut g.blocked,
        );
    }
    let mut out: Vec<FlowGroup> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.src.cmp(&b.src))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::{flows, rich_flows};

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn groups_by_label_pair_with_fallbacks() {
        let groups = group_by_labels(&flows(), &keys(&["app", "env"]));
        assert_eq!(groups[0].src, vec!["shop", "prod"]);
        assert_eq!(groups[0].dst, vec!["shop", "prod"]);
        assert_eq!(groups[0].flows, 1);
        assert_eq!(groups[0].connections, 310);
        assert_eq!(groups[0].bytes, 124_000);
        assert_eq!(groups[0].allowed, 1);
        let corp = groups
            .iter()
            .find(|g| g.src == vec!["ipl:Corp", "-"])
            .expect("ip list source falls back to its name");
        assert_eq!(corp.potentially_blocked, 1);
        assert_eq!(corp.bytes, 0);
        let nas = groups
            .iter()
            .find(|g| g.src == vec!["unmanaged-nas", "-"])
            .expect("unlabeled workload falls back to its name");
        assert_eq!(nas.blocked, 1);
        let ext = groups
            .iter()
            .find(|g| g.dst == vec!["fqdn-1.example", "-"])
            .expect("fqdn destination");
        assert_eq!(ext.connections, 5);
        assert!(
            groups
                .windows(2)
                .all(|w| w[0].connections >= w[1].connections),
            "sorted by connections desc"
        );
    }

    #[test]
    fn group_values_dash_a_missing_key_and_every_slot_of_a_non_workload() {
        let f = flows();
        let k = keys(&["app", "env", "datacenter"]);
        assert_eq!(group_values(&f[0].src, &k), vec!["shop", "prod", "-"]);
        assert_eq!(group_text(&f[0].src, &k), "shop/prod/-");
        assert_eq!(group_values(&f[1].src, &k), vec!["-", "-", "-"], "IP list");
        assert_eq!(group_text(&f[3].dst, &keys(&["app"])), "-", "FQDN");
        assert_eq!(
            group_values(&f[2].src, &k),
            vec!["-", "-", "-"],
            "a workload without labels"
        );
        assert_eq!(
            group_values(&f[0].src, &keys(&["role", "app"])),
            vec!["web", "shop"],
            "in the order of the keys"
        );
        assert_eq!(group_key(&f[1].src, &k), vec!["ipl:Corp", "-", "-"]);
        assert_eq!(group_key(&f[0].src, &k), vec!["shop", "prod", "-"]);
        assert_eq!(group_text(&f[0].src, &[]), "", "no keys, no slots");
    }

    #[test]
    fn keys_seen_puts_the_preferred_keys_first_and_counts_flows_once() {
        let pref = KeyOrder::from_keys(&keys(&["app", "env", "role", "loc"]));
        let seen = keys_seen(&rich_flows(), &pref);
        assert_eq!(
            seen,
            vec![
                ("app".to_string(), 30),
                ("env".to_string(), 30),
                ("role".to_string(), 30),
                ("loc".to_string(), 30),
                ("datacenter".to_string(), 11),
            ]
        );
        let seen = keys_seen(&rich_flows(), &KeyOrder::from_keys(&keys(&["role"])));
        let order: Vec<&str> = seen.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(order, vec!["role", "app", "datacenter", "env", "loc"]);
        let few = keys_seen(&flows(), &pref);
        assert_eq!(
            few,
            vec![
                ("app".to_string(), 4),
                ("env".to_string(), 4),
                ("role".to_string(), 4),
            ],
            "the 4-flow fixture: every flow has a labelled side; loc is on no workload"
        );
        assert!(keys_seen(&[], &pref).is_empty());
    }

    #[test]
    fn keys_seen_leaves_ignored_keys_out() {
        let order = KeyOrder::new(&keys(&["app", "env"]), "role")
            .with_ignored(&keys(&["datacenter", "loc"]));
        let seen: Vec<String> = keys_seen(&rich_flows(), &order)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(seen, keys(&["app", "env", "role"]));
    }

    #[test]
    fn rich_groups_sum_bytes_and_count_unknown_only_in_flows() {
        let groups = group_by_labels(&rich_flows(), &keys(&["app", "env"]));
        assert_eq!(groups.len(), 21);
        let top: Vec<(String, String, u64, u64, u64)> = groups
            .iter()
            .take(3)
            .map(|g| {
                (
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.flows,
                    g.connections,
                    g.bytes,
                )
            })
            .collect();
        assert_eq!(
            top,
            vec![
                ("crm/prod".into(), "crm/prod".into(), 2, 850, 2_480_000),
                ("shop/prod".into(), "shop/prod".into(), 4, 821, 940_000),
                ("ipl:Corp/-".into(), "shop/prod".into(), 2, 702, 8_350_000),
            ]
        );
        let unknown = groups
            .iter()
            .find(|g| g.src == vec!["erp", "prod"] && g.dst == vec!["crm", "prod"])
            .unwrap();
        assert_eq!(
            (unknown.flows, unknown.connections, unknown.allowed),
            (1, 9, 0)
        );
        assert_eq!(unknown.dominant(), PolicyDecision::Unknown);
        assert_eq!(groups[2].dominant(), PolicyDecision::Blocked);
        assert_eq!(
            groups.iter().map(|g| g.connections).sum::<u64>(),
            4_109,
            "every flow lands in exactly one group"
        );
        let f = rich_flows();
        assert!(groups[0].contains(&f[3], &keys(&["app", "env"])));
        assert!(!groups[0].contains(&f[0], &keys(&["app", "env"])));
    }

    #[test]
    fn empty_keys_fall_back_to_app_and_env() {
        assert_eq!(keys_or_default(&[]), keys(&["app", "env"]));
        assert_eq!(
            group_by_labels(&flows(), &[]),
            group_by_labels(&flows(), &keys(&["app", "env"]))
        );
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_234), "1.2K");
        assert_eq!(compact_count(13_625_100), "13.6M");
        assert_eq!(dominant(0, 0, 0), PolicyDecision::Unknown);
        assert_eq!(dominant(3, 1, 0), PolicyDecision::PotentiallyBlocked);
    }
}
