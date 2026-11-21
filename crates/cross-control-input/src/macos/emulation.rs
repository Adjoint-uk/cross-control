//! Input emulation with `CGEventPost`.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use cross_control_types::{
    ButtonState, DeviceInfo, InputEvent, KeyCode, MouseButton, VirtualDeviceId,
};
use tracing::{trace, warn};

use super::{cursor_position, desktop_bounds};
use crate::mac_map::{
    button_event_type, button_number, flags_for, is_modifier, keycode, motion_event_type,
    scroll_deltas, to_fixed, whole_lines, FIELD_MOUSE_BUTTON_NUMBER, FIELD_MOUSE_DELTA_X,
    FIELD_MOUSE_DELTA_Y, FIELD_SCROLL_FIXED_DELTA_AXIS_1, FIELD_SCROLL_FIXED_DELTA_AXIS_2,
    FIELD_SOURCE_USER_DATA, INJECTED_MARKER, LEFT_MOUSE_DOWN, LEFT_MOUSE_DRAGGED, LEFT_MOUSE_UP,
    MOUSE_MOVED, OTHER_MOUSE_DOWN, OTHER_MOUSE_DRAGGED, OTHER_MOUSE_UP, RIGHT_MOUSE_DOWN,
    RIGHT_MOUSE_DRAGGED, RIGHT_MOUSE_UP,
};
use crate::{InputEmulation, InputError};

/// `kCGMouseEventClickState`: 1 for a single click, 2 for a double click…
const FIELD_CLICK_STATE: u32 = 1;
/// Clicks closer together than this (in time and points) count as one
/// multi-click, as with a physical mouse.
const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f64 = 4.0;

/// macOS input emulation.
pub struct MacEmulation {
    next_device: u32,
    held_keys: HashSet<KeyCode>,
    held_buttons: HashSet<MouseButton>,
    /// Last button press, for click counting: (button, when, where, count).
    last_click: Option<(MouseButton, Instant, (f64, f64), i64)>,
    /// Stamped into `kCGEventSourceUserData` on every posted event.
    marker: i64,
}

impl Default for MacEmulation {
    fn default() -> Self {
        Self::new()
    }
}

