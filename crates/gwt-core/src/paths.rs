//! Utility functions for gwt filesystem paths.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{
    error::Result,
    repo_hash::{compute_path_hash, detect_repo_hash, RepoHash},
};

/// Return the gwt home directory (`~/.gwt/`).
pub fn gwt_home() -> PathBuf {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(home) = crate::test_support::gwt_home_override() {
        return home.join(".gwt");
    }

    let home = std::env::var_os("HOME");
    let userprofile = std::env::var_os("USERPROFILE");
    if let Some(test_home) = cargo_test_home_override(home.as_deref(), userprofile.as_deref()) {
        return test_home.join(".gwt");
    }
    resolve_home_dir(home, userprofile, dirs::home_dir()).join(".gwt")
}

static CARGO_TEST_HOME_OVERRIDE: OnceLock<Option<PathBuf>> = OnceLock::new();

fn cargo_test_home_override(home: Option<&OsStr>, userprofile: Option<&OsStr>) -> Option<PathBuf> {
    if is_explicit_isolated_home(home) || is_explicit_isolated_home(userprofile) {
        return None;
    }
    CARGO_TEST_HOME_OVERRIDE
        .get_or_init(detect_cargo_test_home)
        .clone()
}

fn detect_cargo_test_home() -> Option<PathBuf> {
    cargo_test_home_for_exe(
        &std::env::current_exe().ok()?,
        &std::env::temp_dir(),
        std::process::id(),
    )
}

fn cargo_test_home_for_exe(exe: &Path, temp_dir: &Path, process_id: u32) -> Option<PathBuf> {
    if !is_cargo_test_binary(exe) {
        return None;
    }
    let binary_name = exe.file_name()?.to_string_lossy();
    Some(
        temp_dir
            .join("gwt-cargo-test-home")
            .join(sanitize_test_binary_name(&binary_name))
            .join(process_id.to_string()),
    )
}

fn is_cargo_test_binary(exe: &Path) -> bool {
    exe.parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "deps")
        && exe
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.contains('-'))
}

fn sanitize_test_binary_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn is_explicit_isolated_home(path: Option<&OsStr>) -> bool {
    let Some(path) = path
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    else {
        return false;
    };
    path.starts_with(std::env::temp_dir())
}

fn resolve_home_dir(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    fallback: Option<PathBuf>,
) -> PathBuf {
    non_empty_os(home)
        .or_else(|| non_empty_os(userprofile))
        .map(PathBuf::from)
        .or(fallback)
        .expect("home directory must be resolvable")
}

fn non_empty_os(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

/// Normalize host filesystem paths before passing them to child processes.
///
/// Windows APIs and PowerShell can surface provider-qualified or verbatim
/// paths such as `Microsoft.PowerShell.Core\FileSystem::\\?\C:\repo` or
/// `//?/C:/repo`.
/// Those forms are valid in some Windows APIs but confuse shells and agent
/// CLIs when used as cwd or `GWT_PROJECT_ROOT`. Non-prefixed paths are
/// returned unchanged.
pub fn normalize_windows_child_process_path(path: &Path) -> PathBuf {
    let Some(value) = path.to_str() else {
        return path.to_path_buf();
    };
    let normalized = normalize_windows_child_process_path_text(value);
    if normalized == value {
        path.to_path_buf()
    } else {
        PathBuf::from(normalized)
    }
}

/// Normalize a path string with the same rules as
/// [`normalize_windows_child_process_path`].
pub fn normalize_windows_child_process_path_text(value: &str) -> String {
    const POWERSHELL_FILE_SYSTEM_PROVIDER_PREFIX: &str = r"Microsoft.PowerShell.Core\FileSystem::";

    let value = value
        .strip_prefix(POWERSHELL_FILE_SYSTEM_PROVIDER_PREFIX)
        .unwrap_or(value);
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = value.strip_prefix("//?/UNC/") {
        return format!(r"\\{}", rest.replace('/', r"\"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    if let Some(rest) = value.strip_prefix("//?/") {
        return rest.to_string();
    }
    value.to_string()
}

/// Return the path to the global config file (`~/.gwt/config.toml`).
pub fn gwt_config_path() -> PathBuf {
    gwt_home().join("config.toml")
}

/// Return the sessions directory (`~/.gwt/sessions/`).
pub fn gwt_sessions_dir() -> PathBuf {
    gwt_home().join("sessions")
}

/// Return the cache directory (`~/.gwt/cache/`).
pub fn gwt_cache_dir() -> PathBuf {
    gwt_home().join("cache")
}

/// Return the project data root (`~/.gwt/projects/`).
pub fn gwt_projects_dir() -> PathBuf {
    gwt_home().join("projects")
}

/// Return the project data directory for a repository hash.
pub fn gwt_project_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_projects_dir().join(repo_hash.as_str())
}

/// Return the project scope hash for a repository path.
pub fn project_scope_hash(repo_path: &Path) -> RepoHash {
    detect_repo_hash(repo_path).unwrap_or_else(|| compute_path_hash(repo_path))
}

/// Return the project data directory for a repository path.
pub fn gwt_project_dir_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_project_dir(&repo_hash)
}

/// Return the Project State current projection path for a repository hash.
pub fn gwt_project_state_projection_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("project-state/current.json")
}

