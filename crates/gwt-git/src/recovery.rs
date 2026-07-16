//! Git-owned durability for crash-recoverable ephemeral Intake sessions.
//!
//! A RecoveryStore record lives outside Git, but its recorded launch commit
//! still needs a reachability root after a detached Intake worktree vanishes.
//! This module owns a private ref namespace and the narrowly-scoped worktree
//! recreation that consumes those refs.

use std::{
    fs,
    path::{Component, Path},
};

use gwt_core::{GwtError, Result};

use crate::worktree::{main_worktree_root, WorktreeManager};

const RECOVERY_BASE_REF_PREFIX: &str = "refs/gwt/recovery/";
const INTAKE_WORKTREE_PREFIX: &str = ".intake";

/// Return the gwt-owned ref that protects one recovery's launch base.
///
/// Recovery ids are validated instead of interpolated verbatim so callers
/// cannot escape the private namespace or smuggle a Git revision expression.
pub fn recovery_base_ref_name(recovery_id: &str) -> Result<String> {
    if recovery_id.is_empty()
        || recovery_id.len() > 128
        || !recovery_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(GwtError::Git(format!(
            "invalid recovery identity for Git base pin: {recovery_id:?}"
        )));
    }
    Ok(format!("{RECOVERY_BASE_REF_PREFIX}{recovery_id}/base"))
}

/// Create the recovery base ref without ever moving an existing ref.
///
/// A repeated request for the same commit is a no-op. A different OID under
/// the same recovery identity is a hard conflict, including concurrent races.
pub fn ensure_recovery_base_pin(
    repo_path: &Path,
    recovery_id: &str,
    recorded_oid: &str,
) -> Result<String> {
    let repo = main_worktree_root(repo_path)?;
    let reference = recovery_base_ref_name(recovery_id)?;
    let commit_oid = resolve_commit_oid(&repo, recorded_oid)?;
    match read_ref_oid(&repo, &reference)? {
        Some(existing) if existing == commit_oid => return Ok(reference),
        Some(existing) => {
            return Err(GwtError::Git(format!(
                "recovery base pin conflict for {recovery_id}: expected {commit_oid}, found {existing}"
            )));
        }
        None => {}
    }

    // Empty old-value means "the ref must not exist". This is the Git-side
    // compare-and-swap barrier for two launch/startup processes racing to pin.
    let output = gwt_core::process::run_git_logged(
        &["update-ref", &reference, &commit_oid, ""],
        Some(&repo),
    )
    .map_err(|error| GwtError::Git(format!("create recovery base pin: {error}")))?;
    if output.status.success() {
        return Ok(reference);
    }

    // A concurrent idempotent creator is success; every other failure remains
    // fail-closed and includes Git's diagnostic.
    if read_ref_oid(&repo, &reference)?.as_deref() == Some(commit_oid.as_str()) {
        return Ok(reference);
    }
    Err(GwtError::Git(format!(
        "create recovery base pin {reference}: {}",
        command_stderr(&output)
    )))
}

/// Verify that both the recorded OID and its immutable recovery ref resolve to
/// the same commit in the active project repository.
pub fn verify_recovery_base_pin(
    repo_path: &Path,
    recovery_id: &str,
    recorded_oid: &str,
) -> Result<String> {
    let repo = main_worktree_root(repo_path)?;
    let reference = recovery_base_ref_name(recovery_id)?;
    let commit_oid = resolve_commit_oid(&repo, recorded_oid)?;
    match read_ref_oid(&repo, &reference)? {
        Some(existing) if existing == commit_oid => Ok(reference),
        Some(existing) => Err(GwtError::Git(format!(
            "recovery base pin mismatch for {recovery_id}: expected {commit_oid}, found {existing}"
        ))),
        None => Err(GwtError::Git(format!(
            "recovery base pin is missing for {recovery_id}"
        ))),
    }
}

/// Delete an owned recovery ref only when it still points at the recorded OID.
///
/// Missing is idempotent. The expected-old argument prevents cleanup from
/// deleting a ref that was corrupted or reassigned after terminal metadata was
/// committed.
pub fn remove_recovery_base_pin(
    repo_path: &Path,
    recovery_id: &str,
    recorded_oid: &str,
) -> Result<()> {
    let repo = main_worktree_root(repo_path)?;
    let reference = recovery_base_ref_name(recovery_id)?;
    let Some(existing) = read_ref_oid(&repo, &reference)? else {
        return Ok(());
    };
    let expected = resolve_commit_oid(&repo, recorded_oid)?;
    if existing != expected {
        return Err(GwtError::Git(format!(
            "refusing to remove mismatched recovery base pin {reference}: expected {expected}, found {existing}"
        )));
    }
    let output = gwt_core::process::run_git_logged(
        &["update-ref", "-d", &reference, &expected],
        Some(&repo),
    )
    .map_err(|error| GwtError::Git(format!("remove recovery base pin: {error}")))?;
    if output.status.success() || read_ref_oid(&repo, &reference)?.is_none() {
        Ok(())
    } else {
        Err(GwtError::Git(format!(
            "remove recovery base pin {reference}: {}",
            command_stderr(&output)
        )))
    }
}

