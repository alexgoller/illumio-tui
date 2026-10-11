//! Actors (rule providers and consumers, scope members) and rule services
//! as drafts carry them, with the conversions to and from the PCE models.

use super::specs::PortSpec;
use super::{WriteError, checked_href, checked_policy_href};
use crate::models::{Actor, IngressService, Ref};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Label,
    LabelGroup,
    IpList,
    Workload,
    /// Every workload: `{"actors": "ams"}`.
    All,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::Label => "label",
            ActorKind::LabelGroup => "label_group",
            ActorKind::IpList => "ip_list",
            ActorKind::Workload => "workload",
            ActorKind::All => "all",
        }
    }

    /// The PCE excludes labels, label groups and IP lists only.
    pub fn excludable(self) -> bool {
        matches!(
            self,
            ActorKind::Label | ActorKind::LabelGroup | ActorKind::IpList
        )
    }
}

/// One actor of a draft: a provider, a consumer or a scope member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRef {
    pub kind: ActorKind,
    /// `None` only for [`ActorKind::All`].
    pub href: Option<String>,
    pub exclusion: bool,
}

impl ActorRef {
    pub fn new(kind: ActorKind, href: impl Into<String>) -> Self {
        Self {
            kind,
            href: Some(href.into()),
            exclusion: false,
        }
    }

    pub fn all() -> Self {
        Self {
            kind: ActorKind::All,
            href: None,
            exclusion: false,
        }
    }

    pub fn excluded(mut self) -> Self {
        self.exclusion = true;
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        match (self.kind, &self.href) {
            (ActorKind::All, Some(h)) => {
                return Err(format!("the all actor takes no href (got {h})"));
            }
            (ActorKind::All, None) => {}
            (k, None) => return Err(format!("a {} actor needs an href", k.as_str())),
            (_, Some(_)) => {}
        }
        if self.exclusion && self.kind == ActorKind::All {
            return Err("all cannot be excluded".into());
        }
        if self.exclusion && !self.kind.excludable() {
            return Err(format!(
                "only labels, label groups and IP lists can be excluded, not a {}",
                self.kind.as_str()
            ));
        }
        Ok(())
    }

    pub fn to_actor(&self) -> Actor {
        let r = || self.href.clone().map(Ref::new);
        let mut a = Actor::default();
        match self.kind {
            ActorKind::All => a.actors = Some("ams".into()),
            ActorKind::Label => a.label = r(),
            ActorKind::LabelGroup => a.label_group = r(),
            ActorKind::IpList => a.ip_list = r(),
            ActorKind::Workload => a.workload = r(),
        }
        if self.exclusion {
            a.exclusion = Some(true);
        }
        a
    }

