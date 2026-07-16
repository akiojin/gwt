//! Container-local Codex bridge sidecar control transport.
//!
//! The sidecar and remote TUI are separate attached `compose exec` processes
//! in the same selected service. The WebSocket listener remains container
//! loopback-only; durability crosses the already-attached stdin/stdout control
//! pipe as checksummed, versioned NDJSON with per-request acknowledgements.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

use base64::Engine as _;
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    attachment_encoded_footprint, decode_bounded_base64_with_limit,
    max_raw_bytes_for_base64_capacity, open_prepared_local_attachment, parse_image_data_url,
    projected_base64_len, projected_decoded_base64_len, read_prepared_local_attachment,
    register_codex_bridge_route, CodexAppServerLaunch, CodexBridgeFailure, CodexBridgeFailureKind,
    CodexBridgeRouteConfig, CodexBridgeRouteLease, CodexDurabilitySink, PreparedLocalAttachment,
    RecoveryAttachmentAggregateBudget, RootThreadBinding, TransferredAttachment, UserInputCapture,
    VisibleDiscussionCapture, CODEX_REMOTE_AUTH_TOKEN_ENV, MAX_RECOVERY_ATTACHMENT_BYTES,
    MAX_RECOVERY_ATTACHMENT_CONTROL_BYTES,
};

const CONTROL_VERSION: u32 = 1;
const CONTROL_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const CONTROL_ACK_RETRIES: usize = 2;
const CONTROL_READY_TIMEOUT: Duration = Duration::from_secs(20);
const CONTROL_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const MAX_HEARTBEAT_MISSES: usize = 3;
const MAX_CONTROL_FRAME_BYTES: usize = 40 * 1024 * 1024;
const CONTROL_FRAME_METADATA_RESERVE_BYTES: usize = 4 * 1024 * 1024;
const _: () = assert!(
    MAX_RECOVERY_ATTACHMENT_CONTROL_BYTES
        == MAX_CONTROL_FRAME_BYTES - CONTROL_FRAME_METADATA_RESERVE_BYTES
);
const MAX_ACK_CACHE_ENTRIES: usize = 4096;
const SIDECAR_COMMAND: &[&str] = &["/usr/local/bin/gwtd", "__internal", "codex-sidecar"];
const RECOVERY_STAGING_LEASE_FILE: &str = ".lease.lock";
const RECOVERY_STAGING_INVENTORY_LOCK: &str = "/tmp/gwt-codex-recovery-inventory.lock";
const CONTAINER_RECOVERY_STAGING_PREFIX: &str = "/tmp/gwt-codex-recovery-";
// `/tmp` is shared with unrelated processes. These generous bounds keep the
// orphan sweep finite without truncating or deleting from an incomplete scan.
const MAX_RECOVERY_STAGING_PARENT_ENTRIES: usize = 16_384;
const MAX_RECOVERY_STAGING_CHILD_ENTRIES: usize = 256;
const MAX_RECOVERY_STAGING_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct ControlFrame {
    version: u32,
    sequence: u64,
    checksum: String,
    payload: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostControlPayload {
    Init {
        app_server: Box<CodexAppServerLaunch>,
        expected_resume_id: Option<String>,
        recovery_attachments: Option<ContainerRecoveryAttachmentBundle>,
    },
    Ack {
        request_sequence: u64,
        accepted: bool,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SidecarControlPayload {
    Ready {
        endpoint: String,
        auth_token: String,
        recovery_attachment_paths: Vec<String>,
    },
    PersistRoot {
        binding: RootThreadBinding,
        wire_text: String,
    },
    PersistInput {
        input: UserInputCapture,
        attachments: Vec<TransferredAttachment>,
        wire_text: String,
    },
    PersistVisible {
        capture: VisibleDiscussionCapture,
        wire_text: String,
    },
    ProviderReady,
    Failure {
        failure: CodexBridgeFailure,
    },
    Heartbeat,
}

#[derive(Clone, Serialize, Deserialize)]
struct ContainerRecoveryAttachmentTransfer {
    reference: gwt_core::recovery::RecoveryAttachmentRef,
    base64_data: String,
}

impl fmt::Debug for ContainerRecoveryAttachmentTransfer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContainerRecoveryAttachmentTransfer")
            .field("reference", &self.reference)
            .field("base64_data", &"[REDACTED]")
            .finish()
    }
}

/// Host-verified recovery blobs carried to the container only over the
/// attached private sidecar pipe.
#[derive(Serialize, Deserialize)]
pub struct ContainerRecoveryAttachmentBundle {
    staging_id: String,
    attachments: Vec<ContainerRecoveryAttachmentTransfer>,
}

impl fmt::Debug for ContainerRecoveryAttachmentBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContainerRecoveryAttachmentBundle")
            .field("staging_id", &self.staging_id)
            .field("attachment_count", &self.attachments.len())
            .field("attachment_bytes", &"[REDACTED]")
            .finish()
    }
}

impl ContainerRecoveryAttachmentBundle {
    /// Deterministic container paths paired in durable checkpoint order.
    /// Sidecar startup recomputes and confirms this exact manifest after
    /// digest-verifying every staged file.
    pub fn container_paths(&self) -> Result<Vec<String>, String> {
        validate_staging_id(&self.staging_id)?;
        if self.attachments.len() > super::MAX_RECOVERY_ATTACHMENT_COUNT {
            return Err(format!(
                "Codex recovery attachment count exceeds the {} item safety bound",
                super::MAX_RECOVERY_ATTACHMENT_COUNT
            ));
        }
        self.attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                container_recovery_attachment_path(&self.staging_id, index, &attachment.reference)
            })
            .collect()
    }
}

/// Resolve and checksum-verify durable Host blobs, then frame only their
/// metadata and bytes for the private container sidecar channel. Host paths
/// are deliberately absent from the returned value and its Debug output.
pub fn prepare_container_recovery_attachments(
    store: &gwt_core::recovery::RecoveryStore,
    references: &[gwt_core::recovery::RecoveryAttachmentRef],
) -> Result<Option<ContainerRecoveryAttachmentBundle>, String> {
    if references.is_empty() {
        return Ok(None);
    }
    let mut budget = RecoveryAttachmentAggregateBudget::default();
    budget.reserve_count(references.len())?;
    for reference in references {
        validate_container_recovery_reference(reference)?;
        let raw_bytes = usize::try_from(reference.byte_len)
            .map_err(|_| "Codex recovery attachment bundle exceeds the safety bound".to_string())?;
        let encoded_bytes = projected_base64_len(raw_bytes)?
            .checked_add(reference.content_id.len())
            .and_then(|bytes| bytes.checked_add(reference.file_name.len()))
            .ok_or_else(|| {
                "Codex recovery attachment bundle exceeds the safety bound".to_string()
            })?;
        budget.reserve_projected(raw_bytes, encoded_bytes)?;
    }

    let mut attachments = Vec::with_capacity(references.len());
    for reference in references {
        let metadata_bytes = reference
            .content_id
            .len()
            .checked_add(reference.file_name.len())
            .ok_or_else(|| {
                "Codex recovery attachment bundle exceeds the safety bound".to_string()
            })?;
        let encoded_capacity = budget
            .remaining_actual_encoded()
            .checked_sub(metadata_bytes)
            .ok_or_else(|| {
                "Codex recovery attachment bundle exceeds the safety bound".to_string()
            })?;
        let remaining_raw = budget
            .remaining_actual_raw()
            .min(max_raw_bytes_for_base64_capacity(encoded_capacity))
            .min(MAX_RECOVERY_ATTACHMENT_BYTES);
        let bytes = store
            .read_attachment_bytes(reference, remaining_raw as u64)
            .map_err(|error| format!("read durable recovery attachment: {error}"))?;
        let encoded_bytes = projected_base64_len(bytes.len())?
            .checked_add(reference.content_id.len())
            .and_then(|bytes| bytes.checked_add(reference.file_name.len()))
            .ok_or_else(|| {
                "Codex recovery attachment bundle exceeds the safety bound".to_string()
            })?;
        budget.consume_actual(bytes.len(), encoded_bytes)?;
        attachments.push(ContainerRecoveryAttachmentTransfer {
            reference: reference.clone(),
            base64_data: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    let bundle = ContainerRecoveryAttachmentBundle {
        staging_id: uuid::Uuid::new_v4().simple().to_string(),
        attachments,
    };
    Ok(Some(bundle))
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json(&object[key]));
            }
            Value::Object(canonical)
        }
        value => value.clone(),
    }
}

