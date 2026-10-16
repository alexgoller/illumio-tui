//! Fan-in services (#39): an inbound port many peers reach (an
//! infrastructure port, a shared service) is a candidate for one rule in
//! place of one per peer. `c` (the TUI) or `--consolidate` (the CLI)
//! switches a candidate to Any (the built-in Any IP list as consumer) or
//! All Workloads (`ams`, unscoped consumers); `derive` then writes that one
//! rule. Pure: no requests.
use super::rows::is_any_ip_list;
use super::{GenRow, PeerKind, Side};
use crate::resolve::NameIndex;
use crate::write::PortSpec;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Distinct inbound peers on one port that make it a candidate
/// (`[profiles.<name>.generator] consolidate_min_peers`).
pub const DEFAULT_MIN_PEERS: usize = 5;
/// Distinct inbound peers that make an infrastructure port a candidate.
pub const INFRA_MIN_PEERS: usize = 3;

/// Infrastructure ports (tcp or udp): `(port, name, what the line says)`.
pub const INFRA_PORTS: &[(u16, &str, &str)] = &[
    (53, "dns", "DNS"),
    (88, "kerberos", "Kerberos"),
    (123, "ntp", "NTP"),
    (161, "snmp", "SNMP"),
    (162, "snmp", "SNMP trap"),
    (389, "ldap", "LDAP"),
    (445, "smb", "SMB"),
    (464, "kerberos", "Kerberos"),
    (514, "syslog", "syslog"),
    (636, "ldap", "LDAPS"),
    (3268, "ldap", "LDAP global catalog"),
    (5666, "monitoring", "monitoring agent"),
    (9100, "monitoring", "monitoring agent"),
    (10050, "monitoring", "monitoring agent"),
];

/// What a candidate's one rule names as consumer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Consolidate {
    /// One rule per peer, as without consolidation.
    #[default]
    Off,
    /// The built-in Any IP list (`0.0.0.0/0` and `::/0`).
    Any,
    /// All Workloads, unscoped consumers.
    AllWorkloads,
}

impl Consolidate {
    /// `c`: off → Any → All Workloads → off.
    pub fn next(self) -> Self {
        match self {
            Consolidate::Off => Consolidate::Any,
            Consolidate::Any => Consolidate::AllWorkloads,
            Consolidate::AllWorkloads => Consolidate::Off,
        }
    }

    /// `off`, `Any`, `All Workloads`.
    pub fn text(self) -> &'static str {
        match self {
            Consolidate::Off => "off",
            Consolidate::Any => "Any",
            Consolidate::AllWorkloads => "All Workloads",
        }
    }

    /// `off`, `any`, `all`: the CLI's `--consolidate` and the JSON.
    pub fn cli(self) -> &'static str {
        match self {
            Consolidate::Off => "off",
            Consolidate::Any => "any",
            Consolidate::AllWorkloads => "all",
        }
    }

    /// `any`, `all`, `off`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "any" => Some(Consolidate::Any),
            "all" | "all-workloads" | "all_workloads" => Some(Consolidate::AllWorkloads),
            "off" => Some(Consolidate::Off),
            _ => None,
        }
    }

    /// Whether the one rule takes the port's flows from this row: Any the
    /// IP side (IP, list and folded rows) and unchecked workload peers,
    /// while checked workload peers keep their own rule (it gives their
    /// outbound too); All Workloads every workload peer, IP peers keeping
    /// their rows.
    pub fn takes(self, row: &GenRow) -> bool {
        match self {
            Consolidate::Off => false,
            Consolidate::Any => !(is_workload(&row.peer) && row.proposed()),
            Consolidate::AllWorkloads => is_workload(&row.peer),
        }
    }
}

/// Intra-app, an app peer or an unlabelled workload: a workload source.
pub fn is_workload(peer: &PeerKind) -> bool {
    matches!(
        peer,
        PeerKind::Intra | PeerKind::App(_) | PeerKind::Workload { .. }
    )
}

/// Why a port is a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// At least `consolidate_min_peers` peers.
    MinPeers,
    /// A known infrastructure port with at least [`INFRA_MIN_PEERS`]:
    /// its name and its text.
    Infra(&'static str, &'static str),
}

