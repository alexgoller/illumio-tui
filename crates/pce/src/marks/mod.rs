//! Flow marks: the admin's judgement of traffic (critical, important,
//! revisit, accept-blocked, reviewed, squelch), matched against resolved
//! flows. Pure: no I/O and no clock (callers pass `now`); the file
//! (`marks.toml`) is read and written by `illumio-tui::marks`. Marks are
//! never sent to the PCE and never cause a request.
use crate::models::{PolicyDecision, proto_name, proto_number};
use crate::traffic::{ResolvedEndpoint, ResolvedFlow};
use jiff::Timestamp;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::IpAddr;

/// What a mark says about its flows. `Ord` is precedence: the strongest
/// first, so a highlight (and a quiet action) always beats squelch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Critical,
    Important,
    Revisit,
    AcceptBlocked,
    Reviewed,
    Squelch,
}

impl Action {
    /// Every action, strongest first.
    pub const ALL: [Action; 6] = [
        Action::Critical,
        Action::Important,
        Action::Revisit,
        Action::AcceptBlocked,
        Action::Reviewed,
        Action::Squelch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Critical => "critical",
            Action::Important => "important",
            Action::Revisit => "revisit",
            Action::AcceptBlocked => "accept-blocked",
            Action::Reviewed => "reviewed",
            Action::Squelch => "squelch",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Action::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// The position in [`Action::ALL`]: an index into per-action arrays.
    pub fn index(self) -> usize {
        self as usize
    }

    /// `critical`, `important`, `revisit`: shown wherever one flow has it.
    pub fn is_highlight(self) -> bool {
        matches!(self, Action::Critical | Action::Important | Action::Revisit)
    }

    /// `accept-blocked`, `reviewed`: a row is dimmed only when all its
    /// flows have one.
    pub fn is_quiet(self) -> bool {
        matches!(self, Action::AcceptBlocked | Action::Reviewed)
    }

    /// `critical important revisit accept-blocked reviewed squelch`.
    pub fn names() -> String {
        Action::ALL.map(Action::as_str).join(" ")
    }

    /// The expiry a new mark gets unless one is given: a hide or a
    /// forgiveness lapses on its own, a flag does not.
    pub fn default_expiry(self) -> &'static str {
        match self {
            Action::Squelch | Action::AcceptBlocked => "90d",
            Action::Reviewed => "30d",
            Action::Revisit => "7d",
            Action::Critical | Action::Important => "never",
        }
    }
}

/// `expires 2027-01-15 (UTC)`, or `never expires`: how the TUI shows an
/// expiry (in UTC, as stored).
pub fn expiry_text(expires: Option<Timestamp>) -> String {
    match expires {
        Some(t) => format!("expires {} (UTC)", expiry_date(t)),
        None => "never expires".into(),
    }
}

/// `2027-01-15`: the UTC date of an expiry.
pub fn expiry_date(t: Timestamp) -> String {
    t.strftime("%Y-%m-%d").to_string()
}

/// An expiry as typed: `1d`, `7d`, `30d`, `90d` (any `<N>d`: now + N x 24
/// h), a date `YYYY-MM-DD` (23:59:59 UTC that day), an RFC 3339 instant,
/// or `never` (`None`). UTC, as the CLI's date-only `--to`.
pub fn parse_expiry(s: &str, now: Timestamp) -> Result<Option<Timestamp>, String> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("never") {
        return Ok(None);
    }
    if let Some(days) = t.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        if days <= 0 {
            return Err(format!("{t:?}: at least 1d"));
        }
        return Ok(Some(now + jiff::SignedDuration::from_hours(24 * days)));
    }
    if let Ok(ts) = t.parse::<Timestamp>() {
        return Ok(Some(ts));
    }
    if let Ok(d) = t.parse::<jiff::civil::Date>() {
        return d
            .at(23, 59, 59, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .map(|z| Some(z.timestamp()))
            .map_err(|e| format!("{t:?}: {e}"));
    }
    Err(format!(
        "{t:?}: write 1d, 7d, 30d, 90d, YYYY-MM-DD, an RFC 3339 time or never"
    ))
}

/// One side of a mark: which endpoints it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Any,
    /// Every key required, one of its values (a list is OR); workloads only.
    Labels(BTreeMap<String, Vec<String>>),
    /// Hostname, else workload name, case-insensitive. Always PCE-scoped.
    Workload(String),
    /// An address or CIDR containing the endpoint's IP.
    Ip {
        addr: IpAddr,
        bits: u8,
    },
    /// One of the IP lists Explorer reported for the endpoint, by name,
    /// case-insensitive. No range evaluation.
    IpList(String),
}

/// Whether `ip` is inside `net/bits` (the same family only).
fn ip_contains(net: IpAddr, bits: u8, ip: IpAddr) -> bool {
    match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(a)) => {
            let m = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            u32::from(n) & m == u32::from(a) & m
        }
        (IpAddr::V6(n), IpAddr::V6(a)) => {
            let m = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            u128::from(n) & m == u128::from(a) & m
        }
        _ => false,
    }
}

fn max_bits(addr: IpAddr) -> u8 {
    if addr.is_ipv4() { 32 } else { 128 }
}

impl Side {
    pub fn matches(&self, ep: &ResolvedEndpoint) -> bool {
        self.matches_at(ep, endpoint_ip(ep))
    }

