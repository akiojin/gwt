//! Image paste / file attachment pipeline split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - The prepared-payload types ([`ImagePasteFile`],
//!   [`PreparedFileAttachment`], [`UploadedImagePasteOperation`]) and their
//!   error enums ([`ImagePasteError`], [`FileAttachmentError`])
//! - Name/size sanitization and storage-path derivation
//!   (`sanitize_file_attachment_name`, `attachment_storage_paths`, ...)
//! - Streaming save helpers with [`AttachmentProgressUpdate`] progress
//!   dispatch (`save_file_attachment_with_progress`,
//!   `save_image_paste_file_with_progress`)
//! - Qualified ingress through [`AppRuntime::paste_image_uploaded_operation_events`]
//!   and [`AppRuntime::attach_files_operation_events`], with immutable target
//!   capture, visible legacy refusal, and [`AppRuntime::inject_attachment_prompt_events`]
//!   dispatch to the private physical receipt path.
//!
//! SPEC-3064 FR-002 (Pass 2): the former `IMAGE_PASTE_SEQUENCE` module
//! static now lives on [`AppRuntime`] as the `image_paste_sequence` field
//! and is threaded into the token helpers as `&AtomicU64`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::{
    AppEventProxy, AppRuntime, AttachmentProgressPhase, AttachmentUploadStore, BackendEvent,
    ClientId, ContentLimits, OutboundEvent, UserEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImagePasteFile {
    pub(crate) bytes: Option<Vec<u8>>,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) remove_source_after_save: bool,
    pub(crate) storage_path: PathBuf,
    pub(crate) agent_path: String,
    pub(crate) prompt_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImagePasteError {
    UnsupportedMimeType(String),
    EmptyPayload,
    #[cfg(test)]
    InvalidBase64(String),
    WriteFailed(String),
}

impl std::fmt::Display for ImagePasteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMimeType(mime_type) => {
                write!(formatter, "unsupported image MIME type: {mime_type}")
            }
            Self::EmptyPayload => formatter.write_str("image paste payload is empty"),
            #[cfg(test)]
            Self::InvalidBase64(error) => write!(formatter, "invalid image paste payload: {error}"),
            Self::WriteFailed(error) => write!(formatter, "failed to save pasted image: {error}"),
        }
    }
}

const IMAGE_PASTE_PROMPT_PREFIX: &str = "Image file: ";
const FILE_ATTACHMENT_RELATIVE_DIR: &str = ".gwt/drop-files";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedFileAttachment {
    pub(crate) bytes: Option<Vec<u8>>,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) remove_source_after_save: bool,
    pub(crate) storage_path: Option<PathBuf>,
    pub(crate) agent_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileAttachmentError {
    EmptyPath,
    InvalidBase64(String),
    SizeMismatch { declared: u64, actual: u64 },
    TooLarge { size: u64, limit: u64 },
    NotAFile(String),
    ReadFailed(String),
    WriteFailed(String),
    UploadedFileMissing(String),
}

impl std::fmt::Display for FileAttachmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("file attachment path is empty"),
            Self::InvalidBase64(error) => {
                write!(formatter, "invalid file attachment payload: {error}")
            }
            Self::SizeMismatch { declared, actual } => write!(
                formatter,
                "file attachment size mismatch: declared {declared}, decoded {actual}"
            ),
            Self::TooLarge { size, limit } => {
                write!(formatter, "file attachment is too large: {size} > {limit}")
            }
            Self::NotAFile(path) => write!(formatter, "file attachment is not a file: {path}"),
            Self::ReadFailed(error) => write!(formatter, "failed to read file attachment: {error}"),
            Self::WriteFailed(error) => {
                write!(formatter, "failed to save file attachment: {error}")
            }
            Self::UploadedFileMissing(upload_id) => {
                write!(
                    formatter,
                    "uploaded file attachment is missing: {upload_id}"
                )
            }
        }
    }
}

fn image_extension_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn sanitize_image_paste_stem(filename: Option<&str>) -> String {
    let raw_stem = filename
        .and_then(|name| Path::new(name).file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("image");
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for character in raw_stem.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            sanitized.push(character);
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "image".to_string()
    } else {
        sanitized.to_string()
    }
}

