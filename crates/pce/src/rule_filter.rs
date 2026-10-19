//! Rule search: which loaded rules reference a label, an IP list, a
//! service, a workload, an href; which are disabled or deny rules. One
//! grammar for the TUI's `f` on Policy and the CLI's `policy search`:
//!
//! ```text
//! app=shop,crm ipl:Corp svc:443/tcp svc:PostgreSQL wl:web01 href:sec_rules/2 disabled deny
//! ```
//!
//! Terms apart by spaces must all hold (AND). `key=v1,v2`: a label any of
//! whose values is in the rule's scope, consumers or providers, directly
//! or through a label group (sub-groups included). `ipl:NAME`, `svc:NAME`:
//! an IP list or service object by name (any case). `svc:443/tcp` (or
//! `svc:443`, tcp or udp): an inline port or range that holds it, or a
//! service object with such an entry (`All Services` too). `wl:NAME`: a
//! workload actor by hostname or name (of the workloads the caller holds)
//! or by its href's id. `href:TEXT`: the rule's href contains it.
//! `disabled`: the rule or its ruleset is off. `deny`: a deny or override
//! deny rule. A name with spaces is quoted (`ipl:"Any (0.0.0.0/0 and ::/0)"`).
//! Pure: the caller brings the loaded rulesets and the name index.
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
    /// `-1` (every service) holds every port.
    fn held_by(&self, port: Option<u16>, to_port: Option<u16>, proto: i64) -> bool {
        if proto == -1 {
            return true;
        }
        let proto_ok = match self.proto {
            Some(p) => i64::from(p) == proto,
            None => proto == 6 || proto == 17,
        };
        let Some(from) = port else {
            return false;
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

/// One search term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleTerm {
    /// `key=v1,v2`: any of the values (never `!` or `-`).
    Label(LabelTerm),
    IpList(String),
    Service(String),
    Port(PortTerm),
    Workload(String),
    Href(String),
    Disabled,
    Deny,
}

/// Why a term is not one: what the grammar takes.
const GRAMMAR: &str = "use key=value[,value], ipl:NAME, svc:NAME or svc:PORT/PROTO, wl:NAME, href:TEXT, disabled or deny";

impl RuleTerm {
    /// One term (a token of an expression, or a CLI flag's value made one).
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        let lower = t.to_ascii_lowercase();
        if lower == "disabled" {
            return Ok(Self::Disabled);
        }
        if lower == "deny" {
            return Ok(Self::Deny);
        }
        for (prefix, make) in [
            ("ipl:", Self::IpList as fn(String) -> Self),
            ("svc:", Self::Service),
            ("wl:", Self::Workload),
            ("href:", Self::Href),
        ] {
            let Some(rest) = t
                .get(..prefix.len())
                .filter(|h| h.eq_ignore_ascii_case(prefix))
                .map(|_| &t[prefix.len()..])
            else {
                continue;
            };
            let name = strip_quotes(rest.trim())?;
            if name.trim().is_empty() {
                return Err(format!("{t:?}: {prefix} needs a name"));
            }
            if prefix == "svc:"
                && let Some(p) = PortTerm::parse(&name)
            {
                return Ok(Self::Port(p));
            }
            return Ok(make(name));
        }
        if t.contains('=') {
            let term = LabelTerm::parse(t)?;
            if term.negate || term.none {
                return Err(format!(
                    "{t:?}: a rule search takes key=value[,value] (no ! or -)"
                ));
            }
            return Ok(Self::Label(term));
        }
        Err(format!("{t:?}: unknown term; {GRAMMAR}"))
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
            Self::Workload(n) => f.write_str(&prefixed("wl:", n)),
            Self::Href(h) => f.write_str(&prefixed("href:", h)),
            Self::Disabled => f.write_str("disabled"),
            Self::Deny => f.write_str("deny"),
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
    /// key os`, `unknown value app=shpo`, `unknown IP list Crop`, `unknown
    /// service Wbe`): such a term matches no rule. `wl:` and ports are
    /// not checked.
    pub fn unknown(&self, idx: &NameIndex) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.terms {
            match t {
                RuleTerm::Label(l) => {
                    if !idx.label_keys().contains(&l.key) {
                        out.push(format!("unknown key {}", l.key));
                        continue;
                    }
                    let known = idx.values_for_key(&l.key);
                    out.extend(l.values.iter().filter(|v| !known.contains(v)).map(|v| {
                        format!("unknown value {}={}", l.key, crate::label_filter::quote(v))
                    }));
                }
                RuleTerm::IpList(n) if idx.ip_list_href(n).is_none() => {
                    out.push(format!("unknown IP list {n}"));
                }
                RuleTerm::Service(n)
                    if !idx
                        .services()
                        .iter()
                        .any(|s| s.name.eq_ignore_ascii_case(n)) =>
                {
                    out.push(format!("unknown service {n}"));
                }
                _ => {}
            }
        }
        out
    }

    /// Whether `r` holds every term. `workloads` (those the caller holds,
    /// possibly none) name the workload actors for `wl:`.
    pub fn matches(&self, r: &RuleRef, idx: &NameIndex, workloads: &[Workload]) -> bool {
        self.terms
            .iter()
            .all(|t| term_matches(t, r, idx, workloads))
    }
}

fn term_matches(t: &RuleTerm, r: &RuleRef, idx: &NameIndex, workloads: &[Workload]) -> bool {
    match t {
        RuleTerm::Disabled => !r.enabled || !r.ruleset_enabled,
        RuleTerm::Deny => r.kind != RuleKind::Allow,
        RuleTerm::Href(h) => r.href.contains(h.as_str()),
        RuleTerm::Label(term) => r.actors().any(|a| label_hit(a, term, idx)),
        RuleTerm::IpList(name) => r.actors().any(|a| {
            a.ip_list.as_ref().is_some_and(|l| {
                idx.ip_list_name(&l.href)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
        }),
        RuleTerm::Workload(name) => r.actors().any(|a| {
            a.workload
                .as_ref()
                .is_some_and(|w| workload_named(&w.href, name, workloads))
        }),
        RuleTerm::Service(name) => r.services.iter().any(|s| match s {
            IngressService::Ref(s) => idx
                .service_name(&s.href)
                .is_some_and(|n| n.eq_ignore_ascii_case(name)),
            IngressService::Port { .. } => false,
        }),
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

/// A label actor with the term's key and one of its values, or a label
/// group with such a label among its members (sub-groups included).
fn label_hit(a: &Actor, term: &LabelTerm, idx: &NameIndex) -> bool {
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
                "unknown IP list Crop",
                "unknown service Wbe"
            ]
        );
    }

    #[test]
    fn parse_errors_and_round_trip() {
        for bad in ["!app=shop", "app=-", "shop", "ipl:", "svc:\"x"] {
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
}