/// Read-only eligibility check used while rendering Recovery Center actions.
pub fn can_recreate_missing_intake_worktree(
    repo_path: &Path,
    target_path: &Path,
    recovery_id: &str,
    recorded_oid: &str,
) -> Result<()> {
    let repo = main_worktree_root(repo_path)?;
    validate_recovery_intake_target_path(&repo, target_path)?;
    verify_recovery_base_pin(&repo, recovery_id, recorded_oid)?;
    validate_target_contents(target_path)?;

    let manager = WorktreeManager::new(&repo);
    for entry in manager.list()? {
        if same_path(&entry.path, target_path) && !entry.prunable {
            return Err(GwtError::Git(format!(
                "recovery target is already owned by an active worktree: {}",
                target_path.display()
            )));
        }
    }
    Ok(())
}

/// Verify an already-present Intake worktree before reusing it for recovery.
///
/// This is the restart counterpart of [`recreate_missing_intake_worktree`]:
/// if gwt recreated the worktree and then crashed before claiming the
/// recovery, the next process may reuse it only after proving that the path is
/// still an active worktree of the expected repository at the pinned commit.
/// An unrelated repository or user-created directory at the recorded path is
/// a collision and is never adopted.
pub fn verify_recovery_intake_worktree(
    repo_path: &Path,
    target_path: &Path,
    recovery_id: &str,
    recorded_oid: &str,
) -> Result<()> {
    let repo = main_worktree_root(repo_path)?;
    validate_recovery_intake_target_path(&repo, target_path)?;
    verify_recovery_base_pin(&repo, recovery_id, recorded_oid)?;

    let metadata = fs::symlink_metadata(target_path).map_err(|error| {
        GwtError::Git(format!(
            "inspect existing recovery worktree {}: {error}",
            target_path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(GwtError::Git(format!(
            "existing recovery target is not an ordinary directory: {}",
            target_path.display()
        )));
    }

    let manager = WorktreeManager::new(&repo);
    if !manager
        .list()?
        .iter()
        .any(|entry| same_path(&entry.path, target_path) && !entry.prunable)
    {
        return Err(GwtError::Git(format!(
            "existing recovery target is not an active worktree of the expected repository: {}",
            target_path.display()
        )));
    }

    let target_repo = main_worktree_root(target_path)?;
    if !same_path(&target_repo, &repo) {
        return Err(GwtError::Git(format!(
            "existing Intake belongs to a different repository: {}",
            target_path.display()
        )));
    }
    let expected_oid = resolve_commit_oid(&repo, recorded_oid)?;
    let actual_oid = resolve_commit_oid(target_path, "HEAD")?;
    if actual_oid != expected_oid {
        return Err(GwtError::Git(format!(
            "existing Intake HEAD mismatch: expected {expected_oid}, found {actual_oid}"
        )));
    }
    Ok(())
}

/// Recreate one missing, explicitly-recorded ephemeral Intake worktree.
///
/// Only a direct `.intake` / `.intake-N` sibling of the active project repo is
/// accepted. Existing non-empty content, another live worktree, a missing or
/// mismatched pin, and post-create repo/OID mismatches all fail closed.
pub fn recreate_missing_intake_worktree(
    repo_path: &Path,
    target_path: &Path,
    recovery_id: &str,
    recorded_oid: &str,
) -> Result<()> {
    let repo = main_worktree_root(repo_path)?;
    if let Err(error) =
        can_recreate_missing_intake_worktree(&repo, target_path, recovery_id, recorded_oid)
    {
        // Two startup processes may race before either acquires the recovery
        // lease. If the other process already produced the exact pinned
        // worktree, adopting that verified result is idempotent; every other
        // collision preserves the original fail-closed error.
        if verify_recovery_intake_worktree(&repo, target_path, recovery_id, recorded_oid).is_ok() {
            return Ok(());
        }
        return Err(error);
    }
    let commit_oid = resolve_commit_oid(&repo, recorded_oid)?;
    let manager = WorktreeManager::new(&repo);

    if manager
        .list()?
        .iter()
        .any(|entry| same_path(&entry.path, target_path) && entry.prunable)
    {
        manager.prune()?;
        if manager
            .list()?
            .iter()
            .any(|entry| same_path(&entry.path, target_path))
        {
            return Err(GwtError::Git(format!(
                "stale recovery worktree registration could not be pruned: {}",
                target_path.display()
            )));
        }
    }

    if target_path.exists() {
        // `validate_target_contents` proved it is a real, empty directory (not
        // a symlink). Git requires the target itself to be absent.
        if let Err(error) = fs::remove_dir(target_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(GwtError::Git(format!(
                    "remove empty recovery target {}: {error}",
                    target_path.display()
                )));
            }
        }
    }
    if let Err(error) = manager.create_detached(&commit_oid, target_path) {
        if verify_recovery_intake_worktree(&repo, target_path, recovery_id, &commit_oid).is_ok() {
            return Ok(());
        }
        return Err(error);
    }
    verify_recovery_intake_worktree(&repo, target_path, recovery_id, &commit_oid)
}

/// Validate that `target_path` is exactly one gwt-generated Intake slot in the
/// active repository layout. This is path-only and does not create anything.
pub fn validate_recovery_intake_target_path(repo: &Path, target_path: &Path) -> Result<()> {
    let repo = main_worktree_root(repo)?;
    if !target_path.is_absolute()
        || target_path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(GwtError::Git(format!(
            "recovery Intake target must be an absolute normalized path: {}",
            target_path.display()
        )));
    }
    let name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| GwtError::Git("recovery Intake target has no UTF-8 filename".to_string()))?;
    let valid_name = name == INTAKE_WORKTREE_PREFIX
        || name
            .strip_prefix(&format!("{INTAKE_WORKTREE_PREFIX}-"))
            .and_then(|suffix| suffix.parse::<usize>().ok().map(|value| (suffix, value)))
            .is_some_and(|(suffix, value)| value >= 2 && value.to_string() == suffix);
    if !valid_name {
        return Err(GwtError::Git(format!(
            "recovery target is not a gwt ephemeral Intake path: {}",
            target_path.display()
        )));
    }

    let expected_parent = repo.parent().unwrap_or(&repo);
    let target_parent = target_path.parent().ok_or_else(|| {
        GwtError::Git("recovery Intake target has no parent directory".to_string())
    })?;
    if !same_path(expected_parent, target_parent) {
        return Err(GwtError::Git(format!(
            "recovery Intake target is outside the active project layout: {}",
            target_path.display()
        )));
    }
    Ok(())
}