fn sanitize_file_attachment_name(filename: &str) -> String {
    let trimmed = filename.trim();
    let raw_name = trimmed
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("file");
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for character in raw_name.chars() {
        let unsafe_character = character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            );
        if unsafe_character || character.is_whitespace() || character == '-' {
            if !previous_dash {
                sanitized.push('-');
                previous_dash = true;
            }
        } else if character.is_ascii() {
            sanitized.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else {
            sanitized.push(character);
            previous_dash = false;
        }
    }
    let sanitized = sanitized.trim_matches(['-', '.', '_']);
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "file".to_string()
    } else if is_reserved_attachment_basename(sanitized) {
        format!("file-{sanitized}")
    } else {
        sanitized.to_string()
    }
}

fn is_reserved_attachment_basename(filename: &str) -> bool {
    let stem = filename
        .split('.')
        .next()
        .unwrap_or(filename)
        .trim_matches([' ', '.', '_', '-'])
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn attachment_storage_paths(
    worktree_path: &Path,
    _agent_project_root: &str,
    unique_token: &str,
    filename: &str,
) -> (PathBuf, String) {
    let sanitized = sanitize_file_attachment_name(filename);
    let file_name = format!("{unique_token}-{sanitized}");
    let storage_path = worktree_path
        .join(".gwt")
        .join("drop-files")
        .join(&file_name);
    let relative_path = format!("{FILE_ATTACHMENT_RELATIVE_DIR}/{file_name}");
    let agent_path = relative_path;
    (storage_path, agent_path)
}

fn validate_file_attachment_size(size: u64, limit: u64) -> Result<(), FileAttachmentError> {
    if size > limit {
        return Err(FileAttachmentError::TooLarge { size, limit });
    }
    Ok(())
}

pub(crate) fn prepare_file_attachment(
    worktree_path: &Path,
    agent_project_root: &str,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    file: &gwt::FileAttachment,
    unique_token: &str,
    limits: ContentLimits,
    upload_store: &AttachmentUploadStore,
) -> Result<PreparedFileAttachment, FileAttachmentError> {
    let _ = runtime_target;
    match file {
        gwt::FileAttachment::NativePath { path } => {
            let path = path.trim();
            if path.is_empty() {
                return Err(FileAttachmentError::EmptyPath);
            }
            let source = PathBuf::from(path);
            let metadata = std::fs::metadata(&source)
                .map_err(|error| FileAttachmentError::ReadFailed(error.to_string()))?;
            if !metadata.is_file() {
                return Err(FileAttachmentError::NotAFile(path.to_string()));
            }
            let filename = source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file");
            let (storage_path, agent_path) =
                attachment_storage_paths(worktree_path, agent_project_root, unique_token, filename);
            Ok(PreparedFileAttachment {
                bytes: None,
                source_path: Some(source),
                remove_source_after_save: false,
                storage_path: Some(storage_path),
                agent_path,
            })
        }
        gwt::FileAttachment::Inline {
            filename,
            size,
            data_base64,
            ..
        } => {
            validate_file_attachment_size(*size, limits.binary_chunk_max_bytes)?;
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                data_base64.trim(),
            )
            .map_err(|error| FileAttachmentError::InvalidBase64(error.to_string()))?;
            let actual = bytes.len() as u64;
            if actual != *size {
                return Err(FileAttachmentError::SizeMismatch {
                    declared: *size,
                    actual,
                });
            }
            let (storage_path, agent_path) =
                attachment_storage_paths(worktree_path, agent_project_root, unique_token, filename);
            Ok(PreparedFileAttachment {
                bytes: Some(bytes),
                source_path: None,
                remove_source_after_save: false,
                storage_path: Some(storage_path),
                agent_path,
            })
        }
        gwt::FileAttachment::Uploaded {
            upload_id,
            filename,
            size,
            ..
        } => {
            let uploaded = upload_store
                .take(upload_id)
                .map_err(FileAttachmentError::ReadFailed)?
                .ok_or_else(|| FileAttachmentError::UploadedFileMissing(upload_id.clone()))?;
            if uploaded.size != *size {
                return Err(FileAttachmentError::SizeMismatch {
                    declared: *size,
                    actual: uploaded.size,
                });
            }
            let filename = if filename.trim().is_empty() {
                uploaded.filename.as_str()
            } else {
                filename.as_str()
            };
            let (storage_path, agent_path) =
                attachment_storage_paths(worktree_path, agent_project_root, unique_token, filename);
            Ok(PreparedFileAttachment {
                bytes: None,
                source_path: Some(uploaded.path),
                remove_source_after_save: true,
                storage_path: Some(storage_path),
                agent_path,
            })
        }
    }
}

