//! Explorer traffic query request and response types.
use super::common::{PolicyDecision, Ref};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One element of a traffic filter. Serialises to the PCE's one-key objects,
/// e.g. `{"label": {"href": "..."}}` or `{"ip_address": "10.0.0.1"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActorMatch {
    Label {
        label: Ref,
    },
    IpList {
        ip_list: Ref,
    },
    Workload {
        workload: Ref,
    },
    IpAddress {
        ip_address: String,
    },
    Fqdn {
        fqdn: String,
    },
    /// Destination excludes only: `{"transmission": "broadcast"}`.
    Transmission {
        transmission: Transmission,
    },
}

/// How a flow was sent. Explorer filters it as a destination exclude
/// (`traffic_flows_transmission_param`); the flows it returns carry it as
/// `transmission`, absent for unicast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transmission {
    Unicast,
    Broadcast,
    Multicast,
}

impl Transmission {
    pub const ALL: [Transmission; 3] = [
        Transmission::Unicast,
        Transmission::Broadcast,
        Transmission::Multicast,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Transmission::Unicast => "unicast",
            Transmission::Broadcast => "broadcast",
            Transmission::Multicast => "multicast",
        }
    }
}

impl std::str::FromStr for Transmission {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Transmission::ALL
            .into_iter()
            .find(|t| t.as_str().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| format!("unknown transmission {s:?}; use unicast, broadcast, multicast"))
    }
}

/// `include` is an OR of AND-groups; `exclude` is flat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorFilter {
    pub include: Vec<Vec<ActorMatch>>,
    pub exclude: Vec<ActorMatch>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceFilter {
    pub include: Vec<ServiceMatch>,
    pub exclude: Vec<ServiceMatch>,
}

/// Body of `POST /orgs/{org}/traffic_flows/async_queries`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrafficQuery {
    pub start_date: String,
    pub end_date: String,
    pub sources: ActorFilter,
    pub destinations: ActorFilter,
    pub services: ServiceFilter,
    pub policy_decisions: Vec<PolicyDecision>,
    pub exclude_workloads_from_ip_list_query: bool,
    pub sources_destinations_query_op: String,
    pub max_results: u32,
    pub query_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowWorkload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub labels: Vec<Ref>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fqdn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<FlowWorkload>,
    #[serde(default)]
    pub ip_lists: Vec<NamedRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowService {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_service_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimestampRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_detected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_detected: Option<String>,
}

/// One row of the Explorer result. Unknown fields are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    #[serde(default)]
    pub src: Endpoint,
    #[serde(default)]
    pub dst: Endpoint,
    #[serde(default)]
    pub service: FlowService,
    #[serde(default = "unknown_decision")]
    pub policy_decision: PolicyDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_direction: Option<String>,
    #[serde(default)]
    pub num_connections: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_bi: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_bo: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_range: Option<TimestampRange>,
    /// The decision under the draft policy: present only after the query
    /// was re-evaluated (`update_rules`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_policy_decision: Option<DraftDecision>,
    /// The (draft) allow rules behind the flow. Read leniently: an entry
    /// is an href or an essential service rule's name, and a shape this
    /// does not know reads as no rules rather than failing the download.
    #[serde(default, deserialize_with = "lenient_rules")]
    pub rules: Vec<FlowRule>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Flow {
    /// The deny rules some PCE versions list beside `rules`
    /// (`deny_rules`, `override_deny_rules`, `enforcement_boundaries`), as
    /// hrefs. Read leniently: entries without an href are skipped.
    pub fn deny_rule_hrefs(&self) -> Vec<String> {
        let href = |v: &Value| match v {
            Value::String(s) => Some(s.clone()),
            Value::Object(m) => m.get("href").and_then(Value::as_str).map(str::to_string),
            _ => None,
        };
        [
            "deny_rules",
            "override_deny_rules",
            "enforcement_boundaries",
        ]
        .iter()
        .filter_map(|k| self.extra.get(*k)?.as_array())
        .flatten()
        .filter_map(href)
        .collect()
    }
}

fn unknown_decision() -> PolicyDecision {
    PolicyDecision::Unknown
}

