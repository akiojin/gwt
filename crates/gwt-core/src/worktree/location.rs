//! Worktree location types (SPEC-a70a1ece)

use serde::{Deserialize, Serialize};

/// Worktree placement strategy (SPEC-a70a1ece)
///
/// Determines where worktrees are created relative to the repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeLocation {
    /// Traditional: place worktrees under `.worktrees/` subdirectory
    /// Default for backward compatibility
    #[default]
    Subdir,
    /// Bare-based: place worktrees as siblings to the bare repository
    Sibling,
}

impl WorktreeLocation {
    /// Get a human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            Self::Subdir => ".worktrees/ (traditional)",
            Self::Sibling => "sibling (bare-based)",
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

        let sibling = WorktreeLocation::Sibling;
        let json = serde_json::to_string(&sibling).unwrap();
        assert_eq!(json, "\"sibling\"");
    }
}
