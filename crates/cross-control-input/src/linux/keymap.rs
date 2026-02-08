//! Bidirectional mapping between evdev keys and cross-control types.

use cross_control_types::{ButtonState, KeyCode, MouseButton, ScrollAxis};
use evdev::KeyCode as EvdevKey;
use evdev::RelativeAxisCode;

/// Convert an evdev `KeyCode` to our `KeyCode`.
#[allow(clippy::too_many_lines)]
pub fn evdev_key_to_keycode(key: EvdevKey) -> KeyCode {
    match key {
        // Letters
        EvdevKey::KEY_A => KeyCode::KeyA,
        EvdevKey::KEY_B => KeyCode::KeyB,
        EvdevKey::KEY_C => KeyCode::KeyC,
        EvdevKey::KEY_D => KeyCode::KeyD,
        EvdevKey::KEY_E => KeyCode::KeyE,
        EvdevKey::KEY_F => KeyCode::KeyF,
        EvdevKey::KEY_G => KeyCode::KeyG,
        EvdevKey::KEY_H => KeyCode::KeyH,
        EvdevKey::KEY_I => KeyCode::KeyI,
        EvdevKey::KEY_J => KeyCode::KeyJ,
        EvdevKey::KEY_K => KeyCode::KeyK,
        EvdevKey::KEY_L => KeyCode::KeyL,
        EvdevKey::KEY_M => KeyCode::KeyM,
        EvdevKey::KEY_N => KeyCode::KeyN,
        EvdevKey::KEY_O => KeyCode::KeyO,
        EvdevKey::KEY_P => KeyCode::KeyP,
        EvdevKey::KEY_Q => KeyCode::KeyQ,
        EvdevKey::KEY_R => KeyCode::KeyR,
        EvdevKey::KEY_S => KeyCode::KeyS,
        EvdevKey::KEY_T => KeyCode::KeyT,
        EvdevKey::KEY_U => KeyCode::KeyU,
        EvdevKey::KEY_V => KeyCode::KeyV,
        EvdevKey::KEY_W => KeyCode::KeyW,
        EvdevKey::KEY_X => KeyCode::KeyX,
        EvdevKey::KEY_Y => KeyCode::KeyY,
        EvdevKey::KEY_Z => KeyCode::KeyZ,

        // Numbers
        EvdevKey::KEY_0 => KeyCode::Digit0,
        EvdevKey::KEY_1 => KeyCode::Digit1,
        EvdevKey::KEY_2 => KeyCode::Digit2,
        EvdevKey::KEY_3 => KeyCode::Digit3,
        EvdevKey::KEY_4 => KeyCode::Digit4,
        EvdevKey::KEY_5 => KeyCode::Digit5,
        EvdevKey::KEY_6 => KeyCode::Digit6,
        EvdevKey::KEY_7 => KeyCode::Digit7,
        EvdevKey::KEY_8 => KeyCode::Digit8,
        EvdevKey::KEY_9 => KeyCode::Digit9,

        // Function keys
        EvdevKey::KEY_F1 => KeyCode::F1,
        EvdevKey::KEY_F2 => KeyCode::F2,
        EvdevKey::KEY_F3 => KeyCode::F3,
        EvdevKey::KEY_F4 => KeyCode::F4,
        EvdevKey::KEY_F5 => KeyCode::F5,
        EvdevKey::KEY_F6 => KeyCode::F6,
        EvdevKey::KEY_F7 => KeyCode::F7,
        EvdevKey::KEY_F8 => KeyCode::F8,
        EvdevKey::KEY_F9 => KeyCode::F9,
        EvdevKey::KEY_F10 => KeyCode::F10,
        EvdevKey::KEY_F11 => KeyCode::F11,
        EvdevKey::KEY_F12 => KeyCode::F12,

        // Modifiers
        EvdevKey::KEY_LEFTSHIFT => KeyCode::LeftShift,
        EvdevKey::KEY_RIGHTSHIFT => KeyCode::RightShift,
        EvdevKey::KEY_LEFTCTRL => KeyCode::LeftCtrl,
        EvdevKey::KEY_RIGHTCTRL => KeyCode::RightCtrl,
        EvdevKey::KEY_LEFTALT => KeyCode::LeftAlt,
        EvdevKey::KEY_RIGHTALT => KeyCode::RightAlt,
        EvdevKey::KEY_LEFTMETA => KeyCode::LeftMeta,
        EvdevKey::KEY_RIGHTMETA => KeyCode::RightMeta,

        // Navigation
        EvdevKey::KEY_ENTER => KeyCode::Enter,
        EvdevKey::KEY_ESC => KeyCode::Escape,
        EvdevKey::KEY_BACKSPACE => KeyCode::Backspace,
        EvdevKey::KEY_TAB => KeyCode::Tab,
        EvdevKey::KEY_SPACE => KeyCode::Space,
        EvdevKey::KEY_CAPSLOCK => KeyCode::CapsLock,
        EvdevKey::KEY_SYSRQ => KeyCode::PrintScreen,
        EvdevKey::KEY_SCROLLLOCK => KeyCode::ScrollLock,
        EvdevKey::KEY_PAUSE => KeyCode::Pause,
        EvdevKey::KEY_INSERT => KeyCode::Insert,
        EvdevKey::KEY_DELETE => KeyCode::Delete,
        EvdevKey::KEY_HOME => KeyCode::Home,
        EvdevKey::KEY_END => KeyCode::End,
        EvdevKey::KEY_PAGEUP => KeyCode::PageUp,
        EvdevKey::KEY_PAGEDOWN => KeyCode::PageDown,
        EvdevKey::KEY_UP => KeyCode::ArrowUp,
        EvdevKey::KEY_DOWN => KeyCode::ArrowDown,
        EvdevKey::KEY_LEFT => KeyCode::ArrowLeft,
        EvdevKey::KEY_RIGHT => KeyCode::ArrowRight,

        // Punctuation
        EvdevKey::KEY_MINUS => KeyCode::Minus,
        EvdevKey::KEY_EQUAL => KeyCode::Equal,
        EvdevKey::KEY_LEFTBRACE => KeyCode::BracketLeft,
        EvdevKey::KEY_RIGHTBRACE => KeyCode::BracketRight,
        EvdevKey::KEY_BACKSLASH => KeyCode::Backslash,
        EvdevKey::KEY_SEMICOLON => KeyCode::Semicolon,
        EvdevKey::KEY_APOSTROPHE => KeyCode::Quote,
        EvdevKey::KEY_GRAVE => KeyCode::Backquote,
        EvdevKey::KEY_COMMA => KeyCode::Comma,
        EvdevKey::KEY_DOT => KeyCode::Period,
        EvdevKey::KEY_SLASH => KeyCode::Slash,

        // Numpad
        EvdevKey::KEY_NUMLOCK => KeyCode::NumLock,
        EvdevKey::KEY_KPSLASH => KeyCode::NumpadDivide,
        EvdevKey::KEY_KPASTERISK => KeyCode::NumpadMultiply,
        EvdevKey::KEY_KPMINUS => KeyCode::NumpadSubtract,
        EvdevKey::KEY_KPPLUS => KeyCode::NumpadAdd,
        EvdevKey::KEY_KPENTER => KeyCode::NumpadEnter,
        EvdevKey::KEY_KP0 => KeyCode::Numpad0,
        EvdevKey::KEY_KP1 => KeyCode::Numpad1,
        EvdevKey::KEY_KP2 => KeyCode::Numpad2,
        EvdevKey::KEY_KP3 => KeyCode::Numpad3,
        EvdevKey::KEY_KP4 => KeyCode::Numpad4,
        EvdevKey::KEY_KP5 => KeyCode::Numpad5,
        EvdevKey::KEY_KP6 => KeyCode::Numpad6,
        EvdevKey::KEY_KP7 => KeyCode::Numpad7,
        EvdevKey::KEY_KP8 => KeyCode::Numpad8,
        EvdevKey::KEY_KP9 => KeyCode::Numpad9,
        EvdevKey::KEY_KPDOT => KeyCode::NumpadDecimal,

        // Media
        EvdevKey::KEY_MUTE => KeyCode::Mute,
        EvdevKey::KEY_VOLUMEUP => KeyCode::VolumeUp,
        EvdevKey::KEY_VOLUMEDOWN => KeyCode::VolumeDown,

        other => KeyCode::Unknown(u32::from(other.0)),
    }
}

