//! Private, offline coordinator storage. The offline admin executable supplies
//! a validated fixed directory in private gwt home and must keep its ancestors
//! trusted and non-replaceable. The separate
//! lock file must never be renamed/unlinked; a future server must retain this
//! same guard for its entire lifetime. macOS extended ACL entries are rejected;
//! Windows requires a later ACL adapter.

use super::{
    encode, CoordinatorRegistry, CredentialError, CredentialIdentity, PreparedCredentialUpdate,
    MAX_REGISTRY_BYTES,
};
use crate::mobile_connection::HostBinding;
use gwt_core::repo_hash::ProjectKey;
use serde::Serialize;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_CREDENTIAL_BYTES: usize = 128 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("mobile storage requires owned 0700 directories and owned, single-link regular 0600 files, with no extended ACL entries on macOS; repair storage before reopening")]
    UnsafeStorage,
    #[error("mobile coordinator storage is locked; stop the coordinator before administration")]
    Busy,
    #[error("private mobile storage is unsupported on this platform")]
    UnsupportedPrivateStorage,
    #[error("invalid mobile coordinator HTTPS origin")]
    InvalidOrigin,
    #[error("mobile Host credential file exceeds the 128 KiB limit")]
    CredentialTooLarge,
    #[error("mobile registry commit outcome is uncertain; drop the store and explicitly reopen to inspect current authority")]
    CommitUncertain,
    #[error("mobile store is unusable after a failed checkpoint or uncertain commit; drop it and explicitly reopen")]
    Unusable,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("mobile private storage I/O failed: {0}")]
    Io(#[from] io::Error),
}