impl Reason {
    /// `min_peers`, `infra:monitoring`: the JSON's `reason`.
    pub fn cli(self) -> String {
        match self {
            Reason::MinPeers => "min_peers".into(),
            Reason::Infra(name, _) => format!("infra:{name}"),
        }
    }
}

/// One inbound port many peers reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub spec: PortSpec,
    pub reason: Reason,
    /// The peers (rows other than intra-app and folded IPs), by row text.
    pub peers: Vec<String>,
}

impl Candidate {
    /// `5666/tcp from 9 peers (monitoring agent?)`.
    pub fn text(&self) -> String {
        let what = match self.reason {
            Reason::Infra(_, text) => format!(" ({text}?)"),
            Reason::MinPeers => String::new(),
        };
        format!(
            "{} from {} peers{what}",
            self.spec.display(),
            self.peers.len()
        )
    }
}

/// The infrastructure port `spec` is, if any (tcp or udp).
pub fn infra(spec: &PortSpec) -> Option<(&'static str, &'static str)> {
    if !matches!(spec.proto, 6 | 17) || spec.to_port.is_some() {
        return None;
    }
    let port = spec.port?;
    INFRA_PORTS
        .iter()
        .find(|(p, _, _)| *p == port)
        .map(|(_, n, t)| (*n, *t))
}

