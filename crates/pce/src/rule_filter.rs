//! Rule search: which loaded rules reference a label, an IP list, a
//! service, a workload, an href, and where; which are disabled, deny,
//! intra- or extra-scope rules. One grammar for the TUI's `f` on Policy
//! and the CLI's `policy search`:
//!
//! ```text
//! app=shop,crm ipl:Corp svc:443/tcp svc:PostgreSQL wl:web01 href:sec_rules/2 disabled deny
//! src:role=web dst:role=db extra        scope:app=payment -svc:443/tcp
//! src:!env=prod                         src:all -deny
//! ```
//!
//! Terms apart by spaces must all hold (AND). `key=v1,v2`: a label any of
//! whose values is in the rule's scope, consumers or providers, directly
//! or through a label group (sub-groups included); an exclusion actor
//! (`All except app=shop` in a scope, `!role=web` as a consumer) does not
//! count, and `ams` (All Workloads) names no label. `!key=v1,v2`: an
//! exclusion actor of such a label (or of a label group holding it), and
//! only those. `ipl:NAME`, `svc:NAME`: an IP list or service object by
//! name (any case; a unique prefix will do, see [`RuleFilter::resolve`]).
//! `svc:443/tcp` (or `svc:443`, tcp or udp): an inline port or range that
//! holds it, or a service object with such an entry (`All Services` and
//! protocol-only entries too). `svc:tcp`, `svc:udp`, `svc:icmp`,
//! `svc:icmpv6`: any entry of that protocol. Service objects'
//! `windows_services` (process and Windows service names) are not
//! modelled: only their port entries count. `wl:NAME`: a workload actor by
//! hostname or name (of the workloads the caller holds) or by its href's
//! id. `href:TEXT`: the rule's href contains it.
//!
//! Where an actor sits, a prefix says (none: anywhere): `scope:` the
//! ruleset's scopes (label terms, or `scope:global`: a scope without
//! labels, or none), `src:` (or `consumer:`) the consumers, `dst:` (or
//! `provider:`) the providers; `src:` and `dst:` also take `ipl:NAME`,
//! `wl:NAME` and `all` (All Workloads). A deny rule's scope is its
//! ruleset's.
//!
//! `disabled`: the rule or its ruleset is off; `enabled`: both are on.
//! `deny`: a deny or override deny rule; `allow`: an allow rule. `intra`,
//! `extra`: an allow rule whose consumers are inside its ruleset's scope,
//! or anywhere (`unscoped_consumers`); a deny rule is neither. A leading
//! `-` keeps the rules a term does not match (`-deny`, `-scope:env=dev`,
//! `-svc:443/tcp`): `-`, not `!`, which names an exclusion actor. A name
//! with spaces is quoted (`ipl:"Any (0.0.0.0/0 and ::/0)"`). Pure: the
//! caller brings the loaded rulesets and the name index.
use crate::impact::group_members;
use crate::label_filter::{LabelTerm, split_outside_quotes, strip_quotes};
use crate::models::{Actor, DenyRule, IngressService, Rule, RuleSet, Workload, proto_number};
use crate::resolve::{NameIndex, RuleKind};
use std::fmt;

/// A port a `svc:` term asks for: `443/tcp`, or `443` (tcp or udp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortTerm {
    pub port: u16,
    /// `None`: tcp or udp.
    pub proto: Option<u8>,
}

impl PortTerm {
    /// `443/tcp`, `53/udp`, `443`; `None` for anything else (a name).
    pub fn parse(text: &str) -> Option<Self> {
        let (port, proto) = match text.split_once('/') {
            Some((p, proto)) => (p, Some(proto_number(proto)?)),
            None => (text, None),
        };
        let port = port.trim().parse().ok()?;
        Some(Self { port, proto })
    }

    /// Whether an entry `port[-to_port]/proto` holds this port. Protocol
    /// `-1` (every service) holds every port; an entry without a port (all
    /// ports of its protocol) holds it when the protocol matches.
    fn held_by(&self, port: Option<u16>, to_port: Option<u16>, proto: i64) -> bool {
        if proto == -1 {
            return true;
        }
        let proto_ok = match self.proto {
            Some(p) => i64::from(p) == proto,
            None => proto == 6 || proto == 17,
        };
        let Some(from) = port else {
            return proto_ok;
        };
        let to = to_port.unwrap_or(from).max(from);
        proto_ok && (from..=to).contains(&self.port)
    }
}

impl fmt::Display for PortTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.proto {
            Some(p) => write!(
                f,
                "{}/{}",
                self.port,
                crate::models::proto_name(i64::from(p))
            ),
            None => write!(f, "{}", self.port),
        }
    }
}

/// Where a `scope:`, `src:` or `dst:` term looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// `scope:`: the ruleset's scopes.
    Scope,
    /// `src:` (`consumer:`).
    Consumers,
    /// `dst:` (`provider:`).
    Providers,
}

impl Position {
    /// The prefixes that name a side, aliases after the canonical one.
    const PREFIXES: [(&'static str, Position); 5] = [
        ("scope:", Position::Scope),
        ("src:", Position::Consumers),
        ("consumer:", Position::Consumers),
        ("dst:", Position::Providers),
        ("provider:", Position::Providers),
    ];

    /// The prefix the search writes: `scope:`, `src:`, `dst:`.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Scope => "scope:",
            Self::Consumers => "src:",
            Self::Providers => "dst:",
        }
    }

    /// What a term after this prefix may be.
    fn grammar(self) -> &'static str {
        match self {
            Self::Scope => "key=value[,value], !key=value or global",
            Self::Consumers | Self::Providers => {
                "key=value[,value], !key=value, ipl:NAME, wl:NAME or all"
            }
        }
    }
}

/// What a `scope:`, `src:` or `dst:` term looks for on its side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorTerm {
    /// As [`RuleTerm::Label`].
    Label(LabelTerm),
    IpList(String),
    Workload(String),
    /// `all`: All Workloads (`src:`, `dst:`).
    All,
    /// `global`: a scope without labels, or no scope (`scope:`).
    Global,
}

