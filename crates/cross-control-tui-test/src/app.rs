//! Application state for the TUI test harness.

use std::collections::VecDeque;
use std::time::Duration;

use cross_control_daemon::DaemonStatus;
use cross_control_input::mock::MockEmulationHandle;
use tokio::sync::watch;

/// Maximum number of log lines to keep.
const MAX_LOG_LINES: usize = 100;

/// Application state shared between the event loop and rendering.
pub struct AppState {
    pub status_a: watch::Receiver<DaemonStatus>,
    pub status_b: watch::Receiver<DaemonStatus>,
    #[allow(dead_code)]
    pub emulation_a: MockEmulationHandle,
    pub emulation_b: MockEmulationHandle,
    pub log_lines: VecDeque<String>,
    pub last_injected_count_b: usize,
    pub quit: bool,
    /// Simulated cursor position for display (daemon A's cursor)
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl AppState {
    pub fn new(
        status_a: watch::Receiver<DaemonStatus>,
        status_b: watch::Receiver<DaemonStatus>,
        emulation_a: MockEmulationHandle,
        emulation_b: MockEmulationHandle,
        screen_width: u32,
        screen_height: u32,
    ) -> Self {
        Self {
            status_a,
            status_b,
            emulation_a,
            emulation_b,
            log_lines: VecDeque::new(),
            last_injected_count_b: 0,
            quit: false,
            cursor_x: i32::try_from(screen_width / 2).unwrap_or(960),
            cursor_y: i32::try_from(screen_height / 2).unwrap_or(540),
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

    /// Poll for new injected events on B's emulation and log them.
    pub fn poll_injections(&mut self) {
        let events = self.emulation_b.injected_events();
        for event in events.iter().skip(self.last_injected_count_b) {
            self.log(format!("B: Injected {:?}", event.event));
        }
        self.last_injected_count_b = events.len();
    }

    /// Update cursor from daemon A's status.
    pub fn sync_cursor(&mut self) {
        let status = self.status_a.borrow().clone();
        self.cursor_x = status.cursor_x;
        self.cursor_y = status.cursor_y;
    }

    pub fn status_a_snapshot(&self) -> DaemonStatus {
        self.status_a.borrow().clone()
    }

    pub fn status_b_snapshot(&self) -> DaemonStatus {
        self.status_b.borrow().clone()
    }

    /// Tick interval for the TUI refresh.
    pub fn tick_rate() -> Duration {
        Duration::from_millis(100)
    }
}