/// The inbound ports (tcp/udp with a port) whose distinct peers (app,
/// workload, IP list and IP rows; intra-app and folded IPs aside) reach
/// `min_peers`, or [`INFRA_MIN_PEERS`] on an [`INFRA_PORTS`] port. Most
/// peers first, then by port. Every row counts, checked or not.
pub fn candidates(rows: &[GenRow], min_peers: usize) -> Vec<Candidate> {
    let mut by: BTreeMap<&PortSpec, BTreeSet<String>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.side == Side::Inbound) {
        if matches!(r.peer, PeerKind::Intra | PeerKind::FoldedIps(_)) {
            continue;
        }
        for s in r
            .services
            .iter()
            .filter(|s| matches!(s.proto, 6 | 17) && s.port.is_some() && s.to_port.is_none())
        {
            by.entry(s).or_default().insert(r.peer_text());
        }
    }
    let mut out: Vec<Candidate> = by
        .into_iter()
        .filter_map(|(spec, peers)| {
            let reason = match infra(spec) {
                Some((n, t)) if peers.len() >= INFRA_MIN_PEERS => Reason::Infra(n, t),
                _ if peers.len() >= min_peers.max(2) => Reason::MinPeers,
                _ => return None,
            };
            Some(Candidate {
                spec: spec.clone(),
                reason,
                peers: peers.into_iter().collect(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.peers.len().cmp(&a.peers.len()).then(a.spec.cmp(&b.spec)));
    out
}

/// The built-in Any IP list `(href, name)`: by its external data set
/// first, else by its name. `None`: the Any option is unavailable (the
/// generator never creates one).
pub fn any_ip_list(idx: &NameIndex) -> Option<(String, String)> {
    let lists: Vec<&crate::models::IpList> = idx
        .ip_list_names()
        .iter()
        .filter_map(|n| idx.ip_list_href(n))
        .filter_map(|h| idx.ip_list(h))
        .filter(|l| l.update_type.as_deref() != Some("delete"))
        .collect();
    let builtin = lists
        .iter()
        .find(|l| l.external_data_set.as_deref() == Some("com.illumio.ilo_defaults"))
        .or_else(|| lists.iter().find(|l| is_any_ip_list(l)))?;
    Some((builtin.href.clone(), builtin.name.clone()))
}

/// Why `mode` cannot be used here; `None` when it can.
pub fn unavailable(mode: Consolidate, idx: &NameIndex) -> Option<&'static str> {
    (mode == Consolidate::Any && any_ip_list(idx).is_none())
        .then_some("the PCE has no built-in Any IP list (0.0.0.0/0 and ::/0)")
}

/// What a consolidated rule stands for (`GenRule::consolidated`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consolidated {
    pub spec: PortSpec,
    pub mode: Consolidate,
    /// The peers whose flows it takes (intra-app and folded IPs aside), by
    /// row text.
    pub peers: Vec<String>,
    /// Folded IP peers it takes (Any).
    pub folded_ips: usize,
    /// Of the rows it takes, the unchecked ones: allowed by this rule alone.
    pub unchecked: Vec<String>,
    /// The workloads it opens the port on: `every workload of shop/prod`,
    /// `role=db of shop/prod`.
    pub to: String,
    /// Rows on the port that keep their own rule: checked workload peers
    /// (Any), IP peers (All Workloads).
    pub kept: Vec<String>,
    /// Workload peers it takes (Any, unchecked ones): allowed at the
    /// destination only.
    pub workload_sources: Vec<String>,
}

impl Consolidated {
    /// What the rule allows beyond the checked peers: the unchecked ones,
    /// then the sources not seen.
    pub fn also_allows(&self) -> Vec<String> {
        let unseen = match self.mode {
            Consolidate::AllWorkloads => "any workload not seen",
            _ => "any source not seen",
        };
        self.unchecked
            .iter()
            .cloned()
            .chain(std::iter::once(unseen.to_string()))
            .collect()
    }

    /// `3 peers`, `3 peers +4 more IPs`.
    pub fn from_text(&self) -> String {
        let n = self.peers.len();
        let mut s = format!("{n} {}", if n == 1 { "peer" } else { "peers" });
        if self.folded_ips > 0 {
            s.push_str(&format!(" +{} more IPs", self.folded_ips));
        }
        s
    }

    /// What to know before writing it, the consequence first. Explorer's
    /// flows carry no enforcement mode, so workload sources are named, not
    /// sorted by mode.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self.mode {
            // Only the destination enforces an IP list consumer.
            Consolidate::Any => {
                if !self.workload_sources.is_empty() {
                    out.push(format!(
                        "destination only for {}: outbound rule needed in full enforcement",
                        self.workload_sources.join(", ")
                    ));
                }
                if !self.kept.is_empty() {
                    out.push(format!(
                        "keeps {} managed peer {} (their outbound); All Workloads covers managed sources in one rule",
                        self.kept.len(),
                        if self.kept.len() == 1 { "rule" } else { "rules" },
                    ));
                }
                if out.is_empty() {
                    out.push(
                        "destination only: outbound rules needed for workloads in full enforcement"
                            .into(),
                    );
                }
            }
            Consolidate::AllWorkloads => {
                out.push("every VEN recomputes: the rule reaches every managed workload".into());
                if !self.kept.is_empty() {
                    out.push(format!(
                        "not for IP peers ({}): their own rows",
                        self.kept.join(", ")
                    ));
                }
            }
            Consolidate::Off => {}
        }
        out
    }

    /// The lines `p` and the confirm show under the rule: the warnings
    /// first.
    pub fn notes(&self) -> Vec<String> {
        let mut out = self.warnings();
        out.push(format!(
            "consolidates {} from {} → {} · also allows: {}",
            self.spec.display(),
            self.from_text(),
            self.to,
            self.also_allows().join(", ")
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::DecisionCounts;

    fn row(peer: PeerKind, ports: &[&str]) -> GenRow {
        GenRow {
            side: Side::Inbound,
            peer,
            checked: true,
            checkable: true,
            stop: None,
            flags: Vec::new(),
            flows: 1,
            connections: 1,
            decisions: DecisionCounts::default(),
            services: ports.iter().map(|p| PortSpec::parse(p).unwrap()).collect(),
        }
    }

    fn app(name: &str, ports: &[&str]) -> GenRow {
        row(PeerKind::App(vec![name.into(), "prod".into()]), ports)
    }

    #[test]
    fn five_peers_or_three_on_an_infra_port_make_a_candidate() {
        let mut rows = vec![row(PeerKind::Intra, &["443/tcp", "5666/tcp"])];
        for n in ["a", "b", "c"] {
            rows.push(app(n, &["443/tcp", "5666/tcp", "53/udp"]));
        }
        rows.push(app("d", &["443/tcp"]));
        rows.push(row(
            PeerKind::Ip {
                ip: "10.0.0.1".into(),
            },
            &["443/tcp"],
        ));
        rows.push(row(PeerKind::FoldedIps(9), &["443/tcp", "8080/tcp"]));
        let mut out_row = app("e", &["443/tcp"]);
        out_row.side = Side::Outbound;
        rows.push(out_row);
        let c = candidates(&rows, DEFAULT_MIN_PEERS);
        let shown: Vec<String> = c.iter().map(Candidate::text).collect();
        assert_eq!(
            shown,
            vec![
                "443/tcp from 5 peers",
                "5666/tcp from 3 peers (monitoring agent?)",
                "53/udp from 3 peers (DNS?)",
            ],
            "intra-app, folded IPs and outbound rows are not peers"
        );
        assert_eq!(c[0].reason.cli(), "min_peers");
        assert_eq!(c[1].reason.cli(), "infra:monitoring");
        assert!(
            candidates(&rows, 6)
                .iter()
                .all(|c| c.spec.port != Some(443)),
            "below the threshold"
        );
    }

    #[test]
    fn c_cycles_off_any_all_workloads() {
        let m = Consolidate::Off;
        assert_eq!(m.next(), Consolidate::Any);
        assert_eq!(m.next().next(), Consolidate::AllWorkloads);
        assert_eq!(m.next().next().next(), Consolidate::Off);
        assert_eq!(Consolidate::parse("ALL"), Some(Consolidate::AllWorkloads));
        assert_eq!(Consolidate::parse("none"), None);
        let ip = row(PeerKind::Ip { ip: "x".into() }, &[]);
        let mut intra = row(PeerKind::Intra, &[]);
        assert!(Consolidate::Any.takes(&ip));
        assert!(!Consolidate::AllWorkloads.takes(&ip));
        assert!(Consolidate::AllWorkloads.takes(&intra));
        assert!(!Consolidate::Any.takes(&intra), "checked: keeps its rule");
        intra.checked = false;
        assert!(
            Consolidate::Any.takes(&intra),
            "unchecked: destination side"
        );
    }

    #[test]
    fn the_any_list_is_found_by_its_data_set_or_its_name() {
        let lists: Vec<crate::models::IpList> = serde_json::from_value(serde_json::json!([
            {"href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Corp",
             "ip_ranges": [{"from_ip": "10.0.0.0/8"}]},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/2", "name": "Any (0.0.0.0/0 and ::/0)",
             "ip_ranges": [{"from_ip": "0.0.0.0/0"}]}
        ]))
        .unwrap();
        let idx = NameIndex::new(&[], &[], &lists, &[]);
        assert_eq!(
            any_ip_list(&idx).map(|x| x.0).as_deref(),
            Some("/orgs/5/sec_policy/draft/ip_lists/2")
        );
        let idx = NameIndex::new(&[], &[], &lists[..1], &[]);
        assert_eq!(any_ip_list(&idx), None);
        assert_eq!(
            unavailable(Consolidate::Any, &idx),
            Some("the PCE has no built-in Any IP list (0.0.0.0/0 and ::/0)")
        );
        assert_eq!(unavailable(Consolidate::AllWorkloads, &idx), None);
    }

    #[test]
    fn the_notes_lead_with_the_mode_s_catch_and_say_what_it_also_allows() {
        let mut c = Consolidated {
            spec: PortSpec::parse("5666/tcp").unwrap(),
            mode: Consolidate::Any,
            peers: vec!["a/prod".into(), "b/dev".into()],
            folded_ips: 4,
            unchecked: vec!["b/dev".into()],
            to: "every workload of shop/prod".into(),
            kept: vec!["intra-app".into()],
            workload_sources: vec!["b/dev".into()],
        };
        assert_eq!(
            c.notes(),
            vec![
                "destination only for b/dev: outbound rule needed in full enforcement",
                "keeps 1 managed peer rule (their outbound); All Workloads covers managed sources in one rule",
                "consolidates 5666/tcp from 2 peers +4 more IPs → every workload of shop/prod · also allows: b/dev, any source not seen",
            ]
        );
        c.workload_sources.clear();
        c.kept.clear();
        assert_eq!(
            c.warnings(),
            vec!["destination only: outbound rules needed for workloads in full enforcement"]
        );
        c.mode = Consolidate::AllWorkloads;
        c.kept = vec!["10.0.0.1".into()];
        assert_eq!(
            c.warnings(),
            vec![
                "every VEN recomputes: the rule reaches every managed workload",
                "not for IP peers (10.0.0.1): their own rows",
            ]
        );
        assert_eq!(c.also_allows().last().unwrap(), "any workload not seen");
    }
}
