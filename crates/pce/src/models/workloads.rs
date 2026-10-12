//! Workloads (VEN-managed hosts and unmanaged entries).
use super::common::Ref;
use crate::KeyOrder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interface {
    #[serde(default)]
    pub name: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cidr_block: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_state: Option<String>,
    /// Everything else the PCE reports (gateway, friendly name, ...); kept
    /// so a full interfaces PUT does not clear it.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
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

/// A heartbeat older than this means the VEN is offline, whatever its status says.
pub const HEARTBEAT_OFFLINE: Duration = Duration::from_secs(15 * 60);
/// A policy sync still not applied after this long is an error.
pub const SYNC_STALE: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenHealth {
    Healthy,
    Warning,
    Error,
    Offline,
    Unmanaged,
}

impl VenHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            VenHealth::Healthy => "healthy",
            VenHealth::Warning => "warning",
            VenHealth::Error => "error",
            VenHealth::Offline => "offline",
            VenHealth::Unmanaged => "unmanaged",
        }
    }

    /// Higher is worse; used for sorting.
    pub fn rank(self) -> u8 {
        match self {
            VenHealth::Healthy => 0,
            VenHealth::Unmanaged => 1,
            VenHealth::Warning => 2,
            VenHealth::Offline => 3,
            VenHealth::Error => 4,
        }
    }
}

/// The parts of `agent.status` the UI cares about, with messages flattened.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AgentStatus {
    pub status: Option<String>,
    pub sync_state: Option<String>,
    pub last_heartbeat: Option<String>,
    pub applied_at: Option<String>,
    pub version: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub fw_config_current: Option<bool>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// `{"type": "x", "message": "y"}` becomes `x: y`; strings pass through; anything else is compact JSON.
fn health_item(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    let t = v.get("type").and_then(Value::as_str);
    let m = v.get("message").and_then(Value::as_str);
    match (t, m) {
        (Some(t), Some(m)) => format!("{t}: {m}"),
        (Some(t), None) => t.to_string(),
        (None, Some(m)) => m.to_string(),
        (None, None) => v.to_string(),
    }
}

/// `agent_health` severities that count as errors; any other severity is a warning.
fn is_error_severity(sev: &str) -> bool {
    ["err", "error", "critical", "crit"]
        .iter()
        .any(|e| sev.eq_ignore_ascii_case(e))
}

/// Merges the legacy `agent_health_errors {errors, warnings}` lists with the current
/// `agent_health: [{type, severity, audit_event}]` items. An `agent_health` item renders
/// as `<type>` (plus `: <audit_event>`) and is dropped when the same type is already
/// reported in that bucket (by the legacy lists or an earlier item).
fn fold_health(
    legacy_errors: &[Value],
    legacy_warnings: &[Value],
    agent_health: &[Value],
) -> (Vec<String>, Vec<String>) {
    let item_type = |v: &Value| v.get("type").and_then(Value::as_str).map(str::to_string);
    let mut errors: Vec<String> = legacy_errors.iter().map(health_item).collect();
    let mut warnings: Vec<String> = legacy_warnings.iter().map(health_item).collect();
    let mut error_types: Vec<String> = legacy_errors.iter().filter_map(item_type).collect();
    let mut warning_types: Vec<String> = legacy_warnings.iter().filter_map(item_type).collect();
    for item in agent_health {
        let Some(t) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        let sev = item.get("severity").and_then(Value::as_str).unwrap_or("");
        let (out, seen) = if is_error_severity(sev) {
            (&mut errors, &mut error_types)
        } else {
            (&mut warnings, &mut warning_types)
        };
        if seen.iter().any(|s| s == t) {
            continue;
        }
        seen.push(t.to_string());
        out.push(match item.get("audit_event").and_then(Value::as_str) {
            Some(ev) => format!("{t}: {ev}"),
            None => t.to_string(),
        });
    }
    (errors, warnings)
}

