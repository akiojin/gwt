//! Branch operations

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, error, info, warn};

use crate::{
    error::{GwtError, Result},
    logging::{log_flow_failure, log_flow_start, log_flow_success},
};

const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
const GH_MERGE_BASE_BAD_KEY: &str = "branch..gh-merge-base";

fn run_git_output(repo_path: &Path, operation: &str, args: &[&str]) -> Result<Output> {
    crate::process::command("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: operation.to_string(),
            details: e.to_string(),
        })
}

fn run_for_each_ref_with_repair(repo_path: &Path, format_arg: &str, refs: &str) -> Result<Output> {
    let args = ["for-each-ref", format_arg, refs];
    let output = run_git_output(repo_path, "for-each-ref", &args)?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !should_attempt_gh_merge_base_repair(repo_path, &stderr) {
        return Err(GwtError::GitOperationFailed {
            operation: "for-each-ref".to_string(),
            details: stderr,
        });
    }

    warn!(
        category = "git",
        repo_path = %repo_path.display(),
        "Detected invalid gh-merge-base branch config, attempting local config repair"
    );

    match repair_bad_gh_merge_base_config(repo_path) {
        Ok(true) => {
            let retry = run_git_output(repo_path, "for-each-ref", &args)?;
            if retry.status.success() {
                return Ok(retry);
            }
            Err(GwtError::GitOperationFailed {
                operation: "for-each-ref".to_string(),
                details: String::from_utf8_lossy(&retry.stderr).to_string(),
            })
        }
        Ok(false) => Err(GwtError::GitOperationFailed {
            operation: "for-each-ref".to_string(),
            details: stderr,
        }),
        Err(repair_err) => Err(GwtError::GitOperationFailed {
            operation: "for-each-ref".to_string(),
            details: format!(
                "{}\nconfig repair failed: {}",
                stderr.trim_end(),
                repair_err
            ),
        }),
    }
}

fn should_attempt_gh_merge_base_repair(repo_path: &Path, stderr: &str) -> bool {
    let lowered = stderr.to_ascii_lowercase();
    if !lowered.contains("bad config") {
        return false;
    }
    if lowered.contains("gh-merge-base") {
        return true;
    }
    config_contains_repairable_entries(repo_path)
}

fn config_contains_repairable_entries(repo_path: &Path) -> bool {
    resolve_repo_config_paths(repo_path)
        .iter()
        .any(|config_path| {
            let Ok(contents) = std::fs::read_to_string(config_path) else {
                return false;
            };
            let (_, changed) = strip_bad_gh_merge_base_entries(&contents);
            changed
        })
}

fn repair_bad_gh_merge_base_config(repo_path: &Path) -> std::io::Result<bool> {
    let mut repaired = false;
    for config_path in resolve_repo_config_paths(repo_path) {
        let contents = std::fs::read_to_string(&config_path)?;
        let (sanitized, changed) = strip_bad_gh_merge_base_entries(&contents);
        if !changed {
            continue;
        }

        write_config_atomic(&config_path, &sanitized)?;
        repaired = true;
    }

    Ok(repaired)
}

fn resolve_repo_config_paths(repo_path: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let dot_git = repo_path.join(".git");
    if dot_git.is_dir() {
        push_existing_unique_config_path(&mut paths, dot_git.join("config"));
        return paths;
    }

    if dot_git.is_file() {
        if let Some(gitdir) = resolve_gitdir_from_dot_git_file(&dot_git) {
            if let Some(common_dir) = resolve_common_dir_from_gitdir(&gitdir) {
                push_existing_unique_config_path(&mut paths, common_dir.join("config"));
            }
            push_existing_unique_config_path(&mut paths, gitdir.join("config"));
        }
        return paths;
    }

    let bare_config = repo_path.join("config");
    push_existing_unique_config_path(&mut paths, bare_config);
    paths
}

fn push_existing_unique_config_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidate.is_file() && !paths.iter().any(|path| path == &candidate) {
        paths.push(candidate);
    }
}

fn resolve_common_dir_from_gitdir(gitdir: &Path) -> Option<PathBuf> {
    let commondir_path = gitdir.join("commondir");
    let raw = std::fs::read_to_string(commondir_path).ok()?;
    let common = raw.trim();
    if common.is_empty() {
        return None;
    }

    let resolved = if Path::new(common).is_absolute() {
        PathBuf::from(common)
    } else {
        gitdir.join(common)
    };

    Some(dunce::canonicalize(&resolved).unwrap_or(resolved))
}

fn resolve_gitdir_from_dot_git_file(dot_git_path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(dot_git_path).ok()?;
    let raw = content
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))?
        .trim();
    if raw.is_empty() {
        return None;
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Some(path)
    } else {
        dot_git_path.parent().map(|p| p.join(path))
    }
}

fn strip_bad_gh_merge_base_entries(contents: &str) -> (String, bool) {
    let mut sanitized = String::with_capacity(contents.len());
    let mut changed = false;
    let mut in_empty_branch_section = false;

    for raw_line in contents.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        let is_section_header = trimmed.starts_with('[') && trimmed.ends_with(']');

        if is_section_header {
            in_empty_branch_section = is_empty_branch_section_header(trimmed);
            if in_empty_branch_section {
                changed = true;
                continue;
            }
        } else if in_empty_branch_section {
            changed = true;
            continue;
        }

        if is_bad_gh_merge_base_key_assignment(trimmed) {
            changed = true;
            continue;
        }

        sanitized.push_str(raw_line);
    }

    if !changed {
        (contents.to_string(), false)
    } else {
        (sanitized, true)
    }
}

