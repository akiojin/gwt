//! Unpublished reader for coordinator-issued Host credentials (Issue #6476).
//!
//! The caller selects a directory under private gwt home with trusted,
//! non-replaceable ancestors. This checks its immediate parent and file, not
//! home placement or ancestor ownership. Loading proves neither issuance nor
//! registry liveness: orphaned, rotated and revoked files may still parse.
//! Authenticated coordinator registration must check current authority before
//! use. No import, network connection, device authority or PC enable happens here.

use crate::mobile_connection::HostBinding;
use crate::mobile_coordinator::{
    storage::{read_private_credential, StoreError},
    CredentialIdentity,
};
use gwt_core::repo_hash::ProjectKey;
use serde::Deserialize;
use std::{collections::HashSet, path::Path};

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
    secret: String,
}

/// Deliberately not Debug, Clone or Serialize. Local parsing is not proof that
/// the coordinator accepts this generation; the bearer must only be sent to
/// the configured, TLS-authenticated coordinator's Host registration endpoint.
pub struct LoadedHostCredential {
    identity: CredentialIdentity,
    binding: HostBinding,
    project: ProjectKey,
    expires_at: u64,
    secret: [u8; 32],
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
    let stored: StoredCredential =
        serde_json::from_slice(&bytes).map_err(|_| CredentialLoadError::InvalidCredential)?;
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
        || stored.secret.len() != 64
    {
        return Err(CredentialLoadError::InvalidCredential);
    }
    let mut secret = [0; 32];
    hex::decode_to_slice(&stored.secret, &mut secret)
        .map_err(|_| CredentialLoadError::InvalidCredential)?;
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
        secret,
    })
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
}
