//! The interactive TUI. Elm-style: `model` holds all state, `msg` lists
//! every event, `update` is a pure state transition that returns `Cmd`s,
//! `ui` renders `&Model`, and `app` is the only place I/O happens.
pub mod app;
pub mod commands;
pub mod diff;
pub mod edit;
pub mod filter;
pub mod form;
pub mod impact;
pub mod model;
pub mod msg;
pub mod table;
pub mod test_support;
pub mod ui;
pub mod update;

pub use model::Model;
pub use msg::{Cmd, Msg};
pub use ui::view;
pub use update::update;
