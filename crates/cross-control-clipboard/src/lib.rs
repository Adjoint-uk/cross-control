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
//! The text path (`ClipboardFormat::PlainText`) is wired end-to-end. HTML
//! and image formats are defined in the wire protocol and accepted by the
//! trait but are not yet implemented by either backend.

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
    /// Get the current clipboard content in the preferred format.
    async fn get(&self) -> Result<ClipboardContent, ClipboardError>;

    /// Set the clipboard content.
    async fn set(&mut self, content: ClipboardContent) -> Result<(), ClipboardError>;

    /// List the formats currently available on the clipboard.
    async fn available_formats(&self) -> Result<Vec<ClipboardFormat>, ClipboardError>;

    /// Watch for clipboard changes, notifying via the returned receiver.
    async fn watch(
        &mut self,
    ) -> Result<tokio::sync::mpsc::Receiver<ClipboardContent>, ClipboardError>;
}