impl MacEmulation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_device: 0,
            held_keys: HashSet::new(),
            held_buttons: HashSet::new(),
            last_click: None,
            marker: INJECTED_MARKER,
        }
    }

    /// Tests only: an emulator whose events carry no marker, so this
    /// process's own tap captures them as if they came from hardware.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn as_hardware() -> Self {
        Self {
            marker: 0,
            ..Self::new()
        }
    }

    fn flags(&self) -> CGEventFlags {
        CGEventFlags::from_bits_retain(flags_for(self.held_keys.iter().copied()))
    }

    fn post(&self, event: &CGEvent) {
        // Mark it as ours, so this process's own tap ignores it.
        event.set_integer_value_field(FIELD_SOURCE_USER_DATA, self.marker);
        event.set_flags(self.flags());
        event.post(CGEventTapLocation::HID);
    }

    fn click_count(&mut self, button: MouseButton, at: (f64, f64)) -> i64 {
        let now = Instant::now();
        let count = match self.last_click {
            Some((b, when, (x, y), n))
                if b == button
                    && now.duration_since(when) <= DOUBLE_CLICK_TIME
                    && (x - at.0).hypot(y - at.1) <= DOUBLE_CLICK_DISTANCE =>
            {
                n + 1
            }
            _ => 1,
        };
        self.last_click = Some((button, now, at, count));
        count
    }

    fn inject_event(&mut self, event: &InputEvent) -> Result<(), InputError> {
        let source = || {
            CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|()| fail("cannot create an event source"))
        };
        match *event {
            InputEvent::Key { code, state } => {
                let Some(vk) = keycode(code) else {
                    return Ok(()); // no macOS key for it
                };
                let pressed = state == ButtonState::Pressed;
                if pressed {
                    self.held_keys.insert(code);
                } else {
                    self.held_keys.remove(&code);
                }
                let event = CGEvent::new_keyboard_event(source()?, vk, pressed)
                    .map_err(|()| fail("cannot create a key event"))?;
                if is_modifier(code) {
                    event.set_type(CGEventType::FlagsChanged);
                }
                self.post(&event);
            }
            InputEvent::MouseMove { dx, dy } => {
                let (x, y) = cursor_position().unwrap_or_default();
                let (left, top, width, height) = desktop_bounds();
                let to = (
                    (x + f64::from(dx)).clamp(left, left + width - 1.0),
                    (y + f64::from(dy)).clamp(top, top + height - 1.0),
                );
                let event = self.motion_event(source()?, to)?;
                event.set_integer_value_field(FIELD_MOUSE_DELTA_X, i64::from(dx));
                event.set_integer_value_field(FIELD_MOUSE_DELTA_Y, i64::from(dy));
                self.post(&event);
            }
            InputEvent::MouseMoveAbsolute { x, y } => {
                let (left, top, width, height) = desktop_bounds();
                let to = (
                    left + x.clamp(0.0, 1.0) * (width - 1.0),
                    top + y.clamp(0.0, 1.0) * (height - 1.0),
                );
                let event = self.motion_event(source()?, to)?;
                self.post(&event);
            }
            InputEvent::MouseButton { button, state } => {
                let at = cursor_position().unwrap_or_default();
                let pressed = state == ButtonState::Pressed;
                let clicks = if pressed {
                    self.held_buttons.insert(button);
                    self.click_count(button, at)
                } else {
                    self.held_buttons.remove(&button);
                    self.last_click.map_or(1, |(_, _, _, n)| n)
                };
                let event = CGEvent::new_mouse_event(
                    source()?,
                    cg_type(button_event_type(button, state)),
                    CGPoint::new(at.0, at.1),
                    cg_button(button),
                )
                .map_err(|()| fail("cannot create a mouse button event"))?;
                event.set_integer_value_field(
                    FIELD_MOUSE_BUTTON_NUMBER,
                    i64::from(button_number(button)),
                );
                event.set_integer_value_field(FIELD_CLICK_STATE, clicks);
                self.post(&event);
            }
            InputEvent::Scroll {
                axis,
                direction,
                amount,
            } => {
                let (vertical, horizontal) = scroll_deltas(axis, direction, amount);
                let event = CGEvent::new_scroll_event(
                    source()?,
                    ScrollEventUnit::LINE,
                    2,
                    whole_lines(vertical),
                    whole_lines(horizontal),
                    0,
                )
                .map_err(|()| fail("cannot create a scroll event"))?;
                // Fractional (smooth) scrolling travels in the fixed-point fields.
                event.set_integer_value_field(FIELD_SCROLL_FIXED_DELTA_AXIS_1, to_fixed(vertical));
                event
                    .set_integer_value_field(FIELD_SCROLL_FIXED_DELTA_AXIS_2, to_fixed(horizontal));
                self.post(&event);
            }
        }
        Ok(())
    }

    /// A motion event to `to`: "dragged" while a button is held, as macOS
    /// sends for a physical mouse (drag and drop depends on it).
    fn motion_event(&self, source: CGEventSource, to: (f64, f64)) -> Result<CGEvent, InputError> {
        let left = self.held_buttons.contains(&MouseButton::Left);
        let right = self.held_buttons.contains(&MouseButton::Right);
        let other = self
            .held_buttons
            .iter()
            .find(|b| !matches!(b, MouseButton::Left | MouseButton::Right));
        let event_type = motion_event_type(left, right, other.is_some());
        let button = if left {
            MouseButton::Left
        } else if right {
            MouseButton::Right
        } else {
            other.copied().unwrap_or(MouseButton::Left)
        };
        let event = CGEvent::new_mouse_event(
            source,
            cg_type(event_type),
            CGPoint::new(to.0, to.1),
            cg_button(button),
        )
        .map_err(|()| fail("cannot create a motion event"))?;
        if event_type == OTHER_MOUSE_DRAGGED {
            event.set_integer_value_field(
                FIELD_MOUSE_BUTTON_NUMBER,
                i64::from(button_number(button)),
            );
        }
        Ok(event)
    }
}