pub(super) fn save_file_attachment_with_progress(
    file: &PreparedFileAttachment,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), FileAttachmentError> {
    let Some(storage_path) = file.storage_path.as_ref() else {
        return Ok(());
    };
    if let Some(bytes) = file.bytes.as_ref() {
        return write_attachment_bytes_with_progress(storage_path, bytes, &mut on_progress)
            .map_err(FileAttachmentError::WriteFailed);
    }
    if let Some(source_path) = file.source_path.as_ref() {
        copy_attachment_file_with_progress(
            source_path,
            storage_path,
            file.remove_source_after_save,
            &mut on_progress,
        )
        .map_err(FileAttachmentError::WriteFailed)?;
    }
    Ok(())
}

fn quote_file_attachment_path(path: &str) -> String {
    let escaped = path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

pub(crate) fn format_file_attachment_prompt(paths: &[String]) -> String {
    match paths {
        [] => String::new(),
        [path] => format!("File: {}", quote_file_attachment_path(path)),
        _ => format!(
            "Files: [{}]",
            paths
                .iter()
                .map(|path| quote_file_attachment_path(path))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn normalize_attachment_operation_id(
    operation_id: Option<String>,
    image_paste_sequence: &std::sync::atomic::AtomicU64,
) -> String {
    operation_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            format!(
                "attachment-{}",
                image_paste_unique_token(image_paste_sequence)
            )
        })
}

fn display_attachment_basename(filename: &str) -> String {
    filename
        .trim()
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .unwrap_or("file")
        .to_string()
}

fn display_name_for_file_attachment(file: &gwt::FileAttachment) -> String {
    match file {
        gwt::FileAttachment::NativePath { path } => display_attachment_basename(path),
        gwt::FileAttachment::Inline { filename, .. }
        | gwt::FileAttachment::Uploaded { filename, .. } => display_attachment_basename(filename),
    }
}

/// The actual admitted producer, retained unchanged across staging and input.
#[derive(Debug, Clone)]
pub(crate) struct AttachmentOperation {
    pub(crate) client_id: ClientId,
    pub(crate) id: String,
    pub(crate) operation_id: String,
    pub(crate) expected_target: gwt::protocol::AttachmentExpectedTarget,
    pub(crate) context: super::ProjectContext,
    pub(crate) connection_id: u64,
    pub(crate) identity: gwt::daemon_subscriber::ProjectOwnerIdentity,
    pub(crate) owner_pane_id: String,
    pub(crate) file_count: usize,
    pub(crate) filename: Option<String>,
}

impl AttachmentOperation {
    pub(super) fn progress(
        &self,
        phase: AttachmentProgressPhase,
        message: Option<&str>,
    ) -> OutboundEvent {
        let mut update =
            AttachmentProgressUpdate::new(&self.id, &self.operation_id, phase, self.file_count)
                .target(Some(self.expected_target.clone()))
                .filename(self.filename.clone());
        if let Some(message) = message {
            update = update.message(message);
        }
        update.outbound(self.client_id.clone())
    }
}

const TARGET_UNAVAILABLE: &str =
    "Attachment target unavailable. Reload the current UI and try again.";
const LEGACY_RELOAD: &str =
    "Reload the current UI before attaching files. This attachment was not sent.";

#[derive(Debug, Clone)]
struct AttachmentProgressUpdate {
    id: String,
    operation_id: String,
    phase: AttachmentProgressPhase,
    expected_target: Option<gwt::protocol::AttachmentExpectedTarget>,
    file_index: Option<usize>,
    file_count: usize,
    filename: Option<String>,
    bytes_done: Option<u64>,
    bytes_total: Option<u64>,
    message: Option<String>,
}

impl AttachmentProgressUpdate {
    fn new(
        id: impl Into<String>,
        operation_id: impl Into<String>,
        phase: AttachmentProgressPhase,
        file_count: usize,
    ) -> Self {
        Self {
            id: id.into(),
            operation_id: operation_id.into(),
            phase,
            expected_target: None,
            file_index: None,
            file_count,
            filename: None,
            bytes_done: None,
            bytes_total: None,
            message: None,
        }
    }

    fn target(mut self, target: Option<gwt::protocol::AttachmentExpectedTarget>) -> Self {
        self.expected_target = target;
        self
    }

    fn filename(mut self, filename: Option<String>) -> Self {
        self.filename = filename;
        self
    }

    fn file(mut self, index: usize, filename: String) -> Self {
        self.file_index = Some(index);
        self.filename = Some(filename);
        self
    }

    fn bytes(mut self, bytes_done: u64, bytes_total: Option<u64>) -> Self {
        self.bytes_done = Some(bytes_done);
        self.bytes_total = bytes_total;
        self
    }

    fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    fn outbound(self, client_id: ClientId) -> OutboundEvent {
        OutboundEvent::reply(
            client_id,
            BackendEvent::AttachmentProgress {
                id: self.id,
                operation_id: self.operation_id,
                phase: self.phase,
                expected_target: self.expected_target,
                file_index: self.file_index,
                file_count: self.file_count,
                filename: self.filename,
                bytes_done: self.bytes_done,
                bytes_total: self.bytes_total,
                message: self.message,
            },
        )
    }

    fn dispatch(self, proxy: &AppEventProxy, client_id: &ClientId) {
        proxy.send(UserEvent::Dispatch(vec![self.outbound(client_id.clone())]));
    }
}

pub(super) struct UploadedImagePasteOperation {
    pub(super) upload_id: String,
    pub(super) mime_type: String,
    pub(super) filename: Option<String>,
    pub(super) size: u64,
}

#[cfg(test)]
pub(crate) fn prepare_image_paste_file(
    worktree_path: &Path,
    agent_project_root: &str,
    data_base64: &str,
    mime_type: &str,
    filename: Option<&str>,
    unique_token: &str,
) -> Result<ImagePasteFile, ImagePasteError> {
    let extension = image_extension_for_mime(mime_type)
        .ok_or_else(|| ImagePasteError::UnsupportedMimeType(mime_type.to_string()))?;
    if data_base64.trim().is_empty() {
        return Err(ImagePasteError::EmptyPayload);
    }
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        data_base64.trim(),
    )
    .map_err(|error| ImagePasteError::InvalidBase64(error.to_string()))?;
    if bytes.is_empty() {
        return Err(ImagePasteError::EmptyPayload);
    }

    let stem = sanitize_image_paste_stem(filename);
    let file_name = format!("{unique_token}-{stem}.{extension}");
    let storage_path = worktree_path
        .join(".gwt")
        .join("drop-files")
        .join(&file_name);
    let relative_path = format!("{FILE_ATTACHMENT_RELATIVE_DIR}/{file_name}");
    let _ = agent_project_root;
    let agent_path = relative_path;
    let prompt_text = format!("{IMAGE_PASTE_PROMPT_PREFIX}{agent_path}");

    Ok(ImagePasteFile {
        bytes: Some(bytes),
        source_path: None,
        remove_source_after_save: false,
        storage_path,
        agent_path,
        prompt_text,
    })
}

pub(crate) fn prepare_uploaded_image_paste_file(
    worktree_path: &Path,
    upload_store: &AttachmentUploadStore,
    upload_id: &str,
    mime_type: &str,
    filename: Option<&str>,
    declared_size: u64,
    unique_token: &str,
) -> Result<ImagePasteFile, ImagePasteError> {
    let extension = image_extension_for_mime(mime_type)
        .ok_or_else(|| ImagePasteError::UnsupportedMimeType(mime_type.to_string()))?;
    let uploaded = upload_store
        .take(upload_id)
        .map_err(ImagePasteError::WriteFailed)?
        .ok_or_else(|| {
            ImagePasteError::WriteFailed(format!("uploaded image missing: {upload_id}"))
        })?;
    if uploaded.size == 0 || declared_size == 0 {
        return Err(ImagePasteError::EmptyPayload);
    }
    let stem = sanitize_image_paste_stem(filename.or(Some(uploaded.filename.as_str())));
    let file_name = format!("{unique_token}-{stem}.{extension}");
    let storage_path = worktree_path
        .join(".gwt")
        .join("drop-files")
        .join(&file_name);
    let relative_path = format!("{FILE_ATTACHMENT_RELATIVE_DIR}/{file_name}");
    let agent_path = relative_path;
    let prompt_text = format!("{IMAGE_PASTE_PROMPT_PREFIX}{agent_path}");

    Ok(ImagePasteFile {
        bytes: None,
        source_path: Some(uploaded.path),
        remove_source_after_save: true,
        storage_path,
        agent_path,
        prompt_text,
    })
}

fn image_paste_unique_token(image_paste_sequence: &std::sync::atomic::AtomicU64) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let sequence = image_paste_sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{millis}-{sequence}")
}

