//! Drives the full Normal → Nested Bare+Worktree migration sequence
//! (SPEC-1934 US-6).
//!
//! Composes `gwt_core::migration::*` (pure logic) with `gwt_git::migration::*`
//! (Git side-effects). Failures roll back via `gwt_core::migration::rollback`
//! and propagate `MigrationError` with the originating phase.

use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use gwt_core::config::BareProjectConfig;
use gwt_core::migration::backup::{self, BackupSnapshot};
use gwt_core::migration::rollback;
use gwt_core::migration::types::{
    MigrationError, MigrationOptions, MigrationOutcome, MigrationPhase, RecoveryState,
};
use gwt_core::migration::validator;
use gwt_git::migration as git_migration;

/// Execute every migration phase end-to-end. Calls `progress(phase, 0..=100)`
/// at phase boundaries so the caller (WebSocket bridge) can stream updates.
pub fn execute_migration(
    project_root: &Path,
    options: MigrationOptions,
    mut progress: impl FnMut(MigrationPhase, u8),
) -> Result<MigrationOutcome, MigrationError> {
    progress(MigrationPhase::Validate, 0);
    validator::validate(project_root).map_err(|e| MigrationError {
        phase: MigrationPhase::Validate,
        message: e.to_string(),
        recovery: RecoveryState::Untouched,
    })?;

    progress(MigrationPhase::Backup, 0);
    let snapshot = backup::create(project_root).map_err(|e| MigrationError {
        phase: MigrationPhase::Backup,
        message: e.to_string(),
        recovery: RecoveryState::Untouched,
    })?;

    let outcome = run_post_backup(project_root, &options, &snapshot, &mut progress);

    match outcome {
        Ok(out) => {
            if !options.keep_backup_on_success {
                let _ = backup::discard(snapshot);
            }
            progress(MigrationPhase::Done, 100);
            Ok(out)
        }
        Err(err) => {
            // Attempt to roll back. The recovery state on the returned error
            // reflects whether the rollback succeeded.
            let recovered = rollback::rollback_migration(&snapshot).is_ok();
            let recovery = if recovered {
                RecoveryState::RolledBack
            } else {
                RecoveryState::Partial
            };
            Err(MigrationError {
                phase: err.phase,
                message: err.message,
                recovery,
            })
        }
    }
}

