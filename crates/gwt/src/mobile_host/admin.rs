//! Offline PC setup only; existing private APIs own all credential storage.

use super::{
    credential::{
        import_host_credential, CredentialImportError, CredentialLoadError, ImportedHostCredential,
    },
    HostConfigError, MobileHostConfig,
};
use crate::{mobile_connection::HostBinding, mobile_coordinator::storage::StoreError};
use gwt_core::repo_hash::ProjectKey;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
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
    Import {
        source: PathBuf,
        binding: HostBinding,
        project: String,
    },
    Inspect {
        binding: HostBinding,
        project: String,
    },
}

#[derive(Debug)]
pub enum AdminError {
    InvalidRequest,
    UnsafeHome,
    ClockUnavailable,
    Import(CredentialImportError),
    Inspect(HostConfigError),
}

impl AdminError {
    /// Fixed diagnostics only: do not echo requests, paths or underlying errors.
    pub fn response(&self) -> Value {
        let code = match self {
            Self::InvalidRequest => "invalid_request",
            Self::UnsafeHome => "unsafe_storage",
            Self::ClockUnavailable => "clock_unavailable",
            Self::Import(error) => match error {
                CredentialImportError::InvalidPath => "invalid_request",
                CredentialImportError::RepositoryStorage => "repository_storage",
                CredentialImportError::ClockUnavailable => "clock_unavailable",
                CredentialImportError::AlreadyExists => "already_exists",
                CredentialImportError::Credential(error) => credential_code(error),
                CredentialImportError::Storage(error) => storage_code(error),
            },
            Self::Inspect(error) => match error {
                HostConfigError::InvalidConfig => "invalid_configuration",
                HostConfigError::ScopeMismatch => "scope_mismatch",
                HostConfigError::IdentityMismatch => "identity_mismatch",
                HostConfigError::InvalidHome => "unsafe_storage",
                HostConfigError::RepositoryStorage => "repository_storage",
                HostConfigError::ClockUnavailable => "clock_unavailable",
                HostConfigError::Credential(error) => credential_code(error),
                HostConfigError::Storage(error) => storage_code(error),
            },
        };
        let (diagnostic, recovery) = match code {
            "invalid_request" => ("Expected one strict schema1 JSON request at most 16 KiB, with independent valid binding/project and an absolute import source without parent components; request refused before storage access.", "Use --help, correct the request and retry."),
            "unsafe_storage" => ("Private storage validation failed; owned 0700 directories and single-link regular 0600 files without symlinks or macOS extended ACLs are required.", "Explicitly repair private storage and its trusted ancestors, then retry; no permissions are changed automatically."),
            "repository_storage" => ("Credential source or storage has repository markers; operation refused.", "Move private storage outside repositories and retry explicitly."),
            "clock_unavailable" => ("Trusted Unix time is unavailable; success was not established.", "Repair the clock and retry explicitly."),
            "already_exists" | "commit_uncertain" => ("The credential copy already exists or its commit outcome is uncertain; it was not replaced.", "Explicitly select the issuance receipt's public identity in config.json and run inspect; do not automatically retry, delete or reissue."),
            "scope_mismatch" => ("The credential or configuration does not match the independently expected Host/origin/Project.", "Correct the expected scope or explicitly select the correct imported credential, then inspect."),
            "identity_mismatch" => ("The imported credential identity differs from config.json.", "Correct the explicit configuration using the issuance receipt's public identity, then inspect."),
            "invalid_configuration" => ("The fixed config.json is not a valid schema1 Host configuration.", "Correct config.json with public binding, project and identity only, then inspect."),
            "invalid_credential" => ("The source credential is invalid or outside its validity period.", "Obtain the correct private issued credential and trusted time before explicitly retrying; local inspection does not prove registry acceptance."),
            "unsupported_private_storage" => ("Private storage validation is unavailable on this platform.", "Use a supported Unix host; Windows requires a private ACL adapter."),
            _ => ("Private storage is unavailable; success was not established.", "Inspect the private files and repair the cause; for an uncertain copy explicitly configure its public identity and run inspect before retrying."),
        };
        json!({"ok":false,"code":code,"diagnostic":diagnostic,"recovery":recovery})
    }
}

