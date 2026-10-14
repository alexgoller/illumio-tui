//! Flow rows with hrefs replaced by names: what gets printed and displayed.
use crate::key_order::KeyOrder;
use crate::models::{DraftDecision, Endpoint, Flow, PolicyDecision, proto_name};
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

    /// `app=shop env=prod role=web`: `labels` in their (already sorted)
    /// order, minus the keys `order` does not show.
    pub fn label_text(&self, order: &KeyOrder) -> String {
        self.labels
            .iter()
            .filter(|l| order.shows(&l.key))
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
    /// The decision under the draft policy, after a draft re-evaluation.
    pub draft_policy_decision: Option<DraftDecision>,
    pub flow_direction: Option<String>,
    pub first_detected: Option<String>,
    pub last_detected: Option<String>,
    pub rules: Vec<String>,
    /// Bytes the destination received (`dst_bi`), when the PCE reported it.
    pub bytes_in: Option<u64>,
    /// Bytes the destination sent (`dst_bo`), when the PCE reported it.
    pub bytes_out: Option<u64>,
}

impl ResolvedFlow {
    /// `bytes_in + bytes_out`, 0 when the PCE reported neither.
    pub fn bytes(&self) -> u64 {
        self.bytes_in.unwrap_or(0) + self.bytes_out.unwrap_or(0)
    }

    /// Whether the PCE reported a byte count for this flow.
    pub fn has_bytes(&self) -> bool {
        self.bytes_in.is_some() || self.bytes_out.is_some()
    }

    /// `5432/tcp`, or just `icmp` when there is no port.
    pub fn service_text(&self) -> String {
        service_text(self.port, &self.proto)
    }

    /// Blocked or potentially blocked now, allowed under the draft policy:
    /// a flow the draft rules would fix.
    pub fn fixed_by_draft(&self) -> bool {
        is_blocked(self.policy_decision)
            && self
                .draft_policy_decision
                .as_ref()
                .is_some_and(|d| d.class() == PolicyDecision::Allowed)
    }

    /// Allowed now, blocked or potentially blocked under the draft policy.
    pub fn blocked_by_draft(&self) -> bool {
        self.policy_decision == PolicyDecision::Allowed
            && self
                .draft_policy_decision
                .as_ref()
                .is_some_and(|d| is_blocked(d.class()))
    }

    /// The draft decision when its verdict is not the reported one: what
    /// the Flows view marks (`blocked → allowed`). Judged by class, so
    /// `allowed_across_boundary` on an allowed flow is no change; a value
    /// of no known class always is.
    pub fn draft_change(&self) -> Option<&DraftDecision> {
        self.draft_policy_decision
            .as_ref()
            .filter(|d| d.class() == PolicyDecision::Unknown || d.class() != self.policy_decision)
    }
}

fn is_blocked(d: PolicyDecision) -> bool {
    matches!(
        d,
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
    )
}

/// What a draft re-evaluation changes in a set of flows: the flows the
/// draft would allow (`fixed`) and those it would block (`newly_blocked`).
/// `evaluated` is whether any flow carries a draft decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DraftSummary {
    pub evaluated: bool,
    pub fixed: u64,
    pub newly_blocked: u64,
}

impl DraftSummary {
    pub fn of<'a>(flows: impl IntoIterator<Item = &'a ResolvedFlow>) -> Self {
        let mut s = DraftSummary::default();
        for f in flows {
            s.evaluated |= f.draft_policy_decision.is_some();
            s.fixed += u64::from(f.fixed_by_draft());
            s.newly_blocked += u64::from(f.blocked_by_draft());
        }
        s
    }
}