    /// The PCE JSON for a request body.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self.to_actor()).unwrap_or(Value::Null)
    }

    /// An actor as the PCE returned it. Virtual services, virtual servers
    /// and `actors` values other than `ams` are refused.
    pub fn from_actor(a: &Actor) -> Result<Self, String> {
        let exclusion = a.exclusion.unwrap_or(false);
        let with = |kind: ActorKind, r: &Ref| -> Result<Self, String> {
            Ok(Self {
                kind,
                href: Some(r.href.clone()),
                exclusion,
            })
        };
        if let Some(s) = &a.actors {
            return if s == "ams" {
                Ok(Self {
                    kind: ActorKind::All,
                    href: None,
                    exclusion,
                })
            } else {
                Err(format!("actors {s:?} is not supported in this version"))
            };
        }
        if a.virtual_service.is_some() {
            return Err("virtual_service actors are not supported in this version".into());
        }
        if a.virtual_server.is_some() {
            return Err("virtual_server actors are not supported in this version".into());
        }
        if let Some(r) = &a.label {
            return with(ActorKind::Label, r);
        }
        if let Some(r) = &a.label_group {
            return with(ActorKind::LabelGroup, r);
        }
        if let Some(r) = &a.ip_list {
            return with(ActorKind::IpList, r);
        }
        if let Some(r) = &a.workload {
            return with(ActorKind::Workload, r);
        }
        Err("actor names no label, label group, IP list, workload or all".into())
    }

    /// An href typed by its components: `/orgs/N/labels/ID`,
    /// `/orgs/N/workloads/ID`, `/orgs/N/sec_policy/<scope>/label_groups/ID`
    /// or `/orgs/N/sec_policy/<scope>/ip_lists/ID`, then checked strictly
    /// (`active` becomes `draft`).
    pub fn from_href(href: &str, org_id: u64) -> Result<Self, WriteError> {
        let parts: Vec<&str> = href.trim().trim_end_matches('/').split('/').collect();
        let (kind, segment) = match parts.as_slice() {
            ["", "orgs", _, "labels", _] => (ActorKind::Label, "labels"),
            ["", "orgs", _, "workloads", _] => (ActorKind::Workload, "workloads"),
            ["", "orgs", _, "sec_policy", _, "label_groups", _] => {
                (ActorKind::LabelGroup, "label_groups")
            }
            ["", "orgs", _, "sec_policy", _, "ip_lists", _] => (ActorKind::IpList, "ip_lists"),
            _ => {
                return Err(WriteError::invalid(format!(
                    "{:?} is not a label, label group, IP list or workload href",
                    href.trim()
                )));
            }
        };
        Ok(Self::new(kind, checked_href(href, segment, org_id)?))
    }

    pub fn same_actor(&self, other: &ActorRef) -> bool {
        self.kind == other.kind && self.href == other.href && self.exclusion == other.exclusion
    }

    /// `label /orgs/5/labels/1`, `!ip_list /orgs/..`, `all`: for messages.
    pub fn display(&self) -> String {
        let base = match &self.href {
            Some(h) => format!("{} {h}", self.kind.as_str()),
            None => self.kind.as_str().to_string(),
        };
        if self.exclusion {
            format!("!{base}")
        } else {
            base
        }
    }
}

/// One service of a rule: a service object or an inline port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceRef {
    Href(String),
    Port(PortSpec),
}

impl ServiceRef {
    pub fn to_ingress(&self) -> IngressService {
        match self {
            ServiceRef::Href(h) => IngressService::Ref(Ref::new(h.clone())),
            ServiceRef::Port(p) => IngressService::Port {
                port: p.port,
                to_port: p.to_port,
                proto: p.proto,
            },
        }
    }

    pub fn from_ingress(s: &IngressService) -> Self {
        match s {
            IngressService::Ref(r) => ServiceRef::Href(r.href.clone()),
            IngressService::Port {
                port,
                to_port,
                proto,
            } => ServiceRef::Port(PortSpec {
                port: *port,
                to_port: to_port.filter(|t| Some(*t) != *port),
                proto: *proto,
            }),
        }
    }

    pub fn from_href(href: &str, org_id: u64) -> Result<Self, WriteError> {
        checked_policy_href(href, "services", org_id).map(ServiceRef::Href)
    }

    /// Hrefs compare exactly; inline ports through [`PortSpec`].
    pub fn same_service(&self, other: &Self) -> bool {
        self == other
    }

    pub fn to_json(&self) -> Value {
        match self {
            ServiceRef::Href(h) => serde_json::json!({ "href": h }),
            ServiceRef::Port(p) => p.to_json(),
        }
    }

