//! The interactive TUI. Elm-style: `model` holds all state, `msg` lists
//! every event, `update` is a pure state transition that returns `Cmd`s,
//! `ui` renders `&Model`, and `app` is the only place I/O happens.
pub mod app;
pub mod app_picker;
pub mod app_view;
pub mod commands;
pub mod complete;
pub mod diff;
pub mod edit;
pub mod event_diff;
pub mod export;
pub mod filter;
pub mod form;
pub mod gen_view;
pub mod impact;
pub mod mark_form;
pub mod model;
pub mod msg;
pub mod picker;
pub mod table;
pub mod test_support;
pub mod traffic_marks;
pub mod ui;
pub mod update;

pub use model::Model;
pub use msg::{Cmd, Msg};
pub use ui::view;
pub use update::update;
