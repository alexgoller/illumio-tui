//! Labels and the policy objects rules refer to by href.
use super::common::{Ref, proto_name};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub href: String,
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// What holds the label, as the PCE says: only with `usage=true`
    /// (`Pce::labels_with_usage`), and absent from a PCE that ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LabelUsageFlags>,
}

/// A label's `usage` object from `GET /labels?usage=true`: one flag per
/// kind of object that can hold a label (`workload`, `rule`, `ruleset`,
/// `label_group`, `pairing_profile`, `permission`, `enforcement_boundary`,
/// `container_workload_profile`, `virtual_service`, ...). Kept as a map so
/// kinds a newer PCE adds parse, count and print too.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LabelUsageFlags(pub BTreeMap<String, serde_json::Value>);

impl LabelUsageFlags {
    /// The kinds holding the label, sorted. A flag counts unless it is
    /// `false`, `null` or `0`, so an unexpected value never hides a use.
    pub fn kinds(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(_, v)| {
                !matches!(v, Value::Bool(false) | Value::Null) && v.as_f64() != Some(0.0)
            })
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Whether anything holds the label; unused means every flag is false.
    pub fn is_used(&self) -> bool {
        !self.kinds().is_empty()
    }
}

impl Label {
    pub fn display(&self) -> String {
        format!("{}={}", self.key, self.value)
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted.unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelGroup {
    pub href: String,
    pub name: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub labels: Vec<Ref>,
    #[serde(default)]
    pub sub_groups: Vec<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpRange {
    pub from_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Fields this tool does not model, carried through a full update.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fqdn {
    pub fqdn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Fields this tool does not model, carried through a full update.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Fqdn {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self {
            fqdn: name.as_ref().trim().to_string(),
            description: None,
            extra: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpList {
    pub href: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub ip_ranges: Vec<IpRange>,
    #[serde(default)]
    pub fqdns: Vec<Fqdn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

impl IpList {
    /// `10.1.0.0-10.1.255.255, *.corp.example`
    pub fn summary(&self) -> String {
        self.entries().join(", ")
    }

    /// The ranges (`!` before an exclusion), then the FQDNs, one string each.
    pub fn entries(&self) -> Vec<String> {
        let mut parts: Vec<String> = self
            .ip_ranges
            .iter()
            .map(|r| {
                let excl = if r.exclusion.unwrap_or(false) {
                    "!"
                } else {
                    ""
                };
                match &r.to_ip {
                    Some(to) => format!("{excl}{}-{to}", r.from_ip),
                    None => format!("{excl}{}", r.from_ip),
                }
            })
            .collect();
        parts.extend(self.fqdns.iter().map(|f| f.fqdn.clone()));
        parts
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServicePort {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icmp_type: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icmp_code: Option<i64>,
}

impl ServicePort {
    /// `5432/tcp`, `8000-8099/tcp`, `icmp`, `any`
    pub fn display(&self) -> String {
        let proto = self.proto.map(proto_name).unwrap_or_else(|| "any".into());
        match (self.port, self.to_port) {
            (Some(p), Some(t)) if t != p => format!("{p}-{t}/{proto}"),
            (Some(p), _) => format!("{p}/{proto}"),
            (None, _) => proto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Service {
    pub href: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub service_ports: Vec<ServicePort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
    /// `com.illumio.ilo_defaults` for the PCE's built-in services.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_data_set: Option<String>,
    /// `ICMP` for the built-in ICMP service, whatever it is named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_data_reference: Option<String>,
}

impl Service {
    pub fn summary(&self) -> String {
        if self.service_ports.is_empty() {
            return "-".into();
        }
        self.service_ports
            .iter()
            .map(ServicePort::display)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_fixture_parses_including_deleted_without_value() {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        assert_eq!(labels.len(), 6);
        assert_eq!(labels[0].display(), "role=web");
        let deleted = &labels[5];
        assert!(deleted.is_deleted());
        assert_eq!(deleted.value, "");
        assert_eq!(deleted.display(), "app=");
    }

    #[test]
    fn usage_flags_parse_and_unknown_kinds_are_kept() {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels_usage.json")).unwrap();
        let u = |i: usize| labels[i].usage.as_ref().unwrap();
        assert_eq!(u(0).kinds(), vec!["rule", "workload"]);
        assert_eq!(
            u(4).kinds(),
            vec!["pairing_profile", "permission"],
            "held by what our own counts never read"
        );
        assert!(u(4).is_used());
        assert!(!u(6).is_used() && u(6).kinds().is_empty());
        let json = serde_json::to_value(&labels[6]).unwrap();
        assert_eq!(
            json["usage"]["a_kind_from_a_newer_pce"], false,
            "an unknown kind survives a round trip"
        );
        // Without `usage=true` the field is absent and stays so.
        let plain: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        assert!(plain.iter().all(|l| l.usage.is_none()));
        assert!(
            serde_json::to_value(&plain[0])
                .unwrap()
                .get("usage")
                .is_none()
        );
        // A newer PCE's flag that is not a plain boolean: anything but
        // false, null or 0 counts as a use, so it never hides one.
        let odd: LabelUsageFlags = serde_json::from_value(serde_json::json!({
            "rule": false, "zero": 0, "none": null, "count": 2, "detail": {"x": 1}
        }))
        .unwrap();
        assert_eq!(odd.kinds(), vec!["count", "detail"]);
        let unused: LabelUsageFlags =
            serde_json::from_value(serde_json::json!({"rule": false, "zero": 0})).unwrap();
        assert!(!unused.is_used());
    }

    #[test]
    fn label_groups_ip_lists_services_parse() {
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        assert_eq!(groups[0].labels[0].href, "/orgs/5/labels/4");
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        assert_eq!(lists[1].summary(), "10.1.0.0-10.1.255.255, *.corp.example");
        assert_eq!(lists[0].summary(), "0.0.0.0/0, ::/0");
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        assert_eq!(services[0].summary(), "any");
        assert_eq!(services[1].summary(), "5432/tcp");
        assert_eq!(services[2].summary(), "80/tcp, 443/tcp, 8000-8099/tcp");
    }
}