fn is_empty_branch_section_header(trimmed: &str) -> bool {
    let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(str::trim)
    else {
        return false;
    };

    if !inner.starts_with("branch") {
        return false;
    }

    let rest = inner["branch".len()..].trim();
    rest == "\"\"" || rest == "''"
}

fn is_bad_gh_merge_base_key_assignment(trimmed: &str) -> bool {
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
        return false;
    }

    let Some((key, _value)) = trimmed.split_once('=') else {
        return false;
    };

    key.trim() == GH_MERGE_BASE_BAD_KEY
}

fn write_config_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let tmp_path = path.with_file_name(format!("{file_name}.gwt.tmp"));

    std::fs::write(&tmp_path, contents.as_bytes())?;
    if let Err(first_rename_err) = std::fs::rename(&tmp_path, path) {
        let mut backup_contents: Option<Vec<u8>> = None;
        if path.exists() {
            backup_contents = Some(std::fs::read(path).map_err(|backup_err| {
                std::io::Error::new(
                    backup_err.kind(),
                    format!(
                        "failed to backup config before rename retry: {} (initial rename error: {})",
                        backup_err, first_rename_err
                    ),
                )
            })?);
            std::fs::remove_file(path)?;
        }

        if let Err(second_rename_err) = std::fs::rename(&tmp_path, path) {
            if let Some(backup) = backup_contents {
                let _ = std::fs::write(path, backup);
            }
            let _ = std::fs::remove_file(&tmp_path);
            return Err(std::io::Error::new(
                second_rename_err.kind(),
                format!(
                    "failed to replace config atomically: initial rename error: {}; retry rename error: {}",
                    first_rename_err, second_rename_err
                ),
            ));
        }
    }

    Ok(())
}

fn run_git_with_timeout(
    repo_path: &Path,
    operation: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Output> {
    let mut child = crate::process::command("git")
        .args(args)
        .current_dir(repo_path)
        // Avoid hanging on interactive auth prompts.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: operation.to_string(),
            details: e.to_string(),
        })?;

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle.join().unwrap_or_else(|_| Vec::new());
                let stderr = stderr_handle.join().unwrap_or_else(|_| Vec::new());
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(GwtError::GitOperationFailed {
                        operation: operation.to_string(),
                        details: format!("timeout after {}ms", timeout.as_millis()),
                    });
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(GwtError::GitOperationFailed {
                    operation: operation.to_string(),
                    details: e.to_string(),
                });
            }
        }
    }
}

/// Represents a Git branch
#[derive(Debug, Clone)]
pub struct Branch {
    /// Branch name (e.g., "main", "feature/foo")
    pub name: String,
    /// Whether this is the current branch
    pub is_current: bool,
    /// Whether this branch has a remote tracking branch
    pub has_remote: bool,
    /// Remote tracking branch name (e.g., "origin/main")
    pub upstream: Option<String>,
    /// Commit SHA
    pub commit: String,
    /// Commits ahead of upstream
    pub ahead: usize,
    /// Commits behind upstream
    pub behind: usize,
    /// Last commit timestamp (Unix timestamp in seconds) - FR-041
    pub commit_timestamp: Option<i64>,
    /// Whether the upstream branch has been deleted (gone) - FR-085
    pub is_gone: bool,
}

impl Branch {
    fn delete_with_flag(repo_path: &Path, name: &str, flag: &str) -> Result<Output> {
        crate::process::command("git")
            .args(["branch", flag, name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "branch delete".to_string(),
                details: e.to_string(),
            })
    }

    fn merge_target_for_safe_delete(repo_path: &Path, name: &str) -> String {
        Self::list_basic(repo_path)
            .ok()
            .and_then(|branches| {
                branches
                    .into_iter()
                    .find(|branch| branch.name == name)
                    .and_then(|branch| branch.upstream)
            })
            .unwrap_or_else(|| "HEAD".to_string())
    }

    fn should_force_delete_after_safe_delete_failure(repo_path: &Path, name: &str) -> bool {
        if let Ok(Some(current)) = Self::current(repo_path) {
            if current.name == name {
                return false;
            }
        }

        if let Ok(false) = Self::exists(repo_path, name) {
            return false;
        }

        let target = Self::merge_target_for_safe_delete(repo_path, name);

        matches!(Self::is_merged_into(repo_path, name, &target), Ok(false))
    }

