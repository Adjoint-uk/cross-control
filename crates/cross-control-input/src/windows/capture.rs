//! Input capture with low-level hooks.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use cross_control_types::{Barrier, BarrierId, ButtonState, CapturedEvent, DeviceId, InputEvent};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetMessageW, GetSystemMetrics, PostThreadMessageW, SetCursorPos,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED,
    LLKHF_INJECTED, LLKHF_UP, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, SM_CXSCREEN, SM_CXVIRTUALSCREEN,
    SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_QUIT,
};

use super::{KEYBOARD, MOUSE};
use crate::win32_map::{key_from_hook, mouse_event_from_hook, normalize_position, WM_MOUSEMOVE};
use crate::{InputCapture, InputError};

/// State shared between the async side and the hook thread.
struct Shared {
    grabbed: AtomicBool,
    /// Where the cursor is parked while grabbed.
    park_x: AtomicI32,
    park_y: AtomicI32,
    /// Set when the hook thread must forget the last cursor position (after
    /// a grab or release moved the cursor).
    resync: AtomicBool,
}

/// Hook-thread state. Hook procedures run on the thread that installed them,
/// so this lives in a thread-local.
struct HookCtx {
    shared: Arc<Shared>,
    tx: mpsc::Sender<CapturedEvent>,
    started: Instant,
    last: Option<POINT>,
}

thread_local! {
    static CTX: RefCell<Option<HookCtx>> = const { RefCell::new(None) };
}

impl HookCtx {
    fn emit(&self, device_id: DeviceId, event: InputEvent) {
        let timestamp_us = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        // Never block in a hook: Windows removes hooks that are slow. A full
        // channel means the daemon is behind; dropping is the only option.
        let _ = self.tx.try_send(CapturedEvent {
            device_id,
            timestamp_us,
            event,
        });
    }
}

/// Windows input capture.
pub struct WindowsCapture {
    shared: Arc<Shared>,
    thread: Option<(std::thread::JoinHandle<()>, u32)>,
    /// Cursor position before the current grab, restored on release.
    pre_grab: Option<POINT>,
    next_barrier: u32,
}

impl Default for WindowsCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsCapture {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                grabbed: AtomicBool::new(false),
                park_x: AtomicI32::new(0),
                park_y: AtomicI32::new(0),
                resync: AtomicBool::new(true),
            }),
            thread: None,
            pre_grab: None,
            next_barrier: 0,
        }
    }

    fn stop_thread(&mut self) -> Option<std::thread::JoinHandle<()>> {
        let (handle, thread_id) = self.thread.take()?;
        // SAFETY: posting WM_QUIT to a thread id we own; harmless if the
        // thread already exited.
        unsafe {
            PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
        }
        Some(handle)
    }
}

impl Drop for WindowsCapture {
    fn drop(&mut self) {
        // Do not join here (we may be on an async runtime thread); the hook
        // thread exits on WM_QUIT.
        let _ = self.stop_thread();
    }
}

fn cursor_pos() -> Option<POINT> {
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: `p` is a valid, writable POINT.
    (unsafe { GetCursorPos(&raw mut p) } != 0).then_some(p)
}