/// Reject repository markers even when their symlink targets are unavailable.
/// Callers supply a canonical directory whose ancestors are trusted.
pub(crate) fn has_repository_marker(directory: &Path) -> Result<bool, StoreError> {
    fn entry_present(path: &Path) -> Result<bool, StoreError> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    for ancestor in directory.ancestors() {
        if entry_present(&ancestor.join(".git"))? {
            return Ok(true);
        }
        let head = entry_present(&ancestor.join("HEAD"))?;
        let refs = entry_present(&ancestor.join("refs"))?;
        if head && refs {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Public metadata only; the bearer is returned exclusively through its file.
#[derive(Debug)]
pub struct IssuedCredential {
    pub identity: CredentialIdentity,
    pub path: PathBuf,
}

struct LockedDirectory {
    path: PathBuf,
    directory: File,
    lock: File,
}

impl Drop for LockedDirectory {
    fn drop(&mut self) {
        // A fork/dup may retain the open-file description after our File closes.
        let _ = fs2::FileExt::unlock(&self.lock);
    }
}

impl LockedDirectory {
    fn acquire(path: &Path) -> Result<Self, StoreError> {
        let directory = open_directory(path)?;
        let lock = open_private_file(&path.join("registry.lock"), true)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                StoreError::Busy
            } else {
                StoreError::Io(error)
            }
        })?;
        let guard = Self {
            path: path.to_owned(),
            directory,
            lock,
        };
        guard.check()?;
        guard.lock.sync_all()?;
        guard.directory.sync_all()?;
        Ok(guard)
    }

    fn check(&self) -> Result<(), StoreError> {
        let directory = open_directory(&self.path)?;
        let lock = open_private_file(&self.path.join("registry.lock"), false)?;
        if !same_file(&directory, &self.directory)? || !same_file(&lock, &self.lock)? {
            return Err(StoreError::UnsafeStorage);
        }
        Ok(())
    }

    fn temp(&self, bytes: &[u8]) -> Result<tempfile::NamedTempFile, StoreError> {
        self.check()?;
        // Establish private mode before writing, including under a restrictive umask.
        let mut temp = tempfile::Builder::new()
            .prefix(".mobile-")
            .tempfile_in(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        Ok(temp)
    }

    fn read_registry(&self, now: u64) -> Result<(CoordinatorRegistry, Vec<u8>), StoreError> {
        self.check()?;
        let file = open_private_file(&self.path.join("registry.json"), false)?;
        let mut bytes = Vec::new();
        file.take(MAX_REGISTRY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        let registry = CoordinatorRegistry::from_json(&bytes, now)?;
        Ok((registry, bytes))
    }
}

/// Owns the clock watermark/latch and the single writer lock. Registration
/// actors checkpoint trusted observations before processing requests; raw
/// registry borrows alone do not establish durability across restarts.
pub struct CoordinatorStore {
    guard: LockedDirectory,
    registry: CoordinatorRegistry,
    usable: bool,
    persisted_wall: Option<u64>,
}

impl CoordinatorStore {
    /// Explicit initialization only. Never replaces an existing registry.
    pub fn initialize(directory: &Path, now: u64) -> Result<Self, StoreError> {
        let guard = LockedDirectory::acquire(directory)?;
        let bytes = encode(&CoordinatorRegistry::empty(now).stored)?;
        guard
            .temp(&bytes)?
            .persist_noclobber(guard.path.join("registry.json"))
            .map_err(|e| e.error)?;
        let registry = (|| {
            guard.directory.sync_all()?;
            guard.read_registry(now).map(|(registry, _)| registry)
        })()
        .map_err(|_: StoreError| StoreError::CommitUncertain)?;
        Ok(Self {
            guard,
            registry,
            usable: true,
            persisted_wall: Some(now),
        })
    }

    /// Missing, corrupt or unsafe files are errors, never an empty fallback.
    pub fn open(directory: &Path, now: u64) -> Result<Self, StoreError> {
        let guard = LockedDirectory::acquire(directory)?;
        let (registry, _) = guard.read_registry(now)?;
        Ok(Self {
            guard,
            registry,
            usable: true,
            // Opening advances RAM time; the first checkpoint must save it.
            persisted_wall: None,
        })
    }

    fn check(&self) -> Result<(), StoreError> {
        if !self.usable {
            return Err(StoreError::Unusable);
        }
        self.guard.check()
    }

    /// A trusted registration adapter may borrow authority only with this guard.
    pub fn registry(&mut self) -> Result<&mut CoordinatorRegistry, StoreError> {
        self.check()?;
        Ok(&mut self.registry)
    }

    /// Persist trusted time before request validation, including rejected
    /// requests. Open's first observation is saved even in the same second;
    /// subsequent same-second calls still check the guard and clock latch.
    /// Any failure requires explicit reopen, never reuse of this authority.
    pub(super) fn checkpoint(&mut self, now: u64) -> Result<(), StoreError> {
        let result = (|| {
            self.check()?;
            self.registry.observe(now)?;
            if self.persisted_wall != Some(now) {
                let snapshot = encode(&self.registry.stored)?;
                self.commit_registry(&snapshot, now, File::sync_all)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.usable = false;
        }
        result
    }

    pub fn issue(
        &mut self,
        origin: &str,
        host_id: Uuid,
        projects: &[ProjectKey],
        expires_at: u64,
        now: u64,
    ) -> Result<IssuedCredential, StoreError> {
        self.check()?;
        let candidate = self
            .registry
            .prepare_issue(host_id, projects, expires_at, now)?;
        self.persist_credential(candidate, origin, now)
    }

    pub fn rotate(
        &mut self,
        origin: &str,
        credential_id: Uuid,
        expires_at: u64,
        now: u64,
    ) -> Result<IssuedCredential, StoreError> {
        self.check()?;
        let candidate = self
            .registry
            .prepare_rotate(credential_id, expires_at, now)?;
        self.persist_credential(candidate, origin, now)
    }

    pub fn revoke(&mut self, credential_id: Uuid, now: u64) -> Result<(), StoreError> {
        self.check()?;
        let snapshot = self.registry.prepare_revoke(credential_id, now)?;
        self.commit_registry(&snapshot, now, File::sync_all)
    }

    fn persist_credential(
        &mut self,
        candidate: PreparedCredentialUpdate,
        origin: &str,
        now: u64,
    ) -> Result<IssuedCredential, StoreError> {
        self.check()?;
        let credential = candidate.credential();
        let identity = *credential.identity();
        let binding = HostBinding::new(&identity.host_id.to_string(), origin)
            .map_err(|_| StoreError::InvalidOrigin)?;
        let stored = CoordinatorRegistry::from_json(candidate.registry_snapshot(), now)?;
        let record = stored
            .stored
            .records
            .iter()
            .find(|r| r.identity == identity)
            .ok_or(CredentialError::InvalidRegistry)?;
        #[derive(Serialize)]
        struct PrivateCredential<'a> {
            schema_version: u32,
            identity: CredentialIdentity,
            binding: HostBinding,
            projects: &'a [String],
            issued_at: u64,
            expires_at: u64,
            secret: String,
        }
        let bytes = serde_json::to_vec(&PrivateCredential {
            schema_version: 1,
            identity,
            binding,
            projects: &record.projects,
            issued_at: record.issued_at,
            expires_at: record.expires_at,
            secret: hex::encode(credential.secret_bytes()),
        })
        .map_err(|_| CredentialError::InvalidRegistry)?;
        if bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(StoreError::CredentialTooLarge);
        }
        let path = self
            .guard
            .path
            .join(format!("host-{}.json", identity.credential_id));
        self.guard
            .temp(&bytes)?
            .persist_noclobber(&path)
            .map_err(|e| e.error)?;
        self.guard.directory.sync_all()?;
        // A failure before registry replacement leaves an unauthorized orphan.
        // After replacement, an error must never be reported as old authority.
        self.commit_registry(candidate.registry_snapshot(), now, File::sync_all)?;
        Ok(IssuedCredential { identity, path })
    }

    fn commit_registry(
        &mut self,
        snapshot: &[u8],
        now: u64,
        sync_directory: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<(), StoreError> {
        self.check()?;
        CoordinatorRegistry::from_json(snapshot, now)?;
        // Do not silently replace an unsafe existing registry with a fresh one.
        open_private_file(&self.guard.path.join("registry.json"), false)?;
        let temp = self.guard.temp(snapshot)?;
        self.usable = false;
        // From this point, conservatively require reopen on every failure.
        let registry = (|| -> Result<CoordinatorRegistry, StoreError> {
            temp.persist(self.guard.path.join("registry.json"))
                .map_err(|e| e.error)?;
            sync_directory(&self.guard.directory)?;
            let (registry, bytes) = self.guard.read_registry(now)?;
            if bytes != snapshot {
                return Err(StoreError::CommitUncertain);
            }
            Ok(registry)
        })()
        .map_err(|_| StoreError::CommitUncertain)?;
        self.registry = registry;
        self.persisted_wall = Some(self.registry.stored.last_observed);
        self.usable = true;
        Ok(())
    }
}

/// Shared producer/consumer bound and private-file checks. Ancestors must be
/// trusted by the caller; the immediate directory and file are rechecked after
/// reading. No registry lock or registration authority is acquired.
#[cfg(unix)]
pub(crate) fn read_private_credential(path: &Path) -> Result<Vec<u8>, StoreError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(StoreError::UnsafeStorage)?;
    let directory = open_directory(parent)?;
    let file = open_private_file(path, false)?;
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_CREDENTIAL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CREDENTIAL_BYTES {
        return Err(StoreError::CredentialTooLarge);
    }
    if !same_file(&directory, &open_directory(parent)?)?
        || !same_file(&file, &open_private_file(path, false)?)?
    {
        return Err(StoreError::UnsafeStorage);
    }
    Ok(bytes)
}

#[cfg(not(unix))]
pub(crate) fn read_private_credential(_: &Path) -> Result<Vec<u8>, StoreError> {
    Err(StoreError::UnsupportedPrivateStorage)
}

#[cfg(unix)]
pub(crate) fn open_directory(path: &Path) -> Result<File, StoreError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_NONBLOCK)
        .open(path)?;
    let meta = directory.metadata()?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o7777 != 0o700 {
        return Err(StoreError::UnsafeStorage);
    }
    #[cfg(target_os = "macos")]
    macos_acl::require_empty(&directory)?;
    Ok(directory)
}

