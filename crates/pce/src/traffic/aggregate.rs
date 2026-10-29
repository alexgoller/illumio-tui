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

/// A group's connections split by decision: the lifetime counts of its
/// flow records (`num_connections`), summed per decision. Flattened into
/// JSON as `allowed_conns`, `pot_blocked_conns`, `blocked_conns` and
/// `unknown_conns`; the four sum to the group's connections.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ConnsByDecision {
    pub allowed_conns: u64,
    pub pot_blocked_conns: u64,
    pub blocked_conns: u64,
    /// Flows the PCE reported without a decision.
    pub unknown_conns: u64,
}

impl ConnsByDecision {
    /// Adds one flow's connections to its decision.
    pub fn add(&mut self, decision: PolicyDecision, connections: u64) {
        match decision {
            PolicyDecision::Allowed => self.allowed_conns += connections,
            PolicyDecision::PotentiallyBlocked => self.pot_blocked_conns += connections,
            PolicyDecision::Blocked => self.blocked_conns += connections,
            PolicyDecision::Unknown => self.unknown_conns += connections,
        }
    }

    /// The four counts as cells: allowed, potentially blocked, blocked, unknown.
    pub fn cells(&self) -> [String; 4] {
        [
            self.allowed_conns.to_string(),
            self.pot_blocked_conns.to_string(),
            self.blocked_conns.to_string(),
            self.unknown_conns.to_string(),
        ]
    }
}

/// What the draft policy would change in a group: flow records blocked or
/// potentially blocked now that the draft allows (`draft_allowed`, "would
/// allow"), and allowed ones it would block (`draft_blocked`). Present only
/// when a flow of the group carries a draft decision; flattened into JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DraftCounts {
    pub draft_allowed: u64,
    pub draft_blocked: u64,
}

impl DraftCounts {
    /// Counts `f` into `counts` when it carries a draft decision.
    pub fn add(counts: &mut Option<DraftCounts>, f: &ResolvedFlow) {
        if f.draft_policy_decision.is_some() {
            let c = counts.get_or_insert_with(DraftCounts::default);
            c.draft_allowed += u64::from(f.fixed_by_draft());
            c.draft_blocked += u64::from(f.blocked_by_draft());
        }
    }

    /// `+3 would allow · 1 would block`, or `-` with nothing to report.
    pub fn text(counts: Option<&DraftCounts>) -> String {
        let Some(c) = counts else {
            return String::new();
        };
        let mut parts = Vec::new();
        if c.draft_allowed > 0 {
            parts.push(format!("{} would allow", c.draft_allowed));
        }
        if c.draft_blocked > 0 {
            parts.push(format!("{} would block", c.draft_blocked));
        }
        if parts.is_empty() {
            "-".into()
        } else {
            parts.join(" · ")
        }
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

/// A byte count as `ls -h` prints it (1024-based): `0B`, `512B`, `1.5K`,
/// `12M`, `2.1G`. One decimal below 10, none from 10 up, truncated and
/// never rounded up; units `B K M G T P`.
pub fn human_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n}B");
    }
    let n = u128::from(n);
    let mut scale = 1024u128;
    for unit in ['K', 'M', 'G', 'T', 'P'] {
        if n < scale * 1024 || unit == 'P' {
            let whole = n / scale;
            return if whole < 10 {
                format!("{whole}.{}{unit}", n % scale * 10 / scale)
            } else {
                format!("{whole}{unit}")
            };
        }
        scale *= 1024;
    }
    unreachable!("the P arm returns")
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
    /// `5432/tcp` (or `icmp`) when grouped by service as well
    /// (`group_by_service`); `None` for a plain pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Flow records.
    pub flows: u64,
    /// The records' lifetime connection counts, summed.
    pub connections: u64,
    /// Bytes in plus out over the group's flows; 0 when none reported any.
    pub bytes: u64,
    /// Flow records per decision.
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
    /// Connections per decision.
    #[serde(flatten)]
    pub conns: ConnsByDecision,
    /// What the draft policy would change, with draft decisions loaded.
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
    /// What the marks say about the row (`pce::marks::annotate_*`);
    /// flattened into JSON as `mark`, `marks` and `squelched`.
    #[serde(flatten)]
    pub marks: Option<crate::marks::RowMarks>,
    /// Compare mode: the row's change against the baseline
    /// (`traffic::compare`); flattened into JSON as `change` and
    /// `baseline_decision`.
    #[serde(flatten)]
    pub change: Option<super::compare::RowChange>,
}