/// One search term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleTerm {
    /// `key=v1,v2`: any of the values; `!key=v1,v2`: an exclusion actor of
    /// any of them (never `key=-`).
    Label(LabelTerm),
    IpList(String),
    Service(String),
    Port(PortTerm),
    /// `svc:tcp`, `svc:udp`, `svc:icmp`, `svc:icmpv6`: any port of it.
    Proto(u8),
    Workload(String),
    Href(String),
    /// `scope:…`, `src:…`, `dst:…`: an actor term on one side only.
    At(Position, ActorTerm),
    /// `-term`: the rules the term does not match.
    Not(Box<RuleTerm>),
    Disabled,
    Enabled,
    Deny,
    Allow,
    Intra,
    Extra,
}

/// Why a term is not one: what the grammar takes.
const GRAMMAR: &str = "use [!]key=value[,value], ipl:NAME, svc:NAME, svc:PORT/PROTO or svc:tcp|udp|icmp, wl:NAME, href:TEXT, scope:|src:|dst: before an actor, disabled, enabled, deny, allow, intra, extra, or -term to negate";

/// The protocols `svc:` takes by name.
const PROTOS: [&str; 4] = ["tcp", "udp", "icmp", "icmpv6"];

/// The property words and their terms.
const WORDS: [(&str, RuleTerm); 6] = [
    ("disabled", RuleTerm::Disabled),
    ("enabled", RuleTerm::Enabled),
    ("deny", RuleTerm::Deny),
    ("allow", RuleTerm::Allow),
    ("intra", RuleTerm::Intra),
    ("extra", RuleTerm::Extra),
];

/// `t` after `prefix` (any case), else `None`.
fn after_prefix<'t>(t: &'t str, prefix: &str) -> Option<&'t str> {
    t.get(..prefix.len())
        .filter(|h| h.eq_ignore_ascii_case(prefix))
        .map(|_| &t[prefix.len()..])
}

/// The name after `prefix` in `t` (`rest`), unquoted; an error when none.
fn name_after(t: &str, prefix: &str, rest: &str) -> Result<String, String> {
    let name = strip_quotes(rest.trim())?;
    if name.trim().is_empty() {
        return Err(format!("{t:?}: {prefix} needs a name"));
    }
    Ok(name)
}

/// A label term as a rule search takes it: `key=v1,v2` or `!key=v1,v2`
/// (no `-`: a rule names labels, not their absence).
fn label_term(t: &str, text: &str) -> Result<LabelTerm, String> {
    let term = LabelTerm::parse(text)?;
    if term.none {
        return Err(format!(
            "{t:?}: a rule search takes key=value[,value] or !key=value (no key=-)"
        ));
    }
    Ok(term)
}

impl ActorTerm {
    /// The term after `side`'s prefix in `t` (`rest`).
    fn parse(t: &str, side: Position, rest: &str) -> Result<Self, String> {
        let r = rest.trim();
        let lower = r.to_ascii_lowercase();
        let either = side != Position::Scope;
        if either && lower == "all" {
            return Ok(Self::All);
        }
        if !either && lower == "global" {
            return Ok(Self::Global);
        }
        if either {
            for (prefix, make) in [
                ("ipl:", Self::IpList as fn(String) -> Self),
                ("wl:", Self::Workload),
            ] {
                if let Some(name) = after_prefix(r, prefix) {
                    return Ok(make(name_after(t, prefix, name)?));
                }
            }
        }
        if r.contains('=') {
            return Ok(Self::Label(label_term(t, r)?));
        }
        Err(format!("{t:?}: {} takes {}", side.prefix(), side.grammar()))
    }
}

impl fmt::Display for ActorTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Label(t) => write!(f, "{t}"),
            Self::IpList(n) => f.write_str(&prefixed("ipl:", n)),
            Self::Workload(n) => f.write_str(&prefixed("wl:", n)),
            Self::All => f.write_str("all"),
            Self::Global => f.write_str("global"),
        }
    }
}

impl RuleTerm {
    /// A `svc:` value (or the CLI's `--service`): a port (`443/tcp`,
    /// `443`), a protocol (`tcp`, `udp`, `icmp`, `icmpv6`), else a service
    /// object's name.
    pub fn service(text: &str) -> Self {
        if let Some(p) = PortTerm::parse(text) {
            return Self::Port(p);
        }
        let lower = text.trim().to_ascii_lowercase();
        match crate::models::proto_number(&lower) {
            Some(p) if PROTOS.contains(&lower.as_str()) => Self::Proto(p),
            _ => Self::Service(text.to_string()),
        }
    }

    /// One term (a token of an expression, or a CLI flag's value made one).
    /// A leading `-` negates it: `-deny`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        let Some(rest) = t.strip_prefix('-') else {
            return Self::parse_plain(t);
        };
        if rest.starts_with('-') {
            return Err(format!(
                "{t:?}: one - negates a term (-deny); a term cannot start with --"
            ));
        }
        if rest.trim().is_empty() {
            return Err(format!(
                "{t:?}: - needs a term right after it (-deny, -scope:env=dev)"
            ));
        }
        Ok(Self::Not(Box::new(Self::parse_plain(rest)?)))
    }

    /// A term without the leading `-`.
    fn parse_plain(t: &str) -> Result<Self, String> {
        let lower = t.to_ascii_lowercase();
        if let Some((_, term)) = WORDS.iter().find(|(w, _)| *w == lower) {
            return Ok(term.clone());
        }
        for (prefix, side) in Position::PREFIXES {
            if let Some(rest) = after_prefix(t, prefix) {
                return Ok(Self::At(side, ActorTerm::parse(t, side, rest)?));
            }
        }
        for (prefix, make) in [
            ("ipl:", Self::IpList as fn(String) -> Self),
            ("svc:", Self::Service),
            ("wl:", Self::Workload),
            ("href:", Self::Href),
        ] {
            let Some(rest) = after_prefix(t, prefix) else {
                continue;
            };
            let name = name_after(t, prefix, rest)?;
            if prefix == "svc:" {
                return Ok(Self::service(&name));
            }
            return Ok(make(name));
        }
        if t.contains('=') {
            return Ok(Self::Label(label_term(t, t)?));
        }
        Err(format!("{t:?}: unknown term; {GRAMMAR}"))
    }

    /// Whether the term names a workload (`wl:`, `src:wl:`, `-wl:`): the
    /// caller needs the workloads to match it.
    pub fn names_workload(&self) -> bool {
        match self {
            Self::Workload(_) | Self::At(_, ActorTerm::Workload(_)) => true,
            Self::Not(t) => t.names_workload(),
            _ => false,
        }
    }
}

