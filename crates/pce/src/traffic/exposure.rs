//! Exposure (#82): which risky services (RDP, SMB, WinRM, SSH, database
//! ports …) reach each destination group, from where, and under which
//! decision. One Explorer query over the list's ports brings the flows;
//! this module names the list, checks a profile's own, and turns the flows
//! into one row per destination group and risky service. No request here.
use super::aggregate::{ConnsByDecision, DraftCounts, group_key, group_values};
use super::filter::parse_service;
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::models::{PolicyDecision, ServiceMatch, proto_name, proto_number};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The risky services when a profile names none: name, then its ports.
pub const DEFAULT_SERVICES: [(&str, &[&str]); 15] = [
    ("rdp", &["3389/tcp", "3389/udp"]),
    ("smb", &["445/tcp", "139/tcp"]),
    ("netbios", &["137/udp", "138/udp"]),
    ("rpc", &["135/tcp"]),
    ("winrm", &["5985/tcp", "5986/tcp"]),
    ("ssh", &["22/tcp"]),
    ("telnet", &["23/tcp"]),
    ("vnc", &["5900/tcp"]),
    ("mssql", &["1433/tcp"]),
    ("mysql", &["3306/tcp"]),
    ("postgres", &["5432/tcp"]),
    ("oracle", &["1521/tcp"]),
    ("mongodb", &["27017/tcp"]),
    ("redis", &["6379/tcp"]),
    ("ldap", &["389/tcp", "636/tcp"]),
];

/// One risky service: its name and its ports (`3389/tcp`, a range
/// `8000-8100/tcp` too), each with a protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskyService {
    pub name: String,
    pub ports: Vec<ServiceMatch>,
}

/// `3389/tcp`, `8000-8100/tcp`.
fn port_text(m: &ServiceMatch) -> String {
    let proto = m
        .proto
        .map(|p| proto_name(i64::from(p)))
        .unwrap_or_default();
    match (m.port, m.to_port) {
        (Some(p), Some(to)) => format!("{p}-{to}/{proto}"),
        (Some(p), None) => format!("{p}/{proto}"),
        (None, _) => proto,
    }
}

impl RiskyService {
    /// The service from its name and port texts, each `PORT/PROTO` or
    /// `FROM-TO/PROTO`: a port alone or a protocol alone is refused (a
    /// risky service is a port of one protocol).
    pub fn parse(name: &str, ports: &[String]) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() || name.contains([',', ' ']) {
            return Err(format!(
                "{name:?}: a service name is one word (rdp, ssh, mssql)"
            ));
        }
        if ports.is_empty() {
            return Err(format!("{name}: no ports (e.g. [\"3389/tcp\"])"));
        }
        let ports = ports
            .iter()
            .map(|p| {
                let m = parse_service(p).map_err(|e| format!("{name}: {p:?}: {e}"))?;
                if m.port.is_none() || m.proto.is_none() {
                    return Err(format!(
                        "{name}: {p:?}: give a port and a protocol, e.g. 3389/tcp"
                    ));
                }
                Ok(m)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            name: name.to_string(),
            ports,
        })
    }

    /// Whether `f`'s port and protocol are one of the service's.
    pub fn matches(&self, f: &ResolvedFlow) -> bool {
        let (Some(port), Some(proto)) = (f.port, proto_number(&f.proto)) else {
            return false;
        };
        self.ports.iter().any(|m| {
            m.proto == Some(proto)
                && m.port.is_some_and(|from| {
                    let to = m.to_port.unwrap_or(from);
                    (from..=to).contains(&port)
                })
        })
    }

    /// The ports as the filter and the config write them: `3389/tcp`.
    pub fn port_texts(&self) -> Vec<String> {
        self.ports.iter().map(port_text).collect()
    }
}

/// The list when a profile names none.
pub fn default_services() -> Vec<RiskyService> {
    DEFAULT_SERVICES
        .iter()
        .map(|(name, ports)| RiskyService {
            name: (*name).to_string(),
            ports: ports
                .iter()
                .map(|p| parse_service(p).expect("a default port parses"))
                .collect(),
        })
        .collect()
}

