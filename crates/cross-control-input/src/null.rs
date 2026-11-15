//! A backend that touches no devices: capture produces nothing and emulation
//! discards what it is given.
//!
//! Selected with `[input] backend = "none"`. The daemon then still runs its
//! network side (discovery, mutual TLS, trust, sessions, clipboard), which
//! makes it useful for smoke-testing a deployment in containers or CI where
//! no input devices exist, and on machines that should be reachable but not
//! share their own keyboard and mouse.

use async_trait::async_trait;
use cross_control_types::{
    Barrier, BarrierId, CapturedEvent, DeviceInfo, InputEvent, VirtualDeviceId,
};
use tokio::sync::mpsc;
use tracing::debug;

use crate::{InputCapture, InputEmulation, InputError};

/// Capture that never produces an event.
#[derive(Debug, Default)]
pub struct NullCapture {
    next_barrier: u32,
}

#[async_trait]
impl InputCapture for NullCapture {
    async fn start(&mut self, _tx: mpsc::Sender<CapturedEvent>) -> Result<(), InputError> {
        debug!("null input capture: no local devices are read");
        Ok(())
    }

    async fn add_barrier(&mut self, _barrier: Barrier) -> Result<BarrierId, InputError> {
        self.next_barrier = self.next_barrier.wrapping_add(1);
        Ok(BarrierId(self.next_barrier))
    }

    async fn remove_barrier(&mut self, _id: BarrierId) -> Result<(), InputError> {
        Ok(())
    }

    async fn grab(&mut self) -> Result<(), InputError> {
        Ok(())
    }

    async fn release(&mut self) -> Result<(), InputError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        Ok(())
    }
}

/// Emulation that accepts devices and drops events, keeping no state beyond
/// an id counter (so it is safe in a long-running daemon).
#[derive(Debug, Default)]
pub struct NullEmulation {
    next_device: u32,
}

#[async_trait]
impl InputEmulation for NullEmulation {
    async fn create_device(&mut self, info: &DeviceInfo) -> Result<VirtualDeviceId, InputError> {
        self.next_device = self.next_device.wrapping_add(1);
        debug!(device = %info.name, "null input emulation: device accepted, events will be dropped");
        Ok(VirtualDeviceId(self.next_device))
    }

    async fn inject(
        &mut self,
        _device: VirtualDeviceId,
        _event: InputEvent,
    ) -> Result<(), InputError> {
        Ok(())
    }

    async fn destroy_device(&mut self, _device: VirtualDeviceId) -> Result<(), InputError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cross_control_types::{DeviceCapability, DeviceId};

    #[tokio::test]
    async fn null_backends_accept_everything_and_keep_nothing() {
        let mut emulation = NullEmulation::default();
        let info = DeviceInfo {
            id: DeviceId(1),
            name: "kbd".into(),
            capabilities: vec![DeviceCapability::Keyboard],
        };
        let a = emulation.create_device(&info).await.unwrap();
        let b = emulation.create_device(&info).await.unwrap();
        assert_ne!(a, b);
        emulation
            .inject(a, InputEvent::MouseMove { dx: 1, dy: 1 })
            .await
            .unwrap();

        let mut capture = NullCapture::default();
        let (tx, mut rx) = mpsc::channel(1);
        capture.start(tx).await.unwrap();
        capture.grab().await.unwrap();
        capture.release().await.unwrap();
        // Nothing is ever produced; the sender was simply dropped.
        assert!(rx.recv().await.is_none());
    }
}
