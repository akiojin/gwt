//! Unpublished reader and explicit private import for Host credentials (Issue #6476).
//!
//! The reader checks its immediate parent and file; the import API additionally
//! enforces fixed private directories below the caller-selected user home.
//! Source and home ancestors must be trusted and non-replaceable. Loading proves neither issuance nor
//! registry liveness: orphaned, rotated and revoked files may still parse.
//! Authenticated coordinator registration must check current authority before
//! use. Import only copies validated bytes to fixed private storage; it never
//! selects an active credential, opens a network connection or enables the PC.

use crate::mobile_connection::HostBinding;
use crate::mobile_coordinator::{
    storage::{read_private_credential, StoreError},
    CredentialIdentity,
};
use gwt_core::repo_hash::ProjectKey;
use serde::Deserialize;
use std::{collections::HashSet, path::Path};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum CredentialLoadError {
    #[error("invalid mobile Host credential file")]
    InvalidCredential,
    #[error("mobile Host credential does not match the expected Host and origin")]
    BindingMismatch,
    #[error("mobile Host credential does not allow the expected Project")]
    ProjectDenied,
    #[error("mobile Host credential is not yet valid")]
    NotYetValid,
    #[error("mobile Host credential expired")]
    Expired,
    #[error(transparent)]
    Storage(#[from] StoreError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCredential {
    schema_version: u32,
    identity: CredentialIdentity,
    binding: HostBinding,
    projects: Vec<String>,
    issued_at: u64,
    expires_at: u64,
    #[serde(deserialize_with = "crate::mobile_coordinator::borrowed_hex_secret")]
    secret: Zeroizing<[u8; 32]>,
}

/// Deliberately not Debug, Clone or Serialize. Local parsing is not proof that
/// the coordinator accepts this generation; the bearer must only be sent to
/// the configured, TLS-authenticated coordinator's Host registration endpoint.
pub struct LoadedHostCredential {
    identity: CredentialIdentity,
    binding: HostBinding,
    project: ProjectKey,
    expires_at: u64,
    secret: Zeroizing<[u8; 32]>,
}

impl LoadedHostCredential {
    pub fn identity(&self) -> &CredentialIdentity {
        &self.identity
    }

    pub fn binding(&self) -> &HostBinding {
        &self.binding
    }

    pub fn project(&self) -> &ProjectKey {
        &self.project
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn secret_bytes(&self) -> &[u8; 32] {
        &self.secret
    }
}

/// Read the existing private schema without modifying it or falling back to
/// another credential. `now` is trusted Unix time; this does not supply clock
/// rollback history across restarts or replace the live registration lease.
pub fn load_host_credential(
    path: &Path,
    expected_binding: &HostBinding,
    expected_project: &ProjectKey,
    now: u64,
) -> Result<LoadedHostCredential, CredentialLoadError> {
    let bytes = read_private_credential(path)?;
    decode_host_credential(&bytes, expected_binding, expected_project, now)
}

pub(super) fn decode_host_credential(
    bytes: &[u8],
    expected_binding: &HostBinding,
    expected_project: &ProjectKey,
    now: u64,
) -> Result<LoadedHostCredential, CredentialLoadError> {
    let stored: StoredCredential =
        serde_json::from_slice(bytes).map_err(|_| CredentialLoadError::InvalidCredential)?;
    let mut projects = HashSet::new();
    if stored.schema_version != 1
        || stored.identity.host_id.is_nil()
        || stored.identity.credential_id.is_nil()
        || stored.identity.generation.is_nil()
        || stored.binding.host_id() != stored.identity.host_id.to_string()
        || !(1..=16).contains(&stored.projects.len())
        || stored
            .projects
            .iter()
            .any(|p| ProjectKey::parse(p).is_err() || !projects.insert(p))
        || stored.issued_at >= stored.expires_at
    {
        return Err(CredentialLoadError::InvalidCredential);
    }
    if &stored.binding != expected_binding {
        return Err(CredentialLoadError::BindingMismatch);
    }
    if !stored
        .projects
        .iter()
        .any(|p| p == expected_project.as_str())
    {
        return Err(CredentialLoadError::ProjectDenied);
    }
    if now < stored.issued_at {
        return Err(CredentialLoadError::NotYetValid);
    }
    if now >= stored.expires_at {
        return Err(CredentialLoadError::Expired);
    }
    Ok(LoadedHostCredential {
        identity: stored.identity,
        binding: stored.binding,
        project: expected_project.clone(),
        expires_at: stored.expires_at,
        secret: stored.secret,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialImportError {
    #[error("expected absolute trusted home/source paths without parent components; correct the paths before retrying")]
    InvalidPath,
    #[error("Host credential source or destination is inside a repository; explicitly move it to private storage outside source control")]
    RepositoryStorage,
    #[error("trusted Unix time is unavailable; repair the clock before retrying")]
    ClockUnavailable,
    #[error("this credential copy already exists; explicitly load and inspect it instead of overwriting or automatically retrying")]
    AlreadyExists,
    #[error(transparent)]
    Credential(#[from] CredentialLoadError),
    #[error(transparent)]
    Storage(#[from] StoreError),
}

/// Public receipt of a private copy, not registration or activation authority.
#[derive(Debug)]
pub struct ImportedHostCredential {
    pub identity: CredentialIdentity,
    pub path: std::path::PathBuf,
}

/// Explicitly copy one credential to `HOME/.gwt/mobile/host/host-<UUID>.json`.
/// The caller supplies the canonical user home, trusted/non-replaceable source
/// and home ancestors, and independently selected Host/origin/Project scope.
/// Existing private directories are required; permissions are never repaired.
/// Time is sampled after the bounded source read. No existing copy is replaced,
/// and no current credential pointer, device approval or enable state is changed.
/// After CommitUncertain, inspect the copy with load_host_credential before any
/// explicit recovery; do not automatically retry or delete a published copy.
pub fn import_host_credential(
    home: &Path,
    source: &Path,
    expected_binding: &HostBinding,
    expected_project: &ProjectKey,
    clock: impl FnOnce() -> Result<u64, CredentialImportError>,
) -> Result<ImportedHostCredential, CredentialImportError> {
    import_with_sync(
        home,
        source,
        expected_binding,
        expected_project,
        clock,
        std::fs::File::sync_all,
    )
}

fn import_with_sync(
    home: &Path,
    source: &Path,
    expected_binding: &HostBinding,
    expected_project: &ProjectKey,
    clock: impl FnOnce() -> Result<u64, CredentialImportError>,
    sync_directory: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
) -> Result<ImportedHostCredential, CredentialImportError> {
    use crate::mobile_coordinator::storage::{has_repository_marker, open_directory, same_file};
    use std::{io::Write, path::Component};

    if [home, source].iter().any(|path| {
        !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir))
    }) {
        return Err(CredentialImportError::InvalidPath);
    }
    // Validate and persist this exact read, never re-read the source for copying.
    let bytes = read_private_credential(source)?;
    let now = clock()?;
    let loaded = decode_host_credential(&bytes, expected_binding, expected_project, now)?;
    let source_parent = source.parent().ok_or(CredentialImportError::InvalidPath)?;
    let source_parent = source_parent.canonicalize().map_err(StoreError::Io)?;
    if has_repository_marker(&source_parent)? {
        return Err(CredentialImportError::RepositoryStorage);
    }
    let paths = [
        home.join(".gwt"),
        home.join(".gwt/mobile"),
        home.join(".gwt/mobile/host"),
    ];
    let guards = paths
        .iter()
        .map(|p| open_directory(p))
        .collect::<Result<Vec<_>, _>>()?;
    let directory = paths[2].canonicalize().map_err(StoreError::Io)?;
    if has_repository_marker(&directory)? {
        return Err(CredentialImportError::RepositoryStorage);
    }
    let recheck = || -> Result<(), StoreError> {
        for (path, guard) in paths.iter().zip(&guards) {
            if !same_file(&open_directory(path)?, guard)? {
                return Err(StoreError::UnsafeStorage);
            }
        }
        Ok(())
    };
    recheck()?;
    let mut temp = tempfile::Builder::new()
        .prefix(".host-")
        .tempfile_in(&directory)
        .map_err(StoreError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(StoreError::Io)?;
    }
    temp.write_all(&bytes).map_err(StoreError::Io)?;
    temp.as_file().sync_all().map_err(StoreError::Io)?;
    if read_private_credential(temp.path())? != bytes {
        return Err(StoreError::UnsafeStorage.into());
    }
    recheck()?;
    let identity = *loaded.identity();
    let path = directory.join(format!("host-{}.json", identity.credential_id));
    let _published = temp.persist_noclobber(&path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            CredentialImportError::AlreadyExists
        } else {
            StoreError::CommitUncertain.into()
        }
    })?;
    (|| -> Result<(), CredentialImportError> {
        recheck()?;
        sync_directory(&guards[2]).map_err(StoreError::Io)?;
        let persisted = read_private_credential(&path)?;
        if persisted != bytes {
            return Err(StoreError::CommitUncertain.into());
        }
        decode_host_credential(&persisted, expected_binding, expected_project, now)?;
        Ok(())
    })()
    .map_err(|_| StoreError::CommitUncertain)?;
    Ok(ImportedHostCredential { identity, path })
}

#[cfg(all(test, not(unix)))]
#[test]
fn credential_loading_refuses_unsupported_private_storage() {
    let binding = HostBinding::new(
        &uuid::Uuid::new_v4().to_string(),
        "https://coordinator.example",
    )
    .unwrap();
    assert!(matches!(
        load_host_credential(
            Path::new("unused"),
            &binding,
            &ProjectKey::parse("0123456789abcdef").unwrap(),
            100
        ),
        Err(CredentialLoadError::Storage(
            StoreError::UnsupportedPrivateStorage
        ))
    ));
    assert!(matches!(
        import_host_credential(
            &std::env::temp_dir(),
            &std::env::temp_dir().join("unused-credential"),
            &binding,
            &ProjectKey::parse("0123456789abcdef").unwrap(),
            || Ok(100),
        ),
        Err(CredentialImportError::Storage(
            StoreError::UnsupportedPrivateStorage
        ))
    ));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::mobile_coordinator::{storage::CoordinatorStore, CredentialError};
    use serde_json::{json, Value};
    use std::os::unix::fs::{symlink, PermissionsExt};
    use uuid::Uuid;

    const ORIGIN: &str = "https://coordinator.example";

    fn project() -> ProjectKey {
        ProjectKey::parse("0123456789abcdef").unwrap()
    }

    fn fixture() -> (
        tempfile::TempDir,
        CoordinatorStore,
        std::path::PathBuf,
        HostBinding,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = CoordinatorStore::initialize(dir.path(), 100).unwrap();
        let host = Uuid::new_v4();
        let issued = store.issue(ORIGIN, host, &[project()], 200, 100).unwrap();
        let binding = HostBinding::new(&host.to_string(), ORIGIN).unwrap();
        (dir, store, issued.path, binding)
    }

    fn host_home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        for name in [".gwt", ".gwt/mobile", ".gwt/mobile/host"] {
            let path = home.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        home
    }

    #[test]
    fn import_copies_exact_issued_bytes_once_without_replacing_credentials() {
        let (_source, mut store, path, binding) = fixture();
        let home = host_home();
        let original = std::fs::read(&path).unwrap();
        let imported = import_host_credential(home.path(), &path, &binding, &project(), || {
            // The trusted clock runs after read. A later source replacement must
            // never change which bytes are validated or copied.
            std::fs::write(&path, b"replaced after source read").unwrap();
            Ok(100)
        })
        .unwrap();
        assert_eq!(std::fs::read(&imported.path).unwrap(), original);
        assert_eq!(
            imported.path.parent().unwrap(),
            home.path().join(".gwt/mobile/host").canonicalize().unwrap()
        );
        assert_eq!(
            imported.path.file_name().unwrap().to_str().unwrap(),
            format!("host-{}.json", imported.identity.credential_id)
        );
        let loaded = load_host_credential(&imported.path, &binding, &project(), 100).unwrap();
        assert_eq!(loaded.identity(), &imported.identity);
        store
            .registry()
            .unwrap()
            .verify(loaded.identity(), &project(), loaded.secret_bytes(), 100)
            .unwrap();
        let mut changed: Value = serde_json::from_slice(&original).unwrap();
        changed["secret"] = json!("ab".repeat(32));
        std::fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)),
            Err(CredentialImportError::AlreadyExists)
        ));
        assert_eq!(std::fs::read(&imported.path).unwrap(), original);
        let next = store
            .rotate(ORIGIN, imported.identity.credential_id, 300, 110)
            .unwrap();
        let next_copy =
            import_host_credential(home.path(), &next.path, &binding, &project(), || Ok(110))
                .unwrap();
        assert_ne!(imported.path, next_copy.path);
        assert_eq!(std::fs::read(&imported.path).unwrap(), original);
        assert_eq!(
            std::fs::read_dir(home.path().join(".gwt/mobile/host"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn import_rejects_foreign_or_expired_credentials_before_creating_output() {
        let (_source, _store, path, binding) = fixture();
        let home = host_home();
        let foreign = HostBinding::new(&Uuid::new_v4().to_string(), ORIGIN).unwrap();
        assert!(matches!(
            import_host_credential(home.path(), &path, &foreign, &project(), || Ok(100)),
            Err(CredentialImportError::Credential(
                CredentialLoadError::BindingMismatch
            ))
        ));
        let other = ProjectKey::parse("fedcba9876543210").unwrap();
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &other, || Ok(100)),
            Err(CredentialImportError::Credential(
                CredentialLoadError::ProjectDenied
            ))
        ));
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(200)),
            Err(CredentialImportError::Credential(
                CredentialLoadError::Expired
            ))
        ));
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &project(), || Err(
                CredentialImportError::ClockUnavailable
            )),
            Err(CredentialImportError::ClockUnavailable)
        ));
        assert_eq!(
            std::fs::read_dir(home.path().join(".gwt/mobile/host"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn import_requires_private_non_repository_storage_and_explicit_repair() {
        let (source, _store, path, binding) = fixture();
        let home = host_home();
        let directory = home.path().join(".gwt/mobile/host");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)),
            Err(CredentialImportError::Storage(StoreError::UnsafeStorage))
        ));
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let saved = home.path().join("saved-host");
        std::fs::rename(&directory, &saved).unwrap();
        symlink(&saved, &directory).unwrap();
        assert!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)).is_err()
        );
        assert_eq!(std::fs::read_dir(&saved).unwrap().count(), 0);
        std::fs::remove_file(&directory).unwrap();
        std::fs::rename(saved, &directory).unwrap();
        for marker in [
            home.path().join(".gwt/mobile/.git"),
            source.path().join(".git"),
        ] {
            std::fs::write(&marker, "gitdir: unused-test-marker").unwrap();
            assert!(matches!(
                import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)),
                Err(CredentialImportError::RepositoryStorage)
            ));
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
            std::fs::remove_file(&marker).unwrap();
            symlink("missing-git-directory", &marker).unwrap();
            assert!(matches!(
                import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)),
                Err(CredentialImportError::RepositoryStorage)
            ));
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
            std::fs::remove_file(marker).unwrap();
        }
        assert!(matches!(
            import_host_credential(Path::new("relative"), &path, &binding, &project(), || Ok(
                100
            )),
            Err(CredentialImportError::InvalidPath)
        ));
        assert!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)).is_ok()
        );
    }

    #[test]
    fn import_reports_uncertain_publication_without_deleting_the_copy() {
        let (_source, _store, path, binding) = fixture();
        let home = host_home();
        let original = std::fs::read(&path).unwrap();
        assert!(matches!(
            import_with_sync(
                home.path(),
                &path,
                &binding,
                &project(),
                || Ok(100),
                |_| { Err(std::io::Error::other("injected directory sync failure")) }
            ),
            Err(CredentialImportError::Storage(StoreError::CommitUncertain))
        ));
        let paths: Vec<_> = std::fs::read_dir(home.path().join(".gwt/mobile/host"))
            .unwrap()
            .collect();
        assert_eq!(paths.len(), 1);
        let saved = paths[0].as_ref().unwrap().path();
        assert_eq!(std::fs::read(&saved).unwrap(), original);
        assert!(load_host_credential(&saved, &binding, &project(), 100).is_ok());
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)),
            Err(CredentialImportError::AlreadyExists)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn emitted_credentials_load_without_claiming_registry_liveness() {
        let (_dir, mut store, path, binding) = fixture();
        let original = std::fs::read(&path).unwrap();
        let loaded = load_host_credential(&path, &binding, &project(), 100).unwrap();
        assert_eq!(loaded.binding(), &binding);
        assert_eq!(loaded.project(), &project());
        assert_eq!(loaded.expires_at(), 200);
        store
            .registry()
            .unwrap()
            .verify(loaded.identity(), &project(), loaded.secret_bytes(), 100)
            .unwrap();
        let rotated = store
            .rotate(ORIGIN, loaded.identity().credential_id, 300, 110)
            .unwrap();
        let next = load_host_credential(&rotated.path, &binding, &project(), 110).unwrap();
        store
            .registry()
            .unwrap()
            .verify(next.identity(), &project(), next.secret_bytes(), 110)
            .unwrap();
        // Old files remain readable; only authenticated registration checks revocation.
        let old = load_host_credential(&path, &binding, &project(), 110).unwrap();
        assert_eq!(
            store
                .registry()
                .unwrap()
                .verify(old.identity(), &project(), old.secret_bytes(), 110),
            Err(CredentialError::Unauthorized)
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    fn zeroizes_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}

    #[test]
    fn loaded_secret_is_zeroize_on_drop_and_escaped_secrets_are_refused() {
        let (_dir, _store, path, binding) = fixture();
        let loaded = load_host_credential(&path, &binding, &project(), 100).unwrap();
        zeroizes_on_drop(&loaded.secret);
        // An escaped secret cannot be borrowed and would need an owned copy.
        let original = std::fs::read_to_string(&path).unwrap();
        let start = original.find("\"secret\":\"").unwrap() + "\"secret\":\"".len();
        let escaped = format!(
            "{}\\u{:04x}{}",
            &original[..start],
            original.as_bytes()[start],
            &original[start + 1..]
        );
        std::fs::write(&path, &escaped).unwrap();
        assert!(matches!(
            load_host_credential(&path, &binding, &project(), 100),
            Err(CredentialLoadError::InvalidCredential)
        ));
        std::fs::write(&path, original).unwrap();
        assert!(load_host_credential(&path, &binding, &project(), 100).is_ok());
    }

    #[test]
    fn load_rejects_foreign_scope_and_time_without_changing_the_file() {
        let (_dir, _store, path, binding) = fixture();
        let original = std::fs::read(&path).unwrap();
        let foreign_host = HostBinding::new(&Uuid::new_v4().to_string(), ORIGIN).unwrap();
        let host: Value = serde_json::from_slice(&original).unwrap();
        let foreign_origin = HostBinding::new(
            host["identity"]["host_id"].as_str().unwrap(),
            "https://other.example",
        )
        .unwrap();
        for expected in [&foreign_host, &foreign_origin] {
            assert!(matches!(
                load_host_credential(&path, expected, &project(), 100),
                Err(CredentialLoadError::BindingMismatch)
            ));
        }
        let other = ProjectKey::parse("fedcba9876543210").unwrap();
        assert!(matches!(
            load_host_credential(&path, &binding, &other, 100),
            Err(CredentialLoadError::ProjectDenied)
        ));
        assert!(matches!(
            load_host_credential(&path, &binding, &project(), 99),
            Err(CredentialLoadError::NotYetValid)
        ));
        assert!(matches!(
            load_host_credential(&path, &binding, &project(), 200),
            Err(CredentialLoadError::Expired)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn load_rejects_malformed_envelopes_and_recovers_with_original_bytes() {
        let (_dir, _store, path, binding) = fixture();
        let original = std::fs::read(&path).unwrap();
        let valid: Value = serde_json::from_slice(&original).unwrap();
        let mut malformed = Vec::new();
        for (field, replacement) in [
            ("schema_version", json!(2)),
            ("extra", json!(true)),
            ("projects", json!([])),
            ("projects", json!(["0123456789abcdef", "0123456789abcdef"])),
            ("projects", json!(["0123456789abcdeF"])),
            (
                "projects",
                json!((0..17).map(|id| format!("{id:016x}")).collect::<Vec<_>>()),
            ),
            ("secret", json!("00")),
            ("secret", json!("x".repeat(64))),
            ("issued_at", json!(200)),
            ("expires_at", json!(99)),
        ] {
            let mut value = valid.clone();
            value[field] = replacement;
            malformed.push(serde_json::to_vec(&value).unwrap());
        }
        for field in ["host_id", "credential_id", "generation"] {
            let mut value = valid.clone();
            value["identity"][field] = json!(Uuid::nil());
            malformed.push(serde_json::to_vec(&value).unwrap());
        }
        let mut value = valid.clone();
        value["binding"]["host_id"] = json!(Uuid::new_v4());
        malformed.push(serde_json::to_vec(&value).unwrap());
        for field in ["binding", "identity"] {
            let mut value = valid.clone();
            value[field]["extra"] = json!(true);
            malformed.push(serde_json::to_vec(&value).unwrap());
        }
        let duplicate = format!(
            "{{\"schema_version\":1,{}",
            std::str::from_utf8(&original)
                .unwrap()
                .trim_start_matches('{')
        );
        malformed.push(duplicate.into_bytes());
        for bytes in malformed {
            std::fs::write(&path, &bytes).unwrap();
            assert!(matches!(
                load_host_credential(&path, &binding, &project(), 100),
                Err(CredentialLoadError::InvalidCredential)
            ));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        let mut oversized = original.clone();
        oversized.resize(128 * 1024 + 1, b' ');
        std::fs::write(&path, &oversized).unwrap();
        assert!(matches!(
            load_host_credential(&path, &binding, &project(), 100),
            Err(CredentialLoadError::Storage(StoreError::CredentialTooLarge))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), oversized);
        std::fs::write(&path, original).unwrap();
        assert!(load_host_credential(&path, &binding, &project(), 100).is_ok());
    }

    #[test]
    fn load_requires_private_regular_files_and_private_parent() {
        let (dir, _store, path, binding) = fixture();
        let original = std::fs::read(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_host_credential(&path, &binding, &project(), 100).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(load_host_credential(&path, &binding, &project(), 100).is_err());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = dir.path().join("alias.json");
        symlink(&path, &alias).unwrap();
        assert!(load_host_credential(&alias, &binding, &project(), 100).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(load_host_credential(&path, &binding, &project(), 100).is_err());
        std::fs::remove_file(&alias).unwrap();
        assert!(load_host_credential(dir.path(), &binding, &project(), 100).is_err());
        assert!(load_host_credential(&alias, &binding, &project(), 100).is_err());
        assert!(load_host_credential(&path, &binding, &project(), 100).is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_acl_grants_refuse_private_reads_and_writes_until_explicit_repair() {
        fn acl(path: &Path, args: &[&str]) {
            let output = gwt_core::process::resolved_command(
                gwt_core::process::ProcessPlanRequest::new("/bin/chmod"),
            )
            .unwrap()
            .args(args)
            .arg(path)
            .output()
            .unwrap();
            assert!(
                output.status.success(),
                "ACL fixture: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let (dir, mut store, path, binding) = fixture();
        let original = std::fs::read(&path).unwrap();
        acl(&path, &["+a", "everyone allow read"]);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        assert!(matches!(
            load_host_credential(&path, &binding, &project(), 100),
            Err(CredentialLoadError::Storage(StoreError::UnsafeStorage))
        ));
        acl(&path, &["-N"]);
        assert!(load_host_credential(&path, &binding, &project(), 100).is_ok());

        let registry = dir.path().join("registry.json");
        let before = std::fs::read(&registry).unwrap();
        let count = std::fs::read_dir(dir.path()).unwrap().count();
        acl(dir.path(), &["+a", "everyone allow list,search"]);
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert!(matches!(
            load_host_credential(&path, &binding, &project(), 100),
            Err(CredentialLoadError::Storage(StoreError::UnsafeStorage))
        ));
        assert!(matches!(
            store.issue(ORIGIN, Uuid::new_v4(), &[project()], 200, 100),
            Err(StoreError::UnsafeStorage)
        ));
        assert_eq!(std::fs::read(&registry).unwrap(), before);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), count);
        acl(dir.path(), &["-N"]);
        let loaded = load_host_credential(&path, &binding, &project(), 100).unwrap();
        store
            .registry()
            .unwrap()
            .verify(loaded.identity(), &project(), loaded.secret_bytes(), 100)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);

        let home = host_home();
        let destination = home.path().join(".gwt/mobile/host");
        acl(&destination, &["+a", "everyone allow list,search"]);
        assert!(matches!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)),
            Err(CredentialImportError::Storage(StoreError::UnsafeStorage))
        ));
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 0);
        acl(&destination, &["-N"]);
        assert!(
            import_host_credential(home.path(), &path, &binding, &project(), || Ok(100)).is_ok()
        );
    }
}
