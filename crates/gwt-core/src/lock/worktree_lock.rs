//! Worktree file locking

use crate::error::{GwtError, Result};
use fs2::FileExt;
use std::fs::File;
use std::path::{Path, PathBuf};

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

        // Create parent directory if needed
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = File::create(&lock_path)?;

        match file.try_lock_exclusive() {
            Ok(()) => {
                self.lock_file = Some(file);
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Acquire the lock (blocking)
    pub fn lock(&mut self) -> Result<()> {
        let lock_path = self.lock_file_path();

        // Create parent directory if needed
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = File::create(&lock_path)?;
        file.lock_exclusive()
            .map_err(|_| GwtError::WorktreeLocked {
                path: self.worktree_path.clone(),
            })?;

        self.lock_file = Some(file);
        Ok(())
    }

    /// Release the lock
    pub fn unlock(&mut self) -> Result<()> {
        if let Some(file) = self.lock_file.take() {
            file.unlock()?;
        }
        Ok(())
    }

    /// Check if a worktree is locked
    pub fn is_locked(worktree_path: &Path) -> bool {
        let lock_path = worktree_path.join(LOCK_FILE_NAME);
        if !lock_path.exists() {
            return false;
        }

        match File::open(&lock_path) {
            Ok(file) => file.try_lock_exclusive().is_err(),
            Err(_) => false,
        }
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
