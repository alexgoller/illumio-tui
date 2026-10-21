//! Serde types for PCE API objects. Unknown fields are ignored unless a
//! type keeps an `extra` map on purpose (flows do, so the TUI can show
//! anything the PCE adds later).
pub mod common;
pub mod event_select;
pub mod events;
pub mod labels;
pub mod policy;
pub mod traffic;
pub mod workloads;

pub use common::{PolicyDecision, Ref, proto_name, proto_number};
pub use event_select::{
    SEVERITIES, SeverityFilter, TermText, TypeFilter, exclude_text, severity_level,
};
pub use events::{Event, EventFilter};
pub use labels::{Fqdn, IpList, IpRange, Label, LabelGroup, LabelUsageFlags, Service, ServicePort};
pub use policy::{
    Actor, DenyRule, IngressService, PendingChange, PolicyScope, ResolveLabelsAs, Rule, RuleSet,
    UNKNOWN_CHANGE_TYPE, parse_pending,
};
pub use traffic::{
    ActorFilter, ActorMatch, AsyncQueryStatus, DraftDecision, Endpoint, Flow, FlowRule,
    FlowService, FlowWorkload, NamedRef, QueryRegion, QueryResultRef, ServiceFilter, ServiceMatch,
    TimestampRange, TrafficQuery, Transmission,
};
pub use workloads::{
    AgentStatus, HEARTBEAT_OFFLINE, Interface, SYNC_STALE, UNMANAGED, VenHealth, Workload,
    WorkloadFilter, WorkloadLabel,
};