    pub fn display(&self) -> String {
        match self {
            ServiceRef::Href(h) => format!("service {h}"),
            ServiceRef::Port(p) => p.display(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleSet;
    use serde_json::json;

    fn fixture() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    #[test]
    fn actors_serialise_in_pce_form() {
        assert_eq!(ActorRef::all().to_json(), json!({"actors": "ams"}));
        assert_eq!(
            ActorRef::new(ActorKind::Label, "/orgs/5/labels/1").to_json(),
            json!({"label": {"href": "/orgs/5/labels/1"}})
        );
        assert_eq!(
            ActorRef::new(
                ActorKind::LabelGroup,
                "/orgs/5/sec_policy/draft/label_groups/10"
            )
            .to_json(),
            json!({"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}})
        );
        assert_eq!(
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/20")
                .excluded()
                .to_json(),
            json!({"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}, "exclusion": true})
        );
        assert_eq!(
            ActorRef::new(ActorKind::Workload, "/orgs/5/workloads/aaaa-1").to_json(),
            json!({"workload": {"href": "/orgs/5/workloads/aaaa-1"}})
        );
    }

    #[test]
    fn fixture_actors_convert_both_ways() {
        let rs = fixture();
        let ams = ActorRef::from_actor(&rs[1].rules[0].providers[0]).unwrap();
        assert_eq!(ams, ActorRef::all());
        let group = ActorRef::from_actor(&rs[1].rules[0].consumers[0]).unwrap();
        assert_eq!(
            group,
            ActorRef::new(
                ActorKind::LabelGroup,
                "/orgs/5/sec_policy/draft/label_groups/10"
            )
        );
        let ipl = ActorRef::from_actor(&rs[0].rules[1].consumers[0]).unwrap();
        assert_eq!(
            ipl,
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/21")
        );
        for a in [&ams, &group, &ipl] {
            assert_eq!(&ActorRef::from_actor(&a.to_actor()).unwrap(), a);
        }
        let scope = ActorRef::from_actor(&rs[0].scopes[0][0]).unwrap();
        assert_eq!(scope, ActorRef::new(ActorKind::Label, "/orgs/5/labels/3"));
    }

    #[test]
    fn unsupported_actors_are_refused_with_the_spec_message() {
        let vs: Actor = serde_json::from_value(
            json!({"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/1"}}),
        )
        .unwrap();
        assert_eq!(
            ActorRef::from_actor(&vs).unwrap_err(),
            "virtual_service actors are not supported in this version"
        );
        let vsrv: Actor = serde_json::from_value(
            json!({"virtual_server": {"href": "/orgs/5/sec_policy/draft/virtual_servers/1"}}),
        )
        .unwrap();
        assert_eq!(
            ActorRef::from_actor(&vsrv).unwrap_err(),
            "virtual_server actors are not supported in this version"
        );
        let odd: Actor = serde_json::from_value(json!({"actors": "container_hosts"})).unwrap();
        assert!(
            ActorRef::from_actor(&odd)
                .unwrap_err()
                .contains("container_hosts")
        );
        assert!(ActorRef::from_actor(&Actor::default()).is_err());
    }

    #[test]
    fn same_actor_compares_kind_href_and_exclusion() {
        let a = ActorRef::new(ActorKind::Label, "/orgs/5/labels/1");
        assert!(a.same_actor(&a.clone()));
        assert!(!a.same_actor(&a.clone().excluded()));
        assert!(!a.same_actor(&ActorRef::new(ActorKind::Label, "/orgs/5/labels/2")));
        assert!(
            !ActorRef::new(ActorKind::LabelGroup, "/x/1")
                .same_actor(&ActorRef::new(ActorKind::IpList, "/x/1"))
        );
        // An explicit "exclusion": false from the PCE equals no flag.
        let explicit: Actor = serde_json::from_value(
            json!({"label": {"href": "/orgs/5/labels/1"}, "exclusion": false}),
        )
        .unwrap();
        assert!(ActorRef::from_actor(&explicit).unwrap().same_actor(&a));
    }

    #[test]
    fn validation_refuses_excluded_all_and_excluded_workloads() {
        assert_eq!(
            ActorRef::all().excluded().validate().unwrap_err(),
            "all cannot be excluded"
        );
        assert_eq!(
            ActorRef::new(ActorKind::Workload, "/orgs/5/workloads/aaaa-1")
                .excluded()
                .validate()
                .unwrap_err(),
            "only labels, label groups and IP lists can be excluded, not a workload"
        );
        assert!(
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/20")
                .excluded()
                .validate()
                .is_ok()
        );
        let no_href = ActorRef {
            kind: ActorKind::Label,
            href: None,
            exclusion: false,
        };
        assert_eq!(
            no_href.validate().unwrap_err(),
            "a label actor needs an href"
        );
        let all_href = ActorRef {
            kind: ActorKind::All,
            href: Some("/x".into()),
            exclusion: false,
        };
        assert_eq!(
            all_href.validate().unwrap_err(),
            "the all actor takes no href (got /x)"
        );
    }

    #[test]
    fn hrefs_pick_the_actor_kind_by_component() {
        assert_eq!(
            ActorRef::from_href("/orgs/5/labels/1", 5).unwrap(),
            ActorRef::new(ActorKind::Label, "/orgs/5/labels/1")
        );
        assert_eq!(
            ActorRef::from_href("/orgs/5/workloads/aaaa-1", 5)
                .unwrap()
                .kind,
            ActorKind::Workload
        );
        assert_eq!(
            ActorRef::from_href("/orgs/5/sec_policy/active/ip_lists/21", 5).unwrap(),
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/21")
        );
        assert_eq!(
            ActorRef::from_href("/orgs/5/sec_policy/draft/label_groups/10", 5)
                .unwrap()
                .kind,
            ActorKind::LabelGroup
        );
        for bad in [
            "/orgs/5/sec_policy/draft/labels/1",
            "/orgs/5/ip_lists/21",
            "/orgs/5/sec_policy/draft/services/31",
            "/orgs/5/labels/../workloads/1",
            "/orgs/5/labels/1/2",
            "/orgs/5/labels/%2e%2e",
        ] {
            assert!(ActorRef::from_href(bad, 5).is_err(), "{bad}");
        }
        let err = ActorRef::from_href("/orgs/7/labels/1", 5).unwrap_err();
        assert!(err.to_string().contains("org 7"), "{err}");
    }

    #[test]
    fn services_convert_and_compare_by_port_spec() {
        let rs = fixture();
        let inline = &rs[0].rules[1].ingress_services;
        assert_eq!(ServiceRef::from_ingress(&inline[0]), port("443/tcp"));
        assert!(ServiceRef::from_ingress(&inline[1]).same_service(&port("8000-8099/tcp")));
        assert!(!ServiceRef::from_ingress(&inline[1]).same_service(&port("8000-8099/udp")));
        let href = ServiceRef::from_ingress(&rs[0].rules[0].ingress_services[0]);
        assert_eq!(
            href,
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        for s in [&href, &port("icmp"), &port("8000-8099/tcp")] {
            assert_eq!(&ServiceRef::from_ingress(&s.to_ingress()), s);
        }
        // A to_port equal to port is the single port.
        let same = IngressService::Port {
            port: Some(53),
            to_port: Some(53),
            proto: 17,
        };
        assert_eq!(ServiceRef::from_ingress(&same), port("53/udp"));
        assert_eq!(
            port("8000-8099/tcp").to_json(),
            json!({"port": 8000, "to_port": 8099, "proto": 6})
        );
        assert_eq!(
            href.to_json(),
            json!({"href": "/orgs/5/sec_policy/draft/services/31"})
        );
        assert_eq!(port("443/tcp").display(), "443/tcp");
        assert_eq!(
            href.display(),
            "service /orgs/5/sec_policy/draft/services/31"
        );
        assert_eq!(
            ServiceRef::from_href("/orgs/5/sec_policy/active/services/32", 5).unwrap(),
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/32".into())
        );
        assert!(ServiceRef::from_href("/orgs/5/services/32", 5).is_err());
    }
}
