//! Git worktree management

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use gwt_core::{paths::normalize_windows_child_process_path, GwtError, Result};
use serde::{Deserialize, Serialize};

const REMOTE_DELETE_TIMEOUT: Duration = Duration::from_secs(120);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Information about a single worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// Filesystem path of the worktree.
    pub path: PathBuf,
    /// Branch checked out in this worktree.
    pub branch: Option<String>,
    /// Whether the worktree is locked.
    pub locked: bool,
    /// Whether the worktree is prunable (orphaned).
    pub prunable: bool,
}

/// Manages Git worktrees for a repository.
pub struct WorktreeManager {
    repo_path: PathBuf,
}

/// Outcome of an optional remote-branch delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDeleteOutcome {
    Deleted,
    SkippedMissing,
}

impl WorktreeManager {
    /// Create a new manager for the repository at `repo_path`.
    pub fn new(repo_path: impl AsRef<Path>) -> Self {
        Self {
            repo_path: repo_path.as_ref().to_path_buf(),
        }
    }

    /// List all worktrees for this repository.
    pub fn list(&self) -> Result<Vec<WorktreeInfo>> {
        let output = gwt_core::process::run_git_logged(
            &["worktree", "list", "--porcelain"],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree list: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(format!("worktree list: {stderr}")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_porcelain_output(&stdout))
    }

    /// Fetch latest refs from `origin`.
    pub fn fetch_origin(&self) -> Result<()> {
        let output = gwt_core::process::run_git_logged(
            &["fetch", "origin", "--prune"],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("fetch origin: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(format!("fetch origin: {stderr}")));
        }

        Ok(())
    }

    /// Prepare `origin/develop` as the canonical Start Work base.
    ///
    /// Fresh bare clones can lack both `remote.origin.fetch` and
    /// `refs/remotes/origin/*`. Normalize the refspec first, prune stale
    /// tracking refs, and create remote `develop` from the remote default
    /// branch when it is missing.
    pub fn prepare_start_work_remote_develop(&self) -> Result<()> {
        crate::migration::normalize_fetch_refspec(&self.repo_path)?;
        self.fetch_origin()?;
        if self.remote_branch_exists("origin/develop")? {
            return Ok(());
        }

        let default_branch = self.remote_default_branch()?;
        let default_remote_ref = format!("origin/{default_branch}");
        self.fetch_remote_branch_tracking_ref(&default_branch)?;
        if !self.remote_branch_exists(&default_remote_ref)? {
            return Err(GwtError::Git(format!(
                "remote default branch is not available locally after fetch: {default_remote_ref}"
            )));
        }

        self.create_remote_branch_from_base(&default_remote_ref, "develop")?;
        if !self.remote_branch_exists("origin/develop")? {
            return Err(GwtError::Git(
                "failed to create and fetch origin/develop".to_string(),
            ));
        }
        Ok(())
    }

    fn remote_default_branch(&self) -> Result<String> {
        let output = gwt_core::process::run_git_logged(
            &["ls-remote", "--symref", "origin", "HEAD"],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("ls-remote origin HEAD: {e}")))?;

        if output.status.success() {
            if let Some(branch) = parse_ls_remote_head_symref(&output.stdout) {
                return Ok(branch);
            }
        }

        let local_head = gwt_core::process::run_git_logged(
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD",
            ],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("symbolic-ref origin/HEAD: {e}")))?;
        if local_head.status.success() {
            let branch = String::from_utf8_lossy(&local_head.stdout)
                .trim()
                .strip_prefix("origin/")
                .map(str::to_string)
                .unwrap_or_else(|| {
                    String::from_utf8_lossy(&local_head.stdout)
                        .trim()
                        .to_string()
                });
            if !branch.is_empty() {
                return Ok(branch);
            }
        }

