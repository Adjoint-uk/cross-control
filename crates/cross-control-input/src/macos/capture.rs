//! Input capture with a Quartz event tap.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};
use core_graphics::geometry::CGPoint;
use cross_control_types::{Barrier, BarrierId, ButtonState, CapturedEvent, DeviceId, InputEvent};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::{desktop_bounds, KEYBOARD, MOUSE};
use crate::mac_map::{
    button_event, from_fixed, is_motion, key_from_keycode, modifier_state, normalize_position,
    scroll_from_deltas, FIELD_KEYBOARD_KEYCODE, FIELD_MOUSE_BUTTON_NUMBER, FIELD_MOUSE_DELTA_X,
    FIELD_MOUSE_DELTA_Y, FIELD_SCROLL_FIXED_DELTA_AXIS_1, FIELD_SCROLL_FIXED_DELTA_AXIS_2,
    FIELD_SOURCE_USER_DATA, FLAGS_CHANGED, INJECTED_MARKER, KEY_DOWN, KEY_UP, SCROLL_WHEEL,
    TAP_DISABLED_BY_TIMEOUT, TAP_DISABLED_BY_USER_INPUT,
};
use crate::{InputCapture, InputError};

/// State shared between the async side and the tap thread.
struct Shared {
    grabbed: AtomicBool,
}

/// Tap-thread state. The callback runs on the thread whose run loop the
/// tap is attached to, so this lives in a thread-local.
struct TapCtx {
    shared: Arc<Shared>,
    tx: mpsc::Sender<CapturedEvent>,
    started: Instant,
    last: Option<(f64, f64)>,
    /// The tap itself, so the callback can re-enable it when macOS
    /// disables it (after a slow callback or secure input).
    tap: Option<CGEventTap<'static>>,
}

thread_local! {
    static CTX: RefCell<Option<TapCtx>> = const { RefCell::new(None) };
}

impl TapCtx {
    fn emit(&self, device_id: DeviceId, event: InputEvent) {
        let timestamp_us = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        // Never block in the tap: macOS disables taps that are slow. A full
        // channel means the daemon is behind; dropping is the only option.
        let _ = self.tx.try_send(CapturedEvent {
            device_id,
            timestamp_us,
            event,
        });
    }
}

/// macOS input capture.
pub struct MacCapture {
    shared: Arc<Shared>,
    thread: Option<(std::thread::JoinHandle<()>, CFRunLoop)>,
    /// Cursor position before the current grab, restored on release.
    pre_grab: Option<(f64, f64)>,
    next_barrier: u32,
}

impl Default for MacCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl MacCapture {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                grabbed: AtomicBool::new(false),
            }),
            thread: None,
            pre_grab: None,
            next_barrier: 0,
        }
    }

    fn stop_thread(&mut self) -> Option<std::thread::JoinHandle<()>> {
        let (handle, run_loop) = self.thread.take()?;
        run_loop.stop();
        Some(handle)
    }
}

impl Drop for MacCapture {
    fn drop(&mut self) {
        if self.shared.grabbed.swap(false, Ordering::SeqCst) {
            let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
        }
        let _ = self.stop_thread();
    }
}

#[async_trait]
impl InputCapture for MacCapture {
    async fn start(&mut self, tx: mpsc::Sender<CapturedEvent>) -> Result<(), InputError> {
        if self.thread.is_some() {
            return Err(InputError::Other(anyhow::anyhow!(
                "capture already started"
            )));
        }
        let shared = Arc::clone(&self.shared);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<CFRunLoop, String>>();
        let handle = std::thread::Builder::new()
            .name("cross-control-tap".to_string())
            .spawn(move || tap_thread(shared, tx, &ready_tx))
            .map_err(|e| InputError::Other(e.into()))?;
        match ready_rx.recv() {
            Ok(Ok(run_loop)) => {
                self.thread = Some((handle, run_loop));
                info!("input capture started (Quartz event tap)");
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(InputError::Other(anyhow::anyhow!(e)))
            }
            Err(_) => Err(InputError::Other(anyhow::anyhow!(
                "event tap thread exited during start-up"
            ))),
        }
    }

    async fn add_barrier(&mut self, _barrier: Barrier) -> Result<BarrierId, InputError> {
        self.next_barrier = self.next_barrier.wrapping_add(1);
        Ok(BarrierId(self.next_barrier))
    }

    async fn remove_barrier(&mut self, _id: BarrierId) -> Result<(), InputError> {
        Ok(())
    }

    async fn grab(&mut self) -> Result<(), InputError> {
        if self.shared.grabbed.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.pre_grab = super::cursor_position();
        // Park mid-screen and detach the cursor from the mouse: motion is
        // still reported as deltas, but the cursor stays put and never pins
        // against an edge.
        let main = CGDisplay::main().bounds();
        let park = CGPoint::new(
            main.origin.x + main.size.width / 2.0,
            main.origin.y + main.size.height / 2.0,
        );
        let _ = CGDisplay::warp_mouse_cursor_position(park);
        if CGDisplay::associate_mouse_and_mouse_cursor_position(false).is_err() {
            warn!("could not detach the cursor from the mouse");
        }
        self.shared.grabbed.store(true, Ordering::SeqCst);
        debug!("grabbed local input");
        Ok(())
    }

