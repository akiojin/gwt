//! Bounded offline management; this adapter never starts a listener.
//!
//! The caller selects the canonical OS user home, with trusted, non-replaceable
//! ancestors. Storage is fixed below it, never selected by the JSON request.

use super::{
    storage::{open_directory, same_file, CoordinatorStore, StoreError},
    CredentialError,
};
use crate::mobile_connection::HostBinding;
use gwt_core::repo_hash::ProjectKey;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Read,
    path::{Component, Path},
};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u32,
    command: Command,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Initialize {},
    Status {},
    Issue {
        origin: String,
        host_id: Uuid,
        #[serde(deserialize_with = "projects")]
        projects: Vec<ProjectKey>,
        expires_at: u64,
    },
    Rotate {
        origin: String,
        credential_id: Uuid,
        expires_at: u64,
    },
    Revoke {
        credential_id: Uuid,
    },
}

fn projects<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<ProjectKey>, D::Error> {
    Vec::<String>::deserialize(deserializer)?
        .iter()
        .map(|p| ProjectKey::parse(p).map_err(|_| serde::de::Error::custom("invalid project key")))
        .collect()
}

#[derive(Debug)]
pub enum AdminError {
    InvalidRequest,
    UnsafeHome,
    RepositoryStorage,
    ClockUnavailable,
    Storage(StoreError),
}

impl From<StoreError> for AdminError {
    fn from(error: StoreError) -> Self {
        Self::Storage(error)
    }
}

impl AdminError {
    /// Fixed diagnostics only: never echo input, paths, serde or OS errors.
    pub fn response(&self) -> Value {
        let (code, diagnostic, recovery) = match self {
            Self::InvalidRequest => ("invalid_request", "Expected one schema1 JSON request, at most 16 KiB, with a supported operation and valid public fields; request rejected before storage access.", "Use --help; correct the request and retry."),
            Self::UnsafeHome | Self::Storage(StoreError::UnsafeStorage) => ("unsafe_storage", "Expected an absolute trusted user home and owned 0700 .gwt/mobile/coordinator directories, without symlinks or macOS extended ACLs; private storage validation failed.", "Inspect and explicitly repair the private directories and their ancestors, then retry; permissions are never changed automatically."),
            Self::RepositoryStorage => ("repository_storage", "Credential storage is inside a repository or worktree; no registry was opened.", "Select a trusted user home outside repositories; do not place credentials in source control."),
            Self::ClockUnavailable => ("clock_unavailable", "Trusted Unix time is unavailable; no registry was opened.", "Repair the Host clock and retry."),
            Self::Storage(StoreError::Busy) => ("busy", "The coordinator registry lock is held; this request did not change registry authority.", "Stop the coordinator or other administrator, then retry."),
            Self::Storage(StoreError::UnsupportedPrivateStorage) => ("unsupported_private_storage", "Private storage validation is unavailable on this platform.", "Use a supported Unix host; Windows requires a private ACL adapter."),
            Self::Storage(StoreError::CommitUncertain | StoreError::Unusable) => ("commit_uncertain", "The registry commit outcome is uncertain; authority may have changed.", "Run status to reopen and inspect current public records; do not automatically repeat issuance."),
            Self::Storage(StoreError::Credential(CredentialError::ClockRollback)) => ("clock_rollback", "The current clock predates the registry watermark; operation refused.", "Restore trusted time and run status; do not reset or replace the registry."),
            Self::Storage(StoreError::Credential(CredentialError::Unauthorized | CredentialError::Expired)) => ("unauthorized", "The credential is unknown, revoked or expired for this operation.", "Inspect status and explicitly choose a current credential."),
            Self::Storage(StoreError::Credential(CredentialError::Capacity) | StoreError::CredentialTooLarge) => ("capacity", "A finite credential or registry capacity was reached; tombstones are retained.", "Inspect status; do not delete tombstones or automatically retry issuance."),
            Self::Storage(StoreError::InvalidOrigin | StoreError::Credential(CredentialError::InvalidRegistry)) => ("invalid_registry", "Registry or issuance parameters are invalid; initialization never replaces existing data.", "Inspect status and the existing private files before correcting the request or storage."),
            Self::Storage(StoreError::Credential(CredentialError::Unavailable) | StoreError::Io(_)) => ("storage_unavailable", "Private storage or secure randomness is unavailable; success was not established.", "Inspect the fixed private directories and current records, repair the cause, then explicitly retry."),
        };
        json!({"ok":false,"code":code,"diagnostic":diagnostic,"recovery":recovery})
    }
}

