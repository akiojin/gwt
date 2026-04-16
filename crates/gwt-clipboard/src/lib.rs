//! gwt-clipboard: Clipboard abstraction for file paste and text operations.

pub mod file_paste;
pub mod text;

pub use file_paste::{
    clipboard_payload_to_bytes, parse_clipboard_paste, ClipboardError, ClipboardFilePaste,
    ClipboardPasteContent,
};
pub use text::ClipboardText;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parse_clipboard_paste_returns_file_paths_when_every_line_is_absolute() {
        let parsed = parse_clipboard_paste("/usr/bin/git\n/tmp/file.txt\n");
        assert!(matches!(
            parsed,
            ClipboardPasteContent::FilePaths(paths)
                if paths == vec![PathBuf::from("/usr/bin/git"), PathBuf::from("/tmp/file.txt")]
        ));
    }

    #[test]
    fn parse_clipboard_paste_returns_text_when_any_line_is_not_a_path() {
        let parsed = parse_clipboard_paste("/usr/bin/git\nrelative/path\n");
        assert!(matches!(
            parsed,
            ClipboardPasteContent::Text(text) if text == "/usr/bin/git\nrelative/path\n"
        ));
    }

    #[test]
    fn parse_clipboard_paste_returns_empty_text_for_blank_clipboard() {
        let parsed = parse_clipboard_paste("   \n\t\n");
        assert!(matches!(parsed, ClipboardPasteContent::Text(text) if text.is_empty()));
    }

    #[test]
    fn clipboard_error_display() {
        let err = ClipboardError::UnsupportedPlatform;
        assert_eq!(err.to_string(), "Unsupported platform for clipboard access");
    }

    #[test]
    fn clipboard_error_command_failed_display() {
        let err = ClipboardError::CommandFailed("pbpaste: not found".to_string());
        assert!(err.to_string().contains("pbpaste"));
    }

    #[test]
    fn clipboard_error_invalid_utf8_display() {
        let err = ClipboardError::InvalidUtf8("bad bytes".to_string());
        assert!(err.to_string().contains("UTF-8"));
    }
}
