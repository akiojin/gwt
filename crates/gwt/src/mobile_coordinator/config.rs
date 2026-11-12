//! Deployment configuration for the future coordinator listener.
//!
//! `from_json` performs no I/O; `load` validates only private config storage.
//! `load_from_home` also checks the fixed private coordinator directory chain.
//! `tls_server_config` reads the configured certificate/key from private
//! storage and checks that they match. None grants registration or device
//! authority. Before serving, the caller must retain the existing
//! CoordinatorStore's separate registry lock; certificate names and validity
//! periods are left to the clients' standard verification. Registry paths
//! remain fixed by that store. TURN configuration is not supported yet.

use crate::mobile_connection::normalize_coordinator_origin;
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

const MAX_CONFIG_BYTES: usize = 16 * 1024;
#[cfg(unix)]
const MAX_TLS_FILE_BYTES: usize = 64 * 1024;
const MAX_CERTIFICATES: usize = 8;

#[cfg(unix)]
fn storage_error(error: super::storage::StoreError) -> ConfigError {
    use super::storage::StoreError;
    match error {
        StoreError::UnsafeStorage => ConfigError::UnsafeStorage,
        StoreError::Io(error)
            if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) =>
        {
            ConfigError::UnsafeStorage
        }
        StoreError::UnsupportedPrivateStorage => ConfigError::UnsupportedPrivateStorage,
        _ => ConfigError::StorageUnavailable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("unsafe coordinator config storage; use a canonical absolute directory outside repositories, owned 0700, and an owned single-link regular 0600 config.json, without symlinks or macOS extended ACLs; explicitly repair storage and load again")]
    UnsafeStorage,
    #[error("coordinator config storage is inside a repository; select a trusted private directory outside repositories and load again")]
    RepositoryStorage,
    #[error("coordinator config storage is unavailable; inspect the selected directory and config.json, explicitly repair missing or unreadable storage, and load again")]
    StorageUnavailable,
    #[error("private coordinator config storage is unsupported on this platform; use a supported Unix host until a Windows ACL adapter is available")]
    UnsupportedPrivateStorage,
    #[error("coordinator config exceeds 16 KiB; reduce the configuration and parse again")]
    TooLarge,
    #[error("invalid coordinator config JSON: required fields must appear exactly once and unknown fields are unsupported; correct the configuration and parse again")]
    InvalidJson,
    #[error("unsupported coordinator config schema; use schema_version 1 and parse again")]
    InvalidSchema,
    #[error("invalid coordinator origin; use an HTTPS authority with a nonzero port, without userinfo, path, query or fragment, and parse again")]
    InvalidOrigin,
    #[error(
        "invalid coordinator bind; use a numeric IP address and port 1..=65535 and parse again"
    )]
    InvalidBind,
    #[error("invalid TLS paths; use distinct absolute file paths without parent components or control characters and parse again; private storage and TLS still require validation before serving")]
    InvalidTlsPath,
    #[error("invalid coordinator limits; use positive values no greater than registrations=64, peers_per_host=8, device rate/burst=32/64, Host rate/burst=256/512, with burst at least rate; correct limits and parse again")]
    InvalidLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TlsError {
    #[error("unsafe coordinator TLS storage; place the certificate and private key as owned single-link regular 0600 files in a canonical owned 0700 directory outside repositories, without symlinks or macOS extended ACLs; explicitly repair storage and load again")]
    UnsafeStorage,
    #[error("coordinator TLS storage is inside a repository; move the certificate and private key to a trusted private directory outside repositories and load again")]
    RepositoryStorage,
    #[error("coordinator TLS storage is unavailable; inspect the configured certificate and private key files, explicitly repair missing or unreadable storage, and load again")]
    StorageUnavailable,
    #[error("private coordinator TLS storage is unsupported on this platform; use a supported Unix host until a Windows ACL adapter is available")]
    UnsupportedPrivateStorage,
    #[error("a coordinator TLS file exceeds 64 KiB; reduce it and load again")]
    TooLarge,
    #[error("invalid coordinator TLS certificate; provide 1 to 8 PEM CERTIFICATE blocks and nothing else, end-entity first, and load again")]
    InvalidCertificate,
    #[error("invalid coordinator TLS private key; provide exactly one supported PEM private key and nothing else, and load again")]
    InvalidPrivateKey,
    #[error("the coordinator TLS private key does not match the certificate; provide the matching pair and load again")]
    KeyMismatch,
}

