//! Library surface for the `opencode` CLI.
//!
//! The binary in `main.rs` is a single large module, so the pieces of `run` that must be
//! observable from tests live here instead of behind the `[[bin]]` boundary.

pub mod background;
pub mod children;
pub mod livestream;
pub mod plugin_notify;
pub mod remote_status;
pub mod run_session;