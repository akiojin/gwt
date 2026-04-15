//! Worktree identification via canonicalized absolute path hashing.

use std::{fmt, path::Path};

use sha2::{Digest, Sha256};

use crate::error::{GwtError, Result};

const HASH_HEX_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorktreeHash(String);

impl WorktreeHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorktreeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Compute a `WorktreeHash` from an absolute Worktree directory path.
///
/// The path is canonicalized (symlinks resolved) before hashing, so two paths
/// pointing at the same on-disk directory always produce the same hash.
/// Returns an error if the path is relative.
pub fn compute_worktree_hash(path: &Path) -> Result<WorktreeHash> {
    if !path.is_absolute() {
        return Err(GwtError::Other(format!(
            "worktree path must be absolute: {}",
            path.display()
        )));
    }

    let canonical = dunce::canonicalize(path)
        .map_err(|e| GwtError::Other(format!("canonicalize {} failed: {}", path.display(), e)))?;

    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex_full = hex::encode(digest);
    Ok(WorktreeHash(hex_full[..HASH_HEX_LEN].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_rejected() {
        let result = compute_worktree_hash(Path::new("relative/path"));
        assert!(result.is_err());
    }

    #[test]
    fn deterministic_for_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let a = compute_worktree_hash(tmp.path()).unwrap();
        let b = compute_worktree_hash(tmp.path()).unwrap();
        assert_eq!(a.as_str(), b.as_str());
    }
}
