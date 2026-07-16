//! Content-addressed recovery attachments.
//!
//! Source paths are used only while copying bytes. Persisted checkpoint
//! metadata contains a digest, basename, and byte count; it never retains an
//! arbitrary absolute source path.

use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};

use crate::bounded_file::BoundedRegularFile;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    create_private_dir_all, open_private_lock, open_private_new, sanitize_visible_text,
    sync_directory, RecoveryStore, RecoveryStoreFaultPoint, RecoveryStoreResult,
};

const CONTENT_ID_PREFIX: &str = "sha256:";
/// Maximum bytes retained for one recovery attachment (32 MiB).
pub const MAX_RECOVERY_ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024;
// Content-addressed storage normally contains far fewer files. These bounds
// allow large projects while keeping startup GC/repair metadata work finite.
const MAX_ATTACHMENT_PREFIX_ENTRIES: usize = 512;
const MAX_ATTACHMENT_BLOB_ENTRIES: usize = 16_384;
const MAX_ATTACHMENT_BLOB_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ATTACHMENT_TEMP_ENTRIES: usize = 4_096;
const MAX_ATTACHMENT_TEMP_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

type AttachmentBlobInventory = (Vec<PathBuf>, Vec<(PathBuf, String)>);

/// Durable reference to one copied attachment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RecoveryAttachmentRef {
    pub content_id: String,
    pub file_name: String,
    pub byte_len: u64,
}

impl RecoveryStore {
    /// Copy a regular file into the project-scoped content-addressed store.
    ///
    /// The copy is fsynced under a unique temporary name before publication.
    /// Identical bytes share one blob while each checkpoint may retain its own
    /// sanitized display basename.
    pub fn copy_attachment(&self, source: &Path) -> RecoveryStoreResult<RecoveryAttachmentRef> {
        self.with_attachment_inventory_lock(|| {
            self.prune_attachment_temp_files_unlocked()?;
            self.copy_attachment_unlocked(source)
        })
    }

