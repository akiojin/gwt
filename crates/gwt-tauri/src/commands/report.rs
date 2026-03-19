//! Tauri commands for error reporting and feature suggestions.

use gwt_core::terminal::scrollback::{strip_ansi, ScrollbackFile};
use gwt_core::StructuredError;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;
use tracing::instrument;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSystemInfo {
    pub os_name: String,
    pub os_version: String,
    pub arch: String,
    pub gwt_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTarget {
    pub owner: String,
    pub repo: String,
    pub display: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIssueResult {
    pub url: String,
    pub number: u64,
}

fn candidate_log_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".gwt").join("logs"));
    }

    if let Some(project_dirs) = directories::ProjectDirs::from("", "", "gwt") {
        roots.push(project_dirs.data_dir().join("logs"));
    }

    roots.push(PathBuf::from(".gwt/logs"));

    let mut seen = HashSet::new();
    roots
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn collect_log_candidates(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    let mut collect_from_dir = |dir: &Path| {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && is_log_file_candidate(&path) {
                    candidates.push(path);
                }
            }
        }
    };

    // Support logs written directly under the root path.
    collect_from_dir(root);

    // Support logs written under workspace subdirectories.
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_from_dir(&path);
            }
        }
    }

    candidates
}

fn select_most_recent_log(mut candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.sort_by(|a, b| {
        let ma = a.metadata().and_then(|m| m.modified()).ok();
        let mb = b.metadata().and_then(|m| m.modified()).ok();
        mb.cmp(&ma)
    });
    candidates.into_iter().next()
}

fn is_log_file_candidate(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    name.ends_with(".jsonl") || name.contains(".jsonl.")
}

/// Read the last `max_lines` lines from the most recent log file.
#[instrument(skip_all, fields(command = "read_recent_logs"))]
#[tauri::command]
pub fn read_recent_logs(max_lines: Option<u32>) -> Result<String, StructuredError> {
    let max = max_lines.unwrap_or(100) as usize;
    let mut candidates: Vec<PathBuf> = Vec::new();

    for root in candidate_log_roots() {
        candidates.extend(collect_log_candidates(&root));
    }

    let Some(latest_log_path) = select_most_recent_log(candidates) else {
        return Ok("(No log files found)".to_string());
    };

    let content = fs::read_to_string(&latest_log_path)
        .map_err(|e| StructuredError::internal(&e.to_string(), "read_recent_logs"))?;

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max);
    Ok(lines[start..].join("\n"))
}

/// Get basic system info for error reports.
#[instrument(skip_all, fields(command = "get_report_system_info"))]
#[tauri::command]
pub fn get_report_system_info() -> ReportSystemInfo {
    ReportSystemInfo {
        os_name: std::env::consts::OS.to_string(),
        os_version: os_version(),
        arch: std::env::consts::ARCH.to_string(),
        gwt_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        gwt_core::process::command("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        gwt_core::process::command("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        gwt_core::process::command("uname")
            .arg("-r")
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
}

const DEFAULT_REPORT_OWNER: &str = "akiojin";
const DEFAULT_REPORT_REPO: &str = "gwt";

/// Parse owner/repo from a git remote URL.
///
/// Supports:
///   - SSH:   `git@github.com:owner/repo.git`
///   - HTTPS: `https://github.com/owner/repo.git`
fn parse_owner_repo_from_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();

    // SSH format: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let rest = rest.strip_suffix(".git").unwrap_or(rest);
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next()?.to_string();
        let repo = parts.next()?.to_string();
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner, repo));
        }
    }

    // HTTPS format: https://github.com/owner/repo.git
    if url.contains("github.com/") {
        let after = url.split("github.com/").nth(1)?;
        let after = after.strip_suffix(".git").unwrap_or(after);
        let mut parts = after.splitn(2, '/');
        let owner = parts.next()?.to_string();
        let repo = parts.next()?.to_string();
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner, repo));
        }
    }

    None
}