fn write_attachment_bytes_with_progress(
    storage_path: &Path,
    bytes: &[u8],
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let Some(parent) = storage_path.parent() else {
        return Err("attachment path has no parent directory".to_string());
    };
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let total = bytes.len() as u64;
    on_progress(0, Some(total));
    std::fs::write(storage_path, bytes).map_err(|error| error.to_string())?;
    on_progress(total, Some(total));
    Ok(())
}

fn copy_attachment_file_with_progress(
    source_path: &Path,
    storage_path: &Path,
    remove_source_after_save: bool,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let Some(parent) = storage_path.parent() else {
        return Err("attachment path has no parent directory".to_string());
    };
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let total = std::fs::metadata(source_path)
        .ok()
        .map(|metadata| metadata.len());
    on_progress(0, total);
    let mut source = std::fs::File::open(source_path).map_err(|error| error.to_string())?;
    let mut destination = std::fs::File::create(storage_path).map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let read =
            std::io::Read::read(&mut source, &mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        if let Err(error) = destination.write_all(&buffer[..read]) {
            let _ = std::fs::remove_file(storage_path);
            return Err(error.to_string());
        }
        copied += read as u64;
        on_progress(copied, total);
    }
    destination.flush().map_err(|error| error.to_string())?;
    if remove_source_after_save {
        let _ = std::fs::remove_file(source_path);
    }
    Ok(())
}