/// Return the Project State summary journal path for a repository hash.
pub fn gwt_project_state_journal_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("project-state/journal.jsonl")
}

/// Return the Project State Work hot projection path for a repository hash.
pub fn gwt_project_state_works_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("project-state/works.json")
}

/// Return the Project State Work event log path for a repository hash.
pub fn gwt_project_state_work_events_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("project-state/work-events.jsonl")
}

/// Return the Workspace current projection path for a repository hash.
///
/// This compatibility function now points at the Project State storage root.
pub fn gwt_workspace_projection_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_state_projection_path(repo_hash)
}

/// Return the Workspace summary journal path for a repository hash.
///
/// This compatibility function now points at the Project State storage root.
pub fn gwt_workspace_journal_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_state_journal_path(repo_hash)
}

/// Return the Workspace WorkItem hot projection path for a repository hash.
///
/// This compatibility function now points at `project-state/works.json`.
pub fn gwt_workspace_work_items_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_state_works_path(repo_hash)
}

/// Return the Workspace WorkItem event log path for a repository hash.
///
/// This compatibility function now points at `project-state/work-events.jsonl`.
pub fn gwt_workspace_work_events_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_state_work_events_path(repo_hash)
}

/// Return the Workspace current projection path for a repository path.
pub fn gwt_workspace_projection_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_workspace_projection_path(&repo_hash)
}

/// Return the Workspace summary journal path for a repository path.
pub fn gwt_workspace_journal_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_workspace_journal_path(&repo_hash)
}

/// Return the Workspace WorkItem hot projection path for a repository path.
pub fn gwt_workspace_work_items_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_workspace_work_items_path(&repo_hash)
}

/// Return the Workspace WorkItem event log path for a repository path.
pub fn gwt_workspace_work_events_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_workspace_work_events_path(&repo_hash)
}

/// Return the home-scoped close-event log path for a repository hash
/// (`~/.gwt/projects/<hash>/project-state/work-events-closed.jsonl`).
///
/// SPEC-2359 Phase W-15 (FR-384): close-kind work events (Pause / Done /
/// Discard) are home-persisted only — they are recorded after the PR merged,
/// so they can never ride the PR, and they must not enter the git-tracked
/// repo-local log. This log is deliberately separate from
/// `work-events.jsonl`: that file is the FR-358 migration source and would be
/// copied wholesale into the repo-local tracked file on first migration.
pub fn gwt_workspace_work_events_closed_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("project-state/work-events-closed.jsonl")
}

/// Return the home-scoped close-event log path for a repository path.
pub fn gwt_workspace_work_events_closed_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_workspace_work_events_closed_path(&repo_hash)
}

/// SPEC-2359 W-16 (FR-387): fingerprint cache for the cross-machine work
/// events intake (`source → blob oid / content sha256`). Pure optimization —
/// deleting it only costs re-reading sources (dedup is event-id based).
pub fn gwt_workspace_work_events_intake_state_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("project-state/work-events-intake.json")
}

/// Return the intake fingerprint cache path for a repository path.
pub fn gwt_workspace_work_events_intake_state_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_workspace_work_events_intake_state_path(&repo_hash)
}

/// Resolve the main worktree root (git common dir) for a repository or linked
/// worktree path.
///
/// NOTE: repo-local, git-tracked files (`.gwt/work/…`) must NOT use this. For a
/// linked worktree the common dir is the shared (often bare) git directory,
/// which has no checked-out working tree to track files in. Those callers use
/// [`resolve_current_worktree_root`] (`git rev-parse --show-toplevel`). This
/// function remains a utility for resolving the canonical main repository root.
///
/// `gwt-core` cannot depend on `gwt-git`, so this mirrors
/// `gwt_git::worktree::main_worktree_root`: it asks git for the absolute
/// `--git-common-dir`, strips a trailing `.git`, and falls back to a
/// first child bare repository for the workspace-home layout (a directory
/// that contains child bare repos but is not itself a git work tree).
///
/// When the path cannot be resolved through git (for example a plain
/// non-repository directory), the input path is returned unchanged so the
/// caller still gets a deterministic, non-failing repo-local location.
pub fn resolve_main_worktree_root(repo_path: &Path) -> PathBuf {
    let Ok(output) = crate::process::run_git_logged(
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        Some(repo_path),
    ) else {
        return first_child_bare_repository(repo_path).unwrap_or_else(|| repo_path.to_path_buf());
    };

    if !output.status.success() {
        // Workspace-home layout: the home directory itself is not a git work
        // tree, but it contains a child bare repository (e.g. `gwt.git`).
        return first_child_bare_repository(repo_path)
            .map(|bare| {
                let bare = std::fs::canonicalize(&bare).unwrap_or(bare);
                normalize_windows_child_process_path(&bare)
            })
            .unwrap_or_else(|| repo_path.to_path_buf());
    }

    let common_dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if common_dir.as_os_str().is_empty() {
        return repo_path.to_path_buf();
    }

    if common_dir.file_name().and_then(|name| name.to_str()) == Some(".git") {
        if let Some(repo_root) = common_dir.parent() {
            return normalize_windows_child_process_path(repo_root);
        }
    }

    normalize_windows_child_process_path(&common_dir)
}