    /// Copy one path while the caller owns the global attachment inventory
    /// lock. This is used by compound checkpoint operations that must keep blob
    /// publication and the referencing recovery event indivisible to GC.
    pub(super) fn copy_attachment_unlocked(
        &self,
        source: &Path,
    ) -> RecoveryStoreResult<RecoveryAttachmentRef> {
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_attachment_file_name)
            .transpose()?
            .unwrap_or_else(|| "attachment".to_string());
        self.copy_attachment_reader(
            file_name,
            open_attachment_source(source, MAX_RECOVERY_ATTACHMENT_BYTES)?,
        )
    }

    /// Import already-framed attachment bytes, as used by the container
    /// sidecar after reading a container-local image. The caller-provided name
    /// is reduced to a basename before any metadata is persisted.
    pub fn copy_attachment_bytes(
        &self,
        file_name: &str,
        bytes: &[u8],
    ) -> RecoveryStoreResult<RecoveryAttachmentRef> {
        self.with_attachment_inventory_lock(|| {
            self.prune_attachment_temp_files_unlocked()?;
            self.copy_attachment_bytes_unlocked(file_name, bytes)
        })
    }

    pub(super) fn copy_attachment_bytes_unlocked(
        &self,
        file_name: &str,
        bytes: &[u8],
    ) -> RecoveryStoreResult<RecoveryAttachmentRef> {
        if bytes.len() as u64 > MAX_RECOVERY_ATTACHMENT_BYTES {
            return Err(attachment_size_limit_error());
        }
        let file_name = sanitize_attachment_file_name(file_name)?;
        self.copy_attachment_reader(file_name, Cursor::new(bytes))
    }

    fn copy_attachment_reader(
        &self,
        file_name: String,
        mut input: impl Read,
    ) -> RecoveryStoreResult<RecoveryAttachmentRef> {
        let temp_dir = self.attachment_temp_dir();
        create_private_dir_all(&temp_dir)?;
        let temp_path = temp_dir.join(format!("{}.tmp", Uuid::new_v4().simple()));
        let output = open_private_new(&temp_path)?;
        let mut temp_guard = AttachmentTempGuard::new(output, temp_path.clone(), temp_dir.clone());
        let mut hasher = Sha256::new();
        let mut byte_len = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if byte_len.saturating_add(read as u64) > MAX_RECOVERY_ATTACHMENT_BYTES {
                return Err(attachment_size_limit_error());
            }
            hasher.update(&buffer[..read]);
            temp_guard.file_mut().write_all(&buffer[..read])?;
            byte_len = byte_len.saturating_add(read as u64);
        }
        temp_guard.sync_and_close()?;
        let digest = hex::encode(hasher.finalize());
        let reference = RecoveryAttachmentRef {
            content_id: format!("{CONTENT_ID_PREFIX}{digest}"),
            file_name,
            byte_len,
        };

        let blob_dir = self.attachment_blob_dir(&digest);
        create_private_dir_all(&blob_dir)?;
        let final_path = blob_dir.join(&digest);
        let lock_path = self.locks_dir().join(format!("attachment-{digest}.lock"));
        create_private_dir_all(&self.locks_dir())?;
        let lock = open_private_lock(&lock_path)?;
        FileExt::lock_exclusive(&lock)?;
        let publication = (|| -> RecoveryStoreResult<()> {
            if final_path.exists() {
                verify_blob(&final_path, &reference)?;
                fs::remove_file(&temp_path)?;
                temp_guard.disarm();
                sync_directory(&temp_dir)?;
                return Ok(());
            }
            fs::rename(&temp_path, &final_path)?;
            temp_guard.disarm();
            sync_directory(&blob_dir)?;
            sync_directory(&temp_dir)?;
            verify_blob(&final_path, &reference)?;
            self.inject_fault(RecoveryStoreFaultPoint::AfterAttachmentPublication)
        })();
        let unlock = FileExt::unlock(&lock);
        match (publication, unlock) {
            (Ok(()), Ok(())) => Ok(reference),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error.into()),
        }
    }

    /// Verify a durable attachment blob through one bounded no-follow handle.
    pub fn verify_attachment(&self, reference: &RecoveryAttachmentRef) -> RecoveryStoreResult<()> {
        let digest = validate_reference(reference)?;
        let path = self.attachment_blob_dir(digest).join(digest);
        verify_blob(&path, reference)
    }

    /// Read and verify durable attachment bytes through the same opened handle.
    ///
    /// This is the byte-consuming counterpart to [`Self::verify_attachment`].
    /// It deliberately does not return a verified path, which prevents callers
    /// from introducing a verify-then-reopen race.
    pub fn read_attachment_bytes(
        &self,
        reference: &RecoveryAttachmentRef,
        max_bytes: u64,
    ) -> RecoveryStoreResult<Vec<u8>> {
        let digest = validate_reference(reference)?;
        let max_bytes = max_bytes.min(MAX_RECOVERY_ATTACHMENT_BYTES);
        if reference.byte_len > max_bytes {
            return Err(attachment_size_limit_error_with(max_bytes));
        }
        let path = self.attachment_blob_dir(digest).join(digest);
        let bytes = open_attachment_source(&path, max_bytes)?.read_all()?;
        verify_blob_bytes(&bytes, reference)?;
        Ok(bytes)
    }

    /// Return a verified Host path for inclusion in a provider prompt.
    ///
    /// gwt never reopens this path for byte consumption. Callers that need
    /// bytes must use [`Self::read_attachment_bytes`] instead.
    pub(super) fn resolve_attachment_path(
        &self,
        reference: &RecoveryAttachmentRef,
    ) -> RecoveryStoreResult<PathBuf> {
        let digest = validate_reference(reference)?;
        let path = self.attachment_blob_dir(digest).join(digest);
        verify_blob(&path, reference)?;
        Ok(path)
    }

    /// Remove blobs that are no longer referenced by any unresolved record.
    /// Corrupt recovery inventory fails closed and leaves every blob intact.
    pub(super) fn prune_unreferenced_attachments(&self) -> RecoveryStoreResult<usize> {
        self.with_attachment_inventory_lock(|| self.prune_unreferenced_attachments_unlocked())
    }

    /// Remove unpublished attachment bytes left by a process/OS crash.
    ///
    /// Only gwt's UUID-named `.tmp` files are eligible. The inventory lock
    /// prevents a live attachment publication from being mistaken for an
    /// orphan while startup or GC performs the sweep.
    pub fn repair_attachment_inventory(&self) -> RecoveryStoreResult<usize> {
        self.with_attachment_inventory_lock(|| self.prune_attachment_temp_files_unlocked())
    }

    fn prune_unreferenced_attachments_unlocked(&self) -> RecoveryStoreResult<usize> {
        self.prune_attachment_temp_files_unlocked()?;
        let mut referenced = HashSet::new();
        for record in self.list()? {
            if let Some(checkpoint) = record.checkpoint {
                for attachment in checkpoint.attachment_refs {
                    referenced.insert(validate_reference(&attachment)?.to_string());
                }
            }
        }
        let root = self.attachment_blobs_dir();
        if !root.is_dir() {
            return Ok(0);
        }
        let (prefixes, blobs) = self.attachment_blob_inventory_with_limits(
            MAX_ATTACHMENT_PREFIX_ENTRIES,
            MAX_ATTACHMENT_BLOB_ENTRIES,
            MAX_ATTACHMENT_BLOB_TOTAL_BYTES,
        )?;
        let mut removed = 0;
        for (path, digest) in blobs {
            if !referenced.contains(&digest) {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        for prefix in prefixes {
            // Only one entry is requested here; this emptiness probe cannot
            // become an unbounded enumeration.
            if fs::read_dir(&prefix)?.next().is_none() {
                fs::remove_dir(prefix)?;
            }
        }
        if removed > 0 {
            sync_directory(&root)?;
        }
        Ok(removed)
    }

    pub(super) fn prune_attachment_temp_files_unlocked(&self) -> RecoveryStoreResult<usize> {
        let temp_dir = self.attachment_temp_dir();
        if !temp_dir.is_dir() {
            return Ok(0);
        }
        let owned_temp_files = self.attachment_temp_inventory_with_limits(
            MAX_ATTACHMENT_TEMP_ENTRIES,
            MAX_ATTACHMENT_TEMP_TOTAL_BYTES,
        )?;
        let mut removed = 0;
        for path in owned_temp_files {
            fs::remove_file(path)?;
            removed += 1;
        }
        if removed > 0 {
            sync_directory(&temp_dir)?;
        }
        Ok(removed)
    }

    fn attachment_blob_inventory_with_limits(
        &self,
        max_prefix_entries: usize,
        max_blob_entries: usize,
        max_total_bytes: u64,
    ) -> RecoveryStoreResult<AttachmentBlobInventory> {
        let root = self.attachment_blobs_dir();
        let mut prefixes = Vec::new();
        let mut blobs = Vec::new();
        let mut prefix_entries = 0_usize;
        let mut blob_entries = 0_usize;
        let mut total_bytes = 0_u64;
        for prefix in fs::read_dir(&root)? {
            let prefix = prefix?;
            prefix_entries = prefix_entries.saturating_add(1);
            if prefix_entries > max_prefix_entries {
                return Err(super::content_limit_error(
                    "recovery_attachment_prefix_inventory",
                    max_prefix_entries,
                    "entries",
                ));
            }
            if !prefix.file_type()?.is_dir() {
                continue;
            }
            let prefix_path = prefix.path();
            prefixes.push(prefix_path.clone());
            for entry in fs::read_dir(&prefix_path)? {
                let entry = entry?;
                blob_entries = blob_entries.saturating_add(1);
                if blob_entries > max_blob_entries {
                    return Err(super::content_limit_error(
                        "recovery_attachment_blob_inventory",
                        max_blob_entries,
                        "entries",
                    ));
                }
                if !entry.file_type()?.is_file() {
                    continue;
                }
                total_bytes = total_bytes
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| {
                        super::content_limit_error(
                            "recovery_attachment_blob_inventory",
                            usize::try_from(max_total_bytes).unwrap_or(usize::MAX),
                            "aggregate bytes",
                        )
                    })?;
                if total_bytes > max_total_bytes {
                    return Err(super::content_limit_error(
                        "recovery_attachment_blob_inventory",
                        usize::try_from(max_total_bytes).unwrap_or(usize::MAX),
                        "aggregate bytes",
                    ));
                }
                blobs.push((
                    entry.path(),
                    entry.file_name().to_string_lossy().into_owned(),
                ));
            }
        }
        Ok((prefixes, blobs))
    }

    fn attachment_temp_inventory_with_limits(
        &self,
        max_entries: usize,
        max_total_bytes: u64,
    ) -> RecoveryStoreResult<Vec<PathBuf>> {
        let temp_dir = self.attachment_temp_dir();
        let mut entry_count = 0_usize;
        let mut total_bytes = 0_u64;
        let mut owned = Vec::new();
        for entry in fs::read_dir(&temp_dir)? {
            let entry = entry?;
            entry_count = entry_count.saturating_add(1);
            if entry_count > max_entries {
                return Err(super::content_limit_error(
                    "recovery_attachment_temp_inventory",
                    max_entries,
                    "entries",
                ));
            }
            let file_type = entry.file_type()?;
            if file_type.is_file() {
                total_bytes = total_bytes
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| {
                        super::content_limit_error(
                            "recovery_attachment_temp_inventory",
                            usize::try_from(max_total_bytes).unwrap_or(usize::MAX),
                            "aggregate bytes",
                        )
                    })?;
                if total_bytes > max_total_bytes {
                    return Err(super::content_limit_error(
                        "recovery_attachment_temp_inventory",
                        usize::try_from(max_total_bytes).unwrap_or(usize::MAX),
                        "aggregate bytes",
                    ));
                }
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_owned_temp = file_type.is_file()
                && name.len() == 36
                && name.ends_with(".tmp")
                && name[..32].bytes().all(|byte| byte.is_ascii_hexdigit());
            if is_owned_temp {
                owned.push(entry.path());
            }
        }
        Ok(owned)
    }

    fn attachments_dir(&self) -> PathBuf {
        self.root.join("attachments")
    }

    fn attachment_blobs_dir(&self) -> PathBuf {
        self.attachments_dir().join("sha256")
    }

    fn attachment_temp_dir(&self) -> PathBuf {
        self.attachments_dir().join("tmp")
    }

    fn attachment_blob_dir(&self, digest: &str) -> PathBuf {
        self.attachment_blobs_dir().join(&digest[..2])
    }
}