/// A flow's decision under the draft policy (`draft_policy_decision`).
/// The PCE sends a string; values this does not know (the boundary and
/// override-deny variants of newer PCEs) are kept verbatim in `Other`, so
/// a new value never breaks a download.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DraftDecision {
    Allowed,
    PotentiallyBlocked,
    Blocked,
    Other(String),
}

impl DraftDecision {
    pub fn parse(s: &str) -> Self {
        match s {
            "allowed" => DraftDecision::Allowed,
            "potentially_blocked" => DraftDecision::PotentiallyBlocked,
            "blocked" => DraftDecision::Blocked,
            other => DraftDecision::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            DraftDecision::Allowed => "allowed",
            DraftDecision::PotentiallyBlocked => "potentially_blocked",
            DraftDecision::Blocked => "blocked",
            DraftDecision::Other(s) => s,
        }
    }

    /// The reported-decision class this falls in: an unknown value by its
    /// prefix (`allowed_across_boundary` is allowed,
    /// `blocked_by_boundary` blocked), else `Unknown`.
    pub fn class(&self) -> PolicyDecision {
        match self {
            DraftDecision::Allowed => PolicyDecision::Allowed,
            DraftDecision::PotentiallyBlocked => PolicyDecision::PotentiallyBlocked,
            DraftDecision::Blocked => PolicyDecision::Blocked,
            DraftDecision::Other(s) if s.starts_with("allowed") => PolicyDecision::Allowed,
            DraftDecision::Other(s) if s.starts_with("potentially_blocked") => {
                PolicyDecision::PotentiallyBlocked
            }
            DraftDecision::Other(s) if s.starts_with("blocked") => PolicyDecision::Blocked,
            DraftDecision::Other(_) => PolicyDecision::Unknown,
        }
    }
}

impl Serialize for DraftDecision {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DraftDecision {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(DraftDecision::parse(&String::deserialize(d)?))
    }
}

/// One rule behind a flow: an allow rule's href, or the name of an
/// essential service rule (`traffic_flows_rules`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub essential_service_rule: Option<String>,
}

impl FlowRule {
    pub fn href(href: impl Into<String>) -> Self {
        Self {
            href: Some(href.into()),
            essential_service_rule: None,
        }
    }

    /// The href, else `essential:<name>`; `None` for an empty entry.
    pub fn text(&self) -> Option<String> {
        self.href.clone().or_else(|| {
            self.essential_service_rule
                .as_ref()
                .map(|n| format!("essential:{n}"))
        })
    }
}

impl From<Ref> for FlowRule {
    fn from(r: Ref) -> Self {
        FlowRule::href(r.href)
    }
}

