use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime},
};

use fs2::FileExt;

/// Environment variable that lets a user force-bypass the GUI single-instance
/// lock when a previous gwt crash left a stale OS lock on Windows. Set to
/// `"1"` / `"true"` / `"yes"` (case-insensitive) to skip the lock acquisition
/// failure path and continue startup. Phase C6 escape hatch for Issue #1764
/// follow-ups where the lock file cannot be unlocked by normal means
/// (antivirus / filesystem cache pinning the handle alive after the owning
/// process exited).
const FORCE_NEW_INSTANCE_ENV: &str = "GWT_FORCE_NEW_INSTANCE";

/// Phase C6: if the lock file is older than this threshold AND `try_lock_*`
/// fails, the failure is logged with a recovery hint pointing at the lock
/// path. Long-lived gwt processes do not refresh the lock file mtime, so
/// "old file" alone is not enough to drop the lock; we only surface the hint.
const STALE_LOCK_HINT_AGE: Duration = Duration::from_secs(60 * 60);

#[derive(Debug)]
pub struct GuiInstanceLock {
    path: PathBuf,
    file: File,
}

#[derive(Debug, thiserror::Error)]
pub enum GuiInstanceLockError {
    #[error("gwt GUI is already running for worktree {project_root} (lock: {lock_path})")]
    AlreadyRunning {
        project_root: PathBuf,
        lock_path: PathBuf,
    },
    #[error("failed to prepare gwt GUI single-instance lock for {project_root}: {source}")]
    Io {
        project_root: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve gwt GUI single-instance scope for {project_root}: {reason}")]
    Scope {
        project_root: PathBuf,
        reason: String,
    },
}

impl Drop for GuiInstanceLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let mut registry = process_lock_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.remove(&self.path);
    }
}

pub fn gui_instance_lock_path(
    gwt_home: &Path,
    project_root: &Path,
) -> Result<PathBuf, GuiInstanceLockError> {
    let repo_hash = gwt_core::paths::project_scope_hash(project_root);
    let worktree_hash =
        gwt_core::worktree_hash::compute_worktree_hash(project_root).map_err(|error| {
            GuiInstanceLockError::Scope {
                project_root: project_root.to_path_buf(),
                reason: error.to_string(),
            }
        })?;
    Ok(gwt_home
        .join("projects")
        .join(repo_hash.as_str())
        .join("runtime")
        .join("gui")
        .join(format!("{}.lock", worktree_hash.as_str())))
}

pub fn acquire_gui_instance_lock(
    gwt_home: &Path,
    project_root: &Path,
) -> Result<GuiInstanceLock, GuiInstanceLockError> {
    let lock_path = gui_instance_lock_path(gwt_home, project_root)?;
    let parent = lock_path
        .parent()
        .ok_or_else(|| GuiInstanceLockError::Scope {
            project_root: project_root.to_path_buf(),
            reason: format!("lock path has no parent: {}", lock_path.display()),
        })?;
    fs::create_dir_all(parent).map_err(|source| GuiInstanceLockError::Io {
        project_root: project_root.to_path_buf(),
        source,
    })?;

    let override_active = force_new_instance_requested();
    {
        let mut registry = process_lock_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry.contains(&lock_path) {
            // Phase C6: the in-process registry guards against double-acquire
            // within the same gwt process. Honour the override here too so
            // the env var has uniform semantics: setting it always lets the
            // current process proceed, regardless of which layer reported
            // the collision.
            if override_active {
                tracing::warn!(
                    target: "gwt::startup::lock",
                    lock_path = %lock_path.display(),
                    "GWT_FORCE_NEW_INSTANCE override bypassed the in-process single-instance registry"
                );
            } else {
                return Err(GuiInstanceLockError::AlreadyRunning {
                    project_root: project_root.to_path_buf(),
                    lock_path,
                });
            }
        }
        registry.insert(lock_path.clone());
    }

    let file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(source) => {
            unregister_process_lock(&lock_path);
            return Err(GuiInstanceLockError::Io {
                project_root: project_root.to_path_buf(),
                source,
            });
        }
    };

    match file.try_lock_exclusive() {
        Ok(()) => Ok(GuiInstanceLock {
            path: lock_path,
            file,
        }),
        Err(_) => {
            // Phase C6 escape hatch: a previous gwt crash on Windows can
            // leave the OS-level file lock pinned even after the process
            // exits if security software is holding the file open. Setting
            // GWT_FORCE_NEW_INSTANCE=1 skips the lock check so the user can
            // recover without having to delete the lock file by hand.
            if force_new_instance_requested() {
                tracing::warn!(
                    target: "gwt::startup::lock",
                    lock_path = %lock_path.display(),
                    "GWT_FORCE_NEW_INSTANCE override bypassed the single-instance lock"
                );
                return Ok(GuiInstanceLock {
                    path: lock_path,
                    file,
                });
            }
            log_stale_lock_hint(&lock_path);
            unregister_process_lock(&lock_path);
            Err(GuiInstanceLockError::AlreadyRunning {
                project_root: project_root.to_path_buf(),
                lock_path,
            })
        }
    }
}

