//! File Tree / File Content window handlers split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - [`AppRuntime::load_file_tree_event`] /
//!   [`AppRuntime::list_file_tree_worktrees_event`] /
//!   [`AppRuntime::select_file_tree_worktree_event`] — File Tree surface
//!   listing and the SPEC worktree picker
//! - [`AppRuntime::load_file_content_event`] /
//!   [`AppRuntime::save_file_content_event`] — text/hex read and the
//!   SPEC-2006 Phase 2 write-back, with the error mapping helpers
//!   (`file_content_save_error`, `write_error_to_event`,
//!   `file_content_error_to_event`)
//!
//! Behavior-preserving move: the filesystem domain logic stays in
//! `crate::file_tree` / `crate::file_content` (re-exported via the `gwt`
//! library crate).

use std::path::Path;

use base64::Engine as _;

use super::{
    list_directory_entries, read_binary_chunk, read_text_file, AppRuntime, BackendEvent,
    ContentLimits, FileContentError, FileContentErrorKind, FileContentMode, WindowPreset,
};

fn file_content_save_error(
    id: &str,
    path: &str,
    mode: FileContentMode,
    kind: gwt::FileContentSaveErrorKind,
    message: String,
    current_mtime: Option<u64>,
    current_size: Option<u64>,
) -> BackendEvent {
    BackendEvent::FileContentSaveError {
        id: id.to_string(),
        path: path.to_string(),
        mode,
        error_kind: kind,
        message,
        current_mtime,
        current_size,
    }
}

fn write_error_to_event(
    id: &str,
    path: &str,
    mode: FileContentMode,
    err: FileContentError,
) -> BackendEvent {
    use gwt::FileContentSaveErrorKind as Kind;
    let (kind, message, current_mtime, current_size) = match err {
        FileContentError::Denied => (Kind::Denied, "Access denied".to_string(), None, None),
        FileContentError::TooLarge { size, limit } => (
            Kind::TooLarge,
            format!("File too large ({} bytes, limit {})", size, limit),
            None,
            None,
        ),
        FileContentError::IoError(message) => (Kind::IoError, message, None, None),
        FileContentError::NotAFile => (Kind::NotAFile, "Not a file".to_string(), None, None),
        FileContentError::BinaryNotText => (
            Kind::IoError,
            "Cannot decode as text".to_string(),
            None,
            None,
        ),
        FileContentError::Conflict {
            current_mtime,
            current_size,
        } => (
            Kind::Conflict,
            format!("File changed externally (current mtime={current_mtime}, size={current_size})"),
            Some(current_mtime),
            Some(current_size),
        ),
        FileContentError::ReadOnly => (Kind::ReadOnly, "File is read-only".to_string(), None, None),
        FileContentError::OutOfRange { offset, size } => (
            Kind::OutOfRange,
            format!("Offset {offset} is outside file (size {size})"),
            None,
            Some(size),
        ),
    };
    file_content_save_error(id, path, mode, kind, message, current_mtime, current_size)
}

fn file_content_error_to_event(id: &str, path: &str, err: FileContentError) -> BackendEvent {
    let (kind, message, size, limit) = match err {
        FileContentError::Denied => (
            FileContentErrorKind::Denied,
            "Access denied".to_string(),
            None,
            None,
        ),
        FileContentError::TooLarge { size, limit } => (
            FileContentErrorKind::TooLarge,
            format!("File too large ({} bytes, limit {})", size, limit),
            Some(size),
            Some(limit),
        ),
        FileContentError::IoError(message) => (FileContentErrorKind::IoError, message, None, None),
        FileContentError::NotAFile => (
            FileContentErrorKind::NotAFile,
            "Not a file".to_string(),
            None,
            None,
        ),
        FileContentError::BinaryNotText => (
            FileContentErrorKind::BinaryNotText,
            "Cannot decode as text".to_string(),
            None,
            None,
        ),
        // SPEC-2006 Phase 2 variants are write-only and should never reach
        // the read-path mapping. Map defensively to IoError so the read
        // surface keeps working if a future caller funnels them here by
        // mistake; the write surface owns the structured Save error variant.
        FileContentError::Conflict {
            current_mtime,
            current_size,
        } => (
            FileContentErrorKind::IoError,
            format!("Unexpected Conflict in read path (mtime={current_mtime} size={current_size})"),
            Some(current_size),
            None,
        ),
        FileContentError::ReadOnly => (
            FileContentErrorKind::IoError,
            "Unexpected ReadOnly in read path".to_string(),
            None,
            None,
        ),
        FileContentError::OutOfRange { offset, size } => (
            FileContentErrorKind::IoError,
            format!("Unexpected OutOfRange in read path (offset={offset} size={size})"),
            Some(size),
            None,
        ),
    };
    BackendEvent::FileContentError {
        id: id.to_string(),
        path: path.to_string(),
        error_kind: kind,
        message,
        size,
        limit,
    }
}

