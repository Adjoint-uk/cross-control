//! Core daemon for cross-control.
//!
//! Implements the state machine for barrier logic, event routing, session
//! management, and IPC server for the CLI to communicate with.

pub mod config;
pub mod daemon;
pub mod error;
pub mod session;
pub mod setup;
pub mod state;

pub use config::Config;
pub use daemon::{Daemon, DaemonEvent, DaemonStatus};
pub use error::DaemonError;
