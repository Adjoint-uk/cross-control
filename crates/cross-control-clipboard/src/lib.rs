//! Clipboard synchronisation for cross-control.
//!
//! [`ClipboardProvider`] is the abstraction the daemon uses. Two backends
//! ship today:
//!
//! - [`arboard_backend::ArboardClipboard`] (feature `arboard`, default) —
//!   real clipboard over X11, macOS, Windows, and wlroots-based Wayland.
//! - [`mock::MockClipboard`] (feature `mock`) — in-memory, for tests and
//!   headless runs.
//!
//! Text, HTML, and PNG images are wired end-to-end through the `arboard`
//! backend. Image payloads convert between the wire's PNG bytes and the raw
//! RGBA the platform clipboard uses.

use async_trait::async_trait;
use cross_control_types::{ClipboardContent, ClipboardFormat};

#[cfg(feature = "arboard")]
pub mod arboard_backend;
pub mod error;
#[cfg(feature = "mock")]
pub mod mock;

#[cfg(feature = "arboard")]
pub use arboard_backend::ArboardClipboard;
pub use error::ClipboardError;
#[cfg(feature = "mock")]
pub use mock::MockClipboard;

/// Platform clipboard access.
///
/// `Sync` is required because the daemon holds `&self.clipboard` across
/// `.await` points when issuing reads, and tokio spawns the daemon onto a
/// multi-thread runtime.
#[async_trait]
pub trait ClipboardProvider: Send + Sync + 'static {
    /// Get the current clipboard content in the preferred format (text if
    /// available). Used for size hints; format-specific reads go through
    /// [`ClipboardProvider::get_format`].
    async fn get(&self) -> Result<ClipboardContent, ClipboardError>;

    /// Get the current clipboard content in a specific format, or
    /// [`ClipboardError::FormatUnavailable`] if that format isn't present.
    async fn get_format(&self, format: ClipboardFormat)
        -> Result<ClipboardContent, ClipboardError>;

    /// Set the clipboard content.
    async fn set(&mut self, content: ClipboardContent) -> Result<(), ClipboardError>;

    /// List the formats currently available on the clipboard.
    async fn available_formats(&self) -> Result<Vec<ClipboardFormat>, ClipboardError>;

    /// Watch for clipboard changes, notifying via the returned receiver.
    async fn watch(
        &mut self,
    ) -> Result<tokio::sync::mpsc::Receiver<ClipboardContent>, ClipboardError>;
}