/// Execute one offline request using a trusted clock and user-home selection.
/// The complete request is validated before any storage operation, including lock
/// creation. Time is sampled after reading the request, so delayed stdin cannot
/// backdate issuance. Success receipts never contain a bearer or digest.
pub fn run(
    input: impl Read,
    home: &Path,
    clock: impl FnOnce() -> Result<u64, AdminError>,
) -> Result<Value, AdminError> {
    let mut bytes = Vec::new();
    input
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AdminError::InvalidRequest)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(AdminError::InvalidRequest);
    }
    let request: Request =
        serde_json::from_slice(&bytes).map_err(|_| AdminError::InvalidRequest)?;
    if request.schema_version != 1 {
        return Err(AdminError::InvalidRequest);
    }
    let now = clock()?;
    match &request.command {
        Command::Issue {
            origin,
            host_id,
            projects,
            expires_at,
        } => {
            let unique: HashSet<_> = projects.iter().collect();
            if host_id.is_nil()
                || !(1..=16).contains(&projects.len())
                || unique.len() != projects.len()
                || *expires_at <= now
                || HostBinding::new(&host_id.to_string(), origin).is_err()
            {
                return Err(AdminError::InvalidRequest);
            }
        }
        Command::Rotate {
            origin,
            credential_id,
            expires_at,
        } => {
            if credential_id.is_nil()
                || *expires_at <= now
                || HostBinding::new(&credential_id.to_string(), origin).is_err()
            {
                return Err(AdminError::InvalidRequest);
            }
        }
        Command::Revoke { credential_id } if credential_id.is_nil() => {
            return Err(AdminError::InvalidRequest)
        }
        _ => {}
    }
    if !home.is_absolute() || home.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(AdminError::UnsafeHome);
    }
    let paths = [
        home.join(".gwt"),
        home.join(".gwt/mobile"),
        home.join(".gwt/mobile/coordinator"),
    ];
    // Keep the checked descriptors alive and recheck identity before opening the
    // store. Ancestors remain trusted operator state, not an attacker-writable tree.
    let guards = paths
        .iter()
        .map(|p| open_directory(p))
        .collect::<Result<Vec<_>, _>>()?;
    let directory = paths[2]
        .canonicalize()
        .map_err(|_| AdminError::UnsafeHome)?;
    if gwt_git::Repository::discover(&directory).is_ok() {
        return Err(AdminError::RepositoryStorage);
    }
    for (path, guard) in paths.iter().zip(&guards) {
        if !same_file(&open_directory(path)?, guard)? {
            return Err(AdminError::UnsafeHome);
        }
    }
    if matches!(request.command, Command::Initialize {}) {
        CoordinatorStore::initialize(&directory, now)?;
        return Ok(json!({"ok":true,"operation":"initialize"}));
    }
    let mut store = CoordinatorStore::open(&directory, now)?;
    let issued = match request.command {
        Command::Issue {
            origin,
            host_id,
            projects,
            expires_at,
        } => store.issue(&origin, host_id, &projects, expires_at, now)?,
        Command::Rotate {
            origin,
            credential_id,
            expires_at,
        } => store.rotate(&origin, credential_id, expires_at, now)?,
        Command::Revoke { credential_id } => {
            store.revoke(credential_id, now)?;
            return Ok(json!({"ok":true,"operation":"revoke","credential_id":credential_id}));
        }
        Command::Status {} => {
            let registry = store.registry()?;
            let records: Vec<_> = registry
                .stored
                .records
                .iter()
                .map(|r| {
                    json!({
                        "identity":r.identity,"projects":r.projects,"issued_at":r.issued_at,
                        "expires_at":r.expires_at,"revoked":r.revoked
                    })
                })
                .collect();
            return Ok(
                json!({"ok":true,"operation":"status","last_observed":registry.stored.last_observed,"records":records}),
            );
        }
        Command::Initialize {} => unreachable!("handled before opening an existing registry"),
    };
    Ok(json!({"ok":true,"identity":issued.identity,"credential_file":issued.path}))
}

