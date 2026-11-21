//! Translation between cross-control's input types and macOS Quartz event
//! data: virtual key codes (`kVK_*`, which name physical key positions, so
//! the mapping is independent of the keyboard layout), modifier flags,
//! mouse buttons and scroll deltas.
//!
//! Pure functions and constants with no Quartz calls, so the mapping is
//! unit-tested on every platform.
#![cfg_attr(not(all(feature = "macos", target_os = "macos")), allow(dead_code))]

use cross_control_types::{ButtonState, KeyCode, MouseButton, ScrollAxis, ScrollDirection};

// --- CGEventType values ------------------------------------------------------

pub const LEFT_MOUSE_DOWN: u32 = 1;
pub const LEFT_MOUSE_UP: u32 = 2;
pub const RIGHT_MOUSE_DOWN: u32 = 3;
pub const RIGHT_MOUSE_UP: u32 = 4;
pub const MOUSE_MOVED: u32 = 5;
pub const LEFT_MOUSE_DRAGGED: u32 = 6;
pub const RIGHT_MOUSE_DRAGGED: u32 = 7;
pub const KEY_DOWN: u32 = 10;
pub const KEY_UP: u32 = 11;
pub const FLAGS_CHANGED: u32 = 12;
pub const SCROLL_WHEEL: u32 = 22;
pub const OTHER_MOUSE_DOWN: u32 = 25;
pub const OTHER_MOUSE_UP: u32 = 26;
pub const OTHER_MOUSE_DRAGGED: u32 = 27;
/// The system disabled the tap because a callback took too long.
pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
/// The system disabled the tap because of user input (secure input fields).
pub const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

// --- CGEventField values -----------------------------------------------------

pub const FIELD_MOUSE_BUTTON_NUMBER: u32 = 3;
pub const FIELD_MOUSE_DELTA_X: u32 = 4;
pub const FIELD_MOUSE_DELTA_Y: u32 = 5;
pub const FIELD_KEYBOARD_KEYCODE: u32 = 9;
/// `kCGEventSourceUserData`: a value the poster attaches to an event.
pub const FIELD_SOURCE_USER_DATA: u32 = 42;
pub const FIELD_SCROLL_FIXED_DELTA_AXIS_1: u32 = 93;
pub const FIELD_SCROLL_FIXED_DELTA_AXIS_2: u32 = 94;

/// What this process stamps into `kCGEventSourceUserData` on every event it
/// posts, so its own capture tap can recognise and ignore them ("CCMI").
pub const INJECTED_MARKER: i64 = 0x4343_4D49;

// --- Modifier flags (CGEventFlags) -------------------------------------------

pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CONTROL: u64 = 0x0004_0000;
pub const FLAG_ALTERNATE: u64 = 0x0008_0000;
pub const FLAG_COMMAND: u64 = 0x0010_0000;

// Device-dependent bits (IOKit NX_DEVICE*KEYMASK) that tell left from right.
const DEVICE_LCTL: u64 = 0x0000_0001;
const DEVICE_LSHIFT: u64 = 0x0000_0002;
const DEVICE_RSHIFT: u64 = 0x0000_0004;
const DEVICE_LCMD: u64 = 0x0000_0008;
const DEVICE_RCMD: u64 = 0x0000_0010;
const DEVICE_LALT: u64 = 0x0000_0020;
const DEVICE_RALT: u64 = 0x0000_0040;
const DEVICE_RCTL: u64 = 0x0000_2000;

// --- Virtual key codes -------------------------------------------------------

pub const KVK_CAPS_LOCK: u16 = 0x39;

