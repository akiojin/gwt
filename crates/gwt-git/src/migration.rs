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

/// File-system entries that must never be moved by the dirty-file
/// evacuation / restore pipeline. `.git` is the working tree's pointer (or the
/// bare repo itself) and the migration backup is the rollback safety net.
const EVACUATION_EXCLUSIONS: &[&str] = &[".git", ".gwt-migration-backup"];

/// Add a clean worktree at `<target>` for `<branch>` from the bare repo
/// (FR-024).
pub fn add_worktree_clean(bare: &Path, target: &Path, branch: &str) -> Result<()> {
    run_worktree_add(bare, target, branch, false)
}

/// Add a worktree without checkout, so callers can restore evacuated files
/// before running `git reset` (FR-023).
pub fn add_worktree_no_checkout(bare: &Path, target: &Path, branch: &str) -> Result<()> {
    run_worktree_add(bare, target, branch, true)
}

fn run_worktree_add(bare: &Path, target: &Path, branch: &str, no_checkout: bool) -> Result<()> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(GwtError::Io)?;
        }
    }
    let target_str = target.to_str().ok_or_else(|| {
        GwtError::Git(format!(
            "invalid worktree target path: {}",
            target.display()
        ))
    })?;

    let mut args: Vec<&str> = vec!["worktree", "add"];
    if no_checkout {
        args.push("--no-checkout");
    }
    args.push(target_str);
    args.push(branch);

    let output = hidden_command("git")
        .args(&args)
        .current_dir(bare)
        .output()
        .map_err(|e| GwtError::Git(format!("git worktree add: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GwtError::Git(format!(
            "git worktree add {} failed: {stderr}",
            target.display()
        )))
    }
}

/// Move every top-level entry under `worktree` to `evacuation_root` (creating
/// it as needed). `.git` and the migration backup directory are kept in
/// place. Returns the evacuation root so callers can pass it back into
/// [`restore_evacuated_files`].
pub fn evacuate_dirty_files(worktree: &Path, evacuation_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(evacuation_root).map_err(GwtError::Io)?;
    for entry in fs::read_dir(worktree).map_err(GwtError::Io)? {
        let entry = entry.map_err(GwtError::Io)?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if EVACUATION_EXCLUSIONS.iter().any(|e| **e == *name_str) {
            continue;
        }
        let from = entry.path();
        let to = evacuation_root.join(&name);
        fs::rename(&from, &to).map_err(GwtError::Io)?;
    }
    Ok(evacuation_root.to_path_buf())
}

/// Restore previously-evacuated files into `new_worktree`. Existing entries
/// in the destination (e.g. a `.git` marker created by
/// `git worktree add --no-checkout`) are preserved.
pub fn restore_evacuated_files(evacuation_root: &Path, new_worktree: &Path) -> Result<()> {
    fs::create_dir_all(new_worktree).map_err(GwtError::Io)?;
    for entry in fs::read_dir(evacuation_root).map_err(GwtError::Io)? {
        let entry = entry.map_err(GwtError::Io)?;
        let name = entry.file_name();
        let from = entry.path();
        let to = new_worktree.join(&name);
        if to.exists() {
            // Don't clobber Git's bookkeeping (`.git`, `.gitignore` written by
            // worktree add) — caller can decide whether to replace instead.
            continue;
        }
        fs::rename(&from, &to).map_err(GwtError::Io)?;
    }
    Ok(())
}

/// Run `git submodule update --init --recursive` in `worktree`. Per FR-025
/// the call is best-effort: a repo without submodules exits cleanly, and any
/// real submodule failure is propagated as an error so the executor can log
/// a warning without aborting the migration.
pub fn init_submodules(worktree: &Path) -> Result<()> {
    let output = hidden_command("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .current_dir(worktree)
        .output()
        .map_err(|e| GwtError::Git(format!("git submodule update: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(GwtError::Git(format!(
            "git submodule update failed: {stderr}"
        )))
    }
}

/// Set upstream tracking for `<branch>` to `origin/<branch>` in `worktree`.
/// FR-026 requires this to succeed silently when `origin/<branch>` is missing
/// (e.g. local-only branches), so we treat any non-zero git exit as a no-op.
pub fn set_upstream(worktree: &Path, branch: &str) -> Result<()> {
    let upstream = format!("origin/{branch}");
    let output = hidden_command("git")
        .args(["branch", "--set-upstream-to", &upstream, branch])
        .current_dir(worktree)
        .output()
        .map_err(|e| GwtError::Git(format!("git branch --set-upstream-to: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        // Missing upstream is expected for fresh / local-only branches.
        // Non-zero exits are absorbed here so the migration does not abort.
        Ok(())
    }
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