        Err(GwtError::Git(format!(
            "failed to resolve remote default branch: {}",
            command_stderr(&output)
        )))
    }

    fn fetch_remote_branch_tracking_ref(&self, branch: &str) -> Result<()> {
        if branch.trim().is_empty() {
            return Err(GwtError::Git("remote branch name is empty".to_string()));
        }
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        let output = gwt_core::process::run_git_logged(
            &["fetch", "origin", "--prune", &refspec],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("fetch origin {refspec}: {e}")))?;

        if !output.status.success() {
            return Err(GwtError::Git(format!(
                "fetch origin {refspec}: {}",
                command_stderr(&output)
            )));
        }

        Ok(())
    }

    /// Return whether a remote-tracking branch exists.
    ///
    /// Accepts `origin/<name>` or `refs/remotes/origin/<name>`.
    pub fn remote_branch_exists(&self, remote_ref: &str) -> Result<bool> {
        let tracking_ref = to_tracking_ref(remote_ref)
            .ok_or_else(|| GwtError::Git(format!("invalid remote ref: {remote_ref}")))?;

        let output = gwt_core::process::run_git_logged(
            &["show-ref", "--verify", "--quiet", &tracking_ref],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("show-ref {tracking_ref}: {e}")))?;

        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Err(GwtError::Git(format!("show-ref {tracking_ref}: {stderr}")))
            }
        }
    }

    /// Create a new worktree at `path` for `branch`.
    pub fn create(&self, branch: &str, path: &Path) -> Result<()> {
        let path_arg = path_arg_for_git(path);
        let output = gwt_core::process::run_git_logged(
            &["worktree", "add", path_arg.as_str(), branch],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree add: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Create a new worktree at `path`, creating `new_branch` from `base_branch`.
    pub fn create_from_base(&self, base_branch: &str, new_branch: &str, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(GwtError::Git(format!(
                "worktree path already exists: {}",
                path.display()
            )));
        }

        let path_arg = path_arg_for_git(path);
        let output = gwt_core::process::run_git_logged(
            &[
                "worktree",
                "add",
                "-b",
                new_branch,
                path_arg.as_str(),
                base_branch,
            ],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree add -b: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Create `origin/<new_branch>` from a remote base reference.
    ///
    /// `base_remote_ref` must be `origin/<name>` or `refs/remotes/origin/<name>`.
    /// After a successful push, the corresponding remote-tracking ref is
    /// fetched explicitly so single-branch or freshly-normalized clones can
    /// materialize the branch immediately.
    pub fn create_remote_branch_from_base(
        &self,
        base_remote_ref: &str,
        new_branch: &str,
    ) -> Result<()> {
        let base_ref = normalize_remote_ref(base_remote_ref);
        let push_refspec = format!("{base_ref}:refs/heads/{new_branch}");
        let output = gwt_core::process::run_git_logged(
            &["push", "origin", &push_refspec],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("push origin {push_refspec}: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        self.fetch_remote_branch_tracking_ref(new_branch)?;
        Ok(())
    }

    /// Delete the remote branch that corresponds to `local_branch`.
    ///
    /// When `upstream` is provided it wins; otherwise the helper falls back to
    /// `origin/<local_branch>`. Missing tracking refs are treated as a
    /// successful no-op so cleanup can continue after local deletion has
    /// already succeeded.
    pub fn delete_remote_branch(
        &self,
        local_branch: &str,
        upstream: Option<&str>,
    ) -> Result<RemoteDeleteOutcome> {
        let remote_ref = upstream
            .map(normalize_remote_ref)
            .unwrap_or_else(|| format!("origin/{local_branch}"));

        if !self.remote_branch_exists(&remote_ref)? {
            return Ok(RemoteDeleteOutcome::SkippedMissing);
        }

        let normalized = normalize_remote_ref(&remote_ref);
        let (remote, branch) = normalized
            .split_once('/')
            .ok_or_else(|| GwtError::Git(format!("invalid remote ref: {normalized}")))?;

        let mut command = gwt_core::process::hidden_command("git");
        command
            .args(["push", remote, "--delete", branch])
            .current_dir(&self.repo_path);
        let action = format!("git push {remote} --delete {branch}");
        let output = run_command_with_timeout(&mut command, &action, REMOTE_DELETE_TIMEOUT)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(RemoteDeleteOutcome::Deleted)
    }

    /// Create a local worktree branch from a remote-tracking branch.
    ///
    /// `remote_ref` must be `origin/<name>` or `refs/remotes/origin/<name>`.
    pub fn create_from_remote(
        &self,
        remote_ref: &str,
        local_branch: &str,
        path: &Path,
    ) -> Result<()> {
        let remote_ref = normalize_remote_ref(remote_ref);
        let path_arg = path_arg_for_git(path);
        let output = gwt_core::process::run_git_logged(
            &[
                "worktree",
                "add",
                "-b",
                local_branch,
                path_arg.as_str(),
                &remote_ref,
            ],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree add -b from remote: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        let output = gwt_core::process::run_git_logged(
            &["branch", "--set-upstream-to", &remote_ref, local_branch],
            Some(path),
        )
        .map_err(|e| GwtError::Git(format!("set-upstream-to {remote_ref}: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Remove the worktree at `path`.
    pub fn remove(&self, path: &Path) -> Result<()> {
        let path_arg = path_arg_for_git(path);
        let output = gwt_core::process::run_git_logged(
            &["worktree", "remove", path_arg.as_str()],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree remove: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Force-remove the worktree at `path` (FR-018f). Equivalent to
    /// `git worktree remove --force` so worktrees with uncommitted or
    /// untracked files can also be deleted by Branch Cleanup.
    pub fn remove_force(&self, path: &Path) -> Result<()> {
        let path_arg = path_arg_for_git(path);
        let output = gwt_core::process::run_git_logged(
            &["worktree", "remove", "--force", path_arg.as_str()],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree remove --force: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Force-remove the worktree at `path`, including locked worktrees.
    ///
    /// Git requires `--force` twice for locked worktrees. Keep this behind
    /// the explicit cleanup force mode so normal cleanup still respects that
    /// extra guardrail.
    pub fn remove_force_twice(&self, path: &Path) -> Result<()> {
        let path_arg = path_arg_for_git(path);
        let output = gwt_core::process::hidden_command("git")
            .args([
                "worktree",
                "remove",
                "--force",
                "--force",
                path_arg.as_str(),
            ])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::Git(format!("worktree remove --force --force: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Run `git worktree prune --expire now` to clear orphaned worktree
    /// metadata immediately. Plain `git worktree prune` honors the
    /// `gc.worktreePruneExpire` grace period (default `3.months.ago`), so a
    /// freshly deleted worktree would stay registered and block the force
    /// branch delete that follows inside [`Self::cleanup_branch`]. `--expire
    /// now` disables that grace period.
    pub fn prune(&self) -> Result<()> {
        let output = gwt_core::process::run_git_logged(
            &["worktree", "prune", "--expire", "now"],
            Some(&self.repo_path),
        )
        .map_err(|e| GwtError::Git(format!("worktree prune: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Force-remove the worktree bound to `branch` (when one exists) and
    /// then delete the local branch (FR-018f, FR-018g). Mirrors the old
    /// the legacy GUI cleanup command implementation.
    ///
    /// The function is idempotent against missing worktrees, missing
    /// branches, and orphaned worktree metadata: a `git worktree prune`
    /// fallback handles entries whose on-disk path has already vanished.
    pub fn cleanup_branch(&self, branch: &str) -> Result<()> {
        self.cleanup_branch_with_force_filesystem_delete(branch, false)
    }

    /// Remove the worktree and local branch for `branch`.
    ///
    /// When `force_filesystem_delete` is true, cleanup may use Git's double
    /// force for locked worktrees and may remove leftover filesystem residue
    /// from the Git-registered worktree path if Git reports a non-empty
    /// directory class failure. Callers must still apply branch-level safety
    /// checks before enabling this mode.
    pub fn cleanup_branch_with_force_filesystem_delete(
        &self,
        branch: &str,
        force_filesystem_delete: bool,
    ) -> Result<()> {
        let worktree_path = self
            .list()?
            .into_iter()
            .find(|wt| wt.branch.as_deref() == Some(branch))
            .map(|wt| wt.path);

        if let Some(path) = worktree_path {
            let remove_result = if force_filesystem_delete {
                self.remove_force_twice(&path)
            } else {
                self.remove_force(&path)
            };
            match remove_result {
                Ok(()) => {}
                Err(err) if is_missing_worktree_error(&err) => {
                    // Worktree metadata is stale; prune and continue.
                    self.prune()?;
                }
                Err(err) if force_filesystem_delete && is_filesystem_residue_error(&err) => {
                    validate_force_filesystem_residue_path(&self.repo_path, branch, &path)?;
                    remove_worktree_filesystem_residue(&path)?;
                    self.prune()?;
                }
                Err(err) => return Err(err),
            }
        }

        crate::branch::delete_local_branch(&self.repo_path, branch, true)?;
        Ok(())
    }
}

/// Returns true when `err` looks like a `git worktree remove` failure that
/// stems from the on-disk worktree path having already disappeared.
fn is_missing_worktree_error(err: &GwtError) -> bool {
    let message = err.to_string().to_lowercase();
    message.contains("not a working tree")
        || message.contains("not a worktree")
        || message.contains("not a work tree")
        || message.contains("no such file or directory")
        || message.contains("does not exist")
}

fn is_filesystem_residue_error(err: &GwtError) -> bool {
    let message = err.to_string().to_lowercase();
    message.contains("directory not empty")
        || message.contains("failed to delete")
        || message.contains("failed to remove")
}

fn remove_worktree_filesystem_residue(path: &Path) -> Result<()> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path)
            .map_err(|e| GwtError::Git(format!("remove worktree residue: {e}"))),
        Ok(_) => std::fs::remove_file(path)
            .map_err(|e| GwtError::Git(format!("remove worktree residue: {e}"))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(GwtError::Git(format!("inspect worktree residue: {err}"))),
    }
}

fn validate_force_filesystem_residue_path(
    repo_path: &Path,
    branch: &str,
    path: &Path,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let expected_path = sibling_worktree_path(repo_path, branch);
    let actual = std::fs::canonicalize(path)
        .map_err(|e| GwtError::Git(format!("inspect worktree residue path: {e}")))?;
    let expected = std::fs::canonicalize(&expected_path).map_err(|_| {
        GwtError::Git(format!(
            "refusing to force-delete worktree residue outside managed workspace: {} (expected {})",
            path.display(),
            expected_path.display()
        ))
    })?;

    if actual != expected {
        return Err(GwtError::Git(format!(
            "refusing to force-delete worktree residue outside managed workspace: {} (expected {})",
            path.display(),
            expected_path.display()
        )));
    }

    Ok(())
}

fn run_command_with_timeout(
    command: &mut Command,
    action: &str,
    timeout: Duration,
) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    configure_timeout_command(command);
    let mut child = command
        .spawn()
        .map_err(|error| GwtError::Git(format!("{action}: {error}")))?;
    let mut stdout = child.stdout.take().map(spawn_pipe_reader);
    let mut stderr = child.stderr.take().map(spawn_pipe_reader);
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let status = child
                    .wait()
                    .map_err(|error| GwtError::Git(format!("{action}: {error}")))?;
                return Ok(Output {
                    status,
                    stdout: join_pipe_reader(stdout.take(), action, "stdout")?,
                    stderr: join_pipe_reader(stderr.take(), action, "stderr")?,
                });
            }
            Ok(None) if started.elapsed() >= timeout => {
                terminate_child_tree(&mut child);
                let _ = child.wait();
                join_pipe_reader_lossy(stdout.take());
                join_pipe_reader_lossy(stderr.take());
                return Err(GwtError::Git(format!(
                    "{action} timed out after {}ms",
                    timeout.as_millis()
                )));
            }
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(error) => {
                terminate_child_tree(&mut child);
                let _ = child.wait();
                join_pipe_reader_lossy(stdout.take());
                join_pipe_reader_lossy(stderr.take());
                return Err(GwtError::Git(format!("{action}: {error}")));
            }
        }
    }
}

#[cfg(unix)]
fn configure_timeout_command(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_timeout_command(_command: &mut Command) {}

fn terminate_child_tree(child: &mut Child) {
    terminate_child_tree_platform(child.id());
    let _ = child.kill();
}

#[cfg(unix)]
fn terminate_child_tree_platform(pid: u32) {
    let process_group = -(pid as libc::pid_t);
    // Kill the dedicated process group so descendants that inherited pipes close them too.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_child_tree_platform(pid: u32) {
    let _ = gwt_core::process::hidden_command("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_child_tree_platform(_pid: u32) {}

fn spawn_pipe_reader<T>(mut pipe: T) -> JoinHandle<std::io::Result<Vec<u8>>>
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).map(|_| bytes)
    })
}

fn join_pipe_reader(
    reader: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    action: &str,
    stream: &str,
) -> Result<Vec<u8>> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    match reader.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(GwtError::Git(format!("{action} read {stream}: {error}"))),
        Err(_) => Err(GwtError::Git(format!(
            "{action} read {stream}: reader thread panicked"
        ))),
    }
}

fn join_pipe_reader_lossy(reader: Option<JoinHandle<std::io::Result<Vec<u8>>>>) {
    if let Some(reader) = reader {
        let _ = reader.join();
    }
}

fn parse_ls_remote_head_symref(stdout: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stdout).lines().find_map(|line| {
        let (symref, target) = line.split_once('\t')?;
        if target != "HEAD" {
            return None;
        }
        symref
            .strip_prefix("ref: refs/heads/")
            .filter(|branch| !branch.is_empty())
            .map(str::to_string)
    })
}

fn command_stderr(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        "git command failed without stderr".to_string()
    } else {
        stderr
    }
}

fn normalize_remote_ref(remote_ref: &str) -> String {
    if let Some(stripped) = remote_ref.strip_prefix("refs/remotes/") {
        stripped.to_string()
    } else {
        remote_ref.to_string()
    }
}

fn to_tracking_ref(remote_ref: &str) -> Option<String> {
    if remote_ref.starts_with("refs/remotes/") {
        Some(remote_ref.to_string())
    } else if remote_ref.contains('/') {
        Some(format!("refs/remotes/{remote_ref}"))
    } else {
        None
    }
}

/// Resolve the main worktree root for a repository or linked worktree path.
pub fn main_worktree_root(repo_path: &Path) -> Result<PathBuf> {
    let output = gwt_core::process::run_git_logged(
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        Some(repo_path),
    )
    .map_err(|e| GwtError::Git(format!("rev-parse --git-common-dir: {e}")))?;

    if !output.status.success() {
        if let Some(bare_child) = first_child_bare_repository(repo_path) {
            let bare_child = std::fs::canonicalize(&bare_child).unwrap_or(bare_child);
            return Ok(normalize_windows_child_process_path(&bare_child));
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!(
            "rev-parse --git-common-dir: {stderr}"
        )));
    }

    let common_dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if common_dir.as_os_str().is_empty() {
        return Err(GwtError::Git(
            "rev-parse --git-common-dir returned an empty path".to_string(),
        ));
    }

    if common_dir.file_name().and_then(|name| name.to_str()) == Some(".git") {
        let repo_root = common_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
            GwtError::Git(format!(
                "git common dir has no parent repository: {}",
                common_dir.display()
            ))
        })?;
        return Ok(normalize_windows_child_process_path(&repo_root));
    }

    Ok(normalize_windows_child_process_path(&common_dir))
}

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

/// Derive a sibling worktree path from the repo root and branch name.
///
/// The layout root stays at the same directory level as the repository or
/// bare common-dir, while the branch name itself becomes the relative
/// directory hierarchy (for example `feature/aaa` -> `../feature/aaa`).
pub fn sibling_worktree_path(repo_path: &Path, branch: &str) -> PathBuf {
    let layout_root = repo_path.parent().unwrap_or(repo_path);
    let mut path = layout_root.to_path_buf();

    for segment in branch.trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        path.push(segment);
    }

    path
}

fn path_arg_for_git(path: &Path) -> String {
    normalize_windows_child_process_path(path)
        .to_string_lossy()
        .into_owned()
}

/// Parse `git worktree list --porcelain` output into `WorktreeInfo` entries.
fn parse_porcelain_output(output: &str) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut locked = false;
    let mut prunable = false;

    for line in output.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            // Flush previous entry
            if let Some(prev_path) = path.take() {
                worktrees.push(WorktreeInfo {
                    path: prev_path,
                    branch: branch.take(),
                    locked,
                    prunable,
                });
                locked = false;
                prunable = false;
            }
            path = Some(normalize_windows_child_process_path(Path::new(p)));
        } else if let Some(b) = line.strip_prefix("branch ") {
            // Strip refs/heads/ prefix
            branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        } else if matches_annotation(line, "locked") {
            locked = true;
        } else if matches_annotation(line, "prunable") {
            prunable = true;
        }
    }

    // Flush last entry
    if let Some(p) = path {
        worktrees.push(WorktreeInfo {
            path: p,
            branch,
            locked,
            prunable,
        });
    }

    worktrees
}

fn matches_annotation(line: &str, key: &str) -> bool {
    line == key
        || line
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comparable_path(path: &Path) -> String {
        path.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "/")
    }

    #[cfg(windows)]
    #[test]
    fn path_arg_for_git_strips_windows_verbatim_prefixes() {
        assert_eq!(
            path_arg_for_git(Path::new(r"\\?\C:\tmp\repo\work")),
            r"C:\tmp\repo\work"
        );
        assert_eq!(
            path_arg_for_git(Path::new(r"\\?\UNC\server\share\work")),
            r"\\server\share\work"
        );
    }

    fn init_git_repo(path: &Path) {
        let output = gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");

        let email = gwt_core::process::run_git_logged(
            &["config", "user.email", "test@example.com"],
            Some(path),
        )
        .expect("git config user.email");
        assert!(email.status.success(), "git config user.email failed");

        let name =
            gwt_core::process::run_git_logged(&["config", "user.name", "Test User"], Some(path))
                .expect("git config user.name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn init_bare_git_repo(path: &Path) {
        let output = gwt_core::process::hidden_command("git")
            .args(["init", "--bare", path.to_str().unwrap()])
            .output()
            .expect("git init --bare");
        assert!(output.status.success(), "git init --bare failed");
    }

    fn slow_command() -> std::process::Command {
        if cfg!(windows) {
            let mut command = gwt_core::process::hidden_command("powershell");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Milliseconds 250"]);
            command
        } else {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "sleep 0.25"]);
            command
        }
    }

    fn verbose_command() -> std::process::Command {
        if cfg!(windows) {
            let mut command = gwt_core::process::hidden_command("powershell");
            command.args([
                "-NoProfile",
                "-Command",
                "[Console]::Out.Write(('x' * 200000))",
            ]);
            command
        } else {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "yes x | head -c 200000"]);
            command
        }
    }

    fn lingering_pipe_command() -> std::process::Command {
        if cfg!(windows) {
            let mut command = gwt_core::process::hidden_command("powershell");
            command.args([
                "-NoProfile",
                "-Command",
                "$psi = [Diagnostics.ProcessStartInfo]::new('powershell'); $psi.Arguments = '-NoProfile -Command Start-Sleep -Milliseconds 3000'; $psi.UseShellExecute = $false; [Diagnostics.Process]::Start($psi) | Out-Null; Start-Sleep -Milliseconds 3000",
            ]);
            command
        } else {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "(sleep 3) & sleep 3"]);
            command
        }
    }

    fn git_clone_repo(src: &Path, dst: &Path) {
        let output = gwt_core::process::hidden_command("git")
            .args(["clone", src.to_str().unwrap(), dst.to_str().unwrap()])
            .output()
            .expect("git clone");
        assert!(
            output.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_push_branch(path: &Path, branch: &str) {
        let output =
            gwt_core::process::run_git_logged(&["push", "-u", "origin", branch], Some(path))
                .expect("git push -u origin");
        assert!(
            output.status.success(),
            "git push -u origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_commit_allow_empty(path: &Path, message: &str) {
        let output = gwt_core::process::run_git_logged(
            &["commit", "--allow-empty", "-m", message],
            Some(path),
        )
        .expect("git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_checkout_new_branch(path: &Path, branch: &str) {
        let output = gwt_core::process::run_git_logged(&["checkout", "-b", branch], Some(path))
            .expect("git checkout -b");
        assert!(
            output.status.success(),
            "git checkout -b failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn parse_porcelain_single_entry() {
        let output = "worktree /home/user/repo\nbranch refs/heads/main\nHEAD abc1234\n\n";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/home/user/repo"));
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].locked);
        assert!(!entries[0].prunable);
    }

    #[test]
    fn parse_porcelain_strips_windows_verbatim_prefixes() {
        let output = "\
worktree \\\\?\\E:\\gwt\\work\\20260525-0919
branch refs/heads/work/20260525-0919

worktree \\\\?\\UNC\\server\\share\\work
branch refs/heads/work/unc
";
        let entries = parse_porcelain_output(output);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, PathBuf::from(r"E:\gwt\work\20260525-0919"));
        assert_eq!(entries[0].branch.as_deref(), Some("work/20260525-0919"));
        assert_eq!(entries[1].path, PathBuf::from(r"\\server\share\work"));
        assert_eq!(entries[1].branch.as_deref(), Some("work/unc"));
    }

    #[test]
    fn parse_porcelain_multiple_entries() {
        let output = "\
worktree /repo
branch refs/heads/main

worktree /repo/wt-1
branch refs/heads/feature
locked

worktree /repo/wt-2
branch refs/heads/fix
prunable
";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].locked);
        assert_eq!(entries[1].branch.as_deref(), Some("feature"));
        assert!(entries[1].locked);
        assert_eq!(entries[2].branch.as_deref(), Some("fix"));
        assert!(entries[2].prunable);
    }

    #[test]
    fn parse_porcelain_reasoned_annotations() {
        let output = "\
worktree /repo
branch refs/heads/main
locked because maintenance is running

worktree /repo/wt-1
branch refs/heads/feature
prunable gitdir file points to non-existent location
";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(entries[0].locked);
        assert!(!entries[0].prunable);
        assert_eq!(entries[1].branch.as_deref(), Some("feature"));
        assert!(!entries[1].locked);
        assert!(entries[1].prunable);
    }

    #[test]
    fn parse_porcelain_empty() {
        let entries = parse_porcelain_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_porcelain_detached_head() {
        let output = "worktree /repo\nHEAD abc1234\ndetached\n\n";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].branch.is_none());
    }

    #[test]
    fn sibling_worktree_path_preserves_branch_hierarchy() {
        let repo_path = Path::new("/tmp/my-repo");
        let worktree = sibling_worktree_path(repo_path, "feature/banner");
        assert_eq!(worktree, PathBuf::from("/tmp/feature/banner"));
    }

    #[test]
    fn main_worktree_root_returns_primary_repo_for_linked_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("gwt");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let linked_worktree = tmp.path().join("develop");
        let output = gwt_core::process::run_git_logged(
            &[
                "worktree",
                "add",
                "-b",
                "develop",
                linked_worktree.to_str().unwrap(),
            ],
            Some(&repo_path),
        )
        .expect("git worktree add -b");
        assert!(
            output.status.success(),
            "git worktree add -b failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(
            comparable_path(&main_worktree_root(&linked_worktree).unwrap()),
            comparable_path(&std::fs::canonicalize(&repo_path).unwrap())
        );
    }

    #[test]
    fn main_worktree_root_uses_bare_common_dir_for_linked_workspace_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let bare_repo_path = tmp.path().join("gwt.git");
        init_bare_git_repo(&bare_repo_path);

        let bootstrap_path = tmp.path().join("bootstrap");
        git_clone_repo(&bare_repo_path, &bootstrap_path);
        let email = gwt_core::process::run_git_logged(
            &["config", "user.email", "test@example.com"],
            Some(&bootstrap_path),
        )
        .expect("git config user.email");
        assert!(email.status.success(), "git config user.email failed");
        let name = gwt_core::process::run_git_logged(
            &["config", "user.name", "Test User"],
            Some(&bootstrap_path),
        )
        .expect("git config user.name");
        assert!(name.status.success(), "git config user.name failed");
        git_checkout_new_branch(&bootstrap_path, "develop");
        git_commit_allow_empty(&bootstrap_path, "initial commit");
        git_push_branch(&bootstrap_path, "develop");

        let linked_worktree = tmp.path().join("develop");
        let output = gwt_core::process::run_git_logged(
            &[
                "worktree",
                "add",
                linked_worktree.to_str().unwrap(),
                "develop",
            ],
            Some(&bare_repo_path),
        )
        .expect("git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let layout_root = main_worktree_root(&linked_worktree).unwrap();
        assert_eq!(
            comparable_path(&layout_root),
            comparable_path(&std::fs::canonicalize(&bare_repo_path).unwrap())
        );
        let expected_parent = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(
            comparable_path(&sibling_worktree_path(&layout_root, "feature/banner")),
            comparable_path(&expected_parent.join("feature").join("banner"))
        );
    }

    #[test]
    fn main_worktree_root_accepts_workspace_home_with_child_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let bare_repo_path = tmp.path().join("gwt.git");
        init_bare_git_repo(&bare_repo_path);

        let layout_root = main_worktree_root(tmp.path()).unwrap();

        assert_eq!(
            comparable_path(&layout_root),
            comparable_path(&std::fs::canonicalize(&bare_repo_path).unwrap())
        );
    }

    #[test]
    fn create_from_base_creates_new_branch_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_new_branch(&repo_path, "develop");

        let manager = WorktreeManager::new(&repo_path);
        let worktree_path = sibling_worktree_path(&repo_path, "feature/materialized");

        manager
            .create_from_base("develop", "feature/materialized", &worktree_path)
            .unwrap();

        assert!(worktree_path.exists());
        let branch_output =
            gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&worktree_path))
                .expect("git branch --show-current");
        assert!(branch_output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&branch_output.stdout).trim(),
            "feature/materialized"
        );
    }

    #[test]
    fn remove_force_deletes_worktree_with_uncommitted_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let manager = WorktreeManager::new(&repo_path);
        let worktree_path = sibling_worktree_path(&repo_path, "feature/dirty");
        manager
            .create_from_base("main", "feature/dirty", &worktree_path)
            .or_else(|_| {
                // Default branch may be `master` on older git installations.
                manager.create_from_base("master", "feature/dirty", &worktree_path)
            })
            .unwrap();

        // Dirty the worktree (uncommitted + untracked).
        std::fs::write(worktree_path.join("dirty.txt"), "dirty\n").unwrap();
        std::fs::write(worktree_path.join("staged.txt"), "staged\n").unwrap();
        gwt_core::process::run_git_logged(&["add", "staged.txt"], Some(&worktree_path)).unwrap();

        // Plain remove() refuses; remove_force() succeeds.
        assert!(manager.remove(&worktree_path).is_err());
        manager.remove_force(&worktree_path).unwrap();
        assert!(!worktree_path.exists());
    }

    #[test]
    fn cleanup_branch_force_filesystem_mode_removes_locked_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let manager = WorktreeManager::new(&repo_path);
        let worktree_path = sibling_worktree_path(&repo_path, "work/locked");
        manager
            .create_from_base("main", "work/locked", &worktree_path)
            .or_else(|_| manager.create_from_base("master", "work/locked", &worktree_path))
            .unwrap();

        let lock = gwt_core::process::hidden_command("git")
            .args(["worktree", "lock", worktree_path.to_str().unwrap()])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree lock");
        assert!(
            lock.status.success(),
            "git worktree lock failed: {}",
            String::from_utf8_lossy(&lock.stderr)
        );

        assert!(
            manager.cleanup_branch("work/locked").is_err(),
            "single-force cleanup should not remove a locked worktree"
        );
        manager
            .cleanup_branch_with_force_filesystem_delete("work/locked", true)
            .unwrap();

        assert!(!worktree_path.exists());
        let branches = crate::branch::list_branches(&repo_path).unwrap();
        assert!(!branches
            .iter()
            .any(|b| b.is_local && b.name == "work/locked"));
    }

    #[test]
    fn filesystem_residue_helper_removes_non_empty_worktree_path() {
        let tmp = tempfile::tempdir().unwrap();
        let residue_path = tmp.path().join("work").join("residue");
        std::fs::create_dir_all(residue_path.join("nested")).unwrap();
        std::fs::write(
            residue_path.join("nested").join("leftover.txt"),
            "leftover\n",
        )
        .unwrap();

        remove_worktree_filesystem_residue(&residue_path).unwrap();

        assert!(!residue_path.exists());
    }

    #[test]
    fn force_filesystem_residue_path_validation_rejects_path_outside_expected_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        let unrelated_path = tmp.path().join("unrelated").join("work").join("residue");
        std::fs::create_dir_all(&repo_path).unwrap();
        std::fs::create_dir_all(&unrelated_path).unwrap();

        let err =
            validate_force_filesystem_residue_path(&repo_path, "work/residue", &unrelated_path)
                .expect_err("force residue cleanup must reject unrelated paths");

        assert!(err
            .to_string()
            .contains("refusing to force-delete worktree residue outside managed workspace"));
        assert!(
            unrelated_path.exists(),
            "validation must not remove unrelated paths"
        );
    }

    #[test]
    fn force_filesystem_residue_path_validation_accepts_expected_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        let expected_path = sibling_worktree_path(&repo_path, "work/residue");
        std::fs::create_dir_all(&repo_path).unwrap();
        std::fs::create_dir_all(&expected_path).unwrap();

        validate_force_filesystem_residue_path(&repo_path, "work/residue", &expected_path)
            .expect("expected sibling worktree path should be force-deletable");
    }

    #[test]
    fn filesystem_residue_error_matches_git_directory_not_empty_failures() {
        assert!(is_filesystem_residue_error(&GwtError::Git(
            "error: failed to delete '/repo/work/old': Directory not empty".to_string(),
        )));
        assert!(!is_filesystem_residue_error(&GwtError::Git(
            "fatal: invalid reference: work/old".to_string(),
        )));
    }

    #[test]
    fn cleanup_branch_removes_worktree_and_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let manager = WorktreeManager::new(&repo_path);
        let worktree_path = sibling_worktree_path(&repo_path, "feature/cleanup-me");
        manager
            .create_from_base("main", "feature/cleanup-me", &worktree_path)
            .or_else(|_| manager.create_from_base("master", "feature/cleanup-me", &worktree_path))
            .unwrap();
        // Make it dirty so a non-force remove would fail.
        std::fs::write(worktree_path.join("scratch.txt"), "scratch\n").unwrap();

        manager.cleanup_branch("feature/cleanup-me").unwrap();

        assert!(!worktree_path.exists());
        let branches = crate::branch::list_branches(&repo_path).unwrap();
        assert!(
            !branches
                .iter()
                .any(|b| b.is_local && b.name == "feature/cleanup-me"),
            "branch should be deleted: {branches:?}"
        );
    }

    #[test]
    fn cleanup_branch_is_idempotent_for_missing_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let manager = WorktreeManager::new(&repo_path);
        // Branch was never created — cleanup must still succeed.
        manager.cleanup_branch("feature/never-existed").unwrap();
    }

    #[test]
    fn cleanup_branch_succeeds_after_worktree_dir_was_manually_removed() {
        // Regression guard for the `prune()` grace-period bug: if the
        // worktree directory is wiped from disk behind git's back,
        // `cleanup_branch` must still be able to deregister it and delete
        // the branch. Without `--expire now` on `git worktree prune`, the
        // stale metadata would linger for ~3 months and `git branch -D`
        // would refuse to delete the branch because it still looks
        // checked out.
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let manager = WorktreeManager::new(&repo_path);
        let worktree_path = sibling_worktree_path(&repo_path, "feature/orphaned");
        manager
            .create_from_base("main", "feature/orphaned", &worktree_path)
            .or_else(|_| manager.create_from_base("master", "feature/orphaned", &worktree_path))
            .unwrap();

        // Simulate an external tool (rm -rf, macOS finder, ...) wiping the
        // worktree directory without telling git.
        std::fs::remove_dir_all(&worktree_path).unwrap();

        manager.cleanup_branch("feature/orphaned").unwrap();

        let branches = crate::branch::list_branches(&repo_path).unwrap();
        assert!(
            !branches
                .iter()
                .any(|b| b.is_local && b.name == "feature/orphaned"),
            "orphaned branch should be deleted: {branches:?}"
        );
    }

    #[test]
    fn cleanup_branch_without_worktree_deletes_branch_only() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        // Create a branch but no worktree.
        gwt_core::process::run_git_logged(&["branch", "feature/no-worktree"], Some(&repo_path))
            .unwrap();

        let manager = WorktreeManager::new(&repo_path);
        manager.cleanup_branch("feature/no-worktree").unwrap();

        let branches = crate::branch::list_branches(&repo_path).unwrap();
        assert!(!branches
            .iter()
            .any(|b| b.is_local && b.name == "feature/no-worktree"));
    }

    #[test]
    fn remote_branch_exists_checks_origin_tracking_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        let remote_path = tmp.path().join("origin.git");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_new_branch(&repo_path, "develop");
        git_push_branch(&repo_path, "develop");

        let manager = WorktreeManager::new(&repo_path);
        manager.fetch_origin().unwrap();

        assert!(manager.remote_branch_exists("origin/develop").unwrap());
        assert!(!manager
            .remote_branch_exists("origin/feature/missing")
            .unwrap());
    }

    #[test]
    fn delete_remote_branch_removes_existing_upstream_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        let remote_path = tmp.path().join("origin.git");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_new_branch(&repo_path, "feature/prune-me");
        git_push_branch(&repo_path, "feature/prune-me");

        let manager = WorktreeManager::new(&repo_path);
        manager.fetch_origin().unwrap();

        assert_eq!(
            manager
                .delete_remote_branch("feature/prune-me", Some("origin/feature/prune-me"))
                .unwrap(),
            RemoteDeleteOutcome::Deleted
        );
        assert!(!manager
            .remote_branch_exists("origin/feature/prune-me")
            .unwrap());
    }

    #[test]
    fn delete_remote_branch_skips_when_tracking_ref_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        let remote_path = tmp.path().join("origin.git");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let manager = WorktreeManager::new(&repo_path);
        manager.fetch_origin().unwrap();

        assert_eq!(
            manager
                .delete_remote_branch("feature/missing", None)
                .unwrap(),
            RemoteDeleteOutcome::SkippedMissing
        );
    }

    #[test]
    fn run_command_with_timeout_reports_timeout() {
        let mut command = slow_command();
        let err = run_command_with_timeout(
            &mut command,
            "git push origin --delete feature/slow",
            std::time::Duration::from_millis(10),
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("git push origin --delete feature/slow timed out"),
            "unexpected timeout error: {err}"
        );
    }

    #[test]
    fn run_command_with_timeout_drains_verbose_child_output() {
        let mut command = verbose_command();
        let output =
            run_command_with_timeout(&mut command, "verbose child", Duration::from_secs(5))
                .expect("verbose child should exit without filling pipe buffers");

        assert!(output.status.success());
        assert!(
            output.stdout.len() >= 200_000,
            "expected captured stdout, got {} bytes",
            output.stdout.len()
        );
    }

    #[test]
    fn run_command_with_timeout_does_not_wait_for_lingering_descendant_pipes() {
        let mut command = lingering_pipe_command();
        let started = Instant::now();
        let err = run_command_with_timeout(
            &mut command,
            "lingering descendant",
            Duration::from_millis(500),
        )
        .unwrap_err();
        let elapsed = started.elapsed();

        assert!(
            err.to_string().contains("lingering descendant timed out"),
            "unexpected timeout error: {err}"
        );
        assert!(
            elapsed < Duration::from_millis(1500),
            "timeout path waited for descendant pipe handles for {elapsed:?}"
        );
    }

    #[test]
    fn create_remote_branch_from_base_then_create_from_remote_materializes_tracking_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");
        let remote_path = tmp.path().join("origin.git");
        std::fs::create_dir_all(&repo_path).unwrap();
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_new_branch(&repo_path, "develop");
        git_push_branch(&repo_path, "develop");

        let manager = WorktreeManager::new(&repo_path);
        manager.fetch_origin().unwrap();
        manager
            .create_remote_branch_from_base("origin/develop", "feature/materialized")
            .unwrap();
        manager.fetch_origin().unwrap();
        assert!(manager
            .remote_branch_exists("origin/feature/materialized")
            .unwrap());

        let worktree_path = sibling_worktree_path(&repo_path, "feature/materialized");
        manager
            .create_from_remote(
                "origin/feature/materialized",
                "feature/materialized",
                &worktree_path,
            )
            .unwrap();
        assert!(worktree_path.exists());

        let branch_output =
            gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&worktree_path))
                .expect("git branch --show-current");
        assert!(branch_output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&branch_output.stdout).trim(),
            "feature/materialized"
        );

        let upstream_output = gwt_core::process::run_git_logged(
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
            Some(&worktree_path),
        )
        .expect("git rev-parse @{upstream}");
        assert!(upstream_output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&upstream_output.stdout).trim(),
            "origin/feature/materialized"
        );
    }

    #[test]
    fn list_worktrees_in_test_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        gwt_core::process::run_git_logged(&["commit", "--allow-empty", "-m", "init"], Some(path))
            .unwrap();

        let mgr = WorktreeManager::new(path);
        let wts = mgr.list().unwrap();
        // At minimum the main worktree
        assert!(!wts.is_empty());
    }

    fn git_add_remote(path: &Path, name: &str, remote: &Path) {
        let output = gwt_core::process::run_git_logged(
            &["remote", "add", name, remote.to_str().unwrap()],
            Some(path),
        )
        .expect("git remote add");
        assert!(
            output.status.success(),
            "git remote add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
