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

/// `arboard`-backed [`ClipboardProvider`]. Supports text, HTML, and PNG
/// images. Image payloads are converted between the wire's PNG encoding and
/// the raw RGBA the platform clipboard exposes.
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

    async fn get_format(
        &self,
        format: ClipboardFormat,
    ) -> Result<ClipboardContent, ClipboardError> {
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|_| ClipboardError::AccessDenied)?;
            match format {
                ClipboardFormat::PlainText => guard.get_text().map(|t| ClipboardContent::text(&t)),
                ClipboardFormat::Html => guard.get().html().map(|h| ClipboardContent::html(&h)),
                ClipboardFormat::Png => {
                    let img = guard.get_image().map_err(map_arboard_error)?;
                    return rgba_to_png(img.width, img.height, &img.bytes)
                        .map(ClipboardContent::png);
                }
            }
            .map_err(map_arboard_error)
        })
        .await
        .map_err(|e| ClipboardError::Other(e.into()))?
    }

    async fn set(&mut self, content: ClipboardContent) -> Result<(), ClipboardError> {
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|_| ClipboardError::AccessDenied)?;
            match content.format {
                ClipboardFormat::PlainText => {
                    let text = content.as_text().ok_or(ClipboardError::FormatUnavailable)?;
                    guard.set_text(text).map_err(map_arboard_error)
                }
                ClipboardFormat::Html => {
                    let html = content.as_html().ok_or(ClipboardError::FormatUnavailable)?;
                    guard.set_html(html, None).map_err(map_arboard_error)
                }
                ClipboardFormat::Png => {
                    let image = png_to_rgba(&content.data)?;
                    guard.set_image(image).map_err(map_arboard_error)
                }
            }
        })
        .await
        .map_err(|e| ClipboardError::Other(e.into()))?
    }

    async fn available_formats(&self) -> Result<Vec<ClipboardFormat>, ClipboardError> {
        // arboard has no list-formats API, so probe each format we support.
        // A probe that comes back `ContentNotAvailable` just means that
        // format isn't on the clipboard; only a harder error propagates.
        let inner = Arc::clone(&self.inner);
        spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|_| ClipboardError::AccessDenied)?;
            let mut formats = Vec::new();
            probe(guard.get_text(), ClipboardFormat::PlainText, &mut formats)?;
            probe(guard.get().html(), ClipboardFormat::Html, &mut formats)?;
            probe(guard.get_image(), ClipboardFormat::Png, &mut formats)?;
            Ok(formats)
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

/// Record `format` as available when the probe succeeded. A missing format
/// (`ContentNotAvailable`) is skipped; any other error propagates.
fn probe<T>(
    result: Result<T, arboard::Error>,
    format: ClipboardFormat,
    into: &mut Vec<ClipboardFormat>,
) -> Result<(), ClipboardError> {
    match result {
        Ok(_) => {
            into.push(format);
            Ok(())
        }
        Err(arboard::Error::ContentNotAvailable | arboard::Error::ConversionFailure) => Ok(()),
        Err(e) => Err(map_arboard_error(e)),
    }
}

/// Encode raw RGBA8 pixels (row-major, 4 bytes/pixel) as PNG.
fn rgba_to_png(width: usize, height: usize, rgba: &[u8]) -> Result<Vec<u8>, ClipboardError> {
    let w = u32::try_from(width).map_err(|_| ClipboardError::FormatUnavailable)?;
    let h = u32::try_from(height).map_err(|_| ClipboardError::FormatUnavailable)?;
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| ClipboardError::Other(anyhow::anyhow!("RGBA buffer size mismatch")))?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| ClipboardError::Other(e.into()))?;
    Ok(out.into_inner())
}

/// Decode PNG bytes into the raw RGBA [`arboard::ImageData`] the clipboard wants.
fn png_to_rgba(png: &[u8]) -> Result<arboard::ImageData<'static>, ClipboardError> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| ClipboardError::Other(e.into()))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: std::borrow::Cow::Owned(img.into_raw()),
    })
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

    #[test]
    fn png_rgba_round_trip() {
        // A 2x2 image with four distinct RGBA pixels survives encode → decode.
        let width = 2usize;
        let height = 2usize;
        let rgba: Vec<u8> = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 128, // semi-transparent yellow
        ];
        let png = rgba_to_png(width, height, &rgba).expect("encode");
        // PNG magic number.
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
        let decoded = png_to_rgba(&png).expect("decode");
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.bytes.as_ref(), rgba.as_slice());
    }

    #[test]
    fn png_decode_rejects_garbage() {
        assert!(png_to_rgba(&[0, 1, 2, 3, 4]).is_err());
    }

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
