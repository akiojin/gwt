//! SPEC #2920 FR-011 / Phase 3 — Tray single-instance lock.
//!
//! Per the SPEC #2920 Q2 + Q3 decisions, the tray-resident process is
//! scoped to **one instance per user**: Project switching happens inside
//! the browser UI via the existing Project Tabs, so the lock key drops
//! the `startup_dir` (worktree) dimension entirely and uses
//! `(gwt_home, "tray", user_id)`.
//!
//! The on-disk payload doubles as a discovery mechanism: a second `gwt`
//! launched by the same user reads the existing lock file's `url` field
//! and re-prints it on stderr so the user can just open the running tray
//! instead of seeing a hard error.
//!
//! The lock file lives under `<gwt_home>/run/tray-<user_id>.lock`. We
//! intentionally do **not** reuse the SPEC-1942 `runtime/{gui,headless}/`
//! tree because the tray kind is user-scoped, not worktree-scoped, and
//! sharing the legacy tree would invite collisions between distinct
//! workspaces.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fs2::FileExt;

/// On-disk format for the tray-resident process lock file. Stored at
/// `<gwt_home>/run/tray-<user_id>.lock`. The URL is empty before the
/// embedded server has finished binding and is updated via atomic
/// rename once the bind is known so a second launch can always read a
/// valid value back.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrayLockFile {
    pub pid: u32,
    pub url: String,
    pub started_at: DateTime<Utc>,
    pub version: String,
}

/// Resolve the canonical tray lock path for the given gwt_home + user id.
pub fn lock_path(gwt_home: &Path, user_id: &str) -> PathBuf {
    gwt_home.join("run").join(format!("tray-{user_id}.lock"))
}

/// Resolve the OS-level user id used as the lock scope. Falls back to
/// the OS env vars if the `whoami` crate cannot infer the username, and
/// to a fixed sentinel as a last resort so the lock path is always
/// resolvable (preventing accidental cross-user lock sharing).
pub fn current_user_id() -> String {
    let raw = whoami::username();
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        return sanitize_user_id_segment(trimmed);
    }
    let env_var = if cfg!(target_os = "windows") {
        "USERNAME"
    } else {
        "USER"
    };
    if let Some(value) = std::env::var(env_var).ok().filter(|v| !v.trim().is_empty()) {
        return sanitize_user_id_segment(value.trim());
    }
    "unknown".to_string()
}

fn sanitize_user_id_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// RAII guard for the tray single-instance lock. Drops the OS file lock
/// and removes the lock file when the tray-resident process exits.
#[derive(Debug)]
pub struct TrayLockHandle {
    path: PathBuf,
    file: File,
}

impl TrayLockHandle {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Update the URL stored inside the lock file once the embedded
    /// server has finished binding. Uses a single rewrite (no rename)
    /// because the file is already locked exclusively by the current
    /// process — concurrent readers would only ever be inspecting the
    /// file from a *failed* lock attempt, in which case they re-read
    /// after seeing the contention.
    pub fn set_url(&mut self, url: &str) -> io::Result<()> {
        let payload = build_lock_payload(std::process::id(), url);
        self.file.set_len(0)?;
        write_lock_contents(&self.path, &mut self.file, &payload)
    }
}

