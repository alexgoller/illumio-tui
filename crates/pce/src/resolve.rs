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
}
