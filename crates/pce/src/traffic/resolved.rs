//! Flow rows with hrefs replaced by names: what gets printed and displayed.
use crate::models::{Endpoint, Flow, PolicyDecision, proto_name};
use crate::resolve::{NameIndex, ResolvedLabel};
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ResolvedEndpoint {
    pub ip: Option<String>,
    pub fqdn: Option<String>,
    pub hostname: Option<String>,
    pub name: Option<String>,
    pub workload_href: Option<String>,
    pub labels: Vec<ResolvedLabel>,
    pub ip_lists: Vec<String>,
}

impl ResolvedEndpoint {
    fn from_endpoint(e: &Endpoint, idx: &NameIndex) -> Self {
        let wl = e.workload.as_ref();
        Self {
            ip: e.ip.clone(),
            fqdn: e.fqdn.clone(),
            hostname: wl.and_then(|w| w.hostname.clone()),
            name: wl.and_then(|w| w.name.clone()),
            workload_href: wl.and_then(|w| w.href.clone()),
            labels: wl
                .map(|w| idx.resolve_labels(&w.labels))
                .unwrap_or_default(),
            ip_lists: e
                .ip_lists
                .iter()
                .filter_map(|l| {
                    l.name.clone().or_else(|| {
                        l.href
                            .as_deref()
                            .and_then(|h| idx.ip_list_name(h))
                            .map(str::to_string)
                    })
                })
                .collect(),
        }
    }

    /// hostname, else workload name, else fqdn, else ip, else `?`.
    pub fn display_name(&self) -> String {
        [&self.hostname, &self.name, &self.fqdn, &self.ip]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "?".into())
    }

    pub fn label_value(&self, key: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|l| l.key == key)
            .map(|l| l.value.as_str())
    }

    /// `app=shop env=prod role=web`
    pub fn label_text(&self) -> String {
        self.labels
            .iter()
            .map(|l| format!("{}={}", l.key, l.value))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn is_workload(&self) -> bool {
        self.workload_href.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedFlow {
    pub src: ResolvedEndpoint,
    pub dst: ResolvedEndpoint,
    pub port: Option<u16>,
    pub proto: String,
    pub process_name: Option<String>,
    pub user_name: Option<String>,
    pub num_connections: u64,
    pub policy_decision: PolicyDecision,
    pub draft_policy_decision: Option<String>,
    pub flow_direction: Option<String>,
    pub first_detected: Option<String>,
    pub last_detected: Option<String>,
    pub rules: Vec<String>,
}

impl ResolvedFlow {
    /// `5432/tcp`, or just `icmp` when there is no port.
    pub fn service_text(&self) -> String {
        match self.port {
            Some(p) => format!("{p}/{}", self.proto),
            None => self.proto.clone(),
        }
    }
}

pub fn resolve_flows(flows: &[Flow], idx: &NameIndex) -> Vec<ResolvedFlow> {
    flows
        .iter()
        .map(|f| ResolvedFlow {
            src: ResolvedEndpoint::from_endpoint(&f.src, idx),
            dst: ResolvedEndpoint::from_endpoint(&f.dst, idx),
            port: f.service.port,
            proto: f
                .service
                .proto
                .map(proto_name)
                .unwrap_or_else(|| "?".into()),
            process_name: f.service.process_name.clone(),
            user_name: f.service.user_name.clone(),
            num_connections: f.num_connections,
            policy_decision: f.policy_decision,
            draft_policy_decision: f.draft_policy_decision.clone(),
            flow_direction: f.flow_direction.clone(),
            first_detected: f
                .timestamp_range
                .as_ref()
                .and_then(|t| t.first_detected.clone()),
            last_detected: f
                .timestamp_range
                .as_ref()
                .and_then(|t| t.last_detected.clone()),
            rules: f.rules.iter().map(|r| r.href.clone()).collect(),
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::{Flow, IpList, Label, LabelGroup, PolicyDecision, Service};
    use crate::resolve::NameIndex;

    pub(crate) fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    pub(crate) fn flows() -> Vec<ResolvedFlow> {
        let raw: Vec<Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        resolve_flows(&raw, &index())
    }

    #[test]
    fn resolves_names_labels_and_services() {
        let f = flows();
        assert_eq!(f[0].src.display_name(), "host-1.example");
        assert_eq!(f[0].src.label_text(), "app=shop env=prod role=web");
        assert_eq!(f[0].src.label_value("role"), Some("web"));
        assert_eq!(f[0].dst.label_value("role"), Some("db"));
        assert_eq!(f[0].service_text(), "5432/tcp");
        assert_eq!(f[0].process_name.as_deref(), Some("postgres"));
        assert_eq!(f[0].policy_decision, PolicyDecision::Allowed);
        assert_eq!(f[1].src.display_name(), "10.1.4.9");
        assert_eq!(f[1].src.ip_lists, vec!["Corp"]);
        assert_eq!(f[1].src.label_text(), "");
        assert_eq!(f[3].dst.display_name(), "fqdn-1.example");
        assert_eq!(
            f[3].rules,
            vec!["/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"]
        );
    }

    #[test]
    fn icmp_flow_without_port_renders() {
        let f = flows();
        assert_eq!(f[2].port, None);
        assert_eq!(f[2].proto, "icmp");
        assert_eq!(f[2].service_text(), "icmp");
        assert_eq!(f[2].src.display_name(), "unmanaged-nas");
    }
}