/// Return the first child bare repository directory under `repo_path`, if any.
///
/// A bare repository is identified by the presence of `HEAD`, `objects`, and
/// `refs` entries. The lexicographically smallest match is chosen so the
/// resolution is deterministic across platforms.
fn first_child_bare_repository(repo_path: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(repo_path).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.join("HEAD").exists()
                && path.join("objects").exists()
                && path.join("refs").exists()
        })
        .min()
}

/// Resolve the current worktree's working-tree root via `git rev-parse
/// --show-toplevel`.
///
/// Unlike [`resolve_main_worktree_root`] — which returns the shared git common
/// dir (a bare repo such as `gwt.git` in the workspace-home layout) for a
/// linked worktree — this returns the checked-out working tree of the *current*
/// worktree, the only place a git-tracked file can actually live. Falls back to
/// `repo_path` when git cannot resolve a toplevel (non-repository directory).
pub fn resolve_current_worktree_root(repo_path: &Path) -> PathBuf {
    let Ok(output) = crate::process::run_git_logged(
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        Some(repo_path),
    ) else {
        return repo_path.to_path_buf();
    };
    if !output.status.success() {
        return repo_path.to_path_buf();
    }
    let toplevel = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if toplevel.as_os_str().is_empty() {
        return repo_path.to_path_buf();
    }
    normalize_windows_child_process_path(&toplevel)
}

/// Return the repo-local Work storage directory (`<worktree_root>/.gwt/work/`).
///
/// SPEC-2359 Phase W-12 Slice 5b (FR-353): the persistent Work core lives as a
/// git-tracked file in the *current* worktree's working tree. Each worktree
/// commits its own `.gwt/work/`; branch divergence is reconciled via the
/// `merge=union` gitattribute (FR-355), not via a shared filesystem path. The
/// shared git common dir (bare repo) is never used because it has no working
/// tree to track files in.
pub fn gwt_repo_local_work_dir(repo_root: &Path) -> PathBuf {
    resolve_current_worktree_root(repo_root)
        .join(".gwt")
        .join("work")
}

/// Return the repo-local Work event log path
/// (`<repo_root>/.gwt/work/events.jsonl`).
///
/// SPEC-2359 Phase W-12 Slice 5b (FR-353): the persistent core of a Work is
/// the append-only event log. It is moved out of the home (untracked)
/// project-state directory into a git-tracked, repo-local file so branch
/// divergence is reconciled via the `merge=union` gitattribute (FR-355).
/// Derived projection (`works.json`) and volatile runtime state
/// (`current.json` / `journal.jsonl`) stay in home and are not repo-local.
pub fn gwt_repo_local_work_events_path(repo_root: &Path) -> PathBuf {
    gwt_repo_local_work_dir(repo_root).join("events.jsonl")
}

/// Return the repo-local project memory path
/// (`<repo_root>/.gwt/work/memory.md`).
///
/// SPEC-2359 Phase W-12: project memory (post-mortem entries written by
/// `memory.add`) is a git-tracked, repo-local file living under the
/// shared `.gwt/work/` directory alongside the Work event log. The main
/// worktree root is resolved first so every linked worktree of the same
/// repository shares one canonical file.
pub fn gwt_repo_local_memory_path(repo_root: &Path) -> PathBuf {
    gwt_repo_local_work_dir(repo_root).join("memory.md")
}

/// Return the repo-local discussion log path
/// (`<repo_root>/.gwt/work/discussions.md`).
///
/// SPEC-2359 Phase W-12: the discussion log written by `gwtd discussion
/// update` is a git-tracked, repo-local file under the shared `.gwt/work/`
/// directory. The main worktree root is resolved first so linked worktrees
/// share one canonical file.
pub fn gwt_repo_local_discussions_path(repo_root: &Path) -> PathBuf {
    gwt_repo_local_work_dir(repo_root).join("discussions.md")
}

/// Return the machine-local work-notes directory for a repository
/// (`~/.gwt/projects/<repo-hash>/work-notes/`).
///
/// SPEC-3214 (FR-007): project memory and discussion notes are machine-local
/// scratch, stored branch-independently in the home project dir (the same
/// placement convention as Board / Work state) and shared by every worktree
/// of the repository. Durable team-shared outcomes belong to GitHub
/// (Issue / `gwt-spec` Issue), not to these files.
pub fn gwt_work_notes_dir(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("work-notes")
}

/// Return the machine-local project memory path
/// (`~/.gwt/projects/<repo-hash>/work-notes/memory.md`). See
/// [`gwt_work_notes_dir`].
pub fn gwt_work_notes_memory_path(repo_path: &Path) -> PathBuf {
    gwt_work_notes_dir(repo_path).join("memory.md")
}