impl From<ConfigError> for TlsError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::RepositoryStorage => Self::RepositoryStorage,
            ConfigError::StorageUnavailable => Self::StorageUnavailable,
            ConfigError::UnsupportedPrivateStorage => Self::UnsupportedPrivateStorage,
            ConfigError::TooLarge => Self::TooLarge,
            _ => Self::UnsafeStorage,
        }
    }
}

/// Admission ceilings, not proof that any listener or rate limiter is active.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinatorLimits {
    pub registrations: u16,
    pub peers_per_host: u16,
    pub device_messages_per_second: u16,
    pub device_burst: u16,
    pub host_messages_per_second: u16,
    pub host_burst: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredConfig {
    schema_version: u32,
    origin: String,
    bind: String,
    tls_certificate: PathBuf,
    tls_private_key: PathBuf,
    #[serde(deserialize_with = "deserialize_object")]
    limits: CoordinatorLimits,
}

fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ObjectVisitor<T>(std::marker::PhantomData<T>);

    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object with named fields")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
}

/// Validated syntax. TLS paths have not been opened or proven private.
pub struct CoordinatorConfig {
    origin: String,
    bind: SocketAddr,
    tls_certificate: PathBuf,
    tls_private_key: PathBuf,
    limits: CoordinatorLimits,
}