fn control_checksum(version: u32, sequence: u64, payload: &Value) -> Result<String, String> {
    let canonical = canonical_json(payload);
    let bytes = serde_json::to_vec(&(version, sequence, canonical))
        .map_err(|error| format!("serialize Codex sidecar checksum: {error}"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn encode_control_frame<T: Serialize>(sequence: u64, payload: &T) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_value(payload)
        .map_err(|error| format!("serialize Codex sidecar payload: {error}"))?;
    let frame = ControlFrame {
        version: CONTROL_VERSION,
        sequence,
        checksum: control_checksum(CONTROL_VERSION, sequence, &payload)?,
        payload,
    };
    let mut encoded = serde_json::to_vec(&frame)
        .map_err(|error| format!("serialize Codex sidecar frame: {error}"))?;
    if encoded.len() > MAX_CONTROL_FRAME_BYTES {
        return Err("Codex sidecar control frame exceeds the safety bound".to_string());
    }
    encoded.push(b'\n');
    Ok(encoded)
}

fn decode_control_frame<T: DeserializeOwned>(line: &[u8]) -> Result<(u64, String, T), String> {
    let frame: ControlFrame = serde_json::from_slice(line)
        .map_err(|error| format!("parse Codex sidecar control frame: {error}"))?;
    if frame.version != CONTROL_VERSION {
        return Err("Codex sidecar control protocol version mismatch".to_string());
    }
    let checksum = control_checksum(frame.version, frame.sequence, &frame.payload)?;
    if checksum != frame.checksum {
        return Err("Codex sidecar control checksum mismatch".to_string());
    }
    let payload = serde_json::from_value(frame.payload)
        .map_err(|error| format!("parse Codex sidecar control payload: {error}"))?;
    Ok((frame.sequence, frame.checksum, payload))
}

fn read_bounded_control_line<R: BufRead>(
    reader: &mut R,
    destination: &mut Vec<u8>,
) -> Result<Option<()>, String> {
    destination.clear();
    loop {
        let (consumed, done) = {
            let available = reader
                .fill_buf()
                .map_err(|error| format!("read Codex sidecar control channel: {error}"))?;
            if available.is_empty() {
                if destination.is_empty() {
                    return Ok(None);
                }
                (0, true)
            } else if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
                if destination.len() + index > MAX_CONTROL_FRAME_BYTES {
                    return Err("Codex sidecar control frame exceeds the safety bound".to_string());
                }
                destination.extend_from_slice(&available[..index]);
                (index + 1, true)
            } else {
                if destination.len() + available.len() > MAX_CONTROL_FRAME_BYTES {
                    return Err("Codex sidecar control frame exceeds the safety bound".to_string());
                }
                destination.extend_from_slice(available);
                (available.len(), false)
            }
        };
        reader.consume(consumed);
        if done {
            if destination.last() == Some(&b'\r') {
                destination.pop();
            }
            return Ok(Some(()));
        }
    }
}

fn validate_staging_id(staging_id: &str) -> Result<(), String> {
    if staging_id.len() == 32
        && staging_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("Codex recovery attachment staging identity is invalid".to_string())
    }
}

fn validate_container_recovery_reference(
    reference: &gwt_core::recovery::RecoveryAttachmentRef,
) -> Result<&str, String> {
    let digest = reference
        .content_id
        .strip_prefix("sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| "Codex recovery attachment content identity is invalid".to_string())?;
    let file_name = reference.file_name.trim();
    if file_name.is_empty()
        || file_name != reference.file_name
        || file_name.contains(['/', '\\', '\n', '\r', '\t'])
        || matches!(file_name, "." | "..")
        || Path::new(file_name)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(file_name)
    {
        return Err("Codex recovery attachment file name is invalid".to_string());
    }
    if reference.byte_len > MAX_RECOVERY_ATTACHMENT_BYTES as u64 {
        return Err("Codex attachment exceeds the recovery safety bound".to_string());
    }
    Ok(digest)
}

fn container_recovery_attachment_path(
    staging_id: &str,
    index: usize,
    reference: &gwt_core::recovery::RecoveryAttachmentRef,
) -> Result<String, String> {
    validate_staging_id(staging_id)?;
    let digest = validate_container_recovery_reference(reference)?;
    Ok(format!(
        "{CONTAINER_RECOVERY_STAGING_PREFIX}{staging_id}/{index:03}-{digest}"
    ))
}

fn create_private_staging_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

fn open_private_staging_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

struct ContainerRecoveryAttachmentStaging {
    directory: Option<PathBuf>,
    lease: Option<fs::File>,
    files: Vec<PathBuf>,
    paths: Vec<String>,
}

#[cfg(test)]
fn decode_verified_container_recovery_attachment(
    attachment: &ContainerRecoveryAttachmentTransfer,
) -> Result<Vec<u8>, String> {
    decode_verified_container_recovery_attachment_with_limit(
        attachment,
        MAX_RECOVERY_ATTACHMENT_BYTES,
    )
}

fn decode_verified_container_recovery_attachment_with_limit(
    attachment: &ContainerRecoveryAttachmentTransfer,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let digest = validate_container_recovery_reference(&attachment.reference)?;
    let bytes = decode_bounded_base64_with_limit(&attachment.base64_data, max_bytes)?;
    if bytes.len() as u64 != attachment.reference.byte_len
        || hex::encode(Sha256::digest(&bytes)) != digest
    {
        return Err("Codex recovery attachment failed sidecar verification".to_string());
    }
    Ok(bytes)
}

impl ContainerRecoveryAttachmentStaging {
    fn empty() -> Self {
        Self {
            directory: None,
            lease: None,
            files: Vec::new(),
            paths: Vec::new(),
        }
    }
}

impl Drop for ContainerRecoveryAttachmentStaging {
    fn drop(&mut self) {
        let inventory = open_recovery_staging_lock(Path::new(RECOVERY_STAGING_INVENTORY_LOCK))
            .ok()
            .and_then(|inventory| FileExt::lock_exclusive(&inventory).ok().map(|_| inventory));
        for path in self.files.iter().rev() {
            let _ = fs::remove_file(path);
        }
        if let Some(directory) = self.directory.as_ref() {
            if let Some(lease) = self.lease.take() {
                let _ = FileExt::unlock(&lease);
                drop(lease);
                let _ = fs::remove_file(directory.join(RECOVERY_STAGING_LEASE_FILE));
            }
            let _ = fs::remove_dir(directory);
        }
        if let Some(inventory) = inventory {
            let _ = FileExt::unlock(&inventory);
        }
    }
}

fn open_recovery_staging_lock(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Codex recovery staging lock must be a regular file",
        ));
    }
    Ok(file)
}

fn staging_lock_is_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || cfg!(windows) && error.raw_os_error() == Some(33)
}

