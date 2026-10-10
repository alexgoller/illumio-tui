//! Pure aggregations over resolved flows. The TUI's app-to-app and blocked
//! views and the CLI's `--group` / `--blocked` use these; no requests.
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::models::PolicyDecision;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowGroup {
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub flows: u64,
    pub connections: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

/// Group key for one endpoint: the label value per key, `-` when the label
/// is missing. An endpoint without labels at all puts its name (or
/// `ipl:<list>`) in the first slot so it still reads sensibly.
fn group_key(e: &ResolvedEndpoint, keys: &[String]) -> Vec<String> {
    if e.labels.is_empty() || !e.is_workload() {
        let name = e
            .ip_lists
            .first()
            .map(|l| format!("ipl:{l}"))
            .unwrap_or_else(|| e.display_name());
        let mut k = vec![name];
        k.extend(std::iter::repeat_n(
            "-".to_string(),
            keys.len().saturating_sub(1),
        ));
        return k;
    }
    keys.iter()
        .map(|k| e.label_value(k).unwrap_or("-").to_string())
        .collect()
}

pub fn group_by_labels(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup> {
    let keys: Vec<String> = if keys.is_empty() {
        vec!["app".into(), "env".into()]
    } else {
        keys.to_vec()
    };
    let mut map: BTreeMap<(Vec<String>, Vec<String>), FlowGroup> = BTreeMap::new();
    for f in flows {
        let key = (group_key(&f.src, &keys), group_key(&f.dst, &keys));
        let g = map.entry(key.clone()).or_insert_with(|| FlowGroup {
            src: key.0.clone(),
            dst: key.1.clone(),
            flows: 0,
            connections: 0,
            allowed: 0,
            potentially_blocked: 0,
            blocked: 0,
        });
        g.flows += 1;
        g.connections += f.num_connections;
        match f.policy_decision {
            PolicyDecision::Allowed => g.allowed += 1,
            PolicyDecision::PotentiallyBlocked => g.potentially_blocked += 1,
            PolicyDecision::Blocked => g.blocked += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let mut out: Vec<FlowGroup> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.src.cmp(&b.src))
    });
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedGroup {
    pub dst: String,
    pub dst_labels: String,
    pub port: Option<u16>,
    pub proto: String,
    pub flows: u64,
    pub connections: u64,
    /// Distinct source names, at most 5.
    pub sources: Vec<String>,
    pub last_detected: Option<String>,
}

pub fn blocked_summary(flows: &[ResolvedFlow]) -> Vec<BlockedGroup> {
    let mut map: BTreeMap<(String, Option<u16>, String), BlockedGroup> = BTreeMap::new();
    for f in flows.iter().filter(|f| {
        matches!(
            f.policy_decision,
            PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
        )
    }) {
        let dst = f.dst.display_name();
        let key = (dst.clone(), f.port, f.proto.clone());
        let g = map.entry(key).or_insert_with(|| BlockedGroup {
            dst,
            dst_labels: f.dst.label_text(),
            port: f.port,
            proto: f.proto.clone(),
            flows: 0,
            connections: 0,
            sources: Vec::new(),
            last_detected: None,
        });
        g.flows += 1;
        g.connections += f.num_connections;
        let src = f.src.display_name();
        if !g.sources.contains(&src) && g.sources.len() < 5 {
            g.sources.push(src);
        }
        if f.last_detected > g.last_detected {
            g.last_detected = f.last_detected.clone();
        }
    }
    let mut out: Vec<BlockedGroup> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.last_detected
            .cmp(&a.last_detected)
            .then_with(|| a.dst.cmp(&b.dst))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::flows;

    #[test]
    fn groups_by_label_pair_with_fallbacks() {
        let groups = group_by_labels(&flows(), &["app".to_string(), "env".to_string()]);
        assert_eq!(groups[0].src, vec!["shop", "prod"]);
        assert_eq!(groups[0].dst, vec!["shop", "prod"]);
        assert_eq!(groups[0].flows, 1);
        assert_eq!(groups[0].connections, 310);
        assert_eq!(groups[0].allowed, 1);
        let corp = groups
            .iter()
            .find(|g| g.src == vec!["ipl:Corp", "-"])
            .expect("ip list source falls back to its name");
        assert_eq!(corp.potentially_blocked, 1);
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
    fn blocked_summary_groups_by_destination_and_port() {
        let b = blocked_summary(&flows());
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].dst, "host-1.example");
        assert_eq!(b[0].port, Some(8443));
        assert_eq!(b[0].proto, "tcp");
        assert_eq!(b[0].sources, vec!["10.1.4.9"]);
        assert_eq!(b[0].last_detected.as_deref(), Some("2026-10-10T07:30:58Z"));
        assert_eq!(b[1].dst, "host-2.example");
        assert_eq!(b[1].dst_labels, "app=shop env=prod role=db");
        assert_eq!(b[1].proto, "icmp");
        assert_eq!(b[1].connections, 3);
    }
}