struct AttachmentTempGuard {
    file: Option<File>,
    path: PathBuf,
    parent: PathBuf,
    armed: bool,
}

impl AttachmentTempGuard {
    fn new(file: File, path: PathBuf, parent: PathBuf) -> Self {
        Self {
            file: Some(file),
            path,
            parent,
            armed: true,
        }
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("temporary file remains open")
    }

    fn sync_and_close(&mut self) -> RecoveryStoreResult<()> {
        if let Some(file) = self.file.take() {
            file.sync_all()?;
        }
        Ok(())
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AttachmentTempGuard {
    fn drop(&mut self) {
        self.file.take();
        if self.armed && fs::remove_file(&self.path).is_ok() {
            let _ = sync_directory(&self.parent);
        }
    }
}

fn sanitize_attachment_file_name(value: &str) -> RecoveryStoreResult<String> {
    let sanitized = sanitize_visible_text(value);
    let sanitized = sanitized.trim();
    let basename = Path::new(sanitized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if sanitized.is_empty()
        || basename != sanitized
        || sanitized.contains('/')
        || sanitized.contains('\\')
        || sanitized.contains(['\n', '\t'])
        || matches!(sanitized, "." | "..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment file name must be a non-empty basename",
        )
        .into());
    }
    Ok(sanitized.to_string())
}

pub(super) fn validate_reference(reference: &RecoveryAttachmentRef) -> RecoveryStoreResult<&str> {
    let Some(digest) = reference.content_id.strip_prefix(CONTENT_ID_PREFIX) else {
        return Err(invalid_reference());
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_reference());
    }
    if reference.file_name.trim().is_empty()
        || reference.byte_len > MAX_RECOVERY_ATTACHMENT_BYTES
        || reference.file_name.contains(['\n', '\t'])
        || Path::new(&reference.file_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(reference.file_name.as_str())
    {
        return Err(invalid_reference());
    }
    Ok(digest)
}

fn attachment_size_limit_error() -> super::RecoveryStoreError {
    attachment_size_limit_error_with(MAX_RECOVERY_ATTACHMENT_BYTES)
}

fn attachment_size_limit_error_with(max_bytes: u64) -> super::RecoveryStoreError {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("recovery attachment exceeds the {max_bytes} byte size limit"),
    )
    .into()
}

