//! App sections: the loaded flows grouped under each app (a group of the
//! `g` keys), inbound or outbound, one row per peer group and service.
//! App-to-app's `in` / `out` / `both` states, the Blocked view and the CLI's
//! `traffic groups --by` all read these; the Flows view's app sections
//! (`flow_sections`) share their totals and order. Pure; no requests.
use super::aggregate::{
    ConnsByDecision, Dominant, DraftCounts, dominant, group_key, keys_or_default, tally,
};
use super::resolved::{ResolvedFlow, service_text};
use crate::models::PolicyDecision;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

/// At most this many names in `PeerRow::peer_names` and `app_names`.
pub const PEER_NAMES: usize = 5;

/// Which side of a flow the section's app is on: `In` = the destination
/// (the peers are the sources), `Out` = the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }

    /// `←` for inbound rows, `→` for outbound ones.
    pub fn arrow(self) -> &'static str {
        match self {
            Direction::In => "←",
            Direction::Out => "→",
        }
    }
}

/// The decision filter: which flows the sections count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decisions {
    #[default]
    All,
    /// Blocked and potentially blocked.
    Blocked,
    /// Blocked only.
    BlockedOnly,
    /// Blocked or potentially blocked now, allowed under the draft policy
    /// (`ResolvedFlow::fixed_by_draft`): only with draft decisions loaded.
    FixedByDraft,
}

impl Decisions {
    pub fn keeps(self, f: &ResolvedFlow) -> bool {
        let d = f.policy_decision;
        match self {
            Decisions::All => true,
            Decisions::Blocked => matches!(
                d,
                PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
            ),
            Decisions::BlockedOnly => d == PolicyDecision::Blocked,
            Decisions::FixedByDraft => f.fixed_by_draft(),
        }
    }

    /// The flows this filter keeps, in order: all of them, borrowed, for
    /// `All`. What the pairs (plain and by service) group.
    pub fn kept(self, flows: &[ResolvedFlow]) -> Cow<'_, [ResolvedFlow]> {
        if self == Decisions::All {
            return Cow::Borrowed(flows);
        }
        Cow::Owned(flows.iter().filter(|f| self.keeps(f)).cloned().collect())
    }

    /// `all` → `blocked` → `blocked only` → `all`; with `draft` decisions
    /// loaded, `fixed by draft` after `blocked only`.
    pub fn next(self, draft: bool) -> Self {
        match self {
            Decisions::All => Decisions::Blocked,
            Decisions::Blocked => Decisions::BlockedOnly,
            Decisions::BlockedOnly if draft => Decisions::FixedByDraft,
            Decisions::BlockedOnly | Decisions::FixedByDraft => Decisions::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Decisions::All => "all",
            Decisions::Blocked => "blocked",
            Decisions::BlockedOnly => "blocked only",
            Decisions::FixedByDraft => "fixed by draft",
        }
    }
}

/// The order of the sections (`s` on a sectioned view): by connections
/// (the default), by app name, or by flow records. The rows inside a
/// section keep theirs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SectionOrder {
    #[default]
    Connections,
    Name,
    Flows,
}

impl SectionOrder {
    /// `connections ↓` → `app A→Z` → `flows ↓` → `connections ↓`.
    pub fn next(self) -> Self {
        match self {
            SectionOrder::Connections => SectionOrder::Name,
            SectionOrder::Name => SectionOrder::Flows,
            SectionOrder::Flows => SectionOrder::Connections,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SectionOrder::Connections => "by conns ↓",
            SectionOrder::Name => "by app A→Z",
            SectionOrder::Flows => "by flows ↓",
        }
    }
}

/// A section's or an app's counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// Distinct peer groups behind the section.
    pub groups: usize,
    pub flows: u64,
    pub connections: u64,
    pub bytes: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
    /// What the draft policy would change, with draft decisions loaded.
    #[serde(flatten)]
    pub draft: Option<DraftCounts>,
}

impl Totals {
    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// `7 groups · 412 conns · 1 blocked · 2 pot.blocked`; the decision
    /// counts only when not zero.
    pub fn text(&self) -> String {
        format!(
            "{} group{} · {}",
            self.groups,
            if self.groups == 1 { "" } else { "s" },
            self.counts_text()
        )
    }

    /// `text` without the group count: `412 conns · 1 blocked`.
    pub fn counts_text(&self) -> String {
        let mut s = format!("{} conns", self.connections);
        if self.blocked > 0 {
            s.push_str(&format!(" · {} blocked", self.blocked));
        }
        if self.potentially_blocked > 0 {
            s.push_str(&format!(" · {} pot.blocked", self.potentially_blocked));
        }
        if let Some(d) = &self.draft {
            if d.draft_allowed > 0 {
                s.push_str(&format!(" · draft allows {}", d.draft_allowed));
            }
            if d.draft_blocked > 0 {
                s.push_str(&format!(" · draft blocks {}", d.draft_blocked));
            }
        }
        s
    }