/// The profile's list: `services` in place of the defaults, then `extra`
/// added (an `extra` name already listed replaces that entry). Every
/// entry is checked; the first bad one is the error, named by its table
/// (`exposure.services.rdp: "3389/xyz": unknown protocol "xyz"`).
pub fn service_list(
    services: Option<&BTreeMap<String, Vec<String>>>,
    extra: Option<&BTreeMap<String, Vec<String>>>,
) -> Result<Vec<RiskyService>, String> {
    let parse = |table: &str, entries: &BTreeMap<String, Vec<String>>| {
        entries
            .iter()
            .map(|(name, ports)| {
                RiskyService::parse(name, ports).map_err(|e| format!("exposure.{table}.{e}"))
            })
            .collect::<Result<Vec<_>, _>>()
    };
    let mut list = match services {
        Some(s) => {
            let list = parse("services", s)?;
            if list.is_empty() {
                return Err(
                    "exposure.services: names no service (leave it out for the defaults)".into(),
                );
            }
            list
        }
        None => default_services(),
    };
    if let Some(x) = extra {
        for s in parse("extra", x)? {
            match list.iter_mut().find(|l| l.name == s.name) {
                Some(l) => *l = s,
                None => list.push(s),
            }
        }
    }
    Ok(list)
}

/// `list` narrowed to `names` (`--service rdp,ssh`), in the list's
/// order; an unknown name is an error naming the known ones.
pub fn select(list: &[RiskyService], names: &[String]) -> Result<Vec<RiskyService>, String> {
    let names: Vec<&str> = names
        .iter()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() {
        return Ok(list.to_vec());
    }
    if let Some(bad) = names.iter().find(|n| !list.iter().any(|s| s.name == **n)) {
        return Err(format!(
            "unknown risky service {bad:?}; the list has: {}",
            list.iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(list
        .iter()
        .filter(|s| names.contains(&s.name.as_str()))
        .cloned()
        .collect())
}

/// Every port of the list, once, in order: the query's services filter.
pub fn service_terms(list: &[RiskyService]) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for t in list.iter().flat_map(RiskyService::port_texts) {
        if !terms.contains(&t) {
            terms.push(t);
        }
    }
    terms
}

/// The first service of the list `f` uses; `None` for any other flow.
pub fn service_of<'a>(list: &'a [RiskyService], f: &ResolvedFlow) -> Option<&'a RiskyService> {
    list.iter().find(|s| s.matches(f))
}

/// Where a row stands, judged on the flows from outside its group:
/// allowed (`exposed`), else potentially blocked, else blocked; `intra`
/// when only its own group's flows reach it. The order rows sort in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Exposed,
    PotentiallyBlocked,
    Blocked,
    Intra,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Exposed => "exposed",
            Status::PotentiallyBlocked => "potentially_blocked",
            Status::Blocked => "blocked",
            Status::Intra => "intra",
        }
    }

    /// The table cell: `pot.blocked` for the long one.
    pub fn short(self) -> &'static str {
        match self {
            Status::PotentiallyBlocked => "pot.blocked",
            s => s.as_str(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "exposed" => Some(Status::Exposed),
            "potentially_blocked" | "potentially-blocked" | "pb" => {
                Some(Status::PotentiallyBlocked)
            }
            "blocked" => Some(Status::Blocked),
            "intra" => Some(Status::Intra),
            _ => None,
        }
    }
}

/// Where a row's flows come from: its own group (`intra`, flow records),
/// other labelled groups (`apps`, how many), unlabelled hosts and IP
/// lists (`ip_lists`, how many), with an `Any (0.0.0.0/0 …)` IP list
/// (`any`) or a public address (`internet`) among them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Sources {
    pub intra: u64,
    pub apps: u64,
    pub ip_lists: u64,
    pub any: bool,
    pub internet: bool,
}

