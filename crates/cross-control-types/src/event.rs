//! Input event types.
//!
//! Platform-agnostic representations of keyboard, mouse, and scroll events.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::device::DeviceId;

/// A captured input event with metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct CapturedEvent {
    /// Which device produced this event.
    pub device_id: DeviceId,
    /// Microsecond timestamp (monotonic, relative to session start).
    pub timestamp_us: u64,
    /// The event itself.
    pub event: InputEvent,
}

/// A platform-agnostic input event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub enum InputEvent {
    /// Key press or release.
    Key { code: KeyCode, state: ButtonState },

    /// Relative mouse motion.
    MouseMove { dx: i32, dy: i32 },

    /// Absolute mouse position (normalised 0.0..1.0).
    MouseMoveAbsolute { x: f64, y: f64 },

    /// Mouse button press or release.
    MouseButton {
        button: MouseButton,
        state: ButtonState,
    },

    /// Scroll wheel.
    Scroll {
        axis: ScrollAxis,
        direction: ScrollDirection,
        /// Scroll amount (positive = forward/right, negative = backward/left).
        /// For discrete scrolls this is typically 1; for smooth scrolling it
        /// may be fractional.
        amount: f64,
    },
}

/// Button/key state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// Keyboard key code.
///
/// Uses a subset of USB HID usage codes for cross-platform compatibility.
/// Platform backends translate native scancodes to/from these codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum KeyCode {
    // Letters
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,

    // Numbers
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Modifiers
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    LeftMeta,
    RightMeta,

    // Navigation
    Enter,
    Escape,
    Backspace,
    Tab,
    Space,
    CapsLock,
    PrintScreen,
    ScrollLock,
    Pause,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,

    // Punctuation
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,

    // Numpad
    NumLock,
    NumpadDivide,
    NumpadMultiply,
    NumpadSubtract,
    NumpadAdd,
    NumpadEnter,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadDecimal,

    // Media
    Mute,
    VolumeUp,
    VolumeDown,

    /// Fallback for unmapped keys. The value is the raw platform scancode.
    Unknown(u32),
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    /// Extra buttons beyond the standard five.
    Other(u16),
}

/// Scroll axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ScrollAxis {
    Vertical,
    Horizontal,
}

/// Scroll direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ScrollDirection {
    /// Forward (scroll up) or right.
    Positive,
    /// Backward (scroll down) or left.
    Negative,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_event_key_roundtrip() {
        let event = InputEvent::Key {
            code: KeyCode::KeyA,
            state: ButtonState::Pressed,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (InputEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn input_event_mouse_move_roundtrip() {
        let event = InputEvent::MouseMove { dx: -42, dy: 100 };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (InputEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn input_event_scroll_roundtrip() {
        let event = InputEvent::Scroll {
            axis: ScrollAxis::Vertical,
            direction: ScrollDirection::Negative,
            amount: 1.5,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (InputEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn input_event_mouse_button_roundtrip() {
        let event = InputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Released,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (InputEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn captured_event_roundtrip() {
        let event = CapturedEvent {
            device_id: DeviceId(1),
            timestamp_us: 123_456_789,
            event: InputEvent::Key {
                code: KeyCode::LeftCtrl,
                state: ButtonState::Pressed,
            },
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (CapturedEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event.device_id, decoded.device_id);
        assert_eq!(event.timestamp_us, decoded.timestamp_us);
        assert_eq!(event.event, decoded.event);
    }

    #[test]
    fn unknown_keycode_roundtrip() {
        let event = InputEvent::Key {
            code: KeyCode::Unknown(0xDEAD),
            state: ButtonState::Pressed,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (InputEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn mouse_button_other_roundtrip() {
        let event = InputEvent::MouseButton {
            button: MouseButton::Other(42),
            state: ButtonState::Pressed,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&event, config).unwrap();
        let (decoded, _): (InputEvent, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(event, decoded);
    }
}
