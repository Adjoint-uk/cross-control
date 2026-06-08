//! `arboard`-based clipboard backend.
//!
//! Covers X11, macOS, Windows, and wlroots-based Wayland compositors (Sway,
//! Hyprland, river). GNOME and KDE on Wayland use a different protocol and
//! will fall back to "format unavailable" — `wl-clipboard-rs` is the planned
//! follow-up for those compositors.
//!
//! Watch is polled. `arboard` exposes no native change-notification API, so
//! we read the clipboard at a configurable interval and emit whenever the
//! content differs from the last value. The default 500ms is a defensible
//! middle ground between latency and CPU.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cross_control_types::{ClipboardContent, ClipboardFormat};
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
use tracing::{debug, warn};

use crate::{ClipboardError, ClipboardProvider};

/// Polling interval for `watch`. Tunable via [`ArboardClipboard::with_poll_interval`].
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// `arboard`-backed [`ClipboardProvider`]. Text-only for now.
pub struct ArboardClipboard {
    /// Shared so `spawn_blocking` tasks can each grab a fresh lock.
    inner: Arc<Mutex<arboard::Clipboard>>,
    poll_interval: Duration,
    watch_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ArboardClipboard {
    /// Construct a clipboard handle. Returns `Unavailable` if no display server
    /// is reachable (headless host, missing X/Wayland socket, etc.).
    pub fn new() -> Result<Self, ClipboardError> {
        let clipboard = arboard::Clipboard::new().map_err(map_arboard_error)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(clipboard)),
            poll_interval: DEFAULT_POLL_INTERVAL,
            watch_handle: None,
        })
    }

    /// Set a custom polling interval for [`ClipboardProvider::watch`].
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }
}

#[async_trait]
impl ClipboardProvider for ArboardClipboard {
    async fn get(&self) -> Result<ClipboardContent, ClipboardError> {
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|_| ClipboardError::AccessDenied)?;
            match guard.get_text() {
                Ok(text) => Ok(ClipboardContent::text(&text)),
                Err(e) => Err(map_arboard_error(e)),
            }
        })
        .await
        .map_err(|e| ClipboardError::Other(e.into()))?
    }

    async fn set(&mut self, content: ClipboardContent) -> Result<(), ClipboardError> {
        if content.format != ClipboardFormat::PlainText {
            return Err(ClipboardError::FormatUnavailable);
        }
        let text = content
            .as_text()
            .ok_or(ClipboardError::FormatUnavailable)?
            .to_string();
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|_| ClipboardError::AccessDenied)?;
            guard.set_text(text).map_err(map_arboard_error)
        })
        .await
        .map_err(|e| ClipboardError::Other(e.into()))?
    }

    async fn available_formats(&self) -> Result<Vec<ClipboardFormat>, ClipboardError> {
        // arboard doesn't expose a list-formats API. We probe `get_text` and
        // report PlainText if it works. HTML / image probing would add round
        // trips for formats we don't yet sync — skip until those land.
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|_| ClipboardError::AccessDenied)?;
            match guard.get_text() {
                Ok(_) => Ok(vec![ClipboardFormat::PlainText]),
                Err(arboard::Error::ContentNotAvailable) => Ok(Vec::new()),
                Err(e) => Err(map_arboard_error(e)),
            }
        })
        .await
        .map_err(|e| ClipboardError::Other(e.into()))?
    }

    async fn watch(&mut self) -> Result<mpsc::Receiver<ClipboardContent>, ClipboardError> {
        let (tx, rx) = mpsc::channel::<ClipboardContent>(8);
        let inner = Arc::clone(&self.inner);
        let interval = self.poll_interval;

        let handle = tokio::spawn(async move {
            let mut last: Option<ClipboardContent> = None;
            let mut ticker = tokio::time::interval(interval);
            // Skip the immediate tick — we don't want to emit on startup
            // unless content actually changes.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let inner = Arc::clone(&inner);
                let result = spawn_blocking(move || {
                    let mut guard = inner.lock().ok()?;
                    guard.get_text().ok().map(|t| ClipboardContent::text(&t))
                })
                .await;
                let current = match result {
                    Ok(opt) => opt,
                    Err(e) => {
                        warn!(error = %e, "clipboard poll panicked");
                        continue;
                    }
                };
                if let Some(content) = current {
                    if last.as_ref() != Some(&content) {
                        debug!(size = content.size(), "clipboard changed");
                        if tx.send(content.clone()).await.is_err() {
                            break;
                        }
                        last = Some(content);
                    }
                }
            }
        });

        // Replace any prior watcher.
        if let Some(prev) = self.watch_handle.take() {
            prev.abort();
        }
        self.watch_handle = Some(handle);
        Ok(rx)
    }
}

impl Drop for ArboardClipboard {
    fn drop(&mut self) {
        if let Some(h) = self.watch_handle.take() {
            h.abort();
        }
    }
}

/// Translate an [`arboard::Error`] into our [`ClipboardError`] variants.
fn map_arboard_error(e: arboard::Error) -> ClipboardError {
    match e {
        arboard::Error::ContentNotAvailable | arboard::Error::ConversionFailure => {
            ClipboardError::FormatUnavailable
        }
        arboard::Error::ClipboardNotSupported | arboard::Error::ClipboardOccupied => {
            ClipboardError::Unavailable
        }
        other => ClipboardError::Other(anyhow::anyhow!(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructing the backend requires a display server; ignore by default.
    #[tokio::test]
    #[ignore = "requires a display server"]
    async fn construct_and_set_get_text() {
        let mut cb = ArboardClipboard::new().expect("display available");
        cb.set(ClipboardContent::text("hello cross-control"))
            .await
            .expect("set");
        let content = cb.get().await.expect("get");
        assert_eq!(content.as_text(), Some("hello cross-control"));
    }
}
