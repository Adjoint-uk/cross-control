//! evdev-based input capture for Linux.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use cross_control_types::{
    Barrier, BarrierId, CapturedEvent, DeviceCapability, DeviceId, DeviceInfo, InputEvent,
    ScrollDirection,
};
use evdev::{Device, EventSummary, EventType, KeyCode as EvdevKey, RelativeAxisCode};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::keymap;
use crate::error::InputError;
use crate::InputCapture;

/// Linux input capture using evdev.
///
/// Reads events from `/dev/input/event*` devices passively (no exclusive grab
/// by default). The daemon calls [`grab`] when switching to remote control
/// and [`release`] when returning.
pub struct EvdevCapture {
    devices: HashMap<DeviceId, DeviceEntry>,
    barriers: HashMap<BarrierId, Barrier>,
    next_barrier_id: u32,
    task: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

struct DeviceEntry {
    path: PathBuf,
    #[allow(dead_code)]
    info: DeviceInfo,
}

impl Default for EvdevCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl EvdevCapture {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
            barriers: HashMap::new(),
            next_barrier_id: 1,
            task: None,
            shutdown_tx: None,
        }
    }

    /// Enumerate input devices and return info about keyboards and mice.
    pub fn enumerate_devices() -> Vec<(PathBuf, DeviceInfo)> {
        let mut result = Vec::new();
        let mut dev_id: u32 = 0;

        for (path, device) in evdev::enumerate() {
            let supported = device.supported_events();
            let mut capabilities = Vec::new();

            if supported.contains(EventType::KEY) {
                if let Some(keys) = device.supported_keys() {
                    // Check for keyboard keys
                    let has_keyboard_keys = keys.contains(EvdevKey::KEY_A)
                        && keys.contains(EvdevKey::KEY_Z)
                        && keys.contains(EvdevKey::KEY_ENTER);

                    if has_keyboard_keys {
                        capabilities.push(DeviceCapability::Keyboard);
                    }

                    // Check for mouse buttons
                    if keys.contains(EvdevKey::BTN_LEFT) {
                        capabilities.push(DeviceCapability::RelativeMouse);
                    }
                }
            }

            if supported.contains(EventType::RELATIVE) {
                if let Some(rel) = device.supported_relative_axes() {
                    if rel.contains(RelativeAxisCode::REL_WHEEL)
                        || rel.contains(RelativeAxisCode::REL_HWHEEL)
                    {
                        capabilities.push(DeviceCapability::Scroll);
                    }
                    // Also add RelativeMouse if REL_X/REL_Y present but not already added
                    if (rel.contains(RelativeAxisCode::REL_X)
                        || rel.contains(RelativeAxisCode::REL_Y))
                        && !capabilities.contains(&DeviceCapability::RelativeMouse)
                    {
                        capabilities.push(DeviceCapability::RelativeMouse);
                    }
                }
            }

            if capabilities.is_empty() {
                continue;
            }

            let name = device.name().unwrap_or("Unknown Device").to_string();
            let info = DeviceInfo {
                id: DeviceId(dev_id),
                name,
                capabilities,
            };
            result.push((path, info));
            dev_id += 1;
        }

        result
    }

    /// Grab all tracked devices exclusively (prevents local desktop from receiving input).
    pub fn grab(&mut self) -> Result<(), InputError> {
        for entry in self.devices.values() {
            if let Ok(mut device) = Device::open(&entry.path) {
                device
                    .grab()
                    .map_err(|e| InputError::DeviceGrab(e.to_string()))?;
            }
        }
        info!("grabbed all input devices");
        Ok(())
    }
}