fn validate_target_contents(target_path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(target_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(GwtError::Git(format!(
                "inspect recovery target {}: {error}",
                target_path.display()
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(GwtError::Git(format!(
            "recovery target is not an empty ordinary directory: {}",
            target_path.display()
        )));
    }
    let mut entries = fs::read_dir(target_path).map_err(|error| {
        GwtError::Git(format!(
            "inspect recovery target {}: {error}",
            target_path.display()
        ))
    })?;
    if entries.next().transpose()?.is_some() {
        return Err(GwtError::Git(format!(
            "recovery target path is not empty: {}",
            target_path.display()
        )));
    }
    Ok(())
}

fn resolve_commit_oid(repo: &Path, revision: &str) -> Result<String> {
    if revision != "HEAD"
        && (!matches!(revision.len(), 40 | 64)
            || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(GwtError::Git(format!(
            "invalid recorded recovery commit OID: {revision:?}"
        )));
    }
    let commit = format!("{revision}^{{commit}}");
    let output = gwt_core::process::run_git_logged(&["rev-parse", "--verify", &commit], Some(repo))
        .map_err(|error| GwtError::Git(format!("resolve recovery commit: {error}")))?;
    if !output.status.success() {
        return Err(GwtError::Git(format!(
            "resolve recovery commit {revision}: {}",
            command_stderr(&output)
        )));
    }
    let oid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GwtError::Git(format!(
            "Git returned an invalid recovery commit OID: {oid:?}"
        )));
    }
    Ok(oid)
}

fn read_ref_oid(repo: &Path, reference: &str) -> Result<Option<String>> {
    let output = gwt_core::process::run_git_logged(
        &["rev-parse", "--verify", "--quiet", reference],
        Some(repo),
    )
    .map_err(|error| GwtError::Git(format!("inspect recovery base pin: {error}")))?;
    match output.status.code() {
        Some(0) => {
            let oid = String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_ascii_lowercase();
            if matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                Ok(Some(oid))
            } else {
                Err(GwtError::Git(format!(
                    "Git returned an invalid OID for recovery base pin {reference}"
                )))
            }
        }
        Some(1) => Ok(None),
        _ => Err(GwtError::Git(format!(
            "inspect recovery base pin {reference}: {}",
            command_stderr(&output)
        ))),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left_normalized = gwt_core::paths::normalize_windows_child_process_path(left);
    let right_normalized = gwt_core::paths::normalize_windows_child_process_path(right);
    if left_normalized == right_normalized {
        return true;
    }
    #[cfg(windows)]
    if left_normalized
        .to_string_lossy()
        .eq_ignore_ascii_case(&right_normalized.to_string_lossy())
    {
        return true;
    }
    let (Ok(left), Ok(right)) = (
        fs::canonicalize(left_normalized),
        fs::canonicalize(right_normalized),
    ) else {
        return false;
    };
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn command_stderr(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        "git command failed without stderr".to_string()
    } else {
        stderr
    }
}