/// Convert our `KeyCode` to an evdev `KeyCode`.
#[allow(clippy::too_many_lines)]
pub fn keycode_to_evdev_key(code: KeyCode) -> EvdevKey {
    match code {
        // Letters
        KeyCode::KeyA => EvdevKey::KEY_A,
        KeyCode::KeyB => EvdevKey::KEY_B,
        KeyCode::KeyC => EvdevKey::KEY_C,
        KeyCode::KeyD => EvdevKey::KEY_D,
        KeyCode::KeyE => EvdevKey::KEY_E,
        KeyCode::KeyF => EvdevKey::KEY_F,
        KeyCode::KeyG => EvdevKey::KEY_G,
        KeyCode::KeyH => EvdevKey::KEY_H,
        KeyCode::KeyI => EvdevKey::KEY_I,
        KeyCode::KeyJ => EvdevKey::KEY_J,
        KeyCode::KeyK => EvdevKey::KEY_K,
        KeyCode::KeyL => EvdevKey::KEY_L,
        KeyCode::KeyM => EvdevKey::KEY_M,
        KeyCode::KeyN => EvdevKey::KEY_N,
        KeyCode::KeyO => EvdevKey::KEY_O,
        KeyCode::KeyP => EvdevKey::KEY_P,
        KeyCode::KeyQ => EvdevKey::KEY_Q,
        KeyCode::KeyR => EvdevKey::KEY_R,
        KeyCode::KeyS => EvdevKey::KEY_S,
        KeyCode::KeyT => EvdevKey::KEY_T,
        KeyCode::KeyU => EvdevKey::KEY_U,
        KeyCode::KeyV => EvdevKey::KEY_V,
        KeyCode::KeyW => EvdevKey::KEY_W,
        KeyCode::KeyX => EvdevKey::KEY_X,
        KeyCode::KeyY => EvdevKey::KEY_Y,
        KeyCode::KeyZ => EvdevKey::KEY_Z,

        // Numbers
        KeyCode::Digit0 => EvdevKey::KEY_0,
        KeyCode::Digit1 => EvdevKey::KEY_1,
        KeyCode::Digit2 => EvdevKey::KEY_2,
        KeyCode::Digit3 => EvdevKey::KEY_3,
        KeyCode::Digit4 => EvdevKey::KEY_4,
        KeyCode::Digit5 => EvdevKey::KEY_5,
        KeyCode::Digit6 => EvdevKey::KEY_6,
        KeyCode::Digit7 => EvdevKey::KEY_7,
        KeyCode::Digit8 => EvdevKey::KEY_8,
        KeyCode::Digit9 => EvdevKey::KEY_9,

        // Function keys
        KeyCode::F1 => EvdevKey::KEY_F1,
        KeyCode::F2 => EvdevKey::KEY_F2,
        KeyCode::F3 => EvdevKey::KEY_F3,
        KeyCode::F4 => EvdevKey::KEY_F4,
        KeyCode::F5 => EvdevKey::KEY_F5,
        KeyCode::F6 => EvdevKey::KEY_F6,
        KeyCode::F7 => EvdevKey::KEY_F7,
        KeyCode::F8 => EvdevKey::KEY_F8,
        KeyCode::F9 => EvdevKey::KEY_F9,
        KeyCode::F10 => EvdevKey::KEY_F10,
        KeyCode::F11 => EvdevKey::KEY_F11,
        KeyCode::F12 => EvdevKey::KEY_F12,

        // Modifiers
        KeyCode::LeftShift => EvdevKey::KEY_LEFTSHIFT,
        KeyCode::RightShift => EvdevKey::KEY_RIGHTSHIFT,
        KeyCode::LeftCtrl => EvdevKey::KEY_LEFTCTRL,
        KeyCode::RightCtrl => EvdevKey::KEY_RIGHTCTRL,
        KeyCode::LeftAlt => EvdevKey::KEY_LEFTALT,
        KeyCode::RightAlt => EvdevKey::KEY_RIGHTALT,
        KeyCode::LeftMeta => EvdevKey::KEY_LEFTMETA,
        KeyCode::RightMeta => EvdevKey::KEY_RIGHTMETA,

        // Navigation
        KeyCode::Enter => EvdevKey::KEY_ENTER,
        KeyCode::Escape => EvdevKey::KEY_ESC,
        KeyCode::Backspace => EvdevKey::KEY_BACKSPACE,
        KeyCode::Tab => EvdevKey::KEY_TAB,
        KeyCode::Space => EvdevKey::KEY_SPACE,
        KeyCode::CapsLock => EvdevKey::KEY_CAPSLOCK,
        KeyCode::PrintScreen => EvdevKey::KEY_SYSRQ,
        KeyCode::ScrollLock => EvdevKey::KEY_SCROLLLOCK,
        KeyCode::Pause => EvdevKey::KEY_PAUSE,
        KeyCode::Insert => EvdevKey::KEY_INSERT,
        KeyCode::Delete => EvdevKey::KEY_DELETE,
        KeyCode::Home => EvdevKey::KEY_HOME,
        KeyCode::End => EvdevKey::KEY_END,
        KeyCode::PageUp => EvdevKey::KEY_PAGEUP,
        KeyCode::PageDown => EvdevKey::KEY_PAGEDOWN,
        KeyCode::ArrowUp => EvdevKey::KEY_UP,
        KeyCode::ArrowDown => EvdevKey::KEY_DOWN,
        KeyCode::ArrowLeft => EvdevKey::KEY_LEFT,
        KeyCode::ArrowRight => EvdevKey::KEY_RIGHT,

        // Punctuation
        KeyCode::Minus => EvdevKey::KEY_MINUS,
        KeyCode::Equal => EvdevKey::KEY_EQUAL,
        KeyCode::BracketLeft => EvdevKey::KEY_LEFTBRACE,
        KeyCode::BracketRight => EvdevKey::KEY_RIGHTBRACE,
        KeyCode::Backslash => EvdevKey::KEY_BACKSLASH,
        KeyCode::Semicolon => EvdevKey::KEY_SEMICOLON,
        KeyCode::Quote => EvdevKey::KEY_APOSTROPHE,
        KeyCode::Backquote => EvdevKey::KEY_GRAVE,
        KeyCode::Comma => EvdevKey::KEY_COMMA,
        KeyCode::Period => EvdevKey::KEY_DOT,
        KeyCode::Slash => EvdevKey::KEY_SLASH,

        // Numpad
        KeyCode::NumLock => EvdevKey::KEY_NUMLOCK,
        KeyCode::NumpadDivide => EvdevKey::KEY_KPSLASH,
        KeyCode::NumpadMultiply => EvdevKey::KEY_KPASTERISK,
        KeyCode::NumpadSubtract => EvdevKey::KEY_KPMINUS,
        KeyCode::NumpadAdd => EvdevKey::KEY_KPPLUS,
        KeyCode::NumpadEnter => EvdevKey::KEY_KPENTER,
        KeyCode::Numpad0 => EvdevKey::KEY_KP0,
        KeyCode::Numpad1 => EvdevKey::KEY_KP1,
        KeyCode::Numpad2 => EvdevKey::KEY_KP2,
        KeyCode::Numpad3 => EvdevKey::KEY_KP3,
        KeyCode::Numpad4 => EvdevKey::KEY_KP4,
        KeyCode::Numpad5 => EvdevKey::KEY_KP5,
        KeyCode::Numpad6 => EvdevKey::KEY_KP6,
        KeyCode::Numpad7 => EvdevKey::KEY_KP7,
        KeyCode::Numpad8 => EvdevKey::KEY_KP8,
        KeyCode::Numpad9 => EvdevKey::KEY_KP9,
        KeyCode::NumpadDecimal => EvdevKey::KEY_KPDOT,

        // Media
        KeyCode::Mute => EvdevKey::KEY_MUTE,
        KeyCode::VolumeUp => EvdevKey::KEY_VOLUMEUP,
        KeyCode::VolumeDown => EvdevKey::KEY_VOLUMEDOWN,

        #[allow(clippy::cast_possible_truncation)]
        KeyCode::Unknown(raw) => EvdevKey(raw as u16),
    }
}