fn fail(what: &str) -> InputError {
    warn!("{what}");
    InputError::Inject(what.to_string())
}

fn cg_button(button: MouseButton) -> CGMouseButton {
    match button {
        MouseButton::Left => CGMouseButton::Left,
        MouseButton::Right => CGMouseButton::Right,
        _ => CGMouseButton::Center,
    }
}

fn cg_type(event_type: u32) -> CGEventType {
    match event_type {
        LEFT_MOUSE_DOWN => CGEventType::LeftMouseDown,
        LEFT_MOUSE_UP => CGEventType::LeftMouseUp,
        RIGHT_MOUSE_DOWN => CGEventType::RightMouseDown,
        RIGHT_MOUSE_UP => CGEventType::RightMouseUp,
        LEFT_MOUSE_DRAGGED => CGEventType::LeftMouseDragged,
        RIGHT_MOUSE_DRAGGED => CGEventType::RightMouseDragged,
        OTHER_MOUSE_DOWN => CGEventType::OtherMouseDown,
        OTHER_MOUSE_UP => CGEventType::OtherMouseUp,
        OTHER_MOUSE_DRAGGED => CGEventType::OtherMouseDragged,
        MOUSE_MOVED => CGEventType::MouseMoved,
        _ => CGEventType::Null,
    }
}

#[async_trait]
impl InputEmulation for MacEmulation {
    async fn create_device(&mut self, _info: &DeviceInfo) -> Result<VirtualDeviceId, InputError> {
        self.next_device = self.next_device.wrapping_add(1);
        Ok(VirtualDeviceId(self.next_device))
    }

    async fn inject(
        &mut self,
        _device: VirtualDeviceId,
        event: InputEvent,
    ) -> Result<(), InputError> {
        trace!(?event, "injecting");
        self.inject_event(&event)
    }

    async fn destroy_device(&mut self, _device: VirtualDeviceId) -> Result<(), InputError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        // Release anything still held, so no key or button stays stuck.
        for code in self.held_keys.clone() {
            let _ = self.inject_event(&InputEvent::Key {
                code,
                state: ButtonState::Released,
            });
        }
        for button in self.held_buttons.clone() {
            let _ = self.inject_event(&InputEvent::MouseButton {
                button,
                state: ButtonState::Released,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macos::{desktop_unavailable, permissions, DESKTOP_LOCK};

    #[test]
    fn click_counting_matches_a_physical_mouse() {
        let mut e = MacEmulation::new();
        assert_eq!(e.click_count(MouseButton::Left, (10.0, 10.0)), 1);
        assert_eq!(e.click_count(MouseButton::Left, (11.0, 10.0)), 2);
        assert_eq!(e.click_count(MouseButton::Right, (11.0, 10.0)), 1);
        assert_eq!(e.click_count(MouseButton::Right, (40.0, 10.0)), 1);
    }

    #[tokio::test]
    async fn relative_motion_moves_the_cursor() {
        let _desktop = DESKTOP_LOCK.lock().await;
        if !permissions().post {
            desktop_unavailable("no Accessibility permission to post events");
            return;
        }
        let _ = core_graphics::display::CGDisplay::warp_mouse_cursor_position(CGPoint::new(
            200.0, 200.0,
        ));
        let before = cursor_position().map_or(0.0, |p| p.0);
        let mut emulation = MacEmulation::new();
        emulation
            .inject(VirtualDeviceId(1), InputEvent::MouseMove { dx: 40, dy: 0 })
            .await
            .unwrap();
        let mut after = before;
        for _ in 0..50 {
            after = cursor_position().map_or(0.0, |p| p.0);
            if after > before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            after > before,
            "cursor did not move right: {before} -> {after}"
        );
    }
}