/// `kVK_*` code for each named key. PC-only keys use the codes Apple
/// assigns to them on PC keyboards: Insert is Help, Num Lock is Clear, and
/// Print Screen / Scroll Lock / Pause are F13 / F14 / F15.
const KEY_CODES: &[(KeyCode, u16)] = &[
    (KeyCode::KeyA, 0x00),
    (KeyCode::KeyS, 0x01),
    (KeyCode::KeyD, 0x02),
    (KeyCode::KeyF, 0x03),
    (KeyCode::KeyH, 0x04),
    (KeyCode::KeyG, 0x05),
    (KeyCode::KeyZ, 0x06),
    (KeyCode::KeyX, 0x07),
    (KeyCode::KeyC, 0x08),
    (KeyCode::KeyV, 0x09),
    (KeyCode::KeyB, 0x0B),
    (KeyCode::KeyQ, 0x0C),
    (KeyCode::KeyW, 0x0D),
    (KeyCode::KeyE, 0x0E),
    (KeyCode::KeyR, 0x0F),
    (KeyCode::KeyY, 0x10),
    (KeyCode::KeyT, 0x11),
    (KeyCode::Digit1, 0x12),
    (KeyCode::Digit2, 0x13),
    (KeyCode::Digit3, 0x14),
    (KeyCode::Digit4, 0x15),
    (KeyCode::Digit6, 0x16),
    (KeyCode::Digit5, 0x17),
    (KeyCode::Equal, 0x18),
    (KeyCode::Digit9, 0x19),
    (KeyCode::Digit7, 0x1A),
    (KeyCode::Minus, 0x1B),
    (KeyCode::Digit8, 0x1C),
    (KeyCode::Digit0, 0x1D),
    (KeyCode::BracketRight, 0x1E),
    (KeyCode::KeyO, 0x1F),
    (KeyCode::KeyU, 0x20),
    (KeyCode::BracketLeft, 0x21),
    (KeyCode::KeyI, 0x22),
    (KeyCode::KeyP, 0x23),
    (KeyCode::Enter, 0x24),
    (KeyCode::KeyL, 0x25),
    (KeyCode::KeyJ, 0x26),
    (KeyCode::Quote, 0x27),
    (KeyCode::KeyK, 0x28),
    (KeyCode::Semicolon, 0x29),
    (KeyCode::Backslash, 0x2A),
    (KeyCode::Comma, 0x2B),
    (KeyCode::Slash, 0x2C),
    (KeyCode::KeyN, 0x2D),
    (KeyCode::KeyM, 0x2E),
    (KeyCode::Period, 0x2F),
    (KeyCode::Tab, 0x30),
    (KeyCode::Space, 0x31),
    (KeyCode::Backquote, 0x32),
    (KeyCode::Backspace, 0x33),
    (KeyCode::Escape, 0x35),
    (KeyCode::RightMeta, 0x36),
    (KeyCode::LeftMeta, 0x37),
    (KeyCode::LeftShift, 0x38),
    (KeyCode::CapsLock, KVK_CAPS_LOCK),
    (KeyCode::LeftAlt, 0x3A),
    (KeyCode::LeftCtrl, 0x3B),
    (KeyCode::RightShift, 0x3C),
    (KeyCode::RightAlt, 0x3D),
    (KeyCode::RightCtrl, 0x3E),
    (KeyCode::NumpadDecimal, 0x41),
    (KeyCode::NumpadMultiply, 0x43),
    (KeyCode::NumpadAdd, 0x45),
    (KeyCode::NumLock, 0x47),
    (KeyCode::VolumeUp, 0x48),
    (KeyCode::VolumeDown, 0x49),
    (KeyCode::Mute, 0x4A),
    (KeyCode::NumpadDivide, 0x4B),
    (KeyCode::NumpadEnter, 0x4C),
    (KeyCode::NumpadSubtract, 0x4E),
    (KeyCode::Numpad0, 0x52),
    (KeyCode::Numpad1, 0x53),
    (KeyCode::Numpad2, 0x54),
    (KeyCode::Numpad3, 0x55),
    (KeyCode::Numpad4, 0x56),
    (KeyCode::Numpad5, 0x57),
    (KeyCode::Numpad6, 0x58),
    (KeyCode::Numpad7, 0x59),
    (KeyCode::Numpad8, 0x5B),
    (KeyCode::Numpad9, 0x5C),
    (KeyCode::F5, 0x60),
    (KeyCode::F6, 0x61),
    (KeyCode::F7, 0x62),
    (KeyCode::F3, 0x63),
    (KeyCode::F8, 0x64),
    (KeyCode::F9, 0x65),
    (KeyCode::F11, 0x67),
    (KeyCode::PrintScreen, 0x69),
    (KeyCode::ScrollLock, 0x6B),
    (KeyCode::F10, 0x6D),
    (KeyCode::F12, 0x6F),
    (KeyCode::Pause, 0x71),
    (KeyCode::Insert, 0x72),
    (KeyCode::Home, 0x73),
    (KeyCode::PageUp, 0x74),
    (KeyCode::Delete, 0x75),
    (KeyCode::F4, 0x76),
    (KeyCode::End, 0x77),
    (KeyCode::F2, 0x78),
    (KeyCode::PageDown, 0x79),
    (KeyCode::F1, 0x7A),
    (KeyCode::ArrowLeft, 0x7B),
    (KeyCode::ArrowRight, 0x7C),
    (KeyCode::ArrowDown, 0x7D),
    (KeyCode::ArrowUp, 0x7E),
];