/// Try to convert an evdev `KeyCode` in the BTN_* range to a `MouseButton`.
pub fn evdev_key_to_mouse_button(key: EvdevKey) -> Option<MouseButton> {
    match key {
        EvdevKey::BTN_LEFT => Some(MouseButton::Left),
        EvdevKey::BTN_RIGHT => Some(MouseButton::Right),
        EvdevKey::BTN_MIDDLE => Some(MouseButton::Middle),
        EvdevKey::BTN_SIDE => Some(MouseButton::Back),
        EvdevKey::BTN_EXTRA => Some(MouseButton::Forward),
        other if other.0 >= 0x110 && other.0 <= 0x11f => Some(MouseButton::Other(other.0)),
        _ => None,
    }
}

/// Convert a `MouseButton` to an evdev `KeyCode`.
pub fn mouse_button_to_evdev_key(button: MouseButton) -> EvdevKey {
    match button {
        MouseButton::Left => EvdevKey::BTN_LEFT,
        MouseButton::Right => EvdevKey::BTN_RIGHT,
        MouseButton::Middle => EvdevKey::BTN_MIDDLE,
        MouseButton::Back => EvdevKey::BTN_SIDE,
        MouseButton::Forward => EvdevKey::BTN_EXTRA,
        MouseButton::Other(code) => EvdevKey(code),
    }
}

