//! Test-only: synthesize input that the capture hooks treat as if it came
//! from a physical device, so end-to-end tests can drive real capture on a
//! real Windows desktop (e.g. a CI runner). Only built with `test-support`.

use cross_control_types::InputEvent;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_KEYBOARD};

use super::emulation::{keyboard_input, mouse_input};
use crate::InputError;

/// `dwExtraInfo` tag that marks synthetic test input as "hardware".
pub const MARKER: usize = 0x4343_5445; // "CCTE"

/// Inject `event` tagged with [`MARKER`], so the hooks capture it.
pub fn inject_as_hardware(event: &InputEvent) -> Result<(), InputError> {
    let Some(mut input) = keyboard_input(event).or_else(|| mouse_input(event)) else {
        return Ok(());
    };
    // Writing a union field is safe; write the variant the builder
    // initialised (its type says which).
    if input.r#type == INPUT_KEYBOARD {
        input.Anonymous.ki.dwExtraInfo = MARKER;
    } else {
        input.Anonymous.mi.dwExtraInfo = MARKER;
    }
    // SAFETY: one fully initialised INPUT, with its correct size.
    let sent = unsafe {
        SendInput(
            1,
            &raw const input,
            i32::try_from(std::mem::size_of::<INPUT>()).unwrap_or(i32::MAX),
        )
    };
    if sent == 1 {
        Ok(())
    } else {
        Err(InputError::Inject(
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

/// The cursor position, in physical pixels.
#[must_use]
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: valid, writable POINT.
    (unsafe { GetCursorPos(&raw mut p) } != 0).then_some((p.x, p.y))
}

/// Move the cursor. False when there is no interactive desktop.
#[must_use]
pub fn set_cursor_pos(x: i32, y: i32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
    // SAFETY: plain call.
    unsafe { SetCursorPos(x, y) != 0 }
}

/// The primary screen's size, in physical pixels (after the backend made the
/// process DPI aware).
#[must_use]
pub fn primary_screen_size() -> (u32, u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    super::ensure_dpi_aware();
    // SAFETY: plain metric queries.
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0))
}

/// Whether the key with this set-1 scan code is currently down, system-wide
/// (as seen by any application).
#[must_use]
pub fn is_scan_code_down(scan: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MapVirtualKeyW, MAPVK_VSC_TO_VK_EX,
    };
    // SAFETY: plain calls.
    unsafe {
        let vk = MapVirtualKeyW(u32::from(scan), MAPVK_VSC_TO_VK_EX);
        vk != 0 && GetAsyncKeyState(i32::try_from(vk).unwrap_or(0)).cast_unsigned() & 0x8000 != 0
    }
}

/// Tests that need an interactive desktop skip without one, unless
/// `CROSS_CONTROL_REQUIRE_DESKTOP` is set (CI's Windows job sets it, so a
/// silent skip can never pass for a real result). Call with the reason the
/// desktop looks unavailable; returns normally only when skipping is allowed.
///
/// # Panics
///
/// When a desktop is required.
pub fn skip_without_desktop(reason: &str) {
    let required = std::env::var_os("CROSS_CONTROL_REQUIRE_DESKTOP").is_some_and(|v| !v.is_empty());
    assert!(
        !required,
        "interactive desktop required but unavailable: {reason}"
    );
    eprintln!("skipping: {reason}");
}
