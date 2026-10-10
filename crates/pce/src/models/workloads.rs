//! Workloads (VEN-managed hosts and unmanaged entries).
use super::common::Ref;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interface {
    #[serde(default)]
    pub name: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cidr_block: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadLabel {
    pub href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workload {
    pub href: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    #[serde(default)]
    pub labels: Vec<WorkloadLabel>,
    #[serde(default)]
    pub enforcement_mode: Option<String>,
    #[serde(default)]
    pub visibility_level: Option<String>,
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub managed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ven: Option<Ref>,
}

impl Workload {
    /// hostname, else name, else the href id.
    pub fn display_name(&self) -> String {
        self.hostname
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.name.clone().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| Ref::new(&self.href).id().to_string())
    }

    pub fn primary_ip(&self) -> Option<&str> {
        self.interfaces.first().map(|i| i.address.as_str())
    }

    /// `agent.status.status` when present.
    pub fn agent_status(&self) -> Option<String> {
        self.agent
            .as_ref()?
            .get("status")?
            .get("status")?
            .as_str()
            .map(str::to_string)
    }

    /// `app=shop env=prod role=web`, sorted by key.
    pub fn label_text(&self) -> String {
        let mut parts: Vec<String> = self
            .labels
            .iter()
            .filter_map(|l| {
                Some(format!(
                    "{}={}",
                    l.key.as_ref()?,
                    l.value.as_deref().unwrap_or("")
                ))
            })
            .collect();
        parts.sort();
        parts.join(" ")
    }

    pub fn label_hrefs(&self) -> Vec<&str> {
        self.labels.iter().map(|l| l.href.as_str()).collect()
    }
}

/// Query parameters for `GET /orgs/{org}/workloads`. `labels` is an OR of
/// AND-groups of label hrefs, sent as a JSON string (the PCE's format).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkloadFilter {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub enforcement_mode: Option<String>,
    pub managed: Option<bool>,
    pub online: Option<bool>,
    #[serde(default)]
    pub labels: Vec<Vec<String>>,
    pub max_results: Option<u32>,
}

impl WorkloadFilter {
    pub fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(v) = &self.name {
            q.push(("name", v.clone()));
        }
        if let Some(v) = &self.hostname {
            q.push(("hostname", v.clone()));
        }
        if let Some(v) = &self.ip_address {
            q.push(("ip_address", v.clone()));
        }
        if let Some(v) = &self.enforcement_mode {
            q.push(("enforcement_mode", v.clone()));
        }
        if let Some(v) = self.managed {
            q.push(("managed", v.to_string()));
        }
        if let Some(v) = self.online {
            q.push(("online", v.to_string()));
        }
        if !self.labels.is_empty() {
            q.push((
                "labels",
                serde_json::to_string(&self.labels).unwrap_or_default(),
            ));
        }
        if let Some(v) = self.max_results {
            q.push(("max_results", v.to_string()));
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    #[test]
    fn parses_and_derives_display_fields() {
        let w = fixture();
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].display_name(), "host-1.example");
        assert_eq!(w[0].primary_ip(), Some("10.0.1.10"));
        assert_eq!(w[0].agent_status(), Some("active".into()));
        assert_eq!(w[0].label_text(), "app=shop env=prod role=web");
        assert_eq!(w[1].display_name(), "host-2.example");
        assert_eq!(w[1].agent_status(), Some("stopped".into()));
        assert_eq!(w[2].display_name(), "unmanaged-nas");
        assert_eq!(w[2].agent_status(), None);
        assert_eq!(w[2].label_text(), "");
        assert_eq!(w[2].online, None);
    }

    #[test]
    fn filter_encodes_labels_as_nested_json() {
        let f = WorkloadFilter {
            hostname: Some("web".into()),
            labels: vec![
                vec!["/orgs/5/labels/1".into(), "/orgs/5/labels/3".into()],
                vec!["/orgs/5/labels/2".into()],
            ],
            online: Some(true),
            max_results: Some(50),
            ..Default::default()
        };
        let q = f.to_query();
        assert!(q.contains(&("hostname", "web".to_string())));
        assert!(q.contains(&("online", "true".to_string())));
        assert!(q.contains(&("max_results", "50".to_string())));
        assert!(q.contains(&(
            "labels",
            r#"[["/orgs/5/labels/1","/orgs/5/labels/3"],["/orgs/5/labels/2"]]"#.to_string()
        )));
        assert_eq!(WorkloadFilter::default().to_query(), vec![]);
    }
}
