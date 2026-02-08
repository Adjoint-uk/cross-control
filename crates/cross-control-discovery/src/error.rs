//! Discovery subsystem errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS registration failed: {0}")]
    Registration(String),

    #[error("mDNS browse failed: {0}")]
    Browse(String),

    #[error("backend not available on this platform")]
    Unavailable,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
