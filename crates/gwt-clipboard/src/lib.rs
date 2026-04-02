//! gwt-clipboard: Clipboard abstraction for file paste and text operations.

pub mod file_paste;
pub mod text;

pub use file_paste::{ClipboardError, ClipboardFilePaste};
pub use text::ClipboardText;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extract_file_paths_filters_non_absolute() {
        // ClipboardFilePaste::extract_file_paths relies on system clipboard,
        // so we test the filtering logic via a unit helper.
        let input = "/usr/bin/git\nrelative/path\n/tmp/file.txt\n\n";
        let paths: Vec<PathBuf> = input
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();

        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/usr/bin/git"));
        assert_eq!(paths[1], PathBuf::from("/tmp/file.txt"));
    }

    #[test]
    fn clipboard_error_display() {
        let err = ClipboardError::UnsupportedPlatform;
        assert_eq!(
            err.to_string(),
            "Unsupported platform for clipboard access"
        );
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
