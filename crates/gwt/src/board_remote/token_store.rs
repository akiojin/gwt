//! Secure per-provider OAuth token storage for remote Board providers
//! (SPEC-2963 FR-006).
//!
//! Tokens never live in `config.toml`. They are written to a
//! permission-restricted JSON file under `~/.gwt/credentials/`
//! (`board-<provider>.json`, mode 0600 on Unix; user-profile scoped on
//! Windows). The `*_in` functions take an explicit directory so they are unit
//! testable against a temp dir.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// OAuth token set persisted for a remote provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    /// Bearer access token used for API calls.
    pub access_token: String,
    /// Refresh token used to mint a new access token when expired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Absolute expiry of the access token, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl TokenSet {
    /// Whether the access token is expired at `now`. A token with no known
    /// expiry is treated as not-expired (callers may still refresh on 401).
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expiry| now >= expiry)
    }
}

fn token_file(dir: &Path, provider: &str) -> PathBuf {
    dir.join(format!("board-{provider}.json"))
}

/// Default credentials directory: `~/.gwt/credentials`.
pub fn default_dir() -> PathBuf {
    gwt_core::paths::gwt_home().join("credentials")
}

/// Save `token` for `provider` under `dir` with restricted permissions.
pub fn save_in(dir: &Path, provider: &str, token: &TokenSet) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = token_file(dir, provider);
    let json = serde_json::to_vec_pretty(token).map_err(io::Error::other)?;
    fs::write(&path, &json)?;
    restrict_permissions(&path)?;
    Ok(())
}

/// Load the token for `provider` from `dir`, if present.
pub fn load_in(dir: &Path, provider: &str) -> io::Result<Option<TokenSet>> {
    let path = token_file(dir, provider);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)?;
    let token = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    Ok(Some(token))
}

/// Remove the stored token for `provider` under `dir` (idempotent).
pub fn clear_in(dir: &Path, provider: &str) -> io::Result<()> {
    let path = token_file(dir, provider);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Save into the default credentials directory.
pub fn save(provider: &str, token: &TokenSet) -> io::Result<()> {
    save_in(&default_dir(), provider, token)
}

/// Load from the default credentials directory.
pub fn load(provider: &str) -> io::Result<Option<TokenSet>> {
    load_in(&default_dir(), provider)
}

/// Clear from the default credentials directory.
pub fn clear(provider: &str) -> io::Result<()> {
    clear_in(&default_dir(), provider)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> io::Result<()> {
    // Windows: the file lives under the user profile (`~/.gwt`), whose ACL
    // already restricts it to the owner. Explicit ACL tightening is a
    // follow-up; never widen access here.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    fn token(access: &str) -> TokenSet {
        TokenSet {
            access_token: access.to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: None,
        }
    }

    #[test]
    fn save_load_clear_roundtrip() {
        let dir = tempdir().unwrap();
        assert_eq!(load_in(dir.path(), "slack").unwrap(), None);

        let stored = token("xoxb-1");
        save_in(dir.path(), "slack", &stored).unwrap();
        assert_eq!(load_in(dir.path(), "slack").unwrap(), Some(stored));

        clear_in(dir.path(), "slack").unwrap();
        assert_eq!(load_in(dir.path(), "slack").unwrap(), None);
        // clear is idempotent.
        clear_in(dir.path(), "slack").unwrap();
    }

    #[test]
    fn token_is_provider_scoped_json_not_config_toml() {
        let dir = tempdir().unwrap();
        save_in(dir.path(), "teams", &token("secret")).unwrap();
        assert!(dir.path().join("board-teams.json").exists());
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn is_expired_respects_expires_at() {
        let now = Utc.timestamp_opt(1_000, 0).unwrap();
        let expired = TokenSet {
            access_token: "a".into(),
            refresh_token: None,
            expires_at: Some(Utc.timestamp_opt(500, 0).unwrap()),
        };
        let valid = TokenSet {
            access_token: "a".into(),
            refresh_token: None,
            expires_at: Some(Utc.timestamp_opt(2_000, 0).unwrap()),
        };
        let no_expiry = TokenSet {
            access_token: "a".into(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(expired.is_expired(now));
        assert!(!valid.is_expired(now));
        assert!(!no_expiry.is_expired(now));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        save_in(dir.path(), "slack", &token("a")).unwrap();
        let mode = fs::metadata(token_file(dir.path(), "slack"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
