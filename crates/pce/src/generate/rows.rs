//! Flows to rows: one intra-app row, one row per app peer, existing IP
//! list (its IP peers grouped), IP peer in no list and unlabelled
//! workload, on each side, with their default checkbox and their flags.
//! Broadcast and multicast are left out first.
use super::services::{Expr, express, flow_spec, fold_ports};
use super::{DecisionCounts, GenRow, PeerKey, PeerKind, RowFlag, RowKey, Side};
use crate::iplist_size::ip_list_size;
use crate::models::IpList;
use crate::resolve::NameIndex;
use crate::traffic::{Focus, ResolvedEndpoint, ResolvedFlow, group_key};
use crate::write::PortSpec;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

/// IP peer rows kept per section; the rest fold into one row.
pub const MAX_IP_ROWS: usize = 25;
/// A rule with more service entries than this is flagged.
pub const MANY_SERVICES: usize = 32;
/// A list's IPs named before `+N` (`p`, the confirm).
pub const SEEN_IPS: usize = 3;
/// Destination ports from here up are flagged as possibly ephemeral.
pub const EPHEMERAL_FROM: u16 = 49152;

/// The PCE's built-in Any list (`0.0.0.0/0` and `::/0`).
const ANY_LIST: &str = "Any (0.0.0.0/0 and ::/0)";

/// A multicast (`224.0.0.0/4`, `ff00::/8`) or limited broadcast
/// (`255.255.255.255`) destination: never proposed.
pub fn excluded(f: &ResolvedFlow) -> bool {
    f.dst
        .ip
        .as_deref()
        .and_then(|s| s.parse::<IpAddr>().ok())
        .is_some_and(|ip| match ip {
            IpAddr::V4(v4) => v4.is_multicast() || v4.is_broadcast(),
            IpAddr::V6(v6) => v6.is_multicast(),
        })
}

/// The built-in Any list, by its external data set, else by its name.
pub fn is_any_ip_list(l: &IpList) -> bool {
    l.external_data_set.as_deref() == Some("com.illumio.ilo_defaults") || l.name == ANY_LIST
}

/// The IP list the batch creates for an IP peer: `shop-prod peer 10.4.1.9`.
pub fn ip_list_name(app: &[String], ip: &str) -> String {
    format!("{} peer {ip}", app.join("-"))
}

/// `ips` by address; one that does not parse first, by its text.
pub fn by_address<'a>(ips: impl IntoIterator<Item = &'a String>) -> Vec<&'a String> {
    let mut v: Vec<&String> = ips.into_iter().collect();
    v.sort_by_key(|s| (s.parse::<IpAddr>().ok(), (*s).clone()));
    v
}

/// `ipl:Corp: 7 IPs seen: 10.0.0.1, 10.0.0.2, 10.0.0.3 +4`: the IP peers
/// a rule names an existing list for, the first [`SEEN_IPS`] by address.
pub fn seen_text(list: &str, ips: &BTreeSet<String>) -> String {
    let all = by_address(ips);
    let mut shown: Vec<&str> = all.iter().take(SEEN_IPS).map(|s| s.as_str()).collect();
    let more = format!("+{}", all.len().saturating_sub(SEEN_IPS));
    if all.len() > SEEN_IPS {
        shown.push(&more);
    }
    format!(
        "ipl:{list}: {} {} seen: {}",
        all.len(),
        if all.len() == 1 { "IP" } else { "IPs" },
        shown.join(", ")
    )
}

/// `in` when the destination is the app (a self-flow too), `out` when
/// only the source is; `None` for a flow that is not the app's.
pub(crate) fn side_of(f: &ResolvedFlow, focus: &Focus) -> Option<Side> {
    if group_key(&f.dst, &focus.keys) == focus.app {
        Some(Side::Inbound)
    } else if group_key(&f.src, &focus.keys) == focus.app {
        Some(Side::Outbound)
    } else {
        None
    }
}

/// The endpoint's value under every key, or `None` when one is missing.
pub(crate) fn scope_values(ep: &ResolvedEndpoint, keys: &[String]) -> Option<Vec<String>> {
    if !ep.is_workload() {
        return None;
    }
    keys.iter()
        .map(|k| ep.label_value(k).map(str::to_string))
        .collect()
}

/// The row a flow belongs to; `None` when excluded or not the app's.
pub(crate) fn row_key(f: &ResolvedFlow, focus: &Focus, idx: &NameIndex) -> Option<RowKey> {
    classify(f, focus, idx).map(|(k, _)| k)
}

