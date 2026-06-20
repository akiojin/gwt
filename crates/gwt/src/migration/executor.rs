//! Drives the full Normal → Nested Bare+Worktree migration sequence
//! (SPEC-1934 US-6).
//!
//! Composes `gwt_core::migration::*` (pure logic) with `gwt_git::migration::*`
//! (Git side-effects). Failures roll back via `gwt_core::migration::rollback`
//! and propagate `MigrationError` with the originating phase.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use chrono::Utc;
use gwt_core::config::BareProjectConfig;
use gwt_core::migration::backup::{self, BackupSnapshot};
use gwt_core::migration::rollback;
use gwt_core::migration::types::{
    MigrationError, MigrationOptions, MigrationOutcome, MigrationPhase, RecoveryState,
    WorktreeMigration,
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
    let planned_worktrees = git_migration::list_worktrees(project_root).unwrap_or_default();
    let external_roots = external_worktree_roots(project_root, &planned_worktrees);
    let mut snapshot =
        backup::create_with_external_roots(project_root, &external_roots).map_err(|e| {
            MigrationError {
                phase: MigrationPhase::Backup,
                message: e.to_string(),
                recovery: RecoveryState::Untouched,
            }
        })?;

    // Capture the project's pre-normalize `remote.origin.fetch` before the
    // Bareify phase normalizes a `--single-branch` refspec to the wildcard form
    // (SPEC-1934 US-7 / FR-033, T-156). Recorded on the backup snapshot so a
    // later-phase failure can restore the original refspec on rollback.
    snapshot.pre_normalize_fetch_refspec = read_origin_fetch_refspec(&project_root.join(".git"));

    let outcome = run_post_backup(
        project_root,
        &options,
        &snapshot,
        &planned_worktrees,
        &mut progress,
    );

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
    planned_worktrees: &[WorktreeMigration],
    progress: &mut impl FnMut(MigrationPhase, u8),
) -> Result<MigrationOutcome, MigrationError> {
    progress(MigrationPhase::Bareify, 0);

    let bare_repo_name = derive_bare_repo_name(project_root);
    let bare_target = project_root.join(&bare_repo_name);
    let dot_git = project_root.join(".git");

    let origin_url = read_origin_url(&dot_git);

    let bare_repo_path = match origin_url.as_deref() {
        Some(url) => match git_migration::clone_bare_from_normal(url, &bare_target) {
            Ok(p) => {
                refresh_bare_refs_from_local(&dot_git, &p)?;
                p
            }
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

    // SPEC-1934 US-7 / FR-033: rewrite a `--single-branch` style fetch refspec
    // on the new bare repo to the canonical `+refs/heads/*:refs/remotes/origin/*`
    // and refresh remote-tracking refs. Config rewrite is strict (a failure
    // means the new bare repo cannot serve later Start Work). The follow-up
    // `git fetch origin --prune` is best-effort: a network or auth failure
    // here still leaves a correctly-configured repo whose next manual fetch
    // will succeed, so we swallow that specific failure to avoid forcing
    // rollback when the bare contents are otherwise complete.
    match git_migration::normalize_fetch_refspec(&bare_repo_path) {
        Ok(_) => {}
        Err(gwt_core::GwtError::Git(msg)) if msg.contains("fetch failed") => {
            tracing::warn!(
                bare = %bare_repo_path.display(),
                error = %msg,
                "normalize_fetch_refspec: post-rewrite fetch did not succeed; \
                 continuing because refspec was already normalized",
            );
        }
        Err(e) => {
            return Err(MigrationError {
                phase: MigrationPhase::Bareify,
                message: format!("normalize fetch refspec: {e}"),
                recovery: RecoveryState::Partial,
            });
        }
    }

    progress(MigrationPhase::Worktrees, 0);
    let mut worktrees = migration_worktrees(
        project_root,
        &dot_git,
        &bare_repo_path,
        options,
        planned_worktrees,
    )?;
    worktrees.sort_by_key(|worktree| !worktree.is_main_repo);
    remove_copied_worktree_metadata(&bare_repo_path).map_err(|e| MigrationError {
        phase: MigrationPhase::Worktrees,
        message: e.to_string(),
        recovery: RecoveryState::Partial,
    })?;

    let main_exclusions =
        main_worktree_top_level_exclusions(project_root, &bare_target, &worktrees);
    let mut migrated_worktrees = Vec::with_capacity(worktrees.len());
    for worktree in &worktrees {
        let target = worktree_target_path(project_root, &worktree.branch);
        let evacuation_root = snapshot
            .backup_dir
            .join("evacuation")
            .join(sanitize_branch_for_path(&worktree.branch));

        if let Err(e) = fs::create_dir_all(&evacuation_root) {
            return Err(MigrationError {
                phase: MigrationPhase::Worktrees,
                message: format!("create evacuation dir: {e}"),
                recovery: RecoveryState::Partial,
            });
        }

        if worktree.is_main_repo {
            move_top_level_into(project_root, &evacuation_root, &main_exclusions).map_err(|e| {
                MigrationError {
                    phase: MigrationPhase::Worktrees,
                    message: e.to_string(),
                    recovery: RecoveryState::Partial,
                }
            })?;
        } else {
            git_migration::evacuate_dirty_files(&worktree.path, &evacuation_root).map_err(|e| {
                MigrationError {
                    phase: MigrationPhase::Worktrees,
                    message: e.to_string(),
                    recovery: RecoveryState::Partial,
                }
            })?;
            if worktree.path.exists() {
                fs::remove_dir_all(&worktree.path).map_err(|e| MigrationError {
                    phase: MigrationPhase::Worktrees,
                    message: format!("remove old worktree {}: {e}", worktree.path.display()),
                    recovery: RecoveryState::Partial,
                })?;
            }
        }

        git_migration::add_worktree_no_checkout(&bare_repo_path, &target, &worktree.branch)
            .map_err(|e| MigrationError {
                phase: MigrationPhase::Worktrees,
                message: e.to_string(),
                recovery: RecoveryState::Partial,
            })?;

        git_migration::restore_evacuated_files(&evacuation_root, &target).map_err(|e| {
            MigrationError {
                phase: MigrationPhase::Worktrees,
                message: e.to_string(),
                recovery: RecoveryState::Partial,
            }
        })?;

        // Best-effort index refresh after restoring the evacuated tree.
        let _ = gwt_core::process::hidden_command("git")
            .args(["reset"])
            .current_dir(&target)
            .output();

        migrated_worktrees.push(target);
    }

    let _ = fs::remove_dir_all(snapshot.backup_dir.join("evacuation"));

    progress(MigrationPhase::Submodules, 0);
    for worktree in &migrated_worktrees {
        let _ = git_migration::init_submodules(worktree);
    }

    progress(MigrationPhase::Tracking, 0);
    for (migration, worktree) in worktrees.iter().zip(migrated_worktrees.iter()) {
        let _ = git_migration::set_upstream(worktree, &migration.branch);
    }

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
        branch_worktree_path: migrated_worktrees
            .first()
            .cloned()
            .unwrap_or_else(|| project_root.to_path_buf()),
        bare_repo_path,
        migrated_worktrees,
    })
}

