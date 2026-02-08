//! Clipboard content types.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Format of clipboard content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ClipboardFormat {
    /// Plain UTF-8 text.
    PlainText,
    /// HTML content.
    Html,
    /// PNG image data.
    Png,
}

/// Clipboard content with format metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct ClipboardContent {
    pub format: ClipboardFormat,
    pub data: Vec<u8>,
}

impl ClipboardContent {
    /// Create text clipboard content.
    #[must_use]
    pub fn text(s: &str) -> Self {
        Self {
            format: ClipboardFormat::PlainText,
            data: s.as_bytes().to_vec(),
        }
    }

    /// Try to interpret the data as UTF-8 text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        if self.format == ClipboardFormat::PlainText {
            std::str::from_utf8(&self.data).ok()
        } else {
            None
        }
    }

    /// Size of the content in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_text_roundtrip() {
        let content = ClipboardContent::text("hello clipboard");
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&content, config).unwrap();
        let (decoded, _): (ClipboardContent, _) =
            bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(content, decoded);
        assert_eq!(decoded.as_text(), Some("hello clipboard"));
    }

    #[test]
    fn clipboard_png_roundtrip() {
        let content = ClipboardContent {
            format: ClipboardFormat::Png,
            data: vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A],
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&content, config).unwrap();
        let (decoded, _): (ClipboardContent, _) =
            bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(content, decoded);
        assert_eq!(decoded.as_text(), None);
    }

    #[test]
    fn clipboard_size() {
        let content = ClipboardContent::text("abc");
        assert_eq!(content.size(), 3);
    }
}
