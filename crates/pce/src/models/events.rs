//! PCE audit events (`GET /orgs/{org}/events`). The schema is wide and
//! version-dependent; we keep every field in `extra` and derive a few
//! display helpers.
use crate::traffic::format_timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Value>,
    #[serde(default)]
    pub notifications: Vec<Value>,
    #[serde(default)]
    pub resource_changes: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Event {
    /// `user:<username>`, `agent:<hostname>`, `system`, or `?`.
    pub fn actor(&self) -> String {
        let Some(cb) = &self.created_by else {
            return "?".into();
        };
        if let Some(u) = cb.get("user") {
            let name = u
                .get("username")
                .or_else(|| u.get("email"))
                .or_else(|| u.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            return format!("user:{name}");
        }
        if let Some(a) = cb.get("agent") {
            let name = a
                .get("hostname")
                .or_else(|| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            return format!("agent:{name}");
        }
        if cb.get("system").is_some() {
            return "system".into();
        }
        cb.as_object()
            .and_then(|o| o.keys().next())
            .cloned()
            .unwrap_or_else(|| "?".into())
    }

    /// `workload host-1.example`, `sec_policy 42`, or empty.
    pub fn resource(&self) -> String {
        let Some(res) = self
            .resource_changes
            .first()
            .and_then(|c| c.get("resource"))
            .and_then(Value::as_object)
        else {
            return String::new();
        };
        let Some((kind, v)) = res.iter().next() else {
            return String::new();
        };
        let label = v
            .get("hostname")
            .or_else(|| v.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                v.get("href")
                    .and_then(Value::as_str)
                    .map(|h| h.rsplit('/').next().unwrap_or(h).to_string())
            })
            .unwrap_or_default();
        if label.is_empty() {
            kind.clone()
        } else {
            format!("{kind} {label}")
        }
    }

    pub fn severity_rank(&self) -> u8 {
        match self
            .severity
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("critical") | Some("crit") => 3,
            Some("err") | Some("error") => 2,
            Some("warning") | Some("warn") => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventFilter {
    pub severity: Option<String>,
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub since: Option<jiff::Timestamp>,
    pub until: Option<jiff::Timestamp>,
    pub max_results: Option<u32>,
}

impl EventFilter {
    pub fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(v) = &self.severity {
            q.push(("severity", v.clone()));
        }
        if let Some(v) = &self.event_type {
            q.push(("event_type", v.clone()));
        }
        if let Some(v) = &self.status {
            q.push(("status", v.clone()));
        }
        if let Some(t) = self.since {
            q.push(("timestamp[gte]", format_timestamp(t)));
        }
        if let Some(t) = self.until {
            q.push(("timestamp[lte]", format_timestamp(t)));
        }
        if let Some(n) = self.max_results {
            q.push(("max_results", n.to_string()));
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Event> {
        serde_json::from_str(include_str!("../../../../fixtures/events.json")).unwrap()
    }

    #[test]
    fn parses_and_derives_actor_resource_rank() {
        let e = fixture();
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].actor(), "user:user-1");
        assert_eq!(e[0].resource(), "");
        assert_eq!(e[1].resource(), "sec_policy 42");
        assert_eq!(e[2].actor(), "agent:host-1.example");
        assert_eq!(e[2].resource(), "workload host-1.example");
        assert_eq!(e[2].severity_rank(), 2);
        assert_eq!(e[3].actor(), "system");
        assert_eq!(e[3].severity_rank(), 1);
        assert_eq!(e[0].severity_rank(), 0);
        assert!(e[0].extra.is_empty());
    }

    #[test]
    fn filter_encodes_pce_params() {
        let f = EventFilter {
            severity: Some("err".into()),
            event_type: None,
            status: Some("failure".into()),
            since: Some("2026-10-09T00:00:00Z".parse().unwrap()),
            until: Some("2026-10-10T00:00:00Z".parse().unwrap()),
            max_results: Some(50),
        };
        let q = f.to_query();
        assert!(q.contains(&("severity", "err".to_string())));
        assert!(q.contains(&("status", "failure".to_string())));
        assert!(q.contains(&("timestamp[gte]", "2026-10-09T00:00:00Z".to_string())));
        assert!(q.contains(&("timestamp[lte]", "2026-10-10T00:00:00Z".to_string())));
        assert!(q.contains(&("max_results", "50".to_string())));
        assert!(!q.iter().any(|(k, _)| *k == "event_type"));
        assert_eq!(EventFilter::default().to_query(), vec![]);
    }
}