/// Detect the current working repository's owner/repo from git remote.
#[instrument(skip_all, fields(command = "detect_report_target", project_path))]
#[tauri::command]
pub fn detect_report_target(project_path: String) -> Result<ReportTarget, StructuredError> {
    let output = gwt_core::process::command("git")
        .args(["-C", &project_path, "remote", "get-url", "origin"])
        .output()
        .map_err(|e| StructuredError::internal(&e.to_string(), "detect_report_target"))?;

    if output.status.success() {
        let remote_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some((owner, repo)) = parse_owner_repo_from_remote(&remote_url) {
            return Ok(ReportTarget {
                display: format!("{}/{}", owner, repo),
                owner,
                repo,
            });
        }
    }

    // Fallback to default gwt repository
    Ok(ReportTarget {
        owner: DEFAULT_REPORT_OWNER.to_string(),
        repo: DEFAULT_REPORT_REPO.to_string(),
        display: format!("{}/{}", DEFAULT_REPORT_OWNER, DEFAULT_REPORT_REPO),
    })
}

/// Create a GitHub issue using the gh CLI.
#[instrument(skip_all, fields(command = "create_github_issue"))]
#[tauri::command]
pub fn create_github_issue(
    owner: String,
    repo: String,
    title: String,
    body: String,
    labels: Vec<String>,
) -> Result<CreateIssueResult, StructuredError> {
    let repo_slug = format!("{}/{}", owner, repo);

    let mut cmd = gwt_core::git::gh_cli::gh_command();
    cmd.args([
        "issue", "create", "--repo", &repo_slug, "--title", &title, "--body", &body,
    ]);

    for label in &labels {
        cmd.args(["--label", label]);
    }

    let output = cmd
        .output()
        .map_err(|e| StructuredError::internal(&e.to_string(), "create_github_issue"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(StructuredError::internal(
            &format!("gh issue create failed: {}", stderr),
            "create_github_issue",
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // gh outputs the issue URL, e.g. https://github.com/owner/repo/issues/123
    let number = stdout
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(CreateIssueResult {
        url: stdout,
        number,
    })
}

/// Maximum bytes to capture per pane for screen text capture.
const SCREEN_CAPTURE_MAX_BYTES: usize = 8192;

/// Capture the current terminal/screen text for bug reports.
///
/// Iterates over all active panes and captures their scrollback tail.
/// ANSI escape sequences are stripped to produce plain text output.
#[instrument(skip_all, fields(command = "capture_screen_text"))]
#[tauri::command]
pub fn capture_screen_text(
    state: State<'_, crate::state::AppState>,
) -> Result<String, StructuredError> {
    let mut mgr = state.pane_manager.lock().map_err(|_| {
        StructuredError::internal("Failed to lock pane manager", "capture_screen_text")
    })?;

    // Flush all panes first so we get the latest data
    for pane in mgr.panes_mut() {
        let _ = pane.flush_scrollback();
    }

    // Now iterate immutably and capture text
    let pane_ids: Vec<String> = mgr
        .panes()
        .iter()
        .map(|p| p.pane_id().to_string())
        .collect();
    drop(mgr);

    let mut text = String::new();
    for pane_id in &pane_ids {
        let path = match ScrollbackFile::scrollback_path_for_pane(pane_id) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let bytes = match ScrollbackFile::read_tail_bytes_at(&path, SCREEN_CAPTURE_MAX_BYTES) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let content = strip_ansi(&bytes);
        if !content.trim().is_empty() {
            text.push_str(&format!("--- Pane: {} ---\n", pane_id));
            text.push_str(&content);
            text.push('\n');
        }
    }

    if text.is_empty() {
        text = "(No active terminal panes)".to_string();
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn parse_ssh_remote() {
        let (owner, repo) = parse_owner_repo_from_remote("git@github.com:akiojin/gwt.git").unwrap();
        assert_eq!(owner, "akiojin");
        assert_eq!(repo, "gwt");
    }

    #[test]
    fn parse_ssh_remote_without_git_suffix() {
        let (owner, repo) = parse_owner_repo_from_remote("git@github.com:akiojin/gwt").unwrap();
        assert_eq!(owner, "akiojin");
        assert_eq!(repo, "gwt");
    }

    #[test]
    fn parse_https_remote() {
        let (owner, repo) =
            parse_owner_repo_from_remote("https://github.com/akiojin/gwt.git").unwrap();
        assert_eq!(owner, "akiojin");
        assert_eq!(repo, "gwt");
    }

    #[test]
    fn parse_https_remote_without_git_suffix() {
        let (owner, repo) = parse_owner_repo_from_remote("https://github.com/akiojin/gwt").unwrap();
        assert_eq!(owner, "akiojin");
        assert_eq!(repo, "gwt");
    }

    #[test]
    fn parse_invalid_remote_returns_none() {
        assert!(parse_owner_repo_from_remote("not-a-url").is_none());
        assert!(parse_owner_repo_from_remote("").is_none());
    }

    #[test]
    fn parse_remote_with_whitespace() {
        let (owner, repo) =
            parse_owner_repo_from_remote("  git@github.com:owner/repo.git  \n").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn report_target_struct_serializes() {
        let target = ReportTarget {
            owner: "akiojin".to_string(),
            repo: "gwt".to_string(),
            display: "akiojin/gwt".to_string(),
        };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["owner"], "akiojin");
        assert_eq!(json["repo"], "gwt");
        assert_eq!(json["display"], "akiojin/gwt");
    }

    #[test]
    fn create_issue_result_serializes() {
        let result = CreateIssueResult {
            url: "https://github.com/akiojin/gwt/issues/42".to_string(),
            number: 42,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["url"], "https://github.com/akiojin/gwt/issues/42");
        assert_eq!(json["number"], 42);
    }

    #[test]
    fn log_file_candidate_matches_jsonl() {
        assert!(is_log_file_candidate(Path::new("gwt.jsonl")));
    }

    #[test]
    fn log_file_candidate_matches_rotated_jsonl() {
        assert!(is_log_file_candidate(Path::new("gwt.jsonl.2026-02-21")));
    }

    #[test]
    fn log_file_candidate_rejects_non_jsonl() {
        assert!(!is_log_file_candidate(Path::new("gwt.log")));
    }

    #[test]
    fn candidate_log_roots_include_relative_fallback_and_have_no_duplicates() {
        let roots = candidate_log_roots();

        assert!(roots.contains(&PathBuf::from(".gwt/logs")));

        if let Some(home) = dirs::home_dir() {
            assert!(roots.contains(&home.join(".gwt").join("logs")));
        }

        let unique: HashSet<PathBuf> = roots.iter().cloned().collect();
        assert_eq!(unique.len(), roots.len());
    }

    #[test]
    fn collect_log_candidates_includes_files_under_root() {
        let temp = tempdir().unwrap();
        let root_log = temp.path().join("gwt.jsonl");
        fs::write(&root_log, "line-1\nline-2\n").unwrap();

        let candidates = collect_log_candidates(temp.path());

        assert!(candidates.contains(&root_log));
    }

    #[test]
    fn collect_log_candidates_includes_files_under_workspace_directories() {
        let temp = tempdir().unwrap();
        let workspace_dir = temp.path().join("workspace-a");
        fs::create_dir_all(&workspace_dir).unwrap();
        let workspace_log = workspace_dir.join("gwt.jsonl.2026-03-04");
        fs::write(&workspace_log, "line\n").unwrap();

        let candidates = collect_log_candidates(temp.path());

        assert!(candidates.contains(&workspace_log));
    }

    #[test]
    fn collect_log_candidates_rejects_non_jsonl_files() {
        let temp = tempdir().unwrap();
        let non_log_file = temp.path().join("gwt.log");
        fs::write(&non_log_file, "not a jsonl log").unwrap();

        let candidates = collect_log_candidates(temp.path());

        assert!(candidates.is_empty());
    }

    #[test]
    fn select_most_recent_log_prefers_newest_file() {
        let temp = tempdir().unwrap();
        let older = temp.path().join("gwt.jsonl.2026-03-03");
        let newer = temp.path().join("gwt.jsonl.2026-03-04");

        fs::write(&older, "older log\n").unwrap();
        thread::sleep(Duration::from_millis(1200));
        fs::write(&newer, "newer log\n").unwrap();

        let selected = select_most_recent_log(vec![older, newer.clone()]).unwrap();

        assert_eq!(selected, newer);
    }
}
