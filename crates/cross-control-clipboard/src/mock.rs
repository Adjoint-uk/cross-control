//! In-memory [`ClipboardProvider`] for tests and headless daemon runs.
//!
//! Useful when the host has no display server (CI, container) but daemon
//! code that depends on a clipboard still needs to compile and exercise
//! the message paths.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cross_control_types::{ClipboardContent, ClipboardFormat};
use tokio::sync::mpsc;

use crate::{ClipboardError, ClipboardProvider};

/// Shared in-memory clipboard. Cheap to clone; all clones see the same state.
#[derive(Debug, Clone, Default)]
pub struct MockClipboard {
    state: Arc<Mutex<Option<ClipboardContent>>>,
    /// Channels to notify on `set` so `watch` callers see changes.
    watchers: Arc<Mutex<Vec<mpsc::Sender<ClipboardContent>>>>,
}

impl MockClipboard {
    /// Construct an empty mock clipboard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-populate with the given content.
    #[must_use]
    pub fn with_content(content: ClipboardContent) -> Self {
        Self {
            state: Arc::new(Mutex::new(Some(content))),
            watchers: Arc::default(),
        }
    }
}

#[async_trait]
impl ClipboardProvider for MockClipboard {
    async fn get(&self) -> Result<ClipboardContent, ClipboardError> {
        let guard = self
            .state
            .lock()
            .map_err(|_| ClipboardError::AccessDenied)?;
        guard.clone().ok_or(ClipboardError::FormatUnavailable)
    }

    async fn get_format(
        &self,
        format: ClipboardFormat,
    ) -> Result<ClipboardContent, ClipboardError> {
        let guard = self
            .state
            .lock()
            .map_err(|_| ClipboardError::AccessDenied)?;
        match guard.as_ref() {
            Some(content) if content.format == format => Ok(content.clone()),
            _ => Err(ClipboardError::FormatUnavailable),
        }
    }

    async fn set(&mut self, content: ClipboardContent) -> Result<(), ClipboardError> {
        {
            let mut guard = self
                .state
                .lock()
                .map_err(|_| ClipboardError::AccessDenied)?;
            *guard = Some(content.clone());
        }
        // Notify watchers. Drop dead senders.
        let mut watchers = self
            .watchers
            .lock()
            .map_err(|_| ClipboardError::AccessDenied)?;
        watchers.retain(|tx| tx.try_send(content.clone()).is_ok());
        Ok(())
    }

    async fn available_formats(&self) -> Result<Vec<ClipboardFormat>, ClipboardError> {
        let guard = self
            .state
            .lock()
            .map_err(|_| ClipboardError::AccessDenied)?;
        Ok(guard.as_ref().map(|c| vec![c.format]).unwrap_or_default())
    }

    async fn watch(&mut self) -> Result<mpsc::Receiver<ClipboardContent>, ClipboardError> {
        let (tx, rx) = mpsc::channel::<ClipboardContent>(8);
        self.watchers
            .lock()
            .map_err(|_| ClipboardError::AccessDenied)?
            .push(tx);
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_then_get_returns_content() {
        let mut cb = MockClipboard::new();
        cb.set(ClipboardContent::text("hello")).await.unwrap();
        let got = cb.get().await.unwrap();
        assert_eq!(got.as_text(), Some("hello"));
    }

    #[tokio::test]
    async fn empty_clipboard_returns_format_unavailable() {
        let cb = MockClipboard::new();
        let err = cb.get().await.unwrap_err();
        assert!(matches!(err, ClipboardError::FormatUnavailable));
    }

    #[tokio::test]
    async fn watch_emits_on_set() {
        let mut cb = MockClipboard::new();
        let mut rx = cb.watch().await.unwrap();
        cb.set(ClipboardContent::text("first")).await.unwrap();
        cb.set(ClipboardContent::text("second")).await.unwrap();

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.as_text(), Some("first"));
        assert_eq!(second.as_text(), Some("second"));
    }

    #[tokio::test]
    async fn available_formats_is_empty_when_unset() {
        let cb = MockClipboard::new();
        assert!(cb.available_formats().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn available_formats_reflects_set_format() {
        let mut cb = MockClipboard::new();
        cb.set(ClipboardContent::text("x")).await.unwrap();
        assert_eq!(
            cb.available_formats().await.unwrap(),
            vec![ClipboardFormat::PlainText]
        );
    }

    #[tokio::test]
    async fn get_format_matches_stored_format() {
        let mut cb = MockClipboard::new();
        cb.set(ClipboardContent::html("<i>hi</i>")).await.unwrap();
        let got = cb.get_format(ClipboardFormat::Html).await.unwrap();
        assert_eq!(got.as_html(), Some("<i>hi</i>"));
        // A different format isn't available.
        assert!(matches!(
            cb.get_format(ClipboardFormat::Png).await,
            Err(ClipboardError::FormatUnavailable)
        ));
    }

    #[tokio::test]
    async fn get_format_returns_png() {
        let mut cb = MockClipboard::new();
        cb.set(ClipboardContent::png(vec![1, 2, 3])).await.unwrap();
        let got = cb.get_format(ClipboardFormat::Png).await.unwrap();
        assert_eq!(got.format, ClipboardFormat::Png);
        assert_eq!(got.data, vec![1, 2, 3]);
    }
}