    /// `12 flows · 412 conns · 1 blocked`: a Flows section's header;
    /// `gone` for a section of compare mode's gone rows only.
    pub fn flows_text(&self) -> String {
        if self.flows == 0 {
            return "gone".into();
        }
        format!(
            "{} flow{} · {}",
            self.flows,
            if self.flows == 1 { "" } else { "s" },
            self.counts_text()
        )
    }

    /// One flow record counted (the groups are counted by the caller).
    fn add_flow(&mut self, f: &ResolvedFlow) {
        self.flows += 1;
        self.connections += f.num_connections;
        self.bytes += f.bytes();
        tally(
            f.policy_decision,
            &mut self.allowed,
            &mut self.potentially_blocked,
            &mut self.blocked,
        );
        DraftCounts::add(&mut self.draft, f);
    }

    fn add_row(&mut self, r: &PeerRow) {
        self.flows += r.flows;
        self.connections += r.connections;
        self.bytes += r.bytes;
        self.allowed += r.allowed;
        self.potentially_blocked += r.potentially_blocked;
        self.blocked += r.blocked;
        if let Some(d) = &r.draft {
            let t = self.draft.get_or_insert_with(DraftCounts::default);
            t.draft_allowed += d.draft_allowed;
            t.draft_blocked += d.draft_blocked;
        }
    }
}

/// One peer group and service under an app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeerRow {
    /// The peer's group (`group_key`): `crm/prod`, `ipl:Corp/-`, `fqdn/-`.
    pub peer: Vec<String>,
    /// Distinct peer display names, at most `PEER_NAMES`.
    pub peer_names: Vec<String>,
    /// Distinct peer hosts behind the row, uncapped.
    pub peer_hosts: usize,
    /// `5432/tcp`, or `icmp`.
    pub service: String,
    /// Flow records.
    pub flows: u64,
    /// The records' lifetime connection counts, summed.
    pub connections: u64,
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
    /// Distinct display names on the app's side, at most `PEER_NAMES`
    /// (what Blocked shows as `dst name`).
    pub app_names: Vec<String>,
    /// Distinct hosts on the app's side behind the row, uncapped.
    pub app_hosts: usize,
    /// The newest flow's last detection.
    pub last_detected: Option<String>,
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

/// The one name when there is one host, else `N hosts`.
fn names_text(names: &[String], hosts: usize) -> String {
    match names {
        [one] if hosts <= 1 => one.clone(),
        _ => format!("{hosts} hosts"),
    }
}

impl PeerRow {
    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// `crm/prod`.
    pub fn peer_text(&self) -> String {
        self.peer.join("/")
    }

    /// The peer's one host name, or `N hosts`.
    pub fn peer_name_text(&self) -> String {
        names_text(&self.peer_names, self.peer_hosts)
    }

    /// The app side's one host name, or `N hosts`.
    pub fn app_name_text(&self) -> String {
        names_text(&self.app_names, self.app_hosts)
    }
}

/// Every peer row of one app in one direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppSection {
    pub app: Vec<String>,
    pub direction: Direction,
    pub rows: Vec<PeerRow>,
    pub totals: Totals,
}

impl AppSection {
    /// `shop/prod`.
    pub fn app_text(&self) -> String {
        self.app.join("/")
    }

    /// `in · 7 groups · 412 conns · 1 blocked`.
    pub fn header_text(&self) -> String {
        app_header(&[(self.direction, &self.totals)])
    }
}

/// The header of one app's sections: `in · 7 groups · 412 conns · 1
/// blocked` for one direction; with both, `← 1545 conns · 2 blocked  →
/// 1004 conns` (no group counts, so it fits 80 columns).
/// A direction without a recent flow (compare mode: gone rows only)
/// reads `gone`.
pub fn app_header(parts: &[(Direction, &Totals)]) -> String {
    let gone = |t: &Totals| t.flows == 0;
    match parts {
        [(d, t)] if gone(t) => format!("{} · gone", d.as_str()),
        [(d, t)] => format!("{} · {}", d.as_str(), t.text()),
        _ => parts
            .iter()
            .map(|(d, t)| {
                if gone(t) {
                    format!("{} gone", d.arrow())
                } else {
                    format!("{} {}", d.arrow(), t.counts_text())
                }
            })
            .collect::<Vec<_>>()
            .join("  "),
    }
}

/// `(app, peer)` of `f` for `direction` under `keys`.
pub fn sides(
    f: &ResolvedFlow,
    keys: &[String],
    direction: Direction,
) -> (Vec<String>, Vec<String>) {
    let (src, dst) = (group_key(&f.src, keys), group_key(&f.dst, keys));
    match direction {
        Direction::In => (dst, src),
        Direction::Out => (src, dst),
    }
}