/// Convert an evdev `RelativeAxisCode` to a `ScrollAxis`, if applicable.
pub fn evdev_rel_to_scroll_axis(axis: RelativeAxisCode) -> Option<ScrollAxis> {
    match axis {
        RelativeAxisCode::REL_WHEEL | RelativeAxisCode::REL_WHEEL_HI_RES => {
            Some(ScrollAxis::Vertical)
        }
        RelativeAxisCode::REL_HWHEEL | RelativeAxisCode::REL_HWHEEL_HI_RES => {
            Some(ScrollAxis::Horizontal)
        }
        _ => None,
    }
}

/// Convert a `ScrollAxis` to the evdev `RelativeAxisCode`.
pub fn scroll_axis_to_evdev_rel(axis: ScrollAxis) -> RelativeAxisCode {
    match axis {
        ScrollAxis::Vertical => RelativeAxisCode::REL_WHEEL,
        ScrollAxis::Horizontal => RelativeAxisCode::REL_HWHEEL,
    }
}

/// Convert an evdev event value (0=released, 1=pressed, 2=repeat) to `ButtonState`.
pub fn evdev_value_to_button_state(value: i32) -> Option<ButtonState> {
    match value {
        0 => Some(ButtonState::Released),
        1 | 2 => Some(ButtonState::Pressed),
        _ => None,
    }
}

