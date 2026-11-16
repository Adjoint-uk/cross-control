//! Translation between cross-control's input types and Windows input data:
//! keyboard scan codes (set 1, as delivered by low-level hooks and accepted by
//! `SendInput` with `KEYEVENTF_SCANCODE`) and mouse window messages.
//!
//! Pure functions with no Win32 calls, so the mapping is unit-tested on every
//! platform. Scan codes identify physical key positions, so the mapping is
//! independent of the keyboard layout — the same model as evdev key codes on
//! Linux.

use cross_control_types::{
    ButtonState, InputEvent, KeyCode, MouseButton, ScrollAxis, ScrollDirection,
};

/// Marks an extended (E0-prefixed) scan code in this module's encoding.
const EXT: u16 = 0xE000;

// Virtual-key codes needed for keys whose scan codes are ambiguous.
pub const VK_PAUSE: u16 = 0x13;
pub const VK_SNAPSHOT: u16 = 0x2C;
pub const VK_NUMLOCK: u16 = 0x90;
pub const VK_RSHIFT: u16 = 0xA1;

// Window messages delivered to low-level mouse hooks.
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_XBUTTONDOWN: u32 = 0x020B;
pub const WM_XBUTTONUP: u32 = 0x020C;
pub const WM_MOUSEHWHEEL: u32 = 0x020E;

/// One wheel detent.
pub const WHEEL_DELTA: f64 = 120.0;

/// Scan code (with [`EXT`] for extended keys) for each named key.
const SCAN_CODES: &[(KeyCode, u16)] = &[
    (KeyCode::KeyA, 0x1E),
    (KeyCode::KeyB, 0x30),
    (KeyCode::KeyC, 0x2E),
    (KeyCode::KeyD, 0x20),
    (KeyCode::KeyE, 0x12),
    (KeyCode::KeyF, 0x21),
    (KeyCode::KeyG, 0x22),
    (KeyCode::KeyH, 0x23),
    (KeyCode::KeyI, 0x17),
    (KeyCode::KeyJ, 0x24),
    (KeyCode::KeyK, 0x25),
    (KeyCode::KeyL, 0x26),
    (KeyCode::KeyM, 0x32),
    (KeyCode::KeyN, 0x31),
    (KeyCode::KeyO, 0x18),
    (KeyCode::KeyP, 0x19),
    (KeyCode::KeyQ, 0x10),
    (KeyCode::KeyR, 0x13),
    (KeyCode::KeyS, 0x1F),
    (KeyCode::KeyT, 0x14),
    (KeyCode::KeyU, 0x16),
    (KeyCode::KeyV, 0x2F),
    (KeyCode::KeyW, 0x11),
    (KeyCode::KeyX, 0x2D),
    (KeyCode::KeyY, 0x15),
    (KeyCode::KeyZ, 0x2C),
    (KeyCode::Digit1, 0x02),
    (KeyCode::Digit2, 0x03),
    (KeyCode::Digit3, 0x04),
    (KeyCode::Digit4, 0x05),
    (KeyCode::Digit5, 0x06),
    (KeyCode::Digit6, 0x07),
    (KeyCode::Digit7, 0x08),
    (KeyCode::Digit8, 0x09),
    (KeyCode::Digit9, 0x0A),
    (KeyCode::Digit0, 0x0B),
    (KeyCode::F1, 0x3B),
    (KeyCode::F2, 0x3C),
    (KeyCode::F3, 0x3D),
    (KeyCode::F4, 0x3E),
    (KeyCode::F5, 0x3F),
    (KeyCode::F6, 0x40),
    (KeyCode::F7, 0x41),
    (KeyCode::F8, 0x42),
    (KeyCode::F9, 0x43),
    (KeyCode::F10, 0x44),
    (KeyCode::F11, 0x57),
    (KeyCode::F12, 0x58),
    (KeyCode::LeftShift, 0x2A),
    (KeyCode::RightShift, 0x36),
    (KeyCode::LeftCtrl, 0x1D),
    (KeyCode::RightCtrl, EXT | 0x1D),
    (KeyCode::LeftAlt, 0x38),
    (KeyCode::RightAlt, EXT | 0x38),
    (KeyCode::LeftMeta, EXT | 0x5B),
    (KeyCode::RightMeta, EXT | 0x5C),
    (KeyCode::Enter, 0x1C),
    (KeyCode::Escape, 0x01),
    (KeyCode::Backspace, 0x0E),
    (KeyCode::Tab, 0x0F),
    (KeyCode::Space, 0x39),
    (KeyCode::CapsLock, 0x3A),
    (KeyCode::PrintScreen, EXT | 0x37),
    (KeyCode::ScrollLock, 0x46),
    (KeyCode::Insert, EXT | 0x52),
    (KeyCode::Delete, EXT | 0x53),
    (KeyCode::Home, EXT | 0x47),
    (KeyCode::End, EXT | 0x4F),
    (KeyCode::PageUp, EXT | 0x49),
    (KeyCode::PageDown, EXT | 0x51),
    (KeyCode::ArrowUp, EXT | 0x48),
    (KeyCode::ArrowDown, EXT | 0x50),
    (KeyCode::ArrowLeft, EXT | 0x4B),
    (KeyCode::ArrowRight, EXT | 0x4D),
    (KeyCode::Minus, 0x0C),
    (KeyCode::Equal, 0x0D),
    (KeyCode::BracketLeft, 0x1A),
    (KeyCode::BracketRight, 0x1B),
    (KeyCode::Backslash, 0x2B),
    (KeyCode::Semicolon, 0x27),
    (KeyCode::Quote, 0x28),
    (KeyCode::Backquote, 0x29),
    (KeyCode::Comma, 0x33),
    (KeyCode::Period, 0x34),
    (KeyCode::Slash, 0x35),
    (KeyCode::NumpadDivide, EXT | 0x35),
    (KeyCode::NumpadMultiply, 0x37),
    (KeyCode::NumpadSubtract, 0x4A),
    (KeyCode::NumpadAdd, 0x4E),
    (KeyCode::NumpadEnter, EXT | 0x1C),
    (KeyCode::Numpad7, 0x47),
    (KeyCode::Numpad8, 0x48),
    (KeyCode::Numpad9, 0x49),
    (KeyCode::Numpad4, 0x4B),
    (KeyCode::Numpad5, 0x4C),
    (KeyCode::Numpad6, 0x4D),
    (KeyCode::Numpad1, 0x4F),
    (KeyCode::Numpad2, 0x50),
    (KeyCode::Numpad3, 0x51),
    (KeyCode::Numpad0, 0x52),
    (KeyCode::NumpadDecimal, 0x53),
    (KeyCode::Mute, EXT | 0x20),
    (KeyCode::VolumeDown, EXT | 0x2E),
    (KeyCode::VolumeUp, EXT | 0x30),
];