/// Whether `f` is counted under `app` in `direction` (any direction when
/// `None`) with `decisions`. `keys` are the resolved keys (never empty).
pub fn in_app(
    f: &ResolvedFlow,
    keys: &[String],
    app: &[String],
    direction: Option<Direction>,
    decisions: Decisions,
) -> bool {
    decisions.keeps(f)
        && match direction {
            Some(d) => sides(f, keys, d).0 == app,
            None => group_key(&f.src, keys) == app || group_key(&f.dst, keys) == app,
        }
}

/// Whether `f` is behind the peer row `(peer, service)` under `app`.
/// `keys` are the resolved keys (never empty).
pub fn in_peer(
    f: &ResolvedFlow,
    keys: &[String],
    app: &[String],
    direction: Direction,
    peer: &[String],
    service: &str,
    decisions: Decisions,
) -> bool {
    if !decisions.keeps(f) || service_text(f.port, &f.proto) != service {
        return false;
    }
    let (a, p) = sides(f, keys, direction);
    a == app && p == peer
}

#[derive(Default)]
struct Seen {
    peer_names: BTreeSet<String>,
    app_names: BTreeSet<String>,
}

fn push_capped(names: &mut Vec<String>, name: &str) {
    if names.len() < PEER_NAMES && !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
}

/// The loaded flows `decisions` keeps, one section per app on the
/// `direction` side, one row per peer group and service. Sections by
/// total connections (then app), rows by connections (then peer and
/// service). A non-workload peer is grouped by its name (`group_key`).
pub fn app_sections(
    flows: &[ResolvedFlow],
    keys: &[String],
    direction: Direction,
    decisions: Decisions,
) -> Vec<AppSection> {
    let keys = keys_or_default(keys);
    type RowKey = (Vec<String>, String);
    let mut apps: BTreeMap<Vec<String>, BTreeMap<RowKey, (PeerRow, Seen)>> = BTreeMap::new();
    for f in flows.iter().filter(|f| decisions.keeps(f)) {
        let (app, peer) = sides(f, &keys, direction);
        let (peer_ep, app_ep) = match direction {
            Direction::In => (&f.src, &f.dst),
            Direction::Out => (&f.dst, &f.src),
        };
        let service = service_text(f.port, &f.proto);
        let (row, seen) = apps
            .entry(app)
            .or_default()
            .entry((peer.clone(), service.clone()))
            .or_insert_with(|| {
                (
                    PeerRow {
                        peer,
                        peer_names: Vec::new(),
                        peer_hosts: 0,
                        service,
                        flows: 0,
                        connections: 0,
                        bytes: 0,
                        allowed: 0,
                        potentially_blocked: 0,
                        blocked: 0,
                        conns: ConnsByDecision::default(),
                        draft: None,
                        app_names: Vec::new(),
                        app_hosts: 0,
                        last_detected: None,
                        marks: None,
                        change: None,
                    },
                    Seen::default(),
                )
            });
        row.flows += 1;
        row.connections += f.num_connections;
        row.bytes += f.bytes();
        tally(
            f.policy_decision,
            &mut row.allowed,
            &mut row.potentially_blocked,
            &mut row.blocked,
        );
        row.conns.add(f.policy_decision, f.num_connections);
        DraftCounts::add(&mut row.draft, f);
        let peer_name = peer_ep.display_name();
        push_capped(&mut row.peer_names, &peer_name);
        seen.peer_names.insert(peer_name);
        row.peer_hosts = seen.peer_names.len();
        let app_name = app_ep.display_name();
        push_capped(&mut row.app_names, &app_name);
        seen.app_names.insert(app_name);
        row.app_hosts = seen.app_names.len();
        if f.last_detected > row.last_detected {
            row.last_detected = f.last_detected.clone();
        }
    }
    let out: Vec<AppSection> = apps
        .into_iter()
        .map(|(app, rows)| {
            let mut rows: Vec<PeerRow> = rows.into_values().map(|(r, _)| r).collect();
            rows.sort_by(|a, b| {
                b.connections
                    .cmp(&a.connections)
                    .then_with(|| a.peer.cmp(&b.peer))
                    .then_with(|| a.service.cmp(&b.service))
            });
            let mut totals = Totals::default();
            let mut peers: BTreeSet<&Vec<String>> = BTreeSet::new();
            for r in &rows {
                totals.add_row(r);
                peers.insert(&r.peer);
            }
            totals.groups = peers.len();
            AppSection {
                app,
                direction,
                rows,
                totals,
            }
        })
        .collect();
    order_sections(out, SectionOrder::Connections, |s| s)
}