impl Sources {
    /// `intra, 2 apps, 1 ext (any)`, the table's sources cell.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.intra > 0 {
            parts.push("intra".to_string());
        }
        if self.apps > 0 {
            parts.push(format!(
                "{} app{}",
                self.apps,
                if self.apps == 1 { "" } else { "s" }
            ));
        }
        if self.ip_lists > 0 {
            let flag = match (self.any, self.internet) {
                (true, _) => " (any)",
                (false, true) => " (internet)",
                _ => "",
            };
            parts.push(format!("{} ext{flag}", self.ip_lists));
        }
        parts.join(", ")
    }
}

/// One destination group and risky service.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExposureRow {
    /// The risky service's name, `rdp`.
    pub service: String,
    /// The service's ports seen in the flows, `3389/tcp`.
    pub ports: Vec<String>,
    /// The destination group (the group keys' values).
    pub dst: Vec<String>,
    pub status: Status,
    pub sources: Sources,
    /// Flow records, and their lifetime connection counts.
    pub flows: u64,
    pub connections: u64,
    /// Flow records per decision.
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
    /// Allowed flow records from outside the group: what makes it exposed.
    pub exposed_flows: u64,
    #[serde(flatten)]
    pub conns: ConnsByDecision,
    /// With draft decisions loaded: what the draft policy would change.
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
    /// The flow to explain: the exposing one with the most connections,
    /// else the row's busiest.
    #[serde(skip)]
    pub sample: ResolvedFlow,
}

impl ExposureRow {
    pub fn dst_text(&self) -> String {
        self.dst.join("/")
    }
}

/// An `Any (0.0.0.0/0 and ::/0)` IP list, as the PCE names its default one.
fn is_any_list(name: &str) -> bool {
    name.starts_with("Any (0.0.0.0/0")
}

/// A public address: neither private, loopback, link-local, shared
/// (100.64/10) nor unique local (fc00::/7).
fn is_public(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(a)) => {
            let [o1, o2, ..] = a.octets();
            !(a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || a.is_unspecified()
                || a.is_broadcast()
                || a.is_multicast()
                || (o1 == 100 && (64..128).contains(&o2)))
        }
        Ok(std::net::IpAddr::V6(a)) => {
            let first = a.segments()[0];
            !(a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
        Err(_) => false,
    }
}

/// A source outside the group that is no labelled workload.
fn external(ep: &ResolvedEndpoint) -> bool {
    ep.labels.is_empty() || !ep.is_workload()
}

/// Whether `f` stays inside its destination's group: both ends labelled
/// workloads whose group values are equal and set for every key. Anything
/// else (an IP list, an unlabelled or unmanaged host, a workload missing a
/// group key) is from outside: a report of exposure may over-report, never
/// hide one.
pub fn inside_group(f: &ResolvedFlow, keys: &[String]) -> bool {
    if external(&f.src) || external(&f.dst) {
        return false;
    }
    let dst = group_values(&f.dst, keys);
    !dst.iter().any(|v| v == "-") && group_values(&f.src, keys) == dst
}

#[derive(Default)]
struct Acc {
    ports: BTreeSet<(u16, String)>,
    apps: BTreeSet<Vec<String>>,
    ext: BTreeSet<Vec<String>>,
    sources: Sources,
    flows: u64,
    connections: u64,
    allowed: u64,
    potentially_blocked: u64,
    blocked: u64,
    exposed_flows: u64,
    /// Potentially blocked and blocked flow records from outside the group:
    /// what the statuses after `exposed` are judged on.
    outside_pb: u64,
    outside_blocked: u64,
    conns: ConnsByDecision,
    draft: Option<DraftCounts>,
    /// (exposing, connections) of the sample so far.
    sample: Option<(bool, u64, ResolvedFlow)>,
}

