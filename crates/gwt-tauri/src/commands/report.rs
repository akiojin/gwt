//! Tauri commands for error reporting and feature suggestions.

use gwt_core::terminal::scrollback::{strip_ansi, ScrollbackFile};
use gwt_core::StructuredError;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

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

fn log_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "gwt")
        .map(|p| p.data_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".gwt/logs"))
}

fn is_log_file_candidate(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    name.ends_with(".jsonl") || name.contains(".jsonl.")
}

fn collect_log_candidates(log_base: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let Ok(entries) = fs::read_dir(log_base) else {
        return candidates;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if is_log_file_candidate(&path) {
                candidates.push(path);
            }
            continue;
        }

        if !path.is_dir() {
            continue;
        }

        if let Ok(files) = fs::read_dir(&path) {
            for file_entry in files.flatten() {
                let file_path = file_entry.path();
                if file_path.is_file() && is_log_file_candidate(&file_path) {
                    candidates.push(file_path);
                }
            }
        }
    }

    candidates
}

/// Read the last `max_lines` lines from the most recent log file.
#[tauri::command]
pub fn read_recent_logs(max_lines: Option<u32>) -> Result<String, StructuredError> {
    let max = max_lines.unwrap_or(100) as usize;
    let log_base = log_dir();

    // Find the most recent .jsonl file across workspace dirs and root-level files.
    let mut candidates = collect_log_candidates(&log_base);

    if candidates.is_empty() {
        return Ok("(No log files found)".to_string());
    }

    // Sort by modification time (most recent first)
    candidates.sort_by(|a, b| {
        let ma = a.metadata().and_then(|m| m.modified()).ok();
        let mb = b.metadata().and_then(|m| m.modified()).ok();
        mb.cmp(&ma)
    });

    let content = fs::read_to_string(&candidates[0])
        .map_err(|e| StructuredError::internal(&e.to_string(), "read_recent_logs"))?;

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max);
    Ok(lines[start..].join("\n"))
}

/// Get basic system info for error reports.
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
    fn collect_log_candidates_includes_root_and_workspace_dirs() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let base = tmp.path();

        let root_log = base.join("root.jsonl");
        std::fs::write(&root_log, "{\"msg\":\"root\"}\n").expect("write root log");

        let workspace_dir = base.join("workspace-a");
        std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
        let nested_log = workspace_dir.join("nested.jsonl");
        std::fs::write(&nested_log, "{\"msg\":\"nested\"}\n").expect("write nested log");

        let ignored = workspace_dir.join("ignored.log");
        std::fs::write(&ignored, "ignored\n").expect("write ignored");

        let mut collected = collect_log_candidates(base);
        collected.sort();

        assert!(collected.contains(&root_log));
        assert!(collected.contains(&nested_log));
        assert!(!collected.contains(&ignored));
    }
}