fn invalid_reference() -> super::RecoveryStoreError {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid recovery attachment reference",
    )
    .into()
}

fn verify_blob(path: &Path, reference: &RecoveryAttachmentRef) -> RecoveryStoreResult<()> {
    let expected = validate_reference(reference)?;
    let mut file = open_attachment_source(path, MAX_RECOVERY_ATTACHMENT_BYTES)?;
    let mut hasher = Sha256::new();
    let mut byte_len = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        byte_len = byte_len.saturating_add(read as u64);
    }
    let actual = hex::encode(hasher.finalize());
    if actual != expected || byte_len != reference.byte_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery attachment content failed digest verification",
        )
        .into());
    }
    Ok(())
}

fn verify_blob_bytes(bytes: &[u8], reference: &RecoveryAttachmentRef) -> RecoveryStoreResult<()> {
    let expected = validate_reference(reference)?;
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != expected || bytes.len() as u64 != reference.byte_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery attachment content failed digest verification",
        )
        .into());
    }
    Ok(())
}

/// Read a host attachment without following a path that became a symlink.
///
/// The file handle is opened first with the platform's no-follow flag. Its
/// own metadata, rather than pre-open path metadata, controls the regular-file
/// and size checks. A bounded stream then catches a file that grows after the
/// handle was opened without allocating beyond the recovery attachment cap.
pub fn read_recovery_attachment_bytes(path: &Path) -> RecoveryStoreResult<Vec<u8>> {
    read_recovery_attachment_bytes_with_limit(path, MAX_RECOVERY_ATTACHMENT_BYTES)
}