/// `rules` as an array of entries, or an object holding arrays of them
/// (`{"sec_rules": [...]}` on some PCE versions). Entries without an href
/// or a name, and any other shape, are dropped.
fn lenient_rules<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<FlowRule>, D::Error> {
    fn entries(v: Value, out: &mut Vec<FlowRule>) {
        match v {
            Value::Array(items) => {
                for item in items {
                    if let Ok(r) = serde_json::from_value::<FlowRule>(item)
                        && (r.href.is_some() || r.essential_service_rule.is_some())
                    {
                        out.push(r);
                    }
                }
            }
            Value::Object(map) => {
                for (_, v) in map {
                    if v.is_array() {
                        entries(v, out);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    entries(Value::deserialize(d)?, &mut out);
    Ok(out)
}

/// `result` on the async query status is a string href on some PCE versions
/// and `{"href": ...}` on others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryResultRef {
    Href(String),
    Obj { href: String },
}

impl QueryResultRef {
    pub fn href(&self) -> &str {
        match self {
            QueryResultRef::Href(h) => h,
            QueryResultRef::Obj { href } => href,
        }
    }
}

/// Response of the POST and of each poll.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AsyncQueryStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<QueryResultRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches_count: Option<u64>,
    /// Rows the result holds, after the cap: `flows_count >= max_results`
    /// is the cap signal. `matches_count` is a bounded raw count, never a
    /// total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flows_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// A supercluster's per-region counts; each region is capped on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<Vec<QueryRegion>>,
    /// When the PCE last changed the query: a new `update_rules` run moves
    /// it, which tells a fresh `completed` from the previous one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// The draft re-evaluation's state after `update_rules` (`queued`,
    /// `working`, `completed`, `failed`); absent before. Kept as a value: an
    /// unexpected shape must not fail the poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Value>,
}

impl AsyncQueryStatus {
    /// `rules` as a string state, if it is one; `{"status": ...}` too.
    pub fn rules_state(&self) -> Option<&str> {
        match self.rules.as_ref()? {
            Value::String(s) => Some(s),
            Value::Object(m) => m.get("status").and_then(Value::as_str),
            _ => None,
        }
    }
}

/// One region of a supercluster's async query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryRegion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pce_fqdn: Option<String>,
    /// `false` when the region did not answer: the result misses its rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flows_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches_count: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_decisions_parse_and_keep_unknown_values() {
        let d: Vec<DraftDecision> = serde_json::from_str(
            r#"["allowed","potentially_blocked","blocked","blocked_by_boundary","x"]"#,
        )
        .unwrap();
        assert_eq!(
            d[..3],
            [
                DraftDecision::Allowed,
                DraftDecision::PotentiallyBlocked,
                DraftDecision::Blocked
            ]
        );
        assert_eq!(d[3], DraftDecision::Other("blocked_by_boundary".into()));
        assert_eq!(d[3].class(), PolicyDecision::Blocked);
        assert_eq!(d[4].class(), PolicyDecision::Unknown);
        assert_eq!(
            DraftDecision::Other("allowed_across_boundary".into()).class(),
            PolicyDecision::Allowed
        );
        assert_eq!(
            DraftDecision::Other("potentially_blocked_by_boundary".into()).class(),
            PolicyDecision::PotentiallyBlocked
        );
        assert_eq!(
            serde_json::to_string(&d).unwrap(),
            r#"["allowed","potentially_blocked","blocked","blocked_by_boundary","x"]"#,
            "written back verbatim"
        );
    }

    fn flow(extra: &str) -> Flow {
        serde_json::from_str(&format!(
            r#"{{"src":{{}},"dst":{{}},"service":{{}},"policy_decision":"blocked"{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_draft_download_parses_with_every_draft_field() {
        let f = flow(
            r#","draft_policy_decision":"allowed",
               "rules":[{"href":"/orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2"},
                        {"essential_service_rule":"dhcp"}],
               "enforcement_boundaries":[{"href":"/orgs/1/sec_policy/draft/enforcement_boundaries/3"}],
               "override_deny_rules":[{"href":"/orgs/1/sec_policy/draft/rule_sets/1/deny_rules/4"}]"#,
        );
        assert_eq!(f.draft_policy_decision, Some(DraftDecision::Allowed));
        assert_eq!(
            f.rules
                .iter()
                .filter_map(FlowRule::text)
                .collect::<Vec<_>>(),
            vec![
                "/orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2".to_string(),
                "essential:dhcp".to_string()
            ]
        );
        assert!(f.extra.contains_key("enforcement_boundaries"));
        assert!(f.extra.contains_key("override_deny_rules"));
        assert_eq!(flow("").draft_policy_decision, None);
    }

    #[test]
    fn rules_in_other_shapes_never_fail_the_flow() {
        let f = flow(r#","rules":{"sec_rules":[{"href":"/r/1"}],"x":1}"#);
        assert_eq!(f.rules, vec![FlowRule::href("/r/1")]);
        assert!(flow(r#","rules":"weird""#).rules.is_empty());
        assert!(flow(r#","rules":null"#).rules.is_empty());
        assert!(flow(r#","rules":[{}, 3, {"href":"/r/2"}]"#).rules.len() == 1);
    }

    #[test]
    fn the_status_rules_state() {
        let s: AsyncQueryStatus =
            serde_json::from_str(r#"{"status":"completed","rules":"completed"}"#).unwrap();
        assert_eq!(s.rules_state(), Some("completed"));
        let s: AsyncQueryStatus =
            serde_json::from_str(r#"{"status":"completed","rules":{"status":"working"}}"#).unwrap();
        assert_eq!(s.rules_state(), Some("working"));
        let s: AsyncQueryStatus = serde_json::from_str(r#"{"status":"completed"}"#).unwrap();
        assert_eq!(s.rules_state(), None);
    }
}