/// `5432/tcp`, or just `icmp` when there is no port.
pub(crate) fn service_text(port: Option<u16>, proto: &str) -> String {
    match port {
        Some(p) => format!("{p}/{proto}"),
        None => proto.to_string(),
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
            rules: f.rules.iter().filter_map(|r| r.text()).collect(),
            bytes_in: f.dst_bi,
            bytes_out: f.dst_bo,
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

    /// `index()` plus the labels only the rich fixture uses (`app=crm`,
    /// `env=dev`, `role=api`, `loc=us`, `datacenter=fra1`, ...).
    pub(crate) fn rich_index() -> NameIndex {
        let mut labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let rich: Vec<Label> = serde_json::from_str(include_str!(
            "../../../../fixtures/traffic_labels_rich.json"
        ))
        .unwrap();
        labels.extend(rich);
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    /// The 30-flow fixture: 4 apps x 2 envs, roles, `loc`, `datacenter` on
    /// some workloads, the Corp IP list and an FQDN, users, processes, both
    /// directions, bytes on 9 flows.
    pub(crate) fn rich_flows() -> Vec<ResolvedFlow> {
        let raw: Vec<Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows_rich.json"))
                .unwrap();
        resolve_flows(&raw, &rich_index())
    }

    /// `rich_flows` after a draft re-evaluation: every other blocked or
    /// potentially blocked flow is allowed in draft, every seventh allowed
    /// one (from the fourth) blocked, the rest keep their decision.
    pub(crate) fn rich_flows_draft() -> Vec<ResolvedFlow> {
        use crate::models::DraftDecision;
        let mut f = rich_flows();
        for (i, x) in f.iter_mut().enumerate() {
            let d = x.policy_decision;
            x.draft_policy_decision = Some(match d {
                PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked if i % 2 == 0 => {
                    DraftDecision::Allowed
                }
                PolicyDecision::Allowed if i % 7 == 3 => DraftDecision::Blocked,
                d => DraftDecision::parse(d.as_str()),
            });
        }
        f
    }

    pub(crate) fn flows() -> Vec<ResolvedFlow> {
        let raw: Vec<Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        resolve_flows(&raw, &index())
    }

    #[test]
    fn draft_changes_are_counted_by_class() {
        use crate::models::DraftDecision as D;
        let mut f = rich_flows();
        let at = |f: &[ResolvedFlow], d: PolicyDecision| {
            f.iter().position(|x| x.policy_decision == d).unwrap()
        };
        assert_eq!(DraftSummary::of(&f), DraftSummary::default());
        let (b, p, a) = (
            at(&f, PolicyDecision::Blocked),
            at(&f, PolicyDecision::PotentiallyBlocked),
            at(&f, PolicyDecision::Allowed),
        );
        f[b].draft_policy_decision = Some(D::Allowed);
        f[p].draft_policy_decision = Some(D::Other("allowed_across_boundary".into()));
        f[a].draft_policy_decision = Some(D::Blocked);
        assert!(f[b].fixed_by_draft() && f[p].fixed_by_draft());
        assert!(f[a].blocked_by_draft() && !f[a].fixed_by_draft());
        assert_eq!(f[b].draft_change(), Some(&D::Allowed));
        f[b].draft_policy_decision = Some(D::Blocked);
        assert_eq!(f[b].draft_change(), None, "the same decision is no change");
        f[b].draft_policy_decision = Some(D::Other("blocked_by_override_deny".into()));
        assert_eq!(f[b].draft_change(), None, "nor the same verdict, refined");
        f[b].draft_policy_decision = Some(D::Other("new_value".into()));
        assert!(
            f[b].draft_change().is_some(),
            "an unknown value always shows"
        );
        f[b].draft_policy_decision = Some(D::Blocked);
        assert_eq!(
            DraftSummary::of(&f),
            DraftSummary {
                evaluated: true,
                fixed: 1,
                newly_blocked: 1
            }
        );
    }

    #[test]
    fn resolves_names_labels_and_services() {
        let f = flows();
        assert_eq!(f[0].src.display_name(), "host-1.example");
        assert_eq!(
            f[0].src.label_text(&KeyOrder::default()),
            "app=shop env=prod role=web"
        );
        assert_eq!(f[0].src.label_value("role"), Some("web"));
        assert_eq!(f[0].dst.label_value("role"), Some("db"));
        assert_eq!(f[0].service_text(), "5432/tcp");
        assert_eq!(f[0].process_name.as_deref(), Some("postgres"));
        assert_eq!(f[0].policy_decision, PolicyDecision::Allowed);
        assert_eq!(f[1].src.display_name(), "10.1.4.9");
        assert_eq!(f[1].src.ip_lists, vec!["Corp"]);
        assert_eq!(f[1].src.label_text(&KeyOrder::default()), "");
        assert_eq!(f[3].dst.display_name(), "fqdn-1.example");
        assert_eq!(
            f[3].rules,
            vec!["/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"]
        );
    }

    #[test]
    fn endpoint_labels_follow_the_index_key_order() {
        let raw: Vec<Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        let idx = index().with_key_order(crate::KeyOrder::new(&["env".into()], "role"));
        let f = resolve_flows(&raw, &idx);
        assert_eq!(
            f[0].src.label_text(&KeyOrder::default()),
            "env=prod role=web app=shop"
        );
        assert_eq!(f[1].src.label_text(&KeyOrder::default()), "");
    }

    #[test]
    fn endpoint_label_text_hides_ignored_keys_but_labels_keep_them() {
        let f = flows();
        let order = KeyOrder::default().with_ignored(&["env".into()]);
        assert_eq!(f[0].src.label_text(&order), "app=shop role=web");
        assert_eq!(f[0].src.label_value("env"), Some("prod"));
        let json = serde_json::to_string(&f[0].src).unwrap();
        assert!(json.contains("\"key\":\"env\""), "{json}");
    }

    #[test]
    fn bytes_are_copied_from_dst_bi_and_dst_bo() {
        let f = flows();
        assert_eq!(
            (f[0].bytes_in, f[0].bytes_out),
            (Some(120_000), Some(4_000))
        );
        assert_eq!(f[0].bytes(), 124_000);
        assert!(f[0].has_bytes());
        assert_eq!((f[1].bytes_in, f[1].bytes_out), (None, None));
        assert_eq!(f[1].bytes(), 0);
        assert!(!f[1].has_bytes());
        let rich = rich_flows();
        assert_eq!(rich.len(), 30);
        assert_eq!(rich.iter().filter(|f| f.has_bytes()).count(), 9);
        assert_eq!(
            rich.iter().map(ResolvedFlow::bytes).sum::<u64>(),
            13_625_100
        );
        assert_eq!(rich[4].src.label_value("app"), Some("crm"));
        assert_eq!(rich[4].src.label_value("datacenter"), Some("fra1"));
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