impl FlowGroup {
    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// Whether `f` is one of this group's flows under `keys` (on its
    /// service too when it has one).
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        let keys = keys_or_default(keys);
        group_key(&f.src, &keys) == self.src
            && group_key(&f.dst, &keys) == self.dst
            && self.service.as_ref().is_none_or(|s| *s == f.service_text())
    }
}

/// One row per source group and destination group.
pub fn group_by_labels(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup> {
    group(flows, keys, false)
}

/// One row per source group, destination group and service (`port/proto`,
/// or `icmp` without a port): the policy tuple. Hosts, IPs, users and
/// processes collapse; the totals are those of `group_by_labels`.
pub fn group_by_service(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup> {
    group(flows, keys, true)
}

/// `group_by_service` over borrowed flows: a subset is grouped without
/// cloning it.
pub fn group_by_service_refs<'a>(
    flows: impl IntoIterator<Item = &'a ResolvedFlow>,
    keys: &[String],
) -> Vec<FlowGroup> {
    group(flows, keys, true)
}

fn group<'a>(
    flows: impl IntoIterator<Item = &'a ResolvedFlow>,
    keys: &[String],
    by_service: bool,
) -> Vec<FlowGroup> {
    let keys = keys_or_default(keys);
    type Key = (Vec<String>, Vec<String>, Option<String>);
    let mut map: BTreeMap<Key, FlowGroup> = BTreeMap::new();
    for f in flows {
        let key = (
            group_key(&f.src, &keys),
            group_key(&f.dst, &keys),
            by_service.then(|| f.service_text()),
        );
        let g = map.entry(key.clone()).or_insert_with(|| FlowGroup {
            src: key.0.clone(),
            dst: key.1.clone(),
            service: key.2.clone(),
            flows: 0,
            connections: 0,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 0,
            blocked: 0,
            conns: ConnsByDecision::default(),
            draft: None,
            marks: None,
            change: None,
        });
        g.flows += 1;
        DraftCounts::add(&mut g.draft, f);
        g.connections += f.num_connections;
        g.bytes += f.bytes();
        tally(
            f.policy_decision,
            &mut g.allowed,
            &mut g.potentially_blocked,
            &mut g.blocked,
        );
        g.conns.add(f.policy_decision, f.num_connections);
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

    #[test]
    fn human_bytes_is_ls_h() {
        for (n, want) in [
            (0, "0B"),
            (3, "3B"),
            (512, "512B"),
            (1023, "1023B"),
            (1024, "1.0K"),
            (1536, "1.5K"),
            (10239, "9.9K"),
            (10240, "10K"),
            (12 * 1024 * 1024, "12M"),
            (2254857831, "2.1G"),
            (1 << 40, "1.0T"),
            (1 << 50, "1.0P"),
            (u64::MAX, "16383P"),
        ] {
            assert_eq!(human_bytes(n), want, "{n}");
        }
        assert_eq!(human_bytes((1 << 20) - 1), "1023K");
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pairs_count_what_the_draft_would_allow_and_block() {
        use crate::traffic::resolved::tests::rich_flows_draft;
        let k = keys(&["app", "env"]);
        assert!(
            group_by_labels(&rich_flows(), &k)
                .iter()
                .all(|g| g.draft.is_none())
        );
        let f = rich_flows_draft();
        for groups in [group_by_labels(&f, &k), group_by_service(&f, &k)] {
            let total = |pick: fn(&DraftCounts) -> u64| -> u64 {
                groups
                    .iter()
                    .map(|g| g.draft.as_ref().map_or(0, pick))
                    .sum()
            };
            assert_eq!(
                total(|d| d.draft_allowed),
                f.iter().filter(|f| f.fixed_by_draft()).count() as u64
            );
            assert_eq!(
                total(|d| d.draft_blocked),
                f.iter().filter(|f| f.blocked_by_draft()).count() as u64
            );
            assert!(groups.iter().all(|g| g.draft.is_some()));
        }
        let c = DraftCounts {
            draft_allowed: 2,
            draft_blocked: 1,
        };
        assert_eq!(DraftCounts::text(Some(&c)), "2 would allow · 1 would block");
        assert_eq!(DraftCounts::text(Some(&DraftCounts::default())), "-");
        assert_eq!(DraftCounts::text(None), "");
    }

    #[test]
    fn groups_count_connections_per_decision_and_serialize_them_flat() {
        let k = keys(&["app", "env"]);
        for groups in [
            group_by_labels(&rich_flows(), &k),
            group_by_service(&rich_flows(), &k),
        ] {
            for g in &groups {
                let c = &g.conns;
                assert_eq!(
                    c.allowed_conns + c.pot_blocked_conns + c.blocked_conns + c.unknown_conns,
                    g.connections,
                    "the four sum to the connections: {g:?}"
                );
                let decided = g.allowed + g.potentially_blocked + g.blocked == g.flows;
                assert_eq!(c.unknown_conns == 0, decided || g.connections == 0, "{g:?}");
                assert_eq!(c.allowed_conns > 0, g.allowed > 0, "{g:?}");
                assert_eq!(c.blocked_conns > 0, g.blocked > 0, "{g:?}");
            }
        }
        let groups = group_by_labels(&flows(), &k);
        let nas = groups
            .iter()
            .find(|g| g.src == vec!["unmanaged-nas", "-"])
            .unwrap();
        assert_eq!(
            (
                nas.conns.allowed_conns,
                nas.conns.pot_blocked_conns,
                nas.conns.blocked_conns
            ),
            (0, 0, nas.connections)
        );
        let json = serde_json::to_value(nas).unwrap();
        assert_eq!(json["blocked_conns"], nas.connections);
        assert_eq!(json["allowed_conns"], 0);
        assert_eq!(json["pot_blocked_conns"], 0);
        assert_eq!(json["unknown_conns"], 0);
        assert!(json.get("conns").is_none(), "flattened");
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
    fn group_by_service_splits_pairs_by_port_and_keeps_the_totals() {
        let k = keys(&["app", "env"]);
        let f = flows();
        let tuples = group_by_service(&f, &k);
        let icmp = tuples
            .iter()
            .find(|g| g.src == vec!["unmanaged-nas", "-"])
            .expect("the icmp flow");
        assert_eq!(icmp.service.as_deref(), Some("icmp"), "no port: just icmp");
        assert!(icmp.contains(&f[2], &k));
        assert!(group_by_labels(&f, &k).iter().all(|g| g.service.is_none()));

        let rich = rich_flows();
        let pairs = group_by_labels(&rich, &k);
        let tuples = group_by_service(&rich, &k);
        assert!(tuples.len() > pairs.len(), "some pair carries two services");
        assert!(tuples.iter().all(|g| g.service.is_some()));
        let sum = |gs: &[FlowGroup]| {
            gs.iter().fold((0, 0, 0, 0, 0, 0), |a, g| {
                (
                    a.0 + g.flows,
                    a.1 + g.connections,
                    a.2 + g.bytes,
                    a.3 + g.allowed,
                    a.4 + g.potentially_blocked,
                    a.5 + g.blocked,
                )
            })
        };
        assert_eq!(sum(&tuples), sum(&pairs), "every flow in exactly one tuple");
        for p in &pairs {
            let parts: Vec<&FlowGroup> = tuples
                .iter()
                .filter(|t| t.src == p.src && t.dst == p.dst)
                .collect();
            assert_eq!(
                parts.iter().map(|t| t.connections).sum::<u64>(),
                p.connections
            );
        }
        for t in &tuples {
            let behind: Vec<&ResolvedFlow> = rich.iter().filter(|f| t.contains(f, &k)).collect();
            assert_eq!(behind.len() as u64, t.flows);
            assert!(behind.iter().all(|f| Some(f.service_text()) == t.service));
        }
    }

    #[test]
    fn group_by_service_collapses_hosts_processes_and_users() {
        let k = keys(&["app", "env"]);
        let mut a = flows()[0].clone();
        a.process_name = Some("nginx".into());
        a.user_name = Some("www".into());
        let mut b = a.clone();
        b.src.hostname = Some("other-host".into());
        b.src.name = Some("other-host".into());
        b.src.ip = Some("10.9.9.9".into());
        b.dst.ip = Some("10.9.9.10".into());
        b.process_name = Some("httpd".into());
        b.user_name = Some("root".into());
        b.num_connections = 5;
        let tuples = group_by_service(&[a.clone(), b], &k);
        assert_eq!(tuples.len(), 1, "{tuples:?}");
        assert_eq!(tuples[0].flows, 2);
        assert_eq!(tuples[0].connections, a.num_connections + 5);
        assert_eq!(tuples[0].service, Some(a.service_text()));
        let json = serde_json::to_value(&tuples[0]).unwrap();
        assert_eq!(json["service"], a.service_text());
        let json = serde_json::to_value(&group_by_labels(&[a], &k)[0]).unwrap();
        assert!(json.get("service").is_none(), "pairs JSON unchanged");
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