    /// Create a new branch instance
    pub fn new(name: impl Into<String>, commit: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_current: false,
            has_remote: false,
            upstream: None,
            commit: commit.into(),
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        }
    }

    /// List all local branches in a repository
    pub fn list(repo_path: &Path) -> Result<Vec<Branch>> {
        Self::list_with_options(repo_path, true)
    }

    /// List all local branches without computing divergence (fast path)
    pub fn list_basic(repo_path: &Path) -> Result<Vec<Branch>> {
        Self::list_with_options(repo_path, false)
    }

    fn list_with_options(repo_path: &Path, include_divergence: bool) -> Result<Vec<Branch>> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            include_divergence,
            "Listing branches"
        );

        let output = run_for_each_ref_with_repair(
            repo_path,
            "--format=%(refname:short)%09%(objectname:short)%09%(upstream:short)%09%(HEAD)%09%(committerdate:unix)%09%(upstream:track)",
            "refs/heads/",
        )?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut branches = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 5 {
                let name = parts[0].to_string();
                let commit = parts[1].to_string();
                let upstream = if parts[2].is_empty() {
                    None
                } else {
                    Some(parts[2].to_string())
                };
                let is_current = parts[3] == "*";
                let commit_timestamp = parts[4].parse::<i64>().ok();
                // FR-085: Detect gone status from upstream:track
                let track_info = parts.get(5).unwrap_or(&"");
                let is_gone = track_info.contains("[gone]");

                let mut branch = Branch {
                    name,
                    is_current,
                    has_remote: upstream.is_some(),
                    upstream: upstream.clone(),
                    commit,
                    ahead: 0,
                    behind: 0,
                    commit_timestamp,
                    is_gone,
                };

                if include_divergence {
                    // Get ahead/behind counts if upstream exists and not gone
                    if let Some(ref up) = upstream {
                        if !is_gone {
                            if let Ok((ahead, behind)) =
                                Self::get_divergence(repo_path, &branch.name, up)
                            {
                                branch.ahead = ahead;
                                branch.behind = behind;
                            }
                        }
                    }
                }

                branches.push(branch);
            }
        }

        Ok(branches)
    }

    /// List all remote branches
    pub fn list_remote(repo_path: &Path) -> Result<Vec<Branch>> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            "Listing remote branches"
        );

        let output = run_for_each_ref_with_repair(
            repo_path,
            "--format=%(refname)%09%(refname:short)%09%(objectname:short)%09%(committerdate:unix)",
            "refs/remotes/",
        )?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut branches = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                let full_ref = parts[0];
                let short_ref = parts[1];
                // Skip symbolic remote HEAD aliases like refs/remotes/origin/HEAD.
                if full_ref.ends_with("/HEAD") {
                    continue;
                }
                let commit_timestamp = parts[3].parse::<i64>().ok();
                branches.push(Branch {
                    name: short_ref.to_string(),
                    is_current: false,
                    has_remote: true,
                    upstream: None,
                    commit: parts[2].to_string(),
                    ahead: 0,
                    behind: 0,
                    commit_timestamp,
                    is_gone: false,
                });
            }
        }

        Ok(branches)
    }

    /// List remote branches using ls-remote (for bare repositories)
    /// gwt-spec issue: Bare repositories don't have refs/remotes/, so we use ls-remote
    pub fn list_remote_from_origin(repo_path: &Path) -> Result<Vec<Branch>> {
        Self::list_remote_from_remote(repo_path, "origin")
    }

    /// List remote branches using ls-remote for the given remote name.
    pub fn list_remote_from_remote(repo_path: &Path, remote: &str) -> Result<Vec<Branch>> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            remote,
            "Listing remote branches via ls-remote"
        );

        let output = run_git_with_timeout(
            repo_path,
            "ls-remote",
            &["ls-remote", "--heads", remote],
            LS_REMOTE_TIMEOUT,
        )?;

        if !output.status.success() {
            // If origin doesn't exist, return empty list
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("No such remote")
                || stderr.contains("does not appear to be a git repository")
            {
                debug!(category = "git", remote, "Remote not configured");
                return Ok(Vec::new());
            }
            return Err(GwtError::GitOperationFailed {
                operation: "ls-remote".to_string(),
                details: stderr.to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut branches = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let commit = &parts[0][..7.min(parts[0].len())]; // Short SHA
                let ref_name = parts[1];
                // Convert refs/heads/branch-name to <remote>/branch-name
                if let Some(branch_name) = ref_name.strip_prefix("refs/heads/") {
                    branches.push(Branch {
                        name: format!("{}/{}", remote, branch_name),
                        is_current: false,
                        has_remote: true,
                        upstream: None,
                        commit: commit.to_string(),
                        ahead: 0,
                        behind: 0,
                        commit_timestamp: None, // ls-remote doesn't provide timestamp
                        is_gone: false,
                    });
                }
            }
        }

        debug!(
            category = "git",
            count = branches.len(),
            "Found remote branches via ls-remote"
        );

        Ok(branches)
    }

    /// List remote branches, supplementing local remote-tracking refs with ls-remote results.
    pub fn list_remote_complete(repo_path: &Path, remote: &str) -> Result<Vec<Branch>> {
        let mut refs = Self::list_remote(repo_path)?;
        let prefix = format!("{}/", remote);
        refs.retain(|b| b.name.starts_with(&prefix));

        let mut map: HashMap<String, Branch> =
            refs.into_iter().map(|b| (b.name.clone(), b)).collect();

        let remote_heads = Self::list_remote_from_remote(repo_path, remote)?;
        for branch in remote_heads {
            map.entry(branch.name.clone()).or_insert(branch);
        }

        Ok(map.into_values().collect())
    }

    /// Get the current branch
    pub fn current(repo_path: &Path) -> Result<Option<Branch>> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            "Getting current branch"
        );

        let output = crate::process::command("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Ok(None);
        }

        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name == "HEAD" {
            return Ok(None); // Detached HEAD
        }

        // Get commit
        let commit_output = crate::process::command("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        let commit = String::from_utf8_lossy(&commit_output.stdout)
            .trim()
            .to_string();

        // Get commit timestamp (FR-041)
        let timestamp_output = crate::process::command("git")
            .args(["log", "-1", "--format=%ct", "HEAD"])
            .current_dir(repo_path)
            .output();

        let commit_timestamp = timestamp_output.ok().and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<i64>()
                    .ok()
            } else {
                None
            }
        });

        // Get upstream
        let upstream_output = crate::process::command("git")
            .args(["rev-parse", "--abbrev-ref", "@{u}"])
            .current_dir(repo_path)
            .output();

        let upstream = upstream_output.ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

        let mut branch = Branch {
            name: name.clone(),
            is_current: true,
            has_remote: upstream.is_some(),
            upstream: upstream.clone(),
            commit,
            ahead: 0,
            behind: 0,
            commit_timestamp,
            is_gone: false, // Current branch cannot be gone
        };

        // Get ahead/behind
        if let Some(ref up) = upstream {
            if let Ok((ahead, behind)) = Self::get_divergence(repo_path, &name, up) {
                branch.ahead = ahead;
                branch.behind = behind;
            }
        }

        Ok(Some(branch))
    }

    /// Create a new branch from a base
    pub fn create(repo_path: &Path, name: &str, base: &str) -> Result<Branch> {
        log_flow_start("git", "create_branch");
        debug!(category = "git", branch = name, base, "Creating branch");

        let output = crate::process::command("git")
            .args(["branch", name, base])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "branch create".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                branch = name,
                base,
                error = err_msg.as_str(),
                "Failed to create branch"
            );
            log_flow_failure("git", "create_branch", &err_msg);
            return Err(GwtError::BranchCreateFailed {
                name: name.to_string(),
                details: err_msg,
            });
        }

        // Get commit of new branch
        let commit_output = crate::process::command("git")
            .args(["rev-parse", "--short", name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        let commit = String::from_utf8_lossy(&commit_output.stdout)
            .trim()
            .to_string();

        info!(
            category = "git",
            operation = "branch_create",
            branch = name,
            base,
            commit = commit.as_str(),
            "Branch created"
        );

        log_flow_success("git", "create_branch");
        Ok(Branch::new(name, commit))
    }

    /// Set upstream tracking configuration for a branch (gwt-spec issue FR-001)
    ///
    /// Runs `git config branch.<name>.remote <remote>` and
    /// `git config branch.<name>.merge refs/heads/<name>`.
    /// This is a local config-only operation (no network required).
    pub fn set_upstream_config(repo_path: &Path, branch_name: &str, remote: &str) -> Result<()> {
        debug!(
            category = "git",
            branch = branch_name,
            remote,
            "Setting upstream config"
        );

        let remote_key = format!("branch.{}.remote", branch_name);
        let output = crate::process::command("git")
            .args(["config", &remote_key, remote])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "config".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "config".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let merge_key = format!("branch.{}.merge", branch_name);
        let merge_value = format!("refs/heads/{}", branch_name);
        let output = crate::process::command("git")
            .args(["config", &merge_key, &merge_value])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "config".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "config".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        info!(
            category = "git",
            operation = "set_upstream_config",
            branch = branch_name,
            remote,
            "Upstream config set"
        );

        Ok(())
    }

    /// Delete a branch
    pub fn delete(repo_path: &Path, name: &str, force: bool) -> Result<()> {
        log_flow_start("git", "delete_branch");
        debug!(category = "git", branch = name, force, "Deleting branch");

        let flag = if force { "-D" } else { "-d" };
        let output = Self::delete_with_flag(repo_path, name, flag)?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "branch_delete",
                branch = name,
                force,
                "Branch deleted"
            );
            log_flow_success("git", "delete_branch");
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();

            if !force && Self::should_force_delete_after_safe_delete_failure(repo_path, name) {
                warn!(
                    category = "git",
                    branch = name,
                    error = err_msg.as_str(),
                    "Branch delete with -d failed and branch is not merged, retrying with -D"
                );

                let forced_output = Self::delete_with_flag(repo_path, name, "-D")?;
                if forced_output.status.success() {
                    info!(
                        category = "git",
                        operation = "branch_delete",
                        branch = name,
                        force = true,
                        fallback = true,
                        "Branch deleted via automatic force fallback"
                    );
                    log_flow_success("git", "delete_branch");
                    return Ok(());
                }

                let forced_err = String::from_utf8_lossy(&forced_output.stderr).to_string();
                let combined_err = format!(
                    "branch -d failed: {}\nbranch -D fallback failed: {}",
                    err_msg, forced_err
                );
                error!(
                    category = "git",
                    branch = name,
                    force,
                    error = combined_err.as_str(),
                    "Failed to delete branch after fallback"
                );
                log_flow_failure("git", "delete_branch", &combined_err);
                return Err(GwtError::BranchDeleteFailed {
                    name: name.to_string(),
                    details: combined_err,
                });
            }

            error!(
                category = "git",
                branch = name,
                force,
                error = err_msg.as_str(),
                "Failed to delete branch"
            );
            log_flow_failure("git", "delete_branch", &err_msg);
            Err(GwtError::BranchDeleteFailed {
                name: name.to_string(),
                details: err_msg,
            })
        }
    }

    /// Get divergence (ahead, behind) between branch and upstream
    fn get_divergence(repo_path: &Path, branch: &str, upstream: &str) -> Result<(usize, usize)> {
        let output = crate::process::command("git")
            .args([
                "rev-list",
                "--left-right",
                "--count",
                &format!("{branch}...{upstream}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-list".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Ok((0, 0));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split('\t').collect();
        if parts.len() == 2 {
            let ahead = parts[0].parse().unwrap_or(0);
            let behind = parts[1].parse().unwrap_or(0);
            Ok((ahead, behind))
        } else {
            Ok((0, 0))
        }
    }

    /// Get divergence (ahead, behind) between two refs
    pub fn divergence_between(repo_path: &Path, left: &str, right: &str) -> Result<(usize, usize)> {
        let output = crate::process::command("git")
            .args([
                "rev-list",
                "--left-right",
                "--count",
                &format!("{left}...{right}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-list".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "rev-list".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split('\t').collect();
        if parts.len() == 2 {
            let ahead = parts[0].parse().unwrap_or(0);
            let behind = parts[1].parse().unwrap_or(0);
            Ok((ahead, behind))
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "rev-list parse".to_string(),
                details: stdout.to_string(),
            })
        }
    }

    /// Get the divergence status from remote
    pub fn divergence_status(&self) -> DivergenceStatus {
        if !self.has_remote {
            return DivergenceStatus::NoRemote;
        }

        match (self.ahead, self.behind) {
            (0, 0) => DivergenceStatus::UpToDate,
            (a, 0) => DivergenceStatus::Ahead(a),
            (0, b) => DivergenceStatus::Behind(b),
            (a, b) => DivergenceStatus::Diverged {
                ahead: a,
                behind: b,
            },
        }
    }

    /// Check if a branch exists locally
    pub fn exists(repo_path: &Path, name: &str) -> Result<bool> {
        let output = crate::process::command("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{name}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "show-ref".to_string(),
                details: e.to_string(),
            })?;

        let exists = output.status.success();
        debug!(
            category = "git",
            branch = name,
            exists,
            "Checked branch existence"
        );
        Ok(exists)
    }

    /// Check if a branch exists remotely
    pub fn remote_exists(repo_path: &Path, remote: &str, branch: &str) -> Result<bool> {
        // First try local refs/remotes (works for normal repos)
        let output = crate::process::command("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{remote}/{branch}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "show-ref".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            return Ok(true);
        }

        // gwt-spec issue FR-124: For bare repos, check via ls-remote
        // gwt-spec issue FR-001: Use run_git_with_timeout for GIT_TERMINAL_PROMPT=0 and timeout
        let ls_output = match run_git_with_timeout(
            repo_path,
            "ls-remote",
            &["ls-remote", "--heads", remote, branch],
            LS_REMOTE_TIMEOUT,
        ) {
            Ok(output) => output,
            Err(_) => return Ok(false),
        };

        if ls_output.status.success() {
            let stdout = String::from_utf8_lossy(&ls_output.stdout);
            // ls-remote returns lines like: <sha>\trefs/heads/<branch>
            return Ok(stdout.lines().any(|line| {
                line.split('\t')
                    .nth(1)
                    .is_some_and(|r| r == format!("refs/heads/{}", branch))
            }));
        }

        Ok(false)
    }

    /// Checkout this branch
    pub fn checkout(repo_path: &Path, name: &str) -> Result<()> {
        debug!(category = "git", branch = name, "Checking out branch");

        let output = crate::process::command("git")
            .args(["checkout", name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "checkout".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "checkout",
                branch = name,
                "Branch checked out"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                branch = name,
                error = err_msg.as_str(),
                "Failed to checkout branch"
            );
            Err(GwtError::GitOperationFailed {
                operation: format!("checkout {name}"),
                details: err_msg,
            })
        }
    }

    /// Check if a branch is merged into the base branch
    ///
    /// Uses `git merge-base --is-ancestor` to check if the branch commit
    /// is an ancestor of the base branch (i.e., all commits are included).
    ///
    /// Note: This works for regular merges and fast-forward merges.
    /// For squash merges, the original branch commits are not ancestors
    /// of the base branch, so this will return false even if the changes
    /// were squash-merged.
    pub fn is_merged_into(repo_path: &Path, branch: &str, base: &str) -> Result<bool> {
        debug!(
            category = "git",
            branch = branch,
            base = base,
            "Checking if branch is merged into base"
        );

        let output = crate::process::command("git")
            .args(["merge-base", "--is-ancestor", branch, base])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "merge-base".to_string(),
                details: e.to_string(),
            })?;

        // Exit code 0 means branch is an ancestor (merged)
        // Exit code 1 means branch is not an ancestor (not merged)
        // Other exit codes indicate errors
        let is_merged = match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            code => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(GwtError::GitOperationFailed {
                    operation: "merge-base".to_string(),
                    details: format!("exit code {:?}: {}", code, stderr),
                });
            }
        };

        debug!(
            category = "git",
            branch = branch,
            base = base,
            is_merged,
            "Merge check completed"
        );

        Ok(is_merged)
    }
}