fn save_image_paste_file_with_progress(
    image: &ImagePasteFile,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), ImagePasteError> {
    if let Some(bytes) = image.bytes.as_ref() {
        return write_attachment_bytes_with_progress(&image.storage_path, bytes, &mut on_progress)
            .map_err(|error| {
                ImagePasteError::WriteFailed(format!("{}: {error}", image.storage_path.display()))
            });
    }
    if let Some(source_path) = image.source_path.as_ref() {
        return copy_attachment_file_with_progress(
            source_path,
            &image.storage_path,
            image.remove_source_after_save,
            &mut on_progress,
        )
        .map_err(|error| {
            ImagePasteError::WriteFailed(format!("{}: {error}", image.storage_path.display()))
        });
    }
    Err(ImagePasteError::EmptyPayload)
}

fn record_attachment_failure(
    target: gwt_core::error_ledger::ErrorTarget,
    operation_id: &str,
    message: &str,
) {
    use gwt_core::error_ledger::{self, ErrorKind, ErrorRecord};
    let record = ErrorRecord::new(ErrorKind::AttachmentFailure, message, target).with_context(
        std::collections::BTreeMap::from([("operation_id".to_string(), operation_id.to_string())]),
    );
    if let Err(error) = error_ledger::record(record) {
        tracing::warn!(%error, "failed to record attachment failure");
    }
}

fn image_attachment_prompt(
    image: &ImagePasteFile,
    worktree_path: &Path,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
) -> String {
    // Host PM providers run from the instruction runtime, not their worktree.
    if runtime_target == gwt_agent::LaunchRuntimeTarget::Host
        && gwt::pm_registry::pm_runtime_dir_for_pm_worktree(worktree_path).is_some()
    {
        format!(
            "{IMAGE_PASTE_PROMPT_PREFIX}{}",
            image.storage_path.display()
        )
    } else {
        image.prompt_text.clone()
    }
}

impl AppRuntime {
    /// A refusal only: echo untrusted correlation without resolving/staging a target.
    pub(crate) fn rejected_attachment_request(
        &self,
        client_id: &ClientId,
        event: &gwt::FrontendEvent,
    ) -> Option<Vec<OutboundEvent>> {
        let (id, operation_id, target, count) = match event {
            gwt::FrontendEvent::AttachFiles {
                id,
                operation_id,
                expected_target,
                files,
            } => (
                id,
                operation_id.clone(),
                expected_target.clone(),
                files.len(),
            ),
            gwt::FrontendEvent::PasteImageUploaded {
                id,
                operation_id,
                expected_target,
                ..
            } => (id, operation_id.clone(), expected_target.clone(), 1),
            gwt::FrontendEvent::PasteImage { id, .. } => (id, None, None, 1),
            _ => return None,
        };
        Some(vec![AttachmentProgressUpdate::new(
            id,
            normalize_attachment_operation_id(operation_id, &self.image_paste_sequence),
            AttachmentProgressPhase::Failed,
            count,
        )
        .target(target)
        .message(TARGET_UNAVAILABLE)
        .outbound(client_id.clone())])
    }

