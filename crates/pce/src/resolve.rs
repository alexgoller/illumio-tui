//! Turns PCE hrefs into names. Built once from the four object lists and
//! shared (via `Arc`) by every screen and command. Never makes requests.
use crate::KeyOrder;
use crate::models::{IpList, Label, LabelGroup, Ref, Service};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Active policy references `/sec_policy/active/<type>/<id>`; the index is
/// built from draft lists and ids are the same in both, so every policy
/// href is keyed by its draft form.
pub(crate) fn canon(href: &str) -> Cow<'_, str> {
    const ACTIVE: &str = "/sec_policy/active/";
    if href.contains(ACTIVE) {
        Cow::Owned(href.replacen(ACTIVE, "/sec_policy/draft/", 1))
    } else {
        Cow::Borrowed(href)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedLabel {
    pub href: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct NameIndex {
    labels: HashMap<String, Label>,
    label_by_kv: HashMap<(String, String), String>,
    keys: BTreeMap<String, BTreeSet<String>>,
    label_groups: HashMap<String, LabelGroup>,
    label_group_by_name: HashMap<String, String>,
    ip_lists: HashMap<String, IpList>,
    ip_list_by_name: HashMap<String, String>,
    services: HashMap<String, Service>,
    order: KeyOrder,
}

impl NameIndex {
    pub fn new(
        labels: &[Label],
        groups: &[LabelGroup],
        ip_lists: &[IpList],
        services: &[Service],
    ) -> Self {
        let mut idx = Self::default();
        idx.insert_labels(labels);
        for g in groups {
            idx.label_group_by_name
                .insert(g.name.to_ascii_lowercase(), g.href.clone());
            idx.label_groups
                .insert(canon(&g.href).into_owned(), g.clone());
        }
        idx.insert_ip_lists(ip_lists);
        for s in services {
            idx.services.insert(canon(&s.href).into_owned(), s.clone());
        }
        idx
    }

    /// This index with its labels replaced by `labels`; label groups, IP
    /// lists and services are kept. For a caller that reloaded only the
    /// labels (the TUI after a label write or `r` on Labels), so the index
    /// follows without fetching the other three lists again.
    pub fn with_labels(&self, labels: &[Label]) -> Self {
        let mut idx = Self {
            label_groups: self.label_groups.clone(),
            label_group_by_name: self.label_group_by_name.clone(),
            ip_lists: self.ip_lists.clone(),
            ip_list_by_name: self.ip_list_by_name.clone(),
            services: self.services.clone(),
            order: self.order.clone(),
            ..Self::default()
        };
        idx.insert_labels(labels);
        idx
    }

    /// This index with its IP lists replaced by `lists`; labels, label
    /// groups and services are kept. For the TUI after an IP list reload
    /// (screen 6, a write), so rule impact sizes follow at no extra request.
    pub fn with_ip_lists(&self, lists: &[IpList]) -> Self {
        let mut idx = Self {
            labels: self.labels.clone(),
            label_by_kv: self.label_by_kv.clone(),
            keys: self.keys.clone(),
            label_groups: self.label_groups.clone(),
            label_group_by_name: self.label_group_by_name.clone(),
            services: self.services.clone(),
            order: self.order.clone(),
            ..Self::default()
        };
        idx.insert_ip_lists(lists);
        idx
    }

    /// This index with labels shown in `order` (see [`KeyOrder`]).
    pub fn with_key_order(mut self, order: KeyOrder) -> Self {
        self.order = order;
        self
    }

    /// The order label keys are shown in.
    pub fn key_order(&self) -> &KeyOrder {
        &self.order
    }

    fn insert_ip_lists(&mut self, lists: &[IpList]) {
        for i in lists {
            self.ip_list_by_name
                .insert(i.name.to_ascii_lowercase(), i.href.clone());
            self.ip_lists.insert(canon(&i.href).into_owned(), i.clone());
        }
    }

    fn insert_labels(&mut self, labels: &[Label]) {
        for l in labels {
            self.labels.insert(l.href.clone(), l.clone());
            if !l.is_deleted() {
                self.label_by_kv
                    .insert((l.key.clone(), l.value.clone()), l.href.clone());
                self.keys
                    .entry(l.key.clone())
                    .or_default()
                    .insert(l.value.clone());
            }
        }
    }

    pub fn label(&self, href: &str) -> Option<&Label> {
        self.labels.get(href)
    }

    /// `key=value`, or the href itself when unknown.
    pub fn label_text(&self, href: &str) -> String {
        self.labels
            .get(href)
            .map(Label::display)
            .unwrap_or_else(|| href.to_string())
    }

    pub fn label_href(&self, key: &str, value: &str) -> Option<&str> {
        self.label_by_kv
            .get(&(key.to_string(), value.to_string()))
            .map(String::as_str)
    }

    /// Every label key, in the index's [`KeyOrder`].
    pub fn label_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.keys.keys().cloned().collect();
        keys.sort_by(|a, b| self.order.cmp_keys(a, b));
        keys
    }

    /// The values of `key`, sorted case-insensitively (like `ip_list_names`).
    pub fn values_for_key(&self, key: &str) -> Vec<String> {
        let mut values: Vec<String> = self
            .keys
            .get(key)
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default();
        values.sort_by_key(|s| s.to_lowercase());
        values
    }

    /// A label group's href by name, case-insensitively (like `ip_list_href`).
    pub fn label_group_href(&self, name: &str) -> Option<&str> {
        self.label_group_by_name
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Label group names, sorted case-insensitively (like `ip_list_names`).
    pub fn label_group_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.label_groups.values().map(|g| g.name.clone()).collect();
        names.sort_by_key(|n| n.to_ascii_lowercase());
        names
    }

    pub fn label_group_name(&self, href: &str) -> Option<&str> {
        self.label_groups
            .get(canon(href).as_ref())
            .map(|g| g.name.as_str())
    }

    pub fn ip_list_name(&self, href: &str) -> Option<&str> {
        self.ip_lists
            .get(canon(href).as_ref())
            .map(|l| l.name.as_str())
    }

    /// The label group with this href; an active href finds its draft.
    pub fn label_group(&self, href: &str) -> Option<&LabelGroup> {
        self.label_groups.get(canon(href).as_ref())
    }

    /// The IP list with this href; an active href finds its draft.
    pub fn ip_list(&self, href: &str) -> Option<&IpList> {
        self.ip_lists.get(canon(href).as_ref())
    }

    pub fn ip_list_href(&self, name: &str) -> Option<&str> {
        self.ip_list_by_name
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// IP list names, sorted case-insensitively.
    pub fn ip_list_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.ip_lists.values().map(|l| l.name.clone()).collect();
        names.sort_by_key(|n| n.to_ascii_lowercase());
        names
    }

    pub fn service(&self, href: &str) -> Option<&Service> {
        self.services.get(canon(href).as_ref())
    }

    pub fn service_name(&self, href: &str) -> Option<&str> {
        self.service(href).map(|s| s.name.as_str())
    }

    /// The live services (pending deletions excluded), by name,
    /// case-insensitively, then href.
    pub fn services(&self) -> Vec<&Service> {
        let mut all: Vec<&Service> = self
            .services
            .values()
            .filter(|s| s.update_type.as_deref() != Some("delete"))
            .collect();
        all.sort_by_cached_key(|s| (s.name.to_ascii_lowercase(), s.href.clone()));
        all
    }

    /// The service object a rule uses for icmp and icmpv6 (a rule's inline
    /// services take tcp and udp only): the built-in ICMP service
    /// (`ilo_defaults`/`ICMP`, whatever its name), else the first service
    /// whose ports are all proto 1 or 58 without a port, else one named
    /// `ICMP`.
    pub fn icmp_service(&self) -> Option<&Service> {
        let all = self.services();
        let by = |f: &dyn Fn(&Service) -> bool| all.iter().copied().find(|s| f(s));
        by(&|s| {
            s.external_data_set.as_deref() == Some("com.illumio.ilo_defaults")
                && s.external_data_reference.as_deref() == Some("ICMP")
        })
        .or_else(|| {
            by(&|s| {
                !s.service_ports.is_empty()
                    && s.service_ports
                        .iter()
                        .all(|p| matches!(p.proto, Some(1 | 58)) && p.port.is_none())
            })
        })
        .or_else(|| by(&|s| s.name.eq_ignore_ascii_case("ICMP")))
    }

    pub fn icmp_service_href(&self) -> Option<&str> {
        self.icmp_service().map(|s| s.href.as_str())
    }

    /// The one live service named exactly `name` (like the CLI's lookup:
    /// case-sensitive, pending deletions excluded). `None` when there is no
    /// such service, or several.
    pub fn service_href(&self, name: &str) -> Option<&str> {
        let mut hits = self
            .services
            .values()
            .filter(|s| s.name == name && s.update_type.as_deref() != Some("delete"));
        let first = hits.next()?;
        hits.next().is_none().then_some(first.href.as_str())
    }

    /// Resolves label refs, sorted by the index's [`KeyOrder`], then value.
    /// Unknown hrefs get key `?` (sorted last) and the href as value so
    /// nothing silently disappears.
    pub fn resolve_labels(&self, refs: &[Ref]) -> Vec<ResolvedLabel> {
        let mut out: Vec<ResolvedLabel> = refs
            .iter()
            .map(|r| match self.labels.get(&r.href) {
                Some(l) => ResolvedLabel {
                    href: r.href.clone(),
                    key: l.key.clone(),
                    value: l.value.clone(),
                },
                None => ResolvedLabel {
                    href: r.href.clone(),
                    key: "?".into(),
                    value: r.href.clone(),
                },
            })
            .collect();
        self.order.sort_labels(&mut out);
        out
    }
}

use crate::models::{Actor, DenyRule, IngressService, Rule, RuleSet};

/// One side of a rule with its href kept next to the display name, so JSON
/// consumers can follow the reference and tables can print the name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedActor {
    /// `label`, `label_group`, `ip_list`, `workload`, `virtual_service`,
    /// `virtual_server`, `all_workloads` or `unknown`.
    pub kind: String,
    pub href: Option<String>,
    /// `role=web`, `ipl:Corp`, `group:All prod envs`, `wl:<id>`,
    /// `All workloads`; prefixed with `!` when `exclusion` is set.
    pub name: String,
    pub exclusion: bool,
}

