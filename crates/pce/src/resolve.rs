//! Turns PCE hrefs into names. Built once from the four object lists and
//! shared (via `Arc`) by every screen and command. Never makes requests.
use crate::models::{IpList, Label, LabelGroup, Ref, Service};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    ip_lists: HashMap<String, IpList>,
    ip_list_by_name: HashMap<String, String>,
    services: HashMap<String, Service>,
}

impl NameIndex {
    pub fn new(
        labels: &[Label],
        groups: &[LabelGroup],
        ip_lists: &[IpList],
        services: &[Service],
    ) -> Self {
        let mut idx = Self::default();
        for l in labels {
            idx.labels.insert(l.href.clone(), l.clone());
            if !l.is_deleted() {
                idx.label_by_kv
                    .insert((l.key.clone(), l.value.clone()), l.href.clone());
                idx.keys
                    .entry(l.key.clone())
                    .or_default()
                    .insert(l.value.clone());
            }
        }
        for g in groups {
            idx.label_groups.insert(g.href.clone(), g.clone());
        }
        for i in ip_lists {
            idx.ip_list_by_name
                .insert(i.name.to_ascii_lowercase(), i.href.clone());
            idx.ip_lists.insert(i.href.clone(), i.clone());
        }
        for s in services {
            idx.services.insert(s.href.clone(), s.clone());
        }
        idx
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

    pub fn label_keys(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    pub fn values_for_key(&self, key: &str) -> Vec<String> {
        self.keys
            .get(key)
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn label_group_name(&self, href: &str) -> Option<&str> {
        self.label_groups.get(href).map(|g| g.name.as_str())
    }

    pub fn ip_list_name(&self, href: &str) -> Option<&str> {
        self.ip_lists.get(href).map(|l| l.name.as_str())
    }

    pub fn ip_list_href(&self, name: &str) -> Option<&str> {
        self.ip_list_by_name
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn service(&self, href: &str) -> Option<&Service> {
        self.services.get(href)
    }

    pub fn service_name(&self, href: &str) -> Option<&str> {
        self.services.get(href).map(|s| s.name.as_str())
    }

    /// Resolves label refs, sorted by key. Unknown hrefs get key `?` and the
    /// href as value so nothing silently disappears.
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
        out.sort_by(|a, b| a.key.cmp(&b.key).then(a.value.cmp(&b.value)));
        out
    }
}

use crate::models::{Actor, DenyRule, IngressService, Rule, RuleSet};

/// `role=web`, `ipl:Corp`, `group:All prod envs`, `wl:<id>`, `All workloads`.
pub fn describe_actor(a: &Actor, idx: &NameIndex) -> String {
    let excl = if a.exclusion.unwrap_or(false) {
        "!"
    } else {
        ""
    };
    let body = if a.actors.as_deref() == Some("ams") {
        "All workloads".to_string()
    } else if let Some(l) = &a.label {
        idx.label_text(&l.href)
    } else if let Some(g) = &a.label_group {
        format!("group:{}", idx.label_group_name(&g.href).unwrap_or(&g.href))
    } else if let Some(i) = &a.ip_list {
        format!("ipl:{}", idx.ip_list_name(&i.href).unwrap_or(&i.href))
    } else if let Some(w) = &a.workload {
        format!("wl:{}", w.id())
    } else if let Some(v) = &a.virtual_service {
        format!("vs:{}", v.id())
    } else if let Some(v) = &a.virtual_server {
        format!("vsrv:{}", v.id())
    } else {
        "?".to_string()
    };
    format!("{excl}{body}")
}

pub fn describe_ingress_service(s: &IngressService, idx: &NameIndex) -> String {
    match s {
        IngressService::Ref(r) => idx.service_name(&r.href).unwrap_or(&r.href).to_string(),
        IngressService::Port {
            port,
            to_port,
            proto,
        } => match to_port {
            Some(t) if t != port => format!("{port}-{t}/{}", crate::models::proto_name(*proto)),
            _ => format!("{port}/{}", crate::models::proto_name(*proto)),
        },
    }
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
    pub consumers: Vec<String>,
    pub providers: Vec<String>,
    pub services: Vec<String>,
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
            consumers: r.consumers.iter().map(|a| describe_actor(a, idx)).collect(),
            providers: r.providers.iter().map(|a| describe_actor(a, idx)).collect(),
            services: r
                .ingress_services
                .iter()
                .map(|s| describe_ingress_service(s, idx))
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
            consumers: d.consumers.iter().map(|a| describe_actor(a, idx)).collect(),
            providers: d.providers.iter().map(|a| describe_actor(a, idx)).collect(),
            services: d
                .ingress_services
                .iter()
                .map(|s| describe_ingress_service(s, idx))
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
    pub rule_count: usize,
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
            rule_count: rs.rules.len(),
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
            idx.service_name("/orgs/5/sec_policy/draft/services/31"),
            Some("PostgreSQL")
        );
        assert_eq!(idx.service_name("/x"), None);
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
        assert_eq!(text, vec!["?=/orgs/5/labels/77", "env=prod", "role=web"]);
    }

    #[test]
    fn rule_views_render_names() {
        use crate::models::{DenyRule, RuleSet};
        let idx = index();
        let rs: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        let v = RuleView::from_rule(&rs[0].rules[1], &idx);
        assert_eq!(v.consumers, vec!["ipl:Corp"]);
        assert_eq!(v.providers, vec!["role=web"]);
        assert_eq!(v.services, vec!["443/tcp", "8000-8099/tcp"]);
        assert_eq!(v.kind, RuleKind::Allow);
        assert!(v.unscoped_consumers);
        let v3 = RuleView::from_rule(&rs[1].rules[0], &idx);
        assert_eq!(v3.providers, vec!["All workloads"]);
        assert_eq!(v3.consumers, vec!["group:All prod envs"]);
        assert_eq!(v3.services, vec!["All Services"]);
        let dr: Vec<DenyRule> =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        assert_eq!(
            RuleView::from_deny(&dr[1], &idx).kind,
            RuleKind::OverrideDeny
        );
        assert_eq!(
            RuleView::from_deny(&dr[0], &idx).consumers,
            vec!["ipl:Any (0.0.0.0/0 and ::/0)"]
        );
        let set = RuleSetView::from_rule_set(&rs[0], &idx);
        assert_eq!(set.scopes, vec!["app=shop & env=prod"]);
        assert_eq!(set.rule_count, 2);
        assert_eq!(RuleSetView::from_rule_set(&rs[1], &idx).scopes, vec!["All"]);
    }
}