/// `items` (each with its section, `section`) in `order`: consecutive
/// sections of one app (`both`: in, then out) stay together and weigh
/// together; ties by app. A stable sort: what `order` cannot tell apart
/// keeps its place.
pub fn order_sections<T>(
    items: Vec<T>,
    order: SectionOrder,
    section: impl Fn(&T) -> &AppSection,
) -> Vec<T> {
    // (app, flows, connections, its sections)
    let mut apps: Vec<(Vec<String>, u64, u64, Vec<T>)> = Vec::new();
    for it in items {
        let s = section(&it);
        let (flows, conns) = (s.totals.flows, s.totals.connections);
        match apps.last_mut() {
            Some(last) if last.0 == s.app => {
                last.1 += flows;
                last.2 += conns;
                last.3.push(it);
            }
            _ => apps.push((s.app.clone(), flows, conns, vec![it])),
        }
    }
    apps.sort_by(|a, b| {
        match order {
            SectionOrder::Connections => b.2.cmp(&a.2),
            SectionOrder::Flows => b.1.cmp(&a.1),
            SectionOrder::Name => {
                a.0.join("/")
                    .to_lowercase()
                    .cmp(&b.0.join("/").to_lowercase())
            }
        }
        .then_with(|| a.0.cmp(&b.0))
    });
    apps.into_iter().flat_map(|(_, _, _, s)| s).collect()
}

/// Both directions: for each app its inbound section then its outbound
/// one (either may be missing), apps by their combined connections.
pub fn app_sections_both(
    flows: &[ResolvedFlow],
    keys: &[String],
    decisions: Decisions,
) -> Vec<AppSection> {
    let mut by_app: BTreeMap<Vec<String>, Vec<AppSection>> = BTreeMap::new();
    for d in [Direction::In, Direction::Out] {
        for s in app_sections(flows, keys, d, decisions) {
            by_app.entry(s.app.clone()).or_default().push(s);
        }
    }
    let grouped = by_app.into_values().flatten().collect();
    order_sections(grouped, SectionOrder::Connections, |s| s)
}

