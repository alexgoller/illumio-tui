//! Serde types for PCE API objects. Unknown fields are ignored unless a
//! type keeps an `extra` map on purpose (flows do, so the TUI can show
//! anything the PCE adds later).
pub mod common;
pub mod labels;
pub mod policy;
pub mod traffic;
pub mod workloads;

pub use common::{PolicyDecision, Ref, proto_name, proto_number};
pub use labels::{Fqdn, IpList, IpRange, Label, LabelGroup, Service, ServicePort};
pub use policy::{
    Actor, DenyRule, IngressService, PendingChange, PolicyScope, ResolveLabelsAs, Rule, RuleSet,
    parse_pending,
};
pub use traffic::{
    ActorFilter, ActorMatch, AsyncQueryStatus, Endpoint, Flow, FlowService, FlowWorkload, NamedRef,
    QueryResultRef, ServiceFilter, ServiceMatch, TimestampRange, TrafficQuery,
};
pub use workloads::{Interface, Workload, WorkloadFilter, WorkloadLabel};