    /// `matches` with the endpoint's address parsed once per flow.
    fn matches_at(&self, ep: &ResolvedEndpoint, ip: Option<IpAddr>) -> bool {
        match self {
            Side::Any => true,
            Side::Labels(want) => {
                ep.is_workload()
                    && want.iter().all(|(k, values)| {
                        ep.labels
                            .iter()
                            .any(|l| l.key != "?" && &l.key == k && values.contains(&l.value))
                    })
            }
            Side::Workload(name) => ep
                .hostname
                .as_deref()
                .or(ep.name.as_deref())
                .is_some_and(|h| h.eq_ignore_ascii_case(name)),
            Side::Ip { addr, bits } => ip.is_some_and(|ip| ip_contains(*addr, *bits, ip)),
            Side::IpList(name) => ep.ip_lists.iter().any(|l| l.eq_ignore_ascii_case(name)),
        }
    }

    /// workload 5, a host address 4, labels 2 + one per key, ip_list 2, a
    /// wider CIDR 1, any 0.
    pub fn specificity(&self) -> u32 {
        match self {
            Side::Any => 0,
            Side::Labels(l) => 2 + l.len() as u32,
            Side::Workload(_) => 5,
            Side::Ip { addr, bits } if *bits == max_bits(*addr) => 4,
            Side::Ip { .. } => 1,
            Side::IpList(_) => 2,
        }
    }
}

/// `app=shop env=prod|dr`, `wl:web-01`, `ip:10.0.0.0/8`, `ipl:rfc1918` or
/// `any`: the one text form of a side (the TUI form, the CLI, `:marks`).
pub fn side_text(s: &Side) -> String {
    match s {
        Side::Any => "any".into(),
        Side::Labels(l) => l
            .iter()
            .map(|(k, vs)| {
                let vs: Vec<String> = vs.iter().map(|v| quote(v)).collect();
                format!("{}={}", quote(k), vs.join("|"))
            })
            .collect::<Vec<_>>()
            .join(" "),
        Side::Workload(w) => format!("wl:{w}"),
        Side::Ip { addr, bits } if *bits == max_bits(*addr) => format!("ip:{addr}"),
        Side::Ip { addr, bits } => format!("ip:{addr}/{bits}"),
        Side::IpList(l) => format!("ipl:{l}"),
    }
}

/// A label key or value as the side grammar writes it: quoted when it
/// holds a separator.
fn quote(s: &str) -> String {
    if s.is_empty() || s.contains([' ', ',', '|', '=', '"', '\\']) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Splits `s` at spaces and commas outside double quotes; quotes are
/// removed and `\"` inside them is a quote.
fn tokens(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => match chars.next() {
                Some(n) => cur.push(n),
                None => return Err("a `\\` ends the text".into()),
            },
            ' ' | ',' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            // `|` inside quotes is part of the value: mark it so the value
            // split below leaves it alone.
            '|' if quoted => cur.push('\u{1}'),
            c => cur.push(c),
        }
    }
    if quoted {
        return Err("an unclosed quote".into());
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// `addr` or `addr/N`, N checked against the family.
fn parse_net(s: &str) -> Result<(IpAddr, u8), String> {
    if s.contains('/') {
        return crate::write::specs::parse_cidr(s);
    }
    let addr: IpAddr = s
        .parse()
        .map_err(|_| format!("{s:?} is not an IP address"))?;
    Ok((addr, max_bits(addr)))
}

/// The side grammar of [`side_text`]. Tokens are separated by spaces or
/// commas; `|` within a value is OR; quote values with spaces.
pub fn parse_side(s: &str) -> Result<Side, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("empty side: write any, key=value, wl:NAME, ip:CIDR or ipl:NAME".into());
    }
    if t.eq_ignore_ascii_case("any") {
        return Ok(Side::Any);
    }
    let prefixed = |p: &str| {
        t.get(..p.len())
            .filter(|h| h.eq_ignore_ascii_case(p))
            .map(|_| t[p.len()..].trim())
    };
    if let Some(name) = prefixed("wl:") {
        if name.is_empty() {
            return Err("wl: needs a workload name".into());
        }
        return Ok(Side::Workload(name.to_string()));
    }
    if let Some(net) = prefixed("ipl:") {
        if net.is_empty() {
            return Err("ipl: needs an IP list name".into());
        }
        return Ok(Side::IpList(net.to_string()));
    }
    if let Some(net) = prefixed("ip:") {
        let (addr, bits) = parse_net(net)?;
        return Ok(Side::Ip { addr, bits });
    }
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tok in tokens(t)? {
        let (k, v) = tok
            .split_once('=')
            .ok_or_else(|| format!("{tok:?}: write key=value, wl:, ip:, ipl: or any"))?;
        let k = k.trim().replace('\u{1}', "|");
        if k.is_empty() {
            return Err(format!("{tok:?}: missing label key"));
        }
        let values: Vec<String> = v.split('|').map(|x| x.replace('\u{1}', "|")).collect();
        if values.iter().any(|x| x.is_empty()) {
            return Err(format!("{k}: empty label value"));
        }
        if labels.contains_key(&k) {
            return Err(format!("{k}: key given twice (write {k}=a|b for either)"));
        }
        labels.insert(k, values);
    }
    if labels.is_empty() {
        return Err("empty side".into());
    }
    Ok(Side::Labels(labels))
}

fn endpoint_ip(ep: &ResolvedEndpoint) -> Option<IpAddr> {
    ep.ip.as_deref().and_then(|s| s.parse().ok())
}