/// How to synthesize a key with `SendInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStroke {
    /// By scan code (`KEYEVENTF_SCANCODE`, plus `KEYEVENTF_EXTENDEDKEY` when
    /// `extended`). Layout-independent.
    Scan { code: u16, extended: bool },
    /// By virtual-key code, for keys whose scan codes are ambiguous
    /// (Pause and Num Lock share 0x45).
    Virtual(u16),
}

/// The keystroke that produces `code`.
#[must_use]
pub fn keystroke(code: KeyCode) -> KeyStroke {
    match code {
        KeyCode::Pause => return KeyStroke::Virtual(VK_PAUSE),
        KeyCode::NumLock => return KeyStroke::Virtual(VK_NUMLOCK),
        _ => {}
    }
    let raw = match code {
        // Unknown keys carry this module's encoding (see `key_from_hook`).
        KeyCode::Unknown(raw) => u16::try_from(raw).unwrap_or(0),
        _ => SCAN_CODES
            .iter()
            .find(|(k, _)| *k == code)
            .map_or(0, |(_, scan)| *scan),
    };
    KeyStroke::Scan {
        code: raw & 0xFF,
        extended: raw & EXT == EXT,
    }
}

/// The key a low-level keyboard hook reported, or `None` for events to
/// ignore. `vk`, `scan` and `extended` are `KBDLLHOOKSTRUCT`'s `vkCode`,
/// `scanCode` and `LLKHF_EXTENDED` flag.
#[must_use]
pub fn key_from_hook(vk: u32, scan: u32, extended: bool) -> Option<KeyCode> {
    // AltGr on some layouts is reported as a synthetic Left Ctrl with this
    // scan code, followed by Right Alt: drop the fake Ctrl.
    if scan == 0x21D {
        return None;
    }
    // Keys whose scan codes collide or vary by keyboard: trust the VK.
    match u16::try_from(vk).unwrap_or(0) {
        VK_PAUSE => return Some(KeyCode::Pause),
        VK_NUMLOCK => return Some(KeyCode::NumLock),
        VK_RSHIFT => return Some(KeyCode::RightShift),
        VK_SNAPSHOT => return Some(KeyCode::PrintScreen),
        _ => {}
    }
    let encoded = (u16::try_from(scan & 0xFF).unwrap_or(0)) | if extended { EXT } else { 0 };
    Some(
        SCAN_CODES
            .iter()
            .find(|(_, s)| *s == encoded)
            .map_or(KeyCode::Unknown(u32::from(encoded)), |(k, _)| *k),
    )
}