fn is_owned_recovery_staging_attachment(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some((index, digest)) = name.split_once('-') else {
        return false;
    };
    index.len() >= 3
        && index.bytes().all(|byte| byte.is_ascii_digit())
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn discover_recovery_staging_directories(
    parent: &Path,
    prefix: &str,
    max_entries: usize,
) -> Result<Vec<PathBuf>, String> {
    let mut entries_seen = 0_usize;
    let mut directories = Vec::new();
    for entry in
        fs::read_dir(parent).map_err(|error| format!("scan Codex recovery staging: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read Codex recovery staging: {error}"))?;
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > max_entries {
            return Err(format!(
                "Codex recovery staging parent exceeds the {max_entries} entry inventory limit"
            ));
        }
        if !entry
            .file_type()
            .map_err(|error| format!("inspect Codex recovery staging: {error}"))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(staging_id) = name.strip_prefix(prefix) else {
            continue;
        };
        if validate_staging_id(staging_id).is_ok() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn inventory_owned_recovery_staging_files(
    directory: &Path,
    lease_path: &Path,
    max_entries: usize,
    max_total_bytes: u64,
) -> Result<Vec<PathBuf>, String> {
    let mut entries_seen = 0_usize;
    let mut total_bytes = 0_u64;
    let mut owned_files = Vec::new();
    for child in fs::read_dir(directory)
        .map_err(|error| format!("scan orphan Codex recovery staging: {error}"))?
    {
        let child =
            child.map_err(|error| format!("read orphan Codex recovery staging: {error}"))?;
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > max_entries {
            return Err(format!(
                "orphan Codex recovery staging exceeds the {max_entries} entry inventory limit"
            ));
        }
        if child.path() == lease_path {
            continue;
        }
        let file_type = child
            .file_type()
            .map_err(|error| format!("inspect orphan Codex recovery staging: {error}"))?;
        if !file_type.is_file() || !is_owned_recovery_staging_attachment(&child.file_name()) {
            return Err("orphan Codex recovery staging contains an unknown entry".to_string());
        }
        total_bytes = total_bytes
            .checked_add(
                child
                    .metadata()
                    .map_err(|error| {
                        format!("inspect orphan Codex recovery staging bytes: {error}")
                    })?
                    .len(),
            )
            .ok_or_else(|| "orphan Codex recovery staging byte inventory overflowed".to_string())?;
        if total_bytes > max_total_bytes {
            return Err(format!(
                "orphan Codex recovery staging exceeds the {max_total_bytes} byte inventory limit"
            ));
        }
        owned_files.push(child.path());
    }
    Ok(owned_files)
}

/// Remove staging directories whose sidecar lease is no longer held.
///
/// The caller owns the global staging inventory lock, closing the race where
/// another sidecar has created its directory but not yet acquired its lease.
fn sweep_orphan_container_recovery_staging() -> Result<usize, String> {
    let prefix_path = Path::new(CONTAINER_RECOVERY_STAGING_PREFIX);
    let parent = prefix_path
        .parent()
        .ok_or_else(|| "Codex recovery staging prefix has no parent".to_string())?;
    if !parent.is_dir() {
        return Ok(0);
    }
    let prefix = prefix_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Codex recovery staging prefix is invalid".to_string())?;
    let directories =
        discover_recovery_staging_directories(parent, prefix, MAX_RECOVERY_STAGING_PARENT_ENTRIES)?;
    let mut removed = 0;
    for directory in directories {
        let lease_path = directory.join(RECOVERY_STAGING_LEASE_FILE);
        let lease_existed = lease_path
            .try_exists()
            .map_err(|error| format!("inspect Codex recovery staging lease: {error}"))?;
        if lease_existed
            && !fs::symlink_metadata(&lease_path)
                .map_err(|error| format!("inspect Codex recovery staging lease: {error}"))?
                .file_type()
                .is_file()
        {
            return Err("Codex recovery staging lease is not a regular file".to_string());
        }
        let lease = open_recovery_staging_lock(&lease_path)
            .map_err(|error| format!("open Codex recovery staging lease: {error}"))?;
        match FileExt::try_lock_exclusive(&lease) {
            Ok(()) => {}
            Err(error) if staging_lock_is_contended(&error) => continue,
            Err(error) => {
                if !lease_existed {
                    drop(lease);
                    let _ = fs::remove_file(&lease_path);
                }
                return Err(format!("claim Codex recovery staging lease: {error}"));
            }
        }
        // Validate the complete directory before deleting any entry. An
        // unexpected file, symlink, or nested directory is not assumed to be
        // gwt-owned merely because its parent has a familiar prefix.
        let owned_files = inventory_owned_recovery_staging_files(
            &directory,
            &lease_path,
            MAX_RECOVERY_STAGING_CHILD_ENTRIES,
            MAX_RECOVERY_STAGING_DIRECTORY_BYTES,
        );
        let owned_files = match owned_files {
            Ok(files) => files,
            Err(error) => {
                let _ = FileExt::unlock(&lease);
                drop(lease);
                if !lease_existed {
                    let _ = fs::remove_file(&lease_path);
                }
                return Err(error);
            }
        };
        for path in owned_files {
            fs::remove_file(path)
                .map_err(|error| format!("remove orphan Codex recovery attachment: {error}"))?;
        }
        FileExt::unlock(&lease)
            .map_err(|error| format!("unlock orphan Codex recovery staging: {error}"))?;
        drop(lease);
        fs::remove_file(&lease_path)
            .map_err(|error| format!("remove orphan Codex recovery staging lease: {error}"))?;
        fs::remove_dir(&directory)
            .map_err(|error| format!("remove orphan Codex recovery staging: {error}"))?;
        removed += 1;
    }
    Ok(removed)
}

fn stage_container_recovery_attachments(
    bundle: Option<&ContainerRecoveryAttachmentBundle>,
) -> Result<ContainerRecoveryAttachmentStaging, String> {
    let inventory = open_recovery_staging_lock(Path::new(RECOVERY_STAGING_INVENTORY_LOCK))
        .map_err(|error| format!("open Codex recovery staging inventory lock: {error}"))?;
    FileExt::lock_exclusive(&inventory)
        .map_err(|error| format!("lock Codex recovery staging inventory: {error}"))?;
    sweep_orphan_container_recovery_staging()?;
    let Some(bundle) = bundle else {
        FileExt::unlock(&inventory)
            .map_err(|error| format!("unlock Codex recovery staging inventory: {error}"))?;
        return Ok(ContainerRecoveryAttachmentStaging::empty());
    };
    if bundle.attachments.is_empty() {
        return Err("Codex recovery attachment bundle is empty".to_string());
    }
    let mut budget = RecoveryAttachmentAggregateBudget::default();
    budget.reserve_count(bundle.attachments.len())?;
    for attachment in &bundle.attachments {
        validate_container_recovery_reference(&attachment.reference)?;
        let raw_bytes = projected_decoded_base64_len(&attachment.base64_data)?;
        let encoded_bytes = attachment
            .base64_data
            .len()
            .checked_add(attachment.reference.content_id.len())
            .and_then(|bytes| bytes.checked_add(attachment.reference.file_name.len()))
            .ok_or_else(|| {
                "Codex recovery attachment bundle exceeds the safety bound".to_string()
            })?;
        budget.reserve_projected(raw_bytes, encoded_bytes)?;
    }
    let paths = bundle.container_paths()?;
    let directory = PathBuf::from(format!(
        "{CONTAINER_RECOVERY_STAGING_PREFIX}{}",
        bundle.staging_id
    ));
    create_private_staging_dir(&directory)
        .map_err(|error| format!("create private Codex recovery staging: {error}"))?;
    let lease_path = directory.join(RECOVERY_STAGING_LEASE_FILE);
    let lease = match open_recovery_staging_lock(&lease_path) {
        Ok(lease) => lease,
        Err(error) => {
            let _ = fs::remove_dir(&directory);
            return Err(format!("create Codex recovery staging lease: {error}"));
        }
    };
    if let Err(error) = FileExt::lock_exclusive(&lease) {
        drop(lease);
        let _ = fs::remove_file(&lease_path);
        let _ = fs::remove_dir(&directory);
        return Err(format!("lock Codex recovery staging lease: {error}"));
    }
    if let Err(error) = FileExt::unlock(&inventory) {
        let _ = FileExt::unlock(&lease);
        drop(lease);
        let _ = fs::remove_file(&lease_path);
        let _ = fs::remove_dir(&directory);
        return Err(format!("unlock Codex recovery staging inventory: {error}"));
    }
    let mut staging = ContainerRecoveryAttachmentStaging {
        directory: Some(directory),
        lease: Some(lease),
        files: Vec::with_capacity(paths.len()),
        paths,
    };
    for (attachment, path) in bundle.attachments.iter().zip(staging.paths.iter()) {
        let encoded_bytes = attachment
            .base64_data
            .len()
            .checked_add(attachment.reference.content_id.len())
            .and_then(|bytes| bytes.checked_add(attachment.reference.file_name.len()))
            .ok_or_else(|| {
                "Codex recovery attachment bundle exceeds the safety bound".to_string()
            })?;
        budget.ensure_actual_capacity(0, encoded_bytes)?;
        let bytes = decode_verified_container_recovery_attachment_with_limit(
            attachment,
            budget.remaining_actual_raw(),
        )?;
        budget.consume_actual(bytes.len(), encoded_bytes)?;
        let path = PathBuf::from(path);
        let mut file = open_private_staging_file(&path)
            .map_err(|error| format!("create private Codex recovery attachment: {error}"))?;
        staging.files.push(path);
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("write private Codex recovery attachment: {error}"))?;
    }
    Ok(staging)
}

struct SharedControlWriter {
    writer: Mutex<Box<dyn Write + Send>>,
    next_sequence: AtomicU64,
}

impl SharedControlWriter {
    fn new(writer: impl Write + Send + 'static) -> Self {
        Self {
            writer: Mutex::new(Box::new(BufWriter::new(writer))),
            next_sequence: AtomicU64::new(1),
        }
    }

    fn reserve_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn send_reserved<T: Serialize>(&self, sequence: u64, payload: &T) -> Result<(), String> {
        let encoded = encode_control_frame(sequence, payload)?;
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        writer
            .write_all(&encoded)
            .and_then(|_| writer.flush())
            .map_err(|error| format!("write Codex sidecar control channel: {error}"))?;
        Ok(())
    }

    fn send<T: Serialize>(&self, payload: &T) -> Result<u64, String> {
        let sequence = self.reserve_sequence();
        self.send_reserved(sequence, payload)?;
        Ok(sequence)
    }
}

struct SidecarControlClient {
    writer: Arc<SharedControlWriter>,
    pending: Mutex<HashMap<u64, mpsc::SyncSender<bool>>>,
    shutdown: AtomicBool,
    shutdown_signal: (Mutex<bool>, Condvar),
}

impl SidecarControlClient {
    fn new(writer: Arc<SharedControlWriter>) -> Self {
        Self {
            writer,
            pending: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            shutdown_signal: (Mutex::new(false), Condvar::new()),
        }
    }

    fn send_and_wait(&self, payload: &SidecarControlPayload) -> Result<(), String> {
        self.send_and_wait_with_policy(payload, CONTROL_ACK_TIMEOUT, CONTROL_ACK_RETRIES)
    }

    fn send_and_wait_with_policy(
        &self,
        payload: &SidecarControlPayload,
        acknowledgement_timeout: Duration,
        retry_limit: usize,
    ) -> Result<(), String> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err("Codex sidecar control channel is closed".to_string());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let sequence = self.writer.reserve_sequence();
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(sequence, sender);
        let result = (|| {
            for attempt in 0..=retry_limit {
                self.writer.send_reserved(sequence, payload)?;
                match receiver.recv_timeout(acknowledgement_timeout) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {
                        return Err("Host rejected Codex sidecar durability request".to_string())
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) if attempt < retry_limit => continue,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        return Err("Codex sidecar durability acknowledgement timed out".to_string())
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("Codex sidecar control channel is closed".to_string())
                    }
                }
            }
            Err("Codex sidecar durability acknowledgement timed out".to_string())
        })();
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&sequence);
        result
    }

    fn notify(&self, payload: &SidecarControlPayload) -> Result<(), String> {
        self.writer.send(payload).map(|_| ())
    }

    fn acknowledge(&self, request_sequence: u64, accepted: bool) {
        if let Some(sender) = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&request_sequence)
        {
            let _ = sender.send(accepted);
        }
    }

    fn signal_shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        let (lock, condition) = &self.shutdown_signal;
        *lock.lock().unwrap_or_else(|error| error.into_inner()) = true;
        condition.notify_all();
    }

    fn wait_for_shutdown(&self) {
        let (lock, condition) = &self.shutdown_signal;
        let mut shutdown = lock.lock().unwrap_or_else(|error| error.into_inner());
        while !*shutdown {
            shutdown = condition
                .wait(shutdown)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

struct ControlDurabilitySink {
    client: Arc<SidecarControlClient>,
    source_cwd: PathBuf,
}

enum PreparedContainerAttachment<'a> {
    Retained {
        candidate: &'a super::AttachmentCandidate,
        raw_bytes: usize,
        encoded_bytes: usize,
    },
    Local(PreparedLocalAttachment),
}

impl ControlDurabilitySink {
    fn materialize_container_attachments(
        &self,
        input: &UserInputCapture,
    ) -> Result<(UserInputCapture, Vec<TransferredAttachment>), String> {
        let mut budget = RecoveryAttachmentAggregateBudget::default();
        budget.reserve_count(input.attachment_candidates.len())?;
        let mut prepared = Vec::with_capacity(input.attachment_candidates.len());
        for candidate in &input.attachment_candidates {
            match candidate.kind {
                super::AttachmentCandidateKind::ImageUrl => {
                    let (_, encoded) = parse_image_data_url(&candidate.source)?;
                    let raw_bytes = projected_decoded_base64_len(encoded)?;
                    let encoded_bytes = attachment_encoded_footprint(
                        candidate.source.len(),
                        candidate.detail.as_deref().unwrap_or(""),
                    )?;
                    budget.reserve_projected(raw_bytes, encoded_bytes)?;
                    prepared.push(PreparedContainerAttachment::Retained {
                        candidate,
                        raw_bytes,
                        encoded_bytes,
                    });
                }
                super::AttachmentCandidateKind::LocalImage => {
                    let path = PathBuf::from(&candidate.source);
                    let path = if path.is_absolute() {
                        path
                    } else {
                        self.source_cwd.join(path)
                    };
                    let file_name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or("attachment")
                        .to_string();
                    prepared.push(PreparedContainerAttachment::Local(
                        open_prepared_local_attachment(&path, file_name, &mut budget)
                            .map_err(|error| format!("read container attachment: {error}"))?,
                    ));
                }
            }
        }

        let mut retained_candidates = Vec::new();
        let mut attachments = Vec::new();
        for attachment in prepared {
            match attachment {
                PreparedContainerAttachment::Retained {
                    candidate,
                    raw_bytes,
                    encoded_bytes,
                } => {
                    budget.consume_actual(raw_bytes, encoded_bytes)?;
                    retained_candidates.push(candidate.clone());
                }
                PreparedContainerAttachment::Local(local) => {
                    let (file_name, bytes) = read_prepared_local_attachment(local, &mut budget)
                        .map_err(|error| format!("read container attachment: {error}"))?;
                    attachments.push(TransferredAttachment {
                        file_name,
                        base64_data: base64::engine::general_purpose::STANDARD.encode(bytes),
                    });
                }
            }
        }
        let forwarded = UserInputCapture {
            kind: input.kind,
            thread_id: input.thread_id.clone(),
            client_user_message_id: input.client_user_message_id.clone(),
            text_segments: input.text_segments.clone(),
            attachment_candidates: retained_candidates,
        };
        Ok((forwarded, attachments))
    }
}

impl CodexDurabilitySink for ControlDurabilitySink {
    fn persist_root_binding(
        &self,
        binding: &RootThreadBinding,
        wire_text: &str,
    ) -> Result<(), String> {
        self.client
            .send_and_wait(&SidecarControlPayload::PersistRoot {
                binding: binding.clone(),
                wire_text: wire_text.to_string(),
            })
    }

    fn persist_user_input(&self, input: &UserInputCapture, wire_text: &str) -> Result<(), String> {
        let (input, attachments) = self.materialize_container_attachments(input)?;
        self.client
            .send_and_wait(&SidecarControlPayload::PersistInput {
                input,
                attachments,
                wire_text: wire_text.to_string(),
            })
    }

    fn persist_visible_discussion(
        &self,
        capture: &VisibleDiscussionCapture,
        wire_text: &str,
    ) -> Result<(), String> {
        self.client
            .send_and_wait(&SidecarControlPayload::PersistVisible {
                capture: capture.clone(),
                wire_text: wire_text.to_string(),
            })
    }

    fn persist_transferred_user_input(
        &self,
        input: &UserInputCapture,
        _attachments: &[TransferredAttachment],
        wire_text: &str,
    ) -> Result<(), String> {
        self.persist_user_input(input, wire_text)
    }
}

fn read_sidecar_host_messages(client: Arc<SidecarControlClient>) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = Vec::new();
    loop {
        let result = read_bounded_control_line(&mut reader, &mut line).and_then(|present| {
            if present.is_none() {
                return Ok(None);
            }
            decode_control_frame::<HostControlPayload>(&line).map(Some)
        });
        match result {
            Ok(Some((
                _,
                _,
                HostControlPayload::Ack {
                    request_sequence,
                    accepted,
                },
            ))) => client.acknowledge(request_sequence, accepted),
            Ok(Some((_, _, HostControlPayload::Shutdown))) | Ok(None) | Err(_) => break,
            Ok(Some((_, _, HostControlPayload::Init { .. }))) => break,
        }
    }
    client.signal_shutdown();
}