/// The `kVK_*` code to post for `code`. Unknown keys carry their raw code.
#[must_use]
pub fn keycode(code: KeyCode) -> Option<u16> {
    match code {
        KeyCode::Unknown(raw) => u16::try_from(raw).ok(),
        named => KEY_CODES
            .iter()
            .find(|&&(k, _)| k == named)
            .map(|&(_, vk)| vk),
    }
}

/// The key for a `kVK_*` code from a captured event. Codes without a
/// named key come back as [`KeyCode::Unknown`], so they still round-trip.
#[must_use]
pub fn key_from_keycode(vk: u16) -> KeyCode {
    KEY_CODES
        .iter()
        .find(|&&(_, v)| v == vk)
        .map_or(KeyCode::Unknown(u32::from(vk)), |&(k, _)| k)
}

/// For a modifier key: its generic flag and the device-dependent bit that
/// identifies the left or right key. `None` for other keys (and Caps Lock,
/// which toggles rather than being held).
#[must_use]
pub fn modifier_bits(code: KeyCode) -> Option<(u64, u64)> {
    Some(match code {
        KeyCode::LeftShift => (FLAG_SHIFT, DEVICE_LSHIFT),
        KeyCode::RightShift => (FLAG_SHIFT, DEVICE_RSHIFT),
        KeyCode::LeftCtrl => (FLAG_CONTROL, DEVICE_LCTL),
        KeyCode::RightCtrl => (FLAG_CONTROL, DEVICE_RCTL),
        KeyCode::LeftAlt => (FLAG_ALTERNATE, DEVICE_LALT),
        KeyCode::RightAlt => (FLAG_ALTERNATE, DEVICE_RALT),
        KeyCode::LeftMeta => (FLAG_COMMAND, DEVICE_LCMD),
        KeyCode::RightMeta => (FLAG_COMMAND, DEVICE_RCMD),
        _ => return None,
    })
}

/// A `FlagsChanged` event says a modifier changed, not whether it went
/// down or up: read that from the event's flags. Caps Lock reports each
/// toggle, so it is treated as a press (the caller follows with a release).
#[must_use]
pub fn modifier_state(code: KeyCode, flags: u64) -> Option<ButtonState> {
    if code == KeyCode::CapsLock {
        return Some(ButtonState::Pressed);
    }
    let (_, device) = modifier_bits(code)?;
    Some(if flags & device != 0 {
        ButtonState::Pressed
    } else {
        ButtonState::Released
    })
}

/// The flags to put on posted events while `held` modifiers are down, so
/// applications see e.g. Cmd+C rather than a bare C.
#[must_use]
pub fn flags_for(held: impl IntoIterator<Item = KeyCode>) -> u64 {
    held.into_iter()
        .filter_map(modifier_bits)
        .fold(0, |flags, (generic, device)| flags | generic | device)
}

/// Whether `code` is a modifier key, which macOS posts as `FlagsChanged`.
#[must_use]
pub fn is_modifier(code: KeyCode) -> bool {
    modifier_bits(code).is_some() || code == KeyCode::CapsLock
}

// --- Mouse -------------------------------------------------------------------

/// Quartz button number: 0 left, 1 right, 2 middle, 3 back, 4 forward.
#[must_use]
pub fn button_number(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(n) => u32::from(n),
    }
}

#[must_use]
pub fn button_from_number(n: u32) -> MouseButton {
    match n {
        0 => MouseButton::Left,
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        other => MouseButton::Other(u16::try_from(other).unwrap_or(u16::MAX)),
    }
}

/// The button and state of a captured button event type, or `None` for
/// other event types. "Other" buttons carry their number in the event.
#[must_use]
pub fn button_event(event_type: u32, other_number: u32) -> Option<(MouseButton, ButtonState)> {
    Some(match event_type {
        LEFT_MOUSE_DOWN => (MouseButton::Left, ButtonState::Pressed),
        LEFT_MOUSE_UP => (MouseButton::Left, ButtonState::Released),
        RIGHT_MOUSE_DOWN => (MouseButton::Right, ButtonState::Pressed),
        RIGHT_MOUSE_UP => (MouseButton::Right, ButtonState::Released),
        OTHER_MOUSE_DOWN => (button_from_number(other_number), ButtonState::Pressed),
        OTHER_MOUSE_UP => (button_from_number(other_number), ButtonState::Released),
        _ => return None,
    })
}