    fn capture_attachment_operation(
        &self,
        client_id: ClientId,
        id: String,
        operation_id: Option<String>,
        expected_target: Option<gwt::protocol::AttachmentExpectedTarget>,
        file_count: usize,
        filename: Option<String>,
    ) -> Result<AttachmentOperation, Box<OutboundEvent>> {
        let legacy = operation_id.as_ref().is_none_or(|op| op.trim().is_empty())
            || expected_target.is_none();
        let operation_id =
            normalize_attachment_operation_id(operation_id, &self.image_paste_sequence);
        let refused = || {
            AttachmentProgressUpdate::new(
                &id,
                &operation_id,
                AttachmentProgressPhase::Failed,
                file_count,
            )
            .target(expected_target.clone())
            .filename(filename.clone())
            .message(if legacy {
                LEGACY_RELOAD
            } else {
                TARGET_UNAVAILABLE
            })
            .outbound(client_id.clone())
        };
        if legacy || file_count == 0 {
            return Err(Box::new(refused()));
        }
        let target = expected_target.as_ref().expect("nonlegacy target");
        let Some(context) = self
            .window_lookup
            .get(&id)
            .and_then(|a| self.project_context(&a.tab_id))
        else {
            return Err(Box::new(refused()));
        };
        let Some(state) = self.project_state(&context) else {
            return Err(Box::new(refused()));
        };
        let Some(runtime) = self.runtimes.get(&id) else {
            return Err(Box::new(refused()));
        };
        let Some(session) = self.active_agent_sessions.get(&id) else {
            return Err(Box::new(refused()));
        };
        if target.project_key != context.project_key.to_string()
            || target.session_id.is_empty()
            || target.session_id != session.session_id
            || target.runtime_incarnation == 0
            || target.runtime_incarnation > 9_007_199_254_740_991
            || target.runtime_incarnation != runtime.incarnation
            || target.host_id
                != format!(
                    "{}:{}",
                    runtime.owner_identity.pid, runtime.owner_identity.host_started_at
                )
            || gwt::process::host_process_start_time(runtime.owner_identity.pid)
                != Some(runtime.owner_identity.host_started_at)
            || runtime.owner_identity.host_started_at == 0
            || runtime.owner_pane_id.is_empty()
            || state.project_owner_identity.as_ref() != Some(&runtime.owner_identity)
            || runtime.client.identity() != Some(&runtime.owner_identity)
        {
            return Err(Box::new(refused()));
        }
        Ok(AttachmentOperation {
            client_id,
            id,
            operation_id,
            expected_target: expected_target.unwrap(),
            context,
            connection_id: state.project_owner_connection_id,
            identity: runtime.owner_identity.clone(),
            owner_pane_id: runtime.owner_pane_id.clone(),
            file_count,
            filename,
        })
    }

    pub(super) fn attachment_operation_is_current(&self, operation: &AttachmentOperation) -> bool {
        self.project_owner_event_is_current(
            &operation.context,
            operation.connection_id,
            &operation.identity,
        ) && self.remote_runtime_is_current(
            &operation.context,
            &operation.identity,
            &operation.id,
            operation.expected_target.runtime_incarnation,
        ) && self
            .runtimes
            .get(&operation.id)
            .is_some_and(|runtime| runtime.owner_pane_id == operation.owner_pane_id)
            && self
                .active_agent_sessions
                .get(&operation.id)
                .is_some_and(|session| session.session_id == operation.expected_target.session_id)
    }

    fn attachment_error_target(&self, id: &str) -> Option<gwt_core::error_ledger::ErrorTarget> {
        let address = self.window_lookup.get(id)?;
        let tab = self.tab(&address.tab_id)?;
        Some(gwt_core::error_ledger::ErrorTarget {
            window_id: Some(id.to_string()),
            project_root: Some(tab.project_root.to_string_lossy().into_owned()),
            session_id: self
                .active_agent_sessions
                .get(id)
                .map(|session| session.session_id.clone()),
            ..Default::default()
        })
    }

    pub(crate) fn attachment_upload_failed_events(
        &self,
        id: &str,
        operation_id: &str,
        message: &str,
    ) -> Vec<OutboundEvent> {
        if let Some(target) = self.attachment_error_target(id) {
            record_attachment_failure(target, operation_id, message);
        }
        Vec::new()
    }

    pub(crate) fn inject_attachment_prompt_events(
        &mut self,
        operation: AttachmentOperation,
        prompt: String,
    ) -> Vec<OutboundEvent> {
        if !self.attachment_operation_is_current(&operation) {
            return vec![
                operation.progress(AttachmentProgressPhase::Failed, Some(TARGET_UNAVAILABLE))
            ];
        }
        let mut events = vec![operation.progress(AttachmentProgressPhase::Injecting, None)];
        let id = operation.id.clone();
        let command = gwt::project_runtime::events::OwnerCommand::AttachmentInput {
            id: operation.owner_pane_id.clone(),
            incarnation: operation.expected_target.runtime_incarnation,
            session_id: operation.expected_target.session_id.clone(),
            data: prompt.into_bytes(),
        };
        events.extend(self.queue_remote_runtime_command(
            &id,
            command,
            super::RemoteRuntimeCommandKind::AttachmentInput(Box::new(operation)),
        ));
        events
    }

