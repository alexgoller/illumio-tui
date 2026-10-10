//! Human filter expressions (`app=web`, `ipl:Corp`, `wl:host`, `10.0.0.1`,
//! `fqdn:x`) resolved to PCE actor matches. Unknown names fail here with the
//! known values listed, before anything is sent to the PCE.
use crate::error::{PceError, Result};
use crate::models::{ActorMatch, Ref, Workload};
use crate::resolve::NameIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpr {
    Label { key: String, value: String },
    Ip(String),
    IpList(String),
    Workload(String),
    Fqdn(String),
}

impl FilterExpr {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PceError::Invalid("empty filter expression".into()));
        }
        if let Some(rest) = s.strip_prefix("ip:") {
            return Ok(FilterExpr::Ip(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("ipl:") {
            return Ok(FilterExpr::IpList(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("wl:") {
            return Ok(FilterExpr::Workload(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("fqdn:") {
            return Ok(FilterExpr::Fqdn(rest.to_string()));
        }
        if let Some((k, v)) = s.split_once('=') {
            return Ok(FilterExpr::Label {
                key: k.trim().to_string(),
                value: v.trim().to_string(),
            });
        }
        if s.parse::<std::net::IpAddr>().is_ok()
            || s.split_once('/').is_some_and(|(ip, bits)| {
                ip.parse::<std::net::IpAddr>().is_ok() && bits.parse::<u8>().is_ok()
            })
        {
            return Ok(FilterExpr::Ip(s.to_string()));
        }
        Err(PceError::Invalid(format!(
            "cannot parse filter {s:?}; use key=value, ip:ADDR, ipl:NAME, wl:HOSTNAME or fqdn:NAME"
        )))
    }

    /// `wl:` needs the workload list to resolve a hostname to an href.
    pub fn needs_workloads(&self) -> bool {
        matches!(self, FilterExpr::Workload(_))
    }
}

pub fn resolve_actor(
    expr: &FilterExpr,
    idx: &NameIndex,
    workloads: &[Workload],
) -> Result<ActorMatch> {
    match expr {
        FilterExpr::Label { key, value } => match idx.label_href(key, value) {
            Some(href) => Ok(ActorMatch::Label {
                label: Ref::new(href),
            }),
            None => {
                let known = idx.values_for_key(key);
                let hint = if known.is_empty() {
                    format!("known keys: {}", idx.label_keys().join(", "))
                } else {
                    format!("known values for {key}: {}", known.join(", "))
                };
                Err(PceError::Invalid(format!(
                    "unknown label {key}={value}; {hint}"
                )))
            }
        },
        FilterExpr::IpList(name) => idx
            .ip_list_href(name)
            .map(|h| ActorMatch::IpList {
                ip_list: Ref::new(h),
            })
            .ok_or_else(|| PceError::Invalid(format!("unknown IP list {name:?}"))),
        FilterExpr::Workload(name) => {
            let lower = name.to_ascii_lowercase();
            workloads
                .iter()
                .find(|w| {
                    w.hostname
                        .as_deref()
                        .is_some_and(|h| h.eq_ignore_ascii_case(name))
                        || w.name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(name))
                        || w.href.to_ascii_lowercase().ends_with(&lower)
                })
                .map(|w| ActorMatch::Workload {
                    workload: Ref::new(&w.href),
                })
                .ok_or_else(|| {
                    PceError::Invalid(format!(
                        "unknown workload {name:?} (hostname, name or href id)"
                    ))
                })
        }
        FilterExpr::Ip(ip) => Ok(ActorMatch::IpAddress {
            ip_address: ip.clone(),
        }),
        FilterExpr::Fqdn(f) => Ok(ActorMatch::Fqdn { fqdn: f.clone() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActorMatch, IpList, Label, LabelGroup, Ref, Service, Workload};
    use crate::resolve::NameIndex;

    fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    #[test]
    fn parses_each_form() {
        assert_eq!(
            FilterExpr::parse("app=shop").unwrap(),
            FilterExpr::Label {
                key: "app".into(),
                value: "shop".into()
            }
        );
        assert_eq!(
            FilterExpr::parse("env=a=b").unwrap(),
            FilterExpr::Label {
                key: "env".into(),
                value: "a=b".into()
            }
        );
        assert_eq!(
            FilterExpr::parse("10.0.0.1").unwrap(),
            FilterExpr::Ip("10.0.0.1".into())
        );
        assert_eq!(
            FilterExpr::parse("ip:10.0.0.0/8").unwrap(),
            FilterExpr::Ip("10.0.0.0/8".into())
        );
        assert_eq!(
            FilterExpr::parse("ipl:Corp").unwrap(),
            FilterExpr::IpList("Corp".into())
        );
        assert_eq!(
            FilterExpr::parse("wl:host-1.example").unwrap(),
            FilterExpr::Workload("host-1.example".into())
        );
        assert_eq!(
            FilterExpr::parse("fqdn:x.example").unwrap(),
            FilterExpr::Fqdn("x.example".into())
        );
        assert!(FilterExpr::parse("").is_err());
        assert!(FilterExpr::parse("justaword").is_err());
        assert!(FilterExpr::Workload("x".into()).needs_workloads());
        assert!(!FilterExpr::Ip("x".into()).needs_workloads());
    }

    #[test]
    fn resolves_against_index_and_workloads() {
        let idx = index();
        let wls: Vec<Workload> =
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        assert_eq!(
            resolve_actor(&FilterExpr::parse("app=shop").unwrap(), &idx, &wls).unwrap(),
            ActorMatch::Label {
                label: Ref::new("/orgs/5/labels/3")
            }
        );
        assert_eq!(
            resolve_actor(&FilterExpr::parse("ipl:corp").unwrap(), &idx, &wls).unwrap(),
            ActorMatch::IpList {
                ip_list: Ref::new("/orgs/5/sec_policy/draft/ip_lists/21")
            }
        );
        assert_eq!(
            resolve_actor(&FilterExpr::parse("wl:host-2.example").unwrap(), &idx, &wls).unwrap(),
            ActorMatch::Workload {
                workload: Ref::new("/orgs/5/workloads/aaaa-2")
            }
        );
        assert_eq!(
            resolve_actor(&FilterExpr::parse("wl:unmanaged-nas").unwrap(), &idx, &wls).unwrap(),
            ActorMatch::Workload {
                workload: Ref::new("/orgs/5/workloads/aaaa-3")
            }
        );
        assert_eq!(
            resolve_actor(&FilterExpr::parse("1.2.3.4").unwrap(), &idx, &wls).unwrap(),
            ActorMatch::IpAddress {
                ip_address: "1.2.3.4".into()
            }
        );
        let err = resolve_actor(&FilterExpr::parse("app=nope").unwrap(), &idx, &wls)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown label app=nope"), "{err}");
        assert!(err.contains("shop"), "lists known values: {err}");
        let err = resolve_actor(&FilterExpr::parse("ipl:none").unwrap(), &idx, &wls)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown IP list"));
        let err = resolve_actor(&FilterExpr::parse("wl:ghost").unwrap(), &idx, &wls)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown workload"));
    }
}