/// The event for a low-level mouse hook message other than `WM_MOUSEMOVE`
/// (whose deltas the backend computes from positions). `mouse_data` is
/// `MSLLHOOKSTRUCT::mouseData`.
#[must_use]
pub fn mouse_event_from_hook(message: u32, mouse_data: u32) -> Option<InputEvent> {
    let button = |button, state| Some(InputEvent::MouseButton { button, state });
    let high = (mouse_data >> 16) as u16;
    match message {
        WM_LBUTTONDOWN => button(MouseButton::Left, ButtonState::Pressed),
        WM_LBUTTONUP => button(MouseButton::Left, ButtonState::Released),
        WM_RBUTTONDOWN => button(MouseButton::Right, ButtonState::Pressed),
        WM_RBUTTONUP => button(MouseButton::Right, ButtonState::Released),
        WM_MBUTTONDOWN => button(MouseButton::Middle, ButtonState::Pressed),
        WM_MBUTTONUP => button(MouseButton::Middle, ButtonState::Released),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let state = if message == WM_XBUTTONDOWN {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            let which = match high {
                1 => MouseButton::Back,
                2 => MouseButton::Forward,
                other => MouseButton::Other(other),
            };
            button(which, state)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            // The wheel delta is a signed 16-bit value; positive is away
            // from the user (up) or to the right.
            let delta = f64::from(high.cast_signed());
            if delta == 0.0 {
                return None;
            }
            Some(InputEvent::Scroll {
                axis: if message == WM_MOUSEWHEEL {
                    ScrollAxis::Vertical
                } else {
                    ScrollAxis::Horizontal
                },
                direction: if delta > 0.0 {
                    ScrollDirection::Positive
                } else {
                    ScrollDirection::Negative
                },
                amount: delta.abs() / WHEEL_DELTA,
            })
        }
        _ => None,
    }
}