fn external_worktree_roots(project_root: &Path, worktrees: &[WorktreeMigration]) -> Vec<PathBuf> {
    worktrees
        .iter()
        .filter(|worktree| !worktree.is_main_repo)
        .filter(|worktree| !path_is_inside(&worktree.path, project_root))
        .map(|worktree| worktree.path.clone())
        .collect()
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical_path == canonical_root || canonical_path.starts_with(&canonical_root)
}

fn migration_worktrees(
    project_root: &Path,
    dot_git: &Path,
    bare_repo_path: &Path,
    options: &MigrationOptions,
    planned_worktrees: &[WorktreeMigration],
) -> Result<Vec<WorktreeMigration>, MigrationError> {
    if !planned_worktrees.is_empty() {
        return Ok(planned_worktrees.to_vec());
    }

    match git_migration::list_worktrees(project_root) {
        Ok(worktrees) if !worktrees.is_empty() => Ok(worktrees),
        Ok(_) | Err(_) => {
            let branch = current_branch(dot_git, project_root)
                .or_else(|| current_branch(bare_repo_path, bare_repo_path))
                .or_else(|| options.branch_override.clone())
                .unwrap_or_else(|| "develop".to_string());
            Ok(vec![WorktreeMigration {
                path: project_root.to_path_buf(),
                branch,
                is_main_repo: true,
                is_dirty: false,
                is_locked: false,
            }])
        }
    }
}

