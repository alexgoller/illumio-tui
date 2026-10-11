//! The token grammar the CLI and the TUI share: rule actors (`app=web`,
//! `label:key=value`, `group:NAME`, `iplist:NAME`, `workload:HOST`, `all`,
//! hrefs, `!` before a label, label group or IP list), rule services
//! (`443/tcp`, `icmp`, `HTTPS`, `service:NAME`, hrefs) and ruleset scopes.
//! Parsing makes no request, and neither does checking an href (the
//! library's strict checkers). Resolving a name is the caller's job: the
//! CLI reads each object list at most once, the TUI looks names up in the
//! loaded `NameIndex`.
use pce::write::{
    ActorKind, ActorRef, PortSpec, ServiceRef, WriteError, checked_href, checked_policy_href,
};

const ACTOR_GRAMMAR: &str = "use key=value, label:key=value, group:NAME, iplist:NAME, workload:HOST, all, an href, or ! before a label, label group or IP list";

pub fn is_href(s: &str) -> bool {
    s.trim_start().starts_with("/orgs/")
}

/// `key=value`, split on the first `=`; both sides trimmed and non-empty.
pub fn split_kv(spec: &str) -> Result<(String, String), String> {
    let (k, v) = spec
        .split_once('=')
        .ok_or_else(|| format!("expected key=value, got {spec:?}"))?;
    let (k, v) = (k.trim(), v.trim());
    if k.is_empty() || v.is_empty() {
        return Err(format!(
            "expected key=value with both sides non-empty, got {spec:?}"
        ));
    }
    Ok((k.to_string(), v.to_string()))
}

/// Up to five names that contain `needle` case-insensitively, for an
/// error that found no exact match.
pub fn similar<'a>(needle: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let needle = needle.to_lowercase();
    let mut out: Vec<String> = names
        .filter(|n| n.to_lowercase().contains(&needle))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out.truncate(5);
    out
}

pub fn with_similar(msg: String, similar: &[String]) -> String {
    if similar.is_empty() {
        msg
    } else {
        format!("{msg}; similar: {}", similar.join(", "))
    }
}

/// One provider, consumer or scope token, parsed without any request
/// (grammar in the README's Writes section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorToken {
    All,
    /// `key=value`, already split and re-joined on the first `=`.
    Label {
        spec: String,
        exclusion: bool,
    },
    LabelGroup {
        name: String,
        exclusion: bool,
    },
    IpList {
        name: String,
        exclusion: bool,
    },
    Workload {
        name: String,
    },
    Href {
        href: String,
        exclusion: bool,
    },
}

impl ActorToken {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        let (exclusion, rest) = match t.strip_prefix('!') {
            Some(r) => (true, r.trim()),
            None => (false, t),
        };
        if rest.is_empty() {
            return Err(format!("{s:?} is empty; {ACTOR_GRAMMAR}"));
        }
        let named = |prefix: &str| -> Option<Result<String, String>> {
            rest.strip_prefix(prefix).map(|n| {
                let n = n.trim();
                if n.is_empty() {
                    Err(format!("{s:?}: missing name after {prefix}"))
                } else {
                    Ok(n.to_string())
                }
            })
        };
        if rest.eq_ignore_ascii_case("all") {
            return if exclusion {
                Err(format!("{s:?}: all cannot be excluded"))
            } else {
                Ok(Self::All)
            };
        }
        if is_href(rest) {
            return Ok(Self::Href {
                href: rest.to_string(),
                exclusion,
            });
        }
        if let Some(name) = named("group:") {
            return Ok(Self::LabelGroup {
                name: name?,
                exclusion,
            });
        }
        if let Some(name) = named("iplist:") {
            return Ok(Self::IpList {
                name: name?,
                exclusion,
            });
        }
        if let Some(name) = named("workload:") {
            if exclusion {
                return Err(format!(
                    "{s:?}: only labels, label groups and IP lists can be excluded"
                ));
            }
            return Ok(Self::Workload { name: name? });
        }
        let spec = rest.strip_prefix("label:").unwrap_or(rest);
        if spec != rest && is_href(spec) {
            // `label:<href>` is checked as a label href at resolution.
            return Ok(Self::Label {
                spec: spec.to_string(),
                exclusion,
            });
        }
        if spec.contains('=') {
            let (k, v) = split_kv(spec)?;
            return Ok(Self::Label {
                spec: format!("{k}={v}"),
                exclusion,
            });
        }
        Err(format!("{s:?} is not an actor; {ACTOR_GRAMMAR}"))
    }

    /// Scopes take labels and label groups; an href's kind is checked by
    /// the library without a request.
    pub fn scope_member(&self) -> bool {
        matches!(
            self,
            Self::Label { .. } | Self::LabelGroup { .. } | Self::Href { .. }
        )
    }
}

/// An `icmp` or `icmpv6` token in a rule: a rule's inline services take
/// tcp and udp only, so it becomes the ICMP service object's href.
/// `None` for any other port token.
pub fn icmp_service_ref(p: &PortSpec, idx: &pce::NameIndex) -> Option<Result<ServiceRef, String>> {
    if !matches!(p.proto, 1 | 58) {
        return None;
    }
    Some(
        idx.icmp_service_href()
            .map(|h| ServiceRef::Href(h.to_string()))
            .ok_or_else(|| {
                format!(
                    "{}; no ICMP service object found",
                    pce::write::INLINE_SERVICE
                )
            }),
    )
}

