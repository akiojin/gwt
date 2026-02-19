use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

    let child = gh_command()
        .args(["api", "-X", "DELETE", &endpoint])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = child else {
        return Err("Failed to spawn gh command".to_string());
    };

    match wait_with_timeout(&mut child, Duration::from_secs(10)) {
        Some(status) => {
            if status.success() {
                Ok(())
            } else {
                let stderr = child
                    .stderr
                    .take()
                    .and_then(|mut s| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                        Some(buf)
                    })
                    .unwrap_or_default();
                let stdout = child
                    .stdout
                    .take()
                    .and_then(|mut s| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                        Some(buf)
                    })
                    .unwrap_or_default();
                let combined = format!("{}{}", stderr, stdout);

                if combined.contains("404") || combined.contains("Not Found") {
                    Err(format!("Branch '{}' not found on remote", branch))
                } else if combined.contains("403") || combined.contains("Forbidden") {
                    Err(format!(
                        "Permission denied: cannot delete remote branch '{}'",
                        branch
                    ))
                } else {
                    Err(format!(
                        "Failed to delete remote branch '{}': {}",
                        branch,
                        combined.trim()
                    ))
                }
            }
        }
        None => {
            let _ = child.kill();
            Err(format!("Timeout deleting remote branch '{}' (10s)", branch))
        }
    }
}

/// Get PR statuses for all branches (SPEC-ad1ac432 T007-T008).
///
/// Runs `gh pr list --state all --json headRefName,state,mergedAt --limit 200`.
/// Returns a map of branch name to PrStatus.
/// On failure, returns an empty map (caller decides how to handle).
pub fn get_pr_statuses(repo_path: &Path) -> HashMap<String, PrStatus> {
    let output = gh_command()
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--json",
            "headRefName,state,mergedAt,updatedAt",
            "--limit",
            "200",
        ])
        .current_dir(repo_path)
        .output();

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
/// When multiple PRs exist for the same branch, the latest is used
/// (preferring merged/closed over open when timestamps are equal).
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
        let updated_at = item
            .get("updatedAt")
            .and_then(|v| v.as_str())
            .map(String::from);

        branch_prs
            .entry(head_ref.to_string())
            .or_default()
            .push(PrEntry {
                state: state.to_string(),
                merged_at,
                updated_at,
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
    updated_at: Option<String>,
}

fn select_best_pr_status(prs: &[PrEntry]) -> PrStatus {
    if prs.is_empty() {
        return PrStatus::None;
    }

    // Pick the PR with the latest timestamp
    let best = prs
        .iter()
        .max_by(|a, b| {
            let ts_a = a
                .merged_at
                .as_deref()
                .or(a.updated_at.as_deref())
                .unwrap_or("");
            let ts_b = b
                .merged_at
                .as_deref()
                .or(b.updated_at.as_deref())
                .unwrap_or("");
            ts_a.cmp(ts_b)
        })
        .unwrap();

    gh_state_to_pr_status(&best.state)
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
    let output = gh_command()
        .args(["repo", "view", "--json", "owner,name"])
        .current_dir(repo_path)
        .output()
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
    fn pr_status_multiple_prs_uses_latest() {
        let json = r#"[
            {"headRefName": "feature/multi", "state": "CLOSED", "mergedAt": null, "updatedAt": "2026-01-10T10:00:00Z"},
            {"headRefName": "feature/multi", "state": "MERGED", "mergedAt": "2026-01-15T10:00:00Z", "updatedAt": "2026-01-15T10:00:00Z"}
        ]"#;
        let statuses = parse_pr_statuses_json(json);
        assert_eq!(statuses.get("feature/multi"), Some(&PrStatus::Merged));
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
}
