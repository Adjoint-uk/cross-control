//! Test-only helpers for end-to-end tests on a real macOS desktop (e.g. a
//! CI runner): synthetic input the event tap captures as if it came from
//! hardware, and probes of the real cursor and key state. Only built with
//! `test-support`.

use core_graphics::display::CGDisplay;
use core_graphics::geometry::CGPoint;

pub use super::cursor_position;
use super::emulation::MacEmulation;

/// An emulator whose events this process's own tap captures (they carry no
/// "injected by us" marker), standing in for a physical keyboard and mouse.
#[must_use]
pub fn hardware() -> MacEmulation {
    MacEmulation::as_hardware()
}

/// Move the cursor. False when there is no usable display.
#[must_use]
pub fn set_cursor_position(x: f64, y: f64) -> bool {
    CGDisplay::warp_mouse_cursor_position(CGPoint::new(x, y)).is_ok()
}

/// The main display's size in points.
#[must_use]
pub fn main_display_size() -> (u32, u32) {
    let bounds = CGDisplay::main().bounds();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let size = (bounds.size.width as u32, bounds.size.height as u32);
    size
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

/// Whether the key with virtual key code `vk` is down, as the session (all
/// posted and hardware events) sees it.
#[must_use]
pub fn is_key_down(vk: u16) -> bool {
    /// `kCGEventSourceStateCombinedSessionState`.
    const COMBINED_SESSION_STATE: i32 = 0;
    // SAFETY: plain query with a valid state id.
    unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, vk) }
}

/// Whether the process may listen to and post input.
#[must_use]
pub fn has_permissions() -> bool {
    let p = super::permissions();
    p.listen && p.post
}

/// A desktop test cannot run here: panic if one is required
/// (`CROSS_CONTROL_REQUIRE_DESKTOP`), else print why it is skipped.
pub fn skip_without_desktop(reason: &str) {
    let required = std::env::var_os("CROSS_CONTROL_REQUIRE_DESKTOP").is_some_and(|v| !v.is_empty());
    assert!(!required, "desktop required but unavailable: {reason}");
    eprintln!("skipping: {reason}");
}