/// One row per destination group (by `keys`) and risky service of `list`
/// the flows reach; flows of no listed service are left out. Sorted:
/// exposed, potentially blocked, blocked, intra; then the list's order,
/// the most connections and the destination.
pub fn exposure_rows(
    flows: &[ResolvedFlow],
    list: &[RiskyService],
    keys: &[String],
) -> Vec<ExposureRow> {
    let mut acc: BTreeMap<(usize, Vec<String>), Acc> = BTreeMap::new();
    for f in flows {
        let Some(at) = list.iter().position(|s| s.matches(f)) else {
            continue;
        };
        let dst = group_key(&f.dst, keys);
        let src = group_key(&f.src, keys);
        let a = acc.entry((at, dst.clone())).or_default();
        if let Some(p) = f.port {
            a.ports.insert((p, f.proto.clone()));
        }
        let inside = inside_group(f, keys);
        if inside {
            a.sources.intra += 1;
        } else if external(&f.src) {
            a.ext.insert(src);
            a.sources.any |= f.src.ip_lists.iter().any(|l| is_any_list(l));
            a.sources.internet |= f.src.ip.as_deref().is_some_and(is_public);
        } else {
            a.apps.insert(src);
        }
        a.flows += 1;
        a.connections += f.num_connections;
        a.conns.add(f.policy_decision, f.num_connections);
        match f.policy_decision {
            PolicyDecision::Allowed => a.allowed += 1,
            PolicyDecision::PotentiallyBlocked => a.potentially_blocked += 1,
            PolicyDecision::Blocked => a.blocked += 1,
            PolicyDecision::Unknown => {}
        }
        let exposing = !inside && f.policy_decision == PolicyDecision::Allowed;
        a.exposed_flows += u64::from(exposing);
        if !inside {
            match f.policy_decision {
                PolicyDecision::PotentiallyBlocked => a.outside_pb += 1,
                PolicyDecision::Blocked => a.outside_blocked += 1,
                _ => {}
            }
        }
        DraftCounts::add(&mut a.draft, f);
        let better = a
            .sample
            .as_ref()
            .is_none_or(|(e, c, _)| (exposing, f.num_connections) > (*e, *c));
        if better {
            a.sample = Some((exposing, f.num_connections, f.clone()));
        }
    }
    let mut rows: Vec<(usize, ExposureRow)> = acc
        .into_iter()
        .map(|((at, dst), a)| {
            let status = if a.exposed_flows > 0 {
                Status::Exposed
            } else if a.outside_pb > 0 {
                Status::PotentiallyBlocked
            } else if a.outside_blocked > 0 {
                Status::Blocked
            } else {
                Status::Intra
            };
            let row = ExposureRow {
                service: list[at].name.clone(),
                ports: a
                    .ports
                    .iter()
                    .map(|(p, proto)| format!("{p}/{proto}"))
                    .collect(),
                dst,
                status,
                sources: Sources {
                    apps: a.apps.len() as u64,
                    ip_lists: a.ext.len() as u64,
                    ..a.sources
                },
                flows: a.flows,
                connections: a.connections,
                allowed: a.allowed,
                potentially_blocked: a.potentially_blocked,
                blocked: a.blocked,
                exposed_flows: a.exposed_flows,
                conns: a.conns,
                draft: a.draft,
                sample: a.sample.map(|(_, _, f)| f).expect("a row has a flow"),
            };
            (at, row)
        })
        .collect();
    rows.sort_by(|(ia, a), (ib, b)| {
        a.status
            .cmp(&b.status)
            .then(ia.cmp(ib))
            .then(b.connections.cmp(&a.connections))
            .then_with(|| a.dst.cmp(&b.dst))
    });
    rows.into_iter().map(|(_, r)| r).collect()
}

/// One risky service over every destination: how many it reaches and how
/// many of them are exposed, potentially blocked, blocked or intra.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ServiceSummary {
    pub service: String,
    pub destinations: u64,
    pub exposed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
    pub intra: u64,
}

impl ServiceSummary {
    /// `rdp: 12 destinations · 4 exposed (allowed from other apps) · 3
    /// potentially blocked · 5 blocked`; `· N intra only` when some are.
    pub fn text(&self) -> String {
        let mut s = format!(
            "{}: {} destination{} · {} exposed (allowed from other apps) · {} potentially blocked · {} blocked",
            self.service,
            self.destinations,
            if self.destinations == 1 { "" } else { "s" },
            self.exposed,
            self.potentially_blocked,
            self.blocked
        );
        if self.intra > 0 {
            s.push_str(&format!(" · {} intra only", self.intra));
        }
        s
    }