/// The row a flow belongs to and, for an IP peer in an existing list,
/// which list and why.
fn classify(f: &ResolvedFlow, focus: &Focus, idx: &NameIndex) -> Option<(RowKey, Option<Pick>)> {
    if excluded(f) {
        return None;
    }
    let side = side_of(f, focus)?;
    let other = match side {
        Side::Inbound if group_key(&f.src, &focus.keys) == focus.app => {
            let key = RowKey {
                side,
                peer: PeerKey::Intra,
            };
            return Some((key, None));
        }
        Side::Inbound => &f.src,
        Side::Outbound => &f.dst,
    };
    let (peer, pick) = if other.is_workload() {
        let peer = match scope_values(other, &focus.keys) {
            Some(v) => PeerKey::App(v),
            None => PeerKey::Workload(other.workload_href.clone().unwrap_or_default()),
        };
        (peer, None)
    } else {
        match pick_list(other, &focus.app, idx) {
            Some(p) => (PeerKey::IpList(p.href.clone()), Some(p)),
            None => (
                PeerKey::Ip(other.ip.clone().unwrap_or_else(|| "?".into())),
                None,
            ),
        }
    };
    Some((RowKey { side, peer }, pick))
}

/// Whether role granularity misses a role on a side it uses.
fn lacks_role(f: &ResolvedFlow, key: &RowKey, tier_key: &str) -> bool {
    let role = |e: &ResolvedEndpoint| e.label_value(tier_key).is_none();
    match (&key.side, &key.peer) {
        (Side::Inbound, PeerKey::Intra | PeerKey::App(_)) => role(&f.src) || role(&f.dst),
        (Side::Inbound, _) => role(&f.dst),
        (Side::Outbound, PeerKey::App(_)) => role(&f.src) || role(&f.dst),
        (Side::Outbound, _) => role(&f.src),
    }
}

/// The existing IP list an IP peer's rule names, and the others it is in.
struct Pick {
    href: String,
    name: String,
    /// The other lists the PCE reported for it (not the Any list).
    others: Vec<String>,
    /// Picked by name: the narrowest size was a tie or unknown.
    by_name: bool,
}

/// A list's addresses for "narrowest": unknown (no ranges, FQDNs only)
/// and unbounded sort last.
fn narrowness(l: &IpList) -> u64 {
    let size = ip_list_size(l);
    if size.ranges == 0 {
        u64::MAX
    } else {
        size.sort_key()
    }
}

/// Of the IP lists the PCE reported for the endpoint that the index knows,
/// the built-in Any list aside: the narrowest (fewest addresses), the first
/// by name among equals. Else an existing list named as the batch would
/// name it. `None` when the batch creates one.
fn pick_list(ep: &ResolvedEndpoint, app: &[String], idx: &NameIndex) -> Option<Pick> {
    let mut found: Vec<(u64, String, String)> = ep
        .ip_lists
        .iter()
        .filter_map(|n| {
            let href = idx.ip_list_href(n)?;
            let l = idx.ip_list(href).filter(|l| !is_any_ip_list(l))?;
            Some((narrowness(l), l.name.clone(), href.to_string()))
        })
        .collect();
    found.sort_by_key(|(size, name, _)| (*size, name.to_lowercase()));
    found.dedup_by(|a, b| a.2 == b.2);
    if found.is_empty() {
        let name = ip_list_name(app, ep.ip.as_deref()?);
        let href = idx.ip_list_href(&name)?.to_string();
        return Some(Pick {
            href,
            name,
            others: Vec::new(),
            by_name: false,
        });
    }
    let by_name = found.len() > 1 && found[0].0 == found[1].0;
    let (_, name, href) = found.remove(0);
    Some(Pick {
        href,
        name,
        others: found.into_iter().map(|(_, n, _)| n).collect(),
        by_name,
    })
}

#[derive(Default)]
struct Acc {
    flows: u64,
    connections: u64,
    decisions: DecisionCounts,
    services: BTreeSet<PortSpec>,
    no_role: u64,
    ephemeral: bool,
    /// The workload's name, the IP list's: from the first flow.
    name: String,
    /// A list row's IPs, each with the other lists it is in and whether
    /// the list was picked by name.
    ips: BTreeMap<String, (Vec<String>, bool)>,
}