fn run_post_backup(
    project_root: &Path,
    options: &MigrationOptions,
    snapshot: &BackupSnapshot,
    progress: &mut impl FnMut(MigrationPhase, u8),
) -> Result<MigrationOutcome, MigrationError> {
    progress(MigrationPhase::Bareify, 0);

    let bare_repo_name = derive_bare_repo_name(project_root);
    let bare_target = project_root.join(&bare_repo_name);
    let dot_git = project_root.join(".git");

    let origin_url = read_origin_url(&dot_git);

    let bare_repo_path = match origin_url.as_deref() {
        Some(url) => match git_migration::clone_bare_from_normal(url, &bare_target) {
            Ok(p) => p,
            Err(_) => bareify_local_or_fail(project_root, &bare_target)?,
        },
        None => bareify_local_or_fail(project_root, &bare_target)?,
    };

    git_migration::copy_hooks_to_bare(&dot_git, &bare_repo_path).map_err(|e| MigrationError {
        phase: MigrationPhase::Bareify,
        message: e.to_string(),
        recovery: RecoveryState::Partial,
    })?;
    gwt_git::install_develop_protection(&bare_repo_path).map_err(|e| MigrationError {
        phase: MigrationPhase::Bareify,
        message: e.to_string(),
        recovery: RecoveryState::Partial,
    })?;

    progress(MigrationPhase::Worktrees, 0);
    let branch = current_branch(&dot_git, project_root)
        .or_else(|| current_branch(&bare_repo_path, &bare_repo_path))
        .or_else(|| options.branch_override.clone())
        .unwrap_or_else(|| "develop".to_string());
    let branch_worktree = project_root.join(&branch);

    // Step 1: evacuate everything except `.git`, the bare repo, and the
    // backup. The evacuation root lives inside the backup dir so it is
    // automatically excluded from the walk (`.gwt-migration-backup` is in
    // EVACUATION_EXCLUSIONS).
    let evacuation_root = snapshot.backup_dir.join("evacuation");
    if let Err(e) = fs::create_dir_all(&evacuation_root) {
        return Err(MigrationError {
            phase: MigrationPhase::Worktrees,
            message: format!("create evacuation dir: {e}"),
            recovery: RecoveryState::Partial,
        });
    }
    let bare_basename = bare_target
        .file_name()
        .map(|n| n.to_string_lossy().to_string());
    move_top_level_into(project_root, &evacuation_root, bare_basename.as_deref()).map_err(|e| {
        MigrationError {
            phase: MigrationPhase::Worktrees,
            message: e.to_string(),
            recovery: RecoveryState::Partial,
        }
    })?;

    // Step 2: create the new worktree, importing whatever the bare repo has at HEAD.
    git_migration::add_worktree_no_checkout(&bare_repo_path, &branch_worktree, &branch).map_err(
        |e| MigrationError {
            phase: MigrationPhase::Worktrees,
            message: e.to_string(),
            recovery: RecoveryState::Partial,
        },
    )?;

    // Step 3: bring evacuated files into the new worktree.
    git_migration::restore_evacuated_files(&evacuation_root, &branch_worktree).map_err(|e| {
        MigrationError {
            phase: MigrationPhase::Worktrees,
            message: e.to_string(),
            recovery: RecoveryState::Partial,
        }
    })?;

    // Step 4: re-sync the index so the working tree reports the same status as
    // before the migration. Best-effort: if the worktree had no checkout we
    // still want to clear `git status` of the synthetic deletions added by
    // `--no-checkout`.
    let _ = gwt_core::process::hidden_command("git")
        .args(["reset"])
        .current_dir(&branch_worktree)
        .output();

    // Cleanup the evacuation scratch dir.
    let _ = fs::remove_dir_all(&evacuation_root);

    progress(MigrationPhase::Submodules, 0);
    let _ = git_migration::init_submodules(&branch_worktree);

    progress(MigrationPhase::Tracking, 0);
    let _ = git_migration::set_upstream(&branch_worktree, &branch);

    progress(MigrationPhase::Cleanup, 0);
    if dot_git.exists() {
        fs::remove_dir_all(&dot_git).map_err(|e| MigrationError {
            phase: MigrationPhase::Cleanup,
            message: format!("remove old .git: {e}"),
            recovery: RecoveryState::Partial,
        })?;
    }

    let cfg = BareProjectConfig {
        bare_repo_name,
        remote_url: origin_url,
        created_at: Utc::now().to_rfc3339(),
        migrated_from: Some("normal".to_string()),
    };
    cfg.save(project_root).map_err(|e| MigrationError {
        phase: MigrationPhase::Cleanup,
        message: e.to_string(),
        recovery: RecoveryState::Partial,
    })?;

    Ok(MigrationOutcome {
        branch_worktree_path: branch_worktree,
        bare_repo_path,
        migrated_worktrees: vec![project_root.join(&branch)],
    })
}

fn bareify_local_or_fail(
    project_root: &Path,
    bare_target: &Path,
) -> Result<PathBuf, MigrationError> {
    git_migration::bareify_local(project_root, bare_target).map_err(|e| MigrationError {
        phase: MigrationPhase::Bareify,
        message: e.to_string(),
        recovery: RecoveryState::Partial,
    })
}

fn derive_bare_repo_name(project_root: &Path) -> String {
    let stem = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    format!("{stem}.git")
}

fn read_origin_url(dot_git: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["config", "--get", "remote.origin.url"])
        .env("GIT_DIR", dot_git.to_str().unwrap_or_default())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Move every entry of `src` into `dst`, except `.git`, the migration
/// backup, and an optional named directory (the bare repo we just created).
fn move_top_level_into(
    src: &Path,
    dst: &Path,
    extra_exclusion: Option<&str>,
) -> std::io::Result<()> {
    const ALWAYS_EXCLUDED: &[&str] = &[".git", ".gwt-migration-backup"];
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ALWAYS_EXCLUDED.iter().any(|e| **e == *name_str) {
            continue;
        }
        if extra_exclusion == Some(&name_str) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        fs::rename(&from, &to)?;
    }
    Ok(())
}

fn current_branch(git_dir: &Path, work_dir: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(work_dir)
        .env("GIT_DIR", git_dir.to_str().unwrap_or_default())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}
