//! Core daemon for cross-control.
//!
//! Implements the state machine for barrier logic, event routing, session
//! management, and IPC server for the CLI to communicate with.

pub mod config;
pub mod error;

pub use config::Config;
pub use error::DaemonError;
