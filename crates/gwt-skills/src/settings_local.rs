//! Generate `.claude/settings.local.json` with gwt-managed hooks.

use crate::hooks::{merge_hooks_safe, Hook};
use std::io;
use std::path::Path;

/// Generate `.claude/settings.local.json` in the target worktree.
///
/// Uses `merge_hooks_safe` to preserve user-defined hooks while updating
/// gwt-managed hooks.
pub fn generate_settings_local(worktree: &Path, managed_hooks: &[Hook]) -> io::Result<()> {
    let settings_path = worktree.join(".claude/settings.local.json");

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    merge_hooks_safe(&settings_path, managed_hooks)
        .map_err(|e| io::Error::other(format!("hooks merge failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HooksConfig;

    fn managed_hook() -> Hook {
        Hook {
            event: "PreToolUse".to_string(),
            command: "gwt-hook pre-tool".to_string(),
            comment_marker: Some("# gwt-managed: pre-tool".to_string()),
        }
    }

    fn user_hook() -> Hook {
        Hook {
            event: "PostToolUse".to_string(),
            command: "my-custom-hook".to_string(),
            comment_marker: None,
        }
    }

    #[test]
    fn creates_settings_local_with_managed_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path(), &[managed_hook()]).unwrap();

        let path = dir.path().join(".claude/settings.local.json");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        let cfg: HooksConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(cfg.managed_hooks.len(), 1);
        assert_eq!(cfg.managed_hooks[0].command, "gwt-hook pre-tool");
    }

    #[test]
    fn preserves_existing_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.local.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let initial = HooksConfig {
            managed_hooks: vec![],
            user_hooks: vec![user_hook()],
        };
        std::fs::write(&path, serde_json::to_string(&initial).unwrap()).unwrap();

        generate_settings_local(dir.path(), &[managed_hook()]).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let cfg: HooksConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(cfg.managed_hooks.len(), 1);
        assert_eq!(cfg.user_hooks.len(), 1);
        assert_eq!(cfg.user_hooks[0].command, "my-custom-hook");
    }

    #[test]
    fn creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.local.json");
        assert!(!path.exists());

        generate_settings_local(dir.path(), &[managed_hook()]).unwrap();

        assert!(path.exists());
    }
}