/// One service token: an href, an inline port, or a service name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceToken {
    Href(String),
    Port(PortSpec),
    Name(String),
}

impl ServiceToken {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t.is_empty() {
            return Err("empty service".into());
        }
        if is_href(t) {
            return Ok(Self::Href(t.to_string()));
        }
        if let Some(n) = t.strip_prefix("service:") {
            let n = n.trim();
            return if n.is_empty() {
                Err(format!("{s:?}: missing service name"))
            } else {
                Ok(Self::Name(n.to_string()))
            };
        }
        // A slash or a bare number can only mean a port: report the port
        // error, not "no service named".
        if t.contains('/') || t.bytes().all(|b| b.is_ascii_digit()) {
            return PortSpec::parse(t).map(Self::Port);
        }
        Ok(PortSpec::parse(t)
            .map(Self::Port)
            .unwrap_or_else(|_| Self::Name(t.to_string())))
    }
}

/// One scope: comma-separated labels and label groups. A spec with no
/// token is an empty scope; what that means is the caller's decision (the
/// CLI refuses it, the TUI refuses it between `|`).
pub fn parse_scope(spec: &str) -> Result<Vec<ActorToken>, String> {
    let parsed = spec
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ActorToken::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.iter().any(|t| !t.scope_member()) {
        return Err("scopes take labels and label groups only".into());
    }
    Ok(parsed)
}