/// The event type to post for a button change.
#[must_use]
pub fn button_event_type(button: MouseButton, state: ButtonState) -> u32 {
    let pressed = state == ButtonState::Pressed;
    match (button, pressed) {
        (MouseButton::Left, true) => LEFT_MOUSE_DOWN,
        (MouseButton::Left, false) => LEFT_MOUSE_UP,
        (MouseButton::Right, true) => RIGHT_MOUSE_DOWN,
        (MouseButton::Right, false) => RIGHT_MOUSE_UP,
        (_, true) => OTHER_MOUSE_DOWN,
        (_, false) => OTHER_MOUSE_UP,
    }
}

/// The event type for motion while `held` buttons are down: macOS sends
/// "dragged" events then, and apps rely on them for drag and drop.
#[must_use]
pub fn motion_event_type(left: bool, right: bool, other: bool) -> u32 {
    if left {
        LEFT_MOUSE_DRAGGED
    } else if right {
        RIGHT_MOUSE_DRAGGED
    } else if other {
        OTHER_MOUSE_DRAGGED
    } else {
        MOUSE_MOVED
    }
}

/// Whether an event type carries pointer motion.
#[must_use]
pub fn is_motion(event_type: u32) -> bool {
    matches!(
        event_type,
        MOUSE_MOVED | LEFT_MOUSE_DRAGGED | RIGHT_MOUSE_DRAGGED | OTHER_MOUSE_DRAGGED
    )
}

// --- Scroll ------------------------------------------------------------------

/// A captured scroll, from its fixed-point line deltas (vertical: positive
/// is up; horizontal: positive is left, the opposite of cross-control's
/// convention). `None` when nothing moved on that axis.
#[must_use]
pub fn scroll_from_deltas(
    vertical: f64,
    horizontal: f64,
) -> Vec<(ScrollAxis, ScrollDirection, f64)> {
    let mut out = Vec::new();
    let dir = |positive: bool| {
        if positive {
            ScrollDirection::Positive
        } else {
            ScrollDirection::Negative
        }
    };
    if vertical != 0.0 {
        out.push((ScrollAxis::Vertical, dir(vertical > 0.0), vertical.abs()));
    }
    if horizontal != 0.0 {
        out.push((
            ScrollAxis::Horizontal,
            dir(horizontal < 0.0),
            horizontal.abs(),
        ));
    }
    out
}

/// Line deltas (vertical, horizontal) to post for a scroll, in Quartz's
/// sign convention.
#[must_use]
pub fn scroll_deltas(axis: ScrollAxis, direction: ScrollDirection, amount: f64) -> (f64, f64) {
    let magnitude = amount.abs();
    match (axis, direction) {
        (ScrollAxis::Vertical, ScrollDirection::Positive) => (magnitude, 0.0),
        (ScrollAxis::Vertical, ScrollDirection::Negative) => (-magnitude, 0.0),
        (ScrollAxis::Horizontal, ScrollDirection::Positive) => (0.0, -magnitude),
        (ScrollAxis::Horizontal, ScrollDirection::Negative) => (0.0, magnitude),
    }
}