fn remove_copied_worktree_metadata(bare_repo_path: &Path) -> std::io::Result<()> {
    let worktrees = bare_repo_path.join("worktrees");
    match fs::remove_dir_all(worktrees) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn worktree_target_path(project_root: &Path, branch: &str) -> PathBuf {
    branch
        .split('/')
        .filter(|segment| !segment.is_empty())
        .fold(project_root.to_path_buf(), |path, segment| {
            path.join(segment)
        })
}

fn sanitize_branch_for_path(branch: &str) -> String {
    branch
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn main_worktree_top_level_exclusions(
    project_root: &Path,
    bare_target: &Path,
    worktrees: &[WorktreeMigration],
) -> Vec<String> {
    let mut exclusions = Vec::new();
    if let Some(name) = bare_target.file_name().and_then(|name| name.to_str()) {
        exclusions.push(name.to_string());
    }

    let canonical_project_root =
        std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());

    for worktree in worktrees.iter().filter(|worktree| !worktree.is_main_repo) {
        let canonical_worktree =
            std::fs::canonicalize(&worktree.path).unwrap_or_else(|_| worktree.path.clone());
        let Ok(relative) = canonical_worktree.strip_prefix(&canonical_project_root) else {
            continue;
        };
        let Some(Component::Normal(first)) = relative.components().next() else {
            continue;
        };
        exclusions.push(first.to_string_lossy().to_string());
    }

    exclusions.sort();
    exclusions.dedup();
    exclusions
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

fn refresh_bare_refs_from_local(
    dot_git: &Path,
    bare_repo_path: &Path,
) -> Result<(), MigrationError> {
    let output = gwt_core::process::hidden_command("git")
        .args(["fetch"])
        .arg(dot_git)
        .args(["+refs/*:refs/*"])
        .current_dir(bare_repo_path)
        .output()
        .map_err(|e| MigrationError {
            phase: MigrationPhase::Bareify,
            message: format!("refresh bare refs from local git dir: {e}"),
            recovery: RecoveryState::Partial,
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(MigrationError {
            phase: MigrationPhase::Bareify,
            message: format!(
                "refresh bare refs from local git dir failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
            recovery: RecoveryState::Partial,
        })
    }
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

/// Read the project's `remote.origin.fetch` refspec from its original `.git`
/// directory before migration mutates anything. Returns `None` when there is
/// no `origin`, no `fetch` entry, or the value already matches the canonical
/// wildcard form so rollback has nothing to restore (SPEC-1934 FR-033, T-156).
fn read_origin_fetch_refspec(dot_git: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["config", "--get", "remote.origin.fetch"])
        .env("GIT_DIR", dot_git.to_str().unwrap_or_default())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let refspec = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if refspec.is_empty() || refspec == git_migration::ORIGIN_WILDCARD_FETCH_REFSPEC {
        None
    } else {
        Some(refspec)
    }
}

/// Move every top-level entry of `src` into `dst`, except `.git`, the
/// migration backup, and caller-provided top-level exclusions such as the bare
/// repo and linked worktree parent directories.
fn move_top_level_into(src: &Path, dst: &Path, extra_exclusions: &[String]) -> std::io::Result<()> {
    const ALWAYS_EXCLUDED: &[&str] = &[".git", ".gwt-migration-backup"];
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ALWAYS_EXCLUDED.iter().any(|e| **e == *name_str) {
            continue;
        }
        if extra_exclusions
            .iter()
            .any(|excluded| excluded == &name_str)
        {
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