/// [`parse_scope`] for each spec, in order.
pub fn parse_scope_tokens(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, String> {
    specs.iter().map(String::as_str).map(parse_scope).collect()
}

pub fn with_exclusion(mut a: ActorRef, exclusion: bool) -> ActorRef {
    a.exclusion = exclusion;
    a
}

/// Tokens that carry an href are checked by the library's strict checkers
/// for the kind their prefix names, with no list consulted. `None` for
/// tokens that name an object (or `all`).
pub fn actor_from_href_token(
    token: &ActorToken,
    org_id: u64,
) -> Option<Result<ActorRef, WriteError>> {
    let (kind, href, segment, policy, exclusion) = match token {
        ActorToken::Href { href, exclusion } => {
            return Some(ActorRef::from_href(href, org_id).map(|a| with_exclusion(a, *exclusion)));
        }
        ActorToken::Label { spec, exclusion } if is_href(spec) => {
            (ActorKind::Label, spec, "labels", false, *exclusion)
        }
        ActorToken::LabelGroup { name, exclusion } if is_href(name) => (
            ActorKind::LabelGroup,
            name,
            "label_groups",
            true,
            *exclusion,
        ),
        ActorToken::IpList { name, exclusion } if is_href(name) => {
            (ActorKind::IpList, name, "ip_lists", true, *exclusion)
        }
        ActorToken::Workload { name } if is_href(name) => {
            (ActorKind::Workload, name, "workloads", false, false)
        }
        _ => return None,
    };
    let checked = if policy {
        checked_policy_href(href, segment, org_id)
    } else {
        checked_href(href, segment, org_id)
    };
    Some(checked.map(|h| with_exclusion(ActorRef::new(kind, h), exclusion)))
}

pub fn service_from_href_token(
    token: &ServiceToken,
    org_id: u64,
) -> Option<Result<ServiceRef, WriteError>> {
    match token {
        ServiceToken::Href(h) => Some(ServiceRef::from_href(h, org_id)),
        ServiceToken::Name(n) if is_href(n) => Some(ServiceRef::from_href(n, org_id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_parse_one_per_spec_and_keep_empty_ones_empty() {
        let scopes = parse_scope_tokens(&["app=web, group:G".into(), " , ".into()]).unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(
            scopes[0],
            vec![
                ActorToken::Label {
                    spec: "app=web".into(),
                    exclusion: false
                },
                ActorToken::LabelGroup {
                    name: "G".into(),
                    exclusion: false
                },
            ]
        );
        assert!(
            scopes[1].is_empty(),
            "the caller decides what an empty scope means"
        );
        assert_eq!(
            parse_scope("iplist:Corp").unwrap_err(),
            "scopes take labels and label groups only"
        );
        assert!(
            parse_scope("app=")
                .unwrap_err()
                .contains("expected key=value")
        );
        assert_eq!(split_kv("app=a=b").unwrap(), ("app".into(), "a=b".into()));
        assert!(is_href(" /orgs/5/labels/1"));
    }

    #[test]
    fn actor_tokens_parse_every_form() {
        let label = |spec: &str, exclusion| ActorToken::Label {
            spec: spec.into(),
            exclusion,
        };
        assert_eq!(
            ActorToken::parse("app=web").unwrap(),
            label("app=web", false)
        );
        assert_eq!(
            ActorToken::parse(" label:app = a=b ").unwrap(),
            label("app=a=b", false)
        );
        assert_eq!(
            ActorToken::parse("!env=prod").unwrap(),
            label("env=prod", true)
        );
        assert_eq!(
            ActorToken::parse("group:All prod envs").unwrap(),
            ActorToken::LabelGroup {
                name: "All prod envs".into(),
                exclusion: false
            }
        );
        assert_eq!(
            ActorToken::parse("!iplist:Corp").unwrap(),
            ActorToken::IpList {
                name: "Corp".into(),
                exclusion: true
            }
        );
        assert_eq!(
            ActorToken::parse("workload:web-1.lab").unwrap(),
            ActorToken::Workload {
                name: "web-1.lab".into()
            }
        );
        assert_eq!(ActorToken::parse("ALL").unwrap(), ActorToken::All);
        assert_eq!(
            ActorToken::parse("!/orgs/5/labels/1").unwrap(),
            ActorToken::Href {
                href: "/orgs/5/labels/1".into(),
                exclusion: true
            }
        );
    }

    #[test]
    fn actor_tokens_refuse_bad_input() {
        assert_eq!(
            ActorToken::parse("!all").unwrap_err(),
            r#""!all": all cannot be excluded"#
        );
        assert!(
            ActorToken::parse("!workload:x")
                .unwrap_err()
                .contains("only labels, label groups and IP lists can be excluded")
        );
        assert!(
            ActorToken::parse("group:")
                .unwrap_err()
                .contains("missing name")
        );
        assert!(
            ActorToken::parse("web")
                .unwrap_err()
                .contains("is not an actor")
        );
        assert!(
            ActorToken::parse("app=")
                .unwrap_err()
                .contains("expected key=value")
        );
        assert!(ActorToken::parse("!").unwrap_err().contains("is empty"));
    }

    #[test]
    fn service_tokens_split_ports_names_and_hrefs() {
        use pce::write::PortSpec;
        assert_eq!(
            ServiceToken::parse("/orgs/5/sec_policy/draft/services/31").unwrap(),
            ServiceToken::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        assert_eq!(
            ServiceToken::parse("443/tcp").unwrap(),
            ServiceToken::Port(PortSpec::parse("443/tcp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("8000-8100/udp").unwrap(),
            ServiceToken::Port(PortSpec::parse("8000-8100/udp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("icmp").unwrap(),
            ServiceToken::Port(PortSpec::parse("icmp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("HTTPS").unwrap(),
            ServiceToken::Name("HTTPS".into())
        );
        assert_eq!(
            ServiceToken::parse("service:tcp").unwrap(),
            ServiceToken::Name("tcp".into())
        );
        assert!(
            ServiceToken::parse("443")
                .unwrap_err()
                .contains("bare number is ambiguous")
        );
        assert!(
            ServiceToken::parse("443/tpc")
                .unwrap_err()
                .contains("unknown protocol")
        );
        assert!(ServiceToken::parse("service: ").is_err());
    }

    #[test]
    fn prefixed_hrefs_resolve_through_the_strict_checkers_without_lists() {
        // The href paths are pure (no Context, no Lists, no request), so
        // these tests cannot fetch anything.
        let org = 5;
        let actor = |s: &str| {
            let tok = ActorToken::parse(s).unwrap();
            actor_from_href_token(&tok, org).expect("href token")
        };
        let a = actor("label:/orgs/5/labels/1").unwrap();
        assert_eq!(
            (a.kind, a.href.as_deref()),
            (ActorKind::Label, Some("/orgs/5/labels/1"))
        );
        assert!(actor("!label:/orgs/5/labels/1").unwrap().exclusion);
        let a = actor("group:/orgs/5/sec_policy/active/label_groups/2").unwrap();
        assert_eq!(a.kind, ActorKind::LabelGroup);
        assert_eq!(
            a.href.as_deref(),
            Some("/orgs/5/sec_policy/draft/label_groups/2")
        );
        let a = actor("!iplist:/orgs/5/sec_policy/draft/ip_lists/3").unwrap();
        assert_eq!(a.kind, ActorKind::IpList);
        assert!(a.exclusion);
        let a = actor("workload:/orgs/5/workloads/abc").unwrap();
        assert_eq!(a.kind, ActorKind::Workload);
        assert_eq!(a.href.as_deref(), Some("/orgs/5/workloads/abc"));
        assert!(
            actor("iplist:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("ip_lists")
        );
        assert!(
            actor("group:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("label_groups")
        );
        assert!(
            actor("workload:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("workloads")
        );
        assert!(actor("label:/orgs/9/labels/1").is_err(), "wrong org");
        assert!(actor_from_href_token(&ActorToken::parse("app=web").unwrap(), org).is_none());
        assert!(actor_from_href_token(&ActorToken::parse("iplist:Corp").unwrap(), org).is_none());

        let svc = |s: &str| service_from_href_token(&ServiceToken::parse(s).unwrap(), org);
        assert_eq!(
            svc("service:/orgs/5/sec_policy/active/services/31")
                .unwrap()
                .unwrap(),
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        assert!(svc("service:/orgs/5/labels/1").unwrap().is_err());
        assert!(svc("HTTPS").is_none());
        assert!(svc("443/tcp").is_none());
    }
}