impl AppRuntime {
    pub(crate) fn load_file_tree_event(&self, id: &str, path: &str) -> BackendEvent {
        let root = match self.resolve_file_tree_root(id) {
            Ok(root) => root,
            Err(message) => {
                return BackendEvent::FileTreeError {
                    id: id.to_string(),
                    path: path.to_string(),
                    message,
                };
            }
        };

        let relative_path = if path.is_empty() {
            None
        } else {
            Some(Path::new(path))
        };

        match list_directory_entries(&root, relative_path) {
            Ok(entries) => BackendEvent::FileTreeEntries {
                id: id.to_string(),
                path: path.to_string(),
                entries,
            },
            Err(error) => BackendEvent::FileTreeError {
                id: id.to_string(),
                path: path.to_string(),
                message: error.to_string(),
            },
        }
    }

    /// Resolve the worktree root for a File Tree window. Prefers the user's
    /// picker selection (`file_tree_worktree_roots`); falls back to
    /// `tab.project_root` for backward compatibility with existing callers
    /// that pre-date the picker. Returns a human-readable error message on
    /// invalid window id / wrong preset.
    fn resolve_file_tree_root(&self, id: &str) -> Result<std::path::PathBuf, String> {
        let address = self
            .window_lookup
            .get(id)
            .ok_or_else(|| "Window not found".to_string())?;
        let tab = self
            .tab(&address.tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let window = tab
            .workspace
            .window(&address.raw_id)
            .ok_or_else(|| "Window not found".to_string())?;
        if window.preset != WindowPreset::FileTree {
            return Err("Window is not a file tree".to_string());
        }
        Ok(self
            .file_tree_worktree_roots
            .get(id)
            .cloned()
            .unwrap_or_else(|| tab.project_root.clone()))
    }

    pub(crate) fn list_file_tree_worktrees_event(&self, id: &str) -> BackendEvent {
        let address = match self.window_lookup.get(id) {
            Some(addr) => addr,
            None => {
                return BackendEvent::FileTreeWorktreeError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                };
            }
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            };
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            };
        };
        if window.preset != WindowPreset::FileTree {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window is not a file tree".to_string(),
            };
        }
        match gwt::worktree_inventory::enumerate_worktrees(
            &tab.project_root,
            Some(&tab.project_root),
        ) {
            Ok(entries) => BackendEvent::FileTreeWorktrees {
                id: id.to_string(),
                entries,
            },
            Err(err) => BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: err.to_string(),
            },
        }
    }

    pub(crate) fn select_file_tree_worktree_event(
        &mut self,
        id: &str,
        worktree_id: &str,
    ) -> BackendEvent {
        let address = match self.window_lookup.get(id) {
            Some(addr) => addr,
            None => {
                return BackendEvent::FileTreeWorktreeError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                };
            }
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            };
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            };
        };
        if window.preset != WindowPreset::FileTree {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window is not a file tree".to_string(),
            };
        }
        let entries = match gwt::worktree_inventory::enumerate_worktrees(
            &tab.project_root,
            Some(&tab.project_root),
        ) {
            Ok(entries) => entries,
            Err(err) => {
                return BackendEvent::FileTreeWorktreeError {
                    id: id.to_string(),
                    message: err.to_string(),
                };
            }
        };
        let Some(selected) = entries.into_iter().find(|entry| entry.id == worktree_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Unknown worktree id".to_string(),
            };
        };
        self.file_tree_worktree_roots
            .insert(id.to_string(), selected.path);
        BackendEvent::FileTreeWorktreeSelected {
            id: id.to_string(),
            worktree_id: worktree_id.to_string(),
        }
    }

    pub(crate) fn load_file_content_event(
        &self,
        id: &str,
        path: &str,
        mode: FileContentMode,
        hex_offset: Option<u64>,
        hex_length: Option<u64>,
    ) -> BackendEvent {
        let make_error =
            |kind: FileContentErrorKind, message: String, size: Option<u64>, limit: Option<u64>| {
                BackendEvent::FileContentError {
                    id: id.to_string(),
                    path: path.to_string(),
                    error_kind: kind,
                    message,
                    size,
                    limit,
                }
            };

        let root = match self.resolve_file_tree_root(id) {
            Ok(root) => root,
            Err(message) => {
                let kind = if message == "Window is not a file tree" {
                    FileContentErrorKind::WindowMismatch
                } else {
                    FileContentErrorKind::WindowNotFound
                };
                return make_error(kind, message, None, None);
            }
        };

        let relative_path = Path::new(path);
        let limits = ContentLimits::default();

        match mode {
            FileContentMode::Text => match read_text_file(&root, relative_path, &limits) {
                Ok(result) => BackendEvent::FileContentText {
                    id: id.to_string(),
                    path: path.to_string(),
                    encoding: result.encoding,
                    text: result.text,
                    total_size: result.total_size,
                    mtime: result.mtime,
                    has_bom: result.has_bom,
                    newline: result.newline,
                    read_only: result.read_only,
                },
                Err(err) => file_content_error_to_event(id, path, err),
            },
            FileContentMode::Hex => {
                let offset = hex_offset.unwrap_or(0);
                let length = hex_length.unwrap_or(64 * 16);
                match read_binary_chunk(&root, relative_path, offset, length, &limits) {
                    Ok(chunk) => BackendEvent::FileContentHex {
                        id: id.to_string(),
                        path: path.to_string(),
                        offset: chunk.offset,
                        bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
                        total_size: chunk.total_size,
                        mtime: chunk.mtime,
                        read_only: chunk.read_only,
                    },
                    Err(err) => file_content_error_to_event(id, path, err),
                }
            }
        }
    }

    /// SPEC-2006 Phase 2 amendment: write the modified text or single hex
    /// byte back to disk, mapping every domain error to the structured
    /// `FileContentSaveErrorKind` variant the GUI listens for.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_file_content_event(
        &self,
        id: &str,
        path: &str,
        mode: FileContentMode,
        expected_mtime: u64,
        expected_size: u64,
        text: Option<String>,
        encoding: Option<gwt::Encoding>,
        newline: Option<gwt::Newline>,
        has_bom: Option<bool>,
        hex_offset: Option<u64>,
        hex_byte: Option<u8>,
    ) -> BackendEvent {
        let root = match self.resolve_file_tree_root(id) {
            Ok(root) => root,
            Err(message) => {
                let kind = if message == "Window is not a file tree" {
                    gwt::FileContentSaveErrorKind::WindowMismatch
                } else {
                    gwt::FileContentSaveErrorKind::WindowNotFound
                };
                return BackendEvent::FileContentSaveError {
                    id: id.to_string(),
                    path: path.to_string(),
                    mode,
                    error_kind: kind,
                    message,
                    current_mtime: None,
                    current_size: None,
                };
            }
        };

        let relative_path = Path::new(path);
        let limits = ContentLimits::default();
        let expected = gwt::ExpectedMetadata {
            mtime: expected_mtime,
            size: expected_size,
        };

        match mode {
            FileContentMode::Text => {
                let Some(text) = text else {
                    return file_content_save_error(
                        id,
                        path,
                        mode,
                        gwt::FileContentSaveErrorKind::IoError,
                        "save_file_content(text) missing text payload".to_string(),
                        None,
                        None,
                    );
                };
                let encoding = encoding.unwrap_or(gwt::Encoding::Utf8);
                let newline = newline.unwrap_or(gwt::Newline::Lf);
                let has_bom = has_bom.unwrap_or(false);
                match gwt::write_text_file(
                    &root,
                    relative_path,
                    &text,
                    encoding,
                    newline,
                    has_bom,
                    expected,
                    &limits,
                ) {
                    Ok(outcome) => BackendEvent::FileContentSaved {
                        id: id.to_string(),
                        path: path.to_string(),
                        mode,
                        new_mtime: outcome.new_mtime,
                        new_size: outcome.new_size,
                        encoding_fallback: outcome.encoding_fallback,
                    },
                    Err(err) => write_error_to_event(id, path, mode, err),
                }
            }
            FileContentMode::Hex => {
                let Some(offset) = hex_offset else {
                    return file_content_save_error(
                        id,
                        path,
                        mode,
                        gwt::FileContentSaveErrorKind::IoError,
                        "save_file_content(hex) missing hex_offset".to_string(),
                        None,
                        None,
                    );
                };
                let Some(byte) = hex_byte else {
                    return file_content_save_error(
                        id,
                        path,
                        mode,
                        gwt::FileContentSaveErrorKind::IoError,
                        "save_file_content(hex) missing hex_byte".to_string(),
                        None,
                        None,
                    );
                };
                match gwt::write_binary_byte(&root, relative_path, offset, byte, expected) {
                    Ok(outcome) => BackendEvent::FileContentSaved {
                        id: id.to_string(),
                        path: path.to_string(),
                        mode,
                        new_mtime: outcome.new_mtime,
                        new_size: outcome.new_size,
                        encoding_fallback: outcome.encoding_fallback,
                    },
                    Err(err) => write_error_to_event(id, path, mode, err),
                }
            }
        }
    }
}
