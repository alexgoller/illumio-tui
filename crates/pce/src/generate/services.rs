//! A flow's service as a rule names it: tcp and udp inline (`443/tcp`),
//! icmp through the ICMP service object, another protocol through the one
//! service whose only entry is that protocol. Existing service objects are
//! never matched by their ports.
use crate::models::proto_number;
use crate::resolve::NameIndex;
use crate::traffic::ResolvedFlow;
use crate::write::PortSpec;
use std::collections::BTreeSet;

/// A flow's port and protocol: tcp and udp keep their port, other
/// protocols have none. An unknown protocol is `-1`.
pub fn flow_spec(f: &ResolvedFlow) -> PortSpec {
    let proto = proto_number(&f.proto).map_or(-1, i64::from);
    let port = if matches!(proto, 6 | 17) {
        f.port
    } else {
        None
    };
    PortSpec {
        port,
        to_port: None,
        proto,
    }
}

/// How a seen service goes into a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// tcp or udp, inline.
    Inline(PortSpec),
    /// A service object's href.
    Object(String),
    /// Nothing expresses it: the protocol number.
    Unsupported(i64),
}

/// tcp and udp inline, with their port (a flow without one would read
/// as every port: unsupported); icmp and icmpv6 through the ICMP object
/// when it holds that protocol untyped, else the one object of exactly
/// that protocol; others through the one object of exactly theirs.
pub fn express(spec: &PortSpec, idx: &NameIndex) -> Expr {
    match spec.proto {
        6 | 17 if spec.port.is_some() => Expr::Inline(spec.clone()),
        6 | 17 => Expr::Unsupported(spec.proto),
        p @ (1 | 58) => idx
            .icmp_service()
            .filter(|s| {
                s.service_ports
                    .iter()
                    .any(|e| e.proto == Some(p) && e.icmp_type.is_none())
            })
            .map(|s| s.href.clone())
            .or_else(|| single_proto_service(idx, p))
            .map_or(Expr::Unsupported(p), Expr::Object),
        -1 => Expr::Unsupported(-1),
        p => single_proto_service(idx, p).map_or(Expr::Unsupported(p), Expr::Object),
    }
}

/// The one live service whose only entry is protocol `proto` without a
/// port; `None` for none or several.
fn single_proto_service(idx: &NameIndex, proto: i64) -> Option<String> {
    let hits: Vec<&crate::models::Service> = idx
        .services()
        .into_iter()
        .filter(|s| {
            matches!(s.service_ports.as_slice(), [p] if p.proto == Some(proto) && p.port.is_none())
        })
        .collect();
    match hits.as_slice() {
        [one] => Some(one.href.clone()),
        _ => None,
    }
}