#[async_trait]
impl InputCapture for EvdevCapture {
    async fn start(&mut self, tx: mpsc::Sender<CapturedEvent>) -> Result<(), InputError> {
        let device_list = Self::enumerate_devices();

        if device_list.is_empty() {
            return Err(InputError::DeviceOpen(
                "no keyboard or mouse devices found".to_string(),
            ));
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        for (path, info) in &device_list {
            info!(device = %info.name, path = %path.display(), "tracking device");
            self.devices.insert(
                info.id,
                DeviceEntry {
                    path: path.clone(),
                    info: info.clone(),
                },
            );
        }

        // Spawn reader tasks for each device
        let mut handles = Vec::new();
        for (path, info) in device_list {
            let tx = tx.clone();
            let device_id = info.id;
            let mut shutdown_rx = shutdown_rx.clone();

            let handle: JoinHandle<()> = tokio::spawn(async move {
                let device = match Device::open(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to open device");
                        return;
                    }
                };
                let mut stream = match device.into_event_stream() {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to create event stream");
                        return;
                    }
                };

                loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            break;
                        }
                        result = stream.next_event() => {
                            match result {
                                Ok(ev) => {
                                    if let Some(input_event) = convert_evdev_event(&ev) {
                                        let captured = CapturedEvent {
                                            device_id,
                                            timestamp_us: ev.timestamp().duration_since(std::time::SystemTime::UNIX_EPOCH).ok().and_then(|d| u64::try_from(d.as_micros()).ok()).unwrap_or(0),
                                            event: input_event,
                                        };
                                        if tx.send(captured).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, "device read error");
                                    break;
                                }
                            }
                        }
                    }
                }
            });
            handles.push(handle);
        }

        // Spawn a supervisor that waits for all reader tasks
        self.task = Some(tokio::spawn(async move {
            for h in handles {
                let _ = h.await;
            }
        }));

        info!("input capture started");
        Ok(())
    }

    async fn add_barrier(&mut self, barrier: Barrier) -> Result<BarrierId, InputError> {
        let id = BarrierId(self.next_barrier_id);
        self.next_barrier_id += 1;
        let barrier = Barrier { id, ..barrier };
        debug!(?barrier, "added barrier");
        self.barriers.insert(id, barrier);
        Ok(id)
    }

    async fn remove_barrier(&mut self, id: BarrierId) -> Result<(), InputError> {
        self.barriers
            .remove(&id)
            .ok_or(InputError::BarrierNotFound(id))?;
        Ok(())
    }

    async fn release(&mut self) -> Result<(), InputError> {
        // Re-open devices without grab to release exclusive access
        for entry in self.devices.values() {
            if let Ok(mut device) = Device::open(&entry.path) {
                let _ = device.ungrab();
            }
        }
        info!("released all input devices");
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.release().await?;
        self.devices.clear();
        info!("input capture shut down");
        Ok(())
    }
}

/// Convert a single evdev `InputEvent` to our `InputEvent`, if relevant.
fn convert_evdev_event(ev: &evdev::InputEvent) -> Option<InputEvent> {
    match ev.destructure() {
        EventSummary::Key(_, key, value) => {
            let state = keymap::evdev_value_to_button_state(value)?;
            // Check if it's a mouse button first
            if let Some(button) = keymap::evdev_key_to_mouse_button(key) {
                Some(InputEvent::MouseButton { button, state })
            } else {
                let code = keymap::evdev_key_to_keycode(key);
                Some(InputEvent::Key { code, state })
            }
        }
        EventSummary::RelativeAxis(_, axis, value) => {
            match axis {
                RelativeAxisCode::REL_X => Some(InputEvent::MouseMove { dx: value, dy: 0 }),
                RelativeAxisCode::REL_Y => Some(InputEvent::MouseMove { dx: 0, dy: value }),
                _ => {
                    // Scroll axes
                    if let Some(scroll_axis) = keymap::evdev_rel_to_scroll_axis(axis) {
                        let direction = if value > 0 {
                            ScrollDirection::Positive
                        } else {
                            ScrollDirection::Negative
                        };
                        Some(InputEvent::Scroll {
                            axis: scroll_axis,
                            direction,
                            amount: f64::from(value.abs()),
                        })
                    } else {
                        None
                    }
                }
            }
        }
        _ => None,
    }
}
