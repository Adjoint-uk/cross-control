//! Native Windows input backend.
//!
//! - Capture: low-level keyboard and mouse hooks (`WH_KEYBOARD_LL`,
//!   `WH_MOUSE_LL`) on a dedicated thread with its own message loop. While
//!   grabbed, local events are swallowed (and the cursor is parked mid-screen
//!   so relative motion never stalls at an edge); they are still forwarded.
//! - Emulation: `SendInput`, with keys sent by scan code (layout-independent).
//!
//! Input this process injects is ignored by the hooks, so being controlled
//! never echoes events back to the controller.
//!
//! Known limits of a user-level process: it cannot inject into windows of
//! elevated (administrator) programs, or reach the secure desktop (UAC
//! prompts, Ctrl+Alt+Del, the lock screen).
//!
//! The Win32 API needs `unsafe`; it is confined to this module, with each
//! call's invariants noted where it is made.
#![allow(unsafe_code)]

pub mod capture;
pub mod emulation;

use std::sync::Once;

use cross_control_types::{DeviceCapability, DeviceId, DeviceInfo};

/// Low-level hooks do not say which physical device an event came from, so
/// the backend presents one keyboard and one mouse.
pub const KEYBOARD: DeviceId = DeviceId(1);
pub const MOUSE: DeviceId = DeviceId(2);

/// The devices this backend captures from.
#[must_use]
pub fn devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: KEYBOARD,
            name: "Windows keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        },
        DeviceInfo {
            id: MOUSE,
            name: "Windows mouse".to_string(),
            capabilities: vec![DeviceCapability::RelativeMouse, DeviceCapability::Scroll],
        },
    ]
}

/// Opt in to per-monitor DPI awareness, so hook coordinates and cursor
/// positions are physical pixels on every monitor. Process-wide; once.
fn ensure_dpi_aware() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use windows_sys::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // SAFETY: takes a predefined context constant; failure (e.g. already
        // set by a manifest) is harmless.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    });
}