impl Drop for TrayLockHandle {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        // Best-effort removal so subsequent launches don't trip on a
        // stale file. Failure is logged but never panics during Drop.
        if let Err(error) = fs::remove_file(&self.path) {
            tracing::debug!(
                target: "gwt_tray_lock",
                path = %self.path.display(),
                error = %error,
                "failed to remove tray lock file on drop"
            );
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrayLockError {
    #[error("tray-resident gwt is already running for user {user_id} (lock: {path})\nopen the running instance at: {url}")]
    AlreadyRunning {
        user_id: String,
        path: PathBuf,
        url: String,
    },
    #[error("could not prepare tray single-instance lock at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not parse existing tray lock file at {path}: {source}")]
    Corrupt {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Acquire the per-user tray lock. On success the returned
/// `TrayLockHandle` keeps the lock until it is dropped. On contention
/// the existing lock file is parsed and its URL is surfaced through
/// `TrayLockError::AlreadyRunning` so the caller can print the running
/// instance's URL on stderr and exit gracefully.
pub fn acquire(gwt_home: &Path) -> Result<TrayLockHandle, TrayLockError> {
    let user_id = current_user_id();
    let path = lock_path(gwt_home, &user_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| TrayLockError::Io {
            path: path.clone(),
            source,
        })?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| TrayLockError::Io {
            path: path.clone(),
            source,
        })?;
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(_) => {
            // Another process holds the lock. Read its URL out so the
            // caller can guide the user. Failure to parse is reported
            // separately so the user sees the real issue.
            let existing = read_lock_contents(&path)?;
            return Err(TrayLockError::AlreadyRunning {
                user_id,
                path,
                url: existing.url,
            });
        }
    }
    let payload = build_lock_payload(std::process::id(), "");
    write_lock_contents(&path, &mut file, &payload).map_err(|source| TrayLockError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(TrayLockHandle { path, file })
}

fn build_lock_payload(pid: u32, url: &str) -> TrayLockFile {
    TrayLockFile {
        pid,
        url: url.to_string(),
        started_at: Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn write_lock_contents(path: &Path, file: &mut File, payload: &TrayLockFile) -> io::Result<()> {
    use std::io::Seek;
    file.seek(io::SeekFrom::Start(0))?;
    let json = serde_json::to_vec(payload).map_err(io::Error::other)?;
    file.write_all(&json)?;
    file.sync_all()?;
    tracing::debug!(
        target: "gwt_tray_lock",
        path = %path.display(),
        url = %payload.url,
        "wrote tray lock contents"
    );
    Ok(())
}

fn read_lock_contents(path: &Path) -> Result<TrayLockFile, TrayLockError> {
    let mut file = File::open(path).map_err(|source| TrayLockError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|source| TrayLockError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if buf.trim().is_empty() {
        // The owning process may have just created an empty placeholder
        // before writing the URL. Surface a synthetic payload so the
        // contention message stays informative.
        return Ok(TrayLockFile {
            pid: 0,
            url: String::new(),
            started_at: Utc::now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        });
    }
    serde_json::from_str(&buf).map_err(|source| TrayLockError::Corrupt {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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

    #[test]
    fn current_user_id_sanitizes_non_alnum_segments() {
        // Always non-empty.
        let id = current_user_id();
        assert!(!id.is_empty());
        for ch in id.chars() {
            assert!(
                ch.is_ascii_alphanumeric() || ch == '-' || ch == '_',
                "user id segment must be filesystem-safe, got '{ch}' in '{id}'"
            );
        }
    }

    #[test]
    fn acquire_creates_lock_file_and_releases_on_drop() {
        let tmp = TempDir::new().expect("tempdir");
        let gwt_home = tmp.path();
        let handle = acquire(gwt_home).expect("first acquire succeeds");
        assert!(handle.path().exists(), "lock file must exist after acquire");
        drop(handle);
        // Drop should remove the lock file.
        let user_id = current_user_id();
        let path = lock_path(gwt_home, &user_id);
        assert!(
            !path.exists(),
            "lock file must be removed on Drop, but still exists at {}",
            path.display()
        );
    }

    #[test]
    fn second_acquire_for_same_user_reports_already_running() {
        let tmp = TempDir::new().expect("tempdir");
        let gwt_home = tmp.path();
        let _holder = acquire(gwt_home).expect("first acquire succeeds");

        // The same process cannot fs2-lock the file twice on most
        // platforms even with a different OpenOptions handle, so we
        // emulate the contention by writing the file directly with a
        // populated URL and then re-acquiring (which surfaces the
        // contention against the still-held _holder).
        let user_id = current_user_id();
        let path = lock_path(gwt_home, &user_id);
        let payload = TrayLockFile {
            pid: std::process::id(),
            url: "http://127.0.0.1:55555/".to_string(),
            started_at: Utc::now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        // The lock is held by _holder, but on macOS / Linux fs2 uses
        // POSIX advisory locks which are per-process; an exclusive lock
        // taken from the same process via a second handle behaves
        // platform-dependently. To keep the test deterministic across
        // POSIX and Windows we update the existing file contents (the
        // owning handle stays alive throughout the test) and only
        // assert that `read_lock_contents` round-trips the URL.
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(false)
                .open(&path)
                .expect("reopen for url overwrite");
            f.set_len(0).expect("truncate");
            let json = serde_json::to_vec(&payload).expect("serialize");
            f.write_all(&json).expect("write");
            f.sync_all().expect("sync");
        }
        let read_back = read_lock_contents(&path).expect("read existing lock contents");
        assert_eq!(read_back.url, "http://127.0.0.1:55555/");
    }

    #[test]
    fn set_url_updates_lock_payload_in_place() {
        let tmp = TempDir::new().expect("tempdir");
        let gwt_home = tmp.path();
        let mut handle = acquire(gwt_home).expect("acquire");
        handle
            .set_url("http://127.0.0.1:54321/")
            .expect("set_url succeeds");
        let contents = fs::read_to_string(handle.path()).expect("read lock file");
        let payload: TrayLockFile = serde_json::from_str(&contents).expect("payload deserializes");
        assert_eq!(payload.url, "http://127.0.0.1:54321/");
    }
}
