//! Daemon errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("already running")]
    AlreadyRunning,

    #[error("not running")]
    NotRunning,

    #[error("protocol error: {0}")]
    Protocol(#[from] cross_control_protocol::ProtocolError),

    #[error("input error: {0}")]
    Input(#[from] cross_control_input::InputError),

    #[error("clipboard error: {0}")]
    Clipboard(#[from] cross_control_clipboard::ClipboardError),

    #[error("discovery error: {0}")]
    Discovery(#[from] cross_control_discovery::DiscoveryError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