/// Whole lines for the integer delta fields: at least one line in the
/// right direction for any non-zero fractional scroll.
#[must_use]
pub fn whole_lines(delta: f64) -> i32 {
    if delta == 0.0 {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let lines = delta
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    if lines == 0 {
        if delta > 0.0 {
            1
        } else {
            -1
        }
    } else {
        lines
    }
}

/// 16.16 fixed-point encoding used by the `*FixedPtDelta*` fields.
#[must_use]
pub fn to_fixed(v: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let fixed = (v * 65536.0).round() as i64;
    fixed
}

#[must_use]
pub fn from_fixed(v: i64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let f = v as f64 / 65536.0;
    f
}

/// Position within the display bounds as a fraction (0.0..=1.0); the last
/// point maps to 1.0. Quartz uses floating-point global coordinates.
#[must_use]
pub fn normalize_position(
    x: f64,
    y: f64,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let frac = |v: f64, origin: f64, size: f64| {
        if size <= 1.0 {
            0.0
        } else {
            ((v - origin) / (size - 1.0)).clamp(0.0, 1.0)
        }
    };
    (frac(x, left, width), frac(y, top, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_key_round_trips() {
        for &(code, vk) in KEY_CODES {
            assert_eq!(keycode(code), Some(vk), "{code:?}");
            assert_eq!(key_from_keycode(vk), code, "{vk:#04x}");
        }
    }

    #[test]
    fn key_codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &(code, vk) in KEY_CODES {
            assert!(seen.insert(vk), "{vk:#04x} is mapped twice ({code:?})");
        }
    }

    #[test]
    fn well_known_codes() {
        assert_eq!(keycode(KeyCode::KeyA), Some(0x00));
        assert_eq!(keycode(KeyCode::Enter), Some(0x24));
        assert_eq!(keycode(KeyCode::LeftMeta), Some(0x37));
        assert_eq!(keycode(KeyCode::Escape), Some(0x35));
        assert_eq!(keycode(KeyCode::ArrowUp), Some(0x7E));
        // Backspace is kVK_Delete; forward delete is kVK_ForwardDelete.
        assert_eq!(keycode(KeyCode::Backspace), Some(0x33));
        assert_eq!(keycode(KeyCode::Delete), Some(0x75));
    }

    #[test]
    fn unknown_keys_round_trip_their_code() {
        // F17 has no named key.
        assert_eq!(key_from_keycode(0x40), KeyCode::Unknown(0x40));
        assert_eq!(keycode(KeyCode::Unknown(0x40)), Some(0x40));
        assert_eq!(keycode(KeyCode::Unknown(0x1_0000)), None);
    }

    #[test]
    fn modifiers_tell_left_from_right() {
        let left = flags_for([KeyCode::LeftShift]);
        let right = flags_for([KeyCode::RightShift]);
        assert_eq!(left & FLAG_SHIFT, FLAG_SHIFT);
        assert_ne!(left, right);
        assert_eq!(
            modifier_state(KeyCode::LeftShift, left),
            Some(ButtonState::Pressed)
        );
        assert_eq!(
            modifier_state(KeyCode::RightShift, left),
            Some(ButtonState::Released)
        );
        assert_eq!(modifier_state(KeyCode::KeyA, left), None);
        assert_eq!(
            flags_for([KeyCode::LeftMeta, KeyCode::LeftAlt]) & (FLAG_COMMAND | FLAG_ALTERNATE),
            FLAG_COMMAND | FLAG_ALTERNATE
        );
        assert!(is_modifier(KeyCode::CapsLock) && !is_modifier(KeyCode::Space));
    }

    #[test]
    fn buttons_round_trip() {
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
            MouseButton::Other(7),
        ] {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                let event_type = button_event_type(button, state);
                assert_eq!(
                    button_event(event_type, button_number(button)),
                    Some((button, state))
                );
            }
        }
        assert_eq!(button_event(MOUSE_MOVED, 0), None);
        assert_eq!(motion_event_type(false, false, false), MOUSE_MOVED);
        assert_eq!(motion_event_type(true, true, false), LEFT_MOUSE_DRAGGED);
        assert!(is_motion(OTHER_MOUSE_DRAGGED) && !is_motion(KEY_DOWN));
    }

    #[test]
    fn scroll_round_trips_with_quartz_signs() {
        for axis in [ScrollAxis::Vertical, ScrollAxis::Horizontal] {
            for direction in [ScrollDirection::Positive, ScrollDirection::Negative] {
                let (v, h) = scroll_deltas(axis, direction, 1.5);
                assert_eq!(scroll_from_deltas(v, h), vec![(axis, direction, 1.5)]);
            }
        }
        // Up is positive on Quartz's vertical axis; right is negative on its
        // horizontal axis.
        assert_eq!(
            scroll_deltas(ScrollAxis::Vertical, ScrollDirection::Positive, 1.0),
            (1.0, 0.0)
        );
        assert_eq!(
            scroll_deltas(ScrollAxis::Horizontal, ScrollDirection::Positive, 1.0),
            (0.0, -1.0)
        );
        assert_eq!(scroll_from_deltas(0.0, 0.0), Vec::new());
    }

    #[test]
    fn whole_lines_never_lose_a_small_scroll() {
        assert_eq!(whole_lines(0.0), 0);
        assert_eq!(whole_lines(0.2), 1);
        assert_eq!(whole_lines(-0.2), -1);
        assert_eq!(whole_lines(2.6), 3);
        assert!((from_fixed(to_fixed(1.25)) - 1.25).abs() < 1e-9);
    }

    #[test]
    fn positions_normalize_over_the_display_bounds() {
        assert_eq!(
            normalize_position(0.0, 0.0, 0.0, 0.0, 1920.0, 1080.0),
            (0.0, 0.0)
        );
        assert_eq!(
            normalize_position(1919.0, 1079.0, 0.0, 0.0, 1920.0, 1080.0),
            (1.0, 1.0)
        );
        // A display left of the main one has negative x.
        assert_eq!(
            normalize_position(-1920.0, 0.0, -1920.0, 0.0, 3840.0, 1080.0),
            (0.0, 0.0)
        );
        assert_eq!(
            normalize_position(5000.0, -5.0, 0.0, 0.0, 1920.0, 1080.0),
            (1.0, 0.0)
        );
    }
}
