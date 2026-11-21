//! Native macOS backend (preview).
//!
//! - Capture: a Quartz event tap on a dedicated thread with its own run
//!   loop. While grabbed, local events are swallowed and the cursor is
//!   detached from the mouse (`CGAssociateMouseAndMouseCursorPosition`), so
//!   motion keeps arriving as deltas instead of stopping at a screen edge.
//! - Emulation: `CGEventPost`, with keys sent by virtual key code (the
//!   physical key position, independent of the keyboard layout) and the
//!   modifier flags of held keys applied to every event.
//!
//! Events this process posts carry a marker in `kCGEventSourceUserData`;
//! the tap ignores them, so being controlled never echoes back.
//!
//! macOS gates both halves behind privacy permissions, granted per program
//! in System Settings → Privacy & Security:
//! - **Input Monitoring** to capture (listen to) input;
//! - **Accessibility** to post input, and to swallow events while grabbed.
//!
//! Neither reaches the login window or secure text fields (password
//! prompts), which macOS protects from all event taps.
//!
//! The few direct framework calls need `unsafe`; each states its
//! invariants where it is made.
#![allow(unsafe_code)]

pub mod capture;
pub mod emulation;

use core_graphics::display::CGDisplay;
use cross_control_types::{DeviceCapability, DeviceId, DeviceInfo};

/// Event taps do not say which physical device an event came from, so the
/// backend presents one keyboard and one mouse.
pub const KEYBOARD: DeviceId = DeviceId(1);
pub const MOUSE: DeviceId = DeviceId(2);

/// The devices this backend captures from.
#[must_use]
pub fn devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: KEYBOARD,
            name: "macOS keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        },
        DeviceInfo {
            id: MOUSE,
            name: "macOS mouse".to_string(),
            capabilities: vec![DeviceCapability::RelativeMouse, DeviceCapability::Scroll],
        },
    ]
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

/// Which of the two privacy permissions this process holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    /// Input Monitoring: may listen to input (capture).
    pub listen: bool,
    /// Accessibility: may post input (emulation) and filter it (grab).
    pub post: bool,
}

/// Check the permissions without prompting.
#[must_use]
pub fn permissions() -> Permissions {
    // SAFETY: argument-less queries, available since macOS 10.15.
    unsafe {
        Permissions {
            listen: CGPreflightListenEventAccess(),
            post: CGPreflightPostEventAccess(),
        }
    }
}

/// Ask macOS to show its permission prompts for whatever is missing (the
/// first time only; afterwards the user must change it in System Settings).
pub fn request_permissions() -> Permissions {
    let have = permissions();
    // SAFETY: argument-less calls; they only show a system prompt.
    unsafe {
        if !have.listen {
            CGRequestListenEventAccess();
        }
        if !have.post {
            CGRequestPostEventAccess();
        }
    }
    permissions()
}

/// The bounding box of all active displays, in global coordinates (origin
/// at the top-left of the main display, y down): (left, top, width, height).
#[must_use]
pub fn desktop_bounds() -> (f64, f64, f64, f64) {
    let ids = CGDisplay::active_displays().unwrap_or_default();
    let rects: Vec<_> = if ids.is_empty() {
        vec![CGDisplay::main().bounds()]
    } else {
        ids.into_iter()
            .map(|id| CGDisplay::new(id).bounds())
            .collect()
    };
    let left = rects
        .iter()
        .map(|r| r.origin.x)
        .fold(f64::INFINITY, f64::min);
    let top = rects
        .iter()
        .map(|r| r.origin.y)
        .fold(f64::INFINITY, f64::min);
    let right = rects
        .iter()
        .map(|r| r.origin.x + r.size.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = rects
        .iter()
        .map(|r| r.origin.y + r.size.height)
        .fold(f64::NEG_INFINITY, f64::max);
    (left, top, right - left, bottom - top)
}

/// The current cursor position in global coordinates.
#[must_use]
pub fn cursor_position() -> Option<(f64, f64)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()?;
    let p = CGEvent::new(source).ok()?.location();
    Some((p.x, p.y))
}

/// Whether tests that need the desktop must run rather than skip (set on CI
/// runners known to grant the permissions).
#[cfg(test)]
pub(crate) fn desktop_unavailable(reason: &str) {
    let required = std::env::var_os("CROSS_CONTROL_REQUIRE_DESKTOP").is_some_and(|v| !v.is_empty());
    assert!(!required, "desktop required but unavailable: {reason}");
    eprintln!("skipping: {reason}");
}

/// Tests that move the real cursor or install taps share one desktop.
#[cfg(test)]
pub(crate) static DESKTOP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
