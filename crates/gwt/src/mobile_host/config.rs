//! Read-only PC configuration. Nothing here connects, registers or enables.

use super::credential::{decode_host_credential, CredentialLoadError, LoadedHostCredential};
use crate::{
    mobile_connection::HostBinding,
    mobile_coordinator::{
        storage::{open_directory, read_private_credential, same_file, StoreError},
        CredentialIdentity,
    },
};
use gwt_core::repo_hash::ProjectKey;
use serde::Deserialize;
use std::path::{Component, Path};

#[derive(Debug, thiserror::Error)]
pub enum HostConfigError {
    #[error("expected a private schema1 Host configuration with valid binding, Project and credential identity; correct config.json before retrying")]
    InvalidConfig,
    #[error("Host configuration differs from the caller's expected Host/origin/Project; explicitly correct the configuration or expected scope")]
    ScopeMismatch,
    #[error("the selected credential identity differs from config.json; explicitly select the correct imported credential without replacing existing copies")]
    IdentityMismatch,
    #[error("expected an absolute trusted user home without parent components")]
    InvalidHome,
    #[error("Host configuration is inside a repository; move private storage outside source control before retrying")]
    RepositoryStorage,
    #[error("trusted Unix time is unavailable; repair the clock before retrying")]
    ClockUnavailable,
    #[error(transparent)]
    Credential(#[from] CredentialLoadError),
    #[error(transparent)]
    Storage(#[from] StoreError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredConfig {
    schema_version: u32,
    binding: HostBinding,
    project: String,
    identity: CredentialIdentity,
}

/// A private credential selected by explicit PC configuration, not a live lease.
/// Deliberately not Debug, Clone or Serialize because it holds the bearer.
pub struct MobileHostConfig {
    credential: LoadedHostCredential,
}

impl MobileHostConfig {
    /// Read `HOME/.gwt/mobile/host/config.json` and its selected immutable copy.
    /// Home ancestors must be trusted/non-replaceable. Directories must already
    /// be private; no paths, permissions, files or enable state are changed.
    /// The caller independently selects the expected Host/origin/Project and
    /// supplies trusted time after both bounded reads. Local parsing does not
    /// prove issuance, registry liveness or coordinator acceptance.
    pub fn load(
        home: &Path,
        expected_binding: &HostBinding,
        expected_project: &ProjectKey,
        clock: impl FnOnce() -> Result<u64, HostConfigError>,
    ) -> Result<Self, HostConfigError> {
        if !home.is_absolute() || home.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(HostConfigError::InvalidHome);
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
        if gwt_git::Repository::discover(&directory).is_ok() {
            return Err(HostConfigError::RepositoryStorage);
        }
        let bytes = read_private_credential(&directory.join("config.json"))?;
        let config: StoredConfig =
            serde_json::from_slice(&bytes).map_err(|_| HostConfigError::InvalidConfig)?;
        let project =
            ProjectKey::parse(&config.project).map_err(|_| HostConfigError::InvalidConfig)?;
        if config.schema_version != 1
            || config.identity.host_id.is_nil()
            || config.identity.credential_id.is_nil()
            || config.identity.generation.is_nil()
            || config.binding.host_id() != config.identity.host_id.to_string()
        {
            return Err(HostConfigError::InvalidConfig);
        }
        if &config.binding != expected_binding || &project != expected_project {
            return Err(HostConfigError::ScopeMismatch);
        }
        let path = directory.join(format!("host-{}.json", config.identity.credential_id));
        let bytes = read_private_credential(&path)?;
        let credential =
            decode_host_credential(&bytes, expected_binding, expected_project, clock()?)?;
        if credential.identity() != &config.identity {
            return Err(HostConfigError::IdentityMismatch);
        }
        for (path, guard) in paths.iter().zip(&guards) {
            if !same_file(&open_directory(path)?, guard)? {
                return Err(StoreError::UnsafeStorage.into());
            }
        }
        Ok(Self { credential })
    }

    pub fn credential(&self) -> &LoadedHostCredential {
        &self.credential
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{
        mobile_connection::HostBinding,
        mobile_coordinator::{storage::CoordinatorStore, CredentialIdentity},
        mobile_host::credential::{import_host_credential, CredentialLoadError},
    };
    use gwt_core::repo_hash::ProjectKey;
    use serde_json::{json, Value};
    use std::{fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    fn fixture() -> (
        tempfile::TempDir,
        HostBinding,
        ProjectKey,
        CredentialIdentity,
    ) {
        let source = tempfile::tempdir().unwrap();
        fs::set_permissions(source.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = CoordinatorStore::initialize(source.path(), 100).unwrap();
        let host = Uuid::new_v4();
        let binding = HostBinding::new(&host.to_string(), "https://coordinator.example").unwrap();
        let project = ProjectKey::parse("0123456789abcdef").unwrap();
        let issued = store
            .issue(
                "https://coordinator.example",
                host,
                std::slice::from_ref(&project),
                200,
                100,
            )
            .unwrap();
        let home = tempfile::tempdir().unwrap();
        for name in [".gwt", ".gwt/mobile", ".gwt/mobile/host"] {
            let path = home.path().join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let imported =
            import_host_credential(home.path(), &issued.path, &binding, &project, || Ok(100))
                .unwrap();
        let path = home.path().join(".gwt/mobile/host/config.json");
        fs::write(&path, serde_json::to_vec(&json!({"schema_version":1,"binding":binding,"project":project.as_str(),"identity":imported.identity})).unwrap()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        (home, binding, project, imported.identity)
    }

    #[test]
    fn selects_the_imported_identity_and_never_substitutes_scope_or_generation() {
        let (home, binding, project, identity) = fixture();
        let load = || MobileHostConfig::load(home.path(), &binding, &project, || Ok(100));
        let loaded = load().unwrap();
        assert_eq!(loaded.credential().identity(), &identity);
        assert_eq!(loaded.credential().binding(), &binding);
        assert_eq!(loaded.credential().project(), &project);
        let foreign = ProjectKey::parse("1111111111111111").unwrap();
        assert!(matches!(
            MobileHostConfig::load(home.path(), &binding, &foreign, || Ok(100)),
            Err(HostConfigError::ScopeMismatch)
        ));
        let path = home.path().join(format!(
            ".gwt/mobile/host/host-{}.json",
            identity.credential_id
        ));
        let original = fs::read(&path).unwrap();
        let mut substituted: Value = serde_json::from_slice(&original).unwrap();
        substituted["identity"]["generation"] = json!(Uuid::new_v4());
        fs::write(&path, serde_json::to_vec(&substituted).unwrap()).unwrap();
        assert!(matches!(load(), Err(HostConfigError::IdentityMismatch)));
        substituted["identity"]["generation"] = json!(identity.generation);
        substituted["identity"]["credential_id"] = json!(Uuid::new_v4());
        fs::write(&path, serde_json::to_vec(&substituted).unwrap()).unwrap();
        assert!(matches!(load(), Err(HostConfigError::IdentityMismatch)));
        fs::write(&path, original).unwrap();
        assert!(load().is_ok());
    }

    #[test]
    fn rejects_invalid_configuration_without_fallback_and_recovers_after_correction() {
        let (home, binding, project, _) = fixture();
        let path = home.path().join(".gwt/mobile/host/config.json");
        let original = fs::read(&path).unwrap();
        let valid: Value = serde_json::from_slice(&original).unwrap();
        for (field, value) in [
            ("schema_version", json!(2)),
            ("project", json!("../project")),
            ("enable", json!(true)),
            ("secret", json!("forbidden")),
            ("identity", Value::Null),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(matches!(
                MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)),
                Err(HostConfigError::InvalidConfig)
            ));
        }
        let duplicate =
            String::from_utf8(original.clone())
                .unwrap()
                .replacen('{', "{\"schema_version\":1,", 1);
        fs::write(&path, duplicate).unwrap();
        assert!(matches!(
            MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)),
            Err(HostConfigError::InvalidConfig)
        ));
        fs::remove_file(&path).unwrap();
        assert!(MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)).is_err());
        assert!(!path.exists());
        fs::write(&path, original).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)).is_ok());
    }

    #[test]
    fn samples_time_after_the_exact_credential_read_and_refuses_clock_failure() {
        let (home, binding, project, identity) = fixture();
        let path = home.path().join(format!(
            ".gwt/mobile/host/host-{}.json",
            identity.credential_id
        ));
        assert!(matches!(
            MobileHostConfig::load(home.path(), &binding, &project, || Ok(200)),
            Err(HostConfigError::Credential(CredentialLoadError::Expired))
        ));
        assert!(matches!(
            MobileHostConfig::load(home.path(), &binding, &project, || Err(
                HostConfigError::ClockUnavailable
            )),
            Err(HostConfigError::ClockUnavailable)
        ));
        let loaded = MobileHostConfig::load(home.path(), &binding, &project, || {
            fs::write(&path, b"replaced after bounded read").unwrap();
            Ok(100)
        })
        .unwrap();
        assert_eq!(loaded.credential().identity(), &identity);
    }

    #[test]
    fn requires_private_non_repository_home_and_explicit_repair() {
        let (home, binding, project, _) = fixture();
        assert!(matches!(
            MobileHostConfig::load(std::path::Path::new("relative"), &binding, &project, || Ok(
                100
            )),
            Err(HostConfigError::InvalidHome)
        ));
        let directory = home.path().join(".gwt/mobile");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)).is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let marker = directory.join(".git");
        fs::write(&marker, "gitdir: unused-test-marker").unwrap();
        assert!(matches!(
            MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)),
            Err(HostConfigError::RepositoryStorage)
        ));
        fs::remove_file(marker).unwrap();
        assert!(MobileHostConfig::load(home.path(), &binding, &project, || {
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
            Ok(100)
        })
        .is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(MobileHostConfig::load(home.path(), &binding, &project, || Ok(100)).is_ok());
    }
}
