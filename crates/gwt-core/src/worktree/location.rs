//! Worktree location types

use serde::{Deserialize, Serialize};

/// Worktree placement strategy
///
/// Determines where worktrees are created relative to the repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeLocation {
    /// Place worktrees under `.worktrees/` subdirectory
    #[default]
    Subdir,
}

impl WorktreeLocation {
    /// Get a human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            Self::Subdir => ".worktrees/ (default)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_subdir() {
        assert_eq!(WorktreeLocation::default(), WorktreeLocation::Subdir);
    }

    #[test]
    fn test_serde_roundtrip() {
        let subdir = WorktreeLocation::Subdir;
        let json = serde_json::to_string(&subdir).unwrap();
        assert_eq!(json, "\"subdir\"");
    }
}
