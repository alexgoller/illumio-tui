//! Explorer traffic query request and response types.
use super::common::{PolicyDecision, Ref};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One element of a traffic filter. Serialises to the PCE's one-key objects,
/// e.g. `{"label": {"href": "..."}}` or `{"ip_address": "10.0.0.1"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActorMatch {
    Label { label: Ref },
    IpList { ip_list: Ref },
    Workload { workload: Ref },
    IpAddress { ip_address: String },
    Fqdn { fqdn: String },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_policy_decision: Option<String>,
    #[serde(default)]
    pub rules: Vec<Ref>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn unknown_decision() -> PolicyDecision {
    PolicyDecision::Unknown
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flows_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