#[async_trait]
impl InputCapture for WindowsCapture {
    async fn start(&mut self, tx: mpsc::Sender<CapturedEvent>) -> Result<(), InputError> {
        if self.thread.is_some() {
            return Err(InputError::Other(anyhow::anyhow!(
                "capture already started"
            )));
        }
        super::ensure_dpi_aware();
        let shared = Arc::clone(&self.shared);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        let handle = std::thread::Builder::new()
            .name("cross-control-hooks".to_string())
            .spawn(move || hook_thread(shared, tx, &ready_tx))
            .map_err(|e| InputError::Other(e.into()))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => {
                self.thread = Some((handle, thread_id));
                info!("input capture started (low-level hooks)");
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(InputError::Other(anyhow::anyhow!(
                    "cannot install input hooks: {e}"
                )))
            }
            Err(_) => Err(InputError::Other(anyhow::anyhow!(
                "input hook thread exited during start-up"
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
        self.pre_grab = cursor_pos();
        // Park mid-screen so motion in every direction is reported; at an
        // edge, motion outward would be clamped away.
        // SAFETY: plain metric queries.
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        let (x, y) = (w / 2, h / 2);
        self.shared.park_x.store(x, Ordering::SeqCst);
        self.shared.park_y.store(y, Ordering::SeqCst);
        self.shared.grabbed.store(true, Ordering::SeqCst);
        // SAFETY: plain call with in-range coordinates.
        unsafe {
            SetCursorPos(x, y);
        }
        debug!("grabbed local input");
        Ok(())
    }

    async fn release(&mut self) -> Result<(), InputError> {
        if !self.shared.grabbed.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(p) = self.pre_grab.take() {
            // SAFETY: plain call; restores where the cursor left the screen.
            unsafe {
                SetCursorPos(p.x, p.y);
            }
        }
        self.shared.resync.store(true, Ordering::SeqCst);
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

/// Install the hooks, report readiness, and pump messages until `WM_QUIT`.
fn hook_thread(
    shared: Arc<Shared>,
    tx: mpsc::Sender<CapturedEvent>,
    ready: &std::sync::mpsc::Sender<Result<u32, String>>,
) {
    CTX.with(|ctx| {
        *ctx.borrow_mut() = Some(HookCtx {
            shared,
            tx,
            started: Instant::now(),
            last: None,
        });
    });

    // SAFETY: standard low-level hook installation. The procedures are
    // `extern "system"` functions with the HOOKPROC signature, valid for the
    // life of the process; the module handle of the current executable is
    // what WH_*_LL hooks expect.
    let (keyboard, mouse) = unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        (
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0),
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0),
        )
    };
    if keyboard.is_null() || mouse.is_null() {
        let error = std::io::Error::last_os_error().to_string();
        // SAFETY: only unhooks handles that were actually returned.
        unsafe {
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
        }
        let _ = ready.send(Err(error));
        return;
    }
    // SAFETY: plain call.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));

    // SAFETY: MSG is plain data; GetMessageW fills it. The loop ends on
    // WM_QUIT (0) or error (-1).
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
        UnhookWindowsHookEx(keyboard);
        UnhookWindowsHookEx(mouse);
    }
    CTX.with(|ctx| ctx.borrow_mut().take());
    debug!("input hook thread stopped");
}

/// Whether a hooked event should be captured. Injected input (including
/// our own emulation) is ignored so being controlled never echoes back. With
/// the `test-support` feature, injected input carrying the test marker is
/// accepted, so tests can drive real capture.
fn accepts(injected: bool, extra_info: usize) -> bool {
    if !injected {
        return true;
    }
    #[cfg(feature = "test-support")]
    {
        extra_info == super::test_support::MARKER
    }
    #[cfg(not(feature = "test-support"))]
    {
        let _ = extra_info;
        false
    }
}

/// Low-level keyboard hook. Returns non-zero to swallow the event.
unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION.cast_signed() {
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT that is
        // valid for the duration of the call.
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if accepts(kb.flags & LLKHF_INJECTED != 0, kb.dwExtraInfo) {
            let swallow = CTX.with(|ctx| {
                let ctx = ctx.borrow();
                let Some(ctx) = ctx.as_ref() else {
                    return false;
                };
                if let Some(key) =
                    key_from_hook(kb.vkCode, kb.scanCode, kb.flags & LLKHF_EXTENDED != 0)
                {
                    let state = if kb.flags & LLKHF_UP != 0 {
                        ButtonState::Released
                    } else {
                        ButtonState::Pressed
                    };
                    ctx.emit(KEYBOARD, InputEvent::Key { code: key, state });
                }
                ctx.shared.grabbed.load(Ordering::Relaxed)
            });
            if swallow {
                return 1;
            }
        }
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Low-level mouse hook. Returns non-zero to swallow the event.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION.cast_signed() {
        // SAFETY: for HC_ACTION, lparam points to an MSLLHOOKSTRUCT that is
        // valid for the duration of the call.
        let ms = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if accepts(ms.flags & LLMHF_INJECTED != 0, ms.dwExtraInfo) {
            let message = u32::try_from(wparam).unwrap_or(0);
            let swallow = CTX.with(|ctx| {
                let mut ctx = ctx.borrow_mut();
                let Some(ctx) = ctx.as_mut() else {
                    return false;
                };
                let grabbed = ctx.shared.grabbed.load(Ordering::Relaxed);
                if message == WM_MOUSEMOVE {
                    if let Some(event) = motion(ctx, ms.pt, grabbed) {
                        ctx.emit(MOUSE, event);
                    }
                } else if let Some(event) = mouse_event_from_hook(message, ms.mouseData) {
                    ctx.emit(MOUSE, event);
                }
                grabbed
            });
            if swallow {
                return 1;
            }
        }
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// The event for a reported cursor position, or `None` if there is nothing
/// to report.
///
/// - Grabbed: relative motion from the park point (the swallowed event left
///   the cursor there), forwarded to the controlled machine.
/// - Not grabbed: the absolute position, as a fraction of the virtual
///   desktop. Windows clamps the cursor at the screen edges, so relative
///   deltas computed from positions would be zero while pushing against an
///   edge and the daemon's cursor could never catch up; the absolute position
///   keeps it exactly where the real cursor is.
fn motion(ctx: &mut HookCtx, pt: POINT, grabbed: bool) -> Option<InputEvent> {
    if ctx.shared.resync.swap(false, Ordering::Relaxed) {
        ctx.last = None;
    }
    if grabbed {
        let park = POINT {
            x: ctx.shared.park_x.load(Ordering::Relaxed),
            y: ctx.shared.park_y.load(Ordering::Relaxed),
        };
        let (dx, dy) = (pt.x - park.x, pt.y - park.y);
        if (dx, dy) == (0, 0) {
            return None;
        }
        if dx.abs() > 10_000 || dy.abs() > 10_000 {
            warn!(dx, dy, "ignoring implausible mouse jump");
            return None;
        }
        return Some(InputEvent::MouseMove { dx, dy });
    }
    if ctx
        .last
        .replace(pt)
        .is_some_and(|prev| prev.x == pt.x && prev.y == pt.y)
    {
        return None;
    }
    // SAFETY: plain metric queries (cheap; the desktop can change at runtime).
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let (x, y) = normalize_position(pt.x, pt.y, left, top, width, height);
    Some(InputEvent::MouseMoveAbsolute { x, y })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::emulation::WindowsEmulation;
    use crate::InputEmulation;
    use cross_control_types::{KeyCode, VirtualDeviceId};

    /// Skip without a desktop unless `CROSS_CONTROL_REQUIRE_DESKTOP` is set.
    fn desktop_unavailable(reason: &str) {
        let required =
            std::env::var_os("CROSS_CONTROL_REQUIRE_DESKTOP").is_some_and(|v| !v.is_empty());
        assert!(
            !required,
            "interactive desktop required but unavailable: {reason}"
        );
        eprintln!("skipping: {reason}");
    }

    /// Hooks need an interactive desktop; skip (rather than fail) on a
    /// runner without one.
    async fn started() -> Option<(WindowsCapture, mpsc::Receiver<CapturedEvent>)> {
        let mut capture = WindowsCapture::new();
        let (tx, rx) = mpsc::channel(64);
        match capture.start(tx).await {
            Ok(()) => Some((capture, rx)),
            Err(e) => {
                desktop_unavailable(&format!("cannot install hooks here: {e}"));
                None
            }
        }
    }

    #[tokio::test]
    async fn hooks_install_and_shut_down_cleanly() {
        let _desktop = crate::windows::DESKTOP_LOCK.lock().await;
        let Some((mut capture, _rx)) = started().await else {
            return;
        };
        capture.grab().await.unwrap();
        capture.release().await.unwrap();
        capture.shutdown().await.unwrap();
    }

    /// Being controlled must not echo: events we inject are not captured.
    #[tokio::test]
    async fn injected_input_is_not_captured() {
        let _desktop = crate::windows::DESKTOP_LOCK.lock().await;
        let Some((mut capture, mut rx)) = started().await else {
            return;
        };
        let mut emulation = WindowsEmulation::new();
        let device = VirtualDeviceId(1);
        for event in [
            InputEvent::Key {
                code: KeyCode::Unknown(0x76), // F24: harmless
                state: ButtonState::Pressed,
            },
            InputEvent::Key {
                code: KeyCode::Unknown(0x76), // F24: harmless
                state: ButtonState::Released,
            },
            InputEvent::MouseMove { dx: 3, dy: 0 },
            InputEvent::MouseMove { dx: -3, dy: 0 },
        ] {
            if let Err(e) = emulation.inject(device, event).await {
                desktop_unavailable(&format!("SendInput unavailable here: {e}"));
                capture.shutdown().await.unwrap();
                return;
            }
        }
        let got = tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await;
        assert!(got.is_err(), "injected input was captured: {got:?}");
        capture.shutdown().await.unwrap();
    }
}
