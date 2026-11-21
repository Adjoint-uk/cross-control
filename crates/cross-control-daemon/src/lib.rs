//! Core daemon for cross-control.
//!
//! Implements the state machine for barrier logic, event routing, session
//! management, and the local control channel the CLI talks to.

pub mod config;
pub mod control;
pub mod daemon;
pub mod error;
pub mod session;
pub mod setup;
pub mod state;
pub mod status;
pub mod trust;

pub use config::Config;
pub use daemon::{Daemon, DaemonEvent, DaemonStatus};
pub use error::DaemonError;
pub use status::{PeerStatus, StatusSnapshot};
pub use trust::TrustStore;
