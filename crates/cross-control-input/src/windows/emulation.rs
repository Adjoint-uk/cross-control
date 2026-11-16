//! Input emulation with `SendInput`.

use async_trait::async_trait;
use cross_control_types::{
    ButtonState, DeviceInfo, InputEvent, MouseButton, ScrollAxis, VirtualDeviceId,
};
use tracing::{debug, trace, warn};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{XBUTTON1, XBUTTON2};

use crate::win32_map::{keystroke, wheel_data, KeyStroke};
use crate::{InputEmulation, InputError};

/// Windows input emulation. Windows has no per-device virtual inputs, so
/// "devices" are only ids; all events go through `SendInput`.
#[derive(Debug, Default)]
pub struct WindowsEmulation {
    next_device: u32,
}

impl WindowsEmulation {
    #[must_use]
    pub fn new() -> Self {
        super::ensure_dpi_aware();
        Self::default()
    }
}

fn keyboard_input(event: &InputEvent) -> Option<INPUT> {
    let InputEvent::Key { code, state } = event else {
        return None;
    };
    let up = if *state == ButtonState::Released {
        KEYEVENTF_KEYUP
    } else {
        0
    };
    let ki = match keystroke(*code) {
        KeyStroke::Scan { code: 0, .. } => return None,
        KeyStroke::Scan { code, extended } => KEYBDINPUT {
            wVk: 0,
            wScan: code,
            dwFlags: KEYEVENTF_SCANCODE | up | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 },
            time: 0,
            dwExtraInfo: 0,
        },
        KeyStroke::Virtual(vk) => KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: up,
            time: 0,
            dwExtraInfo: 0,
        },
    };
    Some(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki },
    })
}