fn parse_ts(s: &str) -> Option<jiff::Timestamp> {
    s.parse().ok()
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

    /// `app=shop env=prod role=web`, keys in `order`, then by value.
    pub fn label_text(&self, order: &KeyOrder) -> String {
        let mut parts: Vec<(&str, &str)> = self
            .labels
            .iter()
            .filter_map(|l| Some((l.key.as_deref()?, l.value.as_deref().unwrap_or(""))))
            .collect();
        parts.sort_by(|a, b| order.cmp_keys(a.0, b.0).then_with(|| a.1.cmp(b.1)));
        parts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn label_hrefs(&self) -> Vec<&str> {
        self.labels.iter().map(|l| l.href.as_str()).collect()
    }

    pub fn agent_status_view(&self) -> Option<AgentStatus> {
        let st = self.agent.as_ref()?.get("status")?;
        let legacy = |k: &str| {
            st.get("agent_health_errors")
                .and_then(|e| e.get(k))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let (errors, warnings) = fold_health(
            &legacy("errors"),
            &legacy("warnings"),
            st.get("agent_health")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        );
        let s = |k: &str| st.get(k).and_then(Value::as_str).map(str::to_string);
        Some(AgentStatus {
            status: s("status"),
            sync_state: s("security_policy_sync_state"),
            last_heartbeat: s("last_heartbeat_on"),
            applied_at: s("security_policy_applied_at"),
            version: s("agent_version"),
            uptime_seconds: st.get("uptime_seconds").and_then(Value::as_u64),
            fw_config_current: st.get("fw_config_current").and_then(Value::as_bool),
            errors,
            warnings,
        })
    }

    pub fn ven_version(&self) -> Option<String> {
        self.agent_status_view().and_then(|a| a.version)
    }

    /// Time since the last heartbeat; `None` when unmanaged or the timestamp is missing or unparseable.
    pub fn heartbeat_age(&self, now: jiff::Timestamp) -> Option<Duration> {
        let hb = parse_ts(self.agent_status_view()?.last_heartbeat.as_deref()?)?;
        Some(now.duration_since(hb).unsigned_abs())
    }

    pub fn ven_health(&self, now: jiff::Timestamp) -> VenHealth {
        let Some(a) = self.agent_status_view() else {
            return VenHealth::Unmanaged;
        };
        if self.online == Some(false) {
            return VenHealth::Offline;
        }
        if self
            .heartbeat_age(now)
            .is_some_and(|age| age > HEARTBEAT_OFFLINE)
        {
            return VenHealth::Offline;
        }
        if !a.errors.is_empty() {
            return VenHealth::Error;
        }
        let sync_stale = a.sync_state.as_deref().is_some_and(|s| s != "applied")
            && a.applied_at
                .as_deref()
                .and_then(parse_ts)
                .is_some_and(|t| now.duration_since(t).unsigned_abs() > SYNC_STALE);
        if sync_stale {
            return VenHealth::Error;
        }
        if a.status
            .as_deref()
            .is_some_and(|s| !s.eq_ignore_ascii_case("active"))
        {
            return VenHealth::Warning;
        }
        if !a.warnings.is_empty() {
            return VenHealth::Warning;
        }
        VenHealth::Healthy
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
        assert_eq!(w.len(), 4);
        assert_eq!(w[0].display_name(), "host-1.example");
        assert_eq!(w[0].primary_ip(), Some("10.0.1.10"));
        assert_eq!(w[0].agent_status(), Some("active".into()));
        assert_eq!(
            w[0].label_text(&KeyOrder::default()),
            "app=shop env=prod role=web"
        );
        assert_eq!(w[1].display_name(), "host-2.example");
        assert_eq!(w[1].agent_status(), Some("stopped".into()));
        assert_eq!(w[2].display_name(), "unmanaged-nas");
        assert_eq!(w[2].agent_status(), None);
        assert_eq!(w[2].label_text(&KeyOrder::default()), "");
        assert_eq!(w[2].online, None);
    }

    #[test]
    fn label_text_follows_the_key_order() {
        let w = fixture();
        let order = KeyOrder::new(&["env".into()], "role");
        assert_eq!(w[0].label_text(&order), "env=prod role=web app=shop");
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

    #[test]
    fn ven_health_rules() {
        let w = fixture();
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        assert_eq!(w[0].ven_health(now), VenHealth::Healthy);
        assert_eq!(
            w[0].heartbeat_age(now),
            Some(std::time::Duration::from_secs(120))
        );
        assert_eq!(w[0].ven_version().as_deref(), Some("23.2.31-3113"));
        let a = w[0].agent_status_view().unwrap();
        assert_eq!(a.sync_state.as_deref(), Some("applied"));
        assert!(a.errors.is_empty() && a.warnings.is_empty());
        // workload 2: online=false -> Offline regardless of warnings
        assert_eq!(w[1].ven_health(now), VenHealth::Offline);
        let a2 = w[1].agent_status_view().unwrap();
        assert_eq!(a2.warnings, vec!["policy_sync: policy out of date"]);
        assert_eq!(w[2].ven_health(now), VenHealth::Unmanaged);
        assert_eq!(w[2].heartbeat_age(now), None);
        // stale heartbeat on an online workload -> Offline
        let mut stale = w[0].clone();
        stale.agent.as_mut().unwrap()["status"]["last_heartbeat_on"] =
            serde_json::Value::String("2026-10-10T07:00:00Z".into());
        assert_eq!(stale.ven_health(now), VenHealth::Offline);
        // syncing for longer than SYNC_STALE -> Error; a recent sync -> Warning only if warnings exist, else Healthy
        let mut syncing = w[0].clone();
        syncing.agent.as_mut().unwrap()["status"]["security_policy_sync_state"] =
            serde_json::Value::String("syncing".into());
        syncing.agent.as_mut().unwrap()["status"]["security_policy_applied_at"] =
            serde_json::Value::String("2026-10-10T07:40:00Z".into());
        assert_eq!(syncing.ven_health(now), VenHealth::Error);
        syncing.agent.as_mut().unwrap()["status"]["security_policy_applied_at"] =
            serde_json::Value::String("2026-10-10T07:55:00Z".into());
        assert_eq!(syncing.ven_health(now), VenHealth::Healthy);
        let mut errs = w[0].clone();
        errs.agent.as_mut().unwrap()["status"]["agent_health_errors"]["errors"] =
            serde_json::json!([{"type": "firewall", "message": "iptables failed"}]);
        assert_eq!(errs.ven_health(now), VenHealth::Error);
        assert_eq!(
            errs.agent_status_view().unwrap().errors,
            vec!["firewall: iptables failed"]
        );
        assert!(
            VenHealth::Error.rank() > VenHealth::Offline.rank()
                && VenHealth::Offline.rank() > VenHealth::Warning.rank()
        );
        assert_eq!(
            serde_json::to_string(&VenHealth::Offline).unwrap(),
            "\"offline\""
        );
    }

    #[test]
    fn agent_health_items_fold_into_errors_and_warnings_by_severity() {
        let w = fixture();
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let with_health = |items: serde_json::Value| {
            let mut x = w[0].clone();
            let st = &mut x.agent.as_mut().unwrap()["status"];
            st.as_object_mut().unwrap().remove("agent_health_errors");
            st["agent_health"] = items;
            x
        };
        let err = with_health(serde_json::json!([{"type": "tampering", "severity": "error"}]));
        assert_eq!(err.ven_health(now), VenHealth::Error);
        assert_eq!(err.agent_status_view().unwrap().errors, vec!["tampering"]);
        let warn = with_health(serde_json::json!([{"type": "tampering", "severity": "warning"}]));
        assert_eq!(warn.ven_health(now), VenHealth::Warning);
        assert_eq!(
            warn.agent_status_view().unwrap().warnings,
            vec!["tampering"]
        );
        for sev in ["err", "crit", "critical", "ERROR"] {
            let x = with_health(serde_json::json!([{"type": "t", "severity": sev}]));
            assert_eq!(x.ven_health(now), VenHealth::Error, "{sev}");
        }
        // An unknown severity is a warning; audit_event is appended.
        let odd = with_health(serde_json::json!([
            {"type": "t", "severity": "info", "audit_event": "/orgs/5/events/9"}
        ]));
        assert_eq!(
            odd.agent_status_view().unwrap().warnings,
            vec!["t: /orgs/5/events/9"]
        );
        // Deduplicated against agent_health_errors by type (fixture workload 2 carries both).
        let a2 = w[1].agent_status_view().unwrap();
        assert_eq!(a2.warnings, vec!["policy_sync: policy out of date"]);
        assert!(a2.errors.is_empty());
        // The recorded fixture (agent_health: []) is unchanged.
        let recorded: Vec<Workload> =
            serde_json::from_str(include_str!("../../../../fixtures/recorded/workloads.json"))
                .unwrap();
        assert!(
            recorded
                .iter()
                .filter_map(Workload::agent_status_view)
                .all(|a| a.errors.is_empty() && a.warnings.is_empty())
        );
    }

    #[test]
    fn a_non_active_status_is_at_least_a_warning() {
        let w = fixture();
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        // suspended, online, fresh heartbeat
        assert_eq!(w[3].ven_health(now), VenHealth::Warning);
        let mut upper = w[0].clone();
        upper.agent.as_mut().unwrap()["status"]["status"] = "ACTIVE".into();
        assert_eq!(
            upper.ven_health(now),
            VenHealth::Healthy,
            "case-insensitive"
        );
        // Offline and Error still win over the status warning.
        let mut off = w[3].clone();
        off.online = Some(false);
        assert_eq!(off.ven_health(now), VenHealth::Offline);
        let mut err = w[3].clone();
        err.agent.as_mut().unwrap()["status"]["agent_health_errors"]["errors"] =
            serde_json::json!(["boom"]);
        assert_eq!(err.ven_health(now), VenHealth::Error);
    }

    #[test]
    fn unparseable_or_missing_timestamps_are_unknown_never_a_panic() {
        let w = fixture();
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let mut bad = w[0].clone();
        bad.agent.as_mut().unwrap()["status"]["last_heartbeat_on"] =
            serde_json::Value::String("not a time".into());
        assert_eq!(bad.heartbeat_age(now), None);
        assert_eq!(bad.ven_health(now), VenHealth::Healthy);
        bad.agent.as_mut().unwrap()["status"]
            .as_object_mut()
            .unwrap()
            .remove("last_heartbeat_on");
        assert_eq!(bad.heartbeat_age(now), None);
        // An agent object without a status has nothing to derive from.
        let mut bare = w[0].clone();
        bare.agent = Some(serde_json::json!({"href": "/orgs/5/agents/9"}));
        assert_eq!(bare.agent_status_view(), None);
    }
}