    pub(super) fn paste_image_uploaded_operation_events(
        &mut self,
        client_id: ClientId,
        id: String,
        operation_id: Option<String>,
        expected_target: Option<gwt::protocol::AttachmentExpectedTarget>,
        upload: UploadedImagePasteOperation,
    ) -> Vec<OutboundEvent> {
        let display_filename = upload
            .filename
            .as_deref()
            .map(display_attachment_basename)
            .or_else(|| Some("image".to_string()));
        let operation = match self.capture_attachment_operation(
            client_id.clone(),
            id.clone(),
            operation_id,
            expected_target,
            1,
            display_filename.clone(),
        ) {
            Ok(operation) => operation,
            Err(event) => return vec![*event],
        };
        let operation_id = operation.operation_id.clone();
        let expected_target = Some(operation.expected_target.clone());
        let session = self.active_agent_sessions.get(&id).expect("admitted Agent");
        let worktree_path = session.worktree_path.clone();
        let runtime_target = session.runtime_target;
        let upload_store = self.attachment_uploads.clone();
        let proxy = self.proxy.clone();
        let spawner = self.blocking_tasks.clone();
        let worker_client_id = client_id.clone();
        let worker_window_id = id.clone();
        let worker_operation_id = operation_id.clone();
        let worker_filename = display_filename.clone();
        let unique_token = image_paste_unique_token(&self.image_paste_sequence);
        let error_target = self
            .attachment_error_target(&id)
            .expect("resolved attachment window");

        let worker_target = expected_target.clone();
        let failure_operation = operation.clone();
        if spawner
            .try_spawn(move || {
                let image = match prepare_uploaded_image_paste_file(
                    &worktree_path,
                    &upload_store,
                    &upload.upload_id,
                    &upload.mime_type,
                    upload.filename.as_deref(),
                    upload.size,
                    &unique_token,
                ) {
                    Ok(image) => image,
                    Err(error) => {
                        record_attachment_failure(
                            error_target,
                            &worker_operation_id,
                            &error.to_string(),
                        );
                        AttachmentProgressUpdate::new(
                            worker_window_id.clone(),
                            worker_operation_id.clone(),
                            AttachmentProgressPhase::Failed,
                            1,
                        )
                        .target(worker_target.clone())
                        .file(
                            0,
                            worker_filename
                                .clone()
                                .unwrap_or_else(|| "image".to_string()),
                        )
                        .message(error.to_string())
                        .dispatch(&proxy, &worker_client_id);
                        return;
                    }
                };
                let progress_filename = worker_filename
                    .clone()
                    .or_else(|| Some(display_attachment_basename(&image.agent_path)));
                if let Err(error) =
                    save_image_paste_file_with_progress(&image, |bytes_done, total| {
                        AttachmentProgressUpdate::new(
                            worker_window_id.clone(),
                            worker_operation_id.clone(),
                            AttachmentProgressPhase::Staging,
                            1,
                        )
                        .target(worker_target.clone())
                        .file(
                            0,
                            progress_filename
                                .clone()
                                .unwrap_or_else(|| "image".to_string()),
                        )
                        .bytes(bytes_done, total)
                        .dispatch(&proxy, &worker_client_id);
                    })
                {
                    record_attachment_failure(
                        error_target,
                        &worker_operation_id,
                        &error.to_string(),
                    );
                    AttachmentProgressUpdate::new(
                        worker_window_id.clone(),
                        worker_operation_id.clone(),
                        AttachmentProgressPhase::Failed,
                        1,
                    )
                    .target(worker_target.clone())
                    .filename(progress_filename)
                    .message(error.to_string())
                    .dispatch(&proxy, &worker_client_id);
                    return;
                }
                let prompt = image_attachment_prompt(&image, &worktree_path, runtime_target);
                proxy.send(UserEvent::AttachmentPromptReady {
                    operation: Box::new(operation),
                    prompt,
                });
            })
            .is_err()
        {
            return vec![failure_operation.progress(
                AttachmentProgressPhase::Failed,
                Some("Attachment preparation could not start. Reload the current UI."),
            )];
        }

        vec![
            AttachmentProgressUpdate::new(id, operation_id, AttachmentProgressPhase::Queued, 1)
                .target(expected_target.clone())
                .filename(display_filename)
                .outbound(client_id),
        ]
    }