/// What matching needs of one flow, computed once for all marks: the
/// protocol number (without allocating, as `resolve_flows` writes it) and
/// the parsed addresses.
struct Probe<'f> {
    f: &'f ResolvedFlow,
    proto: Option<u8>,
    src_ip: Option<IpAddr>,
    dst_ip: Option<IpAddr>,
}

impl<'f> Probe<'f> {
    fn new(f: &'f ResolvedFlow) -> Self {
        let proto = match f.proto.as_str() {
            "tcp" => Some(6),
            "udp" => Some(17),
            "icmp" => Some(1),
            "icmpv6" => Some(58),
            n => n.parse().ok(),
        };
        Probe {
            f,
            proto,
            src_ip: endpoint_ip(&f.src),
            dst_ip: endpoint_ip(&f.dst),
        }
    }
}

/// A mark's service: a protocol (any port), or a port or port range of
/// TCP or UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceSpec {
    Proto(u8),
    Ports { proto: u8, from: u16, to: u16 },
}

impl ServiceSpec {
    /// `5666/tcp`, `8000-8010/tcp`, `udp` (any port), or a protocol name
    /// or number `proto_number` knows (`icmp`, `47`).
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        let Some((ports, proto_s)) = t.split_once('/') else {
            let proto = proto_number(t).ok_or_else(|| format!("{t:?}: unknown protocol"))?;
            return Ok(ServiceSpec::Proto(proto));
        };
        let proto = proto_number(proto_s)
            .ok_or_else(|| format!("{t:?}: unknown protocol {:?}", proto_s.trim()))?;
        if !matches!(proto, 6 | 17) {
            return Err(format!(
                "{t:?}: {} has no ports",
                proto_name(i64::from(proto))
            ));
        }
        let port = |p: &str| {
            p.trim()
                .parse::<u16>()
                .map_err(|_| format!("{t:?}: {:?} is not a port (0-65535)", p.trim()))
        };
        let (from, to) = match ports.split_once('-') {
            Some((a, b)) => (port(a)?, port(b)?),
            None => {
                let p = port(ports)?;
                (p, p)
            }
        };
        if from > to {
            return Err(format!("{t:?}: the range ends before it starts"));
        }
        Ok(ServiceSpec::Ports { proto, from, to })
    }

    /// The text `parse` reads back.
    pub fn text(&self) -> String {
        match *self {
            ServiceSpec::Proto(p) => proto_name(i64::from(p)),
            ServiceSpec::Ports { proto, from, to } if from == to => {
                format!("{from}/{}", proto_name(i64::from(proto)))
            }
            ServiceSpec::Ports { proto, from, to } => {
                format!("{from}-{to}/{}", proto_name(i64::from(proto)))
            }
        }
    }

    pub fn matches(&self, proto: Option<u8>, port: Option<u16>) -> bool {
        match *self {
            ServiceSpec::Proto(p) => proto == Some(p),
            ServiceSpec::Ports { proto: p, from, to } => {
                proto == Some(p) && port.is_some_and(|x| (from..=to).contains(&x))
            }
        }
    }

    /// One port 3, a range 2, a protocol 1.
    pub fn specificity(&self) -> u32 {
        match *self {
            ServiceSpec::Proto(_) => 1,
            ServiceSpec::Ports { from, to, .. } if from == to => 3,
            ServiceSpec::Ports { .. } => 2,
        }
    }
}

/// `allowed`, `potentially_blocked` or `blocked`.
pub fn parse_decision(s: &str) -> Result<PolicyDecision, String> {
    PolicyDecision::parse(s).ok_or_else(|| {
        format!(
            "unknown decision {:?}: allowed potentially_blocked blocked",
            s.trim()
        )
    })
}

/// The PCE a mark is limited to; `org` only with a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PceScope {
    pub host: String,
    pub org: Option<u64>,
}

/// The PCE of the profile in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PceId {
    pub host: String,
    pub org: u64,
}

impl PceScope {
    pub fn covers(&self, pce: &PceId) -> bool {
        self.host.eq_ignore_ascii_case(&pce.host) && self.org.is_none_or(|o| o == pce.org)
    }
}

/// One `[[mark]]`, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub id: String,
    pub action: Action,
    pub src: Side,
    pub dst: Side,
    /// Empty: any service.
    pub services: Vec<ServiceSpec>,
    /// Matched against the reported decision; empty: any.
    pub decisions: Vec<PolicyDecision>,
    /// Matched against the draft decision (its class); empty: any. A mark
    /// with one is active only while draft decisions are loaded.
    pub draft_decisions: Vec<PolicyDecision>,
    pub scope: Option<PceScope>,
    pub expires: Option<Timestamp>,
    /// Why the flows were marked; empty only for a hand-written mark
    /// without one (it loads with a warning).
    pub comment: String,
    pub created: Option<Timestamp>,
    pub created_on: Option<String>,
}

impl Mark {
    /// Directional: `src` against the flow's source, `dst` against its
    /// destination. Every field given must match.
    pub fn matches(&self, f: &ResolvedFlow) -> bool {
        self.matches_probe(&Probe::new(f))
    }

    /// `matches` with the cheap fields first and the flow's protocol and
    /// addresses computed once.
    fn matches_probe(&self, p: &Probe) -> bool {
        let f = p.f;
        (self.decisions.is_empty() || self.decisions.contains(&f.policy_decision))
            && (self.draft_decisions.is_empty()
                || f.draft_policy_decision
                    .as_ref()
                    .is_some_and(|d| self.draft_decisions.contains(&d.class())))
            && (self.services.is_empty()
                || self.services.iter().any(|s| s.matches(p.proto, f.port)))
            && self.src.matches_at(&f.src, p.src_ip)
            && self.dst.matches_at(&f.dst, p.dst_ip)
    }