impl ResolvedActor {
    pub fn from_actor(a: &Actor, idx: &NameIndex) -> Self {
        let exclusion = a.exclusion.unwrap_or(false);
        let (kind, href, body) = if a.actors.as_deref() == Some("ams") {
            ("all_workloads", None, "All workloads".to_string())
        } else if let Some(l) = &a.label {
            ("label", Some(&l.href), idx.label_text(&l.href))
        } else if let Some(g) = &a.label_group {
            (
                "label_group",
                Some(&g.href),
                format!("group:{}", idx.label_group_name(&g.href).unwrap_or(&g.href)),
            )
        } else if let Some(i) = &a.ip_list {
            (
                "ip_list",
                Some(&i.href),
                format!("ipl:{}", idx.ip_list_name(&i.href).unwrap_or(&i.href)),
            )
        } else if let Some(w) = &a.workload {
            ("workload", Some(&w.href), format!("wl:{}", w.id()))
        } else if let Some(v) = &a.virtual_service {
            ("virtual_service", Some(&v.href), format!("vs:{}", v.id()))
        } else if let Some(v) = &a.virtual_server {
            ("virtual_server", Some(&v.href), format!("vsrv:{}", v.id()))
        } else {
            ("unknown", None, "?".to_string())
        };
        Self {
            kind: kind.to_string(),
            href: href.cloned(),
            name: if exclusion { format!("!{body}") } else { body },
            exclusion,
        }
    }
}