    /// `text` for a narrow line: `rdp: 12 dst · 4 exposed · 3 pot.bl · 5
    /// blocked`.
    pub fn short(&self) -> String {
        format!(
            "{}: {} dst · {} exposed · {} pot.bl · {} blocked",
            self.service, self.destinations, self.exposed, self.potentially_blocked, self.blocked
        )
    }
}

/// One summary per service of `list`, in its order, those no flow reached
/// included (0 destinations).
pub fn summarize(rows: &[ExposureRow], list: &[RiskyService]) -> Vec<ServiceSummary> {
    list.iter()
        .map(|s| {
            let mut sum = ServiceSummary {
                service: s.name.clone(),
                ..ServiceSummary::default()
            };
            for r in rows.iter().filter(|r| r.service == s.name) {
                sum.destinations += 1;
                match r.status {
                    Status::Exposed => sum.exposed += 1,
                    Status::PotentiallyBlocked => sum.potentially_blocked += 1,
                    Status::Blocked => sum.blocked += 1,
                    Status::Intra => sum.intra += 1,
                }
            }
            sum
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DraftDecision;
    use crate::resolve::ResolvedLabel;
    use crate::traffic::resolved::tests::rich_flows;

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn wl(app: &str, env: &str) -> ResolvedEndpoint {
        ResolvedEndpoint {
            hostname: Some(format!("{app}-{env}")),
            workload_href: Some(format!("/orgs/5/workloads/{app}-{env}")),
            labels: vec![
                ResolvedLabel {
                    href: format!("/orgs/5/labels/app-{app}"),
                    key: "app".into(),
                    value: app.into(),
                },
                ResolvedLabel {
                    href: format!("/orgs/5/labels/env-{env}"),
                    key: "env".into(),
                    value: env.into(),
                },
            ],
            ..ResolvedEndpoint::default()
        }
    }

    fn ip(addr: &str, lists: &[&str]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(addr.into()),
            ip_lists: lists.iter().map(|l| l.to_string()).collect(),
            ..ResolvedEndpoint::default()
        }
    }

    fn flow(
        src: ResolvedEndpoint,
        dst: ResolvedEndpoint,
        port: u16,
        proto: &str,
        d: PolicyDecision,
        conns: u64,
    ) -> ResolvedFlow {
        ResolvedFlow {
            src,
            dst,
            port: Some(port),
            proto: proto.into(),
            process_name: None,
            user_name: None,
            num_connections: conns,
            policy_decision: d,
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            side_rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        }
    }

    use PolicyDecision::{Allowed, Blocked, PotentiallyBlocked};

    #[test]
    fn the_default_list_is_the_issue_s() {
        let list = default_services();
        let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "rdp", "smb", "netbios", "rpc", "winrm", "ssh", "telnet", "vnc", "mssql", "mysql",
                "postgres", "oracle", "mongodb", "redis", "ldap"
            ]
        );
        assert_eq!(list[0].port_texts(), ["3389/tcp", "3389/udp"]);
        assert_eq!(list[14].port_texts(), ["389/tcp", "636/tcp"]);
        let terms = service_terms(&list);
        assert_eq!(terms.len(), 20);
        assert_eq!(terms[..3], ["3389/tcp", "3389/udp", "445/tcp"]);
    }

    #[test]
    fn a_profile_list_replaces_extra_adds_and_bad_entries_are_named() {
        let map = |e: &[(&str, &[&str])]| -> BTreeMap<String, Vec<String>> {
            e.iter()
                .map(|(n, p)| (n.to_string(), p.iter().map(|x| x.to_string()).collect()))
                .collect()
        };
        let own = map(&[("rdp", &["3389/tcp"]), ("app", &["8000-8100/tcp"])]);
        let list = service_list(Some(&own), None).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].port_texts(), ["8000-8100/tcp"]);
        let extra = map(&[("ssh", &["2222/tcp"]), ("nfs", &["2049/tcp"])]);
        let list = service_list(None, Some(&extra)).unwrap();
        assert_eq!(list.len(), 16, "nfs added");
        let ssh = list.iter().find(|s| s.name == "ssh").unwrap();
        assert_eq!(ssh.port_texts(), ["2222/tcp"], "an extra name replaces");
        for (entries, want) in [
            (
                map(&[("rdp", &["3389/xyz"])]),
                "exposure.services.rdp: \"3389/xyz\": unknown protocol \"xyz\"",
            ),
            (
                map(&[("rdp", &["3389"])]),
                "exposure.services.rdp: \"3389\": give a port and a protocol, e.g. 3389/tcp",
            ),
            (
                map(&[("rdp", &[])]),
                "exposure.services.rdp: no ports (e.g. [\"3389/tcp\"])",
            ),
        ] {
            assert_eq!(service_list(Some(&entries), None).unwrap_err(), want);
        }
        let err = service_list(None, Some(&map(&[("x y", &["1/tcp"])]))).unwrap_err();
        assert!(err.starts_with("exposure.extra.\"x y\""), "{err}");
        assert!(service_list(Some(&BTreeMap::new()), None).is_err());
    }

    #[test]
    fn select_narrows_in_list_order_and_names_the_known_ones() {
        let list = default_services();
        let got = select(&list, &["ssh".into(), "rdp".into()]).unwrap();
        assert_eq!(
            got.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["rdp", "ssh"]
        );
        let err = select(&list, &["rpd".into()]).unwrap_err();
        assert!(err.starts_with("unknown risky service \"rpd\"; the list has: rdp, smb"));
        assert_eq!(select(&list, &[]).unwrap().len(), 15);
    }

    #[test]
    fn a_service_matches_port_and_protocol_and_ranges() {
        let list = default_services();
        let f = |p, proto| flow(wl("a", "p"), wl("b", "p"), p, proto, Allowed, 1);
        assert_eq!(service_of(&list, &f(3389, "udp")).unwrap().name, "rdp");
        assert_eq!(service_of(&list, &f(139, "tcp")).unwrap().name, "smb");
        assert!(service_of(&list, &f(22, "udp")).is_none());
        assert!(service_of(&list, &f(443, "tcp")).is_none());
        let r = RiskyService::parse("app", &["8000-8100/tcp".into()]).unwrap();
        assert!(r.matches(&f(8050, "tcp")));
        assert!(!r.matches(&f(8101, "tcp")));
    }

    #[test]
    fn sources_split_into_intra_apps_and_ip_lists_with_any_and_internet() {
        let list = default_services();
        let db = || wl("shop", "prod");
        let flows = vec![
            flow(db(), db(), 22, "tcp", Allowed, 5),
            flow(wl("crm", "prod"), db(), 22, "tcp", Blocked, 2),
            flow(wl("hr", "prod"), db(), 22, "tcp", Blocked, 1),
            flow(wl("hr", "prod"), db(), 22, "tcp", Blocked, 1),
            flow(
                ip("10.0.0.9", &["Corp"]),
                db(),
                22,
                "tcp",
                PotentiallyBlocked,
                3,
            ),
            flow(
                ip("203.0.113.9", &["Any (0.0.0.0/0 and ::/0)"]),
                db(),
                22,
                "tcp",
                Blocked,
                1,
            ),
            // Not risky: left out.
            flow(wl("crm", "prod"), db(), 443, "tcp", Allowed, 9),
        ];
        let rows = exposure_rows(&flows, &list, &keys());
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(
            (r.service.as_str(), r.dst_text()),
            ("ssh", "shop/prod".into())
        );
        assert_eq!(
            r.sources,
            Sources {
                intra: 1,
                apps: 2,
                ip_lists: 2,
                any: true,
                internet: true,
            }
        );
        assert_eq!(r.sources.text(), "intra, 2 apps, 2 ext (any)");
        assert_eq!((r.flows, r.connections), (6, 13));
        assert_eq!((r.allowed, r.potentially_blocked, r.blocked), (1, 1, 4));
        assert_eq!(r.exposed_flows, 0, "the allowed flow is the group's own");
        assert_eq!(r.status, Status::PotentiallyBlocked);
        assert_eq!(r.ports, ["22/tcp"]);
        assert_eq!(r.conns.allowed_conns, 5);
    }

    #[test]
    fn rows_sort_exposed_then_potentially_blocked_then_blocked_then_intra() {
        let list = default_services();
        let flows = vec![
            flow(wl("a", "p"), wl("a", "p"), 3306, "tcp", Allowed, 50),
            flow(wl("x", "p"), wl("b", "p"), 22, "tcp", Blocked, 40),
            flow(
                wl("x", "p"),
                wl("c", "p"),
                22,
                "tcp",
                PotentiallyBlocked,
                30,
            ),
            flow(wl("x", "p"), wl("d", "p"), 3389, "tcp", Allowed, 1),
            flow(wl("x", "p"), wl("d", "p"), 3389, "udp", Blocked, 1),
            flow(wl("x", "p"), wl("e", "p"), 22, "tcp", Allowed, 7),
        ];
        let rows = exposure_rows(&flows, &list, &keys());
        let got: Vec<(String, String, &str)> = rows
            .iter()
            .map(|r| (r.service.clone(), r.dst_text(), r.status.as_str()))
            .collect();
        let want = [
            ("rdp", "d/p", "exposed"),
            ("ssh", "e/p", "exposed"),
            ("ssh", "c/p", "potentially_blocked"),
            ("ssh", "b/p", "blocked"),
            ("mysql", "a/p", "intra"),
        ];
        assert_eq!(got, want.map(|(a, b, c)| (a.to_string(), b.to_string(), c)));
        assert_eq!(rows[0].ports, ["3389/tcp", "3389/udp"]);
        assert_eq!(rows[0].exposed_flows, 1);
        assert_eq!(
            rows[0].sample.proto, "tcp",
            "the exposing flow is the sample"
        );
        let sums = summarize(&rows, &list);
        assert_eq!(sums.len(), 15);
        let ssh = sums.iter().find(|s| s.service == "ssh").unwrap();
        assert_eq!(
            ssh.text(),
            "ssh: 3 destinations · 1 exposed (allowed from other apps) · 1 potentially blocked · 1 blocked"
        );
        assert_eq!(ssh.short(), "ssh: 3 dst · 1 exposed · 1 pot.bl · 1 blocked");
        let mysql = sums.iter().find(|s| s.service == "mysql").unwrap();
        assert_eq!(
            mysql.text(),
            "mysql: 1 destination · 0 exposed (allowed from other apps) · 0 potentially blocked · 0 blocked · 1 intra only"
        );
    }

    #[test]
    fn draft_counts_follow_the_draft_decisions() {
        let list = default_services();
        let mut f = flow(wl("x", "p"), wl("b", "p"), 22, "tcp", Allowed, 1);
        f.draft_policy_decision = Some(DraftDecision::Blocked);
        let rows = exposure_rows(&[f], &list, &keys());
        assert_eq!(
            rows[0].draft,
            Some(DraftCounts {
                draft_allowed: 0,
                draft_blocked: 1
            })
        );
        let rows = exposure_rows(
            &[flow(wl("x", "p"), wl("b", "p"), 22, "tcp", Allowed, 1)],
            &list,
            &keys(),
        );
        assert_eq!(rows[0].draft, None);
    }

    #[test]
    fn the_rich_fixture_has_no_exposed_row() {
        let rows = exposure_rows(&rich_flows(), &default_services(), &keys());
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.status != Status::Exposed));
        assert!(rows.windows(2).all(|w| w[0].status <= w[1].status));
    }

    #[test]
    fn an_unlabelled_destination_is_never_its_sources_group() {
        let list = default_services();
        let any = "Any (0.0.0.0/0 and ::/0)";
        // A managed workload without labels, in the Any list, takes RDP
        // from the internet (also in the Any list).
        let bare = ResolvedEndpoint {
            ip: Some("10.0.2.30".into()),
            hostname: Some("new-host".into()),
            workload_href: Some("/orgs/5/workloads/new".into()),
            ip_lists: vec![any.into()],
            ..ResolvedEndpoint::default()
        };
        let rows = exposure_rows(
            &[flow(
                ip("203.0.113.7", &[any]),
                bare,
                3389,
                "tcp",
                Allowed,
                3,
            )],
            &list,
            &keys(),
        );
        assert_eq!(rows[0].status, Status::Exposed);
        assert_eq!(rows[0].sources.intra, 0);
        assert!(rows[0].sources.internet && rows[0].sources.any);
        // Corp into an unmanaged host in Corp: not intra.
        let rows = exposure_rows(
            &[flow(
                ip("10.1.4.9", &["Corp"]),
                ip("10.1.4.20", &["Corp"]),
                445,
                "tcp",
                Allowed,
                1,
            )],
            &list,
            &keys(),
        );
        assert_eq!(rows[0].status, Status::Exposed);
        assert_eq!(rows[0].sources.ip_lists, 1);
    }

    #[test]
    fn workloads_missing_a_group_key_are_no_group() {
        let list = default_services();
        let role = |r: &str, host: &str| ResolvedEndpoint {
            hostname: Some(host.into()),
            workload_href: Some(format!("/orgs/5/workloads/{host}")),
            labels: vec![ResolvedLabel {
                href: format!("/orgs/5/labels/role-{r}"),
                key: "role".into(),
                value: r.into(),
            }],
            ..ResolvedEndpoint::default()
        };
        let rows = exposure_rows(
            &[flow(
                role("web", "a"),
                role("db", "b"),
                445,
                "tcp",
                Allowed,
                1,
            )],
            &list,
            &keys(),
        );
        assert_eq!(rows[0].status, Status::Exposed, "-/- is no group");
        assert_eq!(rows[0].sources.apps, 1);
        // shop with no env into shop with no env: outside too.
        let shop = |host: &str| ResolvedEndpoint {
            labels: vec![ResolvedLabel {
                href: "/orgs/5/labels/app-shop".into(),
                key: "app".into(),
                value: "shop".into(),
            }],
            ..role("x", host)
        };
        let rows = exposure_rows(
            &[flow(shop("a"), shop("b"), 22, "tcp", Allowed, 1)],
            &list,
            &keys(),
        );
        assert_eq!(rows[0].status, Status::Exposed);
    }

    #[test]
    fn intra_flows_alone_never_set_a_blocked_status() {
        let list = default_services();
        let rows = exposure_rows(
            &[
                flow(wl("a", "p"), wl("a", "p"), 22, "tcp", PotentiallyBlocked, 1),
                flow(wl("a", "p"), wl("a", "p"), 22, "tcp", Blocked, 1),
            ],
            &list,
            &keys(),
        );
        assert_eq!(rows[0].status, Status::Intra);
        assert_eq!((rows[0].potentially_blocked, rows[0].blocked), (1, 1));
        let rows = exposure_rows(
            &[flow(wl("x", "p"), wl("a", "p"), 137, "udp", Blocked, 1)],
            &list,
            &keys(),
        );
        assert_eq!(
            (rows[0].service.as_str(), rows[0].status),
            ("netbios", Status::Blocked),
            "a udp-only service"
        );
    }

    #[test]
    fn public_addresses() {
        for (a, public) in [
            ("10.1.2.3", false),
            ("192.168.1.1", false),
            ("100.64.0.1", false),
            ("203.0.113.9", true),
            ("8.8.8.8", true),
            ("fd00::1", false),
            ("fe80::1", false),
            ("2001:db8::1", true),
            ("nonsense", false),
        ] {
            assert_eq!(is_public(a), public, "{a}");
        }
    }
}