/// `MOUSEINPUT::mouseData` for a scroll of `amount` detents in `direction`.
#[must_use]
pub fn wheel_data(direction: ScrollDirection, amount: f64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let delta = (amount.abs() * WHEEL_DELTA)
        .round()
        .min(f64::from(i32::MAX)) as i32;
    match direction {
        ScrollDirection::Positive => delta,
        ScrollDirection::Negative => -delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_key_round_trips_through_the_hook_path() {
        for &(code, scan) in SCAN_CODES {
            let back = key_from_hook(0, u32::from(scan & 0xFF), scan & EXT == EXT);
            assert_eq!(back, Some(code), "{code:?} (scan {scan:#06x})");
            assert_eq!(
                keystroke(code),
                KeyStroke::Scan {
                    code: scan & 0xFF,
                    extended: scan & EXT == EXT
                }
            );
        }
    }

    #[test]
    fn scan_codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &(code, scan) in SCAN_CODES {
            assert!(seen.insert(scan), "{code:?} reuses scan {scan:#06x}");
        }
    }

    #[test]
    fn well_known_keys_have_their_set1_codes() {
        assert_eq!(
            keystroke(KeyCode::KeyA),
            KeyStroke::Scan {
                code: 0x1E,
                extended: false
            }
        );
        assert_eq!(
            keystroke(KeyCode::Enter),
            KeyStroke::Scan {
                code: 0x1C,
                extended: false
            }
        );
        assert_eq!(
            keystroke(KeyCode::NumpadEnter),
            KeyStroke::Scan {
                code: 0x1C,
                extended: true
            }
        );
        assert_eq!(
            keystroke(KeyCode::RightCtrl),
            KeyStroke::Scan {
                code: 0x1D,
                extended: true
            }
        );
        assert_eq!(
            keystroke(KeyCode::ArrowLeft),
            KeyStroke::Scan {
                code: 0x4B,
                extended: true
            }
        );
        assert_eq!(
            keystroke(KeyCode::Numpad4),
            KeyStroke::Scan {
                code: 0x4B,
                extended: false
            }
        );
    }

    #[test]
    fn ambiguous_keys_use_virtual_keys() {
        assert_eq!(keystroke(KeyCode::Pause), KeyStroke::Virtual(0x13));
        assert_eq!(keystroke(KeyCode::NumLock), KeyStroke::Virtual(0x90));
        // Pause and Num Lock both arrive as scan 0x45; the VK tells them apart.
        assert_eq!(
            key_from_hook(VK_PAUSE.into(), 0x45, false),
            Some(KeyCode::Pause)
        );
        assert_eq!(
            key_from_hook(VK_NUMLOCK.into(), 0x45, true),
            Some(KeyCode::NumLock)
        );
        // Right Shift is sometimes flagged extended; the VK is reliable.
        assert_eq!(
            key_from_hook(VK_RSHIFT.into(), 0x36, true),
            Some(KeyCode::RightShift)
        );
    }

    #[test]
    fn altgr_fake_ctrl_is_dropped() {
        assert_eq!(key_from_hook(0xA2, 0x21D, false), None);
    }

    #[test]
    fn unknown_keys_round_trip_their_scan_code() {
        // The ISO "102nd" key has no name here.
        let code = key_from_hook(0xE2, 0x56, false).unwrap();
        assert_eq!(code, KeyCode::Unknown(0x56));
        assert_eq!(
            keystroke(code),
            KeyStroke::Scan {
                code: 0x56,
                extended: false
            }
        );
        let ext = key_from_hook(0, 0x6A, true).unwrap();
        assert_eq!(
            keystroke(ext),
            KeyStroke::Scan {
                code: 0x6A,
                extended: true
            }
        );
    }

    #[test]
    fn mouse_buttons_and_wheel() {
        assert_eq!(
            mouse_event_from_hook(WM_LBUTTONDOWN, 0),
            Some(InputEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Pressed
            })
        );
        assert_eq!(
            mouse_event_from_hook(WM_XBUTTONUP, 2 << 16),
            Some(InputEvent::MouseButton {
                button: MouseButton::Forward,
                state: ButtonState::Released
            })
        );
        // One notch down: -120 as a 16-bit value in the high word.
        let down = u32::from(0xFF88_u16) << 16;
        assert_eq!(
            mouse_event_from_hook(WM_MOUSEWHEEL, down),
            Some(InputEvent::Scroll {
                axis: ScrollAxis::Vertical,
                direction: ScrollDirection::Negative,
                amount: 1.0
            })
        );
        assert_eq!(
            mouse_event_from_hook(WM_MOUSEHWHEEL, 60 << 16),
            Some(InputEvent::Scroll {
                axis: ScrollAxis::Horizontal,
                direction: ScrollDirection::Positive,
                amount: 0.5
            })
        );
        assert_eq!(mouse_event_from_hook(WM_MOUSEMOVE, 0), None);
    }

    #[test]
    fn wheel_data_round_trips() {
        assert_eq!(wheel_data(ScrollDirection::Negative, 1.0), -120);
        assert_eq!(wheel_data(ScrollDirection::Positive, 0.5), 60);
        for (dir, amount) in [
            (ScrollDirection::Positive, 2.0),
            (ScrollDirection::Negative, 0.25),
        ] {
            let data = wheel_data(dir, amount);
            let high = u32::from(i16::try_from(data).unwrap().cast_unsigned()) << 16;
            match mouse_event_from_hook(WM_MOUSEWHEEL, high) {
                Some(InputEvent::Scroll {
                    direction,
                    amount: a,
                    ..
                }) => {
                    assert_eq!(direction, dir);
                    assert!((a - amount).abs() < 1e-9);
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
