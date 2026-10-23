//! Library half of the binary so integration tests can reach the CLI
//! modules. `main.rs` is a thin wrapper.
pub mod cli;
pub mod config;
pub mod filters;
pub mod marks;
pub mod names;
pub mod rule_scope;
pub mod tokens;
pub mod tui;
