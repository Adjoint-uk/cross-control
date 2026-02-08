//! uinput-based input emulation for Linux.

use std::collections::HashMap;

use async_trait::async_trait;
use cross_control_types::{
    DeviceCapability, DeviceInfo, InputEvent, ScrollDirection, VirtualDeviceId,
};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, KeyCode as EvdevKey, RelativeAxisCode};
use tracing::{debug, info};

use super::keymap;
use crate::error::InputError;
use crate::InputEmulation;

/// Linux input emulation using uinput virtual devices.
pub struct UinputEmulation {
    devices: HashMap<VirtualDeviceId, VirtualDevice>,
    next_id: u32,
}

impl Default for UinputEmulation {
    fn default() -> Self {
        Self::new()
    }
}

impl UinputEmulation {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
            next_id: 1,
        }
    }

    fn build_virtual_device(info: &DeviceInfo) -> Result<VirtualDevice, InputError> {
        let mut builder = VirtualDevice::builder()
            .map_err(|e| InputError::VirtualDeviceCreate(e.to_string()))?
            .name(&info.name);

        for cap in &info.capabilities {
            match cap {
                DeviceCapability::Keyboard => {
                    let mut keys = AttributeSet::<EvdevKey>::new();
                    // Register all standard keys
                    for code in 1..=248 {
                        keys.insert(EvdevKey(code));
                    }
                    builder = builder
                        .with_keys(&keys)
                        .map_err(|e| InputError::VirtualDeviceCreate(e.to_string()))?;
                }
                DeviceCapability::RelativeMouse => {
                    let mut keys = AttributeSet::<EvdevKey>::new();
                    // Mouse buttons
                    keys.insert(EvdevKey::BTN_LEFT);
                    keys.insert(EvdevKey::BTN_RIGHT);
                    keys.insert(EvdevKey::BTN_MIDDLE);
                    keys.insert(EvdevKey::BTN_SIDE);
                    keys.insert(EvdevKey::BTN_EXTRA);
                    builder = builder
                        .with_keys(&keys)
                        .map_err(|e| InputError::VirtualDeviceCreate(e.to_string()))?;

                    let mut rel = AttributeSet::<RelativeAxisCode>::new();
                    rel.insert(RelativeAxisCode::REL_X);
                    rel.insert(RelativeAxisCode::REL_Y);
                    builder = builder
                        .with_relative_axes(&rel)
                        .map_err(|e| InputError::VirtualDeviceCreate(e.to_string()))?;
                }
                DeviceCapability::AbsoluteMouse => {
                    // Not needed for MVP, relative mouse covers Linux
                }
                DeviceCapability::Scroll => {
                    let mut rel = AttributeSet::<RelativeAxisCode>::new();
                    rel.insert(RelativeAxisCode::REL_WHEEL);
                    rel.insert(RelativeAxisCode::REL_HWHEEL);
                    builder = builder
                        .with_relative_axes(&rel)
                        .map_err(|e| InputError::VirtualDeviceCreate(e.to_string()))?;
                }
            }
        }

        builder
            .build()
            .map_err(|e| InputError::VirtualDeviceCreate(e.to_string()))
    }
}

#[async_trait]
impl InputEmulation for UinputEmulation {
    async fn create_device(&mut self, info: &DeviceInfo) -> Result<VirtualDeviceId, InputError> {
        let device = Self::build_virtual_device(info)?;
        let id = VirtualDeviceId(self.next_id);
        self.next_id += 1;
        info!(id = id.0, name = %info.name, "created virtual device");
        self.devices.insert(id, device);
        Ok(id)
    }

    async fn inject(
        &mut self,
        device: VirtualDeviceId,
        event: InputEvent,
    ) -> Result<(), InputError> {
        let vdev = self
            .devices
            .get_mut(&device)
            .ok_or_else(|| InputError::Inject(format!("unknown virtual device {}", device.0)))?;

        let evdev_events = input_event_to_evdev(&event);
        if !evdev_events.is_empty() {
            vdev.emit(&evdev_events)
                .map_err(|e| InputError::Inject(e.to_string()))?;
        }
        debug!(?event, device = device.0, "injected event");
        Ok(())
    }

    async fn destroy_device(&mut self, device: VirtualDeviceId) -> Result<(), InputError> {
        if self.devices.remove(&device).is_some() {
            info!(id = device.0, "destroyed virtual device");
            Ok(())
        } else {
            Err(InputError::Inject(format!(
                "unknown virtual device {}",
                device.0
            )))
        }
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        let count = self.devices.len();
        self.devices.clear();
        info!(count, "shut down emulation backend");
        Ok(())
    }
}

/// Convert our `InputEvent` to a list of evdev `InputEvent`s.
fn input_event_to_evdev(event: &InputEvent) -> Vec<evdev::InputEvent> {
    match event {
        InputEvent::Key { code, state } => {
            let key = keymap::keycode_to_evdev_key(*code);
            let value = keymap::button_state_to_evdev_value(*state);
            vec![evdev::InputEvent::new(EventType::KEY.0, key.0, value)]
        }
        InputEvent::MouseMove { dx, dy } => {
            vec![
                evdev::InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, *dx),
                evdev::InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_Y.0, *dy),
            ]
        }
        InputEvent::MouseMoveAbsolute { .. } => {
            // Absolute mouse not yet supported in Linux MVP
            vec![]
        }
        InputEvent::MouseButton { button, state } => {
            let key = keymap::mouse_button_to_evdev_key(*button);
            let value = keymap::button_state_to_evdev_value(*state);
            vec![evdev::InputEvent::new(EventType::KEY.0, key.0, value)]
        }
        InputEvent::Scroll {
            axis,
            direction,
            amount,
        } => {
            let rel_axis = keymap::scroll_axis_to_evdev_rel(*axis);
            #[allow(clippy::cast_possible_truncation)]
            let value = match direction {
                ScrollDirection::Positive => *amount as i32,
                ScrollDirection::Negative => -(*amount as i32),
            };
            vec![evdev::InputEvent::new(
                EventType::RELATIVE.0,
                rel_axis.0,
                value,
            )]
        }
    }
}