fn start_sidecar_heartbeat(client: Weak<SidecarControlClient>) {
    let _ = thread::Builder::new()
        .name("gwt-codex-sidecar-heartbeat".to_string())
        .spawn(move || {
            let mut missed = 0;
            loop {
                thread::sleep(HEARTBEAT_INTERVAL);
                let Some(client) = client.upgrade() else {
                    return;
                };
                if client.shutdown.load(Ordering::Acquire) {
                    return;
                }
                match client.send_and_wait(&SidecarControlPayload::Heartbeat) {
                    Ok(()) => missed = 0,
                    Err(_) => {
                        missed += 1;
                        if missed >= MAX_HEARTBEAT_MISSES {
                            client.signal_shutdown();
                            return;
                        }
                    }
                }
            }
        });
}

fn command_basename(command: &str) -> &str {
    command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat")
}

fn validate_sidecar_manifest(app_server: &CodexAppServerLaunch) -> Result<(), String> {
    if app_server.env.contains_key(CODEX_REMOTE_AUTH_TOKEN_ENV) {
        return Err("Codex sidecar manifest must not contain its bearer token".to_string());
    }
    if app_server
        .cwd
        .as_ref()
        .is_some_and(|cwd| !cwd.is_absolute() && !cwd.to_string_lossy().starts_with('/'))
    {
        return Err("Codex sidecar working directory must be absolute".to_string());
    }
    let fixed_tail = ["app-server", "--listen", "stdio://"];
    let command = command_basename(&app_server.command).to_ascii_lowercase();
    match command.as_str() {
        "codex" => {
            if app_server.args != fixed_tail {
                return Err("Codex sidecar direct runner manifest is invalid".to_string());
            }
        }
        "bunx" | "npx" => {
            let mut package_index = 0;
            if app_server.args.first().is_some_and(|arg| arg == "--yes") {
                package_index = 1;
            }
            let package = app_server
                .args
                .get(package_index)
                .and_then(|arg| arg.strip_prefix("@openai/codex@"))
                .ok_or_else(|| "Codex sidecar package runner manifest is invalid".to_string())?;
            if semver::Version::parse(package).is_err() {
                return Err("Codex sidecar package runner must use an exact version".to_string());
            }
            let tail = &app_server.args[package_index + 1..];
            if tail != fixed_tail {
                return Err("Codex sidecar app-server arguments are invalid".to_string());
            }
        }
        _ => return Err("Codex sidecar runner is not allowed".to_string()),
    }
    Ok(())
}

/// Hidden `/usr/local/bin/gwtd __internal codex-sidecar` entrypoint.
pub fn run_container_codex_sidecar() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut first_line = Vec::new();
    {
        let mut lock = stdin.lock();
        if read_bounded_control_line(&mut lock, &mut first_line)?.is_none() {
            return Err("Codex sidecar did not receive initialization".to_string());
        }
    }
    let (_, _, init) = decode_control_frame::<HostControlPayload>(&first_line)?;
    let HostControlPayload::Init {
        app_server,
        expected_resume_id,
        recovery_attachments,
    } = init
    else {
        return Err("Codex sidecar expected initialization first".to_string());
    };
    let app_server = *app_server;
    validate_sidecar_manifest(&app_server)?;
    let recovery_staging = stage_container_recovery_attachments(recovery_attachments.as_ref())?;

    let writer = Arc::new(SharedControlWriter::new(std::io::stdout()));
    let client = Arc::new(SidecarControlClient::new(writer));
    let reader_client = client.clone();
    thread::Builder::new()
        .name("gwt-codex-sidecar-control".to_string())
        .spawn(move || read_sidecar_host_messages(reader_client))
        .map_err(|error| format!("start Codex sidecar control reader: {error}"))?;

    let durability: Arc<dyn CodexDurabilitySink> = Arc::new(ControlDurabilitySink {
        client: client.clone(),
        source_cwd: app_server.cwd.clone().unwrap_or_else(|| PathBuf::from(".")),
    });
    let ready_client = client.clone();
    let failure_client = client.clone();
    let lease = register_codex_bridge_route(CodexBridgeRouteConfig {
        app_server,
        expected_resume_id,
        durability,
        on_ready: Arc::new(move || {
            let _ = ready_client.notify(&SidecarControlPayload::ProviderReady);
        }),
        on_failure: Arc::new(move |failure| {
            let _ = failure_client.notify(&SidecarControlPayload::Failure { failure });
        }),
    })
    .map_err(|error| error.to_string())?;

    // This is the only control message carrying the bearer capability. It is
    // written to the attached pipe, never argv, URL query, stderr, or logs.
    client.notify(&SidecarControlPayload::Ready {
        endpoint: lease.endpoint().to_string(),
        auth_token: lease.inner.bearer_token.clone(),
        recovery_attachment_paths: recovery_staging.paths.clone(),
    })?;
    start_sidecar_heartbeat(Arc::downgrade(&client));
    client.wait_for_shutdown();
    drop(lease);
    drop(recovery_staging);
    Ok(())
}