fn mouse(dx: i32, dy: i32, mouse_data: u32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(event: &InputEvent) -> Option<INPUT> {
    match event {
        InputEvent::MouseMove { dx, dy } => Some(mouse(*dx, *dy, 0, MOUSEEVENTF_MOVE)),
        InputEvent::MouseMoveAbsolute { x, y } => {
            // Normalised 0..1 over the whole virtual desktop, to 0..65535.
            #[allow(clippy::cast_possible_truncation)]
            let scale = |v: f64| (v.clamp(0.0, 1.0) * 65535.0).round() as i32;
            Some(mouse(
                scale(*x),
                scale(*y),
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            ))
        }
        InputEvent::MouseButton { button, state } => {
            let pressed = *state == ButtonState::Pressed;
            let (flags, data) = match button {
                MouseButton::Left => (
                    if pressed {
                        MOUSEEVENTF_LEFTDOWN
                    } else {
                        MOUSEEVENTF_LEFTUP
                    },
                    0,
                ),
                MouseButton::Right => (
                    if pressed {
                        MOUSEEVENTF_RIGHTDOWN
                    } else {
                        MOUSEEVENTF_RIGHTUP
                    },
                    0,
                ),
                MouseButton::Middle => (
                    if pressed {
                        MOUSEEVENTF_MIDDLEDOWN
                    } else {
                        MOUSEEVENTF_MIDDLEUP
                    },
                    0,
                ),
                MouseButton::Back | MouseButton::Forward => {
                    let which = if *button == MouseButton::Back {
                        XBUTTON1
                    } else {
                        XBUTTON2
                    };
                    (
                        if pressed {
                            MOUSEEVENTF_XDOWN
                        } else {
                            MOUSEEVENTF_XUP
                        },
                        u32::from(which),
                    )
                }
                MouseButton::Other(n) => {
                    debug!(
                        button = n,
                        "no Windows equivalent for mouse button; dropped"
                    );
                    return None;
                }
            };
            Some(mouse(0, 0, data, flags))
        }
        InputEvent::Scroll {
            axis,
            direction,
            amount,
        } => {
            let flags = match axis {
                ScrollAxis::Vertical => MOUSEEVENTF_WHEEL,
                ScrollAxis::Horizontal => MOUSEEVENTF_HWHEEL,
            };
            // mouseData carries the signed wheel delta in a DWORD.
            #[allow(clippy::cast_sign_loss)]
            let data = wheel_data(*direction, *amount) as u32;
            Some(mouse(0, 0, data, flags))
        }
        InputEvent::Key { .. } => None,
    }
}

#[async_trait]
impl InputEmulation for WindowsEmulation {
    async fn create_device(&mut self, info: &DeviceInfo) -> Result<VirtualDeviceId, InputError> {
        self.next_device = self.next_device.wrapping_add(1);
        debug!(device = %info.name, "remote device mapped to SendInput");
        Ok(VirtualDeviceId(self.next_device))
    }

    async fn inject(
        &mut self,
        device: VirtualDeviceId,
        event: InputEvent,
    ) -> Result<(), InputError> {
        let Some(input) = keyboard_input(&event).or_else(|| mouse_input(&event)) else {
            return Ok(());
        };
        // SAFETY: one fully initialised INPUT, with its correct size.
        let sent = unsafe {
            SendInput(
                1,
                &raw const input,
                i32::try_from(std::mem::size_of::<INPUT>()).unwrap_or(i32::MAX),
            )
        };
        if sent != 1 {
            // Typically: the foreground window belongs to an elevated
            // process (UIPI), or the secure desktop is active.
            let error = std::io::Error::last_os_error();
            warn!(%error, "SendInput was blocked");
            return Err(InputError::Inject(error.to_string()));
        }
        // Trace only: these are keystrokes.
        trace!(?event, device = device.0, "injected event");
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
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

    fn cursor() -> POINT {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: valid, writable POINT.
        unsafe { GetCursorPos(&raw mut p) };
        p
    }

    #[tokio::test]
    async fn relative_motion_moves_the_cursor() {
        // SAFETY: plain call.
        if unsafe { SetCursorPos(200, 200) } == 0 {
            eprintln!("skipping: no interactive desktop");
            return;
        }
        let mut emulation = WindowsEmulation::new();
        if let Err(e) = emulation
            .inject(VirtualDeviceId(1), InputEvent::MouseMove { dx: 40, dy: 0 })
            .await
        {
            eprintln!("skipping: SendInput unavailable here: {e}");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // Pointer acceleration may scale the distance, never the direction.
        assert!(
            cursor().x > 200,
            "cursor did not move right: {:?}",
            cursor().x
        );
    }

    #[test]
    fn events_become_the_right_inputs() {
        use cross_control_types::{KeyCode, ScrollDirection};

        let key = keyboard_input(&InputEvent::Key {
            code: KeyCode::ArrowLeft,
            state: ButtonState::Released,
        })
        .unwrap();
        assert_eq!(key.r#type, INPUT_KEYBOARD);
        // SAFETY: reading the variant that was written.
        let ki = unsafe { key.Anonymous.ki };
        assert_eq!(ki.wScan, 0x4B);
        assert_eq!(
            ki.dwFlags,
            KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
        );

        let wheel = mouse_input(&InputEvent::Scroll {
            axis: ScrollAxis::Vertical,
            direction: ScrollDirection::Negative,
            amount: 1.0,
        })
        .unwrap();
        // SAFETY: reading the variant that was written.
        let mi = unsafe { wheel.Anonymous.mi };
        assert_eq!(mi.dwFlags, MOUSEEVENTF_WHEEL);
        assert_eq!(mi.mouseData.cast_signed(), -120);

        let back = mouse_input(&InputEvent::MouseButton {
            button: MouseButton::Back,
            state: ButtonState::Pressed,
        })
        .unwrap();
        // SAFETY: reading the variant that was written.
        let mi = unsafe { back.Anonymous.mi };
        assert_eq!(
            (mi.dwFlags, mi.mouseData),
            (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1))
        );
    }
}