/// Convert a `ButtonState` to an evdev event value.
pub fn button_state_to_evdev_value(state: ButtonState) -> i32 {
    match state {
        ButtonState::Pressed => 1,
        ButtonState::Released => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_mapped_keycodes() {
        let keys = [
            EvdevKey::KEY_A,
            EvdevKey::KEY_B,
            EvdevKey::KEY_C,
            EvdevKey::KEY_D,
            EvdevKey::KEY_E,
            EvdevKey::KEY_F,
            EvdevKey::KEY_G,
            EvdevKey::KEY_H,
            EvdevKey::KEY_I,
            EvdevKey::KEY_J,
            EvdevKey::KEY_K,
            EvdevKey::KEY_L,
            EvdevKey::KEY_M,
            EvdevKey::KEY_N,
            EvdevKey::KEY_O,
            EvdevKey::KEY_P,
            EvdevKey::KEY_Q,
            EvdevKey::KEY_R,
            EvdevKey::KEY_S,
            EvdevKey::KEY_T,
            EvdevKey::KEY_U,
            EvdevKey::KEY_V,
            EvdevKey::KEY_W,
            EvdevKey::KEY_X,
            EvdevKey::KEY_Y,
            EvdevKey::KEY_Z,
            EvdevKey::KEY_0,
            EvdevKey::KEY_1,
            EvdevKey::KEY_2,
            EvdevKey::KEY_3,
            EvdevKey::KEY_4,
            EvdevKey::KEY_5,
            EvdevKey::KEY_6,
            EvdevKey::KEY_7,
            EvdevKey::KEY_8,
            EvdevKey::KEY_9,
            EvdevKey::KEY_F1,
            EvdevKey::KEY_F2,
            EvdevKey::KEY_F3,
            EvdevKey::KEY_F4,
            EvdevKey::KEY_F5,
            EvdevKey::KEY_F6,
            EvdevKey::KEY_F7,
            EvdevKey::KEY_F8,
            EvdevKey::KEY_F9,
            EvdevKey::KEY_F10,
            EvdevKey::KEY_F11,
            EvdevKey::KEY_F12,
            EvdevKey::KEY_LEFTSHIFT,
            EvdevKey::KEY_RIGHTSHIFT,
            EvdevKey::KEY_LEFTCTRL,
            EvdevKey::KEY_RIGHTCTRL,
            EvdevKey::KEY_LEFTALT,
            EvdevKey::KEY_RIGHTALT,
            EvdevKey::KEY_LEFTMETA,
            EvdevKey::KEY_RIGHTMETA,
            EvdevKey::KEY_ENTER,
            EvdevKey::KEY_ESC,
            EvdevKey::KEY_BACKSPACE,
            EvdevKey::KEY_TAB,
            EvdevKey::KEY_SPACE,
            EvdevKey::KEY_CAPSLOCK,
            EvdevKey::KEY_SYSRQ,
            EvdevKey::KEY_SCROLLLOCK,
            EvdevKey::KEY_PAUSE,
            EvdevKey::KEY_INSERT,
            EvdevKey::KEY_DELETE,
            EvdevKey::KEY_HOME,
            EvdevKey::KEY_END,
            EvdevKey::KEY_PAGEUP,
            EvdevKey::KEY_PAGEDOWN,
            EvdevKey::KEY_UP,
            EvdevKey::KEY_DOWN,
            EvdevKey::KEY_LEFT,
            EvdevKey::KEY_RIGHT,
            EvdevKey::KEY_MINUS,
            EvdevKey::KEY_EQUAL,
            EvdevKey::KEY_LEFTBRACE,
            EvdevKey::KEY_RIGHTBRACE,
            EvdevKey::KEY_BACKSLASH,
            EvdevKey::KEY_SEMICOLON,
            EvdevKey::KEY_APOSTROPHE,
            EvdevKey::KEY_GRAVE,
            EvdevKey::KEY_COMMA,
            EvdevKey::KEY_DOT,
            EvdevKey::KEY_SLASH,
            EvdevKey::KEY_NUMLOCK,
            EvdevKey::KEY_KPSLASH,
            EvdevKey::KEY_KPASTERISK,
            EvdevKey::KEY_KPMINUS,
            EvdevKey::KEY_KPPLUS,
            EvdevKey::KEY_KPENTER,
            EvdevKey::KEY_KP0,
            EvdevKey::KEY_KP1,
            EvdevKey::KEY_KP2,
            EvdevKey::KEY_KP3,
            EvdevKey::KEY_KP4,
            EvdevKey::KEY_KP5,
            EvdevKey::KEY_KP6,
            EvdevKey::KEY_KP7,
            EvdevKey::KEY_KP8,
            EvdevKey::KEY_KP9,
            EvdevKey::KEY_KPDOT,
            EvdevKey::KEY_MUTE,
            EvdevKey::KEY_VOLUMEUP,
            EvdevKey::KEY_VOLUMEDOWN,
        ];

        for key in keys {
            let code = evdev_key_to_keycode(key);
            let back = keycode_to_evdev_key(code);
            assert_eq!(
                key, back,
                "round-trip failed for {key:?} -> {code:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn unknown_key_roundtrip() {
        let exotic = EvdevKey(0x300);
        let code = evdev_key_to_keycode(exotic);
        assert!(matches!(code, KeyCode::Unknown(0x300)));
        let back = keycode_to_evdev_key(code);
        assert_eq!(exotic, back);
    }

    #[test]
    fn mouse_button_roundtrip() {
        let buttons = [
            (EvdevKey::BTN_LEFT, MouseButton::Left),
            (EvdevKey::BTN_RIGHT, MouseButton::Right),
            (EvdevKey::BTN_MIDDLE, MouseButton::Middle),
            (EvdevKey::BTN_SIDE, MouseButton::Back),
            (EvdevKey::BTN_EXTRA, MouseButton::Forward),
        ];

        for (key, expected_btn) in buttons {
            let btn = evdev_key_to_mouse_button(key).unwrap();
            assert_eq!(btn, expected_btn);
            let back = mouse_button_to_evdev_key(btn);
            assert_eq!(key, back);
        }
    }

    #[test]
    fn scroll_axis_roundtrip() {
        assert_eq!(
            evdev_rel_to_scroll_axis(RelativeAxisCode::REL_WHEEL),
            Some(ScrollAxis::Vertical)
        );
        assert_eq!(
            evdev_rel_to_scroll_axis(RelativeAxisCode::REL_HWHEEL),
            Some(ScrollAxis::Horizontal)
        );
        assert_eq!(
            scroll_axis_to_evdev_rel(ScrollAxis::Vertical),
            RelativeAxisCode::REL_WHEEL,
        );
        assert_eq!(
            scroll_axis_to_evdev_rel(ScrollAxis::Horizontal),
            RelativeAxisCode::REL_HWHEEL,
        );
    }

    #[test]
    fn button_state_conversion() {
        assert_eq!(evdev_value_to_button_state(0), Some(ButtonState::Released));
        assert_eq!(evdev_value_to_button_state(1), Some(ButtonState::Pressed));
        assert_eq!(evdev_value_to_button_state(2), Some(ButtonState::Pressed));
        assert_eq!(evdev_value_to_button_state(-1), None);
        assert_eq!(button_state_to_evdev_value(ButtonState::Pressed), 1);
        assert_eq!(button_state_to_evdev_value(ButtonState::Released), 0);
    }

    #[test]
    fn non_mouse_key_returns_none() {
        assert!(evdev_key_to_mouse_button(EvdevKey::KEY_A).is_none());
    }

    #[test]
    fn non_scroll_rel_returns_none() {
        assert!(evdev_rel_to_scroll_axis(RelativeAxisCode::REL_X).is_none());
    }
}
