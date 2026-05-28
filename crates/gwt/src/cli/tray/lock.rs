//! SPEC #2920 FR-011 — Tray single-instance lock metadata.
//!
//! Phase 1 only captures the on-disk payload format so Phase 3 and Phase 4
//! can update the underlying `gui_single_instance` lock without further
//! schema churn. The real file IO (read / write / remove) lands in Phase 3
//! once the lock-kind rewrite is in place.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

/// On-disk format for the tray-resident process lock file. Stored at
/// `<gwt_home>/run/tray-<user_id>.lock` per SPEC #2920 architecture design.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrayLockFile {
    pub pid: u32,
    /// URL that the embedded server bound to, e.g.
    /// `http://127.0.0.1:54321/`. Empty before the server has finished
    /// initialising; updated via atomic rename once the bind is known.
    pub url: String,
    pub started_at: DateTime<Utc>,
    /// Cargo `version.workspace = true` propagated at build time.
    pub version: String,
}

/// Resolve the canonical tray lock path for the given gwt_home + user id.
pub fn lock_path(gwt_home: &Path, user_id: &str) -> PathBuf {
    gwt_home.join("run").join(format!("tray-{user_id}.lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_is_user_scoped_under_gwt_home_run() {
        let gwt_home = Path::new("/tmp/gwt-home");
        assert_eq!(
            lock_path(gwt_home, "alice"),
            PathBuf::from("/tmp/gwt-home/run/tray-alice.lock")
        );
    }

    #[test]
    fn tray_lock_file_serializes_round_trip() {
        let lock = TrayLockFile {
            pid: 12345,
            url: "http://127.0.0.1:54321/".to_string(),
            started_at: DateTime::parse_from_rfc3339("2026-05-28T07:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            version: "10.0.0".to_string(),
        };
        let json = serde_json::to_string(&lock).expect("serialize");
        let round: TrayLockFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round, lock);
    }
}
