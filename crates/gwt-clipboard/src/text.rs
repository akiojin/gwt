//! Plain-text clipboard read/write.

use crate::file_paste::{read_clipboard, write_clipboard, ClipboardError};

/// Plain-text clipboard operations.
pub struct ClipboardText;

impl ClipboardText {
    /// Read plain text from the system clipboard.
    pub fn get_text() -> Result<String, ClipboardError> {
        read_clipboard()
    }

    /// Write plain text to the system clipboard.
    pub fn set_text(text: &str) -> Result<(), ClipboardError> {
        write_clipboard(text)
    }
}