    async fn release(&mut self) -> Result<(), InputError> {
        if !self.shared.grabbed.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
        if let Some((x, y)) = self.pre_grab.take() {
            let _ = CGDisplay::warp_mouse_cursor_position(CGPoint::new(x, y));
        }
        debug!("released local input");
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), InputError> {
        self.release().await?;
        if let Some(handle) = self.stop_thread() {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
        info!("input capture shut down");
        Ok(())
    }
}

/// The event types the tap listens to.
fn events_of_interest() -> Vec<CGEventType> {
    vec![
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::RightMouseDragged,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::OtherMouseDragged,
        CGEventType::ScrollWheel,
    ]
}

/// Where to put the tap, in order of preference. A HID-level tap sees input
/// before any application; a session tap is the fallback where the HID
/// level is not allowed. Test builds tap at the session level, which also
/// sees events posted at the HID level, as the end-to-end tests' synthetic
/// "hardware" input is.
fn tap_locations() -> [CGEventTapLocation; 2] {
    if cfg!(feature = "test-support") {
        [CGEventTapLocation::Session, CGEventTapLocation::HID]
    } else {
        [CGEventTapLocation::HID, CGEventTapLocation::Session]
    }
}

/// Create the tap, report readiness, and run the run loop until stopped.
fn tap_thread(
    shared: Arc<Shared>,
    tx: mpsc::Sender<CapturedEvent>,
    ready: &std::sync::mpsc::Sender<Result<CFRunLoop, String>>,
) {
    CTX.with(|ctx| {
        *ctx.borrow_mut() = Some(TapCtx {
            shared,
            tx,
            started: Instant::now(),
            last: None,
            tap: None,
        });
    });

    let tap = tap_locations().into_iter().find_map(|location| {
        CGEventTap::new(
            location,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            events_of_interest(),
            callback,
        )
        .ok()
    });
    let Some(tap) = tap else {
        let have = super::request_permissions();
        let _ = ready.send(Err(format!(
            "cannot create the event tap: grant cross-control Input Monitoring{} and \
             Accessibility{} in System Settings → Privacy & Security, then restart it",
            if have.listen {
                " (granted)"
            } else {
                " (missing)"
            },
            if have.post {
                " (granted)"
            } else {
                " (missing)"
            },
        )));
        CTX.with(|ctx| ctx.borrow_mut().take());
        return;
    };
    let Ok(source) = tap.mach_port().create_runloop_source(0) else {
        let _ = ready.send(Err("cannot attach the event tap to a run loop".to_string()));
        CTX.with(|ctx| ctx.borrow_mut().take());
        return;
    };
    let run_loop = CFRunLoop::get_current();
    // SAFETY: reading an immutable framework constant.
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    CTX.with(|ctx| {
        if let Some(ctx) = ctx.borrow_mut().as_mut() {
            ctx.tap = Some(tap);
        }
    });
    let _ = ready.send(Ok(run_loop));

    CFRunLoop::run_current();

    // Dropping the context drops (and invalidates) the tap.
    CTX.with(|ctx| ctx.borrow_mut().take());
    debug!("event tap thread stopped");
}

/// The tap callback: translate the event, forward it, and swallow it while
/// grabbed.
fn callback(
    _proxy: *const std::ffi::c_void,
    event_type: CGEventType,
    event: &CGEvent,
) -> CallbackResult {
    let event_type = event_type as u32;
    if event_type == TAP_DISABLED_BY_TIMEOUT || event_type == TAP_DISABLED_BY_USER_INPUT {
        CTX.with(|ctx| {
            if let Some(tap) = ctx.borrow().as_ref().and_then(|c| c.tap.as_ref()) {
                tap.enable();
            }
        });
        debug!("event tap re-enabled after macOS disabled it");
        return CallbackResult::Keep;
    }
    if event.get_integer_value_field(FIELD_SOURCE_USER_DATA) == INJECTED_MARKER {
        return CallbackResult::Keep; // posted by this process
    }
    CTX.with(|ctx| {
        let mut ctx = ctx.borrow_mut();
        let Some(ctx) = ctx.as_mut() else {
            return CallbackResult::Keep;
        };
        let grabbed = ctx.shared.grabbed.load(Ordering::SeqCst);
        translate(ctx, event_type, event, grabbed);
        if grabbed {
            CallbackResult::Drop
        } else {
            CallbackResult::Keep
        }
    })
}

fn translate(ctx: &mut TapCtx, event_type: u32, event: &CGEvent, grabbed: bool) {
    match event_type {
        KEY_DOWN | KEY_UP => {
            let vk =
                u16::try_from(event.get_integer_value_field(FIELD_KEYBOARD_KEYCODE)).unwrap_or(0);
            let state = if event_type == KEY_DOWN {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            ctx.emit(
                KEYBOARD,
                InputEvent::Key {
                    code: key_from_keycode(vk),
                    state,
                },
            );
        }
        FLAGS_CHANGED => {
            let vk =
                u16::try_from(event.get_integer_value_field(FIELD_KEYBOARD_KEYCODE)).unwrap_or(0);
            let code = key_from_keycode(vk);
            match modifier_state(code, event.get_flags().bits()) {
                Some(ButtonState::Pressed) if code == cross_control_types::KeyCode::CapsLock => {
                    // Caps Lock reports each toggle once: press and release.
                    ctx.emit(
                        KEYBOARD,
                        InputEvent::Key {
                            code,
                            state: ButtonState::Pressed,
                        },
                    );
                    ctx.emit(
                        KEYBOARD,
                        InputEvent::Key {
                            code,
                            state: ButtonState::Released,
                        },
                    );
                }
                Some(state) => ctx.emit(KEYBOARD, InputEvent::Key { code, state }),
                None => {}
            }
        }
        SCROLL_WHEEL => {
            let vertical =
                from_fixed(event.get_integer_value_field(FIELD_SCROLL_FIXED_DELTA_AXIS_1));
            let horizontal =
                from_fixed(event.get_integer_value_field(FIELD_SCROLL_FIXED_DELTA_AXIS_2));
            for (axis, direction, amount) in scroll_from_deltas(vertical, horizontal) {
                ctx.emit(
                    MOUSE,
                    InputEvent::Scroll {
                        axis,
                        direction,
                        amount,
                    },
                );
            }
        }
        t if is_motion(t) => {
            if let Some(event) = motion(ctx, event, grabbed) {
                ctx.emit(MOUSE, event);
            }
        }
        t => {
            let number = u32::try_from(event.get_integer_value_field(FIELD_MOUSE_BUTTON_NUMBER))
                .unwrap_or(0);
            if let Some((button, state)) = button_event(t, number) {
                ctx.emit(MOUSE, InputEvent::MouseButton { button, state });
            }
        }
    }
}

/// Grabbed: the mouse's own deltas (the cursor is detached and parked).
/// Otherwise: the cursor's position on the desktop, so the daemon's idea
/// of where it is stays exact (macOS clamps it at screen edges).
fn motion(ctx: &mut TapCtx, event: &CGEvent, grabbed: bool) -> Option<InputEvent> {
    if grabbed {
        ctx.last = None;
        let dx = i32::try_from(event.get_integer_value_field(FIELD_MOUSE_DELTA_X)).ok()?;
        let dy = i32::try_from(event.get_integer_value_field(FIELD_MOUSE_DELTA_Y)).ok()?;
        return (dx != 0 || dy != 0).then_some(InputEvent::MouseMove { dx, dy });
    }
    let p = event.location();
    if ctx.last == Some((p.x, p.y)) {
        return None;
    }
    ctx.last = Some((p.x, p.y));
    let (left, top, width, height) = desktop_bounds();
    let (x, y) = normalize_position(p.x, p.y, left, top, width, height);
    Some(InputEvent::MouseMoveAbsolute { x, y })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macos::emulation::MacEmulation;
    use crate::macos::{desktop_unavailable, permissions, DESKTOP_LOCK};
    use crate::InputEmulation;
    use cross_control_types::{KeyCode, VirtualDeviceId};

    async fn started() -> Option<(MacCapture, mpsc::Receiver<CapturedEvent>)> {
        let p = permissions();
        if !p.listen || !p.post {
            desktop_unavailable(&format!("missing macOS permissions: {p:?}"));
            return None;
        }
        let (tx, rx) = mpsc::channel(256);
        let mut capture = MacCapture::new();
        match capture.start(tx).await {
            Ok(()) => Some((capture, rx)),
            Err(e) => {
                desktop_unavailable(&format!("cannot start the event tap: {e}"));
                None
            }
        }
    }

    #[tokio::test]
    async fn tap_installs_and_shuts_down_cleanly() {
        let _desktop = DESKTOP_LOCK.lock().await;
        let Some((mut capture, _rx)) = started().await else {
            return;
        };
        capture.grab().await.unwrap();
        capture.release().await.unwrap();
        capture.shutdown().await.unwrap();
    }

    /// Being controlled must not echo: events we post are not captured.
    #[tokio::test]
    async fn injected_input_is_not_captured() {
        let _desktop = DESKTOP_LOCK.lock().await;
        let Some((mut capture, mut rx)) = started().await else {
            return;
        };
        let mut emulation = MacEmulation::new();
        let device = VirtualDeviceId(1);
        // F18: harmless if it does reach an application.
        for state in [ButtonState::Pressed, ButtonState::Released] {
            emulation
                .inject(
                    device,
                    InputEvent::Key {
                        code: KeyCode::Unknown(0x4F),
                        state,
                    },
                )
                .await
                .unwrap();
        }
        emulation
            .inject(device, InputEvent::MouseMove { dx: 3, dy: 0 })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        while let Ok(captured) = rx.try_recv() {
            assert!(
                !matches!(
                    captured.event,
                    InputEvent::Key {
                        code: KeyCode::Unknown(0x4F),
                        ..
                    }
                ),
                "captured our own injected key"
            );
        }
        capture.shutdown().await.unwrap();
    }
}
