//! Mock input backends for testing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cross_control_types::{
    Barrier, BarrierId, CapturedEvent, DeviceInfo, InputEvent, VirtualDeviceId,
};
use tokio::sync::mpsc;

use crate::error::InputError;
use crate::{InputCapture, InputEmulation};

// ---------------------------------------------------------------------------
// MockCapture
// ---------------------------------------------------------------------------

/// Mock input capture backend for testing.
///
/// Returns a `mpsc::Sender<CapturedEvent>` that tests use to inject events.
/// When `start()` is called, it spawns a task that forwards injected events
/// to the daemon's capture channel.
pub struct MockCapture {
    feed_rx: Option<mpsc::Receiver<CapturedEvent>>,
    barriers: Arc<Mutex<HashMap<BarrierId, Barrier>>>,
    released: Arc<AtomicBool>,
    next_barrier: AtomicU32,
    shutdown: Arc<AtomicBool>,
}

impl MockCapture {
    /// Create a new mock capture and a sender for injecting events.
    pub fn new() -> (Self, mpsc::Sender<CapturedEvent>) {
        let (feed_tx, feed_rx) = mpsc::channel(1024);
        let capture = Self {
            feed_rx: Some(feed_rx),
            barriers: Arc::new(Mutex::new(HashMap::new())),
            released: Arc::new(AtomicBool::new(false)),
            next_barrier: AtomicU32::new(1),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        (capture, feed_tx)
    }

    /// Check if `release()` was called.
    pub fn was_released(&self) -> bool {
        self.released.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl InputCapture for MockCapture {
    async fn start(&mut self, tx: mpsc::Sender<CapturedEvent>) -> Result<(), InputError> {
        let mut feed_rx = self
            .feed_rx
            .take()
            .ok_or_else(|| InputError::Other(anyhow::anyhow!("MockCapture already started")))?;
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            while let Some(event) = feed_rx.recv().await {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(())
    }

    async fn add_barrier(&mut self, barrier: Barrier) -> Result<BarrierId, InputError> {
        let id = BarrierId(self.next_barrier.fetch_add(1, Ordering::SeqCst));
        let mut b = barrier;
        b.id = id;
        self.barriers.lock().unwrap().insert(id, b);
        Ok(id)
    }

    async fn remove_barrier(&mut self, id: BarrierId) -> Result<(), InputError> {
        self.barriers
            .lock()
            .unwrap()
            .remove(&id)
            .ok_or(InputError::BarrierNotFound(id))?;
        Ok(())
    }

    async fn release(&mut self) -> Result<(), InputError> {
        self.released.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        self.shutdown.store(true, Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockEmulation
// ---------------------------------------------------------------------------

/// Recorded injection event for test observation.
#[derive(Debug, Clone)]
pub struct InjectedEvent {
    pub device: VirtualDeviceId,
    pub event: InputEvent,
}

/// Shared state for observing what `MockEmulation` did.
#[derive(Debug, Default)]
struct MockEmulationState {
    devices: HashMap<VirtualDeviceId, DeviceInfo>,
    injected: Vec<InjectedEvent>,
    next_id: u32,
    shutdown: bool,
}

/// Mock input emulation backend for testing.
pub struct MockEmulation {
    state: Arc<Mutex<MockEmulationState>>,
}

impl Default for MockEmulation {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEmulation {
    /// Create a new mock emulation backend.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockEmulationState::default())),
        }
    }

    /// Get a clonable handle for observing the emulation state from tests.
    pub fn handle(&self) -> MockEmulationHandle {
        MockEmulationHandle {
            state: Arc::clone(&self.state),
        }
    }
}

/// Clonable observer handle for `MockEmulation`.
///
/// Tests use this to inspect created devices and injected events.
#[derive(Clone)]
pub struct MockEmulationHandle {
    state: Arc<Mutex<MockEmulationState>>,
}

impl MockEmulationHandle {
    /// Get a snapshot of all created virtual devices.
    pub fn devices(&self) -> HashMap<VirtualDeviceId, DeviceInfo> {
        self.state.lock().unwrap().devices.clone()
    }

    /// Get a snapshot of all injected events.
    pub fn injected_events(&self) -> Vec<InjectedEvent> {
        self.state.lock().unwrap().injected.clone()
    }

    /// Check if shutdown was called.
    pub fn is_shutdown(&self) -> bool {
        self.state.lock().unwrap().shutdown
    }
}

#[async_trait]
impl InputEmulation for MockEmulation {
    async fn create_device(&mut self, info: &DeviceInfo) -> Result<VirtualDeviceId, InputError> {
        let mut state = self.state.lock().unwrap();
        state.next_id += 1;
        let id = VirtualDeviceId(state.next_id);
        state.devices.insert(id, info.clone());
        Ok(id)
    }

    async fn inject(
        &mut self,
        device: VirtualDeviceId,
        event: InputEvent,
    ) -> Result<(), InputError> {
        let mut state = self.state.lock().unwrap();
        state.injected.push(InjectedEvent { device, event });
        Ok(())
    }

    async fn destroy_device(&mut self, device: VirtualDeviceId) -> Result<(), InputError> {
        let mut state = self.state.lock().unwrap();
        state.devices.remove(&device);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        let mut state = self.state.lock().unwrap();
        state.shutdown = true;
        Ok(())
    }
}