fn force_new_instance_requested() -> bool {
    match std::env::var(FORCE_NEW_INSTANCE_ENV) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

fn log_stale_lock_hint(lock_path: &Path) {
    let age = lock_path
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok());
    if let Some(age) = age {
        if age >= STALE_LOCK_HINT_AGE {
            tracing::warn!(
                target: "gwt::startup::lock",
                lock_path = %lock_path.display(),
                lock_age_secs = age.as_secs(),
                env_hint = FORCE_NEW_INSTANCE_ENV,
                "single-instance lock is older than the stale threshold; if no other gwt is running set GWT_FORCE_NEW_INSTANCE=1 to bypass"
            );
            return;
        }
    }
    tracing::info!(
        target: "gwt::startup::lock",
        lock_path = %lock_path.display(),
        env_hint = FORCE_NEW_INSTANCE_ENV,
        "single-instance lock is held; set GWT_FORCE_NEW_INSTANCE=1 only if you are certain no other gwt is running"
    );
}

fn unregister_process_lock(path: &Path) {
    let mut registry = process_lock_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.remove(path);
}

fn process_lock_registry() -> &'static Mutex<HashSet<PathBuf>> {
    static REGISTRY: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that mutate the `GWT_FORCE_NEW_INSTANCE` env var so
    /// they do not race when run with `cargo test --test-threads > 1`.
    static FORCE_ENV_GUARD: Mutex<()> = Mutex::new(());

    fn with_force_env<T, F: FnOnce() -> T>(value: Option<&str>, body: F) -> T {
        let _guard = FORCE_ENV_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = std::env::var(FORCE_NEW_INSTANCE_ENV).ok();
        match value {
            Some(value) => std::env::set_var(FORCE_NEW_INSTANCE_ENV, value),
            None => std::env::remove_var(FORCE_NEW_INSTANCE_ENV),
        }
        let result = body();
        match previous {
            Some(value) => std::env::set_var(FORCE_NEW_INSTANCE_ENV, value),
            None => std::env::remove_var(FORCE_NEW_INSTANCE_ENV),
        }
        result
    }

    #[test]
    fn force_new_instance_requested_accepts_common_truthy_values() {
        for value in ["1", "true", "TRUE", "Yes", "on"] {
            assert!(
                with_force_env(Some(value), force_new_instance_requested),
                "expected `{value}` to enable the override"
            );
        }
    }

    #[test]
    fn force_new_instance_requested_rejects_falsy_and_missing_values() {
        for value in ["", "0", "false", "no", "off", "garbage"] {
            assert!(
                !with_force_env(Some(value), force_new_instance_requested),
                "expected `{value}` to leave the override disabled"
            );
        }
        assert!(!with_force_env(None, force_new_instance_requested));
    }

    #[test]
    fn acquire_gui_instance_lock_force_override_bypasses_taken_lock() {
        // SPEC-2014 Phase C6: confirm that a fresh acquisition can succeed via
        // the GWT_FORCE_NEW_INSTANCE override even after another process
        // grabbed the same lock first.
        let temp = tempfile::tempdir().expect("tempdir");
        let gwt_home = temp.path().join("gwt-home");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).expect("project dir");

        let _primary =
            acquire_gui_instance_lock(&gwt_home, &project_root).expect("first acquire ok");

        with_force_env(Some("1"), || {
            let secondary = acquire_gui_instance_lock(&gwt_home, &project_root)
                .expect("override must let the second acquire succeed");
            drop(secondary);
        });
    }

    #[test]
    fn acquire_gui_instance_lock_without_override_errors_with_taken_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let gwt_home = temp.path().join("gwt-home");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).expect("project dir");

        let _primary =
            acquire_gui_instance_lock(&gwt_home, &project_root).expect("first acquire ok");

        let error = with_force_env(None, || {
            acquire_gui_instance_lock(&gwt_home, &project_root).expect_err("override absent")
        });
        assert!(matches!(error, GuiInstanceLockError::AlreadyRunning { .. }));
    }
}
