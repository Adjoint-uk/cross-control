//! Application state for the TUI test harness.

use std::collections::VecDeque;
use std::time::Duration;

use cross_control_daemon::DaemonStatus;
use cross_control_input::mock::MockEmulationHandle;
use tokio::sync::watch;

/// Maximum number of log lines to keep.
const MAX_LOG_LINES: usize = 100;

/// Per-screen state tracked by the TUI.
pub struct ScreenState {
    pub name: String,
    pub status: watch::Receiver<DaemonStatus>,
    pub emulation: MockEmulationHandle,
    pub last_injected_count: usize,
}

/// Application state shared between the event loop and rendering.
pub struct AppState {
    pub screens: Vec<ScreenState>,
    pub log_lines: VecDeque<String>,
    pub quit: bool,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl AppState {
    pub fn new(screens: Vec<ScreenState>, screen_width: u32, screen_height: u32) -> Self {
        Self {
            screens,
            log_lines: VecDeque::new(),
            quit: false,
            screen_width,
            screen_height,
        }
    }

    pub fn log(&mut self, msg: String) {
        self.log_lines.push_back(msg);
        if self.log_lines.len() > MAX_LOG_LINES {
            self.log_lines.pop_front();
        }
    }

    /// Poll for new injected events on all screens and log them.
    pub fn poll_injections(&mut self) {
        for screen in &mut self.screens {
            let events = screen.emulation.injected_events();
            for event in events.iter().skip(screen.last_injected_count) {
                self.log_lines
                    .push_back(format!("{}: Injected {:?}", screen.name, event.event));
                if self.log_lines.len() > MAX_LOG_LINES {
                    self.log_lines.pop_front();
                }
            }
            screen.last_injected_count = events.len();
        }
    }

    /// Get a status snapshot for a screen by index.
    pub fn status_snapshot(&self, idx: usize) -> DaemonStatus {
        self.screens[idx].status.borrow().clone()
    }

    /// Tick interval for the TUI refresh.
    pub fn tick_rate() -> Duration {
        Duration::from_millis(100)
    }
}