#[cfg(unix)]
pub(super) fn open_private_file(path: &Path, create: bool) -> Result<File, StoreError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o7777 != 0o600
        || meta.nlink() != 1
    {
        return Err(StoreError::UnsafeStorage);
    }
    #[cfg(target_os = "macos")]
    macos_acl::require_empty(&file)?;
    Ok(file)
}

#[cfg(target_os = "macos")]
mod macos_acl {
    use super::{File, StoreError};
    use std::{io, os::fd::AsRawFd};

    // Public Darwin ACL API, absent from libc's Rust bindings. Compare public
    // external-representation sizes instead of depending on the private layout.
    #[link(name = "System")]
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, acl_type: libc::c_int) -> *mut libc::c_void;
        fn acl_init(count: libc::c_int) -> *mut libc::c_void;
        fn acl_size(acl: *mut libc::c_void) -> libc::ssize_t;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }

    struct Acl(*mut libc::c_void);

    impl Drop for Acl {
        fn drop(&mut self) {
            // Each value owns one successful allocation from the ACL API.
            unsafe { acl_free(self.0) };
        }
    }

    pub(super) fn require_empty(file: &File) -> Result<(), StoreError> {
        // ACL_TYPE_EXTENDED from the public macOS sys/acl.h.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
        if acl.is_null() {
            let error = io::Error::last_os_error();
            // Darwin projects an absent ACL property as NULL + ENOENT even
            // for a valid open FD. Other query failures must remain errors.
            return if error.raw_os_error() == Some(libc::ENOENT) {
                Ok(())
            } else {
                Err(error.into())
            };
        }
        let acl = Acl(acl);
        let empty = unsafe { acl_init(0) };
        if empty.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let empty = Acl(empty);
        let actual_size = unsafe { acl_size(acl.0) };
        if actual_size < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let empty_size = unsafe { acl_size(empty.0) };
        if empty_size < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if empty_size == 0 || actual_size != empty_size {
            return Err(StoreError::UnsafeStorage);
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn same_file(left: &File, right: &File) -> Result<bool, StoreError> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(not(unix))]
pub(crate) fn open_directory(_: &Path) -> Result<File, StoreError> {
    Err(StoreError::UnsupportedPrivateStorage)
}
#[cfg(not(unix))]
pub(super) fn open_private_file(_: &Path, _: bool) -> Result<File, StoreError> {
    Err(StoreError::UnsupportedPrivateStorage)
}
#[cfg(not(unix))]
pub(crate) fn same_file(_: &File, _: &File) -> Result<bool, StoreError> {
    Err(StoreError::UnsupportedPrivateStorage)
}

#[cfg(all(test, not(unix)))]
#[test]
fn unsupported_platform_never_initializes_private_storage() {
    assert!(matches!(
        CoordinatorStore::initialize(Path::new("unused"), 100),
        Err(StoreError::UnsupportedPrivateStorage)
    ));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::mobile_connection::HostBinding;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    fn project() -> ProjectKey {
        ProjectKey::parse("0123456789abcdef").unwrap()
    }

    fn private_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn read_secret(path: &Path) -> (serde_json::Value, Vec<u8>) {
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let secret = hex::decode(data["secret"].as_str().unwrap()).unwrap();
        (data, secret)
    }

    #[test]
    fn repository_markers_require_both_bare_entries_and_propagate_metadata_errors() {
        let dir = private_dir();
        let child = dir.path().join("child");
        std::fs::create_dir(&child).unwrap();
        assert!(!has_repository_marker(&child).unwrap());
        let head = dir.path().join("HEAD");
        let refs = dir.path().join("refs");
        symlink("missing-head", &head).unwrap();
        assert!(!has_repository_marker(&child).unwrap());
        symlink("missing-refs", &refs).unwrap();
        assert!(has_repository_marker(&child).unwrap());
        std::fs::remove_file(head).unwrap();
        assert!(!has_repository_marker(&child).unwrap());
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, b"").unwrap();
        assert!(matches!(
            has_repository_marker(&file),
            Err(StoreError::Io(_))
        ));
    }

    #[test]
    fn oversized_credential_is_refused_before_secret_or_registry_publication() {
        let dir = private_dir();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let first = store
            .issue(
                "https://coordinator.example",
                Uuid::new_v4(),
                &[project()],
                200,
                100,
            )
            .unwrap();
        let before = std::fs::read(dir.path().join("registry.json")).unwrap();
        let files_before = std::fs::read_dir(dir.path()).unwrap().count();
        let origin = format!("https://{}.example", "a".repeat(128 * 1024));
        assert!(matches!(
            store.issue(&origin, Uuid::new_v4(), &[project()], 200, 100),
            Err(StoreError::CredentialTooLarge)
        ));
        assert_eq!(
            std::fs::read(dir.path().join("registry.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), files_before);
        let (_, secret) = read_secret(&first.path);
        store
            .registry()
            .unwrap()
            .verify(&first.identity, &project(), &secret, 100)
            .unwrap();
    }

    #[test]
    fn private_issue_rotate_revoke_and_reopen_preserve_binding_and_tombstones() {
        let dir = private_dir();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let host = Uuid::new_v4();
        let first = store
            .issue("https://coordinator.example", host, &[project()], 200, 100)
            .unwrap();
        let (envelope, secret) = read_secret(&first.path);
        let original_secret_file = std::fs::read(&first.path).unwrap();
        assert_eq!(envelope["schema_version"], 1);
        assert_eq!(envelope["identity"]["host_id"], host.to_string());
        let binding: HostBinding = serde_json::from_value(envelope["binding"].clone()).unwrap();
        assert_eq!(
            binding,
            HostBinding::new(&host.to_string(), "https://coordinator.example").unwrap()
        );
        assert_eq!(
            envelope["projects"],
            serde_json::json!([project().as_str()])
        );
        assert_eq!(envelope["issued_at"], 100);
        assert_eq!(envelope["expires_at"], 200);
        assert_eq!(
            std::fs::metadata(&first.path).unwrap().mode() & 0o777,
            0o600
        );
        assert!(!String::from_utf8_lossy(
            &std::fs::read(dir.path().join("registry.json")).unwrap()
        )
        .contains(envelope["secret"].as_str().unwrap()));
        store
            .registry()
            .unwrap()
            .verify(&first.identity, &project(), &secret, 100)
            .unwrap();
        let second = store
            .rotate(
                "https://coordinator.example",
                first.identity.credential_id,
                300,
                110,
            )
            .unwrap();
        let (_, second_secret) = read_secret(&second.path);
        assert_eq!(std::fs::read(&first.path).unwrap(), original_secret_file);
        assert_eq!(
            store
                .registry()
                .unwrap()
                .verify(&first.identity, &project(), &secret, 110),
            Err(CredentialError::Unauthorized)
        );
        store
            .registry()
            .unwrap()
            .verify(&second.identity, &project(), &second_secret, 110)
            .unwrap();
        store.revoke(second.identity.credential_id, 120).unwrap();
        drop(store);
        let mut reopened = CoordinatorStore::open(dir.path(), 120).unwrap();
        assert_eq!(
            reopened
                .registry()
                .unwrap()
                .verify(&second.identity, &project(), &second_secret, 120),
            Err(CredentialError::Unauthorized)
        );
        let snapshot: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("registry.json")).unwrap())
                .unwrap();
        assert_eq!(snapshot["records"].as_array().unwrap().len(), 2);
        assert!(snapshot["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["revoked"] == true));
        assert!(matches!(
            CoordinatorStore::initialize(dir.path(), 120),
            Err(StoreError::Busy)
        ));
    }

    #[test]
    fn lock_survives_registry_replacement_and_is_released_only_on_drop() {
        let dir = private_dir();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let lock = dir.path().join("registry.lock");
        let lock_inode = std::fs::metadata(&lock).unwrap().ino();
        let registry_inode = std::fs::metadata(dir.path().join("registry.json"))
            .unwrap()
            .ino();
        store
            .issue(
                "https://coordinator.example",
                Uuid::new_v4(),
                &[project()],
                200,
                100,
            )
            .unwrap();
        assert_ne!(
            registry_inode,
            std::fs::metadata(dir.path().join("registry.json"))
                .unwrap()
                .ino()
        );
        assert_eq!(lock_inode, std::fs::metadata(&lock).unwrap().ino());
        assert!(matches!(
            CoordinatorStore::open(dir.path(), 100),
            Err(StoreError::Busy)
        ));
        // Model a fork's shared open-file description without a timing race.
        let inherited_lock = store.guard.lock.try_clone().unwrap();
        drop(store);
        let reopened = CoordinatorStore::open(dir.path(), 100).unwrap();
        drop(inherited_lock);
        assert!(matches!(
            CoordinatorStore::open(dir.path(), 100),
            Err(StoreError::Busy)
        ));
        drop(reopened);
        assert_eq!(lock_inode, std::fs::metadata(&lock).unwrap().ino());
        assert!(CoordinatorStore::initialize(dir.path(), 100).is_err());
    }

    #[test]
    fn private_storage_rejects_bad_paths_without_fallback_and_recovers_after_repair() {
        let dir = private_dir();
        assert!(CoordinatorStore::open(dir.path(), 100).is_err());
        drop(CoordinatorStore::initialize(dir.path(), 100).unwrap());
        let registry = dir.path().join("registry.json");
        let original = std::fs::read(&registry).unwrap();
        std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            CoordinatorStore::open(dir.path(), 100),
            Err(StoreError::UnsafeStorage)
        ));
        std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600)).unwrap();
        let alias = dir.path().join("alias");
        std::fs::hard_link(&registry, &alias).unwrap();
        let hard_link_result = CoordinatorStore::open(dir.path(), 100);
        assert!(
            matches!(&hard_link_result, Err(StoreError::UnsafeStorage)),
            "expected a private hard-link refusal, got {:?}",
            hard_link_result.as_ref().err()
        );
        std::fs::remove_file(&alias).unwrap();
        std::fs::rename(&registry, &alias).unwrap();
        symlink(&alias, &registry).unwrap();
        assert!(CoordinatorStore::open(dir.path(), 100).is_err());
        std::fs::remove_file(&registry).unwrap();
        std::fs::create_dir(&registry).unwrap();
        assert!(CoordinatorStore::open(dir.path(), 100).is_err());
        std::fs::remove_dir(&registry).unwrap();
        std::fs::rename(&alias, &registry).unwrap();
        std::fs::write(&registry, b"{}").unwrap();
        assert!(CoordinatorStore::open(dir.path(), 100).is_err());
        std::fs::write(&registry, &original).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            CoordinatorStore::open(dir.path(), 100),
            Err(StoreError::UnsafeStorage)
        ));
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let parent = private_dir();
        let link = parent.path().join("link");
        symlink(dir.path(), &link).unwrap();
        assert!(CoordinatorStore::open(&link, 100).is_err());
        drop(CoordinatorStore::open(dir.path(), 100).unwrap());
        assert_eq!(std::fs::read(&registry).unwrap(), original);
    }

    #[test]
    fn failed_output_never_publishes_candidate_and_preserves_clock_observations() {
        let dir = private_dir();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let candidate = store
            .registry()
            .unwrap()
            .prepare_issue(Uuid::new_v4(), &[project()], 200, 110)
            .unwrap();
        let identity = *candidate.credential().identity();
        let output = dir
            .path()
            .join(format!("host-{}.json", identity.credential_id));
        std::fs::write(&output, b"existing private file").unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o600)).unwrap();
        let old = std::fs::read(dir.path().join("registry.json")).unwrap();
        assert!(store
            .persist_credential(candidate, "https://coordinator.example", 110)
            .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"existing private file");
        assert_eq!(
            std::fs::read(dir.path().join("registry.json")).unwrap(),
            old
        );
        assert_eq!(
            store
                .registry()
                .unwrap()
                .verify(&identity, &project(), &[0; 32], 110),
            Err(CredentialError::Unauthorized)
        );
        assert!(matches!(
            store.issue(
                "https://coordinator.example",
                Uuid::new_v4(),
                &[project()],
                200,
                109
            ),
            Err(StoreError::Credential(CredentialError::ClockRollback))
        ));
        assert!(matches!(
            store.issue(
                "https://coordinator.example",
                Uuid::new_v4(),
                &[project()],
                200,
                111
            ),
            Err(StoreError::Credential(CredentialError::ClockRollback))
        ));
    }

    #[test]
    fn registry_precommit_failure_leaves_new_secret_inactive_and_old_credential_valid() {
        let dir = private_dir();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let first = store
            .issue(
                "https://coordinator.example",
                Uuid::new_v4(),
                &[project()],
                200,
                100,
            )
            .unwrap();
        let (_, old_secret) = read_secret(&first.path);
        let registry = dir.path().join("registry.json");
        let old = std::fs::read(&registry).unwrap();
        std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert!(matches!(
            store.rotate(
                "https://coordinator.example",
                first.identity.credential_id,
                300,
                110
            ),
            Err(StoreError::UnsafeStorage)
        ));
        assert_eq!(std::fs::read(&registry).unwrap(), old);
        let orphan = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("host-")
                    && p != &first.path
            })
            .unwrap();
        let (envelope, secret) = read_secret(&orphan);
        let identity: CredentialIdentity =
            serde_json::from_value(envelope["identity"].clone()).unwrap();
        assert_eq!(
            store
                .registry()
                .unwrap()
                .verify(&identity, &project(), &secret, 110),
            Err(CredentialError::Unauthorized)
        );
        store
            .registry()
            .unwrap()
            .verify(&first.identity, &project(), &old_secret, 110)
            .unwrap();
        std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(store);
        CoordinatorStore::open(dir.path(), 110)
            .unwrap()
            .registry()
            .unwrap()
            .verify(&first.identity, &project(), &old_secret, 110)
            .unwrap();
    }

    #[test]
    fn post_rename_sync_failure_is_uncertain_and_requires_explicit_reopen() {
        let dir = private_dir();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let candidate = store
            .registry()
            .unwrap()
            .prepare_issue(Uuid::new_v4(), &[project()], 200, 110)
            .unwrap();
        let identity = *candidate.credential().identity();
        let secret = candidate.credential().secret_bytes().to_vec();
        // Exercise a real atomic replacement, then fail the directory sync.
        assert!(matches!(
            store.commit_registry(candidate.registry_snapshot(), 110, |_| Err(
                io::Error::other("injected directory sync failure")
            )),
            Err(StoreError::CommitUncertain)
        ));
        assert!(matches!(store.registry(), Err(StoreError::Unusable)));
        assert!(matches!(
            store.revoke(identity.credential_id, 120),
            Err(StoreError::Unusable)
        ));
        drop(store);
        let mut reopened = CoordinatorStore::open(dir.path(), 110).unwrap();
        reopened
            .registry()
            .unwrap()
            .verify(&identity, &project(), &secret, 110)
            .unwrap();
        reopened.revoke(identity.credential_id, 120).unwrap();
    }
}
