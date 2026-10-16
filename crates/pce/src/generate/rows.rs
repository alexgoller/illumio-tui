//! Flows to rows: one intra-app row, one row per app peer, IP peer and
//! unlabelled workload, on each side, with their default checkbox and
//! their flags. Broadcast and multicast are left out first.
use super::services::{Expr, express, flow_spec, fold_ports};
use super::{DecisionCounts, GenRow, PeerKey, PeerKind, RowFlag, RowKey, Side};
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
pub(crate) fn row_key(f: &ResolvedFlow, focus: &Focus) -> Option<RowKey> {
    if excluded(f) {
        return None;
    }
    let side = side_of(f, focus)?;
    let other = match side {
        Side::Inbound if group_key(&f.src, &focus.keys) == focus.app => {
            return Some(RowKey {
                side,
                peer: PeerKey::Intra,
            });
        }
        Side::Inbound => &f.src,
        Side::Outbound => &f.dst,
    };
    let peer = if other.is_workload() {
        match scope_values(other, &focus.keys) {
            Some(v) => PeerKey::App(v),
            None => PeerKey::Workload(other.workload_href.clone().unwrap_or_default()),
        }
    } else {
        PeerKey::Ip(other.ip.clone().unwrap_or_else(|| "?".into()))
    };
    Some(RowKey { side, peer })
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

/// The first IP list the PCE reported for the endpoint other than the
/// built-in Any list, else an existing list named as the batch would name
/// it; `None` when the batch creates one.
fn peer_list(ep: &ResolvedEndpoint, app: &[String], idx: &NameIndex) -> Option<String> {
    let any = |name: &str| match idx.ip_list_href(name).and_then(|h| idx.ip_list(h)) {
        Some(l) => is_any_ip_list(l),
        None => name == ANY_LIST,
    };
    ep.ip_lists.iter().find(|n| !any(n)).cloned().or_else(|| {
        let ip = ep.ip.as_deref()?;
        let name = ip_list_name(app, ip);
        idx.ip_list_href(&name).map(|_| name)
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
    /// The IP peer's list, the workload's name: from the first flow.
    list: Option<String>,
    name: String,
}

/// The rows of `flows` for the app `focus` names (its keys must be the
/// scope keys; the caller checks). Inbound first, then outbound; in each:
/// intra-app, app peers (by flows), unlabelled workloads, IP peers (by
/// connections, at most [`MAX_IP_ROWS`], the rest folded). Every row's
/// checkbox at its default.
pub fn rows(flows: &[ResolvedFlow], focus: &Focus, tier_key: &str, idx: &NameIndex) -> Vec<GenRow> {
    let mut acc: BTreeMap<RowKey, Acc> = BTreeMap::new();
    for f in flows {
        let Some(key) = row_key(f, focus) else {
            continue;
        };
        let a = acc.entry(key.clone()).or_default();
        if a.flows == 0 {
            let other = match key.side {
                Side::Inbound => &f.src,
                Side::Outbound => &f.dst,
            };
            a.name = other.display_name();
            if matches!(key.peer, PeerKey::Ip(_)) {
                a.list = peer_list(other, &focus.app, idx);
            }
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

/// Intra-app, app peers, workloads, IP peers, folded; then by size.
fn order(r: &GenRow) -> (u8, std::cmp::Reverse<u64>, String) {
    match &r.peer {
        PeerKind::Intra => (0, std::cmp::Reverse(0), String::new()),
        PeerKind::App(v) => (1, std::cmp::Reverse(r.flows), v.join("/")),
        PeerKind::Workload { href, .. } => (2, std::cmp::Reverse(r.flows), href.clone()),
        PeerKind::Ip { ip, .. } => (3, std::cmp::Reverse(r.connections), ip.clone()),
        PeerKind::FoldedIps(_) => (4, std::cmp::Reverse(0), String::new()),
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
            match (&a.list, addressable) {
                (Some(n), _) => flags.push(RowFlag::UsesIpList(n.clone())),
                (None, true) => flags.push(RowFlag::NeedsIpList),
                (None, false) => flags.push(RowFlag::NoAddress),
            }
            let peer = PeerKind::Ip {
                ip: ip.clone(),
                list: a.list.clone(),
            };
            (peer, false, a.list.is_some() || addressable)
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
                (Inbound, "10.1.4.9".into(), false, true),
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
        assert_eq!(r[3].flags, vec![RowFlag::UsesIpList("Corp".into())]);
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
            PeerKind::Ip {
                ip: "10.1.4.9".into(),
                list: Some("Corp".into())
            }
        );
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