/// A rule's service: the service object's href and name, or an inline
/// port/protocol with no href.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedService {
    pub href: Option<String>,
    /// `PostgreSQL`, `443/tcp`, `8000-8099/tcp`, `icmp`.
    pub name: String,
}

impl ResolvedService {
    pub fn from_ingress_service(s: &IngressService, idx: &NameIndex) -> Self {
        match s {
            IngressService::Ref(r) => Self {
                href: Some(r.href.clone()),
                name: idx.service_name(&r.href).unwrap_or(&r.href).to_string(),
            },
            IngressService::Port {
                port,
                to_port,
                proto,
            } => {
                let proto = crate::models::proto_name(*proto);
                let name = match (port, to_port) {
                    (Some(p), Some(t)) if t != p => format!("{p}-{t}/{proto}"),
                    (Some(p), _) => format!("{p}/{proto}"),
                    (None, _) => proto,
                };
                Self { href: None, name }
            }
        }
    }
}

/// `role=web`, `ipl:Corp`, `group:All prod envs`, `wl:<id>`, `All workloads`.
pub fn describe_actor(a: &Actor, idx: &NameIndex) -> String {
    ResolvedActor::from_actor(a, idx).name
}

pub fn describe_ingress_service(s: &IngressService, idx: &NameIndex) -> String {
    ResolvedService::from_ingress_service(s, idx).name
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Allow,
    Deny,
    OverrideDeny,
}

