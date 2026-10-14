//! The Explorer traffic query: build it, run it, make the rows readable,
//! and aggregate them. Nothing here touches the network except `poll`.
pub mod aggregate;
pub mod duration;
pub mod filter;
pub mod focus;
pub mod merge;
pub mod mesh;
pub mod poll;
pub mod query;
pub mod resolved;
pub mod sections;
pub mod steps;
pub mod treemap;

pub use aggregate::{
    ConnsByDecision, Dominant, FlowGroup, compact_count, dominant, group_by_labels,
    group_by_service, group_by_service_refs, group_key, group_text, group_values, keys_or_default,
    keys_seen,
};
pub use duration::compact_duration;
pub use filter::{FilterExpr, resolve_actor};
pub use focus::{
    Focus, FocusDirection, FocusedTraffic, GroupKey, app_labels, apps_in, closest, focus_flows,
    focus_query, parse_app, queries_text, union_flows,
};
pub use merge::{RecordKey, merge_flows, record_key};
pub use mesh::{MeshColumns, MeshKey, MeshRow, mesh, mesh_key, process_sides};
pub use poll::{
    PollConfig, QueryProgress, TrafficResult, Truncation, run_traffic_query, truncation,
};
pub use query::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, DEFAULT_TRAFFIC_MAX_RESULTS, DEFAULT_WINDOW,
    MAX_TRAFFIC_MAX_RESULTS, QueryCost, TrafficQueryBuilder, format_timestamp, parse_window,
};
pub use resolved::{ResolvedEndpoint, ResolvedFlow, resolve_flows};
pub use sections::{
    AppSection, Decisions, Direction, PEER_NAMES, PeerRow, Totals, app_header, app_sections,
    app_sections_both, in_app, in_peer,
};
pub use steps::{
    CAP_STEPS, WINDOW_STEPS, backfill_cap, next_cap, next_window, prev_cap, prev_window,
};