pub struct CodexContainerBridgeConfig {
    pub compose_files: Vec<PathBuf>,
    pub service: String,
    pub working_dir: Option<String>,
    pub app_server: CodexAppServerLaunch,
    pub expected_resume_id: Option<String>,
    pub recovery_attachments: Option<ContainerRecoveryAttachmentBundle>,
    pub durability: Arc<dyn CodexDurabilitySink>,
    pub on_ready: Arc<dyn Fn() + Send + Sync>,
    pub on_failure: Arc<dyn Fn(CodexBridgeFailure) + Send + Sync>,
}

struct ContainerBridgeLeaseInner {
    endpoint: String,
    bearer_token: String,
    recovery_attachment_paths: Vec<String>,
    root_forwarded: Arc<AtomicBool>,
    writer: Arc<SharedControlWriter>,
    cancelled: Arc<AtomicBool>,
    child: Mutex<Option<Child>>,
}

fn stop_container_sidecar(writer: &SharedControlWriter, child: &mut Child) {
    let _ = writer.send(&HostControlPayload::Shutdown);
    let deadline = Instant::now() + CONTROL_SHUTDOWN_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) | Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for ContainerBridgeLeaseInner {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(mut child) = self
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            stop_container_sidecar(&self.writer, &mut child);
        }
    }
}

#[derive(Clone)]
pub struct CodexContainerBridgeLease {
    inner: Arc<ContainerBridgeLeaseInner>,
}

impl CodexContainerBridgeLease {
    pub fn endpoint(&self) -> &str {
        &self.inner.endpoint
    }

    pub fn install_auth_token(&self, env: &mut HashMap<String, String>) {
        env.insert(
            CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
            self.inner.bearer_token.clone(),
        );
    }

    pub fn root_forwarded(&self) -> bool {
        self.inner.root_forwarded.load(Ordering::Acquire)
    }

    pub fn recovery_attachment_paths(&self) -> &[String] {
        &self.inner.recovery_attachment_paths
    }
}

impl fmt::Debug for CodexContainerBridgeLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexContainerBridgeLease")
            .field("endpoint", &self.inner.endpoint)
            .field("bearer_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
pub enum CodexLaunchBridgeLease {
    Host(CodexBridgeRouteLease),
    Container(CodexContainerBridgeLease),
}

impl CodexLaunchBridgeLease {
    pub fn endpoint(&self) -> &str {
        match self {
            Self::Host(lease) => lease.endpoint(),
            Self::Container(lease) => lease.endpoint(),
        }
    }

    pub fn install_auth_token(&self, env: &mut HashMap<String, String>) {
        match self {
            Self::Host(lease) => lease.install_auth_token(env),
            Self::Container(lease) => lease.install_auth_token(env),
        }
    }

    pub fn root_forwarded(&self) -> bool {
        match self {
            Self::Host(lease) => lease.root_forwarded(),
            Self::Container(lease) => lease.root_forwarded(),
        }
    }
}

impl fmt::Debug for CodexLaunchBridgeLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(lease) => lease.fmt(formatter),
            Self::Container(lease) => lease.fmt(formatter),
        }
    }
}

fn valid_sidecar_endpoint(endpoint: &str) -> bool {
    let Some(port_and_path) = endpoint.strip_prefix("ws://127.0.0.1:") else {
        return false;
    };
    let Some(port) = port_and_path.strip_suffix("/codex") else {
        return false;
    };
    port.parse::<u16>().is_ok_and(|port| port != 0)
}

struct SidecarReadyDetails {
    endpoint: String,
    auth_token: String,
    recovery_attachment_paths: Vec<String>,
}

type SidecarReadyResult = Result<SidecarReadyDetails, String>;

struct HostSidecarMessageContext {
    writer: Arc<SharedControlWriter>,
    durability: Arc<dyn CodexDurabilitySink>,
    root_forwarded: Arc<AtomicBool>,
    on_ready: Arc<dyn Fn() + Send + Sync>,
    on_failure: Arc<dyn Fn(CodexBridgeFailure) + Send + Sync>,
    cancelled: Arc<AtomicBool>,
    ready_sender: mpsc::SyncSender<SidecarReadyResult>,
}

fn handle_host_sidecar_messages(stdout: impl Read, context: HostSidecarMessageContext) {
    let HostSidecarMessageContext {
        writer,
        durability,
        root_forwarded,
        on_ready,
        on_failure,
        cancelled,
        ready_sender,
    } = context;
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut ready_sender = Some(ready_sender);
    let mut became_ready = false;
    let mut acknowledged = HashMap::<u64, (String, bool)>::new();
    let mut acknowledgement_order = VecDeque::new();
    loop {
        let decoded = match read_bounded_control_line(&mut reader, &mut line) {
            Ok(Some(())) => decode_control_frame::<SidecarControlPayload>(&line),
            Ok(None) => break,
            Err(error) => Err(error),
        };
        let (sequence, checksum, payload) = match decoded {
            Ok(decoded) => decoded,
            Err(error) => {
                if let Some(sender) = ready_sender.take() {
                    let _ = sender.send(Err(error));
                }
                break;
            }
        };
        let requires_ack = matches!(
            &payload,
            SidecarControlPayload::PersistRoot { .. }
                | SidecarControlPayload::PersistInput { .. }
                | SidecarControlPayload::PersistVisible { .. }
                | SidecarControlPayload::Heartbeat
        );
        if requires_ack {
            if let Some((recorded_checksum, accepted)) = acknowledged.get(&sequence) {
                if recorded_checksum != &checksum {
                    on_failure(CodexBridgeFailure {
                        operation: None,
                        kind: CodexBridgeFailureKind::Protocol,
                        reason: "Codex sidecar reused a control sequence with different content"
                            .to_string(),
                    });
                    break;
                }
                let _ = writer.send(&HostControlPayload::Ack {
                    request_sequence: sequence,
                    accepted: *accepted,
                });
                continue;
            }
        }
        match payload {
            SidecarControlPayload::Ready {
                endpoint,
                auth_token,
                recovery_attachment_paths,
            } => {
                if let Some(sender) = ready_sender.take() {
                    let result = if valid_sidecar_endpoint(&endpoint)
                        && super::CodexRouteIdentity::from_bearer_token(&auth_token).is_ok()
                    {
                        Ok(SidecarReadyDetails {
                            endpoint,
                            auth_token,
                            recovery_attachment_paths,
                        })
                    } else {
                        Err("Codex sidecar returned an invalid loopback capability".to_string())
                    };
                    became_ready = result.is_ok();
                    let _ = sender.send(result);
                }
            }
            SidecarControlPayload::PersistRoot { binding, wire_text } => {
                let accepted = durability
                    .persist_root_binding(&binding, &wire_text)
                    .is_ok();
                remember_control_ack(
                    &mut acknowledged,
                    &mut acknowledgement_order,
                    sequence,
                    checksum,
                    accepted,
                );
                let _ = writer.send(&HostControlPayload::Ack {
                    request_sequence: sequence,
                    accepted,
                });
            }
            SidecarControlPayload::PersistInput {
                input,
                attachments,
                wire_text,
            } => {
                let accepted = durability
                    .persist_transferred_user_input(&input, &attachments, &wire_text)
                    .is_ok();
                remember_control_ack(
                    &mut acknowledged,
                    &mut acknowledgement_order,
                    sequence,
                    checksum,
                    accepted,
                );
                let _ = writer.send(&HostControlPayload::Ack {
                    request_sequence: sequence,
                    accepted,
                });
            }
            SidecarControlPayload::PersistVisible { capture, wire_text } => {
                let accepted = durability
                    .persist_visible_discussion(&capture, &wire_text)
                    .is_ok();
                remember_control_ack(
                    &mut acknowledged,
                    &mut acknowledgement_order,
                    sequence,
                    checksum,
                    accepted,
                );
                let _ = writer.send(&HostControlPayload::Ack {
                    request_sequence: sequence,
                    accepted,
                });
            }
            SidecarControlPayload::Heartbeat => {
                remember_control_ack(
                    &mut acknowledged,
                    &mut acknowledgement_order,
                    sequence,
                    checksum,
                    true,
                );
                let _ = writer.send(&HostControlPayload::Ack {
                    request_sequence: sequence,
                    accepted: true,
                });
            }
            SidecarControlPayload::ProviderReady => {
                if !root_forwarded.swap(true, Ordering::AcqRel) {
                    on_ready();
                }
            }
            SidecarControlPayload::Failure { failure } => on_failure(failure),
        }
    }
    if let Some(sender) = ready_sender.take() {
        let _ = sender.send(Err(
            "Codex sidecar closed before publishing its loopback capability".to_string(),
        ));
    } else if became_ready && !cancelled.load(Ordering::Acquire) {
        on_failure(CodexBridgeFailure {
            operation: None,
            kind: CodexBridgeFailureKind::Transport,
            reason: "Codex container sidecar control channel closed".to_string(),
        });
    }
}