#[cfg(all(test, not(unix)))]
#[test]
fn offline_administration_refuses_unsupported_private_storage() {
    let error = run(
        br#"{"schema_version":1,"command":{"operation":"status"}}"#.as_slice(),
        &std::env::temp_dir(),
        || Ok(100),
    )
    .unwrap_err();
    assert_eq!(error.response()["code"], "unsupported_private_storage");
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{
        mobile_connection::HostBinding,
        mobile_coordinator::{storage::CoordinatorStore, CredentialIdentity},
        mobile_host::credential::load_host_credential,
    };
    use gwt_core::repo_hash::ProjectKey;
    use serde_json::{json, Value};
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};
    use uuid::Uuid;

    const NOW: u64 = 100;
    const ORIGIN: &str = "https://coordinator.example";
    const PROJECT: &str = "0123456789abcdef";

    fn home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        for path in [
            home.path().join(".gwt"),
            home.path().join(".gwt/mobile"),
            directory(home.path()),
        ] {
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        home
    }

    fn directory(home: &Path) -> std::path::PathBuf {
        home.join(".gwt/mobile/coordinator")
    }

    fn call(home: &Path, command: Value, now: u64) -> Result<Value, AdminError> {
        let bytes = serde_json::to_vec(&json!({"schema_version":1,"command":command})).unwrap();
        run(bytes.as_slice(), home, || Ok(now))
    }

    fn issue(host: Uuid) -> Value {
        json!({"operation":"issue","origin":ORIGIN,"host_id":host,"projects":[PROJECT],"expires_at":200})
    }

    #[test]
    fn issuance_observes_trusted_time_after_reading_the_complete_request() {
        let home = home();
        call(home.path(), json!({"operation":"initialize"}), NOW).unwrap();
        let clock = std::cell::Cell::new(NOW);
        struct Reader<'a>(std::io::Cursor<Vec<u8>>, &'a std::cell::Cell<u64>);
        impl std::io::Read for Reader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.1.set(200);
                std::io::Read::read(&mut self.0, buffer)
            }
        }
        let bytes =
            serde_json::to_vec(&json!({"schema_version":1,"command":issue(Uuid::new_v4())}))
                .unwrap();
        let reader = Reader(std::io::Cursor::new(bytes), &clock);
        let original = fs::read(directory(home.path()).join("registry.json")).unwrap();
        let result = run(reader, home.path(), || Ok(clock.get()));
        assert_eq!(result.unwrap_err().response()["code"], "invalid_request");
        assert_eq!(
            fs::read(directory(home.path()).join("registry.json")).unwrap(),
            original
        );
    }

    #[test]
    fn offline_lifecycle_uses_the_real_store_and_returns_only_public_metadata() {
        let home = home();
        assert!(
            call(home.path(), json!({"operation":"initialize"}), NOW).unwrap()["ok"]
                .as_bool()
                .unwrap()
        );
        assert!(call(home.path(), json!({"operation":"initialize"}), NOW).is_err());
        let host = Uuid::new_v4();
        let first = call(home.path(), issue(host), NOW).unwrap();
        let identity: CredentialIdentity =
            serde_json::from_value(first["identity"].clone()).unwrap();
        let binding = HostBinding::new(&host.to_string(), ORIGIN).unwrap();
        let project = ProjectKey::parse(PROJECT).unwrap();
        let credential = load_host_credential(
            Path::new(first["credential_file"].as_str().unwrap()),
            &binding,
            &project,
            NOW,
        )
        .unwrap();
        assert!(!first
            .to_string()
            .contains(&hex::encode(credential.secret_bytes())));
        assert!(first.get("secret").is_none());
        let mut store = CoordinatorStore::open(&directory(home.path()), NOW).unwrap();
        store
            .registry()
            .unwrap()
            .verify(&identity, &project, credential.secret_bytes(), NOW)
            .unwrap();
        drop(store);
        let rotated = call(home.path(), json!({"operation":"rotate","origin":ORIGIN,"credential_id":identity.credential_id,"expires_at":300}), 110).unwrap();
        let next: CredentialIdentity = serde_json::from_value(rotated["identity"].clone()).unwrap();
        let new_credential = load_host_credential(
            Path::new(rotated["credential_file"].as_str().unwrap()),
            &binding,
            &project,
            110,
        )
        .unwrap();
        let mut store = CoordinatorStore::open(&directory(home.path()), 110).unwrap();
        assert!(store
            .registry()
            .unwrap()
            .verify(&identity, &project, credential.secret_bytes(), 110)
            .is_err());
        store
            .registry()
            .unwrap()
            .verify(&next, &project, new_credential.secret_bytes(), 110)
            .unwrap();
        drop(store);
        call(
            home.path(),
            json!({"operation":"revoke","credential_id":next.credential_id}),
            120,
        )
        .unwrap();
        let status = call(home.path(), json!({"operation":"status"}), 120).unwrap();
        assert_eq!(status["records"].as_array().unwrap().len(), 2);
        assert!(status["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["revoked"] == true));
        assert!(!status.to_string().contains("digest"));
        assert!(!status
            .to_string()
            .contains(&hex::encode(new_credential.secret_bytes())));
        assert_eq!(status["last_observed"], 120);
        let mut store = CoordinatorStore::open(&directory(home.path()), 120).unwrap();
        assert!(store
            .registry()
            .unwrap()
            .verify(&next, &project, new_credential.secret_bytes(), 120)
            .is_err());
        assert!(Path::new(first["credential_file"].as_str().unwrap()).exists());
    }

    #[test]
    fn invalid_input_is_bounded_sanitized_and_rejected_before_any_storage_write() {
        let home = home();
        let mut oversized =
            br#"{"schema_version":1,"command":{"operation":"initialize"}}"#.to_vec();
        oversized.resize(16 * 1024 + 1, b' ');
        let mut invalid = vec![b"{".to_vec(), oversized.clone()];
        for command in [
            json!({"operation":"serve"}),
            json!({"operation":"initialize","secret":"sensitive-sentinel"}),
            json!({"operation":"status","secret":"sensitive-sentinel"}),
            json!({"operation":"initialize","now":0}),
            json!({"operation":"initialize","directory":"/tmp"}),
            issue(Uuid::nil()),
            json!({"operation":"issue","origin":"http://wrong.invalid","host_id":Uuid::new_v4(),"projects":[PROJECT],"expires_at":200}),
            json!({"operation":"issue","origin":ORIGIN,"host_id":Uuid::new_v4(),"projects":["../bad"],"expires_at":200}),
            json!({"operation":"issue","origin":ORIGIN,"host_id":Uuid::new_v4(),"projects":[PROJECT,PROJECT],"expires_at":200}),
            json!({"operation":"issue","origin":ORIGIN,"host_id":Uuid::new_v4(),"projects":[],"expires_at":200}),
            json!({"operation":"issue","origin":ORIGIN,"host_id":Uuid::new_v4(),"projects":(0..17).map(|i| format!("{i:016x}")).collect::<Vec<_>>(),"expires_at":200}),
            json!({"operation":"rotate","origin":ORIGIN,"credential_id":Uuid::new_v4(),"expires_at":NOW}),
            json!({"operation":"revoke","credential_id":Uuid::nil()}),
        ] {
            invalid
                .push(serde_json::to_vec(&json!({"schema_version":1,"command":command})).unwrap());
        }
        invalid.push(
            serde_json::to_vec(&json!({"schema_version":2,"command":{"operation":"initialize"}}))
                .unwrap(),
        );
        invalid.push(
            br#"{"schema_version":1,"schema_version":1,"command":{"operation":"initialize"}}"#
                .to_vec(),
        );
        for bytes in invalid {
            let error = run(bytes.as_slice(), home.path(), || Ok(NOW)).unwrap_err();
            assert_eq!(error.response()["code"], "invalid_request");
            assert!(!error.response().to_string().contains("sensitive-sentinel"));
            assert_eq!(fs::read_dir(directory(home.path())).unwrap().count(), 0);
        }
        oversized.pop();
        assert_eq!(oversized.len(), 16 * 1024);
        run(oversized.as_slice(), home.path(), || Ok(NOW)).unwrap();
    }

    #[test]
    fn busy_store_preserves_authority_and_retries_after_the_holder_stops() {
        let home = home();
        let guard = CoordinatorStore::initialize(&directory(home.path()), NOW).unwrap();
        let original = fs::read(directory(home.path()).join("registry.json")).unwrap();
        assert_eq!(
            call(home.path(), issue(Uuid::new_v4()), NOW)
                .unwrap_err()
                .response()["code"],
            "busy"
        );
        assert_eq!(
            fs::read(directory(home.path()).join("registry.json")).unwrap(),
            original
        );
        drop(guard);
        call(home.path(), issue(Uuid::new_v4()), NOW).unwrap();
    }

    #[test]
    fn unsafe_or_symlinked_private_roots_require_explicit_repair() {
        let home = home();
        let anchor = home.path().join(".gwt");
        fs::set_permissions(&anchor, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            call(home.path(), json!({"operation":"initialize"}), NOW)
                .unwrap_err()
                .response()["code"],
            "unsafe_storage"
        );
        assert_eq!(
            fs::metadata(&anchor).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::set_permissions(&anchor, fs::Permissions::from_mode(0o700)).unwrap();
        let mobile = anchor.join("mobile");
        let saved = anchor.join("saved-mobile");
        fs::rename(&mobile, &saved).unwrap();
        std::os::unix::fs::symlink(&saved, &mobile).unwrap();
        assert!(call(home.path(), json!({"operation":"initialize"}), NOW).is_err());
        assert_eq!(fs::read_dir(saved.join("coordinator")).unwrap().count(), 0);
        fs::remove_file(&mobile).unwrap();
        fs::rename(saved, mobile).unwrap();
        call(home.path(), json!({"operation":"initialize"}), NOW).unwrap();
    }

    #[test]
    fn repository_roots_include_git_ancestors_below_the_selected_home() {
        let home = home();
        let marker = home.path().join(".gwt/mobile/.git");
        fs::write(&marker, "gitdir: unused-test-marker").unwrap();
        assert_eq!(
            call(home.path(), json!({"operation":"initialize"}), NOW)
                .unwrap_err()
                .response()["code"],
            "repository_storage"
        );
        assert_eq!(fs::read_dir(directory(home.path())).unwrap().count(), 0);
        fs::remove_file(marker).unwrap();
        call(home.path(), json!({"operation":"initialize"}), NOW).unwrap();
    }
}