/// The rows of `flows` for the app `focus` names (its keys must be the
/// scope keys; the caller checks). Inbound first, then outbound; in each:
/// intra-app, app peers (by flows), unlabelled workloads, IP lists (by
/// connections), IP peers in no list (by connections, at most
/// [`MAX_IP_ROWS`], the rest folded). Every row's checkbox at its default.
pub fn rows(flows: &[ResolvedFlow], focus: &Focus, tier_key: &str, idx: &NameIndex) -> Vec<GenRow> {
    let mut acc: BTreeMap<RowKey, Acc> = BTreeMap::new();
    for f in flows {
        let Some((key, pick)) = classify(f, focus, idx) else {
            continue;
        };
        let a = acc.entry(key.clone()).or_default();
        let other = match key.side {
            Side::Inbound => &f.src,
            Side::Outbound => &f.dst,
        };
        if a.flows == 0 {
            a.name = other.display_name();
        }
        if let Some(p) = pick {
            a.name = p.name;
            let ip = other.ip.clone().unwrap_or_else(|| "?".into());
            a.ips.entry(ip).or_insert((p.others, p.by_name));
        }
        a.flows += 1;
        a.connections += f.num_connections;
        a.decisions.add(f);
        let spec = flow_spec(f);
        a.ephemeral |= spec.port.is_some_and(|p| p >= EPHEMERAL_FROM);
        a.services.insert(spec);
        a.no_role += u64::from(lacks_role(f, &key, tier_key));
    }
    let mut out = Vec::new();
    for side in [Side::Inbound, Side::Outbound] {
        let mut rows: Vec<GenRow> = acc
            .iter()
            .filter(|(k, _)| k.side == side)
            .map(|(k, a)| row(k, a, focus, idx))
            .collect();
        rows.sort_by_key(order);
        let ips = rows
            .iter()
            .filter(|r| matches!(r.peer, PeerKind::Ip { .. }))
            .count();
        if ips > MAX_IP_ROWS {
            let first_ip = rows
                .iter()
                .position(|r| matches!(r.peer, PeerKind::Ip { .. }))
                .unwrap_or(rows.len());
            let rest: Vec<GenRow> = rows.split_off(first_ip + MAX_IP_ROWS);
            rows.push(fold(side, &rest));
        }
        out.extend(rows);
    }
    out
}

/// Intra-app, app peers, workloads, IP lists, IP peers, folded; then by
/// size.
fn order(r: &GenRow) -> (u8, std::cmp::Reverse<u64>, String) {
    match &r.peer {
        PeerKind::Intra => (0, std::cmp::Reverse(0), String::new()),
        PeerKind::App(v) => (1, std::cmp::Reverse(r.flows), v.join("/")),
        PeerKind::Workload { href, .. } => (2, std::cmp::Reverse(r.flows), href.clone()),
        PeerKind::IpList { name, .. } => (3, std::cmp::Reverse(r.connections), name.clone()),
        PeerKind::Ip { ip } => (4, std::cmp::Reverse(r.connections), ip.clone()),
        PeerKind::FoldedIps(_) => (5, std::cmp::Reverse(0), String::new()),
    }
}

