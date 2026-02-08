//! Clipboard synchronisation for cross-control.
//!
//! Defines the [`ClipboardProvider`] trait for platform clipboard access.
//! Backends (arboard, wl-clipboard-rs) will be added in later phases.

use async_trait::async_trait;
use cross_control_types::{ClipboardContent, ClipboardFormat};

pub mod error;

pub use error::ClipboardError;

/// Platform clipboard access.
#[async_trait]
pub trait ClipboardProvider: Send + 'static {
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
