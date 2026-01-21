//! Lock guard for RAII-based locking

use super::WorktreeLock;
use crate::error::Result;
use std::path::Path;

/// RAII lock guard that automatically releases the lock when dropped
pub struct LockGuard {
    lock: WorktreeLock,
}

impl LockGuard {
    /// Acquire a lock and return a guard
    pub fn acquire(worktree_path: &Path) -> Result<Self> {
        let mut lock = WorktreeLock::new(worktree_path);
        lock.lock()?;
        Ok(Self { lock })
    }

    /// Try to acquire a lock (non-blocking)
    pub fn try_acquire(worktree_path: &Path) -> Result<Option<Self>> {
        let mut lock = WorktreeLock::new(worktree_path);
        if lock.try_lock()? {
            Ok(Some(Self { lock }))
        } else {
            Ok(None)
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_guard_raii() {
        let temp = TempDir::new().unwrap();

        {
            let _guard = LockGuard::acquire(temp.path()).unwrap();
            assert!(WorktreeLock::is_locked(temp.path()));
        }
        // Lock should be released after guard is dropped
    }

    #[test]
    fn test_try_acquire() {
        let temp = TempDir::new().unwrap();

        let guard1 = LockGuard::try_acquire(temp.path()).unwrap();
        assert!(guard1.is_some());

        let guard2 = LockGuard::try_acquire(temp.path()).unwrap();
        assert!(guard2.is_none()); // Should fail
    }
}