/// A rule with every href turned into a name. What the CLI prints and the
/// TUI lists.
#[derive(Debug, Clone, Serialize)]
pub struct RuleView {
    pub href: String,
    pub kind: RuleKind,
    pub enabled: bool,
    pub description: Option<String>,
    pub consumers: Vec<ResolvedActor>,
    pub providers: Vec<ResolvedActor>,
    pub services: Vec<ResolvedService>,
    pub unscoped_consumers: bool,
    pub update_type: Option<String>,
}

impl RuleView {
    pub fn from_rule(r: &Rule, idx: &NameIndex) -> Self {
        Self {
            href: r.href.clone(),
            kind: RuleKind::Allow,
            enabled: r.enabled,
            description: r.description.clone(),
            consumers: r
                .consumers
                .iter()
                .map(|a| ResolvedActor::from_actor(a, idx))
                .collect(),
            providers: r
                .providers
                .iter()
                .map(|a| ResolvedActor::from_actor(a, idx))
                .collect(),
            services: r
                .ingress_services
                .iter()
                .map(|s| ResolvedService::from_ingress_service(s, idx))
                .collect(),
            unscoped_consumers: r.unscoped_consumers.unwrap_or(false),
            update_type: r.update_type.clone(),
        }
    }

    pub fn from_deny(d: &DenyRule, idx: &NameIndex) -> Self {
        Self {
            href: d.href.clone(),
            kind: if d.override_ {
                RuleKind::OverrideDeny
            } else {
                RuleKind::Deny
            },
            enabled: d.enabled,
            description: d.description.clone(),
            consumers: d
                .consumers
                .iter()
                .map(|a| ResolvedActor::from_actor(a, idx))
                .collect(),
            providers: d
                .providers
                .iter()
                .map(|a| ResolvedActor::from_actor(a, idx))
                .collect(),
            services: d
                .ingress_services
                .iter()
                .map(|s| ResolvedService::from_ingress_service(s, idx))
                .collect(),
            unscoped_consumers: false,
            update_type: d.update_type.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleSetView {
    pub href: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// One string per scope, `app=shop & env=prod`; `All` for an empty scope.
    pub scopes: Vec<String>,
    /// Allow plus deny rules.
    pub rule_count: usize,
    pub deny_count: usize,
    pub update_type: Option<String>,
}

impl RuleSetView {
    pub fn from_rule_set(rs: &RuleSet, idx: &NameIndex) -> Self {
        let scopes = if rs.scopes.is_empty() {
            vec!["All".to_string()]
        } else {
            rs.scopes
                .iter()
                .map(|scope| {
                    if scope.is_empty() {
                        "All".to_string()
                    } else {
                        scope
                            .iter()
                            .map(|a| describe_actor(a, idx))
                            .collect::<Vec<_>>()
                            .join(" & ")
                    }
                })
                .collect()
        };
        Self {
            href: rs.href.clone(),
            name: rs.name.clone(),
            description: rs.description.clone(),
            enabled: rs.enabled,
            scopes,
            rule_count: rs.rules.len() + rs.deny_rules.len(),
            deny_count: rs.deny_rules.len(),
            update_type: rs.update_type.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, Ref, Service};

    pub(crate) fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    #[test]
    fn icmp_service_prefers_the_builtin_then_all_icmp_ports_then_the_name() {
        let svc = |n: u32, name: &str, extra: serde_json::Value| -> Service {
            let mut v = serde_json::json!({
                "href": format!("/orgs/5/sec_policy/draft/services/{n}"), "name": name,
                "service_ports": [{"proto": 1}]
            });
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value(v).unwrap()
        };
        let builtin = serde_json::json!({
            "external_data_set": "com.illumio.ilo_defaults", "external_data_reference": "ICMP"
        });
        let none = serde_json::json!({});
        // The fixture services hold no ICMP service at all.
        assert_eq!(index().icmp_service_href(), None);
        let named = svc(1, "icmp", none.clone());
        let ports = svc(2, "Ping", none.clone());
        let tcp = Service {
            service_ports: vec![],
            ..svc(3, "ICMP-ish", none.clone())
        };
        let ilo = svc(4, "S-ICMP", builtin);
        let idx = |v: &[&Service]| {
            NameIndex::new(
                &[],
                &[],
                &[],
                &v.iter().map(|s| (*s).clone()).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            idx(&[&named, &ports, &tcp, &ilo])
                .icmp_service()
                .unwrap()
                .name,
            "S-ICMP"
        );
        assert_eq!(
            idx(&[&named, &ports, &tcp]).icmp_service().unwrap().name,
            "icmp"
        );
        assert_eq!(idx(&[&ports, &tcp]).icmp_service().unwrap().name, "Ping");
        let by_name = Service {
            service_ports: vec![],
            ..named
        };
        assert_eq!(idx(&[&by_name, &tcp]).icmp_service().unwrap().name, "icmp");
        assert_eq!(idx(&[&tcp]).icmp_service(), None);
    }

    #[test]
    fn resolves_labels_both_ways() {
        let idx = index();
        assert_eq!(idx.label_text("/orgs/5/labels/1"), "role=web");
        assert_eq!(idx.label_text("/orgs/5/labels/999"), "/orgs/5/labels/999");
        assert_eq!(idx.label_href("app", "shop"), Some("/orgs/5/labels/3"));
        assert_eq!(idx.label_href("app", "nope"), None);
        assert_eq!(idx.label_keys(), vec!["app", "env", "loc", "role"]);
        assert_eq!(idx.values_for_key("role"), vec!["db", "web"]);
    }

    #[test]
    fn deleted_label_without_value_is_indexed_but_not_matchable_by_value() {
        let idx = index();
        assert_eq!(idx.label_text("/orgs/5/labels/6"), "app=");
        assert_eq!(
            idx.label_href("app", ""),
            None,
            "deleted labels never match filters"
        );
    }

    #[test]
    fn with_labels_replaces_the_labels_and_keeps_the_rest() {
        let idx = index();
        let mut labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        labels.push(
            serde_json::from_value(serde_json::json!({
                "href": "/orgs/5/labels/9", "key": "app", "value": "web2"
            }))
            .unwrap(),
        );
        let next = idx.with_labels(&labels);
        assert_eq!(next.label_href("app", "web2"), Some("/orgs/5/labels/9"));
        assert_eq!(next.label_text("/orgs/5/labels/9"), "app=web2");
        assert_eq!(next.values_for_key("app"), vec!["shop", "web2"]);
        assert_eq!(next.label_href("app", "shop"), Some("/orgs/5/labels/3"));
        assert_eq!(
            idx.label_href("app", "web2"),
            None,
            "the original is unchanged"
        );
        // Groups, IP lists and services are carried over as they were.
        let fresh = NameIndex::new(&labels, &[], &[], &[]);
        assert_eq!(next.label_groups.len(), idx.label_groups.len());
        assert!(!next.label_groups.is_empty() && fresh.label_groups.is_empty());
        assert_eq!(next.ip_lists.len(), idx.ip_lists.len());
        assert_eq!(next.ip_list_by_name, idx.ip_list_by_name);
        assert_eq!(next.services.len(), idx.services.len());
        assert!(!next.services.is_empty());
        // Labels gone from the list are gone from the index.
        let only_new = idx.with_labels(&labels[labels.len() - 1..]);
        assert_eq!(only_new.label_href("app", "shop"), None);
        assert_eq!(only_new.label_keys(), vec!["app"]);
    }

    #[test]
    fn with_ip_lists_replaces_the_lists_and_keeps_the_rest() {
        let idx = index();
        let edited: IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/active/ip_lists/21", "name": "Corp HQ",
            "ip_ranges": [{"from_ip": "10.1.0.0/24"}]
        }))
        .unwrap();
        let next = idx.with_ip_lists(std::slice::from_ref(&edited));
        let corp = next
            .ip_list("/orgs/5/sec_policy/draft/ip_lists/21")
            .unwrap();
        assert_eq!(corp.ip_ranges.len(), 1);
        assert_eq!(corp.ip_ranges[0].from_ip, "10.1.0.0/24");
        assert_eq!(
            next.ip_list_href("corp hq"),
            Some("/orgs/5/sec_policy/active/ip_lists/21")
        );
        assert_eq!(next.ip_list_href("corp"), None, "the old name is gone");
        assert_eq!(
            next.ip_lists.len(),
            1,
            "lists gone from the reload are gone"
        );
        assert_eq!(next.labels.len(), idx.labels.len());
        assert_eq!(next.label_by_kv, idx.label_by_kv);
        assert_eq!(next.keys, idx.keys);
        assert_eq!(next.label_groups.len(), idx.label_groups.len());
        assert_eq!(next.label_group_by_name, idx.label_group_by_name);
        assert_eq!(next.services.len(), idx.services.len());
        assert!(
            idx.ip_list_href("corp").is_some(),
            "the original is unchanged"
        );
    }

    #[test]
    fn resolves_groups_lists_services() {
        let idx = index();
        assert_eq!(
            idx.label_group_name("/orgs/5/sec_policy/draft/label_groups/10"),
            Some("All prod envs")
        );
        assert_eq!(
            idx.ip_list_name("/orgs/5/sec_policy/draft/ip_lists/21"),
            Some("Corp")
        );
        assert_eq!(
            idx.ip_list_href("corp"),
            Some("/orgs/5/sec_policy/draft/ip_lists/21")
        );
        assert_eq!(
            idx.ip_list_names(),
            vec!["Any (0.0.0.0/0 and ::/0)", "Corp"]
        );
        assert_eq!(idx.label_group_names(), vec!["All prod envs"]);
        assert_eq!(
            idx.service_name("/orgs/5/sec_policy/draft/services/31"),
            Some("PostgreSQL")
        );
        assert_eq!(idx.service_name("/x"), None);
    }

    #[test]
    fn active_policy_hrefs_resolve_to_draft_names() {
        let idx = index();
        assert_eq!(
            idx.ip_list_name("/orgs/5/sec_policy/active/ip_lists/21"),
            Some("Corp")
        );
        assert_eq!(
            idx.service_name("/orgs/5/sec_policy/active/services/31"),
            Some("PostgreSQL")
        );
        assert_eq!(
            idx.label_group_name("/orgs/5/sec_policy/active/label_groups/10"),
            Some("All prod envs")
        );
        assert_eq!(
            idx.ip_list_name("/orgs/5/sec_policy/active/ip_lists/99"),
            None
        );
        assert_eq!(canon("/orgs/5/labels/1"), "/orgs/5/labels/1");
    }

    #[test]
    fn resolve_labels_sorts_by_key_and_keeps_unknown_hrefs() {
        let idx = index();
        let out = idx.resolve_labels(&[
            Ref::new("/orgs/5/labels/4"),
            Ref::new("/orgs/5/labels/1"),
            Ref::new("/orgs/5/labels/77"),
        ]);
        let text: Vec<String> = out
            .iter()
            .map(|l| format!("{}={}", l.key, l.value))
            .collect();
        assert_eq!(text, vec!["env=prod", "role=web", "?=/orgs/5/labels/77"]);
    }

    fn bu_label() -> Label {
        serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/labels/50", "key": "bu", "value": "x"
        }))
        .unwrap()
    }

    fn ordered(idx: NameIndex) -> NameIndex {
        idx.with_key_order(KeyOrder::new(&["app".into(), "env".into()], "role"))
    }

    #[test]
    fn resolve_labels_follows_the_key_order() {
        let mut labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        labels.push(bu_label());
        let idx = ordered(NameIndex::new(&labels, &[], &[], &[]));
        let out = idx.resolve_labels(&[
            Ref::new("/orgs/5/labels/1"),
            Ref::new("/orgs/5/labels/50"),
            Ref::new("/orgs/5/labels/3"),
            Ref::new("/orgs/5/labels/4"),
        ]);
        let text: Vec<String> = out
            .iter()
            .map(|l| format!("{}={}", l.key, l.value))
            .collect();
        assert_eq!(text, vec!["app=shop", "env=prod", "role=web", "bu=x"]);
        assert_eq!(idx.label_keys(), vec!["app", "env", "role", "bu", "loc"]);
    }

    #[test]
    fn resolve_labels_keeps_ignored_keys() {
        let mut labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        labels.push(bu_label());
        let idx = NameIndex::new(&labels, &[], &[], &[])
            .with_key_order(KeyOrder::new(&["app".into()], "role").with_ignored(&["bu".into()]));
        let out =
            idx.resolve_labels(&[Ref::new("/orgs/5/labels/1"), Ref::new("/orgs/5/labels/50")]);
        let text: Vec<String> = out
            .iter()
            .map(|l| format!("{}={}", l.key, l.value))
            .collect();
        assert_eq!(text, vec!["role=web", "bu=x"]);
        assert_eq!(idx.key_order().ignored(), ["bu".to_string()]);
        assert!(idx.label_keys().contains(&"bu".to_string()));
    }

    #[test]
    fn with_labels_and_with_ip_lists_keep_the_key_order() {
        let idx = ordered(index());
        let mut labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        labels.push(bu_label());
        let next = idx.with_labels(&labels);
        assert_eq!(next.key_order(), idx.key_order());
        assert_eq!(next.label_keys(), vec!["app", "env", "role", "bu", "loc"]);
        let lists = idx.with_ip_lists(&[]);
        assert_eq!(lists.key_order(), idx.key_order());
        assert_eq!(lists.label_keys(), vec!["app", "env", "role", "loc"]);
    }

    #[test]
    fn portless_ingress_service_renders_protocol_only() {
        let idx = index();
        let s: Vec<IngressService> = serde_json::from_str(r#"[{"proto": 1}]"#).unwrap();
        assert_eq!(describe_ingress_service(&s[0], &idx), "icmp");
        let s = IngressService::Port {
            port: Some(53),
            to_port: Some(53),
            proto: 17,
        };
        assert_eq!(describe_ingress_service(&s, &idx), "53/udp");
    }

    #[test]
    fn rule_views_render_names() {
        use crate::models::{DenyRule, RuleSet};
        let idx = index();
        let rs: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        let names = |actors: &[ResolvedActor]| -> Vec<String> {
            actors.iter().map(|a| a.name.clone()).collect()
        };
        let v = RuleView::from_rule(&rs[0].rules[1], &idx);
        assert_eq!(
            v.consumers,
            vec![ResolvedActor {
                kind: "ip_list".into(),
                href: Some("/orgs/5/sec_policy/draft/ip_lists/21".into()),
                name: "ipl:Corp".into(),
                exclusion: false,
            }]
        );
        assert_eq!(names(&v.providers), vec!["role=web"]);
        assert_eq!(v.providers[0].kind, "label");
        assert_eq!(v.providers[0].href.as_deref(), Some("/orgs/5/labels/1"));
        assert_eq!(
            v.services,
            vec![
                ResolvedService {
                    href: None,
                    name: "443/tcp".into()
                },
                ResolvedService {
                    href: None,
                    name: "8000-8099/tcp".into()
                }
            ]
        );
        assert_eq!(v.kind, RuleKind::Allow);
        assert!(v.unscoped_consumers);
        let v1 = RuleView::from_rule(&rs[0].rules[0], &idx);
        assert_eq!(
            v1.services,
            vec![ResolvedService {
                href: Some("/orgs/5/sec_policy/draft/services/31".into()),
                name: "PostgreSQL".into()
            }]
        );
        let v3 = RuleView::from_rule(&rs[1].rules[0], &idx);
        assert_eq!(
            v3.providers,
            vec![ResolvedActor {
                kind: "all_workloads".into(),
                href: None,
                name: "All workloads".into(),
                exclusion: false,
            }]
        );
        assert_eq!(names(&v3.consumers), vec!["group:All prod envs"]);
        assert_eq!(v3.consumers[0].kind, "label_group");
        assert_eq!(v3.services[0].name, "All Services");
        assert_eq!(
            v3.services[0].href.as_deref(),
            Some("/orgs/5/sec_policy/draft/services/30")
        );
        let dr: Vec<DenyRule> =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        assert_eq!(
            RuleView::from_deny(&dr[1], &idx).kind,
            RuleKind::OverrideDeny
        );
        assert_eq!(
            names(&RuleView::from_deny(&dr[0], &idx).consumers),
            vec!["ipl:Any (0.0.0.0/0 and ::/0)"]
        );
        let excluded = Actor {
            label: Some(Ref::new("/orgs/5/labels/1")),
            exclusion: Some(true),
            ..Default::default()
        };
        let a = ResolvedActor::from_actor(&excluded, &idx);
        assert_eq!(a.name, "!role=web");
        assert!(a.exclusion);
        assert_eq!(describe_actor(&excluded, &idx), "!role=web");
        let unknown = ResolvedActor::from_actor(&Actor::default(), &idx);
        assert_eq!(
            (unknown.kind.as_str(), unknown.name.as_str()),
            ("unknown", "?")
        );
        assert!(unknown.href.is_none());
        let set = RuleSetView::from_rule_set(&rs[0], &idx);
        assert_eq!(set.scopes, vec!["app=shop & env=prod"]);
        assert_eq!(
            set.rule_count, 4,
            "two allow rules plus two inline deny rules"
        );
        assert_eq!(set.deny_count, 2);
        let infra = RuleSetView::from_rule_set(&rs[1], &idx);
        assert_eq!(infra.scopes, vec!["All"]);
        assert_eq!((infra.rule_count, infra.deny_count), (1, 0));
    }

    #[test]
    fn label_group_href_is_case_insensitive_and_survives_with_labels() {
        let idx = index();
        assert_eq!(
            idx.label_group_href("all PROD envs"),
            Some("/orgs/5/sec_policy/draft/label_groups/10")
        );
        assert_eq!(idx.label_group_href("All prod env"), None);
        let relabelled = idx.with_labels(&[]);
        assert_eq!(
            relabelled.label_group_href("All prod envs"),
            Some("/orgs/5/sec_policy/draft/label_groups/10")
        );
    }

    #[test]
    fn service_href_finds_one_live_service_by_exact_name() {
        let idx = index();
        assert_eq!(
            idx.service_href("Web"),
            Some("/orgs/5/sec_policy/draft/services/32")
        );
        assert_eq!(idx.service_href("web"), None, "exact, like the CLI");
        assert_eq!(idx.service_href("Nope"), None);
        let services: Vec<Service> = serde_json::from_value(serde_json::json!([
            {"href": "/orgs/5/sec_policy/draft/services/41", "name": "Dup"},
            {"href": "/orgs/5/sec_policy/draft/services/42", "name": "Dup"},
            {"href": "/orgs/5/sec_policy/draft/services/43", "name": "Gone", "update_type": "delete"}
        ]))
        .unwrap();
        let idx = NameIndex::new(&[], &[], &[], &services);
        assert_eq!(
            idx.service_href("Dup"),
            None,
            "ambiguous: the href is needed"
        );
        assert_eq!(idx.service_href("Gone"), None, "pending deletion");
    }

    #[test]
    fn label_groups_and_ip_lists_are_found_by_href_in_either_scope() {
        let idx = index();
        assert_eq!(
            idx.label_group("/orgs/5/sec_policy/active/label_groups/10")
                .map(|g| g.key.as_str()),
            Some("env")
        );
        assert_eq!(
            idx.ip_list("/orgs/5/sec_policy/draft/ip_lists/21")
                .map(|l| l.name.as_str()),
            Some("Corp")
        );
        assert_eq!(
            idx.ip_list("/orgs/5/sec_policy/active/ip_lists/20")
                .map(|l| l.ip_ranges.len()),
            Some(2)
        );
        assert!(
            idx.label_group("/orgs/5/sec_policy/draft/label_groups/99")
                .is_none()
        );
        assert!(idx.ip_list("/orgs/5/labels/1").is_none());
    }
}