/// Consecutive ports of one protocol as one range (`8000-8002/tcp`):
/// only ports that were all seen, so the rule stays exact.
pub fn fold_ports(ports: &BTreeSet<PortSpec>) -> Vec<PortSpec> {
    let mut out: Vec<PortSpec> = Vec::new();
    for p in ports {
        if let Some(last) = out.last_mut()
            && last.proto == p.proto
            && p.to_port.is_none()
            && let (Some(start), Some(port)) = (last.port, p.port)
            && last.to_port.unwrap_or(start).checked_add(1) == Some(port)
        {
            last.to_port = Some(port);
            continue;
        }
        out.push(p.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, Service};
    use crate::traffic::resolved::tests::rich_flows;

    fn spec(s: &str) -> PortSpec {
        PortSpec::parse(s).unwrap()
    }

    fn index_with(services: &[Service]) -> NameIndex {
        let labels: Vec<Label> = Vec::new();
        let groups: Vec<LabelGroup> = Vec::new();
        let lists: Vec<IpList> = Vec::new();
        NameIndex::new(&labels, &groups, &lists, services)
    }

    fn service(href: &str, name: &str, ports: serde_json::Value) -> Service {
        serde_json::from_value(serde_json::json!({
            "href": href, "name": name, "service_ports": ports
        }))
        .unwrap()
    }

    #[test]
    fn a_flow_keeps_its_port_only_for_tcp_and_udp() {
        let f = rich_flows();
        assert_eq!(flow_spec(&f[0]), spec("5432/tcp"));
        assert_eq!(flow_spec(&f[27]), spec("53/udp"));
        assert_eq!(flow_spec(&f[25]), spec("icmp"), "flow 25 is icmp");
    }

    #[test]
    fn icmp_takes_the_icmp_object_and_other_protocols_the_single_matching_one() {
        let none = index_with(&[]);
        assert_eq!(
            express(&spec("443/tcp"), &none),
            Expr::Inline(spec("443/tcp"))
        );
        assert_eq!(express(&spec("icmp"), &none), Expr::Unsupported(1));
        let gre = PortSpec {
            port: None,
            to_port: None,
            proto: 47,
        };
        assert_eq!(express(&gre, &none), Expr::Unsupported(47));
        let one = index_with(&[
            service(
                "/orgs/5/sec_policy/draft/services/7",
                "ICMP",
                serde_json::json!([{"proto": 1}]),
            ),
            service(
                "/orgs/5/sec_policy/draft/services/8",
                "GRE",
                serde_json::json!([{"proto": 47}]),
            ),
        ]);
        assert_eq!(
            express(&spec("icmp"), &one),
            Expr::Object("/orgs/5/sec_policy/draft/services/7".into())
        );
        assert_eq!(
            express(&gre, &one),
            Expr::Object("/orgs/5/sec_policy/draft/services/8".into())
        );
        let two = index_with(&[
            service(
                "/orgs/5/sec_policy/draft/services/8",
                "GRE",
                serde_json::json!([{"proto": 47}]),
            ),
            service(
                "/orgs/5/sec_policy/draft/services/9",
                "GRE2",
                serde_json::json!([{"proto": 47}]),
            ),
        ]);
        assert_eq!(
            express(&gre, &two),
            Expr::Unsupported(47),
            "two match: none is taken"
        );
        let unknown = PortSpec {
            port: None,
            to_port: None,
            proto: -1,
        };
        assert_eq!(express(&unknown, &one), Expr::Unsupported(-1));
    }

    #[test]
    fn icmpv6_needs_an_object_that_holds_protocol_58() {
        let v6 = PortSpec {
            port: None,
            to_port: None,
            proto: 58,
        };
        let v4_only = index_with(&[service(
            "/orgs/5/sec_policy/draft/services/7",
            "ICMP",
            serde_json::json!([{"proto": 1}]),
        )]);
        assert_eq!(express(&v6, &v4_only), Expr::Unsupported(58));
        let typed = index_with(&[service(
            "/orgs/5/sec_policy/draft/services/7",
            "ICMP",
            serde_json::json!([{"proto": 1}, {"proto": 58, "icmp_type": 128}]),
        )]);
        assert_eq!(express(&v6, &typed), Expr::Unsupported(58), "echo only");
        let both = index_with(&[service(
            "/orgs/5/sec_policy/draft/services/7",
            "ICMP",
            serde_json::json!([{"proto": 1}, {"proto": 58}]),
        )]);
        assert_eq!(
            express(&v6, &both),
            Expr::Object("/orgs/5/sec_policy/draft/services/7".into())
        );
    }

    #[test]
    fn tcp_or_udp_without_a_port_is_never_every_port() {
        for proto in [6, 17] {
            let bare = PortSpec {
                port: None,
                to_port: None,
                proto,
            };
            assert_eq!(express(&bare, &index_with(&[])), Expr::Unsupported(proto));
        }
    }

    #[test]
    fn only_consecutive_seen_ports_fold_into_a_range() {
        let ports: BTreeSet<PortSpec> = [
            "8000/tcp", "8001/tcp", "8002/tcp", "8004/tcp", "8003/udp", "53/udp",
        ]
        .into_iter()
        .map(spec)
        .collect();
        let folded: Vec<String> = fold_ports(&ports).iter().map(PortSpec::display).collect();
        assert_eq!(
            folded,
            vec!["8000-8002/tcp", "8004/tcp", "53/udp", "8003/udp"]
        );
    }
}