    pub(super) fn attach_files_operation_events(
        &mut self,
        client_id: ClientId,
        id: String,
        operation_id: Option<String>,
        expected_target: Option<gwt::protocol::AttachmentExpectedTarget>,
        files: Vec<gwt::FileAttachment>,
    ) -> Vec<OutboundEvent> {
        let file_count = files.len();
        let display_filename =
            (file_count == 1).then(|| display_name_for_file_attachment(&files[0]));
        let operation = match self.capture_attachment_operation(
            client_id.clone(),
            id.clone(),
            operation_id,
            expected_target,
            file_count,
            display_filename.clone(),
        ) {
            Ok(operation) => operation,
            Err(event) => return vec![*event],
        };
        let operation_id = operation.operation_id.clone();
        let expected_target = Some(operation.expected_target.clone());
        let session = self.active_agent_sessions.get(&id).expect("admitted Agent");
        let worktree_path = session.worktree_path.clone();
        let agent_project_root = session.agent_project_root.clone();
        let runtime_target = session.runtime_target;
        let upload_store = self.attachment_uploads.clone();
        let limits = ContentLimits::default();
        let proxy = self.proxy.clone();
        let spawner = self.blocking_tasks.clone();
        let worker_client_id = client_id.clone();
        let worker_window_id = id.clone();
        let worker_operation_id = operation_id.clone();
        let worker_display_filename = display_filename.clone();
        let file_tokens: Vec<String> = (0..files.len())
            .map(|index| {
                format!(
                    "{}-{index}",
                    image_paste_unique_token(&self.image_paste_sequence)
                )
            })
            .collect();

        let worker_target = expected_target.clone();
        let failure_operation = operation.clone();
        if spawner
            .try_spawn(move || {
                let mut agent_paths = Vec::with_capacity(files.len());
                for (index, file) in files.iter().enumerate() {
                    let filename = display_name_for_file_attachment(file);
                    let token = file_tokens[index].clone();
                    let prepared = match prepare_file_attachment(
                        &worktree_path,
                        &agent_project_root,
                        runtime_target,
                        file,
                        &token,
                        limits,
                        &upload_store,
                    ) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            AttachmentProgressUpdate::new(
                                worker_window_id.clone(),
                                worker_operation_id.clone(),
                                AttachmentProgressPhase::Failed,
                                file_count,
                            )
                            .target(worker_target.clone())
                            .file(index, filename)
                            .message(error.to_string())
                            .dispatch(&proxy, &worker_client_id);
                            return;
                        }
                    };
                    if let Err(error) =
                        save_file_attachment_with_progress(&prepared, |bytes_done, total| {
                            AttachmentProgressUpdate::new(
                                worker_window_id.clone(),
                                worker_operation_id.clone(),
                                AttachmentProgressPhase::Staging,
                                file_count,
                            )
                            .target(worker_target.clone())
                            .file(index, filename.clone())
                            .bytes(bytes_done, total)
                            .dispatch(&proxy, &worker_client_id);
                        })
                    {
                        AttachmentProgressUpdate::new(
                            worker_window_id.clone(),
                            worker_operation_id.clone(),
                            AttachmentProgressPhase::Failed,
                            file_count,
                        )
                        .target(worker_target.clone())
                        .file(index, filename)
                        .message(error.to_string())
                        .dispatch(&proxy, &worker_client_id);
                        return;
                    }
                    agent_paths.push(prepared.agent_path);
                }

                let prompt = format_file_attachment_prompt(&agent_paths);
                if prompt.is_empty() {
                    AttachmentProgressUpdate::new(
                        worker_window_id.clone(),
                        worker_operation_id.clone(),
                        AttachmentProgressPhase::Failed,
                        file_count,
                    )
                    .target(worker_target.clone())
                    .filename(worker_display_filename.clone())
                    .message("no attachment prompt generated")
                    .dispatch(&proxy, &worker_client_id);
                    return;
                }
                proxy.send(UserEvent::AttachmentPromptReady {
                    operation: Box::new(operation),
                    prompt,
                });
            })
            .is_err()
        {
            return vec![failure_operation.progress(
                AttachmentProgressPhase::Failed,
                Some("Attachment preparation could not start. Reload the current UI."),
            )];
        }

        vec![AttachmentProgressUpdate::new(
            id,
            operation_id,
            AttachmentProgressPhase::Queued,
            file_count,
        )
        .target(expected_target.clone())
        .filename(display_filename)
        .outbound(client_id)]
    }
}