/// `ipl:Corp`, `ipl:"Any (0.0.0.0/0 and ::/0)"`: quoted when it holds a
/// space, a quote or a backslash, as `parse` reads it back.
fn prefixed(prefix: &str, name: &str) -> String {
    if name.contains(char::is_whitespace) || name.contains(['"', '\\']) {
        format!(
            "{prefix}\"{}\"",
            name.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else {
        format!("{prefix}{name}")
    }
}

impl fmt::Display for RuleTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Label(t) => write!(f, "{t}"),
            Self::IpList(n) => f.write_str(&prefixed("ipl:", n)),
            Self::Service(n) => f.write_str(&prefixed("svc:", n)),
            Self::Port(p) => write!(f, "svc:{p}"),
            Self::Proto(p) => write!(f, "svc:{}", crate::models::proto_name(i64::from(*p))),
            Self::Workload(n) => f.write_str(&prefixed("wl:", n)),
            Self::Href(h) => f.write_str(&prefixed("href:", h)),
            Self::At(side, a) => write!(f, "{}{a}", side.prefix()),
            Self::Not(t) => write!(f, "-{t}"),
            word => {
                let (w, _) = WORDS
                    .iter()
                    .find(|(_, t)| t == word)
                    .expect("every other term is a word");
                f.write_str(w)
            }
        }
    }
}

/// Terms that must all hold; empty matches every rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleFilter {
    pub terms: Vec<RuleTerm>,
}

impl fmt::Display for RuleFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let terms: Vec<String> = self.terms.iter().map(ToString::to_string).collect();
        f.write_str(&terms.join(" "))
    }
}

/// One loaded rule, allow or deny, with what the search reads of its ruleset.
#[derive(Debug, Clone, Copy)]
pub struct RuleRef<'a> {
    pub href: &'a str,
    pub kind: RuleKind,
    pub enabled: bool,
    pub ruleset_enabled: bool,
    /// Extra-scope (`unscoped_consumers`); never on a deny rule.
    pub unscoped_consumers: bool,
    pub scopes: &'a [Vec<Actor>],
    pub providers: &'a [Actor],
    pub consumers: &'a [Actor],
    pub services: &'a [IngressService],
}

impl<'a> RuleRef<'a> {
    pub fn of_rule(r: &'a Rule, rs: &'a RuleSet) -> Self {
        Self {
            href: &r.href,
            kind: RuleKind::Allow,
            enabled: r.enabled,
            ruleset_enabled: rs.enabled,
            unscoped_consumers: r.unscoped_consumers.unwrap_or(false),
            scopes: &rs.scopes,
            providers: &r.providers,
            consumers: &r.consumers,
            services: &r.ingress_services,
        }
    }

    pub fn of_deny(d: &'a DenyRule, rs: &'a RuleSet) -> Self {
        Self {
            href: &d.href,
            kind: if d.override_ {
                RuleKind::OverrideDeny
            } else {
                RuleKind::Deny
            },
            enabled: d.enabled,
            ruleset_enabled: rs.enabled,
            unscoped_consumers: false,
            scopes: &rs.scopes,
            providers: &d.providers,
            consumers: &d.consumers,
            services: &d.ingress_services,
        }
    }

    /// Scope members, consumers and providers: every actor a term looks at.
    fn actors(&self) -> impl Iterator<Item = &'a Actor> {
        self.scopes
            .iter()
            .flatten()
            .chain(self.consumers)
            .chain(self.providers)
    }

    /// The actors on one side: scope members, consumers or providers.
    fn on(&self, side: Position) -> Box<dyn Iterator<Item = &'a Actor> + 'a> {
        match side {
            Position::Scope => Box::new(self.scopes.iter().flatten()),
            Position::Consumers => Box::new(self.consumers.iter()),
            Position::Providers => Box::new(self.providers.iter()),
        }
    }

    /// A global ruleset: no scope, or a scope without labels.
    fn global(&self) -> bool {
        self.scopes.is_empty()
            || self.scopes.iter().any(|s| {
                s.iter()
                    .all(|a| a.label.is_none() && a.label_group.is_none())
            })
    }
}

/// A ruleset's rules in evaluation order (override deny, allow, deny), its
/// deny rules from `deny` (the inline list, or the ones fetched for it).
pub fn rules_of<'a>(rs: &'a RuleSet, deny: &'a [DenyRule]) -> Vec<RuleRef<'a>> {
    let mut out: Vec<RuleRef<'a>> = rs
        .rules
        .iter()
        .map(|r| RuleRef::of_rule(r, rs))
        .chain(deny.iter().map(|d| RuleRef::of_deny(d, rs)))
        .collect();
    out.sort_by_key(|r| match r.kind {
        RuleKind::OverrideDeny => 0,
        RuleKind::Allow => 1,
        RuleKind::Deny => 2,
    });
    out
}

