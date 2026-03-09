use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
const GH_FALLBACK_PATHS: &[&str] = &[
    r"C:\Program Files\GitHub CLI\gh.exe",
    r"C:\Program Files (x86)\GitHub CLI\gh.exe",
];

#[cfg(not(target_os = "windows"))]
const GH_FALLBACK_PATHS: &[&str] = &[
    "/opt/homebrew/bin/gh",
    "/usr/local/bin/gh",
    "/opt/local/bin/gh",
    "/usr/bin/gh",
];
const BROKEN_GH_MERGE_BASE_KEY: &str = "branch..gh-merge-base";

/// Sentinel prefix for repository-rule-protected branch deletion errors.
/// Used by `classify_delete_branch_error` (producer) and `cleanup.rs` (consumer).
pub const PROTECTED_BRANCH_PREFIX: &str = "Protected:";

/// PR status for cleanup safety judgment (SPEC-ad1ac432)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrStatus {
    Merged,
    Open,
    Closed,
    None,
    Unknown,
}

fn fallback_gh_path() -> Option<PathBuf> {
    GH_FALLBACK_PATHS
        .iter()
        .map(Path::new)
        .find(|path| path.exists())
        .map(PathBuf::from)
}

pub fn resolve_gh_path() -> Option<PathBuf> {
    which::which("gh").ok().or_else(fallback_gh_path)
}

pub fn gh_command() -> std::process::Command {
    match resolve_gh_path() {
        Some(path) => {
            let program = path.to_string_lossy().into_owned();
            crate::process::command(&program)
        }
        None => crate::process::command("gh"),
    }
}

/// Run a gh command in a repository context, auto-repairing known broken git
/// config (`branch..gh-merge-base`) once and retrying exactly once.
pub fn run_gh_output_with_repair<I, S>(repo_path: &Path, args: I) -> std::io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = collect_gh_args(args);
    run_gh_output_with_repair_once(repo_path, &args)
}

/// Run a gh command with timeout in a repository context, auto-repairing known
/// broken git config (`branch..gh-merge-base`) once and retrying exactly once.
pub fn run_gh_output_with_timeout_and_repair<I, S>(
    repo_path: &Path,
    args: I,
    timeout: Duration,
) -> std::io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = collect_gh_args(args);
    let output = run_gh_output_with_timeout(repo_path, &args, timeout)?;
    if !should_attempt_bad_config_repair(&output) {
        return Ok(output);
    }

    match repair_bad_gh_merge_base_config(repo_path) {
        Ok(true) => run_gh_output_with_timeout(repo_path, &args, timeout),
        Ok(false) => Ok(output),
        Err(err) => Err(std::io::Error::other(format!(
            "Failed to auto-repair broken git config for gh: {}",
            err
        ))),
    }
}

fn collect_gh_args<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect()
}

fn run_gh_output_with_repair_once(repo_path: &Path, args: &[String]) -> std::io::Result<Output> {
    let output = run_gh_output(repo_path, args)?;
    if !should_attempt_bad_config_repair(&output) {
        return Ok(output);
    }

    match repair_bad_gh_merge_base_config(repo_path) {
        Ok(true) => run_gh_output(repo_path, args),
        Ok(false) => Ok(output),
        Err(err) => Err(std::io::Error::other(format!(
            "Failed to auto-repair broken git config for gh: {}",
            err
        ))),
    }
}

fn run_gh_output(repo_path: &Path, args: &[String]) -> std::io::Result<Output> {
    gh_command().args(args).current_dir(repo_path).output()
}

fn run_gh_output_with_timeout(
    repo_path: &Path,
    args: &[String],
    timeout: Duration,
) -> std::io::Result<Output> {
    let mut child = gh_command()
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout_handle = spawn_pipe_reader(child.stdout.take());
    let stderr_handle = spawn_pipe_reader(child.stderr.take());

    let status = match wait_with_timeout(&mut child, timeout) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "gh {} timed out after {}s",
                    args.join(" "),
                    timeout.as_secs()
                ),
            ));
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_pipe_reader<T>(pipe: Option<T>) -> thread::JoinHandle<Vec<u8>>
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let Some(mut pipe) = pipe else {
            return Vec::new();
        };
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

