//! Clipboard subsystem errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard access denied")]
    AccessDenied,

    #[error("requested format not available")]
    FormatUnavailable,

    #[error("clipboard content too large: {size} bytes (max {max} bytes)")]
    TooLarge { size: usize, max: usize },

    #[error("backend not available on this platform")]
    Unavailable,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