fn row(key: &RowKey, a: &Acc, focus: &Focus, idx: &NameIndex) -> GenRow {
    let mut flags = Vec::new();
    let (peer, mut checked, mut checkable) = match &key.peer {
        PeerKey::Intra => (PeerKind::Intra, true, true),
        PeerKey::App(v) => {
            let same_env = v.get(1..) == focus.app.get(1..);
            if !same_env {
                flags.push(RowFlag::EnvMismatch);
            }
            (PeerKind::App(v.clone()), same_env, true)
        }
        PeerKey::Ip(ip) => {
            let addressable = ip.parse::<IpAddr>().is_ok();
            flags.push(if addressable {
                RowFlag::NeedsIpList
            } else {
                RowFlag::NoAddress
            });
            (PeerKind::Ip { ip: ip.clone() }, false, addressable)
        }
        PeerKey::IpList(href) => {
            let multi: Vec<&(Vec<String>, bool)> =
                a.ips.values().filter(|(o, _)| !o.is_empty()).collect();
            if !multi.is_empty() {
                let others: BTreeSet<&String> = multi.iter().flat_map(|(o, _)| o).collect();
                flags.push(RowFlag::AlsoIn {
                    ips: multi.len(),
                    others: others.into_iter().cloned().collect(),
                    by_name: multi.iter().any(|(_, b)| *b),
                });
            }
            let peer = PeerKind::IpList {
                href: href.clone(),
                name: a.name.clone(),
                ips: by_address(a.ips.keys()).into_iter().cloned().collect(),
            };
            (peer, false, true)
        }
        PeerKey::Workload(href) => {
            flags.push(RowFlag::NoScopeLabels);
            let peer = PeerKind::Workload {
                href: href.clone(),
                name: a.name.clone(),
            };
            (peer, false, key.side == Side::Inbound)
        }
        PeerKey::Folded => (PeerKind::FoldedIps(0), false, false),
    };
    let mut inline = BTreeSet::new();
    let mut objects = BTreeSet::new();
    let mut unsupported = BTreeSet::new();
    for s in &a.services {
        match express(s, idx) {
            Expr::Inline(p) => {
                inline.insert(p);
            }
            Expr::Object(h) => {
                objects.insert(h);
            }
            Expr::Unsupported(p) => {
                unsupported.insert(p);
            }
        }
    }
    flags.extend(unsupported.iter().map(|p| RowFlag::UnsupportedProto(*p)));
    if inline.is_empty() && objects.is_empty() {
        checked = false;
        checkable = false;
    }
    let entries = fold_ports(&inline).len() + objects.len();
    if entries > MANY_SERVICES {
        flags.push(RowFlag::ManyServices(entries));
    }
    if a.ephemeral {
        flags.push(RowFlag::Ephemeral);
    }
    if a.no_role > 0 {
        flags.push(RowFlag::NoRole { flows: a.no_role });
    }
    GenRow {
        side: key.side,
        peer,
        checked,
        checkable,
        stop: None,
        flags,
        flows: a.flows,
        connections: a.connections,
        decisions: a.decisions,
        services: a.services.clone(),
    }
}

