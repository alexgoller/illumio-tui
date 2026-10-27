//! The Explorer traffic query: build it, run it, make the rows readable,
//! and aggregate them. Nothing here touches the network except `poll`.
pub mod aggregate;
pub mod compare;
pub mod duration;
pub mod exposure;
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
    ConnsByDecision, Dominant, DraftCounts, FlowGroup, compact_count, dominant, group_by_labels,
    group_by_service, group_by_service_refs, group_key, group_text, group_values, keys_or_default,
    keys_seen,
};
pub use compare::{
    Change, ChangeSet, CompareCounts, Compared, DecisionSet, FlowKey, PairDelta, RowChange, Sides,
    compare_by, compare_flows, compare_groups, compare_mesh, compare_pairs, compare_sections,
    flow_key, keep_changes, keep_section_changes, squelched_keys, without_keys,
};
pub use duration::compact_duration;
pub use exposure::{
    ExposureRow, RiskyService, ServiceSummary, Sources, Status as ExposureStatus, default_services,
    exposure_rows, service_list, service_terms, summarize,
};
pub use filter::{FilterExpr, ServiceExpr, parse_service, resolve_actor, resolve_service};
pub use focus::{
    Focus, FocusDirection, FocusedTraffic, GroupKey, app_labels, apps_in, closest, focus_flows,
    focus_query, parse_app, queries_text, union_flows,
};
pub use merge::{RecordKey, merge_flows, record_key};
pub use mesh::{MeshColumns, MeshKey, MeshRow, mesh, mesh_key, process_sides};
pub use poll::{
    PollConfig, QueryProgress, TrafficResult, Truncation, evaluate_draft, run_traffic_query,
    run_traffic_query_draft, truncation,
};
pub use query::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, DEFAULT_TRAFFIC_MAX_RESULTS, DEFAULT_WINDOW,
    MAX_TRAFFIC_MAX_RESULTS, QueryCost, TrafficQueryBuilder, format_timestamp, parse_window,
};
pub use resolved::{DraftSummary, ResolvedEndpoint, ResolvedFlow, resolve_flows, short_decision};
pub use sections::{
    AppSection, Decisions, Direction, PEER_NAMES, PeerRow, SectionOrder, Totals, app_header,
    app_sections, app_sections_both, flow_sections, in_app, in_peer, order_sections, sides,
};
pub use steps::{
    CAP_STEPS, WINDOW_STEPS, backfill_cap, next_cap, next_window, prev_cap, prev_window,
};