/// Return the machine-local discussion log path
/// (`~/.gwt/projects/<repo-hash>/work-notes/discussions.md`). See
/// [`gwt_work_notes_dir`].
pub fn gwt_work_notes_discussions_path(repo_path: &Path) -> PathBuf {
    gwt_work_notes_dir(repo_path).join("discussions.md")
}

/// Resolve the project memory path for READS: the machine-local home file
/// when present, otherwise the legacy git-tracked repo-local file
/// (`<repo_root>/.gwt/work/memory.md`) as a fallback, otherwise the (not yet
/// created) home path. Writers always target the home path.
pub fn resolve_work_notes_memory_read_path(repo_path: &Path) -> PathBuf {
    resolve_work_notes_read_path(
        gwt_work_notes_memory_path(repo_path),
        gwt_repo_local_memory_path(repo_path),
    )
}

/// Resolve the discussion log path for READS with the same home-first /
/// repo-local-fallback order as [`resolve_work_notes_memory_read_path`].
pub fn resolve_work_notes_discussions_read_path(repo_path: &Path) -> PathBuf {
    resolve_work_notes_read_path(
        gwt_work_notes_discussions_path(repo_path),
        gwt_repo_local_discussions_path(repo_path),
    )
}

fn resolve_work_notes_read_path(home_path: PathBuf, repo_local_path: PathBuf) -> PathBuf {
    if home_path.exists() {
        return home_path;
    }
    if repo_local_path.exists() {
        return repo_local_path;
    }
    home_path
}

/// Return the repo-local remote-Board root-thread mapping path
/// (`<repo_root>/.gwt/work/board-remote-roots.jsonl`).
///
/// SPEC-2963: Slack/Teams thread each Workspace under a "root" summary-card
/// message. The mapping `(provider, channel, key) -> root message id` is
/// git-tracked (like `events.jsonl`) so the root is created once and shared
/// across machines/agents, reconciled on branch divergence by a `merge=union`
/// gitattribute.
pub fn gwt_board_remote_roots_path(repo_root: &Path) -> PathBuf {
    gwt_repo_local_work_dir(repo_root).join("board-remote-roots.jsonl")
}

/// Return the repo-scoped notes root (`~/.gwt/notes/`).
pub fn gwt_notes_dir() -> PathBuf {
    gwt_home().join("notes")
}

/// Return the notes directory for a repository hash.
pub fn gwt_repo_notes_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_notes_dir().join(repo_hash.as_str())
}

/// Return the notes state path for a repository hash.
pub fn gwt_notes_state_path(repo_hash: &RepoHash) -> PathBuf {
    gwt_repo_notes_dir(repo_hash).join("notes.json")
}

/// Return the notes state path for a repository path.
pub fn gwt_notes_state_path_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_notes_state_path(&repo_hash)
}

/// Return the global session state path (`~/.gwt/session.json`).
pub fn gwt_session_state_path() -> PathBuf {
    gwt_home().join("session.json")
}

/// Return the legacy logs root (`~/.gwt/logs/`).
pub fn gwt_logs_dir() -> PathBuf {
    gwt_home().join("logs")
}

/// Return the legacy coordination root (`~/.gwt/coordination/`).
pub fn gwt_coordination_root() -> PathBuf {
    gwt_home().join("coordination")
}

/// Return the coordination directory for a repository hash.
pub fn gwt_coordination_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("coordination")
}

/// Return the coordination directory for a repository path, if `origin` exists.
pub fn gwt_coordination_dir_for_repo_path(repo_path: &Path) -> Option<PathBuf> {
    detect_repo_hash(repo_path).map(|repo_hash| gwt_coordination_dir(&repo_hash))
}

/// Return the structured-log directory for a repository hash.
pub fn gwt_project_logs_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("logs")
}

/// Return the structured-log directory for a repository path, if `origin` exists.
pub fn gwt_project_logs_dir_for_repo_path(repo_path: &Path) -> Option<PathBuf> {
    detect_repo_hash(repo_path).map(|repo_hash| gwt_project_logs_dir(&repo_hash))
}

/// Return the canonical structured-log directory for a project path.
///
/// Git repositories are scoped by normalized `origin` URL. Non-repository
/// paths fall back to a stable path hash, matching project-scoped workspace
/// storage.
pub fn gwt_project_logs_dir_for_project_path(project_path: &Path) -> PathBuf {
    let project_hash = project_scope_hash(project_path);
    gwt_project_logs_dir(&project_hash)
}

/// Return the update check cache path (`~/.gwt/cache/update-check.json`).
pub fn gwt_update_cache_path() -> PathBuf {
    gwt_cache_dir().join("update-check.json")
}

/// Return the updates staging directory (`~/.gwt/updates/`).
pub fn gwt_updates_dir() -> PathBuf {
    gwt_home().join("updates")
}

/// Return the shared runtime directory (`~/.gwt/runtime/`).
pub fn gwt_runtime_dir() -> PathBuf {
    gwt_runtime_dir_from(&gwt_home())
}

/// Return the project index runner path under the shared runtime directory.
pub fn gwt_runtime_runner_path() -> PathBuf {
    gwt_runtime_runner_path_from(&gwt_home())
}

