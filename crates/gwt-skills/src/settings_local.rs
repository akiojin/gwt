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

    merge_hooks_safe(&settings_path, managed_hooks).map_err(|e| {
        io::Error::other(format!("hooks merge failed: {e}"))
    })
}