impl RuleFilter {
    /// Terms apart by whitespace (outside quotes).
    pub fn parse(expr: &str) -> Result<Self, String> {
        let terms = split_outside_quotes(expr, char::is_whitespace)?
            .into_iter()
            .filter(|t| !t.trim().is_empty())
            .map(RuleTerm::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { terms })
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The terms that name nothing the index knows, as warnings (`unknown
    /// key os`, `unknown value app=shpo`, `unknown IP list Crop (closest:
    /// Corp)`, `IP list A matches several: …`): such a term matches no
    /// rule. `wl:`, ports and protocols are not checked.
    pub fn unknown(&self, idx: &NameIndex) -> Vec<String> {
        self.resolve(idx).1
    }

    /// The search with each `ipl:` and `svc:` name made the object's own
    /// name ([`resolve_name`]: exact, else the one name it is a prefix of,
    /// any case), and the terms that name nothing or several things, as
    /// warnings (those terms are kept as typed: they match no rule).
    pub fn resolve(&self, idx: &NameIndex) -> (Self, Vec<String>) {
        let mut out = Vec::new();
        let mut terms = Vec::with_capacity(self.terms.len());
        let ip_lists = idx.ip_list_names();
        let services: Vec<String> = idx.services().iter().map(|s| s.name.clone()).collect();
        for t in &self.terms {
            let mut t = t.clone();
            resolve_term(&mut t, idx, &ip_lists, &services, &mut out);
            terms.push(t);
        }
        (Self { terms }, out)
    }

    /// Whether `r` holds every term. `workloads` (those the caller holds,
    /// possibly none) name the workload actors for `wl:`.
    pub fn matches(&self, r: &RuleRef, idx: &NameIndex, workloads: &[Workload]) -> bool {
        self.terms
            .iter()
            .all(|t| term_matches(t, r, idx, workloads))
    }

    /// Whether a term names a workload: matching needs the workloads.
    pub fn names_workloads(&self) -> bool {
        self.terms.iter().any(RuleTerm::names_workload)
    }
}

/// [`RuleFilter::resolve`] for one term (and the term a `-` negates).
fn resolve_term(
    t: &mut RuleTerm,
    idx: &NameIndex,
    ip_lists: &[String],
    services: &[String],
    out: &mut Vec<String>,
) {
    match t {
        RuleTerm::Label(l) | RuleTerm::At(_, ActorTerm::Label(l)) => {
            if !idx.label_keys().contains(&l.key) {
                out.push(format!("unknown key {}", l.key));
            } else {
                let known = idx.values_for_key(&l.key);
                out.extend(
                    l.values.iter().filter(|v| !known.contains(v)).map(|v| {
                        format!("unknown value {}={}", l.key, crate::label_filter::quote(v))
                    }),
                );
            }
        }
        RuleTerm::IpList(n) | RuleTerm::At(_, ActorTerm::IpList(n)) => {
            resolve_into(n, ip_lists, "IP list", out)
        }
        RuleTerm::Service(n) => resolve_into(n, services, "service", out),
        RuleTerm::Not(inner) => resolve_term(inner, idx, ip_lists, services, out),
        _ => {}
    }
}

fn term_matches(t: &RuleTerm, r: &RuleRef, idx: &NameIndex, workloads: &[Workload]) -> bool {
    let allow = r.kind == RuleKind::Allow;
    match t {
        RuleTerm::Not(t) => !term_matches(t, r, idx, workloads),
        RuleTerm::Disabled => !r.enabled || !r.ruleset_enabled,
        RuleTerm::Enabled => r.enabled && r.ruleset_enabled,
        RuleTerm::Deny => !allow,
        RuleTerm::Allow => allow,
        RuleTerm::Intra => allow && !r.unscoped_consumers,
        RuleTerm::Extra => allow && r.unscoped_consumers,
        RuleTerm::Href(h) => r.href.contains(h.as_str()),
        RuleTerm::Label(term) => r.actors().any(|a| label_hit(a, term, idx)),
        RuleTerm::IpList(name) => r.actors().any(|a| ip_list_hit(a, name, idx)),
        RuleTerm::Workload(name) => r.actors().any(|a| workload_hit(a, name, workloads)),
        RuleTerm::At(Position::Scope, ActorTerm::Global) => r.global(),
        RuleTerm::At(side, term) => r.on(*side).any(|a| match term {
            ActorTerm::Label(l) => label_hit(a, l, idx),
            ActorTerm::IpList(name) => ip_list_hit(a, name, idx),
            ActorTerm::Workload(name) => workload_hit(a, name, workloads),
            ActorTerm::All => a.actors.as_deref() == Some("ams"),
            ActorTerm::Global => false,
        }),
        RuleTerm::Service(name) => r.services.iter().any(|s| match s {
            IngressService::Ref(s) => idx
                .service_name(&s.href)
                .is_some_and(|n| n.eq_ignore_ascii_case(name)),
            IngressService::Port { .. } => false,
        }),
        RuleTerm::Proto(p) => {
            let is = |pr: i64| pr == -1 || pr == i64::from(*p);
            r.services.iter().any(|s| match s {
                IngressService::Port { proto, .. } => is(*proto),
                IngressService::Ref(s) => idx
                    .service(&s.href)
                    .is_some_and(|svc| svc.service_ports.iter().any(|e| e.proto.is_some_and(is))),
            })
        }
        RuleTerm::Port(p) => r.services.iter().any(|s| match s {
            IngressService::Port {
                port,
                to_port,
                proto,
            } => p.held_by(*port, *to_port, *proto),
            IngressService::Ref(s) => idx.service(&s.href).is_some_and(|svc| {
                svc.service_ports
                    .iter()
                    .any(|e| e.proto.is_some_and(|pr| p.held_by(e.port, e.to_port, pr)))
            }),
        }),
    }
}

/// An IP list actor of that name (any case).
fn ip_list_hit(a: &Actor, name: &str, idx: &NameIndex) -> bool {
    a.ip_list.as_ref().is_some_and(|l| {
        idx.ip_list_name(&l.href)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// A workload actor called `name` ([`workload_named`]).
fn workload_hit(a: &Actor, name: &str, workloads: &[Workload]) -> bool {
    a.workload
        .as_ref()
        .is_some_and(|w| workload_named(&w.href, name, workloads))
}

/// A label actor with the term's key and one of its values, or a label
/// group with such a label among its members (sub-groups included). An
/// exclusion actor (`All except app=shop`, `!role=web`) names what the
/// rule leaves out: a hit for a `!key=value` term only, and only it.
fn label_hit(a: &Actor, term: &LabelTerm, idx: &NameIndex) -> bool {
    if a.exclusion.unwrap_or(false) != term.negate {
        return false;
    }
    let is = |href: &str| {
        idx.label(href)
            .is_some_and(|l| l.key == term.key && term.values.contains(&l.value))
    };
    if let Some(l) = &a.label {
        return is(&l.href);
    }
    if let Some(g) = &a.label_group {
        return group_members(&g.href, idx).iter().any(|h| is(h));
    }
    false
}

/// How a typed name finds one of `names`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameMatch {
    /// The name itself (exact, else any case), or the only one the typed
    /// text is a prefix of (any case).
    One(String),
    /// Several names start with the typed text.
    Many(Vec<String>),
    /// None does; up to three names closest to it.
    None(Vec<String>),
}

/// `typed` among `names`: an exact match, else one in any case, else the
/// unique name it is a prefix of (any case); else the names it is a
/// prefix of, or the closest ones (those containing it, then by edit
/// distance).
pub fn resolve_name(typed: &str, names: &[String]) -> NameMatch {
    if let Some(n) = names.iter().find(|n| *n == typed) {
        return NameMatch::One(n.clone());
    }
    if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(typed)) {
        return NameMatch::One(n.clone());
    }
    let lower = typed.to_lowercase();
    let starts: Vec<String> = names
        .iter()
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .cloned()
        .collect();
    match starts.len() {
        1 => return NameMatch::One(starts[0].clone()),
        n if n > 1 => return NameMatch::Many(starts),
        _ => {}
    }
    let mut near: Vec<(usize, &String)> = names
        .iter()
        .map(|n| {
            let l = n.to_lowercase();
            let d = if l.contains(&lower) {
                0
            } else {
                1 + edit_distance(&l, &lower)
            };
            (d, n)
        })
        .collect();
    near.sort_by_key(|&(d, n)| (d, n.to_lowercase()));
    let limit = lower.chars().count().max(3);
    NameMatch::None(
        near.into_iter()
            .filter(|&(d, _)| d <= limit)
            .take(3)
            .map(|(_, n)| n.clone())
            .collect(),
    )
}

/// Levenshtein distance by chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `name` made the object's own name when it finds one; else a warning
/// (`unknown IP list Crop (closest: Corp)`, `IP list A matches several:
/// Any, Apps`) and `name` kept as typed.
fn resolve_into(name: &mut String, names: &[String], what: &str, out: &mut Vec<String>) {
    match resolve_name(name, names) {
        NameMatch::One(n) => *name = n,
        NameMatch::Many(ns) => {
            out.push(format!("{what} {name} matches several: {}", ns.join(", ")))
        }
        NameMatch::None(near) if near.is_empty() => out.push(format!("unknown {what} {name}")),
        NameMatch::None(near) => out.push(format!(
            "unknown {what} {name} (closest: {})",
            near.join(", ")
        )),
    }
}

/// The workload `href` is called `name`: its id, or the hostname or name
/// of the held workload with that href (any case).
fn workload_named(href: &str, name: &str, workloads: &[Workload]) -> bool {
    let id = href.rsplit('/').next().unwrap_or(href);
    id.eq_ignore_ascii_case(name)
        || workloads.iter().any(|w| {
            w.href == href
                && [w.hostname.as_deref(), w.name.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|n| n.eq_ignore_ascii_case(name))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, Service};
    use serde_json::json;

    fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let mut groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
        // A group whose sub-group holds role=db.
        groups.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/label_groups/12", "name": "Tiers", "key": "role",
                "labels": [], "sub_groups": [{"href": "/orgs/5/sec_policy/draft/label_groups/13"}]
            }))
            .unwrap(),
        );
        groups.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/label_groups/13", "name": "Db", "key": "role",
                "labels": [{"href": "/orgs/5/labels/2"}], "sub_groups": []
            }))
            .unwrap(),
        );
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    fn rule_sets() -> Vec<RuleSet> {
        let mut sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        // A rule naming a workload, the nested group and an active IP list href.
        sets[1].rules.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/4", "enabled": false,
                "providers": [{"workload": {"href": "/orgs/5/workloads/aaaa-1"}}],
                "consumers": [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/12"}},
                              {"ip_list": {"href": "/orgs/5/sec_policy/active/ip_lists/21"}}],
                "ingress_services": [{"port": 53, "proto": 17}]
            }))
            .unwrap(),
        );
        sets
    }

    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    /// The hrefs' last two segments (`sec_rules/1`) of the rules `expr` finds.
    fn found(expr: &str) -> Vec<String> {
        let idx = index();
        let sets = rule_sets();
        let wl = workloads();
        let f = RuleFilter::parse(expr).unwrap();
        sets.iter()
            .flat_map(|rs| rules_of(rs, &rs.deny_rules))
            .filter(|r| f.matches(r, &idx, &wl))
            .map(|r| {
                let mut parts: Vec<&str> = r.href.rsplit('/').take(2).collect();
                parts.reverse();
                parts.join("/")
            })
            .collect()
    }

    #[test]
    fn a_label_term_matches_scope_consumers_and_providers() {
        // app=shop is in ruleset 100's scope: every rule of it.
        assert_eq!(
            found("app=shop"),
            ["deny_rules/8", "sec_rules/1", "sec_rules/2", "deny_rules/7"]
        );
        // role=web: a consumer of rule 1, a provider of rule 2.
        assert_eq!(found("role=web"), ["sec_rules/1", "sec_rules/2"]);
        assert!(found("role=nope").is_empty());
        assert!(found("nope=web").is_empty());
    }

    #[test]
    fn a_comma_is_any_of_and_terms_and() {
        assert_eq!(found("role=web,db").len(), 4);
        assert_eq!(found("role=web,db deny"), ["deny_rules/8"]);
        assert_eq!(found("role=web svc:443/tcp"), ["sec_rules/2"]);
    }

    #[test]
    fn a_label_group_holding_the_label_counts() {
        // env=prod through the group in rule 3's consumers (and 100's scope).
        assert!(found("env=prod").contains(&"sec_rules/3".to_string()));
        // role=db through a sub-group of `Tiers` in rule 4.
        assert!(found("role=db").contains(&"sec_rules/4".to_string()));
    }

    #[test]
    fn ip_list_service_and_port_terms() {
        assert_eq!(found("ipl:corp"), ["sec_rules/2", "sec_rules/4"]);
        assert_eq!(found("ipl:\"Any (0.0.0.0/0 and ::/0)\""), ["deny_rules/7"]);
        assert_eq!(found("svc:postgresql"), ["sec_rules/1"]);
        // Inline 443/tcp, the 8000-8099 range, All Services (proto -1).
        assert_eq!(found("svc:443/tcp"), ["sec_rules/2", "sec_rules/3"]);
        assert_eq!(found("svc:8050/tcp"), ["sec_rules/2", "sec_rules/3"]);
        assert_eq!(found("svc:5432"), ["sec_rules/1", "sec_rules/3"]);
        assert_eq!(found("svc:53/udp"), ["sec_rules/3", "sec_rules/4"]);
        assert!(found("svc:443/udp").iter().all(|h| h == "sec_rules/3"));
    }

    #[test]
    fn workload_href_disabled_and_deny_terms() {
        assert_eq!(found("wl:host-1.example"), ["sec_rules/4"]);
        assert_eq!(found("wl:WEB01"), ["sec_rules/4"]);
        assert_eq!(found("wl:aaaa-1"), ["sec_rules/4"]);
        assert!(found("wl:host-2.example").is_empty());
        assert_eq!(found("href:sec_rules/2"), ["sec_rules/2"]);
        // Ruleset 101 is disabled: its rules count as disabled.
        assert_eq!(found("disabled"), ["sec_rules/3", "sec_rules/4"]);
        assert_eq!(found("deny"), ["deny_rules/8", "deny_rules/7"]);
        assert_eq!(found("").len(), 6);
    }

    #[test]
    fn unknown_names_are_named() {
        let f = RuleFilter::parse("os=linux app=shop,shpo ipl:Crop svc:Wbe svc:web wl:x svc:1/tcp")
            .unwrap();
        assert_eq!(
            f.unknown(&index()),
            [
                "unknown key os",
                "unknown value app=shpo",
                "unknown IP list Crop (closest: Corp)",
                "unknown service Wbe (closest: Web)"
            ]
        );
    }

    #[test]
    fn names_resolve_exact_then_by_a_unique_prefix() {
        let names: Vec<String> = ["Any (0.0.0.0/0 and ::/0)", "Apps", "Corp", "corp-eu"]
            .map(String::from)
            .to_vec();
        let one = |n: &str| NameMatch::One(n.to_string());
        assert_eq!(resolve_name("Corp", &names), one("Corp"));
        // Exact (any case) wins over being a prefix of `corp-eu`.
        assert_eq!(resolve_name("CORP", &names), one("Corp"));
        assert_eq!(resolve_name("any", &names), one("Any (0.0.0.0/0 and ::/0)"));
        assert_eq!(
            resolve_name("a", &names),
            NameMatch::Many(vec!["Any (0.0.0.0/0 and ::/0)".into(), "Apps".into()])
        );
        assert_eq!(
            resolve_name("crop", &names),
            NameMatch::None(vec!["Corp".into()])
        );
        assert_eq!(
            resolve_name("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", &names),
            NameMatch::None(vec![])
        );

        // The filter takes the object's own name: `ipl:Any` finds rule 7.
        let (f, warn) = RuleFilter::parse("ipl:Any svc:post")
            .unwrap()
            .resolve(&index());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(
            f.to_string(),
            "ipl:\"Any (0.0.0.0/0 and ::/0)\" svc:PostgreSQL"
        );
        let (_, warn) = RuleFilter::parse("svc:p svc:zz9")
            .unwrap()
            .resolve(&index());
        assert_eq!(warn, ["unknown service zz9"]);
    }

    /// A rule set (no scope, or the given one) with one rule of these
    /// actors and services.
    fn one_rule(scope: serde_json::Value, rule: serde_json::Value) -> RuleSet {
        let mut rule = rule;
        rule["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/9");
        rule["enabled"] = json!(true);
        serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/200", "name": "X", "enabled": true,
            "scopes": [scope], "rules": [rule]
        }))
        .unwrap()
    }

    fn hits(expr: &str, rs: &RuleSet, idx: &NameIndex) -> bool {
        let f = RuleFilter::parse(expr).unwrap();
        rules_of(rs, &[]).iter().any(|r| f.matches(r, idx, &[]))
    }

    #[test]
    fn exclusion_actors_are_not_label_hits() {
        let idx = index();
        // Scope `All except app=shop`, consumers `!role=web`, providers role=db.
        let rs = one_rule(
            json!([{"label": {"href": "/orgs/5/labels/3"}, "exclusion": true}]),
            json!({
                "providers": [{"label": {"href": "/orgs/5/labels/2"}}],
                "consumers": [{"label": {"href": "/orgs/5/labels/1"}, "exclusion": true},
                              {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/13"}, "exclusion": true}],
                "ingress_services": [{"port": 22, "proto": 6}]
            }),
        );
        assert!(!hits("app=shop", &rs, &idx));
        assert!(!hits("role=web", &rs, &idx));
        // role=db: the provider counts, the excluded group does not stop it.
        assert!(hits("role=db", &rs, &idx));
        let rs = one_rule(
            json!([]),
            json!({
                "providers": [{"actors": "ams"}],
                "consumers": [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/13"}, "exclusion": true}],
                "ingress_services": []
            }),
        );
        assert!(!hits("role=db", &rs, &idx));
    }

    #[test]
    fn protocol_only_entries_hold_their_ports_and_proto_terms() {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let mut services: Vec<Service> =
            serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
        services.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/services/40", "name": "All TCP",
                "service_ports": [{"proto": 6}]
            }))
            .unwrap(),
        );
        services.push(
            serde_json::from_value(json!({
                "href": "/orgs/5/sec_policy/draft/services/41", "name": "Ping",
                "service_ports": [{"proto": 1}]
            }))
            .unwrap(),
        );
        let idx = NameIndex::new(&labels, &[], &[], &services);
        let actors = json!({"providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}]});
        let with = |svc: serde_json::Value| {
            let mut r = actors.clone();
            r["ingress_services"] = svc;
            one_rule(json!([]), r)
        };
        let inline_tcp = with(json!([{"proto": 6}]));
        let object_tcp = with(json!([{"href": "/orgs/5/sec_policy/draft/services/40"}]));
        let ping = with(json!([{"href": "/orgs/5/sec_policy/draft/services/41"}]));
        let udp53 = with(json!([{"port": 53, "proto": 17}]));
        for rs in [&inline_tcp, &object_tcp] {
            assert!(hits("svc:443", rs, &idx));
            assert!(hits("svc:443/tcp", rs, &idx));
            assert!(!hits("svc:443/udp", rs, &idx));
            assert!(hits("svc:tcp", rs, &idx));
            assert!(!hits("svc:icmp", rs, &idx));
        }
        assert!(hits("svc:icmp", &ping, &idx));
        assert!(!hits("svc:443", &ping, &idx));
        assert!(hits("svc:UDP", &udp53, &idx));
        assert!(!hits("svc:tcp", &udp53, &idx));
        assert_eq!(RuleTerm::service("ICMP"), RuleTerm::Proto(1));
        assert_eq!(
            RuleTerm::service("17"),
            RuleTerm::Port(PortTerm {
                port: 17,
                proto: None
            })
        );
        assert_eq!(RuleTerm::service("Web"), RuleTerm::Service("Web".into()));
        assert_eq!(RuleFilter::parse("svc:Tcp").unwrap().to_string(), "svc:tcp");
    }

    #[test]
    fn parse_errors_and_round_trip() {
        // `!app=shop` is a term now: an exclusion actor.
        for bad in ["app=-", "shop", "ipl:", "svc:\"x"] {
            assert!(RuleFilter::parse(bad).is_err(), "{bad}");
        }
        let f = RuleFilter::parse(
            "app=shop,crm IPL:\"Any (0.0.0.0/0 and ::/0)\" svc:443/TCP svc:Web wl:web01 href:/x Disabled DENY",
        )
        .unwrap();
        assert_eq!(f.terms.len(), 8);
        assert_eq!(
            f.terms[2],
            RuleTerm::Port(PortTerm {
                port: 443,
                proto: Some(6)
            })
        );
        let text = f.to_string();
        assert_eq!(
            text,
            "app=shop,crm ipl:\"Any (0.0.0.0/0 and ::/0)\" svc:443/tcp svc:Web wl:web01 href:/x disabled deny"
        );
        assert_eq!(RuleFilter::parse(&text).unwrap(), f);
    }

    #[test]
    fn a_side_prefix_looks_on_that_side_only() {
        // role=web: a consumer of rule 1, a provider of rule 2.
        assert_eq!(found("src:role=web"), ["sec_rules/1"]);
        assert_eq!(found("consumer:role=web"), ["sec_rules/1"]);
        assert_eq!(found("dst:role=web"), ["sec_rules/2"]);
        assert_eq!(found("provider:role=web"), ["sec_rules/2"]);
        assert!(found("scope:role=web").is_empty());
        // Ruleset 100's scope holds app=shop: its allow and deny rules.
        assert_eq!(
            found("scope:app=shop"),
            ["deny_rules/8", "sec_rules/1", "sec_rules/2", "deny_rules/7"]
        );
        assert!(found("src:app=shop").is_empty());
        assert_eq!(found("dst:role=db"), ["deny_rules/8", "sec_rules/1"]);
        // Several values: any of them, on that side.
        assert_eq!(
            found("dst:role=web,db"),
            ["deny_rules/8", "sec_rules/1", "sec_rules/2"]
        );
    }

    #[test]
    fn a_side_prefix_counts_label_groups_on_each_side() {
        // env=prod: a group consumer of rule 3 and in ruleset 100's scope.
        assert_eq!(found("src:env=prod"), ["sec_rules/3"]);
        assert!(found("dst:env=prod").is_empty());
        assert_eq!(found("scope:env=prod").len(), 4);
        // role=db through a sub-group of `Tiers`, a consumer of rule 4.
        assert_eq!(found("src:role=db"), ["sec_rules/4"]);
        // A group as a provider and in a scope.
        let idx = index();
        let rs = one_rule(
            json!([{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]),
            json!({
                "providers": [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/12"}}],
                "consumers": [{"actors": "ams"}],
                "ingress_services": []
            }),
        );
        assert!(hits("dst:role=db", &rs, &idx));
        assert!(!hits("src:role=db", &rs, &idx));
        assert!(hits("scope:env=prod", &rs, &idx));
        assert!(!hits("scope:global", &rs, &idx));
    }

    #[test]
    fn src_and_dst_take_ip_lists_workloads_and_all() {
        assert_eq!(found("src:ipl:corp"), ["sec_rules/2", "sec_rules/4"]);
        assert!(found("dst:ipl:corp").is_empty());
        assert_eq!(
            found("src:ipl:\"Any (0.0.0.0/0 and ::/0)\""),
            ["deny_rules/7"]
        );
        assert_eq!(found("dst:wl:web01"), ["sec_rules/4"]);
        assert!(found("src:wl:web01").is_empty());
        // All Workloads: a consumer of deny rule 8, a provider of 7 and 3.
        assert_eq!(found("src:all"), ["deny_rules/8"]);
        assert_eq!(found("dst:ALL"), ["deny_rules/7", "sec_rules/3"]);
        let f = RuleFilter::parse("dst:wl:web01").unwrap();
        assert!(f.names_workloads());
        assert!(RuleFilter::parse("-src:wl:x").unwrap().names_workloads());
        assert!(!RuleFilter::parse("src:ipl:x").unwrap().names_workloads());
    }

    #[test]
    fn scope_global_is_no_scope_or_one_without_labels() {
        // Ruleset 101's scope is `[[]]`.
        assert_eq!(found("scope:global"), ["sec_rules/3", "sec_rules/4"]);
        assert_eq!(found("-scope:global").len(), 4);
        let idx = index();
        let rule = json!({"providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
                          "ingress_services": []});
        let mut none = one_rule(json!([]), rule.clone());
        none.scopes.clear();
        assert!(hits("scope:global", &none, &idx));
        // An exclusion is a label: such a scope is not global.
        let excl = one_rule(
            json!([{"label": {"href": "/orgs/5/labels/3"}, "exclusion": true}]),
            rule.clone(),
        );
        assert!(!hits("scope:global", &excl, &idx));
        // One global scope among others: the ruleset applies everywhere.
        let mut two = one_rule(json!([{"label": {"href": "/orgs/5/labels/3"}}]), rule);
        two.scopes.push(Vec::new());
        assert!(hits("scope:global", &two, &idx));
        assert!(hits("scope:app=shop", &two, &idx));
    }

    #[test]
    fn a_bang_term_finds_exclusion_actors_on_each_side() {
        let idx = index();
        // Scope `All except app=shop`, consumers `!role=web` and the
        // excluded group `Db` (role=db), providers role=db.
        let rs = one_rule(
            json!([{"label": {"href": "/orgs/5/labels/3"}, "exclusion": true}]),
            json!({
                "providers": [{"label": {"href": "/orgs/5/labels/2"}}],
                "consumers": [{"label": {"href": "/orgs/5/labels/1"}, "exclusion": true},
                              {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/13"}, "exclusion": true}],
                "ingress_services": []
            }),
        );
        assert!(hits("scope:!app=shop", &rs, &idx));
        assert!(!hits("scope:app=shop", &rs, &idx));
        assert!(hits("!app=shop", &rs, &idx));
        assert!(hits("src:!role=web", &rs, &idx));
        assert!(!hits("src:role=web", &rs, &idx));
        assert!(!hits("dst:!role=web", &rs, &idx));
        // Through the excluded group, on the consumer side only.
        assert!(hits("src:!role=db", &rs, &idx));
        assert!(!hits("dst:!role=db", &rs, &idx));
        assert!(hits("dst:role=db", &rs, &idx));
        assert!(!hits("src:role=db", &rs, &idx));
        assert!(!hits("scope:!env=prod", &rs, &idx));
        // An excluded provider group.
        let rs = one_rule(
            json!([]),
            json!({
                "providers": [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/12"}, "exclusion": true}],
                "consumers": [{"actors": "ams"}],
                "ingress_services": []
            }),
        );
        assert!(hits("dst:!role=db", &rs, &idx));
        assert!(hits("!role=db", &rs, &idx));
        assert!(!hits("src:!role=db", &rs, &idx));
        assert!(!hits("role=db", &rs, &idx));
        // The fixture has no exclusions.
        assert!(found("!app=shop").is_empty());
    }

    #[test]
    fn a_dash_negates_each_kind_of_term() {
        assert_eq!(
            found("-deny"),
            ["sec_rules/1", "sec_rules/2", "sec_rules/3", "sec_rules/4"]
        );
        assert_eq!(found("-scope:app=shop"), ["sec_rules/3", "sec_rules/4"]);
        assert_eq!(
            found("-svc:443/tcp"),
            ["deny_rules/8", "sec_rules/1", "deny_rules/7", "sec_rules/4"]
        );
        assert_eq!(found("-svc:PostgreSQL").len(), 5);
        assert_eq!(found("-svc:udp").len(), 4);
        assert_eq!(found("-role=web").len(), 4);
        assert_eq!(found("-ipl:corp").len(), 4);
        assert_eq!(found("-wl:web01").len(), 5);
        assert_eq!(
            found("-href:deny_rules"),
            ["sec_rules/1", "sec_rules/2", "sec_rules/3", "sec_rules/4"]
        );
        assert_eq!(found("-disabled"), found("enabled"));
        assert_eq!(found("-allow"), found("deny"));
        assert_eq!(found("-src:all"), found("-href:deny_rules/8"));
        assert_eq!(
            found("-intra"),
            ["deny_rules/8", "sec_rules/2", "deny_rules/7"]
        );
        // Terms AND, negated or not: app=shop rules, no deny, not intra.
        assert_eq!(found("app=shop -deny -intra"), ["sec_rules/2"]);
        let idx = index();
        let rs = one_rule(
            json!([]),
            json!({
                "providers": [{"actors": "ams"}],
                "consumers": [{"label": {"href": "/orgs/5/labels/1"}, "exclusion": true}],
                "ingress_services": []
            }),
        );
        assert!(!hits("-!role=web", &rs, &idx));
        assert!(hits("-role=web", &rs, &idx));
    }

    #[test]
    fn rule_properties_intra_extra_enabled_allow() {
        // Rule 2 is extra-scope; 1, 3 and 4 intra (3 leaves the flag out).
        assert_eq!(found("extra"), ["sec_rules/2"]);
        assert_eq!(
            found("intra"),
            ["sec_rules/1", "sec_rules/3", "sec_rules/4"]
        );
        // Deny rules are neither.
        assert!(found("deny extra").is_empty());
        assert!(found("deny intra").is_empty());
        // Even one that came with the flag set.
        let sets = rule_sets();
        let mut d = RuleRef::of_deny(&sets[0].deny_rules[0], &sets[0]);
        d.unscoped_consumers = true;
        for t in ["intra", "extra", "allow"] {
            assert!(
                !RuleFilter::parse(t).unwrap().matches(&d, &index(), &[]),
                "{t}"
            );
        }
        assert_eq!(
            found("allow"),
            ["sec_rules/1", "sec_rules/2", "sec_rules/3", "sec_rules/4"]
        );
        // Ruleset 101 is off: rules 3 and 4 are not enabled.
        assert_eq!(
            found("enabled"),
            ["deny_rules/8", "sec_rules/1", "sec_rules/2", "deny_rules/7"]
        );
        assert_eq!(found("Enabled ALLOW"), ["sec_rules/1", "sec_rules/2"]);
    }

    #[test]
    fn the_issue_examples() {
        // web to db, extra-scope: rule 1 is intra.
        assert!(found("src:role=web dst:role=db extra").is_empty());
        assert_eq!(found("src:role=web dst:role=db intra"), ["sec_rules/1"]);
        assert_eq!(
            found("scope:app=shop -svc:443/tcp"),
            ["deny_rules/8", "sec_rules/1", "deny_rules/7"]
        );
        assert!(found("src:!env=prod").is_empty());
        assert!(found("src:all -deny").is_empty());
        assert_eq!(found("dst:all -deny"), ["sec_rules/3"]);
    }

    #[test]
    fn side_and_negation_parse_errors() {
        for (bad, says) in [
            ("-", "- needs a term"),
            ("--x", "one - negates"),
            ("--deny", "one - negates"),
            ("- deny", "- needs a term"),
            ("-nope", "unknown term"),
            ("scope:ipl:Corp", "scope: takes"),
            ("scope:all", "scope: takes"),
            ("scope:wl:x", "scope: takes"),
            ("src:global", "src: takes"),
            ("dst:svc:443", "dst: takes"),
            ("src:", "src: takes"),
            ("src:ipl:", "ipl: needs a name"),
            ("dst:wl:\"\"", "wl: needs a name"),
            ("src:app=-", "no key=-"),
            ("!app=-", "no key=-"),
            ("all", "unknown term"),
            ("global", "unknown term"),
        ] {
            let err = RuleFilter::parse(bad).unwrap_err();
            assert!(err.contains(says), "{bad}: {err}");
        }
    }

    #[test]
    fn sides_and_negation_round_trip_and_resolve() {
        let f = RuleFilter::parse(
            "SRC:!env=prod consumer:role=web Provider:ipl:Corp dst:wl:\"db 1\" scope:GLOBAL src:all -deny -scope:app=shop intra extra enabled allow !app=shop",
        )
        .unwrap();
        assert_eq!(
            f.terms[0],
            RuleTerm::At(
                Position::Consumers,
                ActorTerm::Label(LabelTerm::parse("!env=prod").unwrap())
            )
        );
        assert_eq!(f.terms[6], RuleTerm::Not(Box::new(RuleTerm::Deny)));
        let text = f.to_string();
        assert_eq!(
            text,
            "src:!env=prod src:role=web dst:ipl:Corp dst:wl:\"db 1\" scope:global src:all -deny -scope:app=shop intra extra enabled allow !app=shop"
        );
        assert_eq!(RuleFilter::parse(&text).unwrap(), f);
        // Names inside sides and negations resolve and warn as elsewhere.
        let (f, warn) = RuleFilter::parse("src:ipl:any -ipl:Crop scope:os=linux -dst:role=dbx")
            .unwrap()
            .resolve(&index());
        assert_eq!(
            warn,
            [
                "unknown IP list Crop (closest: Corp)",
                "unknown key os",
                "unknown value role=dbx"
            ]
        );
        assert!(
            f.to_string()
                .starts_with("src:ipl:\"Any (0.0.0.0/0 and ::/0)\" ")
        );
    }
}