    /// Sides, the most specific service, 1 each for `decision`,
    /// `draft_decision` and `pce`: which of several marks of one action is
    /// reported for a flow.
    pub fn specificity(&self) -> u32 {
        self.src.specificity()
            + self.dst.specificity()
            + self
                .services
                .iter()
                .map(ServiceSpec::specificity)
                .max()
                .unwrap_or(0)
            + u32::from(!self.decisions.is_empty())
            + u32::from(!self.draft_decisions.is_empty())
            + u32::from(self.scope.is_some())
    }

    /// `now == expires` is expired.
    pub fn is_expired(&self, now: Timestamp) -> bool {
        self.expires.is_some_and(|e| now >= e)
    }

    /// Portable marks are in scope everywhere; a scoped one on its PCE.
    /// Without a PCE (`marks list` with no profile) scope is not judged.
    pub fn in_scope(&self, pce: Option<&PceId>) -> bool {
        match (&self.scope, pce) {
            (None, _) | (_, None) => true,
            (Some(s), Some(p)) => s.covers(p),
        }
    }

    /// `5666/tcp`, `443/tcp, udp`; `None` for any service.
    pub fn service_text(&self) -> Option<String> {
        (!self.services.is_empty()).then(|| {
            self.services
                .iter()
                .map(ServiceSpec::text)
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    /// `app=crm → app=shop env=prod 443/tcp`.
    pub fn match_text(&self) -> String {
        let mut s = format!("{} → {}", side_text(&self.src), side_text(&self.dst));
        if let Some(svc) = self.service_text() {
            s.push(' ');
            s.push_str(&svc);
        }
        let ds = |d: &[PolicyDecision]| d.iter().map(|d| d.as_str()).collect::<Vec<_>>().join("|");
        if !self.decisions.is_empty() {
            s.push_str(&format!(" {}", ds(&self.decisions)));
        }
        if !self.draft_decisions.is_empty() {
            s.push_str(&format!(" draft {}", ds(&self.draft_decisions)));
        }
        s
    }

    /// Everything that identifies the mark, without its id and creation:
    /// what new and derived ids hash.
    pub fn canonical(&self) -> String {
        let ds = |d: &[PolicyDecision]| d.iter().map(|d| d.as_str()).collect::<Vec<_>>().join("|");
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.action.as_str(),
            side_text(&self.src),
            side_text(&self.dst),
            self.service_text().unwrap_or_default(),
            ds(&self.decisions),
            ds(&self.draft_decisions),
            self.scope
                .as_ref()
                .map(|s| format!("{}/{}", s.host.to_ascii_lowercase(), s.org.unwrap_or(0)))
                .unwrap_or_default(),
            self.expires.map(|e| e.to_string()).unwrap_or_default(),
            self.comment
        )
    }

    /// The rules a mark must keep beyond its fields' own syntax.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let workload = |s: &Side| matches!(s, Side::Workload(_));
        if self.scope.is_none() && (workload(&self.src) || workload(&self.dst)) {
            out.push("workload needs pce (workload marks are PCE-scoped)".into());
        }
        if self.action == Action::AcceptBlocked {
            if self.decisions.is_empty() {
                out.push("accept-blocked needs decision: blocked or potentially_blocked".into());
            } else if self.decisions.contains(&PolicyDecision::Allowed) {
                out.push("accept-blocked cannot include allowed".into());
            }
        }
        out
    }
}

/// The marks of one file, validated, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkSet {
    pub marks: Vec<Mark>,
}

/// The winning active mark of one flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub action: Action,
    /// Index into `MarkSet::marks`.
    pub mark: usize,
}

/// The marks in force for one result: in scope, unexpired, and (for a
/// `draft_decision` mark) with draft decisions loaded. Grouped by action,
/// strongest first, each group in report order (specificity, then the
/// earlier `created`, then the lower id).
#[derive(Debug, Clone)]
pub struct ActiveMarks<'a> {
    set: &'a MarkSet,
    groups: [Vec<usize>; 6],
    waiting: usize,
}

impl MarkSet {
    /// The marks active on `pce` at `now`; `draft`: draft decisions are
    /// loaded for the result they are matched against.
    pub fn active(&self, pce: Option<&PceId>, now: Timestamp, draft: bool) -> ActiveMarks<'_> {
        let mut groups: [Vec<usize>; 6] = Default::default();
        let mut waiting = 0;
        for (i, m) in self.marks.iter().enumerate() {
            if m.is_expired(now) || !m.in_scope(pce) {
                continue;
            }
            if !draft && !m.draft_decisions.is_empty() {
                waiting += 1;
                continue;
            }
            groups[m.action.index()].push(i);
        }
        for g in &mut groups {
            g.sort_by(|&a, &b| {
                let (a, b) = (&self.marks[a], &self.marks[b]);
                b.specificity()
                    .cmp(&a.specificity())
                    .then_with(|| a.created.is_none().cmp(&b.created.is_none()))
                    .then_with(|| a.created.cmp(&b.created))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        ActiveMarks {
            set: self,
            groups,
            waiting,
        }
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.marks.iter().position(|m| m.id == id)
    }
}

impl ActiveMarks<'_> {
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(Vec::is_empty)
    }

    /// How many marks are active.
    pub fn len(&self) -> usize {
        self.groups.iter().map(Vec::len).sum()
    }