fn remember_control_ack(
    acknowledged: &mut HashMap<u64, (String, bool)>,
    order: &mut VecDeque<u64>,
    sequence: u64,
    checksum: String,
    accepted: bool,
) {
    acknowledged.insert(sequence, (checksum, accepted));
    order.push_back(sequence);
    while order.len() > MAX_ACK_CACHE_ENTRIES {
        if let Some(expired) = order.pop_front() {
            acknowledged.remove(&expired);
        }
    }
}

/// Start the hidden sidecar through the selected Compose/Podman abstraction.
pub fn start_container_codex_bridge(
    config: CodexContainerBridgeConfig,
) -> Result<CodexContainerBridgeLease, String> {
    let expected_recovery_attachment_paths = config
        .recovery_attachments
        .as_ref()
        .map(ContainerRecoveryAttachmentBundle::container_paths)
        .transpose()?
        .unwrap_or_default();
    let command = SIDECAR_COMMAND
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    let mut child = gwt_docker::spawn_compose_service_exec_attached_with_files(
        &config.compose_files,
        &config.service,
        config.working_dir.as_deref(),
        &[],
        &command,
    )
    .map_err(|error| error.to_string())?;
    let stdin: ChildStdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex sidecar stdin was unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex sidecar stdout was unavailable".to_string())?;
    if let Some(mut stderr) = child.stderr.take() {
        let _ = thread::Builder::new()
            .name("gwt-codex-sidecar-stderr".to_string())
            .spawn(move || {
                let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            });
    }

    let writer = Arc::new(SharedControlWriter::new(stdin));
    if let Err(error) = writer.send(&HostControlPayload::Init {
        app_server: Box::new(config.app_server),
        expected_resume_id: config.expected_resume_id,
        recovery_attachments: config.recovery_attachments,
    }) {
        stop_container_sidecar(&writer, &mut child);
        return Err(error);
    }

    let root_forwarded = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let reader_root_forwarded = root_forwarded.clone();
    let reader_cancelled = cancelled.clone();
    let reader_writer = writer.clone();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    if let Err(error) = thread::Builder::new()
        .name("gwt-codex-sidecar-host-control".to_string())
        .spawn(move || {
            handle_host_sidecar_messages(
                stdout,
                HostSidecarMessageContext {
                    writer: reader_writer,
                    durability: config.durability,
                    root_forwarded: reader_root_forwarded,
                    on_ready: config.on_ready,
                    on_failure: config.on_failure,
                    cancelled: reader_cancelled,
                    ready_sender,
                },
            )
        })
    {
        stop_container_sidecar(&writer, &mut child);
        return Err(format!("start Codex sidecar host control reader: {error}"));
    }

    let ready = match ready_receiver.recv_timeout(CONTROL_READY_TIMEOUT) {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            cancelled.store(true, Ordering::Release);
            stop_container_sidecar(&writer, &mut child);
            return Err(error);
        }
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            stop_container_sidecar(&writer, &mut child);
            return Err("Codex sidecar did not become ready before timeout".to_string());
        }
    };
    let SidecarReadyDetails {
        endpoint,
        auth_token: bearer_token,
        recovery_attachment_paths,
    } = ready;
    if recovery_attachment_paths != expected_recovery_attachment_paths {
        cancelled.store(true, Ordering::Release);
        stop_container_sidecar(&writer, &mut child);
        return Err("Codex sidecar recovery attachment manifest mismatch".to_string());
    }
    Ok(CodexContainerBridgeLease {
        inner: Arc::new(ContainerBridgeLeaseInner {
            endpoint,
            bearer_token,
            recovery_attachment_paths,
            root_forwarded,
            writer,
            cancelled,
            child: Mutex::new(Some(child)),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct ChunkWriter {
        sender: std::sync::mpsc::Sender<Vec<u8>>,
        frames: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl Write for ChunkWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.frames
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(bytes.to_vec());
            self.sender.send(bytes.to_vec()).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test channel closed")
            })?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct ChunkReader {
        receiver: std::sync::mpsc::Receiver<Vec<u8>>,
        pending: VecDeque<u8>,
    }

    impl ChunkReader {
        fn new(receiver: std::sync::mpsc::Receiver<Vec<u8>>) -> Self {
            Self {
                receiver,
                pending: VecDeque::new(),
            }
        }
    }

    impl Read for ChunkReader {
        fn read(&mut self, destination: &mut [u8]) -> std::io::Result<usize> {
            while self.pending.is_empty() {
                match self.receiver.recv() {
                    Ok(bytes) => self.pending.extend(bytes),
                    Err(_) => return Ok(0),
                }
            }
            let count = destination.len().min(self.pending.len());
            for slot in &mut destination[..count] {
                *slot = self.pending.pop_front().expect("pending byte");
            }
            Ok(count)
        }
    }

    struct DropFirstAckWriter {
        sender: std::sync::mpsc::Sender<Vec<u8>>,
        writes: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Write for DropFirstAckWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let write = self.writes.fetch_add(1, Ordering::Relaxed);
            if write > 0 {
                self.sender.send(bytes.to_vec()).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test channel closed")
                })?;
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingDurability {
        root_writes: std::sync::atomic::AtomicUsize,
    }

    impl CodexDurabilitySink for CountingDurability {
        fn persist_root_binding(
            &self,
            _binding: &RootThreadBinding,
            _wire_text: &str,
        ) -> Result<(), String> {
            self.root_writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn persist_user_input(
            &self,
            _input: &UserInputCapture,
            _wire_text: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        fn persist_visible_discussion(
            &self,
            _capture: &VisibleDiscussionCapture,
            _wire_text: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        fn persist_transferred_user_input(
            &self,
            _input: &UserInputCapture,
            _attachments: &[TransferredAttachment],
            _wire_text: &str,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn control_frame_is_versioned_checksummed_and_roundtrips() {
        let payload = SidecarControlPayload::Heartbeat;
        let encoded = encode_control_frame(7, &payload).expect("encode");
        let (sequence, _, decoded) =
            decode_control_frame::<SidecarControlPayload>(&encoded[..encoded.len() - 1])
                .expect("decode");
        assert_eq!(sequence, 7);
        assert!(matches!(decoded, SidecarControlPayload::Heartbeat));

        let mut tampered = encoded[..encoded.len() - 1].to_vec();
        let index = tampered
            .windows("heartbeat".len())
            .position(|window| window == b"heartbeat")
            .expect("payload marker");
        tampered[index] = b'j';
        let error = match decode_control_frame::<SidecarControlPayload>(&tampered) {
            Ok(_) => panic!("checksum must reject mutation"),
            Err(error) => error,
        };
        assert!(error.contains("checksum"));
    }

    #[test]
    fn repeated_control_frame_reuses_ack_without_repeating_durability() {
        let ready = SidecarControlPayload::Ready {
            endpoint: "ws://127.0.0.1:4567/codex".to_string(),
            auth_token: "a".repeat(48),
            recovery_attachment_paths: Vec::new(),
        };
        let persist = SidecarControlPayload::PersistRoot {
            binding: RootThreadBinding {
                thread_id: "thread-1".to_string(),
                session_id: "session-1".to_string(),
                cli_version: "0.144.5".to_string(),
                forked_from_id: None,
                operation: super::super::ThreadOperation::Start,
            },
            wire_text: r#"{"method":"thread/started"}"#.to_string(),
        };
        let mut input = encode_control_frame(1, &ready).expect("ready frame");
        let repeated = encode_control_frame(2, &persist).expect("persist frame");
        input.extend_from_slice(&repeated);
        input.extend_from_slice(&repeated);

        let recorded = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::new(SharedControlWriter::new(RecordingWriter(recorded.clone())));
        let durability = Arc::new(CountingDurability::default());
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        handle_host_sidecar_messages(
            std::io::Cursor::new(input),
            HostSidecarMessageContext {
                writer,
                durability: durability.clone(),
                root_forwarded: Arc::new(AtomicBool::new(false)),
                on_ready: Arc::new(|| {}),
                on_failure: Arc::new(|_| {}),
                cancelled: Arc::new(AtomicBool::new(true)),
                ready_sender,
            },
        );

        let ready = ready_receiver
            .recv()
            .expect("ready result")
            .expect("valid ready");
        assert!(ready.recovery_attachment_paths.is_empty());
        assert_eq!(durability.root_writes.load(Ordering::Relaxed), 1);

        let output = recorded
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let accepted_acks = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .filter_map(|line| decode_control_frame::<HostControlPayload>(line).ok())
            .filter(|(_, _, payload)| {
                matches!(
                    payload,
                    HostControlPayload::Ack {
                        request_sequence: 2,
                        accepted: true
                    }
                )
            })
            .count();
        assert_eq!(accepted_acks, 2);
    }

    #[test]
    fn lost_ack_retries_same_frame_and_host_ack_cache_prevents_duplicate_durability() {
        let (to_host_sender, to_host_receiver) = std::sync::mpsc::channel();
        let (to_client_sender, to_client_receiver) = std::sync::mpsc::channel();
        let sidecar_frames = Arc::new(Mutex::new(Vec::new()));
        let sidecar_writer = Arc::new(SharedControlWriter::new(ChunkWriter {
            sender: to_host_sender.clone(),
            frames: sidecar_frames.clone(),
        }));
        let client = Arc::new(SidecarControlClient::new(sidecar_writer.clone()));
        let host_ack_writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let host_writer = Arc::new(SharedControlWriter::new(DropFirstAckWriter {
            sender: to_client_sender,
            writes: host_ack_writes.clone(),
        }));
        let durability = Arc::new(CountingDurability::default());
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let host = {
            let durability = durability.clone();
            std::thread::spawn(move || {
                handle_host_sidecar_messages(
                    ChunkReader::new(to_host_receiver),
                    HostSidecarMessageContext {
                        writer: host_writer,
                        durability,
                        root_forwarded: Arc::new(AtomicBool::new(false)),
                        on_ready: Arc::new(|| {}),
                        on_failure: Arc::new(|_| {}),
                        cancelled: Arc::new(AtomicBool::new(true)),
                        ready_sender,
                    },
                )
            })
        };
        to_host_sender
            .send(
                encode_control_frame(
                    0,
                    &SidecarControlPayload::Ready {
                        endpoint: "ws://127.0.0.1:4567/codex".to_string(),
                        auth_token: "a".repeat(48),
                        recovery_attachment_paths: Vec::new(),
                    },
                )
                .expect("ready frame"),
            )
            .expect("send ready");
        ready_receiver
            .recv()
            .expect("ready result")
            .expect("valid ready");

        let weak_client = Arc::downgrade(&client);
        let acknowledgements = std::thread::spawn(move || {
            let mut reader = BufReader::new(ChunkReader::new(to_client_receiver));
            let mut line = Vec::new();
            while read_bounded_control_line(&mut reader, &mut line).ok() == Some(Some(())) {
                if let Ok((
                    _,
                    _,
                    HostControlPayload::Ack {
                        request_sequence,
                        accepted,
                    },
                )) = decode_control_frame::<HostControlPayload>(&line)
                {
                    if let Some(client) = weak_client.upgrade() {
                        client.acknowledge(request_sequence, accepted);
                    }
                }
            }
        });

        client
            .send_and_wait_with_policy(
                &SidecarControlPayload::PersistRoot {
                    binding: RootThreadBinding {
                        thread_id: "thread-1".to_string(),
                        session_id: "session-1".to_string(),
                        cli_version: "0.144.5".to_string(),
                        forked_from_id: None,
                        operation: super::super::ThreadOperation::Start,
                    },
                    wire_text: r#"{"method":"thread/started"}"#.to_string(),
                },
                Duration::from_millis(100),
                2,
            )
            .expect("second identical frame receives cached ACK");

        assert_eq!(durability.root_writes.load(Ordering::Relaxed), 1);
        assert_eq!(host_ack_writes.load(Ordering::Relaxed), 2);
        let frames = sidecar_frames
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(frames.len(), 2);
        let first = decode_control_frame::<SidecarControlPayload>(
            frames[0].strip_suffix(b"\n").expect("newline"),
        )
        .expect("first frame");
        let second = decode_control_frame::<SidecarControlPayload>(
            frames[1].strip_suffix(b"\n").expect("newline"),
        )
        .expect("second frame");
        assert_eq!(first.0, second.0);
        assert_eq!(first.1, second.1);

        drop(client);
        drop(sidecar_writer);
        drop(to_host_sender);
        host.join().expect("host reader");
        acknowledgements.join().expect("ack reader");
    }

    #[test]
    fn recovery_attachment_bundle_contains_no_host_path_and_verifies_bytes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = gwt_core::recovery::RecoveryStore::new(temp.path().join("store"));
        let host_only = temp.path().join("host-only-source");
        std::fs::create_dir_all(&host_only).expect("source dir");
        let source = host_only.join("evidence.png");
        std::fs::write(&source, b"sidecar recovery bytes").expect("source bytes");
        let reference = store.copy_attachment(&source).expect("durable copy");

        let bundle = prepare_container_recovery_attachments(&store, &[reference])
            .expect("prepare bundle")
            .expect("bundle");
        let paths = bundle.container_paths().expect("container paths");
        let serialized = serde_json::to_string(&bundle).expect("serialize bundle");
        let host_path = host_only.to_string_lossy();

        assert_eq!(paths.len(), 1);
        assert!(paths[0].starts_with(CONTAINER_RECOVERY_STAGING_PREFIX));
        assert!(!paths[0].contains(host_path.as_ref()));
        assert!(!serialized.contains(host_path.as_ref()));
        assert!(!format!("{bundle:?}").contains(host_path.as_ref()));
        assert_eq!(
            decode_verified_container_recovery_attachment(&bundle.attachments[0])
                .expect("verified bytes"),
            b"sidecar recovery bytes"
        );

        let mut tampered = bundle.attachments[0].clone();
        tampered.base64_data = base64::engine::general_purpose::STANDARD.encode(b"different bytes");
        assert!(decode_verified_container_recovery_attachment(&tampered).is_err());
    }

    #[test]
    fn recovery_attachment_bundle_rejects_count_and_projected_aggregate_before_reads() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = gwt_core::recovery::RecoveryStore::new(temp.path().join("store"));
        let reference = gwt_core::recovery::RecoveryAttachmentRef {
            content_id: format!("sha256:{}", "0".repeat(64)),
            file_name: "missing.bin".to_string(),
            byte_len: 0,
        };
        let too_many = vec![reference.clone(); super::super::MAX_RECOVERY_ATTACHMENT_COUNT + 1];
        let count_error = prepare_container_recovery_attachments(&store, &too_many)
            .expect_err("count must fail before any missing blob is read");
        assert!(count_error.contains("count"), "{count_error}");

        let projected = [
            gwt_core::recovery::RecoveryAttachmentRef {
                byte_len: 17 * 1024 * 1024,
                ..reference.clone()
            },
            gwt_core::recovery::RecoveryAttachmentRef {
                byte_len: 17 * 1024 * 1024,
                ..reference
            },
        ];
        let aggregate_error = prepare_container_recovery_attachments(&store, &projected)
            .expect_err("aggregate projection must fail before any missing blob is read");
        assert!(aggregate_error.contains("aggregate"), "{aggregate_error}");
    }

    #[test]
    fn recovery_attachment_bundle_rejects_underreported_metadata_after_actual_read() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = gwt_core::recovery::RecoveryStore::new(temp.path().join("store"));
        let source = temp.path().join("actual.bin");
        std::fs::write(&source, b"actual attachment bytes").expect("source");
        let mut reference = store.copy_attachment(&source).expect("copy attachment");
        reference.byte_len = 1;

        let error = prepare_container_recovery_attachments(&store, &[reference])
            .expect_err("actual bytes must be checked after metadata preflight");
        assert!(error.contains("digest verification"), "{error}");
    }

    #[test]
    fn container_attachment_materialization_bounds_count_and_sparse_aggregate_before_reads() {
        let temp = tempfile::tempdir().expect("tempdir");
        let client = Arc::new(SidecarControlClient::new(Arc::new(
            SharedControlWriter::new(std::io::sink()),
        )));
        let sink = ControlDurabilitySink {
            client,
            source_cwd: temp.path().to_path_buf(),
        };
        let many = UserInputCapture {
            kind: super::super::UserInputKind::Start,
            thread_id: "root".to_string(),
            client_user_message_id: None,
            text_segments: Vec::new(),
            attachment_candidates: (0..=super::super::MAX_RECOVERY_ATTACHMENT_COUNT)
                .map(|index| super::super::AttachmentCandidate {
                    kind: super::super::AttachmentCandidateKind::LocalImage,
                    source: format!("missing-{index}.png"),
                    detail: None,
                })
                .collect(),
        };
        let count_error = sink
            .materialize_container_attachments(&many)
            .expect_err("count must fail before missing paths are opened");
        assert!(count_error.contains("count"), "{count_error}");

        let first = temp.path().join("first.png");
        let second = temp.path().join("second.png");
        for path in [&first, &second] {
            std::fs::File::create(path)
                .expect("create sparse attachment")
                .set_len(17 * 1024 * 1024)
                .expect("size sparse attachment");
        }
        let aggregate = UserInputCapture {
            attachment_candidates: [first, second]
                .into_iter()
                .map(|path| super::super::AttachmentCandidate {
                    kind: super::super::AttachmentCandidateKind::LocalImage,
                    source: path.display().to_string(),
                    detail: None,
                })
                .collect(),
            ..many
        };
        let aggregate_error = sink
            .materialize_container_attachments(&aggregate)
            .expect_err("aggregate projection must fail before sparse files are read");
        assert!(aggregate_error.contains("aggregate"), "{aggregate_error}");
    }

    #[cfg(unix)]
    #[test]
    fn recovery_attachment_staging_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt as _;

        let _test_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let bytes = b"private sidecar bytes";
        let reference = gwt_core::recovery::RecoveryAttachmentRef {
            content_id: format!("sha256:{}", hex::encode(Sha256::digest(bytes))),
            file_name: "evidence.bin".to_string(),
            byte_len: bytes.len() as u64,
        };
        let bundle = ContainerRecoveryAttachmentBundle {
            staging_id: uuid::Uuid::new_v4().simple().to_string(),
            attachments: vec![ContainerRecoveryAttachmentTransfer {
                reference,
                base64_data: base64::engine::general_purpose::STANDARD.encode(bytes),
            }],
        };
        let staging = stage_container_recovery_attachments(Some(&bundle)).expect("stage");
        let directory = staging.directory.clone().expect("directory");
        let path = PathBuf::from(&staging.paths[0]);

        assert_eq!(std::fs::read(&path).expect("staged bytes"), bytes);
        assert_eq!(
            std::fs::metadata(&directory)
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(staging);
        assert!(!path.exists());
        assert!(!directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_attachment_staging_sweeps_abandoned_directories() {
        let _test_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staging_id = uuid::Uuid::new_v4().simple().to_string();
        let directory = PathBuf::from(format!("{CONTAINER_RECOVERY_STAGING_PREFIX}{staging_id}"));
        create_private_staging_dir(&directory).expect("stale staging directory");
        let attachment = directory.join(format!("000-{}", "a".repeat(64)));
        std::fs::write(attachment, b"stale").expect("stale attachment");
        drop(
            open_recovery_staging_lock(&directory.join(RECOVERY_STAGING_LEASE_FILE))
                .expect("stale lease"),
        );

        let staging = stage_container_recovery_attachments(None).expect("sweep staging");

        assert!(staging.directory.is_none());
        assert!(!directory.exists());
    }

    #[test]
    fn recovery_staging_inventories_reject_overflow_without_truncation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let staging_id = "a".repeat(32);
        let directory = temp.path().join(format!("gwt-codex-recovery-{staging_id}"));
        fs::create_dir(&directory).expect("staging directory");
        fs::write(temp.path().join("unrelated-one"), b"one").unwrap();
        fs::write(temp.path().join("unrelated-two"), b"two").unwrap();

        let parent_error =
            discover_recovery_staging_directories(temp.path(), "gwt-codex-recovery-", 2)
                .expect_err("all parent entries must consume the bound");
        assert!(parent_error.contains("entry inventory limit"));
        assert!(directory.exists());

        let lease = directory.join(RECOVERY_STAGING_LEASE_FILE);
        fs::write(&lease, b"").unwrap();
        let first = directory.join(format!("000-{}", "b".repeat(64)));
        let second = directory.join(format!("001-{}", "c".repeat(64)));
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let child_error = inventory_owned_recovery_staging_files(&directory, &lease, 2, 100)
            .expect_err("all child entries, including the lease, consume the bound");
        assert!(child_error.contains("entry inventory limit"));
        assert!(first.exists() && second.exists());

        let byte_error = inventory_owned_recovery_staging_files(&directory, &lease, 10, 5)
            .expect_err("staging bytes must have an aggregate bound");
        assert!(byte_error.contains("byte inventory limit"));
        assert!(first.exists() && second.exists());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_attachment_staging_preserves_live_leases() {
        let _test_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staging_id = uuid::Uuid::new_v4().simple().to_string();
        let directory = PathBuf::from(format!("{CONTAINER_RECOVERY_STAGING_PREFIX}{staging_id}"));
        create_private_staging_dir(&directory).expect("live staging directory");
        let attachment = directory.join(format!("000-{}", "b".repeat(64)));
        std::fs::write(attachment, b"live").expect("live attachment");
        let lease = open_recovery_staging_lock(&directory.join(RECOVERY_STAGING_LEASE_FILE))
            .expect("live lease");
        FileExt::lock_exclusive(&lease).expect("claim live lease");

        stage_container_recovery_attachments(None).expect("sweep staging");

        assert!(directory.exists());
        FileExt::unlock(&lease).expect("release live lease");
        drop(lease);
        stage_container_recovery_attachments(None).expect("sweep released staging");
        assert!(!directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_attachment_staging_cleans_partial_files_after_decode_failure() {
        let _test_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staging_id = uuid::Uuid::new_v4().simple().to_string();
        let directory = PathBuf::from(format!("{CONTAINER_RECOVERY_STAGING_PREFIX}{staging_id}"));
        let valid_bytes = b"first valid attachment";
        let invalid_bytes = b"tampered second attachment";
        let reference = |name: &str, bytes: &[u8]| gwt_core::recovery::RecoveryAttachmentRef {
            content_id: format!("sha256:{}", hex::encode(Sha256::digest(bytes))),
            file_name: name.to_string(),
            byte_len: bytes.len() as u64,
        };
        let bundle = ContainerRecoveryAttachmentBundle {
            staging_id,
            attachments: vec![
                ContainerRecoveryAttachmentTransfer {
                    reference: reference("first.bin", valid_bytes),
                    base64_data: base64::engine::general_purpose::STANDARD.encode(valid_bytes),
                },
                ContainerRecoveryAttachmentTransfer {
                    reference: reference("second.bin", b"expected second attachment"),
                    base64_data: base64::engine::general_purpose::STANDARD.encode(invalid_bytes),
                },
            ],
        };

        assert!(stage_container_recovery_attachments(Some(&bundle)).is_err());
        assert!(
            !directory.exists(),
            "error cleanup removes files, lease, and staging directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_attachment_staging_fails_closed_on_unknown_nested_content() {
        let _test_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staging_id = uuid::Uuid::new_v4().simple().to_string();
        let directory = PathBuf::from(format!("{CONTAINER_RECOVERY_STAGING_PREFIX}{staging_id}"));
        create_private_staging_dir(&directory).expect("unknown staging directory");
        let owned_attachment = directory.join(format!("000-{}", "c".repeat(64)));
        std::fs::write(&owned_attachment, b"must remain").expect("owned-looking attachment");
        let unknown_directory = directory.join("unexpected");
        std::fs::create_dir(&unknown_directory).expect("nested unknown directory");

        let error = stage_container_recovery_attachments(None)
            .err()
            .expect("reject unknown entry");

        assert!(error.contains("unknown entry"), "{error}");
        assert!(owned_attachment.exists(), "validation precedes deletion");
        assert!(unknown_directory.exists(), "unknown content is retained");
        std::fs::remove_dir(&unknown_directory).expect("cleanup nested fixture");
        std::fs::remove_file(&owned_attachment).expect("cleanup attachment fixture");
        let lease_path = directory.join(RECOVERY_STAGING_LEASE_FILE);
        assert!(
            !lease_path.exists(),
            "a validation failure does not introduce a lease into an unknown directory"
        );
        std::fs::remove_dir(&directory).expect("cleanup staging fixture");
    }

    #[test]
    fn sidecar_endpoint_accepts_container_loopback_only() {
        assert!(valid_sidecar_endpoint("ws://127.0.0.1:4567/codex"));
        for endpoint in [
            "ws://0.0.0.0:4567/codex",
            "ws://host.docker.internal:4567/codex",
            "ws://127.0.0.1:0/codex",
            "ws://127.0.0.1:4567/other",
        ] {
            assert!(!valid_sidecar_endpoint(endpoint), "{endpoint}");
        }
    }

    #[test]
    fn sidecar_manifest_rejects_latest_unknown_runners_and_embedded_tokens() {
        let launch = |command: &str, args: &[&str]| CodexAppServerLaunch {
            command: command.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(PathBuf::from("/workspace")),
        };
        assert!(validate_sidecar_manifest(&launch(
            "npx",
            &[
                "--yes",
                "@openai/codex@0.144.5",
                "app-server",
                "--listen",
                "stdio://",
            ],
        ))
        .is_ok());
        assert!(validate_sidecar_manifest(&launch(
            "npx",
            &[
                "--yes",
                "@openai/codex@latest",
                "app-server",
                "--listen",
                "stdio://",
            ],
        ))
        .is_err());
        assert!(validate_sidecar_manifest(&launch(
            "sh",
            &["-lc", "codex app-server --listen stdio://"],
        ))
        .is_err());

        let mut token_launch = launch("codex", &["app-server", "--listen", "stdio://"]);
        token_launch.env.insert(
            CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
            "must-not-cross-manifest".to_string(),
        );
        assert!(validate_sidecar_manifest(&token_launch).is_err());
    }

    #[test]
    fn container_lease_debug_never_formats_bearer_token() {
        let token = "0123456789abcdef0123456789abcdef-secret";
        let lease = CodexContainerBridgeLease {
            inner: Arc::new(ContainerBridgeLeaseInner {
                endpoint: "ws://127.0.0.1:4567/codex".to_string(),
                bearer_token: token.to_string(),
                recovery_attachment_paths: Vec::new(),
                root_forwarded: Arc::new(AtomicBool::new(false)),
                writer: Arc::new(SharedControlWriter::new(Vec::<u8>::new())),
                cancelled: Arc::new(AtomicBool::new(false)),
                child: Mutex::new(None),
            }),
        };
        assert!(!format!("{lease:?}").contains(token));
    }
}