/// Return the managed project-index virtualenv directory.
pub fn gwt_project_index_venv_dir() -> PathBuf {
    gwt_project_index_venv_dir_from(&gwt_home())
}

pub(crate) fn gwt_runtime_dir_from(gwt_home: &Path) -> PathBuf {
    gwt_home.join("runtime")
}

pub(crate) fn gwt_runtime_runner_path_from(gwt_home: &Path) -> PathBuf {
    gwt_runtime_dir_from(gwt_home).join("chroma_index_runner.py")
}

pub(crate) fn gwt_project_index_venv_dir_from(gwt_home: &Path) -> PathBuf {
    gwt_runtime_dir_from(gwt_home).join("chroma-venv")
}

/// Ensure that the directory at `path` exists, creating it recursively if
/// necessary.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{repo_hash::compute_repo_hash, test_support::env_lock};

    fn gwt_home_suffix(parts: &[&str]) -> PathBuf {
        let mut path = PathBuf::from(".gwt");
        for part in parts {
            path.push(part);
        }
        path
    }

    #[test]
    fn gwt_home_ends_with_dot_gwt() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = gwt_home();
        assert!(home.ends_with(".gwt"));
    }

    #[test]
    fn gwt_home_prefers_home_env_over_dirs_home() {
        let tmp = tempfile::tempdir().unwrap();
        let override_home = tmp.path().join("custom-home");

        let home = resolve_home_dir(
            Some(override_home.clone().into_os_string()),
            Some(tmp.path().join("ignored-userprofile").into_os_string()),
            None,
        )
        .join(".gwt");

        assert_eq!(home, override_home.join(".gwt"));
    }

    #[test]
    fn gwt_home_falls_back_to_userprofile_when_home_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let override_profile = tmp.path().join("custom-userprofile");

        let home = resolve_home_dir(None, Some(override_profile.clone().into_os_string()), None)
            .join(".gwt");

        assert_eq!(home, override_profile.join(".gwt"));
    }

    #[test]
    fn gwt_home_treats_empty_env_values_as_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let override_profile = tmp.path().join("custom-userprofile");
        let fallback = tmp.path().join("fallback-home");

        let userprofile_home = resolve_home_dir(
            Some(OsString::from("")),
            Some(override_profile.clone().into_os_string()),
            Some(fallback.clone()),
        );
        let fallback_home = resolve_home_dir(
            Some(OsString::from("")),
            Some(OsString::from("")),
            Some(fallback.clone()),
        );

        assert_eq!(userprofile_home, override_profile);
        assert_eq!(fallback_home, fallback);
    }

    #[test]
    fn cargo_test_home_for_exe_redirects_test_binaries_to_temp_home() {
        let temp = tempfile::tempdir().unwrap();
        let exe = Path::new("/repo/target/debug/deps/gwt-abc123");

        let home = cargo_test_home_for_exe(exe, temp.path(), 42).expect("test home");

        assert_eq!(home, temp.path().join("gwt-cargo-test-home/gwt-abc123/42"));
    }

    #[test]
    fn cargo_test_home_for_exe_ignores_non_test_binaries() {
        let temp = tempfile::tempdir().unwrap();

        assert!(
            cargo_test_home_for_exe(Path::new("/repo/target/debug/gwtd"), temp.path(), 42)
                .is_none()
        );
    }

    #[test]
    fn cargo_test_home_override_preserves_explicit_temp_home() {
        let temp = tempfile::tempdir().unwrap();

        assert!(cargo_test_home_override(Some(temp.path().as_os_str()), None).is_none());
    }

    #[test]
    fn normalize_windows_child_process_path_strips_drive_verbatim_prefix() {
        assert_eq!(
            normalize_windows_child_process_path(Path::new(r"\\?\E:\gwt\work\20260525-0919")),
            PathBuf::from(r"E:\gwt\work\20260525-0919")
        );
    }

    #[test]
    fn normalize_windows_child_process_path_strips_slash_drive_verbatim_prefix() {
        assert_eq!(
            normalize_windows_child_process_path_text("//?/E:/gwt/work/20260525-0919"),
            "E:/gwt/work/20260525-0919"
        );
    }

    #[test]
    fn normalize_windows_child_process_path_strips_unc_verbatim_prefix() {
        assert_eq!(
            normalize_windows_child_process_path(Path::new(r"\\?\UNC\server\share\work")),
            PathBuf::from(r"\\server\share\work")
        );
    }

    #[test]
    fn normalize_windows_child_process_path_strips_slash_unc_verbatim_prefix() {
        assert_eq!(
            normalize_windows_child_process_path_text("//?/UNC/server/share/work"),
            r"\\server\share\work"
        );
    }

    #[test]
    fn normalize_windows_child_process_path_strips_powershell_provider_prefix() {
        assert_eq!(
            normalize_windows_child_process_path(Path::new(
                r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\20260525-0919"
            )),
            PathBuf::from(r"E:\gwt\work\20260525-0919")
        );
    }

    #[test]
    fn normalize_windows_child_process_path_strips_provider_slash_verbatim_prefix() {
        assert_eq!(
            normalize_windows_child_process_path_text(
                r"Microsoft.PowerShell.Core\FileSystem:://?/E:/gwt/work/20260525-0919"
            ),
            "E:/gwt/work/20260525-0919"
        );
    }

    #[test]
    fn normalize_windows_child_process_path_preserves_regular_paths() {
        assert_eq!(
            normalize_windows_child_process_path(Path::new("/tmp/gwt/work")),
            PathBuf::from("/tmp/gwt/work")
        );
        assert_eq!(
            normalize_windows_child_process_path(Path::new(r"E:\gwt\work")),
            PathBuf::from(r"E:\gwt\work")
        );
    }

    #[cfg(unix)]
    #[test]
    fn normalize_windows_child_process_path_preserves_non_utf8_unix_paths() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let bytes = b"/tmp/gwt-\xFF-work";
        let path = Path::new(OsStr::from_bytes(bytes));
        let normalized = normalize_windows_child_process_path(path);

        assert_eq!(normalized.as_os_str().as_bytes(), bytes);
    }

    #[test]
    fn gwt_config_path_ends_with_config_toml() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_config_path();
        assert_eq!(p.file_name().unwrap(), "config.toml");
        assert!(p.ends_with(gwt_home_suffix(&["config.toml"])));
    }

    #[test]
    fn gwt_sessions_dir_is_under_home() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_sessions_dir();
        assert!(p.ends_with(gwt_home_suffix(&["sessions"])));
    }

    #[test]
    fn gwt_cache_dir_is_under_home() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_cache_dir();
        assert!(p.ends_with(gwt_home_suffix(&["cache"])));
    }

    #[test]
    fn gwt_projects_dir_is_under_home() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_projects_dir();
        assert!(p.ends_with(gwt_home_suffix(&["projects"])));
    }

    #[test]
    fn gwt_project_dir_scopes_by_repo_hash() {
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");
        let p = gwt_project_dir(&repo_hash);
        assert!(p.ends_with(gwt_home_suffix(&["projects", repo_hash.as_str()])));
    }

    #[test]
    fn gwt_session_state_path_is_under_home() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_session_state_path();
        assert!(p.ends_with(gwt_home_suffix(&["session.json"])));
    }

    #[test]
    fn project_scope_hash_falls_back_for_non_repo_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let hash = project_scope_hash(tmp.path());
        assert_eq!(hash.as_str().len(), 16);
        assert!(hash
            .as_str()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn gwt_logs_dir_is_under_home() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_logs_dir();
        assert!(p.ends_with(gwt_home_suffix(&["logs"])));
    }

    #[test]
    fn gwt_project_logs_dir_scopes_by_repo_hash() {
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");
        let p = gwt_project_logs_dir(&repo_hash);
        assert!(p.ends_with(gwt_home_suffix(&["projects", repo_hash.as_str(), "logs"])));
    }

    #[test]
    fn gwt_project_logs_dir_for_project_path_uses_origin_hash() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);
        add_origin(&repo, "https://github.com/example/project.git");

        let p = gwt_project_logs_dir_for_project_path(&repo);
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");

        assert!(p.ends_with(gwt_home_suffix(&["projects", repo_hash.as_str(), "logs"])));
    }

    #[test]
    fn gwt_project_logs_dir_for_project_path_uses_path_hash_without_origin() {
        let dir = tempfile::tempdir().unwrap();

        let p = gwt_project_logs_dir_for_project_path(dir.path());
        let path_hash = compute_path_hash(dir.path());

        assert!(p.ends_with(gwt_home_suffix(&["projects", path_hash.as_str(), "logs"])));
    }

    #[test]
    fn gwt_coordination_dir_scopes_by_repo_hash() {
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");
        let p = gwt_coordination_dir(&repo_hash);
        assert!(p.ends_with(gwt_home_suffix(&[
            "projects",
            repo_hash.as_str(),
            "coordination",
        ])));
    }

    #[test]
    fn gwt_runtime_dir_is_under_home() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_runtime_dir();
        assert!(p.ends_with(gwt_home_suffix(&["runtime"])));
    }

    #[test]
    fn gwt_runtime_runner_path_is_under_runtime_dir() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_runtime_runner_path();
        assert!(p.ends_with(gwt_home_suffix(&["runtime", "chroma_index_runner.py"])));
    }

    #[test]
    fn gwt_project_index_venv_dir_is_under_runtime_dir() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = gwt_project_index_venv_dir();
        assert!(p.ends_with(gwt_home_suffix(&["runtime", "chroma-venv"])));
    }

    #[test]
    fn ensure_dir_creates_missing_directory() {
        let tmp = std::env::temp_dir().join("gwt_test_ensure_dir");
        let _ = std::fs::remove_dir_all(&tmp);

        let target = tmp.join("a").join("b").join("c");
        assert!(!target.exists());
        ensure_dir(&target).unwrap();
        assert!(target.is_dir());

        // Calling again on existing dir is a no-op.
        ensure_dir(&target).unwrap();

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_dir_succeeds_for_existing_directory() {
        let tmp = std::env::temp_dir();
        ensure_dir(&tmp).unwrap();
    }

    fn init_git_repo(path: &Path) {
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(["init", path.to_str().unwrap()]);
        crate::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git init");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn add_origin(path: &Path, url: &str) {
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(["remote", "add", "origin", url]).current_dir(path);
        crate::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git remote add origin");
        assert!(
            output.status.success(),
            "git remote add origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_bare_git_repo(path: &Path) {
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(["init", "--bare", path.to_str().unwrap()]);
        crate::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git init --bare");
        assert!(
            output.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_commit_allow_empty(path: &Path, message: &str) {
        let mut cmd = crate::process::hidden_command("git");
        // Pin the author/committer identity via -c flags so the commit does not
        // depend on an ambient git user.name/user.email. scrub_git_env removes
        // GIT_* env, and CI Linux runners have no global identity, so without
        // this the commit fails with "Author identity unknown".
        cmd.args([
            "-c",
            "user.name=gwt-test",
            "-c",
            "user.email=gwt-test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            message,
        ])
        .current_dir(path);
        crate::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn comparable_path(path: &Path) -> String {
        path.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "/")
    }

    /// SPEC-2359 Phase W-15 (FR-384): close-kind work events get their own
    /// home-scoped log. It must be distinct from both the git-tracked
    /// repo-local event log and the home in-work event log: the latter is the
    /// FR-358 migration source, so writing close events there would copy them
    /// into the git-tracked file on first migration.
    #[test]
    fn workspace_close_events_path_is_home_scoped_and_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);

        let closed = gwt_workspace_work_events_closed_path_for_repo_path(&repo);
        let hash = project_scope_hash(&repo);
        assert!(closed.ends_with(gwt_home_suffix(&[
            "projects",
            hash.as_str(),
            "project-state",
            "work-events-closed.jsonl",
        ])));

        let in_work_home = gwt_workspace_work_events_path_for_repo_path(&repo);
        let repo_local = gwt_repo_local_work_events_path(&repo);
        assert_ne!(closed, in_work_home);
        assert_ne!(closed, repo_local);
    }

    #[test]
    fn gwt_workspace_work_events_intake_state_path_uses_project_state_dir() {
        let repo_hash = compute_repo_hash("git@github.com:akiojin/gwt.git");
        let path = gwt_workspace_work_events_intake_state_path(&repo_hash);
        assert!(path
            .to_string_lossy()
            .ends_with("project-state/work-events-intake.json"));
    }

    #[test]
    fn gwt_repo_local_work_events_path_joins_repo_local_work_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);

        let events = gwt_repo_local_work_events_path(&repo);
        assert!(events.ends_with(PathBuf::from(".gwt").join("work").join("events.jsonl")));
        // The repo-local path resolves to the main worktree root, so it lives
        // under the repository working tree (not under ~/.gwt).
        let root = resolve_main_worktree_root(&repo);
        assert_eq!(
            comparable_path(&events),
            comparable_path(&root.join(".gwt").join("work").join("events.jsonl"))
        );
    }

    #[test]
    fn gwt_repo_local_memory_path_joins_repo_local_work_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);

        let memory = gwt_repo_local_memory_path(&repo);
        assert!(memory.ends_with(PathBuf::from(".gwt").join("work").join("memory.md")));
        let root = resolve_main_worktree_root(&repo);
        assert_eq!(
            comparable_path(&memory),
            comparable_path(&root.join(".gwt").join("work").join("memory.md"))
        );
    }

    #[test]
    fn gwt_repo_local_discussions_path_joins_repo_local_work_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);

        let discussions = gwt_repo_local_discussions_path(&repo);
        assert!(discussions.ends_with(PathBuf::from(".gwt").join("work").join("discussions.md")));
        let root = resolve_main_worktree_root(&repo);
        assert_eq!(
            comparable_path(&discussions),
            comparable_path(&root.join(".gwt").join("work").join("discussions.md"))
        );
    }

    // -----------------------------------------------------------------------
    // SPEC-3214 Phase 4 (T-030 / FR-007) — machine-local work-notes scratch
    // -----------------------------------------------------------------------

    #[test]
    fn gwt_work_notes_paths_live_under_home_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);

        let notes_dir = gwt_work_notes_dir(&repo);
        assert_eq!(
            notes_dir,
            gwt_project_dir_for_repo_path(&repo).join("work-notes"),
            "work-notes must live in the branch-independent home project dir"
        );
        assert!(
            notes_dir.starts_with(gwt_home()),
            "work-notes must be machine-local (under ~/.gwt), got {}",
            notes_dir.display()
        );
        assert_eq!(
            gwt_work_notes_memory_path(&repo),
            notes_dir.join("memory.md")
        );
        assert_eq!(
            gwt_work_notes_discussions_path(&repo),
            notes_dir.join("discussions.md")
        );
        // The legacy repo-local helpers stay distinct: they resolve into the
        // working tree and remain read-fallback sources only.
        assert_ne!(
            comparable_path(&gwt_work_notes_memory_path(&repo)),
            comparable_path(&gwt_repo_local_memory_path(&repo))
        );
    }

    #[test]
    fn work_notes_paths_are_shared_across_worktrees_of_the_same_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("gwt");
        init_git_repo(&repo);
        git_commit_allow_empty(&repo, "initial commit");
        let mut remote = crate::process::hidden_command("git");
        remote
            .args([
                "remote",
                "add",
                "origin",
                "https://example.invalid/gwt-notes-sharing.git",
            ])
            .current_dir(&repo);
        crate::process::scrub_git_env(&mut remote);
        assert!(remote.output().expect("git remote add").status.success());

        let linked = tmp.path().join("develop");
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(["worktree", "add", "-b", "develop", linked.to_str().unwrap()])
            .current_dir(&repo);
        crate::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git worktree add -b");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // SPEC-3214 acceptance scenario 3: notes must be readable from any
        // worktree of the same repository without a git merge, so the home
        // path is keyed by the repo hash (origin URL), not the worktree.
        assert_eq!(
            gwt_work_notes_memory_path(&repo),
            gwt_work_notes_memory_path(&linked)
        );
        assert_eq!(
            gwt_work_notes_discussions_path(&repo),
            gwt_work_notes_discussions_path(&linked)
        );
    }

    #[test]
    fn resolve_work_notes_read_paths_prefer_home_then_repo_local() {
        let dir = tempfile::tempdir().unwrap();
        let _gwt_home = crate::test_support::ScopedGwtHome::set(dir.path());
        let repo = dir.path().join("repo");
        init_git_repo(&repo);

        // Neither file exists → the canonical home path (a fresh writer
        // target) is returned.
        assert_eq!(
            resolve_work_notes_memory_read_path(&repo),
            gwt_work_notes_memory_path(&repo)
        );

        // Only the legacy repo-local file exists → fall back to it for reads.
        let repo_local = gwt_repo_local_memory_path(&repo);
        std::fs::create_dir_all(repo_local.parent().unwrap()).unwrap();
        std::fs::write(&repo_local, "# Memory\n").unwrap();
        assert_eq!(
            comparable_path(&resolve_work_notes_memory_read_path(&repo)),
            comparable_path(&repo_local)
        );

        // Home file exists → home wins even when the repo-local file remains.
        let home_path = gwt_work_notes_memory_path(&repo);
        std::fs::create_dir_all(home_path.parent().unwrap()).unwrap();
        std::fs::write(&home_path, "# Memory\n").unwrap();
        assert_eq!(resolve_work_notes_memory_read_path(&repo), home_path);

        // Discussions follow the same resolution order.
        assert_eq!(
            resolve_work_notes_discussions_read_path(&repo),
            gwt_work_notes_discussions_path(&repo)
        );
        let repo_local_discussions = gwt_repo_local_discussions_path(&repo);
        std::fs::write(&repo_local_discussions, "# Discussions\n").unwrap();
        assert_eq!(
            comparable_path(&resolve_work_notes_discussions_read_path(&repo)),
            comparable_path(&repo_local_discussions)
        );
    }

    #[test]
    fn repo_local_work_events_path_is_per_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("gwt");
        init_git_repo(&repo);
        git_commit_allow_empty(&repo, "initial commit");

        let linked = tmp.path().join("develop");
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(["worktree", "add", "-b", "develop", linked.to_str().unwrap()])
            .current_dir(&repo);
        crate::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git worktree add -b");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // A git-tracked Work event log lives in each worktree's OWN working
        // tree — the shared git common dir (a bare repo in the workspace-home
        // layout) has no working tree to track files in. So the linked worktree
        // resolves to its own `.gwt/work/`, distinct from the primary's; branch
        // divergence is reconciled via the `merge=union` gitattribute on merge,
        // not via a shared filesystem path.
        let linked_events = gwt_repo_local_work_events_path(&linked);
        let repo_events = gwt_repo_local_work_events_path(&repo);
        let tail = PathBuf::from(".gwt").join("work").join("events.jsonl");
        assert_ne!(
            comparable_path(&linked_events),
            comparable_path(&repo_events)
        );
        assert_eq!(
            comparable_path(&linked_events),
            comparable_path(&std::fs::canonicalize(&linked).unwrap().join(&tail))
        );
        assert_eq!(
            comparable_path(&repo_events),
            comparable_path(&std::fs::canonicalize(&repo).unwrap().join(&tail))
        );
    }

    #[test]
    fn resolve_main_worktree_root_accepts_workspace_home_with_child_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("gwt.git");
        init_bare_git_repo(&bare);

        // The home directory itself is not a git work tree; resolution must
        // not fail and must fall back to the child bare repository.
        let layout_root = resolve_main_worktree_root(tmp.path());
        assert_eq!(
            comparable_path(&layout_root),
            comparable_path(&std::fs::canonicalize(&bare).unwrap())
        );
        // The repo-local path must still be derivable without panicking.
        let events = gwt_repo_local_work_events_path(tmp.path());
        assert!(events.ends_with(PathBuf::from(".gwt").join("work").join("events.jsonl")));
    }

    #[test]
    fn resolve_main_worktree_root_falls_back_to_input_for_non_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = resolve_main_worktree_root(tmp.path());
        assert_eq!(comparable_path(&root), comparable_path(tmp.path()));
    }
}
