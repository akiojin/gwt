//! Worktree file locking

use crate::error::{GwtError, Result};
use fs2::FileExt;
use std::fs::File;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Lock file name
const LOCK_FILE_NAME: &str = ".gwt.lock";

/// Worktree lock for multi-instance support
pub struct WorktreeLock {
    /// Worktree path
    worktree_path: PathBuf,
    /// Lock file handle
    lock_file: Option<File>,
}

impl WorktreeLock {
    /// Create a new worktree lock
    pub fn new(worktree_path: impl Into<PathBuf>) -> Self {
        Self {
            worktree_path: worktree_path.into(),
            lock_file: None,
        }
    }

    /// Get the lock file path
    pub fn lock_file_path(&self) -> PathBuf {
        self.worktree_path.join(LOCK_FILE_NAME)
    }

    /// Try to acquire the lock (non-blocking)
    pub fn try_lock(&mut self) -> Result<bool> {
        let lock_path = self.lock_file_path();

        debug!(
            category = "lock",
            worktree_path = %self.worktree_path.display(),
            lock_path = %lock_path.display(),
            "Attempting to acquire lock (non-blocking)"
        );

        // Create parent directory if needed
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = File::create(&lock_path)?;

        match file.try_lock_exclusive() {
            Ok(()) => {
                self.lock_file = Some(file);
                info!(
                    category = "lock",
                    operation = "try_lock",
                    worktree_path = %self.worktree_path.display(),
                    "Lock acquired successfully"
                );
                Ok(true)
            }
            Err(_) => {
                warn!(
                    category = "lock",
                    worktree_path = %self.worktree_path.display(),
                    "Lock already held by another process"
                );
                Ok(false)
            }
        }
    }

    /// Acquire the lock (blocking)
    pub fn lock(&mut self) -> Result<()> {
        let lock_path = self.lock_file_path();

        debug!(
            category = "lock",
            worktree_path = %self.worktree_path.display(),
            lock_path = %lock_path.display(),
            "Attempting to acquire lock (blocking)"
        );

        // Create parent directory if needed
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = File::create(&lock_path)?;
        file.lock_exclusive()
            .map_err(|_| {
                warn!(
                    category = "lock",
                    worktree_path = %self.worktree_path.display(),
                    "Failed to acquire lock (timeout or error)"
                );
                GwtError::WorktreeLocked {
                    path: self.worktree_path.clone(),
                }
            })?;

        self.lock_file = Some(file);
        info!(
            category = "lock",
            operation = "lock",
            worktree_path = %self.worktree_path.display(),
            "Lock acquired (blocking)"
        );
        Ok(())
    }

    /// Release the lock
    pub fn unlock(&mut self) -> Result<()> {
        debug!(
            category = "lock",
            worktree_path = %self.worktree_path.display(),
            "Releasing lock"
        );

        if let Some(file) = self.lock_file.take() {
            file.unlock()?;
            info!(
                category = "lock",
                operation = "unlock",
                worktree_path = %self.worktree_path.display(),
                "Lock released"
            );
        }
        Ok(())
    }

    /// Check if a worktree is locked
    pub fn is_locked(worktree_path: &Path) -> bool {
        let lock_path = worktree_path.join(LOCK_FILE_NAME);
        if !lock_path.exists() {
            debug!(
                category = "lock",
                worktree_path = %worktree_path.display(),
                is_locked = false,
                "Lock file does not exist"
            );
            return false;
        }

        let is_locked = match File::open(&lock_path) {
            Ok(file) => file.try_lock_exclusive().is_err(),
            Err(_) => false,
        };

        debug!(
            category = "lock",
            worktree_path = %worktree_path.display(),
            is_locked,
            "Checked lock status"
        );
        is_locked
    }
}

impl Drop for WorktreeLock {
    fn drop(&mut self) {
        let _ = self.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_unlock() {
        let temp = TempDir::new().unwrap();
        let mut lock = WorktreeLock::new(temp.path());

        assert!(lock.try_lock().unwrap());
        assert!(WorktreeLock::is_locked(temp.path()));

        lock.unlock().unwrap();
        // Note: After unlock, file may still exist but should be lockable
    }

    #[test]
    fn test_double_lock_fails() {
        let temp = TempDir::new().unwrap();
        let mut lock1 = WorktreeLock::new(temp.path());
        let mut lock2 = WorktreeLock::new(temp.path());

        assert!(lock1.try_lock().unwrap());
        assert!(!lock2.try_lock().unwrap()); // Should fail

        lock1.unlock().unwrap();
        assert!(lock2.try_lock().unwrap()); // Should succeed now
    }
}