/// Read a host attachment with a caller-specific limit no greater than the
/// durable recovery attachment maximum.
pub fn read_recovery_attachment_bytes_with_limit(
    path: &Path,
    max_bytes: u64,
) -> RecoveryStoreResult<Vec<u8>> {
    let max_bytes = max_bytes.min(MAX_RECOVERY_ATTACHMENT_BYTES);
    open_attachment_source(path, max_bytes)?
        .read_all()
        .map_err(Into::into)
}

fn open_attachment_source(path: &Path, max_bytes: u64) -> RecoveryStoreResult<BoundedRegularFile> {
    BoundedRegularFile::open(path, max_bytes, "attachment source").map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingReader {
        delivered: bool,
    }

    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.delivered {
                return Err(io::Error::other("injected attachment read failure"));
            }
            self.delivered = true;
            buffer[..4].copy_from_slice(b"data");
            Ok(4)
        }
    }

    #[test]
    fn attachment_temp_guard_removes_partial_bytes_after_io_failure() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("store"));

        assert!(store
            .copy_attachment_reader(
                "evidence.bin".to_string(),
                FailingReader { delivered: false },
            )
            .is_err());

        let temp_dir = store.attachment_temp_dir();
        assert!(temp_dir.is_dir());
        assert!(fs::read_dir(temp_dir).unwrap().next().is_none());
    }

    #[test]
    fn startup_inventory_repair_removes_only_owned_orphan_temp_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("store"));
        let temp_dir = store.attachment_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let orphan = temp_dir.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.tmp");
        let unrelated = temp_dir.join("keep.txt");
        fs::write(&orphan, b"private partial bytes").unwrap();
        fs::write(&unrelated, b"not a gwt temp name").unwrap();

        assert_eq!(store.repair_attachment_inventory().unwrap(), 1);
        assert!(!orphan.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn attachment_gc_inventories_fail_before_any_overflow_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("store"));
        let prefix = store.attachment_blobs_dir().join("aa");
        fs::create_dir_all(&prefix).unwrap();
        let first = prefix.join("first");
        let second = prefix.join("second");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();

        let blob_error = store
            .attachment_blob_inventory_with_limits(10, 1, 100)
            .expect_err("blob overflow must fail the complete inventory");
        assert!(blob_error.to_string().contains("blob_inventory"));
        assert!(first.exists() && second.exists());

        let temp_dir = store.attachment_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();
        let owned = temp_dir.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.tmp");
        let other = temp_dir.join("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.tmp");
        fs::write(&owned, b"one").unwrap();
        fs::write(&other, b"two").unwrap();
        let temp_error = store
            .attachment_temp_inventory_with_limits(1, 100)
            .expect_err("temp overflow must fail the complete inventory");
        assert!(temp_error.to_string().contains("temp_inventory"));
        assert!(owned.exists() && other.exists());
    }

    #[test]
    fn opened_attachment_handle_is_not_redirected_by_a_later_path_swap() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("evidence.txt");
        let replacement = temp.path().join("replacement.txt");
        fs::write(&source, b"original").unwrap();
        fs::write(&replacement, b"replacement").unwrap();
        let mut opened = open_attachment_source(&source, MAX_RECOVERY_ATTACHMENT_BYTES).unwrap();

        fs::remove_file(&source).unwrap();
        fs::rename(&replacement, &source).unwrap();
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes).unwrap();

        assert_eq!(bytes, b"original");
        assert_eq!(fs::read(source).unwrap(), b"replacement");
    }
}
