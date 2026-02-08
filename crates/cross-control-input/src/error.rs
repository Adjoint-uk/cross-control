//! Input subsystem errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("failed to open device: {0}")]
    DeviceOpen(String),

    #[error("failed to grab device: {0}")]
    DeviceGrab(String),

    #[error("failed to create virtual device: {0}")]
    VirtualDeviceCreate(String),

    #[error("failed to inject event: {0}")]
    Inject(String),

    #[error("barrier not found: {0:?}")]
    BarrierNotFound(cross_control_types::BarrierId),

    #[error("backend not available on this platform")]
    Unavailable,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