fn credential_code(error: &CredentialLoadError) -> &'static str {
    match error {
        CredentialLoadError::BindingMismatch | CredentialLoadError::ProjectDenied => {
            "scope_mismatch"
        }
        CredentialLoadError::Storage(error) => storage_code(error),
        _ => "invalid_credential",
    }
}

fn storage_code(error: &StoreError) -> &'static str {
    match error {
        StoreError::UnsafeStorage => "unsafe_storage",
        StoreError::UnsupportedPrivateStorage => "unsupported_private_storage",
        StoreError::CommitUncertain | StoreError::Unusable => "commit_uncertain",
        _ => "storage_unavailable",
    }
}

/// Execute a single offline request. Trusted time is sampled by the existing
/// private API after its credential reads, never before stdin or source I/O.
/// Import never changes configuration, registration or device authority.
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
    let (binding, project) = match &request.command {
        Command::Import {
            binding, project, ..
        }
        | Command::Inspect { binding, project } => (binding, project),
    };
    let project = ProjectKey::parse(project).map_err(|_| AdminError::InvalidRequest)?;
    if request.schema_version != 1 || binding.host_id() == Uuid::nil().to_string() {
        return Err(AdminError::InvalidRequest);
    }
    match request.command {
        Command::Import {
            source, binding, ..
        } => {
            if !source.is_absolute()
                || source
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
            {
                return Err(AdminError::InvalidRequest);
            }
            let receipt = import_host_credential(home, &source, &binding, &project, || {
                clock().map_err(|_| CredentialImportError::ClockUnavailable)
            })
            .map_err(AdminError::Import)?;
            import_response(receipt)
        }
        Command::Inspect { binding, .. } => {
            let config = MobileHostConfig::load(home, &binding, &project, || {
                clock().map_err(|_| HostConfigError::ClockUnavailable)
            })
            .map_err(AdminError::Inspect)?;
            let credential = config.credential();
            Ok(
                json!({"ok":true,"identity":credential.identity(),"binding":credential.binding(),"project":credential.project().as_str(),"expires_at":credential.expires_at()}),
            )
        }
    }
}

