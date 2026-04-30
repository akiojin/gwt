//! Pure Git operations used by the SPEC-1934 US-6 migration: bare-ifying a
//! Normal Git repository, adding worktrees (clean / dirty), evacuating
//! uncommitted files for the dirty path, and restoring upstream + submodule
//! state after the move.
//!
//! Each function is intentionally narrow so tests in
//! `crates/gwt-git/tests/migration_test.rs` can target them in isolation.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_core::{process::hidden_command, GwtError, Result};

/// Clone a Normal repository's `origin` URL into `<target>` as a bare repo
/// (FR-021). The full history is preserved so subsequent worktree adds resolve
/// every branch the original repository knew.
pub fn clone_bare_from_normal(origin_url: &str, target: &Path) -> Result<PathBuf> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(GwtError::Io)?;
        }
    }
    let target_str = target
        .to_str()
        .ok_or_else(|| GwtError::Git(format!("invalid bare target path: {}", target.display())))?;

    let output = hidden_command("git")
        .args(["clone", "--bare", origin_url, target_str])
        .output()
        .map_err(|e| GwtError::Git(format!("git clone --bare: {e}")))?;

    if output.status.success() {
        Ok(target.to_path_buf())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(GwtError::Git(format!("git clone --bare failed: {stderr}")))
    }
}

/// Bare-ify a project's local `.git/` directory in place when no usable
/// `origin` URL is available. The contents of `.git/` are copied into
/// `<target>` and `core.bare` is flipped to true (FR-021 fallback path).
pub fn bareify_local(project_root: &Path, target: &Path) -> Result<PathBuf> {
    let dot_git = project_root.join(".git");
    if !dot_git.is_dir() {
        return Err(GwtError::Git(format!(
            "bareify_local: {} is not a normal Git repository (no .git/ directory)",
            project_root.display()
        )));
    }

    if target.exists() {
        return Err(GwtError::Git(format!(
            "bareify_local: target {} already exists",
            target.display()
        )));
    }

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(GwtError::Io)?;
        }
    }

    copy_dir_recursive(&dot_git, target).map_err(GwtError::Io)?;

    let target_str = target.to_str().ok_or_else(|| {
        GwtError::Git(format!(
            "bareify_local: invalid target path: {}",
            target.display()
        ))
    })?;

    let output = hidden_command("git")
        .args(["config", "--bool", "core.bare", "true"])
        .current_dir(target_str)
        .output()
        .map_err(|e| GwtError::Git(format!("git config core.bare: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!(
            "bareify_local: git config core.bare failed: {stderr}"
        )));
    }

    Ok(target.to_path_buf())
}

/// Copy the contents of `<source_dot_git>/hooks/` into `<bare>/hooks/`,
/// preserving file mode on Unix. `git clone --bare` does not bring user hooks
/// across, so the migration must do it explicitly (FR-022).
pub fn copy_hooks_to_bare(source_dot_git: &Path, bare: &Path) -> Result<()> {
    let src_hooks = source_dot_git.join("hooks");
    if !src_hooks.is_dir() {
        return Ok(());
    }
    let dst_hooks = bare.join("hooks");
    fs::create_dir_all(&dst_hooks).map_err(GwtError::Io)?;

    for entry in fs::read_dir(&src_hooks).map_err(GwtError::Io)? {
        let entry = entry.map_err(GwtError::Io)?;
        let from = entry.path();
        let to = dst_hooks.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from).map_err(GwtError::Io)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_file() {
            fs::copy(&from, &to).map_err(GwtError::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                let perms = std::fs::Permissions::from_mode(mode);
                fs::set_permissions(&to, perms).map_err(GwtError::Io)?;
            }
        }
    }
    Ok(())
}

/// Add a clean worktree at `<target>` for `<branch>` from the bare repo.
pub fn add_worktree_clean(_bare: &Path, _target: &Path, _branch: &str) -> Result<()> {
    Err(GwtError::Git(
        "migration::add_worktree_clean — not implemented (SPEC-1934 T-050)".to_string(),
    ))
}

/// Add a worktree without checkout, so callers can restore evacuated files
/// before running `git reset` (FR-023, T-052).
pub fn add_worktree_no_checkout(_bare: &Path, _target: &Path, _branch: &str) -> Result<()> {
    Err(GwtError::Git(
        "migration::add_worktree_no_checkout — not implemented (SPEC-1934 T-053)".to_string(),
    ))
}

/// Move all files except `.git/` and the migration backup to a temporary
/// evacuation directory; returns the evacuation root for later restore.
pub fn evacuate_dirty_files(_worktree: &Path, _evacuation_root: &Path) -> Result<PathBuf> {
    Err(GwtError::Git(
        "migration::evacuate_dirty_files — not implemented (SPEC-1934 T-053)".to_string(),
    ))
}

/// Restore previously-evacuated files into the new worktree.
pub fn restore_evacuated_files(_evacuation_root: &Path, _new_worktree: &Path) -> Result<()> {
    Err(GwtError::Git(
        "migration::restore_evacuated_files — not implemented (SPEC-1934 T-053)".to_string(),
    ))
}

/// Run `git submodule update --init --recursive` in the new worktree
/// (best effort; failure logs a warning).
pub fn init_submodules(_worktree: &Path) -> Result<()> {
    Err(GwtError::Git(
        "migration::init_submodules — not implemented (SPEC-1934 T-061)".to_string(),
    ))
}

/// Set upstream tracking for `<branch>` to `origin/<branch>`, if it exists.
pub fn set_upstream(_worktree: &Path, _branch: &str) -> Result<()> {
    Err(GwtError::Git(
        "migration::set_upstream — not implemented (SPEC-1934 T-063)".to_string(),
    ))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            // Symlinks inside `.git/` (e.g. submodule worktree pointers) are
            // intentionally skipped: bare layout does not preserve worktree
            // markers and re-creating links can foot-gun across hosts.
            continue;
        }
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
