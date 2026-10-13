//! Labels and the policy objects rules refer to by href.
use super::common::{Ref, proto_name};
use serde::{Deserialize, Serialize};

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