/// The IP rows past the limit as one read-only row.
fn fold(side: Side, rest: &[GenRow]) -> GenRow {
    let mut decisions = DecisionCounts::default();
    for r in rest {
        decisions.merge(&r.decisions);
    }
    GenRow {
        side,
        peer: PeerKind::FoldedIps(rest.len()),
        checked: false,
        checkable: false,
        stop: None,
        flags: Vec::new(),
        flows: rest.iter().map(|r| r.flows).sum(),
        connections: rest.iter().map(|r| r.connections).sum(),
        decisions,
        services: rest
            .iter()
            .flat_map(|r| r.services.iter().cloned())
            .collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::traffic::parse_app;
    use crate::traffic::resolved::tests::{rich_flows, rich_index};

    pub(crate) fn shop() -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
    }

    pub(crate) fn texts(rows: &[GenRow]) -> Vec<(Side, String, bool, bool)> {
        rows.iter()
            .map(|r| (r.side, r.peer_text(), r.checked, r.checkable))
            .collect()
    }

    /// Flow `i` of the rich fixture with its destination IP replaced (and
    /// no workload there).
    pub(crate) fn to_ip(i: usize, ip: &str) -> ResolvedFlow {
        let mut f = rich_flows()[i].clone();
        f.dst = crate::traffic::ResolvedEndpoint {
            ip: Some(ip.into()),
            ..Default::default()
        };
        f
    }

    /// Flow 0 (shop web to shop db, intra-app) on another port.
    pub(crate) fn intra_on(port: u16) -> ResolvedFlow {
        let mut f = rich_flows()[0].clone();
        f.port = Some(port);
        f
    }

    #[test]
    fn shop_prod_has_four_inbound_and_two_outbound_rows_at_their_defaults() {
        let r = rows(&rich_flows(), &shop(), "role", &rich_index());
        use Side::*;
        assert_eq!(
            texts(&r),
            vec![
                (Inbound, "intra-app".into(), true, true),
                (Inbound, "hr/prod".into(), true, true),
                (Inbound, "shop/dev".into(), false, true),
                (Inbound, "IP list Corp · 1 IP".into(), false, true),
                (Outbound, "crm/prod".into(), true, true),
                (Outbound, "203.0.113.7".into(), false, true),
            ]
        );
        assert_eq!(
            (r[0].flows, r[0].connections),
            (4, 821),
            "flows 0, 1, 26, 29"
        );
        assert_eq!(r[0].decisions.allowed, 4);
        assert_eq!(
            r[1].decisions.blocked, 1,
            "potentially blocked counts as blocked"
        );
        assert_eq!(r[2].flags, vec![RowFlag::EnvMismatch]);
        assert_eq!(
            r[2].flag_texts(super::super::Stop::AppPorts, &shop().keys),
            vec!["env mismatch"]
        );
        assert_eq!(r[3].flags, Vec::new(), "the row names its list");
        assert_eq!(
            r[3].services
                .iter()
                .map(PortSpec::display)
                .collect::<Vec<_>>(),
            vec!["443/tcp", "5432/tcp"]
        );
        assert_eq!(r[5].flags, vec![RowFlag::NeedsIpList]);
        assert_eq!(
            r[3].peer,
            PeerKind::IpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                name: "Corp".into(),
                ips: vec!["10.1.4.9".into()],
            }
        );
        assert_eq!(r[3].kind_text(), "ip_list");
    }

    /// The fixture's lists plus `rfc1918` (10/8) and `Labs` (10.1.4.0/24).
    fn lists_index() -> NameIndex {
        let mut lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let extra: Vec<IpList> = serde_json::from_value(serde_json::json!([
            {"href": "/orgs/5/sec_policy/draft/ip_lists/30", "name": "rfc1918",
             "ip_ranges": [{"from_ip": "10.0.0.0/8"}]},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/31", "name": "Labs",
             "ip_ranges": [{"from_ip": "10.1.4.0/24"}]},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/32", "name": "Lab twin",
             "ip_ranges": [{"from_ip": "10.1.4.0", "to_ip": "10.1.4.255"}]}
        ]))
        .unwrap();
        lists.extend(extra);
        let mut labels: Vec<crate::models::Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let rich: Vec<crate::models::Label> = serde_json::from_str(include_str!(
            "../../../../fixtures/traffic_labels_rich.json"
        ))
        .unwrap();
        labels.extend(rich);
        let services: Vec<crate::models::Service> =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &[], &lists, &services)
    }

    /// Flow 17 (Corp 10.1.4.9 into shop web) from `ip`, in `lists`.
    fn from_ip(ip: &str, lists: &[&str], connections: u64) -> ResolvedFlow {
        let mut f = rich_flows()[17].clone();
        f.src.ip = Some(ip.into());
        f.src.ip_lists = lists.iter().map(|s| s.to_string()).collect();
        f.num_connections = connections;
        f
    }

    #[test]
    fn ip_peers_group_into_one_row_per_list_with_the_union_of_their_services() {
        let mut a = from_ip("10.1.4.10", &["Corp"], 5);
        a.port = Some(8443);
        let f = vec![
            from_ip("10.1.4.9", &["Corp"], 5),
            a,
            from_ip("10.1.4.10", &["Corp"], 5),
            from_ip("192.0.2.1", &["Any (0.0.0.0/0 and ::/0)"], 1),
        ];
        let r = rows(&f, &shop(), "role", &rich_index());
        assert_eq!(
            texts(&r),
            vec![
                (Side::Inbound, "IP list Corp · 2 IPs".into(), false, true),
                (Side::Inbound, "192.0.2.1".into(), false, true),
            ],
            "an IP only in Any is in no list"
        );
        assert_eq!(
            r[0].services
                .iter()
                .map(PortSpec::display)
                .collect::<Vec<_>>(),
            vec!["443/tcp", "8443/tcp"]
        );
        assert_eq!((r[0].flows, r[0].connections), (3, 15));
        assert_eq!(r[1].flags, vec![RowFlag::NeedsIpList]);
        assert_eq!(
            r[0].key(),
            RowKey {
                side: Side::Inbound,
                peer: PeerKey::IpList("/orgs/5/sec_policy/draft/ip_lists/21".into())
            }
        );
    }

    #[test]
    fn an_ip_in_several_lists_goes_to_the_narrowest_and_the_row_says_so() {
        let idx = lists_index();
        let f = vec![
            // Labs (256) is narrower than Corp (65536) and rfc1918.
            from_ip("10.1.4.9", &["rfc1918", "Corp", "Labs"], 1),
            // Only in rfc1918.
            from_ip("10.200.0.1", &["rfc1918"], 1),
        ];
        let r = rows(&f, &shop(), "role", &idx);
        assert_eq!(
            texts(&r),
            vec![
                (Side::Inbound, "IP list Labs · 1 IP".into(), false, true),
                (Side::Inbound, "IP list rfc1918 · 1 IP".into(), false, true),
            ]
        );
        assert_eq!(
            r[0].flag_texts(super::super::Stop::AppPorts, &[]),
            vec!["1 IP also in Corp, rfc1918: narrowest list"]
        );
        assert!(r[1].flags.is_empty());
        // Labs and its twin hold as many: the first by name.
        let tie = rows(
            &[from_ip("10.1.4.9", &["Labs", "Lab twin"], 1)],
            &shop(),
            "role",
            &idx,
        );
        assert_eq!(tie[0].peer_text(), "IP list Lab twin · 1 IP");
        assert_eq!(
            tie[0].flag_texts(super::super::Stop::AppPorts, &[]),
            vec!["1 IP also in Labs: first by name"]
        );
    }

    #[test]
    fn an_existing_list_named_as_the_batch_would_name_it_is_a_list_row() {
        let mut lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        lists.push(
            serde_json::from_value(serde_json::json!({
                "href": "/orgs/5/sec_policy/draft/ip_lists/40",
                "name": "shop-prod peer 192.0.2.1",
                "ip_ranges": [{"from_ip": "192.0.2.1"}]
            }))
            .unwrap(),
        );
        let idx = NameIndex::new(&[], &[], &lists, &[]);
        let r = rows(&[from_ip("192.0.2.1", &[], 1)], &shop(), "role", &idx);
        assert_eq!(r[0].peer_text(), "IP list shop-prod peer 192.0.2.1 · 1 IP");
        assert!(r[0].flags.is_empty() && !r[0].checked && r[0].checkable);
    }

    #[test]
    fn seen_ips_name_the_first_few_by_address_and_count_the_rest() {
        let ips: BTreeSet<String> = ["10.0.0.10", "10.0.0.9", "10.0.0.2", "10.0.0.1", "::1"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            seen_text("rfc1918", &ips),
            "ipl:rfc1918: 5 IPs seen: 10.0.0.1, 10.0.0.2, 10.0.0.9, +2"
        );
        let one: BTreeSet<String> = ["10.1.4.9".to_string()].into();
        assert_eq!(seen_text("Corp", &one), "ipl:Corp: 1 IP seen: 10.1.4.9");
    }

    #[test]
    fn broadcast_and_multicast_never_make_a_row() {
        let mut f = rich_flows();
        f.push(to_ip(22, "224.0.0.251"));
        f.push(to_ip(22, "255.255.255.255"));
        f.push(to_ip(22, "ff02::fb"));
        assert!(excluded(&f[30]) && excluded(&f[31]) && excluded(&f[32]));
        let r = rows(&f, &shop(), "role", &rich_index());
        assert_eq!(r.len(), 6, "no row for the three");
        assert!(
            !excluded(&to_ip(22, "10.255.255.255")),
            "a subnet broadcast is an IP peer"
        );
    }

    #[test]
    fn an_unlabelled_workload_is_its_own_row_checkable_inbound_only() {
        let mut f = rich_flows();
        let mut inb = f[12].clone(); // hr/prod web -> shop db
        inb.src.labels.retain(|l| l.key != "env");
        let mut out = f[2].clone(); // shop web -> crm web
        out.dst.labels.clear();
        f.extend([inb, out]);
        let r = rows(&f, &shop(), "role", &rich_index());
        let wl: Vec<&GenRow> = r
            .iter()
            .filter(|r| matches!(r.peer, PeerKind::Workload { .. }))
            .collect();
        assert_eq!(wl.len(), 2);
        assert_eq!(
            (wl[0].side, wl[0].checked, wl[0].checkable),
            (Side::Inbound, false, true)
        );
        assert_eq!(
            (wl[1].side, wl[1].checked, wl[1].checkable),
            (Side::Outbound, false, false)
        );
        assert_eq!(
            wl[0].flags[0].text(Side::Inbound, &shop().keys),
            "no scope labels: rule names the workload"
        );
        assert_eq!(
            wl[1].flags[0].text(Side::Outbound, &shop().keys),
            "no scope labels: no ruleset to write into"
        );
        assert_eq!(
            wl[0].peer,
            PeerKind::Workload {
                href: "/orgs/5/workloads/rich-hr-web-p".into(),
                name: f[12].src.display_name()
            }
        );
    }

    #[test]
    fn ip_peers_past_25_fold_into_one_read_only_row_by_connections() {
        let mut f = rich_flows();
        for i in 0..27u64 {
            let mut x = to_ip(22, &format!("198.51.100.{i}"));
            x.num_connections = 100 + i;
            f.push(x);
        }
        let r = rows(&f, &shop(), "role", &rich_index());
        let out: Vec<&GenRow> = r.iter().filter(|r| r.side == Side::Outbound).collect();
        // crm/prod, 25 IPs, the folded row.
        assert_eq!(out.len(), 27);
        let folded = out.last().unwrap();
        assert_eq!(
            folded.peer,
            PeerKind::FoldedIps(3),
            "the 3 smallest: 203.0.113.7, .0, .1"
        );
        assert_eq!(folded.peer_text(), "3 more IPs: not proposed");
        assert!(!folded.checked && !folded.checkable);
        assert_eq!(out[1].peer_text(), "198.51.100.26", "the largest first");
    }

    #[test]
    fn only_unsupported_services_make_a_row_uncheckable() {
        let mut f = rich_flows();
        let mut icmp = f[12].clone(); // hr/prod -> shop/prod
        icmp.port = None;
        icmp.proto = "icmp".into();
        icmp.src.labels.iter_mut().for_each(|l| {
            if l.key == "app" {
                l.value = "erp".into();
                l.href = "/orgs/5/labels/102".into();
            }
        });
        f.push(icmp);
        let r = rows(&f, &shop(), "role", &rich_index());
        let erp = r.iter().find(|r| r.peer_text() == "erp/prod").unwrap();
        assert_eq!(erp.flags, vec![RowFlag::UnsupportedProto(1)]);
        assert!(
            !erp.checked && !erp.checkable,
            "the fixture has no ICMP service"
        );
        assert_eq!(
            erp.flags[0].text(Side::Inbound, &[]),
            "proto 1 needs a service object"
        );
    }

    #[test]
    fn many_services_and_ephemeral_ports_are_flagged_and_stay_checked() {
        let mut f = rich_flows();
        f.extend((0..33u16).map(|i| intra_on(1000 + 2 * i)));
        f.push(intra_on(50_000));
        let r = rows(&f, &shop(), "role", &rich_index());
        assert!(r[0].checked);
        assert!(
            r[0].flags.contains(&RowFlag::ManyServices(35)),
            "{:?}",
            r[0].flags
        );
        assert!(r[0].flags.contains(&RowFlag::Ephemeral));
    }

    #[test]
    fn a_flow_without_a_role_flags_the_row_only_at_role_stops() {
        let mut f = rich_flows();
        let mut x = f[12].clone();
        x.src.labels.retain(|l| l.key != "role");
        f.push(x);
        let r = rows(&f, &shop(), "role", &rich_index());
        let hr = r.iter().find(|r| r.peer_text() == "hr/prod").unwrap();
        assert_eq!(hr.flags, vec![RowFlag::NoRole { flows: 1 }]);
        assert!(hr.flag_texts(super::super::Stop::AppPorts, &[]).is_empty());
        assert_eq!(
            hr.flag_texts(super::super::Stop::RolePorts, &[]),
            vec!["1 flows without role: app-level for those"]
        );
    }

    #[test]
    fn with_one_scope_key_every_app_peer_is_checked() {
        let shop = Focus::new(parse_app("shop"), vec!["app".into()]);
        let r = rows(&rich_flows(), &shop, "role", &rich_index());
        assert!(
            r.iter()
                .filter(|r| matches!(r.peer, PeerKind::App(_)))
                .all(|r| r.checked && r.flags.is_empty()),
            "{:?}",
            texts(&r)
        );
    }

    #[test]
    fn the_any_list_is_known_by_its_data_set_or_its_name() {
        let any: IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Everything",
            "external_data_set": "com.illumio.ilo_defaults"
        }))
        .unwrap();
        assert!(is_any_ip_list(&any));
        let named: IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/2", "name": "Any (0.0.0.0/0 and ::/0)"
        }))
        .unwrap();
        assert!(is_any_ip_list(&named));
        let mut f = rich_flows()[17].clone();
        f.src.ip_lists = vec!["Any (0.0.0.0/0 and ::/0)".into()];
        let r = rows(&[f], &shop(), "role", &rich_index());
        assert_eq!(
            r[0].flags,
            vec![RowFlag::NeedsIpList],
            "the Any list is never named"
        );
        assert_eq!(
            ip_list_name(&shop().app, "10.1.4.9"),
            "shop-prod peer 10.1.4.9"
        );
    }
}