/// Branch divergence status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// Branch is up to date with remote
    UpToDate,
    /// Branch is ahead of remote
    Ahead(usize),
    /// Branch is behind remote
    Behind(usize),
    /// Branch has diverged from remote
    Diverged { ahead: usize, behind: usize },
    /// No remote tracking branch
    NoRemote,
}

impl std::fmt::Display for DivergenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpToDate => write!(f, "up to date"),
            Self::Ahead(n) => write!(f, "{n} ahead"),
            Self::Behind(n) => write!(f, "{n} behind"),
            Self::Diverged { ahead, behind } => write!(f, "{ahead} ahead, {behind} behind"),
            Self::NoRemote => write!(f, "no remote"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;

    use super::*;

    fn run_git(repo_path: &Path, args: &[&str]) {
        let output = crate::process::command("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "test@test.com"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        // Create initial commit
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        temp
    }

    fn commit_file(repo_path: &Path, filename: &str, content: &str, message: &str) {
        std::fs::write(repo_path.join(filename), content).unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(repo_path, &["commit", "-m", message]);
    }

    fn append_to_local_config(repo_path: &Path, content: &str) {
        let config_path = repo_path.join(".git").join("config");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&config_path)
            .unwrap();
        writeln!(file, "\n{content}").unwrap();
    }

    fn canonicalize_or_self(path: &Path) -> PathBuf {
        dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn create_repo_with_remote() -> (TempDir, String) {
        let temp = create_test_repo();
        let origin = TempDir::new().unwrap();

        run_git(origin.path(), &["init", "--bare"]);

        let branch = Branch::current(temp.path()).unwrap().unwrap().name;

        run_git(
            temp.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );

        run_git(temp.path(), &["push", "-u", "origin", &branch]);

        std::fs::write(temp.path().join("ahead.txt"), "ahead").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "ahead"]);

        (temp, branch)
    }

    #[test]
    fn test_list_branches() {
        let temp = create_test_repo();
        let branches = Branch::list(temp.path()).unwrap();
        assert_eq!(branches.len(), 1);
        assert!(branches[0].is_current);
    }

    #[test]
    fn test_list_branches_recovers_from_bad_gh_merge_base_config() {
        let temp = create_test_repo();
        append_to_local_config(temp.path(), "branch..gh-merge-base = origin/main");

        let branches = Branch::list(temp.path()).unwrap();
        assert!(!branches.is_empty());

        let config = std::fs::read_to_string(temp.path().join(".git").join("config")).unwrap();
        assert!(!config.contains("branch..gh-merge-base"));
    }

    #[test]
    fn test_list_remote_recovers_from_empty_branch_section_config() {
        let temp = create_test_repo();
        append_to_local_config(
            temp.path(),
            "branch..gh-merge-base = origin/main\n[branch \"\"]\n\tgh-merge-base = origin/main",
        );

        let _ = Branch::list_remote(temp.path()).unwrap();

        let config = std::fs::read_to_string(temp.path().join(".git").join("config")).unwrap();
        assert!(!config.contains("[branch \"\"]"));
        assert!(!config.contains("gh-merge-base"));
    }

    #[test]
    fn test_resolve_repo_config_paths_prefers_common_config_for_linked_worktree() {
        let repo = create_test_repo();
        let base = Branch::current(repo.path()).unwrap().unwrap().name;
        let wt_parent = TempDir::new().unwrap();
        let wt_path = wt_parent.path().join("linked-worktree");
        run_git(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                "feature/linked-config",
                wt_path.to_str().unwrap(),
                &base,
            ],
        );

        let paths = resolve_repo_config_paths(&wt_path);
        let shared_config = canonicalize_or_self(&repo.path().join(".git").join("config"));
        assert_eq!(paths.first(), Some(&shared_config));
    }

    #[test]
    fn test_list_branches_repairs_shared_config_from_linked_worktree_path() {
        let repo = create_test_repo();
        let base = Branch::current(repo.path()).unwrap().unwrap().name;
        let wt_parent = TempDir::new().unwrap();
        let wt_path = wt_parent.path().join("repair-worktree");
        run_git(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                "feature/repair-config",
                wt_path.to_str().unwrap(),
                &base,
            ],
        );
        append_to_local_config(repo.path(), "branch..gh-merge-base = origin/main");

        let branches = Branch::list(&wt_path).unwrap();
        assert!(!branches.is_empty());

        let shared_config =
            std::fs::read_to_string(repo.path().join(".git").join("config")).unwrap();
        assert!(!shared_config.contains("branch..gh-merge-base"));
    }

    #[test]
    fn test_list_basic_skips_divergence() {
        let (temp, branch) = create_repo_with_remote();

        let branches_full = Branch::list(temp.path()).unwrap();
        let branch_full = branches_full.iter().find(|b| b.name == branch).unwrap();
        assert!(branch_full.ahead > 0);

        let branches_basic = Branch::list_basic(temp.path()).unwrap();
        let branch_basic = branches_basic.iter().find(|b| b.name == branch).unwrap();
        assert_eq!(branch_basic.ahead, 0);
        assert_eq!(branch_basic.behind, 0);
    }

    #[test]
    fn test_list_remote_from_origin_uses_origin_prefix() {
        // Keep the origin TempDir alive so `git ls-remote origin` can read from it.
        let temp = create_test_repo();
        let origin = TempDir::new().unwrap();

        crate::process::command("git")
            .args(["init", "--bare"])
            .current_dir(origin.path())
            .output()
            .unwrap();

        let branch = Branch::current(temp.path()).unwrap().unwrap().name;

        crate::process::command("git")
            .args(["remote", "add", "origin", origin.path().to_str().unwrap()])
            .current_dir(temp.path())
            .output()
            .unwrap();

        crate::process::command("git")
            .args(["push", "-u", "origin", &branch])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let remotes = Branch::list_remote_from_origin(temp.path()).unwrap();
        let expected = format!("origin/{}", branch);

        assert!(
            remotes.iter().any(|b| b.name == expected),
            "Expected {expected} in list_remote_from_origin result"
        );
        // ls-remote doesn't provide committer timestamp
        assert!(remotes.iter().all(|b| b.commit_timestamp.is_none()));
    }

    #[test]
    fn test_list_remote_skips_remote_head_alias_entry() {
        let temp = create_test_repo();
        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare"]);

        let branch = Branch::current(temp.path()).unwrap().unwrap().name;
        run_git(
            temp.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        run_git(temp.path(), &["push", "-u", "origin", &branch]);
        run_git(temp.path(), &["remote", "set-head", "origin", "-a"]);

        let remotes = Branch::list_remote(temp.path()).unwrap();

        assert!(
            remotes.iter().any(|b| b.name == format!("origin/{branch}")),
            "expected real remote branch to be present"
        );
        assert!(
            !remotes.iter().any(|b| b.name == "origin"),
            "remote HEAD alias should not be surfaced as a branch: {:?}",
            remotes.iter().map(|b| b.name.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_current_branch() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap();
        assert!(current.is_some());
        let branch = current.unwrap();
        assert!(branch.is_current);
        // Could be main or master depending on git version
        assert!(branch.name == "main" || branch.name == "master");
    }

    #[test]
    fn test_create_branch() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();
        let branch = Branch::create(temp.path(), "feature/test", &current.name).unwrap();
        assert_eq!(branch.name, "feature/test");
        assert!(Branch::exists(temp.path(), "feature/test").unwrap());
    }

    #[test]
    fn test_delete_branch() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();
        Branch::create(temp.path(), "feature/test", &current.name).unwrap();
        assert!(Branch::exists(temp.path(), "feature/test").unwrap());
        Branch::delete(temp.path(), "feature/test", false).unwrap();
        assert!(!Branch::exists(temp.path(), "feature/test").unwrap());
    }

    #[test]
    fn test_delete_branch_auto_forces_unmerged_when_not_fully_merged() {
        let temp = create_test_repo();
        let base = Branch::current(temp.path()).unwrap().unwrap().name;

        run_git(temp.path(), &["checkout", "-b", "feature/unmerged"]);
        commit_file(temp.path(), "feature.txt", "feature", "feature commit");
        run_git(temp.path(), &["checkout", &base]);

        assert!(Branch::exists(temp.path(), "feature/unmerged").unwrap());
        Branch::delete(temp.path(), "feature/unmerged", false).unwrap();
        assert!(!Branch::exists(temp.path(), "feature/unmerged").unwrap());
    }

    #[test]
    fn test_delete_current_branch_still_fails_without_force() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();

        let result = Branch::delete(temp.path(), &current.name, false);
        assert!(result.is_err());
        assert!(Branch::exists(temp.path(), &current.name).unwrap());
    }

    #[test]
    fn test_divergence_between() {
        let temp = create_test_repo();
        let base = Branch::current(temp.path()).unwrap().unwrap().name;

        crate::process::command("git")
            .args(["checkout", "-b", "feature/test"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        std::fs::write(temp.path().join("feature.txt"), "feature").unwrap();
        crate::process::command("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["commit", "-m", "feature"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let (ahead, behind) =
            Branch::divergence_between(temp.path(), "feature/test", &base).unwrap();
        assert!(ahead > 0);
        assert_eq!(behind, 0);
    }

    #[test]
    fn test_divergence_status() {
        let branch = Branch {
            name: "main".to_string(),
            is_current: true,
            has_remote: true,
            upstream: Some("origin/main".to_string()),
            commit: "abc123".to_string(),
            ahead: 2,
            behind: 1,
            commit_timestamp: None,
            is_gone: false,
        };
        assert_eq!(
            branch.divergence_status(),
            DivergenceStatus::Diverged {
                ahead: 2,
                behind: 1
            }
        );
    }

    #[test]
    fn test_is_merged_into_true() {
        let temp = create_test_repo();
        let base = Branch::current(temp.path()).unwrap().unwrap().name;

        crate::process::command("git")
            .args(["checkout", "-b", "feature/merged"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        commit_file(temp.path(), "merged.txt", "merged", "merged commit");

        crate::process::command("git")
            .args(["checkout", &base])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let output = crate::process::command("git")
            .args(["merge", "--ff-only", "feature/merged"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success());

        let merged = Branch::is_merged_into(temp.path(), "feature/merged", &base).unwrap();
        assert!(merged);
    }

    #[test]
    fn test_is_merged_into_false() {
        let temp = create_test_repo();
        let base = Branch::current(temp.path()).unwrap().unwrap().name;

        crate::process::command("git")
            .args(["checkout", "-b", "feature/unmerged"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        commit_file(temp.path(), "unmerged.txt", "unmerged", "unmerged commit");

        crate::process::command("git")
            .args(["checkout", &base])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let merged = Branch::is_merged_into(temp.path(), "feature/unmerged", &base).unwrap();
        assert!(!merged);
    }

    #[test]
    fn test_is_merged_into_invalid_ref() {
        let temp = create_test_repo();
        let base = Branch::current(temp.path()).unwrap().unwrap().name;

        let result = Branch::is_merged_into(temp.path(), "no-such-branch", &base);
        assert!(result.is_err());
    }

    // T1402: Test gone detection from upstream:track
    #[test]
    fn test_gone_branch_detection() {
        // Create a repo with a remote
        let temp = create_test_repo();
        let origin = TempDir::new().unwrap();

        crate::process::command("git")
            .args(["init", "--bare"])
            .current_dir(origin.path())
            .output()
            .unwrap();

        let branch = Branch::current(temp.path()).unwrap().unwrap().name;

        crate::process::command("git")
            .args(["remote", "add", "origin", origin.path().to_str().unwrap()])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Create a feature branch and push it
        crate::process::command("git")
            .args(["checkout", "-b", "feature/will-be-gone"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        commit_file(temp.path(), "gone.txt", "gone", "gone commit");

        crate::process::command("git")
            .args(["push", "-u", "origin", "feature/will-be-gone"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Delete the remote branch directly from the bare repo
        crate::process::command("git")
            .args(["branch", "-D", "feature/will-be-gone"])
            .current_dir(origin.path())
            .output()
            .unwrap();

        // Checkout back to main/master so we can test the feature branch
        crate::process::command("git")
            .args(["checkout", &branch])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Fetch with prune to update tracking info
        crate::process::command("git")
            .args(["fetch", "--prune"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // List branches and check if the gone branch is detected
        let branches = Branch::list(temp.path()).unwrap();
        let gone_branch = branches.iter().find(|b| b.name == "feature/will-be-gone");

        assert!(gone_branch.is_some(), "Feature branch should exist locally");
        let gone_branch = gone_branch.unwrap();
        assert!(
            gone_branch.is_gone,
            "Branch should be marked as gone after remote deletion"
        );
    }

    // gwt-spec issue: Test set_upstream_config sets remote and merge
    #[test]
    fn test_set_upstream_config_sets_remote_and_merge() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();
        Branch::create(temp.path(), "feature/upstream-test", &current.name).unwrap();

        Branch::set_upstream_config(temp.path(), "feature/upstream-test", "origin").unwrap();

        let remote_output = crate::process::command("git")
            .args(["config", "branch.feature/upstream-test.remote"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&remote_output.stdout).trim(),
            "origin"
        );

        let merge_output = crate::process::command("git")
            .args(["config", "branch.feature/upstream-test.merge"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&merge_output.stdout).trim(),
            "refs/heads/feature/upstream-test"
        );
    }

    // gwt-spec issue: Test set_upstream_config works without remote ref existing
    #[test]
    fn test_set_upstream_config_no_remote_ref_needed() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();
        Branch::create(temp.path(), "feature/no-remote-ref", &current.name).unwrap();

        // No remote added — config should still succeed (git config doesn't validate)
        let result = Branch::set_upstream_config(temp.path(), "feature/no-remote-ref", "origin");
        assert!(result.is_ok());
    }

    #[test]
    fn test_non_gone_branch() {
        let (temp, branch) = create_repo_with_remote();

        let branches = Branch::list(temp.path()).unwrap();
        let main_branch = branches.iter().find(|b| b.name == branch).unwrap();

        assert!(
            !main_branch.is_gone,
            "Branch with existing remote should not be marked as gone"
        );
    }

    #[test]
    fn test_list_remote_complete_includes_ls_remote_branches_when_remote_refs_missing() {
        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare"]);

        // Base repo with only one remote-tracking ref configured.
        let repo = create_test_repo();
        let base_branch = Branch::current(repo.path()).unwrap().unwrap().name;
        run_git(
            repo.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        run_git(repo.path(), &["push", "-u", "origin", &base_branch]);

        // Fetch only the base branch so refs/remotes/origin/* is intentionally incomplete.
        let refspec = format!(
            "+refs/heads/{}:refs/remotes/origin/{}",
            base_branch, base_branch
        );
        run_git(repo.path(), &["config", "remote.origin.fetch", &refspec]);
        run_git(repo.path(), &["fetch", "origin"]);

        // Push a new branch to origin from a different repo so it does not exist locally.
        let pusher = create_test_repo();
        run_git(
            pusher.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        run_git(pusher.path(), &["checkout", "-b", "feature/missing"]);
        commit_file(pusher.path(), "missing.txt", "missing", "missing");
        run_git(pusher.path(), &["push", "-u", "origin", "feature/missing"]);

        // Remote-tracking refs are missing, but list_remote_complete should still include it via ls-remote.
        let remote_refs = Branch::list_remote(repo.path()).unwrap();
        assert!(
            !remote_refs
                .iter()
                .any(|b| b.name == "origin/feature/missing"),
            "refs/remotes should not include the missing branch in this setup"
        );

        let complete = Branch::list_remote_complete(repo.path(), "origin").unwrap();
        assert!(
            complete.iter().any(|b| b.name == "origin/feature/missing"),
            "list_remote_complete should include missing remote branches via ls-remote"
        );
    }
}
