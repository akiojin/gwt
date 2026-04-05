//! Utility functions for gwt filesystem paths.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Return the gwt home directory (`~/.gwt/`).
pub fn gwt_home() -> PathBuf {
    dirs::home_dir()
        .expect("home directory must be resolvable")
        .join(".gwt")
}

/// Return the path to the global config file (`~/.gwt/config.toml`).
pub fn gwt_config_path() -> PathBuf {
    gwt_home().join("config.toml")
}

/// Return the sessions directory (`~/.gwt/sessions/`).
pub fn gwt_sessions_dir() -> PathBuf {
    gwt_home().join("sessions")
}

/// Return the cache directory (`~/.gwt/cache/`).
pub fn gwt_cache_dir() -> PathBuf {
    gwt_home().join("cache")
}

/// Return the logs directory (`~/.gwt/logs/`).
pub fn gwt_logs_dir() -> PathBuf {
    gwt_home().join("logs")
}

/// Ensure that the directory at `path` exists, creating it recursively if
/// necessary.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gwt_home_ends_with_dot_gwt() {
        let home = gwt_home();
        assert!(home.ends_with(".gwt"));
    }

    #[test]
    fn gwt_config_path_ends_with_config_toml() {
        let p = gwt_config_path();
        assert_eq!(p.file_name().unwrap(), "config.toml");
        assert!(p.starts_with(gwt_home()));
    }

    #[test]
    fn gwt_sessions_dir_is_under_home() {
        let p = gwt_sessions_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("sessions"));
    }

    #[test]
    fn gwt_cache_dir_is_under_home() {
        let p = gwt_cache_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("cache"));
    }

    #[test]
    fn gwt_logs_dir_is_under_home() {
        let p = gwt_logs_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("logs"));
    }

    #[test]
    fn ensure_dir_creates_missing_directory() {
        let tmp = std::env::temp_dir().join("gwt_test_ensure_dir");
        let _ = std::fs::remove_dir_all(&tmp);

        let target = tmp.join("a").join("b").join("c");
        assert!(!target.exists());
        ensure_dir(&target).unwrap();
        assert!(target.is_dir());

        // Calling again on existing dir is a no-op.
        ensure_dir(&target).unwrap();

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_dir_succeeds_for_existing_directory() {
        let tmp = std::env::temp_dir();
        ensure_dir(&tmp).unwrap();
    }
}