    /// Marks with `draft_decision` left out because no draft decisions
    /// are loaded: `N marks wait for the draft decision (D)`.
    pub fn waiting_for_draft(&self) -> usize {
        self.waiting
    }

    pub fn set(&self) -> &MarkSet {
        self.set
    }

    pub fn mark(&self, i: usize) -> &Mark {
        &self.set.marks[i]
    }

    pub fn is_active(&self, i: usize) -> bool {
        self.set
            .marks
            .get(i)
            .is_some_and(|m| self.groups[m.action.index()].contains(&i))
    }

    /// Whether some active mark has `action`.
    pub fn has(&self, action: Action) -> bool {
        !self.groups[action.index()].is_empty()
    }

    /// The position of mark `i` in its action's report order (lower wins).
    pub fn rank(&self, i: usize) -> usize {
        let a = self.set.marks[i].action.index();
        self.groups[a]
            .iter()
            .position(|&x| x == i)
            .unwrap_or(usize::MAX)
    }

    /// The strongest action among the active marks that match, and its
    /// most specific mark. Stops at the first action with a match.
    pub fn classify(&self, f: &ResolvedFlow) -> Option<Hit> {
        let p = Probe::new(f);
        for (a, group) in Action::ALL.into_iter().zip(&self.groups) {
            if let Some(&i) = group.iter().find(|&&i| self.set.marks[i].matches_probe(&p)) {
                return Some(Hit { action: a, mark: i });
            }
        }
        None
    }

    /// Every active mark that matches, the winner first.
    pub fn matching(&self, f: &ResolvedFlow) -> Vec<usize> {
        let p = Probe::new(f);
        self.groups
            .iter()
            .flatten()
            .copied()
            .filter(|&i| self.set.marks[i].matches_probe(&p))
            .collect()
    }
}

/// FNV-1a, 64 bits.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Crockford's base32, lower case.
const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The low 30 bits of `h` as 6 base32 characters.
fn base32_6(h: u64) -> String {
    (0..6)
        .rev()
        .map(|i| CROCKFORD[((h >> (i * 5)) & 31) as usize] as char)
        .collect()
}

/// A new id, `m-` and 6 characters, from the creation time, the process
/// and the mark's canonical text; hashed again with a counter while
/// `taken` says the id is in the file.
pub fn new_id(now_ns: i128, pid: u32, canonical: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut counter: u32 = 0;
    loop {
        let mut bytes = Vec::with_capacity(canonical.len() + 24);
        bytes.extend_from_slice(&now_ns.to_le_bytes());
        bytes.extend_from_slice(&pid.to_le_bytes());
        bytes.extend_from_slice(canonical.as_bytes());
        if counter > 0 {
            bytes.extend_from_slice(&counter.to_le_bytes());
        }
        let id = format!("m-{}", base32_6(fnv1a(&bytes)));
        if !taken(&id) {
            return id;
        }
        counter += 1;
    }
}

