//! The Explorer traffic query: build it, run it, make the rows readable,
//! and aggregate them. Nothing here touches the network except `poll`.
pub mod aggregate;
pub mod filter;
pub mod mesh;
pub mod poll;
pub mod query;
pub mod resolved;

pub use aggregate::{
    BLOCKED_NAMES, BlockedGroup, Dominant, FlowGroup, blocked_summary, compact_count, dominant,
    group_by_labels, group_key, group_text, group_values, keys_or_default, keys_seen,
};
pub use filter::{FilterExpr, resolve_actor};
pub use mesh::{MeshColumns, MeshKey, MeshRow, mesh, mesh_key, process_sides};
pub use poll::{PollConfig, QueryProgress, TrafficResult, run_traffic_query};
pub use query::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, DEFAULT_TRAFFIC_MAX_RESULTS, DEFAULT_WINDOW, QueryCost,
    TrafficQueryBuilder, format_timestamp, parse_window,
};
pub use resolved::{ResolvedEndpoint, ResolvedFlow, resolve_flows};
