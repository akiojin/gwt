//! Syntax-only deployment configuration for the future coordinator listener.
//!
//! Parsing performs no I/O and grants no registration or device authority.
//! Before serving, the caller must independently validate private storage,
//! certificate/key contents, permissions, ownership, ACLs and symlinks, and
//! retain the existing CoordinatorStore's separate registry lock. Registry
//! paths remain fixed by that store. TURN configuration is not supported yet.

use crate::mobile_connection::normalize_coordinator_origin;
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

const MAX_CONFIG_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
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
    limits: CoordinatorLimits,
}

/// Validated syntax only. Paths have not been opened or proven private.
pub struct CoordinatorConfig {
    origin: String,
    bind: SocketAddr,
    tls_certificate: PathBuf,
    tls_private_key: PathBuf,
    limits: CoordinatorLimits,
}

impl CoordinatorConfig {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ConfigError> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let config: StoredConfig =
            serde_json::from_slice(bytes).map_err(|_| ConfigError::InvalidJson)?;
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
}
