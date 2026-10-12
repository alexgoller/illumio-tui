//! Rulesets, allow rules, deny rules, and pending provisioning changes.
use super::common::Ref;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyScope {
    Draft,
    Active,
}

impl PolicyScope {
    pub fn segment(self) -> &'static str {
        match self {
            PolicyScope::Draft => "draft",
            PolicyScope::Active => "active",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" => Some(PolicyScope::Draft),
            "active" => Some(PolicyScope::Active),
            _ => None,
        }
    }
}

/// One side of a rule or one element of a scope. Exactly one of the
/// optional fields is set, or `actors == "ams"` for all workloads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actors: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_group: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_service: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_server: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_list: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<bool>,

    /// Fields this model does not name (`sec_connect` is named; `caps`,
    /// `egress_services`, `use_workload_subnets` and future ones are not),
    /// carried through a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IngressService {
    Ref(Ref),
    /// An inline service. `port` is absent for port-less protocols (ICMP).
    Port {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_port: Option<u16>,
        proto: i64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolveLabelsAs {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub consumers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub href: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub providers: Vec<Actor>,
    #[serde(default)]
    pub consumers: Vec<Actor>,
    #[serde(default)]
    pub ingress_services: Vec<IngressService>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_labels_as: Option<ResolveLabelsAs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unscoped_consumers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sec_connect: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stateless: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_auth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,

    /// Fields this model does not name (`sec_connect` is named; `caps`,
    /// `egress_services`, `use_workload_subnets` and future ones are not),
    /// carried through a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DenyRule {
    pub href: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub providers: Vec<Actor>,
    #[serde(default)]
    pub consumers: Vec<Actor>,
    #[serde(default)]
    pub ingress_services: Vec<IngressService>,
    #[serde(rename = "override", default)]
    pub override_: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,

    /// Fields this model does not name (`sec_connect` is named; `caps`,
    /// `egress_services`, `use_workload_subnets` and future ones are not),
    /// carried through a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub href: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub scopes: Vec<Vec<Actor>>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Rulesets carry their deny rules inline; `Pce::deny_rules` is the
    /// per-ruleset endpoint for PCEs that leave this empty.
    #[serde(default)]
    pub deny_rules: Vec<DenyRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,

    /// Fields this model does not name (`sec_connect` is named; `caps`,
    /// `egress_services`, `use_workload_subnets` and future ones are not),
    /// carried through a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `change_type` of a pending entry that names none.
pub const UNKNOWN_CHANGE_TYPE: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChange {
    pub href: String,
    pub change_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub object_type: String,
}

/// `GET /sec_policy/pending` is either a list of `{href, change_type, name}`
/// or a map of object type to such lists (with `update_type`). Both become
/// one flat list.
pub fn parse_pending(v: Value) -> Vec<PendingChange> {
    fn item(v: &Value) -> Option<PendingChange> {
        let href = v.get("href")?.as_str()?.to_string();
        let change_type = v
            .get("change_type")
            .or_else(|| v.get("update_type"))
            .and_then(Value::as_str)
            .unwrap_or(UNKNOWN_CHANGE_TYPE)
            .to_string();
        let name = v.get("name").and_then(Value::as_str).map(str::to_string);
        let object_type = Ref::new(&href).object_type().to_string();
        Some(PendingChange {
            href,
            change_type,
            name,
            object_type,
        })
    }
    match v {
        Value::Array(items) => items.iter().filter_map(item).collect(),
        Value::Object(map) => map
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(item)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_sets_fixture_parses() {
        let rs: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].rules.len(), 2);
        assert_eq!(rs[0].deny_rules.len(), 2);
        assert!(rs[0].deny_rules[1].override_);
        assert!(rs[1].deny_rules.is_empty());
        assert_eq!(rs[0].scopes[0].len(), 2);
        assert_eq!(
            rs[0].rules[0].ingress_services[0],
            IngressService::Ref(Ref::new("/orgs/5/sec_policy/draft/services/31"))
        );
        assert_eq!(
            rs[0].rules[1].ingress_services[1],
            IngressService::Port {
                port: Some(8000),
                to_port: Some(8099),
                proto: 6
            }
        );
        let portless: Vec<IngressService> = serde_json::from_str(r#"[{"proto": 1}]"#).unwrap();
        assert_eq!(
            portless[0],
            IngressService::Port {
                port: None,
                to_port: None,
                proto: 1
            }
        );
        assert_eq!(rs[0].rules[1].unscoped_consumers, Some(true));
        assert_eq!(rs[1].rules[0].providers[0].actors.as_deref(), Some("ams"));
        assert_eq!(rs[1].update_type.as_deref(), Some("create"));
    }

    #[test]
    fn deny_rules_parse_override_flag() {
        let dr: Vec<DenyRule> =
            serde_json::from_str(include_str!("../../../../fixtures/deny_rules.json")).unwrap();
        assert!(!dr[0].override_);
        assert!(dr[1].override_);
        let back = serde_json::to_value(&dr[1]).unwrap();
        assert_eq!(back["override"], true);
    }

    #[test]
    fn pending_parses_both_shapes() {
        let list: serde_json::Value =
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap();
        let dict: serde_json::Value =
            serde_json::from_str(include_str!("../../../../fixtures/pending_dict.json")).unwrap();
        let a = parse_pending(list);
        let b = parse_pending(dict);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].object_type, "rule_sets");
        assert_eq!(a[0].change_type, "create");
        assert_eq!(a[1].name.as_deref(), Some("Corp"));
        assert_eq!(
            b.len(),
            a.len(),
            "dict shape yields the same changes (order may differ)"
        );
        for change in &a {
            assert!(b.contains(change), "{change:?} missing from dict parse");
        }
    }

    #[test]
    fn scope_segments() {
        assert_eq!(PolicyScope::Draft.segment(), "draft");
        assert_eq!(PolicyScope::parse("ACTIVE"), Some(PolicyScope::Active));
        assert_eq!(PolicyScope::parse("x"), None);
    }

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let v = serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100", "name": "Shop-Prod", "enabled": true,
            "caps": ["write"], "ip_tables_rules": [],
            "scopes": [[{"label": {"href": "/orgs/5/labels/3"}, "exclusion": false, "future": 1}]],
            "rules": [{
                "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "enabled": true,
                "providers": [{"label": {"href": "/orgs/5/labels/2"}, "future": "x"}],
                "consumers": [{"actors": "ams"}],
                "ingress_services": [{"port": 443, "proto": 6}],
                "sec_connect": false, "egress_services": [], "use_workload_subnets": ["providers"],
                "consuming_security_principals": []
            }],
            "deny_rules": [{
                "href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "enabled": true,
                "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
                "ingress_services": [{"port": 22, "proto": 6}], "override": true, "network_type": "brn"
            }]
        });
        let rs: RuleSet = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(rs.extra["caps"], serde_json::json!(["write"]));
        assert_eq!(rs.scopes[0][0].extra["future"], 1);
        assert_eq!(
            rs.rules[0].extra["use_workload_subnets"],
            serde_json::json!(["providers"])
        );
        assert_eq!(rs.rules[0].providers[0].extra["future"], "x");
        assert_eq!(rs.deny_rules[0].extra["network_type"], "brn");
        assert_eq!(
            rs.rules[0].sec_connect,
            Some(false),
            "named fields stay named"
        );
        assert!(!rs.rules[0].extra.contains_key("sec_connect"));
        let back = serde_json::to_value(&rs).unwrap();
        assert_eq!(back, v, "nothing lost, nothing added");
    }
}
