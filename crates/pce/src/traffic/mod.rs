//! The Explorer traffic query: build it, run it, make the rows readable,
//! and aggregate them. Nothing here touches the network except `poll`.
pub mod filter;
pub mod query;

pub use filter::{FilterExpr, resolve_actor};
pub use query::{
    CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, DEFAULT_TRAFFIC_MAX_RESULTS, DEFAULT_WINDOW, QueryCost,
    TrafficQueryBuilder, format_timestamp, parse_window,
};
