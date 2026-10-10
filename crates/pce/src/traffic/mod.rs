//! The Explorer traffic query: build it, run it, make the rows readable,
//! and aggregate them. Nothing here touches the network except `poll`.
pub mod aggregate;
pub mod filter;
pub mod poll;
pub mod query;
pub mod resolved;

pub use aggregate::{BlockedGroup, FlowGroup, blocked_summary, group_by_labels};
pub use filter::{FilterExpr, resolve_actor};
pub use poll::{PollConfig, QueryProgress, TrafficResult, run_traffic_query};
pub use query::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, DEFAULT_TRAFFIC_MAX_RESULTS, DEFAULT_WINDOW, QueryCost,
    TrafficQueryBuilder, format_timestamp, parse_window,
};
pub use resolved::{ResolvedEndpoint, ResolvedFlow, resolve_flows};
