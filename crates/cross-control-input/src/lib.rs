//! Platform-abstracted input capture and emulation for cross-control.
//!
//! This crate defines the [`InputCapture`] and [`InputEmulation`] traits that
//! platform-specific backends must implement. The evdev/uinput (Linux) and
//! Raw Input/SendInput (Windows) backends will be added in later phases.

use async_trait::async_trait;
use cross_control_types::{
    Barrier, BarrierId, CapturedEvent, DeviceInfo, InputEvent, VirtualDeviceId,
};
use tokio::sync::mpsc;

pub mod error;

pub use error::InputError;

/// Captures physical input devices and detects barrier crossings.
///
/// Implementations grab physical keyboards/mice, forward events through a
/// channel, and detect when the cursor hits a screen-edge barrier.
#[async_trait]
pub trait InputCapture: Send + 'static {
    /// Start capturing input, sending events to `tx`.
    async fn start(&mut self, tx: mpsc::Sender<CapturedEvent>) -> Result<(), InputError>;

    /// Register a barrier for cursor-edge detection.
    async fn add_barrier(&mut self, barrier: Barrier) -> Result<BarrierId, InputError>;

    /// Remove a previously registered barrier.
    async fn remove_barrier(&mut self, id: BarrierId) -> Result<(), InputError>;

    /// Release all grabbed devices (give control back to local machine).
    async fn release(&mut self) -> Result<(), InputError>;

    /// Shut down the capture backend and release all resources.
    async fn shutdown(&mut self) -> Result<(), InputError>;
}

/// Creates virtual input devices and injects events on the controlled machine.
#[async_trait]
pub trait InputEmulation: Send + 'static {
    /// Create a virtual device mirroring the given physical device info.
    async fn create_device(&mut self, info: &DeviceInfo) -> Result<VirtualDeviceId, InputError>;

    /// Inject an input event into a virtual device.
    async fn inject(
        &mut self,
        device: VirtualDeviceId,
        event: InputEvent,
    ) -> Result<(), InputError>;

    /// Destroy a virtual device.
    async fn destroy_device(&mut self, device: VirtualDeviceId) -> Result<(), InputError>;

    /// Shut down the emulation backend and destroy all virtual devices.
    async fn shutdown(&mut self) -> Result<(), InputError>;
}