impl CoordinatorConfig {
    /// Read only `HOME/.gwt/mobile/coordinator/config.json`.
    ///
    /// The caller selects a trusted home with non-replaceable ancestors and
    /// prevents concurrent in-place edits. The three existing private directory
    /// descriptors remain held until after the read and identity recheck. HOME
    /// itself need not be 0700. No files or permissions are created or changed;
    /// TLS validation and the registry lock remain separate caller obligations.
    #[cfg(unix)]
    pub fn load_from_home(home: &Path) -> Result<Self, ConfigError> {
        use super::storage::{open_directory, same_file};
        if !home.is_absolute() || home.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(ConfigError::UnsafeStorage);
        }
        let paths = [
            home.join(".gwt"),
            home.join(".gwt/mobile"),
            home.join(".gwt/mobile/coordinator"),
        ];
        let guards = paths
            .iter()
            .map(|path| open_directory(path).map_err(storage_error))
            .collect::<Result<Vec<_>, _>>()?;
        let directory = paths[2]
            .canonicalize()
            .map_err(|_| ConfigError::StorageUnavailable)?;
        let config = Self::load(&directory)?;
        for (path, guard) in paths.iter().zip(&guards) {
            if !same_file(guard, &open_directory(path).map_err(storage_error)?)
                .map_err(storage_error)?
            {
                return Err(ConfigError::UnsafeStorage);
            }
        }
        Ok(config)
    }

    #[cfg(not(unix))]
    pub fn load_from_home(_: &Path) -> Result<Self, ConfigError> {
        Err(ConfigError::UnsupportedPrivateStorage)
    }

    /// Read fixed `config.json` from a trusted canonical private directory.
    ///
    /// The caller must keep ancestors trusted and non-replaceable and prevent
    /// concurrent in-place edits. This does not validate the private HOME layout,
    /// TLS files or their contents, or acquire the coordinator registry lock.
    /// Missing/unsafe files are errors; no files or permissions are changed.
    #[cfg(unix)]
    pub fn load(directory: &Path) -> Result<Self, ConfigError> {
        Self::from_json(&read_private(
            directory,
            "config.json".as_ref(),
            MAX_CONFIG_BYTES,
        )?)
    }

    /// Private storage requires a platform-specific ownership/ACL adapter.
    #[cfg(not(unix))]
    pub fn load(_: &Path) -> Result<Self, ConfigError> {
        Err(ConfigError::UnsupportedPrivateStorage)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ConfigError> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let config: StoredConfig =
            deserialize_object(&mut deserializer).map_err(|_| ConfigError::InvalidJson)?;
        deserializer.end().map_err(|_| ConfigError::InvalidJson)?;
        if config.schema_version != 1 {
            return Err(ConfigError::InvalidSchema);
        }
        let origin =
            normalize_coordinator_origin(&config.origin).map_err(|_| ConfigError::InvalidOrigin)?;
        if reqwest::Url::parse(&origin)
            .map_err(|_| ConfigError::InvalidOrigin)?
            .port()
            == Some(0)
        {
            return Err(ConfigError::InvalidOrigin);
        }
        let bind: SocketAddr = config.bind.parse().map_err(|_| ConfigError::InvalidBind)?;
        if bind.port() == 0 {
            return Err(ConfigError::InvalidBind);
        }
        for path in [&config.tls_certificate, &config.tls_private_key] {
            if !path.is_absolute()
                || path.file_name().is_none()
                || path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
                || path.to_string_lossy().chars().any(char::is_control)
            {
                return Err(ConfigError::InvalidTlsPath);
            }
        }
        if config.tls_certificate == config.tls_private_key {
            return Err(ConfigError::InvalidTlsPath);
        }
        let limits = config.limits;
        for (value, maximum) in [
            (limits.registrations, 64),
            (limits.peers_per_host, 8),
            (limits.device_messages_per_second, 32),
            (limits.device_burst, 64),
            (limits.host_messages_per_second, 256),
            (limits.host_burst, 512),
        ] {
            if value == 0 || value > maximum {
                return Err(ConfigError::InvalidLimits);
            }
        }
        if limits.device_burst < limits.device_messages_per_second
            || limits.host_burst < limits.host_messages_per_second
        {
            return Err(ConfigError::InvalidLimits);
        }
        Ok(Self {
            origin,
            bind,
            tls_certificate: config.tls_certificate,
            tls_private_key: config.tls_private_key,
            limits,
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn bind(&self) -> SocketAddr {
        self.bind
    }

    pub fn tls_certificate(&self) -> &Path {
        &self.tls_certificate
    }

    pub fn tls_private_key(&self) -> &Path {
        &self.tls_private_key
    }

    pub fn limits(&self) -> &CoordinatorLimits {
        &self.limits
    }

    /// Read the configured certificate chain and private key from private
    /// storage and build a ring-backed server config without client auth.
    ///
    /// Each file must be an owned single-link regular 0600 file in a canonical
    /// owned 0700 directory outside repositories. The key must match the
    /// end-entity certificate; only that first certificate is parsed here.
    /// Names, validity periods and the rest of the chain are left to clients.
    /// No files or permissions are created or changed.
    pub fn tls_server_config(&self) -> Result<rustls::ServerConfig, TlsError> {
        use rustls::pki_types::{
            pem::{PemObject, SectionKind},
            CertificateDer, PrivateKeyDer,
        };
        let certificate = read_tls_file(&self.tls_certificate)?;
        // Best effort: the raw PEM is wiped; rustls owns the decoded key.
        let key = zeroize::Zeroizing::new(read_tls_file(&self.tls_private_key)?);
        let certificates = <(SectionKind, Vec<u8>)>::pem_slice_iter(&certificate)
            .map(|section| match section {
                Ok((SectionKind::Certificate, der)) => Ok(CertificateDer::from(der)),
                _ => Err(TlsError::InvalidCertificate),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if certificates.is_empty() || certificates.len() > MAX_CERTIFICATES {
            return Err(TlsError::InvalidCertificate);
        }
        let mut sections = <(SectionKind, Vec<u8>)>::pem_slice_iter(&key);
        let key = match (sections.next(), sections.next()) {
            (Some(Ok((kind, der))), None) => PrivateKeyDer::from_pem(kind, der),
            _ => None,
        }
        .ok_or(TlsError::InvalidPrivateKey)?;
        rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| TlsError::InvalidPrivateKey)?
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map_err(|error| match error {
            rustls::Error::InconsistentKeys(_) => TlsError::KeyMismatch,
            rustls::Error::InvalidCertificate(_) => TlsError::InvalidCertificate,
            _ => TlsError::InvalidPrivateKey,
        })
    }
}

#[cfg(unix)]
fn read_tls_file(path: &Path) -> Result<Vec<u8>, TlsError> {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(TlsError::UnsafeStorage);
    };
    Ok(read_private(directory, name, MAX_TLS_FILE_BYTES)?)
}

#[cfg(not(unix))]
fn read_tls_file(_: &Path) -> Result<Vec<u8>, TlsError> {
    Err(TlsError::UnsupportedPrivateStorage)
}

/// Read at most `limit` bytes of `directory/name` into a preallocated buffer,
/// then recheck directory and file identity.
#[cfg(unix)]
fn read_private(
    directory: &Path,
    name: &std::ffi::OsStr,
    limit: usize,
) -> Result<Vec<u8>, ConfigError> {
    use super::storage::{has_repository_marker, open_directory, open_private_file, same_file};
    use std::io::Read;

    if !directory.is_absolute()
        || directory
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(ConfigError::UnsafeStorage);
    }
    if directory
        .canonicalize()
        .map_err(|_| ConfigError::StorageUnavailable)?
        != directory
    {
        return Err(ConfigError::UnsafeStorage);
    }
    let parent = open_directory(directory).map_err(storage_error)?;
    if has_repository_marker(directory).map_err(storage_error)? {
        return Err(ConfigError::RepositoryStorage);
    }
    let path = directory.join(name);
    let file = open_private_file(&path, false).map_err(storage_error)?;
    let mut bytes = Vec::with_capacity(limit + 1);
    (&file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ConfigError::StorageUnavailable)?;
    if bytes.len() > limit {
        return Err(ConfigError::TooLarge);
    }
    if !same_file(&parent, &open_directory(directory).map_err(storage_error)?)
        .map_err(storage_error)?
        || !same_file(
            &file,
            &open_private_file(&path, false).map_err(storage_error)?,
        )
        .map_err(storage_error)?
    {
        return Err(ConfigError::UnsafeStorage);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::path::Path;

    fn valid() -> Value {
        let directory = Path::new(if cfg!(windows) {
            r"C:\private\mobile"
        } else {
            "/private/mobile"
        });
        json!({
            "schema_version": 1,
            "origin": "https://COORDINATOR.example:443/",
            "bind": "127.0.0.1:8443",
            "tls_certificate": directory.join("cert.pem"),
            "tls_private_key": directory.join("key.pem"),
            "limits": {
                "registrations": 64,
                "peers_per_host": 8,
                "device_messages_per_second": 32,
                "device_burst": 64,
                "host_messages_per_second": 256,
                "host_burst": 512
            }
        })
    }

    fn parse(value: &Value) -> Result<CoordinatorConfig, ConfigError> {
        CoordinatorConfig::from_json(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn accepts_explicit_bounded_configuration_and_normalizes_origin() {
        let mut value = valid();
        let config = parse(&value).unwrap();
        assert_eq!(config.origin(), "https://coordinator.example");
        assert_eq!(config.bind(), "127.0.0.1:8443".parse().unwrap());
        assert_eq!(
            config.tls_certificate(),
            Path::new(value["tls_certificate"].as_str().unwrap())
        );
        assert_eq!(
            config.tls_private_key(),
            Path::new(value["tls_private_key"].as_str().unwrap())
        );
        assert_eq!(config.limits().registrations, 64);
        value["bind"] = json!("[::1]:443");
        value["limits"]["registrations"] = json!(1);
        assert!(parse(&value).is_ok());
    }

    #[test]
    fn refuses_unknown_duplicate_missing_and_oversized_json_without_echoing_input() {
        let mut value = valid();
        value["turn"] = json!("secret-sentinel");
        let error = parse(&value).err().unwrap();
        assert_eq!(error, ConfigError::InvalidJson);
        assert!(!error.to_string().contains("secret-sentinel"));
        value = valid();
        value["limits"]["extra"] = json!(1);
        assert!(matches!(parse(&value), Err(ConfigError::InvalidJson)));
        value = valid();
        value.as_object_mut().unwrap().remove("bind");
        assert!(matches!(parse(&value), Err(ConfigError::InvalidJson)));
        let duplicate =
            serde_json::to_string(&valid())
                .unwrap()
                .replacen('{', "{\"schema_version\":1,", 1);
        assert!(matches!(
            CoordinatorConfig::from_json(duplicate.as_bytes()),
            Err(ConfigError::InvalidJson)
        ));
        let value = valid();
        let positional = json!([
            value["schema_version"],
            value["origin"],
            value["bind"],
            value["tls_certificate"],
            value["tls_private_key"],
            value["limits"]
        ]);
        let mut value = valid();
        value["limits"] = json!([64, 8, 32, 64, 256, 512]);
        assert_eq!(
            [parse(&positional).err(), parse(&value).err()],
            [Some(ConfigError::InvalidJson); 2]
        );
        let trailing = format!("{} true", valid());
        assert!(matches!(
            CoordinatorConfig::from_json(trailing.as_bytes()),
            Err(ConfigError::InvalidJson)
        ));
        assert!(matches!(
            CoordinatorConfig::from_json(&vec![b' '; 16 * 1024 + 1]),
            Err(ConfigError::TooLarge)
        ));
        assert!(parse(&valid()).is_ok());
    }

    #[test]
    fn refuses_other_schemas_and_unsafe_origins_then_accepts_correction() {
        let mut value = valid();
        value["schema_version"] = json!(2);
        assert!(matches!(parse(&value), Err(ConfigError::InvalidSchema)));
        value = valid();
        for origin in [
            "http://coordinator.example",
            "https://user@coordinator.example",
            "https://@coordinator.example",
            "https://coordinator.example/path",
            "https://coordinator.example/.",
            "https://coordinator.example?token=x",
            "https://coordinator.example#secret",
            " https://coordinator.example",
            "https://coordinator.example:0",
        ] {
            value["origin"] = json!(origin);
            assert!(
                matches!(parse(&value), Err(ConfigError::InvalidOrigin)),
                "{origin}"
            );
        }
        assert!(parse(&valid()).is_ok());
    }

    #[test]
    fn refuses_non_numeric_or_ephemeral_bind_and_ambiguous_tls_paths() {
        let mut value = valid();
        for bind in ["localhost:443", "127.0.0.1:0", "127.0.0.1", "[::1]:0"] {
            value["bind"] = json!(bind);
            assert!(matches!(parse(&value), Err(ConfigError::InvalidBind)));
        }
        value = valid();
        for field in ["tls_certificate", "tls_private_key"] {
            for path in [
                "cert.pem",
                "/private/../cert.pem",
                "/",
                "/private/key\n.pem",
            ] {
                value[field] = json!(path);
                assert!(matches!(parse(&value), Err(ConfigError::InvalidTlsPath)));
            }
            value = valid();
        }
        value["tls_private_key"] = value["tls_certificate"].clone();
        assert!(matches!(parse(&value), Err(ConfigError::InvalidTlsPath)));
        assert!(parse(&valid()).is_ok());
    }

    #[test]
    fn refuses_zero_or_above_policy_limits_and_bursts_below_rate() {
        for (field, ceiling) in [
            ("registrations", 64),
            ("peers_per_host", 8),
            ("device_messages_per_second", 32),
            ("device_burst", 64),
            ("host_messages_per_second", 256),
            ("host_burst", 512),
        ] {
            for invalid in [0, ceiling + 1] {
                let mut value = valid();
                value["limits"][field] = json!(invalid);
                assert!(
                    matches!(parse(&value), Err(ConfigError::InvalidLimits)),
                    "{field}={invalid}"
                );
            }
        }
        let mut value = valid();
        value["limits"]["device_burst"] = json!(31);
        assert!(matches!(parse(&value), Err(ConfigError::InvalidLimits)));
        value = valid();
        value["limits"]["host_burst"] = json!(255);
        assert!(matches!(parse(&value), Err(ConfigError::InvalidLimits)));
        assert!(parse(&valid()).is_ok());
    }

    #[cfg(unix)]
    fn private_config() -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().canonicalize().unwrap();
        write_config(&path, &serde_json::to_vec(&valid()).unwrap());
        (directory, path)
    }

    #[cfg(unix)]
    fn write_config(directory: &Path, bytes: &[u8]) {
        write_private(&directory.join("config.json"), bytes);
    }

    #[test]
    #[cfg(unix)]
    fn loads_private_config_and_recovers_after_missing_invalid_and_oversized_input() {
        let (_directory, path) = private_config();
        assert_eq!(
            CoordinatorConfig::load(&path).unwrap().origin(),
            "https://coordinator.example"
        );
        std::fs::remove_file(path.join("config.json")).unwrap();
        assert!(matches!(
            CoordinatorConfig::load(&path),
            Err(ConfigError::StorageUnavailable)
        ));
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        write_config(&path, b"secret-sentinel");
        let error = CoordinatorConfig::load(&path).err().unwrap();
        assert_eq!(error, ConfigError::InvalidJson);
        assert!(!error.to_string().contains("secret-sentinel"));
        write_config(&path, &vec![b' '; MAX_CONFIG_BYTES + 1]);
        assert!(matches!(
            CoordinatorConfig::load(&path),
            Err(ConfigError::TooLarge)
        ));
        let mut bytes = serde_json::to_vec(&valid()).unwrap();
        bytes.resize(MAX_CONFIG_BYTES, b' ');
        write_config(&path, &bytes);
        assert_eq!(
            CoordinatorConfig::load(&path).unwrap().bind(),
            "127.0.0.1:8443".parse().unwrap()
        );
        assert_eq!(std::fs::read(path.join("config.json")).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
    }

    #[test]
    #[cfg(unix)]
    fn refuses_unsafe_config_storage_and_accepts_explicit_repairs() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (_directory, path) = private_config();
        let file = path.join("config.json");
        for (target, unsafe_mode, private_mode) in [(&file, 0o644, 0o600), (&path, 0o755, 0o700)] {
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(unsafe_mode)).unwrap();
            assert!(matches!(
                CoordinatorConfig::load(&path),
                Err(ConfigError::UnsafeStorage)
            ));
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(private_mode))
                .unwrap();
            assert!(CoordinatorConfig::load(&path).is_ok());
        }
        let saved = path.join("saved.json");
        std::fs::rename(&file, &saved).unwrap();
        symlink(&saved, &file).unwrap();
        assert!(matches!(
            CoordinatorConfig::load(&path),
            Err(ConfigError::UnsafeStorage)
        ));
        std::fs::remove_file(&file).unwrap();
        std::fs::rename(&saved, &file).unwrap();
        assert!(CoordinatorConfig::load(&path).is_ok());
        std::fs::hard_link(&file, &saved).unwrap();
        assert!(matches!(
            CoordinatorConfig::load(&path),
            Err(ConfigError::UnsafeStorage)
        ));
        std::fs::remove_file(&saved).unwrap();
        assert!(CoordinatorConfig::load(&path).is_ok());
        symlink("missing-git", path.join(".git")).unwrap();
        assert!(matches!(
            CoordinatorConfig::load(&path),
            Err(ConfigError::RepositoryStorage)
        ));
        std::fs::remove_file(path.join(".git")).unwrap();
        assert!(CoordinatorConfig::load(&path).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn refuses_relative_parent_and_symlink_directory_aliases() {
        use std::os::unix::fs::symlink;
        let (_directory, path) = private_config();
        let alias = path.join("alias");
        symlink(&path, &alias).unwrap();
        for invalid in [
            Path::new("relative").to_owned(),
            path.join("missing/.."),
            alias.clone(),
        ] {
            let error = CoordinatorConfig::load(&invalid).err().unwrap();
            assert_eq!(error, ConfigError::UnsafeStorage);
            assert!(!error.to_string().contains(path.to_str().unwrap()));
        }
        std::fs::remove_file(alias).unwrap();
        assert!(CoordinatorConfig::load(&path).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn loads_only_fixed_home_config_and_requires_private_ancestors() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (_directory, home) = private_config();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            CoordinatorConfig::load_from_home(&home),
            Err(ConfigError::StorageUnavailable)
        ));
        assert!(!home.join(".gwt").exists());
        let paths = [
            home.join(".gwt"),
            home.join(".gwt/mobile"),
            home.join(".gwt/mobile/coordinator"),
        ];
        for path in &paths {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(matches!(
            CoordinatorConfig::load_from_home(&home),
            Err(ConfigError::StorageUnavailable)
        ));
        let file = paths[2].join("config.json");
        std::fs::rename(home.join("config.json"), &file).unwrap();
        let original = std::fs::read(&file).unwrap();
        write_config(&paths[2], b"secret-sentinel");
        assert_eq!(
            CoordinatorConfig::load_from_home(&home).err(),
            Some(ConfigError::InvalidJson)
        );
        write_config(&paths[2], &original);
        assert!(CoordinatorConfig::load_from_home(&home).is_ok());
        for invalid in [Path::new("relative").to_owned(), home.join("missing/..")] {
            assert!(matches!(
                CoordinatorConfig::load_from_home(&invalid),
                Err(ConfigError::UnsafeStorage)
            ));
        }
        for path in &paths {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(matches!(
                CoordinatorConfig::load_from_home(&home),
                Err(ConfigError::UnsafeStorage)
            ));
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o755
            );
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let saved = home.join("saved");
            std::fs::rename(path, &saved).unwrap();
            symlink(&saved, path).unwrap();
            assert!(matches!(
                CoordinatorConfig::load_from_home(&home),
                Err(ConfigError::UnsafeStorage)
            ));
            std::fs::remove_file(path).unwrap();
            std::fs::rename(&saved, path).unwrap();
            assert_eq!(
                CoordinatorConfig::load_from_home(&home).unwrap().origin(),
                "https://coordinator.example"
            );
        }
        symlink("missing-git", home.join(".git")).unwrap();
        assert!(matches!(
            CoordinatorConfig::load_from_home(&home),
            Err(ConfigError::RepositoryStorage)
        ));
        std::fs::remove_file(home.join(".git")).unwrap();
        assert!(CoordinatorConfig::load_from_home(&home).is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert_eq!(std::fs::read_dir(&paths[2]).unwrap().count(), 1);
    }

    #[cfg(unix)]
    fn write_private(path: &Path, bytes: &[u8]) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(unix)]
    fn tls_config(directory: &Path) -> CoordinatorConfig {
        let mut value = valid();
        value["tls_certificate"] = json!(directory.join("cert.pem"));
        value["tls_private_key"] = json!(directory.join("key.pem"));
        parse(&value).unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn builds_tls_server_config_only_from_private_matching_material() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (_directory, path) = private_config();
        let config = tls_config(&path);
        let (cert, key) = (path.join("cert.pem"), path.join("key.pem"));
        let issued =
            rcgen::generate_simple_self_signed(vec!["coordinator.example".into()]).unwrap();
        let (cert_pem, key_pem) = (issued.cert.pem(), issued.signing_key.serialize_pem());
        let tls = |expected: Option<TlsError>| {
            assert_eq!(config.tls_server_config().err(), expected);
        };
        tls(Some(TlsError::StorageUnavailable));
        write_private(&cert, cert_pem.as_bytes());
        write_private(&key, key_pem.as_bytes());
        tls(None);

        let other = rcgen::KeyPair::generate().unwrap().serialize_pem();
        write_private(&key, other.as_bytes());
        tls(Some(TlsError::KeyMismatch));
        for invalid in [
            String::new(),
            "secret-sentinel".into(),
            format!("{key_pem}{other}"),
            cert_pem.clone(),
        ] {
            write_private(&key, invalid.as_bytes());
            let error = config.tls_server_config().err().unwrap();
            assert_eq!(error, TlsError::InvalidPrivateKey);
            assert!(!error.to_string().contains("secret-sentinel"));
        }
        write_private(&key, key_pem.as_bytes());
        for invalid in [
            String::new(),
            key_pem.clone(),
            format!("{cert_pem}{key_pem}"),
            cert_pem.repeat(9),
        ] {
            write_private(&cert, invalid.as_bytes());
            tls(Some(TlsError::InvalidCertificate));
        }
        write_private(
            &cert,
            b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        );
        tls(Some(TlsError::InvalidCertificate));
        write_private(&cert, &vec![b' '; MAX_TLS_FILE_BYTES + 1]);
        tls(Some(TlsError::TooLarge));
        write_private(&cert, cert_pem.as_bytes());
        write_private(&key, &vec![b' '; MAX_TLS_FILE_BYTES + 1]);
        tls(Some(TlsError::TooLarge));
        write_private(&key, key_pem.as_bytes());
        write_private(&cert, cert_pem.repeat(8).as_bytes());
        tls(None);
        write_private(&cert, cert_pem.as_bytes());
        tls(None);

        for (target, unsafe_mode, private_mode) in [
            (&cert, 0o644, 0o600),
            (&key, 0o644, 0o600),
            (&path, 0o755, 0o700),
        ] {
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(unsafe_mode)).unwrap();
            tls(Some(TlsError::UnsafeStorage));
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(private_mode))
                .unwrap();
            tls(None);
        }
        let saved = path.join("saved.pem");
        std::fs::rename(&key, &saved).unwrap();
        symlink(&saved, &key).unwrap();
        tls(Some(TlsError::UnsafeStorage));
        std::fs::remove_file(&key).unwrap();
        std::fs::rename(&saved, &key).unwrap();
        symlink("missing-git", path.join(".git")).unwrap();
        tls(Some(TlsError::RepositoryStorage));
        std::fs::remove_file(path.join(".git")).unwrap();
        tls(None);
        assert_eq!(std::fs::read(&key).unwrap(), key_pem.as_bytes());
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 3);
    }

    #[test]
    #[cfg(not(unix))]
    fn config_loading_requires_supported_private_storage() {
        assert!(matches!(
            CoordinatorConfig::load(Path::new("unused")),
            Err(ConfigError::UnsupportedPrivateStorage)
        ));
        assert!(matches!(
            CoordinatorConfig::load_from_home(Path::new("unused")),
            Err(ConfigError::UnsupportedPrivateStorage)
        ));
        assert_eq!(
            parse(&valid()).unwrap().tls_server_config().err(),
            Some(TlsError::UnsupportedPrivateStorage)
        );
    }
}