/// The Flows view's app sections (`x`): one per app on `direction`'s side
/// of `flows` (`sides`; every flow, no decision filter), in `order`. Each
/// section has no peer rows; with it come the positions in `flows` of its
/// flow records, in their order. Its totals are App-to-app's for the same
/// flows.
pub fn flow_sections<'a>(
    flows: impl IntoIterator<Item = &'a ResolvedFlow>,
    keys: &[String],
    direction: Direction,
    order: SectionOrder,
) -> Vec<(AppSection, Vec<usize>)> {
    let keys = keys_or_default(keys);
    type Acc = (Totals, BTreeSet<Vec<String>>, Vec<usize>);
    let mut apps: BTreeMap<Vec<String>, Acc> = BTreeMap::new();
    for (i, f) in flows.into_iter().enumerate() {
        let (app, peer) = sides(f, &keys, direction);
        let (totals, peers, idx) = apps.entry(app).or_default();
        totals.add_flow(f);
        peers.insert(peer);
        idx.push(i);
    }
    let out = apps
        .into_iter()
        .map(|(app, (mut totals, peers, idx))| {
            totals.groups = peers.len();
            (
                AppSection {
                    app,
                    direction,
                    rows: Vec::new(),
                    totals,
                },
                idx,
            )
        })
        .collect();
    order_sections(out, order, |(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::{flows, rich_flows};

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn app_env() -> Vec<String> {
        keys(&["app", "env"])
    }

    #[test]
    fn peer_rows_count_connections_per_decision() {
        let sections = app_sections_both(&rich_flows(), &app_env(), Decisions::All);
        let rows: Vec<&PeerRow> = sections.iter().flat_map(|s| &s.rows).collect();
        assert!(!rows.is_empty());
        for r in rows {
            let c = &r.conns;
            assert_eq!(
                c.allowed_conns + c.pot_blocked_conns + c.blocked_conns + c.unknown_conns,
                r.connections,
                "{r:?}"
            );
            assert_eq!(c.blocked_conns > 0, r.blocked > 0, "{r:?}");
            let json = serde_json::to_value(r).unwrap();
            assert_eq!(json["blocked_conns"], c.blocked_conns);
        }
    }

    fn sum(s: &AppSection) -> Totals {
        let r = &s.rows;
        Totals {
            groups: r.iter().map(|r| &r.peer).collect::<BTreeSet<_>>().len(),
            flows: r.iter().map(|r| r.flows).sum(),
            connections: r.iter().map(|r| r.connections).sum(),
            bytes: r.iter().map(|r| r.bytes).sum(),
            allowed: r.iter().map(|r| r.allowed).sum(),
            potentially_blocked: r.iter().map(|r| r.potentially_blocked).sum(),
            blocked: r.iter().map(|r| r.blocked).sum(),
            draft: None,
        }
    }

    #[test]
    fn every_flow_lands_once_per_direction_and_totals_sum_the_rows() {
        let f = rich_flows();
        let total: u64 = f.iter().map(|f| f.num_connections).sum();
        for d in [Direction::In, Direction::Out] {
            let s = app_sections(&f, &app_env(), d, Decisions::All);
            assert_eq!(
                s.iter().map(|s| s.totals.flows).sum::<u64>(),
                f.len() as u64,
                "{d:?}"
            );
            assert_eq!(s.iter().map(|s| s.totals.connections).sum::<u64>(), total);
            for sec in &s {
                assert_eq!(sec.totals, sum(sec), "{}", sec.app_text());
                assert_eq!(sec.direction, d);
                for r in &sec.rows {
                    // Each row's flows sit on its side: the app on `d`'s
                    // side, the peer on the other.
                    let n = f
                        .iter()
                        .filter(|x| {
                            in_peer(
                                x,
                                &app_env(),
                                &sec.app,
                                d,
                                &r.peer,
                                &r.service,
                                Decisions::All,
                            )
                        })
                        .count() as u64;
                    assert_eq!(n, r.flows, "{} {}", sec.app_text(), r.peer_text());
                }
            }
        }
    }

    #[test]
    fn sections_and_rows_sort_by_connections() {
        let s = app_sections(&rich_flows(), &app_env(), Direction::In, Decisions::All);
        assert!(
            s.windows(2)
                .all(|w| w[0].totals.connections >= w[1].totals.connections)
        );
        for sec in &s {
            assert!(
                sec.rows
                    .windows(2)
                    .all(|w| w[0].connections >= w[1].connections)
            );
        }
        let first: Vec<(String, u64, usize)> = s
            .iter()
            .take(3)
            .map(|s| (s.app_text(), s.totals.connections, s.totals.groups))
            .collect();
        assert_eq!(
            first,
            vec![
                ("crm/prod".to_string(), 1601, 6),
                ("shop/prod".to_string(), 1545, 4),
                ("erp/prod".to_string(), 692, 4),
            ]
        );
    }

    #[test]
    fn the_four_flow_fixture_reads_as_expected() {
        let s = app_sections(&flows(), &app_env(), Direction::In, Decisions::All);
        let shop = &s[0];
        assert_eq!(shop.app, vec!["shop", "prod"]);
        assert_eq!(
            shop.header_text(),
            "in · 3 groups · 325 conns · 1 blocked · 1 pot.blocked"
        );
        let peers: Vec<(String, String, u64)> = shop
            .rows
            .iter()
            .map(|r| (r.peer_text(), r.service.clone(), r.connections))
            .collect();
        assert_eq!(
            peers,
            vec![
                ("shop/prod".into(), "5432/tcp".into(), 310),
                ("ipl:Corp/-".into(), "8443/tcp".into(), 12),
                ("unmanaged-nas/-".into(), "icmp".into(), 3),
            ]
        );
        // A non-workload peer keeps its name, like the FQDN as an app.
        let out = app_sections(&flows(), &app_env(), Direction::Out, Decisions::All);
        let fqdn = out
            .iter()
            .flat_map(|s| &s.rows)
            .find(|r| r.peer == vec!["fqdn-1.example", "-"])
            .expect("the FQDN peer");
        assert_eq!(fqdn.peer_name_text(), "fqdn-1.example");
        let corp = &shop.rows[1];
        assert_eq!(
            corp.peer_name_text(),
            "10.1.4.9",
            "the IP list peer's one address"
        );
        assert_eq!(corp.app_name_text(), "host-1.example");
        assert_eq!(corp.dominant(), PolicyDecision::PotentiallyBlocked);
    }

    #[test]
    fn the_both_header_drops_the_group_counts_to_fit() {
        let t_in = Totals {
            groups: 4,
            flows: 5,
            connections: 1545,
            blocked: 2,
            potentially_blocked: 1,
            ..Totals::default()
        };
        let t_out = Totals {
            groups: 3,
            flows: 3,
            connections: 1004,
            ..Totals::default()
        };
        assert_eq!(
            app_header(&[(Direction::In, &t_in)]),
            "in · 4 groups · 1545 conns · 2 blocked · 1 pot.blocked"
        );
        assert_eq!(
            app_header(&[(Direction::In, &t_in), (Direction::Out, &t_out)]),
            "← 1545 conns · 2 blocked · 1 pot.blocked  → 1004 conns"
        );
        // Compare mode: a direction of gone rows only.
        let gone = Totals::default();
        assert_eq!(app_header(&[(Direction::Out, &gone)]), "out · gone");
        assert_eq!(
            app_header(&[(Direction::In, &t_in), (Direction::Out, &gone)]),
            "← 1545 conns · 2 blocked · 1 pot.blocked  → gone"
        );
        assert_eq!(gone.flows_text(), "gone");
    }

    #[test]
    fn decision_filters_keep_blocked_and_potentially_blocked_or_blocked_only() {
        let f = rich_flows();
        let count = |d: Decisions| -> u64 {
            app_sections(&f, &app_env(), Direction::In, d)
                .iter()
                .map(|s| s.totals.flows)
                .sum()
        };
        assert_eq!(count(Decisions::All), 30);
        assert_eq!(count(Decisions::Blocked), 14);
        assert_eq!(count(Decisions::BlockedOnly), 7);
        let only = app_sections(&f, &app_env(), Direction::In, Decisions::BlockedOnly);
        assert!(
            only.iter()
                .all(|s| s.totals.allowed == 0 && s.totals.potentially_blocked == 0)
        );
        assert_eq!(Decisions::All.next(false), Decisions::Blocked);
        assert_eq!(Decisions::BlockedOnly.next(false), Decisions::All);
        assert_eq!(Decisions::BlockedOnly.label(), "blocked only");
        assert_eq!(Decisions::BlockedOnly.next(true), Decisions::FixedByDraft);
        assert_eq!(Decisions::FixedByDraft.next(true), Decisions::All);
        assert_eq!(Decisions::FixedByDraft.next(false), Decisions::All);
    }

    #[test]
    fn draft_counts_per_row_and_section_and_the_fixed_by_draft_filter() {
        use crate::traffic::resolved::tests::rich_flows_draft;
        let plain = rich_flows();
        let s = app_sections(&plain, &app_env(), Direction::In, Decisions::All);
        assert!(s.iter().all(|s| s.totals.draft.is_none()));
        assert!(s.iter().flat_map(|s| &s.rows).all(|r| r.draft.is_none()));
        let json = serde_json::to_value(&s[0].rows[0]).unwrap();
        assert!(json.get("draft_allowed").is_none(), "absent without draft");

        let f = rich_flows_draft();
        let fixed = f.iter().filter(|f| f.fixed_by_draft()).count() as u64;
        let newly = f.iter().filter(|f| f.blocked_by_draft()).count() as u64;
        assert!(fixed > 0 && newly > 0);
        let s = app_sections(&f, &app_env(), Direction::In, Decisions::All);
        let sum = |pick: fn(&DraftCounts) -> u64| -> u64 {
            s.iter()
                .map(|s| s.totals.draft.as_ref().map_or(0, pick))
                .sum()
        };
        assert_eq!(sum(|d| d.draft_allowed), fixed);
        let t = s
            .iter()
            .find(|s| s.totals.draft.is_some_and(|d| d.draft_allowed > 0));
        assert!(t.unwrap().totals.counts_text().contains(" · draft allows "));
        assert_eq!(sum(|d| d.draft_blocked), newly);
        let row = s
            .iter()
            .flat_map(|s| &s.rows)
            .find(|r| r.draft.is_some())
            .unwrap();
        let json = serde_json::to_value(row).unwrap();
        assert!(json["draft_allowed"].is_u64() && json["draft_blocked"].is_u64());
        let kept = app_sections(&f, &app_env(), Direction::In, Decisions::FixedByDraft);
        let n: u64 = kept.iter().map(|s| s.totals.flows).sum();
        assert_eq!(n, fixed);
        assert_eq!(
            Decisions::FixedByDraft.kept(&plain).len(),
            0,
            "nothing is fixed without draft decisions"
        );
    }

    #[test]
    fn both_puts_each_apps_inbound_then_outbound_section_together() {
        let f = rich_flows();
        let both = app_sections_both(&f, &app_env(), Decisions::All);
        let ins = app_sections(&f, &app_env(), Direction::In, Decisions::All);
        let outs = app_sections(&f, &app_env(), Direction::Out, Decisions::All);
        assert_eq!(both.len(), ins.len() + outs.len());
        for w in both.windows(2) {
            if w[0].app == w[1].app {
                assert_eq!(
                    (w[0].direction, w[1].direction),
                    (Direction::In, Direction::Out)
                );
            }
        }
        let apps: Vec<&Vec<String>> = both.iter().map(|s| &s.app).collect();
        let mut dedup = apps.clone();
        dedup.dedup();
        let distinct: BTreeSet<&Vec<String>> = apps.iter().copied().collect();
        assert_eq!(
            dedup.len(),
            distinct.len(),
            "an app's sections are adjacent"
        );
    }

    #[test]
    fn in_app_and_in_peer_pick_the_rows_flows() {
        let f = rich_flows();
        let k = app_env();
        for sec in app_sections(&f, &k, Direction::Out, Decisions::Blocked) {
            let n = f
                .iter()
                .filter(|x| in_app(x, &k, &sec.app, Some(Direction::Out), Decisions::Blocked))
                .count() as u64;
            assert_eq!(n, sec.totals.flows);
            for r in &sec.rows {
                let n = f
                    .iter()
                    .filter(|x| {
                        in_peer(
                            x,
                            &k,
                            &sec.app,
                            Direction::Out,
                            &r.peer,
                            &r.service,
                            Decisions::Blocked,
                        )
                    })
                    .count() as u64;
                assert_eq!(n, r.flows);
            }
        }
    }
    #[test]
    fn in_with_blocked_keeps_every_fact_the_old_blocked_view_showed() {
        // What `blocked_summary` reported for the rich fixture (proven equal
        // to these sections before it was removed): per destination group
        // and service, flows, connections, sources and the host.
        let s = app_sections(&rich_flows(), &app_env(), Direction::In, Decisions::Blocked);
        let facts =
            |app: &[&str], service: &str| -> (u64, u64, BTreeSet<String>, BTreeSet<String>) {
                let rows: Vec<&PeerRow> = s
                    .iter()
                    .find(|s| s.app == keys(app))
                    .unwrap()
                    .rows
                    .iter()
                    .filter(|r| r.service == service)
                    .collect();
                (
                    rows.iter().map(|r| r.flows).sum(),
                    rows.iter().map(|r| r.connections).sum(),
                    rows.iter().map(|r| r.peer_text()).collect(),
                    rows.iter().flat_map(|r| r.app_names.clone()).collect(),
                )
            };
        let set = |v: &[&str]| -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() };
        assert_eq!(
            facts(&["shop", "prod"], "5432/tcp"),
            (
                3,
                24,
                set(&["hr/prod", "shop/dev", "ipl:Corp/-"]),
                set(&["shop-db-p.example"])
            )
        );
        let erp: Vec<u64> = ["8443/tcp", "1521/tcp", "22/tcp"]
            .iter()
            .map(|svc| facts(&["erp", "prod"], svc).1)
            .collect();
        assert_eq!(erp, vec![79, 19, 4]);
        let rows: usize = s.iter().map(|s| s.rows.len()).sum();
        assert_eq!(rows, 14, "the 10 old rows, split by source");
        let newest = s
            .iter()
            .flat_map(|s| &s.rows)
            .filter_map(|r| r.last_detected.clone())
            .max();
        assert_eq!(newest.as_deref(), Some("2026-10-10T07:26:00Z"));
        let corp = s.iter().find(|s| s.app == keys(&["hr", "prod"])).unwrap();
        assert_eq!(corp.rows[0].peer_text(), "ipl:Corp/-");
        assert_eq!(corp.rows[0].app_name_text(), "hr-web-p.example");
    }

    #[test]
    fn names_stop_at_five_while_the_host_counts_keep_going() {
        let mut many = rich_flows();
        let template = many[12].clone(); // hr/prod -> shop-db-p 5432 potentially blocked
        for i in 0..7 {
            let mut f = template.clone();
            f.dst.hostname = Some(format!("db-{i}.example"));
            f.src.hostname = Some(format!("src-{i}.example"));
            many.push(f);
        }
        let s = app_sections(&many, &app_env(), Direction::In, Decisions::Blocked);
        let row = s
            .iter()
            .find(|s| s.app == keys(&["shop", "prod"]))
            .unwrap()
            .rows
            .iter()
            .find(|r| r.peer == keys(&["hr", "prod"]))
            .unwrap();
        assert_eq!((row.peer_names.len(), row.peer_hosts), (PEER_NAMES, 8));
        assert_eq!((row.app_names.len(), row.app_hosts), (PEER_NAMES, 8));
        assert_eq!(row.peer_name_text(), "8 hosts");
        assert_eq!(row.app_name_text(), "8 hosts");
        let k = app_env();
        assert!(many.iter().any(|f| in_peer(
            f,
            &k,
            &row_app(),
            Direction::In,
            &row.peer,
            &row.service,
            Decisions::Blocked
        )));
        assert!(
            !in_peer(
                &many[0],
                &k,
                &row_app(),
                Direction::In,
                &row.peer,
                &row.service,
                Decisions::Blocked
            ),
            "an allowed flow is never behind a blocked row"
        );
    }

    #[test]
    fn kept_borrows_everything_for_all_and_filters_otherwise() {
        let flows = rich_flows();
        assert!(matches!(Decisions::All.kept(&flows), Cow::Borrowed(f) if f.len() == 30));
        let blocked = Decisions::Blocked.kept(&flows);
        assert_eq!(blocked.len(), 14);
        assert!(
            blocked
                .iter()
                .all(|f| f.policy_decision != PolicyDecision::Allowed)
        );
        let only = Decisions::BlockedOnly.kept(&flows);
        assert_eq!(only.len(), 7);
        assert!(
            only.iter()
                .all(|f| f.policy_decision == PolicyDecision::Blocked)
        );
    }

    #[test]
    fn the_section_order_cycles_connections_name_flows() {
        let o = SectionOrder::default();
        assert_eq!(o, SectionOrder::Connections);
        let seen: Vec<SectionOrder> = std::iter::successors(Some(o), |o| Some(o.next()))
            .take(4)
            .collect();
        assert_eq!(
            seen,
            vec![
                SectionOrder::Connections,
                SectionOrder::Name,
                SectionOrder::Flows,
                SectionOrder::Connections
            ]
        );
        assert_eq!(SectionOrder::Connections.label(), "by conns ↓");
        assert_eq!(SectionOrder::Name.label(), "by app A→Z");
        assert_eq!(SectionOrder::Flows.label(), "by flows ↓");
    }

    #[test]
    fn order_sections_sorts_by_name_or_flows_and_keeps_the_rows() {
        let f = rich_flows();
        let by_conns = app_sections(&f, &app_env(), Direction::In, Decisions::All);
        let by_name = order_sections(by_conns.clone(), SectionOrder::Name, |s| s);
        let names: Vec<String> = by_name.iter().map(AppSection::app_text).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted);
        let by_flows = order_sections(by_conns.clone(), SectionOrder::Flows, |s| s);
        assert!(
            by_flows
                .windows(2)
                .all(|w| w[0].totals.flows >= w[1].totals.flows)
        );
        // The same sections, each with its rows in their order.
        for s in &by_flows {
            let same = by_conns.iter().find(|c| c.app == s.app).unwrap();
            assert_eq!(s, same);
        }
        let back = order_sections(by_name, SectionOrder::Connections, |s| s);
        assert_eq!(back, by_conns, "connections is the default order");
    }

    #[test]
    fn both_keeps_an_apps_two_sections_together_in_every_order() {
        let f = rich_flows();
        let both = app_sections_both(&f, &app_env(), Decisions::All);
        for order in [SectionOrder::Name, SectionOrder::Flows] {
            let o = order_sections(both.clone(), order, |s| s);
            assert_eq!(o.len(), both.len());
            for w in o.windows(2) {
                if w[0].app == w[1].app {
                    assert_eq!(
                        (w[0].direction, w[1].direction),
                        (Direction::In, Direction::Out)
                    );
                }
            }
            let apps: Vec<&Vec<String>> = o.iter().map(|s| &s.app).collect();
            let mut dedup = apps.clone();
            dedup.dedup();
            let distinct: BTreeSet<&Vec<String>> = apps.iter().copied().collect();
            assert_eq!(dedup.len(), distinct.len(), "{order:?}");
        }
        // By flows, an app weighs its two directions together.
        let o = order_sections(both, SectionOrder::Flows, |s| s);
        let mut seen: Vec<(Vec<String>, u64)> = Vec::new();
        for s in &o {
            match seen.last_mut() {
                Some((a, n)) if *a == s.app => *n += s.totals.flows,
                _ => seen.push((s.app.clone(), s.totals.flows)),
            }
        }
        let weights: Vec<u64> = seen.iter().map(|(_, n)| *n).collect();
        assert!(weights.windows(2).all(|w| w[0] >= w[1]), "{weights:?}");
    }

    #[test]
    fn flow_sections_put_each_flow_under_its_app_in_input_order() {
        let f = rich_flows();
        let k = app_env();
        for d in [Direction::In, Direction::Out] {
            let s = flow_sections(&f, &k, d, SectionOrder::Connections);
            let sections = app_sections(&f, &k, d, Decisions::All);
            // The same apps, order and totals as App-to-app's sections.
            assert_eq!(
                s.iter()
                    .map(|(s, _)| (s.app.clone(), s.totals.clone()))
                    .collect::<Vec<_>>(),
                sections
                    .iter()
                    .map(|s| (s.app.clone(), s.totals.clone()))
                    .collect::<Vec<_>>(),
                "{d:?}"
            );
            let mut all: Vec<usize> = Vec::new();
            for (sec, idx) in &s {
                assert!(sec.rows.is_empty());
                assert_eq!(idx.len() as u64, sec.totals.flows);
                assert!(idx.windows(2).all(|w| w[0] < w[1]), "input order kept");
                for &i in idx {
                    assert_eq!(sides(&f[i], &k, d).0, sec.app);
                }
                all.extend(idx);
            }
            all.sort_unstable();
            assert_eq!(all, (0..f.len()).collect::<Vec<_>>(), "every flow once");
        }
        let by_name = flow_sections(&f, &k, Direction::Out, SectionOrder::Name);
        let names: Vec<String> = by_name.iter().map(|(s, _)| s.app_text()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted);
        assert_eq!(
            by_name[0].0.totals.flows_text(),
            format!(
                "{} flow{} · {}",
                by_name[0].0.totals.flows,
                if by_name[0].0.totals.flows == 1 {
                    ""
                } else {
                    "s"
                },
                by_name[0].0.totals.counts_text()
            )
        );
    }

    #[test]
    fn the_flows_header_counts_flows_and_connections() {
        let t = Totals {
            groups: 4,
            flows: 12,
            connections: 1545,
            blocked: 2,
            ..Totals::default()
        };
        assert_eq!(t.flows_text(), "12 flows · 1545 conns · 2 blocked");
        let one = Totals {
            flows: 1,
            connections: 3,
            ..Totals::default()
        };
        assert_eq!(one.flows_text(), "1 flow · 3 conns");
    }

    fn row_app() -> Vec<String> {
        keys(&["shop", "prod"])
    }
}