fn should_attempt_bad_config_repair(output: &Output) -> bool {
    if output.status.success() {
        return false;
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let combined = format!("{stderr}\n{stdout}");
    combined.contains("bad config variable") && combined.contains(BROKEN_GH_MERGE_BASE_KEY)
}

fn repair_bad_gh_merge_base_config(repo_path: &Path) -> Result<bool, String> {
    let common_dir = resolve_git_common_dir(repo_path)?;
    let config_path = common_dir.join("config");
    let original = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {}: {}", config_path.display(), e))?;
    let repaired = sanitize_bad_gh_merge_base_config(&original);
    if repaired == original {
        return Ok(false);
    }

    write_text_file_atomically(&config_path, &repaired)?;
    Ok(true)
}

fn resolve_git_common_dir(repo_path: &Path) -> Result<PathBuf, String> {
    let output = crate::process::command("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git rev-parse --git-common-dir: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git rev-parse --git-common-dir failed: {}",
            stderr.trim()
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Err("git rev-parse --git-common-dir returned an empty path".to_string());
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(repo_path.join(path))
    }
}

fn sanitize_bad_gh_merge_base_config(content: &str) -> String {
    let mut kept_lines = Vec::new();
    let mut skipping_empty_branch_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if is_git_config_section_header(trimmed) {
            skipping_empty_branch_section = is_empty_branch_section_header(trimmed);
            if skipping_empty_branch_section {
                continue;
            }
        }

        if skipping_empty_branch_section || is_broken_merge_base_key_line(trimmed) {
            continue;
        }

        kept_lines.push(line);
    }

    let mut result = kept_lines.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn config_section_header_inner(trimmed: &str) -> Option<&str> {
    if !trimmed.starts_with('[') {
        return None;
    }

    let close_idx = trimmed.find(']')?;
    let inner = trimmed[1..close_idx].trim();
    if inner.is_empty() {
        return None;
    }

    let trailing = trimmed[close_idx + 1..].trim_start();
    if trailing.is_empty() || trailing.starts_with(';') || trailing.starts_with('#') {
        Some(inner)
    } else {
        None
    }
}

fn is_git_config_section_header(trimmed: &str) -> bool {
    config_section_header_inner(trimmed).is_some()
}

fn is_empty_branch_section_header(trimmed: &str) -> bool {
    let Some(inner) = config_section_header_inner(trimmed) else {
        return false;
    };
    let mut parts = inner.splitn(2, char::is_whitespace);
    let section = parts.next().unwrap_or("").trim();
    if !section.eq_ignore_ascii_case("branch") {
        return false;
    }

    parts.next().unwrap_or("").trim() == "\"\""
}

fn is_broken_merge_base_key_line(trimmed: &str) -> bool {
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
        return false;
    }

    let key = trimmed
        .split_once('=')
        .map(|(k, _)| k)
        .unwrap_or(trimmed)
        .trim();
    key.eq_ignore_ascii_case(BROKEN_GH_MERGE_BASE_KEY)
}

fn write_text_file_atomically(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path has no parent directory: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(
        ".{}.gwt-tmp-{}-{nonce}",
        file_name,
        std::process::id()
    ));

    std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
        format!(
            "Failed to write temporary config {}: {}",
            tmp_path.display(),
            e
        )
    })?;

    if let Ok(metadata) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp_path, metadata.permissions());
    }

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "Failed to replace config {} with {}: {}",
            path.display(),
            tmp_path.display(),
            err
        ));
    }

    Ok(())
}