fn import_response(receipt: ImportedHostCredential) -> Result<Value, AdminError> {
    let path = serde_json::to_value(&receipt.path).map_err(|_| {
        AdminError::Import(CredentialImportError::Storage(StoreError::CommitUncertain))
    })?;
    Ok(json!({"ok":true,"identity":receipt.identity,"path":path}))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::mobile_coordinator::storage::CoordinatorStore;
    use serde_json::json;
    use std::{fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    #[test]
    fn imports_then_inspects_explicit_selection_without_changing_configuration() {
        let source = tempfile::tempdir().unwrap();
        fs::set_permissions(source.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = CoordinatorStore::initialize(source.path(), 100).unwrap();
        let host_id = Uuid::new_v4();
        let binding =
            HostBinding::new(&host_id.to_string(), "https://coordinator.example").unwrap();
        let project = ProjectKey::parse("0123456789abcdef").unwrap();
        let issued = store
            .issue(
                "https://coordinator.example",
                host_id,
                std::slice::from_ref(&project),
                200,
                100,
            )
            .unwrap();
        let original = fs::read(&issued.path).unwrap();
        let secret = serde_json::from_slice::<Value>(&original).unwrap()["secret"]
            .as_str()
            .unwrap()
            .to_owned();
        let home = tempfile::tempdir().unwrap();
        for name in [".gwt", ".gwt/mobile", ".gwt/mobile/host"] {
            let path = home.path().join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config_path = home.path().join(".gwt/mobile/host/config.json");
        let config = serde_json::to_vec(&json!({"schema_version":1,"binding":binding,"project":project.as_str(),"identity":issued.identity})).unwrap();
        fs::write(&config_path, &config).unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
        let request = serde_json::to_vec(&json!({"schema_version":1,"command":{"operation":"import","source":issued.path,"binding":binding,"project":project.as_str()}})).unwrap();
        let receipt = run(request.as_slice(), home.path(), || {
            fs::write(&issued.path, b"replaced at clock observation").unwrap();
            Ok(100)
        })
        .unwrap();
        assert_eq!(receipt["identity"], json!(issued.identity));
        assert_eq!(
            fs::read(receipt["path"].as_str().unwrap()).unwrap(),
            original
        );
        assert_eq!(fs::read(&config_path).unwrap(), config);
        fs::write(&issued.path, &original).unwrap();
        let duplicate = run(request.as_slice(), home.path(), || Ok(100))
            .unwrap_err()
            .response();
        assert_eq!(duplicate["code"], "already_exists");
        let inspect = serde_json::to_vec(&json!({"schema_version":1,"command":{"operation":"inspect","binding":binding,"project":project.as_str()}})).unwrap();
        let response = run(inspect.as_slice(), home.path(), || Ok(100)).unwrap();
        assert_eq!(response["identity"], json!(issued.identity));
        assert_eq!(response["expires_at"], 200);
        assert!(!response.to_string().contains(&secret));
        assert!(!duplicate.to_string().contains(&secret));
        let mut padded = inspect.clone();
        padded.resize(16 * 1024, b' ');
        assert!(run(padded.as_slice(), home.path(), || Ok(100)).is_ok());
        padded.push(b' ');
        assert!(matches!(
            run(padded.as_slice(), home.path(), || Ok(100)),
            Err(AdminError::InvalidRequest)
        ));
        let foreign = String::from_utf8(inspect)
            .unwrap()
            .replace(project.as_str(), "1111111111111111");
        assert_eq!(
            run(foreign.as_bytes(), home.path(), || Ok(100))
                .unwrap_err()
                .response()["code"],
            "scope_mismatch"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config);
    }

    #[test]
    fn reports_unrepresentable_published_path_without_panicking() {
        use crate::mobile_coordinator::CredentialIdentity;
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let receipt = ImportedHostCredential {
            identity: CredentialIdentity {
                host_id: Uuid::new_v4(),
                credential_id: Uuid::new_v4(),
                generation: Uuid::new_v4(),
            },
            path: PathBuf::from(OsString::from_vec(b"/private/pc-\xff/host.json".to_vec())),
        };
        assert_eq!(
            import_response(receipt).unwrap_err().response()["code"],
            "commit_uncertain"
        );
    }

    #[test]
    fn rejects_invalid_requests_before_clock_or_storage_without_echoing_input() {
        let home = tempfile::tempdir().unwrap();
        let valid = format!(
            r#"{{"schema_version":1,"command":{{"operation":"inspect","binding":{{"host_id":"{}","coordinator_origin":"https://coordinator.example"}},"project":"0123456789abcdef"}}}}"#,
            Uuid::new_v4()
        );
        for request in [
            valid.replace(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1",
            ),
            valid.replace(
                "\"operation\":\"inspect\"",
                "\"operation\":\"inspect\",\"secret\":\"DO_NOT_ECHO\"",
            ),
            valid.replace("inspect", "enable"),
            valid.replace("0123456789abcdef", "not-a-project"),
            valid.replace(
                "\"operation\":\"inspect\"",
                "\"operation\":\"import\",\"source\":\"relative.json\"",
            ),
        ] {
            let error = run(request.as_bytes(), home.path(), || {
                panic!("invalid request reached clock")
            })
            .unwrap_err();
            assert!(matches!(error, AdminError::InvalidRequest));
            assert!(!error.response().to_string().contains("DO_NOT_ECHO"));
            assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
        }
    }
}