/// The id of a hand-written mark without one: `h-` and 6 characters of
/// the hash of its canonical text, the same on every load and machine.
pub fn derived_id(canonical: &str) -> String {
    format!("h-{}", base32_6(fnv1a(canonical.as_bytes())))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::DraftDecision;
    use crate::resolve::ResolvedLabel;

    pub(crate) fn wl(host: &str, ip: &str, labels: &[(&str, &str)]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(ip.into()),
            hostname: Some(host.into()),
            workload_href: Some(format!("/orgs/1/workloads/{host}")),
            labels: labels
                .iter()
                .map(|(k, v)| ResolvedLabel {
                    href: format!("/orgs/1/labels/{k}-{v}"),
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            ..Default::default()
        }
    }

    pub(crate) fn addr(ip: &str, lists: &[&str]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(ip.into()),
            ip_lists: lists.iter().map(|l| (*l).into()).collect(),
            ..Default::default()
        }
    }

    pub(crate) fn flow(
        src: ResolvedEndpoint,
        dst: ResolvedEndpoint,
        port: Option<u16>,
        proto: &str,
        d: PolicyDecision,
    ) -> ResolvedFlow {
        ResolvedFlow {
            src,
            dst,
            port,
            proto: proto.into(),
            process_name: None,
            user_name: None,
            num_connections: 1,
            policy_decision: d,
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: None,
            last_detected: None,
            rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        }
    }

    /// A portable mark `src → dst` with `action`, no service, no expiry.
    pub(crate) fn mark(id: &str, action: Action, src: &str, dst: &str) -> Mark {
        Mark {
            id: id.into(),
            action,
            src: parse_side(src).unwrap(),
            dst: parse_side(dst).unwrap(),
            services: Vec::new(),
            decisions: Vec::new(),
            draft_decisions: Vec::new(),
            scope: None,
            expires: None,
            comment: "test".into(),
            created: None,
            created_on: None,
        }
    }

    fn web() -> ResolvedEndpoint {
        wl(
            "web-01",
            "10.0.1.10",
            &[("app", "shop"), ("env", "prod"), ("role", "web")],
        )
    }

    fn db() -> ResolvedEndpoint {
        wl(
            "db-01",
            "10.0.2.20",
            &[("app", "shop"), ("env", "prod"), ("role", "db")],
        )
    }

    fn now() -> Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn every_side_kind_matches_in_one_direction_only() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Allowed);
        for (src, dst) in [
            ("role=web", "role=db"),
            ("wl:WEB-01", "wl:db-01"),
            ("ip:10.0.1.10", "ip:10.0.0.0/16"),
            ("any", "role=db env=prod|dr"),
        ] {
            let m = mark("m-1", Action::Reviewed, src, dst);
            assert!(m.matches(&f), "{src} → {dst}");
            let back = mark("m-2", Action::Reviewed, dst, src);
            assert!(!back.matches(&f), "{dst} → {src} must not match a → b");
        }
        let corp = flow(
            addr("10.1.4.9", &["Corp"]),
            web(),
            Some(8443),
            "tcp",
            PolicyDecision::Blocked,
        );
        assert!(mark("m-3", Action::Squelch, "ipl:corp", "any").matches(&corp));
        assert!(!mark("m-4", Action::Squelch, "ipl:other", "any").matches(&corp));
    }

    #[test]
    fn labels_need_every_key_on_a_workload_and_never_match_unresolved_ones() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Allowed);
        let m = |src: &str| mark("m-1", Action::Reviewed, src, "any");
        assert!(m("app=shop env=prod").matches(&f));
        assert!(m("app=crm|shop").matches(&f), "a list is OR");
        assert!(!m("app=shop loc=eu").matches(&f), "a missing key");
        assert!(!m("app=Shop").matches(&f), "values are case-sensitive");
        let mut unresolved = web();
        unresolved.labels[0].key = "?".into();
        let g = flow(unresolved, db(), None, "icmp", PolicyDecision::Allowed);
        assert!(!m("?=shop").matches(&g));
        // Labels on a non-workload endpoint never match.
        let mut ep = addr("10.9.9.9", &[]);
        ep.labels = web().labels;
        let h = flow(ep, db(), None, "icmp", PolicyDecision::Allowed);
        assert!(!m("app=shop").matches(&h));
    }

    #[test]
    fn cidrs_contain_their_own_family_only() {
        let v6 = flow(
            addr("2001:db8::5", &[]),
            db(),
            Some(443),
            "tcp",
            PolicyDecision::Allowed,
        );
        let m = |src: &str| mark("m-1", Action::Reviewed, src, "any");
        assert!(m("ip:2001:db8::/32").matches(&v6));
        assert!(m("ip:2001:db8::5").matches(&v6), "a host address");
        assert!(!m("ip:2001:db8::6").matches(&v6));
        assert!(
            !m("ip:0.0.0.0/0").matches(&v6),
            "a v4 net never holds a v6 address"
        );
        assert!(m("ip:::/0").matches(&v6));
    }

    #[test]
    fn services_match_ports_ranges_protocols_and_lists() {
        let at = |port: Option<u16>, proto: &str| {
            flow(web(), db(), port, proto, PolicyDecision::Allowed)
        };
        let with = |svc: &[&str]| {
            let mut m = mark("m-1", Action::Reviewed, "any", "any");
            m.services = svc.iter().map(|s| ServiceSpec::parse(s).unwrap()).collect();
            m
        };
        assert!(with(&["5432/tcp"]).matches(&at(Some(5432), "tcp")));
        assert!(!with(&["5432/udp"]).matches(&at(Some(5432), "tcp")));
        let range = with(&["8000-8010/tcp"]);
        assert!(range.matches(&at(Some(8000), "tcp")) && range.matches(&at(Some(8010), "tcp")));
        assert!(!range.matches(&at(Some(7999), "tcp")) && !range.matches(&at(Some(8011), "tcp")));
        assert!(with(&["udp"]).matches(&at(Some(53), "udp")));
        assert!(with(&["icmp"]).matches(&at(None, "icmp")));
        assert!(with(&["47"]).matches(&at(None, "47")));
        let list = with(&["443/tcp", "icmp"]);
        assert!(list.matches(&at(None, "icmp")) && list.matches(&at(Some(443), "tcp")));
        assert!(!list.matches(&at(Some(80), "tcp")));
    }

    #[test]
    fn service_specs_parse_and_print_back() {
        for s in ["5666/tcp", "8000-8010/tcp", "udp", "icmp", "47"] {
            assert_eq!(ServiceSpec::parse(s).unwrap().text(), s);
        }
        assert_eq!(ServiceSpec::parse("80-80/TCP").unwrap().text(), "80/tcp");
        for (bad, why) in [
            ("70000/tcp", "not a port"),
            ("90-80/tcp", "ends before"),
            ("80/icmp", "has no ports"),
            ("80/bogus", "unknown protocol"),
            ("bogus", "unknown protocol"),
            ("x/tcp", "not a port"),
        ] {
            let e = ServiceSpec::parse(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn decisions_match_the_reported_one_and_draft_decisions_the_draft_class() {
        let mut f = flow(web(), db(), None, "icmp", PolicyDecision::Blocked);
        let mut m = mark("m-1", Action::Reviewed, "any", "any");
        m.decisions = vec![PolicyDecision::Blocked, PolicyDecision::PotentiallyBlocked];
        assert!(m.matches(&f));
        m.decisions = vec![PolicyDecision::Allowed];
        assert!(!m.matches(&f));
        m.decisions = vec![PolicyDecision::Blocked];
        m.draft_decisions = vec![PolicyDecision::Allowed];
        assert!(!m.matches(&f), "no draft decision loaded");
        f.draft_policy_decision = Some(DraftDecision::Other("allowed_across_boundary".into()));
        assert!(m.matches(&f), "the draft class is allowed");
        f.draft_policy_decision = Some(DraftDecision::Blocked);
        assert!(!m.matches(&f));
    }

    #[test]
    fn the_strongest_action_wins_whatever_the_specificity() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Blocked);
        let mut broad_squelch = mark("m-s", Action::Squelch, "any", "any");
        broad_squelch.services = vec![ServiceSpec::parse("5432/tcp").unwrap()];
        let set = MarkSet {
            marks: vec![
                broad_squelch,
                mark("m-r", Action::Reviewed, "wl:web-01", "wl:db-01"),
                mark("m-c", Action::Critical, "any", "any"),
            ],
        };
        let mut set = set;
        for m in &mut set.marks {
            m.scope = Some(PceScope {
                host: "pce.example".into(),
                org: None,
            });
        }
        let pce = PceId {
            host: "PCE.example".into(),
            org: 1,
        };
        let a = set.active(Some(&pce), now(), false);
        assert_eq!(
            a.classify(&f),
            Some(Hit {
                action: Action::Critical,
                mark: 2
            })
        );
        assert_eq!(a.matching(&f), vec![2, 1, 0], "the winner first");
        // Every pair of actions: the stronger one wins.
        for (i, strong) in Action::ALL.into_iter().enumerate() {
            for weak in &Action::ALL[i + 1..] {
                let set = MarkSet {
                    marks: vec![
                        mark("m-w", *weak, "wl:web-01", "role=db"),
                        mark("m-s", strong, "any", "any"),
                    ],
                };
                let mut set = set;
                set.marks[0].scope = Some(PceScope {
                    host: "pce.example".into(),
                    org: None,
                });
                let hit = set.active(Some(&pce), now(), false).classify(&f).unwrap();
                assert_eq!(hit.action, strong, "{strong:?} over {weak:?}");
            }
        }
    }

    #[test]
    fn within_one_action_the_most_specific_then_the_oldest_then_the_lowest_id_is_reported() {
        let f = flow(web(), db(), Some(5432), "tcp", PolicyDecision::Allowed);
        let t = |s: &str| Some(s.parse::<Timestamp>().unwrap());
        let mut a = mark("m-b", Action::Reviewed, "any", "any");
        let mut b = mark("m-a", Action::Reviewed, "any", "any");
        let c = mark("m-c", Action::Reviewed, "app=shop", "any");
        a.created = t("2026-01-01T00:00:00Z");
        b.created = t("2026-02-01T00:00:00Z");
        let set = MarkSet {
            marks: vec![a.clone(), b.clone(), c],
        };
        let hit = |s: &MarkSet| s.active(None, now(), false).classify(&f).unwrap().mark;
        assert_eq!(hit(&set), 2, "labels beat any");
        let set = MarkSet {
            marks: vec![b.clone(), a.clone()],
        };
        assert_eq!(hit(&set), 1, "the earlier created");
        b.created = a.created;
        let set = MarkSet {
            marks: vec![a.clone(), b.clone()],
        };
        assert_eq!(hit(&set), 1, "then the lower id (m-a)");
        b.created = None;
        let set = MarkSet { marks: vec![b, a] };
        assert_eq!(hit(&set), 1, "a mark without created comes last");
    }

    #[test]
    fn specificity_scores_sides_services_and_fields() {
        let mut m = mark("m-1", Action::Reviewed, "wl:web-01", "ip:10.0.0.5");
        assert_eq!(m.specificity(), 9);
        m.dst = parse_side("ip:10.0.0.0/8").unwrap();
        m.services = vec![
            ServiceSpec::parse("udp").unwrap(),
            ServiceSpec::parse("80/tcp").unwrap(),
        ];
        m.decisions = vec![PolicyDecision::Blocked];
        m.scope = Some(PceScope {
            host: "h".into(),
            org: None,
        });
        assert_eq!(m.specificity(), 5 + 1 + 3 + 1 + 1);
        assert_eq!(parse_side("app=a env=b").unwrap().specificity(), 4);
        assert_eq!(parse_side("ipl:x").unwrap().specificity(), 2);
        assert_eq!(Side::Any.specificity(), 0);
    }

    #[test]
    fn expiry_is_exclusive_and_scope_follows_host_and_org() {
        let mut m = mark("m-1", Action::Squelch, "any", "any");
        m.expires = Some(now());
        assert!(m.is_expired(now()), "now == expires is expired");
        assert!(!m.is_expired(now() - jiff::SignedDuration::from_secs(1)));
        let pce = |host: &str, org: u64| PceId {
            host: host.into(),
            org,
        };
        assert!(m.in_scope(Some(&pce("any", 1))), "portable");
        m.scope = Some(PceScope {
            host: "pce.example.com".into(),
            org: None,
        });
        assert!(m.in_scope(Some(&pce("PCE.Example.com", 7))));
        assert!(!m.in_scope(Some(&pce("other", 7))));
        m.scope = Some(PceScope {
            host: "pce.example.com".into(),
            org: Some(1),
        });
        assert!(m.in_scope(Some(&pce("pce.example.com", 1))));
        assert!(!m.in_scope(Some(&pce("pce.example.com", 2))));
        let set = MarkSet { marks: vec![m] };
        assert!(set.active(Some(&pce("other", 1)), now(), false).is_empty());
        assert_eq!(
            set.active(Some(&pce("pce.example.com", 1)), now(), false)
                .len(),
            0,
            "expired"
        );
    }

    #[test]
    fn draft_decision_marks_wait_for_draft_decisions() {
        let mut m = mark("m-1", Action::Revisit, "any", "any");
        m.decisions = vec![PolicyDecision::Blocked];
        m.draft_decisions = vec![PolicyDecision::Allowed];
        let set = MarkSet { marks: vec![m] };
        let off = set.active(None, now(), false);
        assert!(off.is_empty());
        assert_eq!(off.waiting_for_draft(), 1);
        let on = set.active(None, now(), true);
        assert_eq!((on.len(), on.waiting_for_draft()), (1, 0));
    }

    #[test]
    fn the_side_grammar_round_trips() {
        for s in [
            "any",
            "app=shop env=dr|prod",
            "wl:web-01",
            "ip:10.0.0.0/8",
            "ip:10.0.0.5",
            "ip:2001:db8::/32",
            "ipl:rfc1918",
            "app=\"my app\" env=prod",
        ] {
            assert_eq!(side_text(&parse_side(s).unwrap()), s, "{s}");
        }
        assert_eq!(
            parse_side("app=shop,env=prod").unwrap(),
            parse_side("env=prod  app=shop").unwrap()
        );
        assert_eq!(
            parse_side("ip:10.0.0.5").unwrap(),
            Side::Ip {
                addr: "10.0.0.5".parse().unwrap(),
                bits: 32
            }
        );
        let quoted = parse_side("app=\"a|b\"").unwrap();
        assert_eq!(
            quoted,
            Side::Labels(BTreeMap::from([("app".into(), vec!["a|b".into()])]))
        );
        for (bad, why) in [
            ("", "empty side"),
            ("app", "key=value"),
            ("=shop", "missing label key"),
            ("app=", "empty label value"),
            ("app=a|", "empty label value"),
            ("app=a app=b", "twice"),
            ("ip:10.0.0.0/33", "at most 32"),
            ("ip:nope", "not an IP"),
            ("wl:", "needs a workload"),
            ("app=\"open", "unclosed quote"),
        ] {
            let e = parse_side(bad).unwrap_err();
            assert!(e.contains(why), "{bad:?}: {e}");
        }
    }

    #[test]
    fn mark_problems_name_the_rules() {
        let mut m = mark("m-1", Action::Critical, "any", "wl:db-01");
        assert!(m.problems()[0].contains("workload needs pce"));
        m.scope = Some(PceScope {
            host: "h".into(),
            org: None,
        });
        assert!(m.problems().is_empty());
        let mut a = mark("m-2", Action::AcceptBlocked, "any", "any");
        assert!(a.problems()[0].contains("needs decision"));
        a.decisions = vec![PolicyDecision::Blocked, PolicyDecision::Allowed];
        assert!(a.problems()[0].contains("cannot include allowed"));
        a.decisions = vec![PolicyDecision::Blocked];
        assert!(a.problems().is_empty());
    }

    #[test]
    fn ids_are_stable_hashes_in_crockford_base32() {
        let m = mark("", Action::Squelch, "app=monitoring", "any");
        let c = m.canonical();
        let a = new_id(1, 2, &c, |_| false);
        assert_eq!(
            a,
            new_id(1, 2, &c, |_| false),
            "the same input, the same id"
        );
        assert!(a.starts_with("m-") && a.len() == 8, "{a}");
        assert!(a[2..].bytes().all(|b| CROCKFORD.contains(&b)), "{a}");
        let b = new_id(1, 2, &c, |id| id == a);
        assert_ne!(a, b, "a taken id is hashed again");
        let h = derived_id(&c);
        assert_eq!(h, derived_id(&m.clone().canonical()), "stable across loads");
        assert!(h.starts_with("h-") && h.len() == 8);
        let mut other = m.clone();
        other.comment = "another reason".into();
        assert_ne!(derived_id(&other.canonical()), h);
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn expiries_parse_in_utc_with_defaults_per_action() {
        let day = jiff::SignedDuration::from_hours(24);
        assert_eq!(parse_expiry("7d", now()).unwrap(), Some(now() + day * 7));
        assert_eq!(parse_expiry("never", now()).unwrap(), None);
        assert_eq!(
            parse_expiry("2027-01-15", now()).unwrap(),
            Some("2027-01-15T23:59:59Z".parse().unwrap())
        );
        assert_eq!(
            parse_expiry("2027-01-15T10:00:00+02:00", now()).unwrap(),
            Some("2027-01-15T08:00:00Z".parse().unwrap())
        );
        assert!(parse_expiry("0d", now()).is_err());
        assert!(
            parse_expiry("soon", now())
                .unwrap_err()
                .contains("YYYY-MM-DD")
        );
        assert_eq!(
            expiry_text(Some("2027-01-15T23:59:59Z".parse().unwrap())),
            "expires 2027-01-15 (UTC)"
        );
        assert_eq!(expiry_text(None), "never expires");
        let defaults: Vec<&str> = Action::ALL.map(Action::default_expiry).to_vec();
        assert_eq!(defaults, ["never", "never", "7d", "90d", "30d", "90d"]);
    }

    #[test]
    fn actions_parse_and_order_strongest_first() {
        assert_eq!(Action::parse("accept-blocked"), Some(Action::AcceptBlocked));
        assert_eq!(Action::parse("nope"), None);
        assert!(Action::Critical < Action::Squelch);
        assert_eq!(
            Action::names(),
            "critical important revisit accept-blocked reviewed squelch"
        );
        assert_eq!(
            serde_json::to_string(&Action::AcceptBlocked).unwrap(),
            "\"accept-blocked\""
        );
        assert!(
            parse_decision("bogus")
                .unwrap_err()
                .contains("allowed potentially_blocked blocked")
        );
    }
}