pub fn is_gh_available() -> bool {
    gh_command()
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if gh CLI is authenticated (SPEC-ad1ac432 T003-T004).
///
/// Runs `gh auth status` with a 5-second timeout.
/// Returns `true` only when the command exits successfully within the timeout.
pub fn check_auth() -> bool {
    let child = gh_command()
        .args(["auth", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    let Ok(mut child) = child else {
        return false;
    };

    match wait_with_timeout(&mut child, Duration::from_secs(5)) {
        Some(status) => status.success(),
        None => {
            let _ = child.kill();
            false
        }
    }
}

/// Delete a remote branch via GitHub API (SPEC-ad1ac432 T005-T006).
///
/// Uses `gh api -X DELETE repos/{owner}/{repo}/git/refs/heads/{branch}`.
/// Timeout: 10 seconds.
pub fn delete_remote_branch(repo_path: &Path, branch: &str) -> Result<(), String> {
    let (owner, repo) = resolve_owner_repo(repo_path)?;

    let endpoint = format!("repos/{}/{}/git/refs/heads/{}", owner, repo, branch);
    let output = run_gh_output_with_timeout_and_repair(
        repo_path,
        ["api", "-X", "DELETE", endpoint.as_str()],
        Duration::from_secs(10),
    )
    .map_err(|e| format!("Failed to run gh api delete branch: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let combined = format!("{}{}", stderr, stdout);
    match classify_delete_branch_error(&combined, branch) {
        None => Ok(()),
        Some(err) => Err(err),
    }
}

/// Check which branches have a "deletion" rule preventing remote deletion (#1404).
///
/// Uses `gh api --paginate --jq '.[].type' repos/{owner}/{repo}/rules/branches/{branch}` per branch.
/// Returns a set of branch names that are delete-protected.
pub fn get_branch_deletion_rules(repo_path: &Path, branches: &[&str]) -> HashSet<String> {
    let Ok((owner, repo)) = resolve_owner_repo(repo_path) else {
        return HashSet::new();
    };
    let mut protected = HashSet::new();
    for branch in branches {
        let endpoint = format!("repos/{}/{}/rules/branches/{}", owner, repo, branch);
        let Ok(output) = run_gh_output_with_timeout_and_repair(
            repo_path,
            ["api", "--paginate", "--jq", ".[].type", endpoint.as_str()],
            Duration::from_secs(10),
        ) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if has_deletion_rule(&stdout) {
            protected.insert(branch.to_string());
        }
    }
    protected
}

fn has_deletion_rule(rule_types_output: &str) -> bool {
    rule_types_output
        .lines()
        .map(str::trim)
        .filter(|rule_type| !rule_type.is_empty())
        .map(|rule_type| {
            serde_json::from_str::<String>(rule_type)
                .unwrap_or_else(|_| rule_type.trim_matches('"').to_string())
        })
        .any(|rule_type| rule_type.eq_ignore_ascii_case("deletion"))
}

/// Get PR statuses for all branches (SPEC-ad1ac432 T007-T008).
///
/// Runs `gh pr list --state all --json headRefName,state,mergedAt --limit 200`.
/// Returns a map of branch name to PrStatus.
/// On failure, returns an empty map (caller decides how to handle).
pub fn get_pr_statuses(repo_path: &Path) -> HashMap<String, PrStatus> {
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "pr",
            "list",
            "--state",
            "all",
            "--json",
            "headRefName,state,mergedAt,updatedAt",
            "--limit",
            "200",
        ],
    );

    let Ok(output) = output else {
        return HashMap::new();
    };

    if !output.status.success() {
        return HashMap::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_statuses_json(&stdout)
}

/// Parse PR list JSON and resolve per-branch statuses.
///
/// Cleanup safety is driven by whether any PR for the same head branch remains
/// unmerged (`mergedAt == null`). Latest PR selection is a separate concern.
fn parse_pr_statuses_json(json_str: &str) -> HashMap<String, PrStatus> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return HashMap::new();
    };

    let Some(arr) = value.as_array() else {
        return HashMap::new();
    };

    // Collect all PR entries grouped by branch
    let mut branch_prs: HashMap<String, Vec<PrEntry>> = HashMap::new();

    for item in arr {
        let Some(head_ref) = item.get("headRefName").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(state) = item.get("state").and_then(|v| v.as_str()) else {
            continue;
        };
        let merged_at = item
            .get("mergedAt")
            .and_then(|v| v.as_str())
            .map(String::from);

        branch_prs
            .entry(head_ref.to_string())
            .or_default()
            .push(PrEntry {
                state: state.to_string(),
                merged_at,
            });
    }

    // For each branch, pick the latest PR
    branch_prs
        .into_iter()
        .map(|(branch, prs)| {
            let status = select_best_pr_status(&prs);
            (branch, status)
        })
        .collect()
}

struct PrEntry {
    state: String,
    merged_at: Option<String>,
}

fn select_best_pr_status(prs: &[PrEntry]) -> PrStatus {
    if prs.is_empty() {
        return PrStatus::None;
    }

    let mut saw_unmerged = false;
    let mut saw_unknown_unmerged = false;

    for pr in prs.iter().filter(|pr| pr.merged_at.is_none()) {
        saw_unmerged = true;
        match gh_state_to_pr_status(&pr.state) {
            PrStatus::Open => return PrStatus::Open,
            PrStatus::Closed => return PrStatus::Closed,
            PrStatus::Unknown => saw_unknown_unmerged = true,
            _ => {}
        }
    }

    if saw_unmerged && saw_unknown_unmerged {
        return PrStatus::Unknown;
    }

    if prs.iter().all(|pr| pr.merged_at.is_some()) {
        return PrStatus::Merged;
    }

    PrStatus::Unknown
}

fn gh_state_to_pr_status(state: &str) -> PrStatus {
    match state.to_uppercase().as_str() {
        "MERGED" => PrStatus::Merged,
        "OPEN" => PrStatus::Open,
        "CLOSED" => PrStatus::Closed,
        _ => PrStatus::Unknown,
    }
}

/// Resolve owner/repo from the repository using `gh repo view`.
fn resolve_owner_repo(repo_path: &Path) -> Result<(String, String), String> {
    let output = run_gh_output_with_repair(repo_path, ["repo", "view", "--json", "owner,name"])
        .map_err(|e| format!("Failed to run gh repo view: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh repo view failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("Failed to parse gh repo view output: {}", e))?;

    let owner = parsed
        .get("owner")
        .and_then(|v| {
            v.get("login")
                .and_then(|l| l.as_str())
                .or_else(|| v.as_str())
        })
        .ok_or("Missing owner in gh repo view output")?
        .to_string();

    let name = parsed
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name in gh repo view output")?
        .to_string();

    Ok((owner, name))
}

/// Build the API endpoint for creating a git ref.
fn create_ref_endpoint(owner: &str, repo: &str) -> String {
    format!("repos/{}/{}/git/refs", owner, repo)
}

/// Build the API endpoint for getting a git ref.
fn get_ref_endpoint(owner: &str, repo: &str, branch: &str) -> String {
    format!("repos/{}/{}/git/ref/heads/{}", owner, repo, branch)
}

/// Parse SHA from GitHub git/ref API response JSON.
fn parse_ref_sha(json_str: &str) -> Result<String, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {}", e))?;
    parsed
        .get("object")
        .and_then(|o| o.get("sha"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing object.sha in response".to_string())
}

/// Resolve the SHA of a remote branch via GitHub API (SPEC-a4fb2db2 FR-002).
///
/// Uses `gh api repos/{owner}/{repo}/git/ref/heads/{branch}`.
/// Returns the commit SHA on success.
pub fn resolve_remote_branch_sha(repo_path: &Path, branch: &str) -> Result<String, String> {
    let (owner, repo) = resolve_owner_repo(repo_path)?;
    let endpoint = get_ref_endpoint(&owner, &repo, branch);
    let output = run_gh_output_with_timeout_and_repair(
        repo_path,
        ["api", endpoint.as_str()],
        Duration::from_secs(10),
    )
    .map_err(|e| format!("Failed to resolve SHA for '{}': {}", branch, e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_ref_sha(&stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "Failed to resolve SHA for '{}': {}",
            branch,
            stderr.trim()
        ))
    }
}

/// Classify a GitHub API error response for branch deletion.
///
/// Returns `None` when the branch does not exist on the remote
/// (idempotent success: 404 or 422 "Reference does not exist"),
/// since the postcondition "branch is not on remote" is already met.
/// Returns `Some(message)` for genuine errors (403, network, etc).
fn classify_delete_branch_error(combined: &str, branch: &str) -> Option<String> {
    if combined.contains("404")
        || combined.contains("Not Found")
        || combined.contains("Reference does not exist")
    {
        None
    } else if combined.contains("403") || combined.contains("Forbidden") {
        Some(format!(
            "Permission denied: cannot delete remote branch '{}'",
            branch
        ))
    } else if combined.contains("Cannot delete this branch")
        || combined.contains("Repository rule violations")
    {
        Some(format!(
            "{} branch '{}' is protected by repository rules",
            PROTECTED_BRANCH_PREFIX, branch
        ))
    } else {
        Some(format!(
            "Failed to delete remote branch '{}': {}",
            branch,
            combined.trim()
        ))
    }
}

/// Classify a GitHub API error response for branch creation.
///
/// Returns a user-friendly error message based on HTTP status codes / keywords
/// found in the combined stderr+stdout output.
fn classify_create_branch_error(combined: &str, branch: &str) -> String {
    if combined.contains("422") || combined.contains("Reference already exists") {
        format!("Branch '{}' already exists on remote", branch)
    } else if combined.contains("403") || combined.contains("Forbidden") {
        format!(
            "Permission denied: cannot create remote branch '{}'",
            branch
        )
    } else if combined.contains("404") || combined.contains("Not Found") {
        format!("Repository not found on remote for branch '{}'", branch)
    } else {
        format!(
            "Failed to create remote branch '{}': {}",
            branch,
            combined.trim()
        )
    }
}

/// Create a remote branch via GitHub API (SPEC-a4fb2db2 FR-001).
///
/// Uses `gh api --method POST repos/{owner}/{repo}/git/refs`.
/// Timeout: 10 seconds.
pub fn create_remote_branch(repo_path: &Path, branch: &str, sha: &str) -> Result<(), String> {
    let (owner, repo) = resolve_owner_repo(repo_path)?;
    let endpoint = create_ref_endpoint(&owner, &repo);
    let ref_value = format!("refs/heads/{}", branch);
    let ref_arg = format!("ref={}", ref_value);
    let sha_arg = format!("sha={}", sha);
    let output = run_gh_output_with_timeout_and_repair(
        repo_path,
        [
            "api",
            "--method",
            "POST",
            endpoint.as_str(),
            "-f",
            ref_arg.as_str(),
            "-f",
            sha_arg.as_str(),
        ],
        Duration::from_secs(10),
    )
    .map_err(|e| format!("Failed to create remote branch '{}': {}", branch, e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let combined = format!("{}{}", stderr, stdout);
        Err(classify_create_branch_error(&combined, branch))
    }
}

/// Fetch PR list via `gh pr list` (SPEC-prlist).
///
/// `state` should be "open", "closed", "merged", or "all".
/// Returns a JSON array of PR objects.
pub fn fetch_pr_list(
    repo_path: &Path,
    state: &str,
    limit: u32,
) -> Result<Vec<serde_json::Value>, String> {
    let limit_str = limit.to_string();
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "pr",
            "list",
            "--state",
            state,
            "--json",
            "number,title,state,isDraft,headRefName,baseRefName,author,labels,createdAt,updatedAt,url,body,reviewRequests,assignees",
            "--limit",
            limit_str.as_str(),
        ],
    )
        .map_err(|e| format!("Failed to run gh pr list: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr list failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&stdout)
        .map_err(|e| format!("Failed to parse gh pr list output: {}", e))?;
    Ok(parsed)
}

/// Fetch authenticated GitHub user login via `gh api user` (SPEC-prlist).
pub fn fetch_authenticated_user(repo_path: &Path) -> Result<String, String> {
    let output = run_gh_output_with_repair(repo_path, ["api", "user", "--jq", ".login"])
        .map_err(|e| format!("Failed to run gh api user: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api user failed: {}", stderr.trim()));
    }

    let login = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if login.is_empty() {
        return Err("Empty login returned from gh api user".to_string());
    }
    Ok(login)
}

/// Merge a PR via `gh pr merge` (SPEC-prlist).
///
/// `method` should be "merge", "squash", or "rebase".
fn split_merge_commit_message(message: &str) -> Option<(String, Option<String>)> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut lines = trimmed.lines();
    let subject = lines.next()?.trim();
    if subject.is_empty() {
        return None;
    }

    let body = lines.collect::<Vec<_>>().join("\n");
    let body = if body.trim().is_empty() {
        None
    } else {
        Some(body.trim().to_string())
    };

    Some((subject.to_string(), body))
}

pub fn merge_pr(
    repo_path: &Path,
    pr_number: u64,
    method: &str,
    delete_branch: bool,
    commit_msg: Option<&str>,
) -> Result<String, String> {
    let mut args = vec![
        "pr".to_string(),
        "merge".to_string(),
        pr_number.to_string(),
        format!("--{}", method),
    ];

    if delete_branch {
        args.push("--delete-branch".to_string());
    }

    if let Some(msg) = commit_msg {
        if let Some((subject, body)) = split_merge_commit_message(msg) {
            args.push("--subject".to_string());
            args.push(subject);
            if let Some(body) = body {
                args.push("--body".to_string());
                args.push(body);
            }
        }
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let output = run_gh_output_with_repair(repo_path, arg_refs)
        .map_err(|e| format!("Failed to run gh pr merge: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr merge failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout)
}

/// Review a PR via `gh pr review` (SPEC-prlist).
///
/// `action` should be "approve", "request-changes", or "comment".
pub fn review_pr(
    repo_path: &Path,
    pr_number: u64,
    action: &str,
    body: Option<&str>,
) -> Result<String, String> {
    let mut args = vec![
        "pr".to_string(),
        "review".to_string(),
        pr_number.to_string(),
        format!("--{}", action),
    ];

    if let Some(body_text) = body {
        args.push("--body".to_string());
        args.push(body_text.to_string());
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let output = run_gh_output_with_repair(repo_path, arg_refs)
        .map_err(|e| format!("Failed to run gh pr review: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr review failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout)
}

/// Mark a draft PR as ready for review via `gh pr ready` (SPEC-prlist).
pub fn mark_pr_ready(repo_path: &Path, pr_number: u64) -> Result<String, String> {
    let pr_number = pr_number.to_string();
    let output = run_gh_output_with_repair(repo_path, ["pr", "ready", pr_number.as_str()])
        .map_err(|e| format!("Failed to run gh pr ready: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr ready failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout)
}

/// Wait for a child process with a timeout.
/// Returns `None` if the timeout is reached.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn is_gh_available_returns_bool() {
        let _result: bool = is_gh_available();
    }

    #[test]
    fn resolve_gh_path_accepts_missing_or_present() {
        let _result = resolve_gh_path();
    }

    // -- T001: PrStatus enum tests --

    #[test]
    fn pr_status_serializes_to_lowercase() {
        assert_eq!(
            serde_json::to_string(&PrStatus::Merged).unwrap(),
            "\"merged\""
        );
        assert_eq!(serde_json::to_string(&PrStatus::Open).unwrap(), "\"open\"");
        assert_eq!(
            serde_json::to_string(&PrStatus::Closed).unwrap(),
            "\"closed\""
        );
        assert_eq!(serde_json::to_string(&PrStatus::None).unwrap(), "\"none\"");
        assert_eq!(
            serde_json::to_string(&PrStatus::Unknown).unwrap(),
            "\"unknown\""
        );
    }

    #[test]
    fn pr_status_deserializes_from_lowercase() {
        assert_eq!(
            serde_json::from_str::<PrStatus>("\"merged\"").unwrap(),
            PrStatus::Merged
        );
        assert_eq!(
            serde_json::from_str::<PrStatus>("\"open\"").unwrap(),
            PrStatus::Open
        );
        assert_eq!(
            serde_json::from_str::<PrStatus>("\"closed\"").unwrap(),
            PrStatus::Closed
        );
        assert_eq!(
            serde_json::from_str::<PrStatus>("\"none\"").unwrap(),
            PrStatus::None
        );
        assert_eq!(
            serde_json::from_str::<PrStatus>("\"unknown\"").unwrap(),
            PrStatus::Unknown
        );
    }

    #[test]
    fn pr_status_roundtrip() {
        for status in [
            PrStatus::Merged,
            PrStatus::Open,
            PrStatus::Closed,
            PrStatus::None,
            PrStatus::Unknown,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: PrStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    // -- T002: Function signature type tests --

    #[test]
    fn check_auth_returns_bool() {
        let _result: bool = check_auth();
    }

    #[test]
    fn get_pr_statuses_returns_hashmap() {
        // Verifies that the function signature compiles correctly
        let _: fn(&Path) -> HashMap<String, PrStatus> = get_pr_statuses;
    }

    #[test]
    fn delete_remote_branch_returns_result() {
        // Verifies that the function signature compiles correctly
        let _: fn(&Path, &str) -> Result<(), String> = delete_remote_branch;
    }

    #[test]
    fn split_merge_commit_message_single_line() {
        let parsed = split_merge_commit_message("  chore: merge release  ").unwrap();
        assert_eq!(parsed.0, "chore: merge release");
        assert!(parsed.1.is_none());
    }

    #[test]
    fn split_merge_commit_message_multiline() {
        let parsed = split_merge_commit_message("feat: merge api\n\n- keep body").unwrap();
        assert_eq!(parsed.0, "feat: merge api");
        assert_eq!(parsed.1, Some("- keep body".to_string()));
    }

    #[test]
    fn split_merge_commit_message_empty_returns_none() {
        assert!(split_merge_commit_message("   \n  ").is_none());
    }

    // -- T003-T004: check_auth tests --

    #[test]
    fn gh_auth_command_structure() {
        // Verify that gh_command() can be constructed with auth args
        let mut cmd = gh_command();
        cmd.args(["auth", "status"]);
        // This is a structural test - actual execution depends on environment
    }

    // -- T007-T008: get_pr_statuses / parse_pr_statuses_json tests --

    #[test]
    fn pr_status_merged() {
        let json = r#"[
            {"headRefName": "feature/done", "state": "MERGED", "mergedAt": "2026-01-15T10:00:00Z", "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        assert_eq!(statuses.get("feature/done"), Some(&PrStatus::Merged));
    }

    #[test]
    fn pr_status_open() {
        let json = r#"[
            {"headRefName": "feature/wip", "state": "OPEN", "mergedAt": null, "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        assert_eq!(statuses.get("feature/wip"), Some(&PrStatus::Open));
    }

    #[test]
    fn pr_status_closed() {
        let json = r#"[
            {"headRefName": "feature/abandoned", "state": "CLOSED", "mergedAt": null, "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        assert_eq!(statuses.get("feature/abandoned"), Some(&PrStatus::Closed));
    }

    #[test]
    fn pr_status_none_for_unknown_branch() {
        let json = r#"[
            {"headRefName": "feature/done", "state": "MERGED", "mergedAt": "2026-01-15T10:00:00Z", "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        // A branch with no PR should not be in the map at all
        assert_eq!(statuses.get("feature/other"), None);
    }

    #[test]
    fn pr_status_multiple_prs_prefers_closed_unmerged_over_merged() {
        let json = r#"[
            {"headRefName": "feature/multi", "state": "CLOSED", "mergedAt": null, "updatedAt": "2026-01-10T10:00:00Z"},
            {"headRefName": "feature/multi", "state": "MERGED", "mergedAt": "2026-01-15T10:00:00Z", "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        assert_eq!(statuses.get("feature/multi"), Some(&PrStatus::Closed));
    }

    #[test]
    fn pr_status_multiple_prs_prefers_open_unmerged_over_merged() {
        let json = r#"[
            {"headRefName": "feature/multi", "state": "OPEN", "mergedAt": null, "updatedAt": "2026-01-10T10:00:00Z"},
            {"headRefName": "feature/multi", "state": "MERGED", "mergedAt": "2026-01-15T10:00:00Z", "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        assert_eq!(statuses.get("feature/multi"), Some(&PrStatus::Open));
    }

    #[test]
    fn pr_status_gh_failure_returns_empty() {
        let json = "not valid json";
        let statuses = parse_pr_statuses_json(json);
        assert!(statuses.is_empty());
    }

    #[test]
    fn pr_status_empty_array() {
        let json = "[]";
        let statuses = parse_pr_statuses_json(json);
        assert!(statuses.is_empty());
    }

    #[test]
    fn gh_state_to_pr_status_variants() {
        assert_eq!(gh_state_to_pr_status("MERGED"), PrStatus::Merged);
        assert_eq!(gh_state_to_pr_status("OPEN"), PrStatus::Open);
        assert_eq!(gh_state_to_pr_status("CLOSED"), PrStatus::Closed);
        assert_eq!(gh_state_to_pr_status("merged"), PrStatus::Merged);
        assert_eq!(gh_state_to_pr_status("unknown_state"), PrStatus::Unknown);
    }

    // -- T005-T006: delete_remote_branch structural tests --

    #[test]
    fn resolve_owner_repo_function_exists() {
        // Structural test: function compiles with expected signature
        let _: fn(&Path) -> Result<(String, String), String> = resolve_owner_repo;
    }

    // -- wait_with_timeout tests --

    #[test]
    fn wait_with_timeout_function_exists() {
        // Structural test
        let _: fn(&mut std::process::Child, Duration) -> Option<std::process::ExitStatus> =
            wait_with_timeout;
    }

    // -- SPEC-a4fb2db2: create_remote_branch / resolve_remote_branch_sha tests --

    #[test]
    fn create_remote_branch_returns_result() {
        // Verifies that the function signature compiles correctly
        let _: fn(&Path, &str, &str) -> Result<(), String> = create_remote_branch;
    }

    #[test]
    fn resolve_remote_branch_sha_returns_result() {
        // Verifies that the function signature compiles correctly
        let _: fn(&Path, &str) -> Result<String, String> = resolve_remote_branch_sha;
    }

    #[test]
    fn create_remote_branch_endpoint_format() {
        let endpoint = create_ref_endpoint("akiojin", "gwt");
        assert_eq!(endpoint, "repos/akiojin/gwt/git/refs");
    }

    #[test]
    fn resolve_remote_branch_sha_endpoint_format() {
        let endpoint = get_ref_endpoint("akiojin", "gwt", "develop");
        assert_eq!(endpoint, "repos/akiojin/gwt/git/ref/heads/develop");
    }

    #[test]
    fn resolve_remote_branch_sha_endpoint_format_with_slash() {
        let endpoint = get_ref_endpoint("akiojin", "gwt", "feature/test");
        assert_eq!(endpoint, "repos/akiojin/gwt/git/ref/heads/feature/test");
    }

    #[test]
    fn resolve_remote_branch_sha_parses_json() {
        let json = r#"{
            "ref": "refs/heads/develop",
            "object": {
                "sha": "abc123def456",
                "type": "commit"
            }
        }"#;
        let sha = parse_ref_sha(json).unwrap();
        assert_eq!(sha, "abc123def456");
    }

    #[test]
    fn resolve_remote_branch_sha_invalid_json() {
        let result = parse_ref_sha("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_remote_branch_sha_missing_sha() {
        let json = r#"{"ref": "refs/heads/develop", "object": {"type": "commit"}}"#;
        let result = parse_ref_sha(json);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_remote_branch_sha_missing_object() {
        let json = r#"{"ref": "refs/heads/develop"}"#;
        let result = parse_ref_sha(json);
        assert!(result.is_err());
    }

    // -- SPEC-a4fb2db2: classify_create_branch_error tests --

    #[test]
    fn classify_error_422_reference_already_exists() {
        let msg = classify_create_branch_error("Reference already exists", "feature/x");
        assert!(msg.contains("already exists on remote"));
        assert!(msg.contains("feature/x"));
    }

    #[test]
    fn classify_error_422_status_code() {
        let msg = classify_create_branch_error(
            r#"{"message":"Reference already exists","status":"422"}"#,
            "feat/y",
        );
        assert!(msg.contains("already exists on remote"));
    }

    #[test]
    fn classify_error_403_forbidden() {
        let msg = classify_create_branch_error("403 Forbidden", "feat/z");
        assert!(msg.contains("Permission denied"));
        assert!(msg.contains("feat/z"));
    }

    #[test]
    fn classify_error_404_not_found() {
        let msg = classify_create_branch_error("404 Not Found", "feat/w");
        assert!(msg.contains("not found on remote"));
        assert!(msg.contains("feat/w"));
    }

    #[test]
    fn classify_error_unknown() {
        let msg = classify_create_branch_error("something unexpected", "feat/u");
        assert!(msg.contains("Failed to create remote branch"));
        assert!(msg.contains("feat/u"));
        assert!(msg.contains("something unexpected"));
    }

    #[test]
    fn classify_error_422_does_not_fallback() {
        // Verify the error message format that terminal.rs checks for non-fallback
        let msg = classify_create_branch_error("422", "feature/no-fallback");
        assert!(msg.contains("already exists on remote"));
    }

    // -- classify_delete_branch_error tests --

    #[test]
    fn classify_delete_error_404_returns_none() {
        assert!(classify_delete_branch_error("404 Not Found", "feature/x").is_none());
    }

    #[test]
    fn classify_delete_error_not_found_returns_none() {
        assert!(classify_delete_branch_error("Not Found", "feature/x").is_none());
    }

    #[test]
    fn classify_delete_error_422_reference_does_not_exist_returns_none() {
        let msg = r#"gh: Reference does not exist (HTTP 422)"#;
        assert!(classify_delete_branch_error(msg, "feature/x").is_none());
    }

    #[test]
    fn classify_delete_error_403_forbidden() {
        let result = classify_delete_branch_error("403 Forbidden", "feat/z");
        assert!(result.is_some());
        assert!(result.unwrap().contains("Permission denied"));
    }

    #[test]
    fn classify_delete_error_unknown() {
        let result = classify_delete_branch_error("something unexpected", "feat/u");
        assert!(result.is_some());
        let err = result.unwrap();
        assert!(err.contains("Failed to delete remote branch"));
        assert!(err.contains("something unexpected"));
    }

    #[test]
    fn classify_delete_error_422_different_message_is_error() {
        // 422 but different message should NOT be treated as success
        let result = classify_delete_branch_error("422 Validation Failed", "feat/v");
        assert!(result.is_some());
    }

    #[test]
    fn classify_delete_error_422_cannot_delete_protected() {
        let msg = r#"{"message":"Cannot delete this branch"}"#;
        let result = classify_delete_branch_error(msg, "main");
        assert!(result.is_some());
        assert!(result.unwrap().starts_with(PROTECTED_BRANCH_PREFIX));
    }

    #[test]
    fn classify_delete_error_repository_rule_violations() {
        let msg = "Repository rule violations found";
        let result = classify_delete_branch_error(msg, "develop");
        assert!(result.is_some());
        assert!(result.unwrap().starts_with(PROTECTED_BRANCH_PREFIX));
    }

    #[test]
    fn get_branch_deletion_rules_returns_hashset() {
        // Structural test: function compiles with expected signature
        let _: fn(&Path, &[&str]) -> HashSet<String> = get_branch_deletion_rules;
    }

    #[test]
    fn has_deletion_rule_true_for_paginated_multi_line_output() {
        let output = "\"required_status_checks\"\n\"pull_request\"\n\"workflow\"\n\"deletion\"\n";
        assert!(has_deletion_rule(output));
    }

    #[test]
    fn has_deletion_rule_false_when_deletion_is_missing() {
        let output = "\"required_status_checks\"\n\"pull_request\"\n\"workflow\"\n";
        assert!(!has_deletion_rule(output));
    }

    #[test]
    fn sanitize_config_removes_empty_branch_section() {
        let input = r#"[core]
  repositoryformatversion = 0
[branch ""]
  gh-merge-base = develop
[branch "feature/test"]
  remote = origin
"#;
        let result = sanitize_bad_gh_merge_base_config(input);
        assert!(!result.contains("[branch \"\"]"));
        assert!(!result.contains("gh-merge-base = develop"));
        assert!(result.contains("[core]"));
        assert!(result.contains("[branch \"feature/test\"]"));
    }

    #[test]
    fn sanitize_config_removes_flat_bad_key_line() {
        let input = r#"[core]
  bare = false
branch..gh-merge-base = develop
"#;
        let result = sanitize_bad_gh_merge_base_config(input);
        assert!(!result.contains("branch..gh-merge-base"));
        assert!(result.contains("[core]"));
    }

    #[test]
    fn sanitize_config_keeps_valid_branch_merge_base() {
        let input = r#"[branch "feature/test"]
  gh-merge-base = develop
"#;
        let result = sanitize_bad_gh_merge_base_config(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_config_preserves_entries_after_comment_style_section_header() {
        let input = r#"[branch ""]
  gh-merge-base = develop
[core] ; keep this section
  bare = false
[remote "origin"]
  fetch = +refs/heads/*:refs/remotes/origin/*
"#;
        let result = sanitize_bad_gh_merge_base_config(input);
        assert!(!result.contains("[branch \"\"]"));
        assert!(result.contains("[core] ; keep this section"));
        assert!(result.contains("bare = false"));
        assert!(result.contains("[remote \"origin\"]"));
    }

    #[test]
    fn sanitize_config_removes_empty_branch_section_with_trailing_comment() {
        let input = r#"[branch ""] ; invalid empty branch section
  gh-merge-base = develop
[core]
  bare = false
"#;
        let result = sanitize_bad_gh_merge_base_config(input);
        assert!(!result.contains("[branch \"\"]"));
        assert!(!result.contains("gh-merge-base = develop"));
        assert!(result.contains("[core]"));
        assert!(result.contains("bare = false"));
    }

    #[test]
    fn repair_bad_merge_base_config_updates_git_config() {
        let repo = TempDir::new().expect("temp dir");
        let init = crate::process::command("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .expect("git init");
        assert!(init.status.success());

        let config_path = repo.path().join(".git").join("config");
        let original = std::fs::read_to_string(&config_path).expect("read git config");
        let modified = format!("{original}\n[branch \"\"]\n\tgh-merge-base = develop\n");
        std::fs::write(&config_path, modified).expect("write broken config");

        let repaired = repair_bad_gh_merge_base_config(repo.path()).expect("repair should succeed");
        assert!(repaired);

        let content = std::fs::read_to_string(&config_path).expect("read repaired config");
        assert!(!content.contains("[branch \"\"]"));
        assert!(!content.contains("branch..gh-merge-base"));
    }

    #[test]
    fn repair_bad_merge_base_config_returns_false_when_no_change() {
        let repo = TempDir::new().expect("temp dir");
        let init = crate::process::command("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .expect("git init");
        assert!(init.status.success());

        let repaired = repair_bad_gh_merge_base_config(repo.path()).expect("repair");
        assert!(!repaired);
    }
}
