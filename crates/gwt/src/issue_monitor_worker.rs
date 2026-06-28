use std::{fmt, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{IssueMonitorInboxItem, IssueMonitorIssue, IssueMonitorIssueState, IssueMonitorState};
use gwt_github::{Cache, IssueState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueMonitorDaemonPayload {
    pub event: String,
    pub payload: Value,
}

pub fn issue_monitor_daemon_payloads(
    monitor: &mut IssueMonitorState,
    gui_connected: bool,
) -> Vec<IssueMonitorDaemonPayload> {
    monitor.set_gui_connected(gui_connected);
    let mut payloads = Vec::new();
    if gui_connected {
        for request in monitor.take_pending_launch_requests() {
            payloads.push(IssueMonitorDaemonPayload {
                event: "launch_request".to_string(),
                payload: serde_json::json!({
                    "issue_number": request.issue_number,
                    "branch_name": request.branch_name,
                    "linked_issue_kind": request.linked_issue_kind,
                }),
            });
            payloads.push(IssueMonitorDaemonPayload {
                event: "toast".to_string(),
                payload: serde_json::json!({
                    "level": "info",
                    "message": "Issue Monitor launch requested",
                    "issue_number": request.issue_number,
                }),
            });
        }
    }

    payloads.extend([
        IssueMonitorDaemonPayload {
            event: "status".to_string(),
            payload: serde_json::to_value(monitor.status_view())
                .expect("issue monitor status serializes"),
        },
        IssueMonitorDaemonPayload {
            event: "inbox".to_string(),
            payload: serde_json::to_value(monitor.inbox.clone())
                .expect("issue monitor inbox serializes"),
        },
    ]);

    payloads
}

pub fn load_open_issue_monitor_candidates(
    owner: &str,
    repo: &str,
) -> Result<Vec<IssueMonitorIssue>, String> {
    let issues = gwt_git::issue::fetch_issues(owner, repo).map_err(|error| error.to_string())?;
    Ok(issues
        .into_iter()
        .map(|issue| IssueMonitorIssue {
            number: issue.number,
            title: issue.title,
            labels: issue.labels,
            state: if issue.state.eq_ignore_ascii_case("closed") {
                IssueMonitorIssueState::Closed
            } else {
                IssueMonitorIssueState::Open
            },
            body: issue.body,
            url: (!issue.url.is_empty()).then_some(issue.url),
        })
        .collect())
}

pub fn load_open_issue_monitor_candidates_for_repo_path(
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> Result<Vec<IssueMonitorIssue>, String> {
    match load_open_issue_monitor_candidates(owner, repo) {
        Ok(issues) => Ok(issues),
        Err(live_error) => {
            let cache_roots = [
                crate::issue_cache::issue_cache_root_for_repo_path(repo_path),
                Some(crate::issue_cache::issue_cache_root_for_repo_slug(
                    owner, repo,
                )),
            ];
            for cache_root in cache_roots.into_iter().flatten() {
                match load_cached_issue_monitor_candidates(&cache_root) {
                    Ok(issues) if !issues.is_empty() => return Ok(issues),
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            "issue monitor cache fallback failed for {}: {error}",
                            cache_root.display()
                        );
                    }
                }
            }
            Err(live_error)
        }
    }
}

/// Mark any active launched Issue whose work branch has a merged PR as
/// `Merged`, freeing the active slot. Skips the network call when nothing is
/// launched, and leaves work launched when the PR query fails (so a transient
/// error never closes the slot on a false signal).
pub fn reconcile_issue_monitor_merges(monitor: &mut IssueMonitorState, repo_path: &Path) {
    if monitor.active_launched_branches().is_empty() {
        return;
    }
    match gwt_git::pr_status::fetch_merged_pr_branches(repo_path) {
        Ok(merged_branches) => {
            let merged = monitor.reconcile_merged_branches(&merged_branches);
            if !merged.is_empty() {
                tracing::info!(
                    issues = ?merged,
                    "issue monitor marked merged work and freed active slots"
                );
            }
        }
        Err(error) => {
            tracing::debug!(
                error = %error,
                "issue monitor merge reconciliation skipped (PR query failed)"
            );
        }
    }
}

pub fn load_cached_issue_monitor_candidates(
    cache_root: &Path,
) -> Result<Vec<IssueMonitorIssue>, String> {
    if !cache_root.is_dir() {
        return Ok(Vec::new());
    }
    let cache = Cache::new(cache_root.to_path_buf());
    let mut issues = cache
        .list_entries()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|entry| IssueMonitorIssue {
            number: entry.snapshot.number.0,
            title: entry.snapshot.title,
            labels: entry.snapshot.labels,
            state: match entry.snapshot.state {
                IssueState::Open => IssueMonitorIssueState::Open,
                IssueState::Closed => IssueMonitorIssueState::Closed,
            },
            body: (!entry.snapshot.body.is_empty()).then_some(entry.snapshot.body),
            url: None,
        })
        .collect::<Vec<_>>();
    issues.sort_by_key(|issue| issue.number);
    Ok(issues)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRemoteResolutionError {
    CommandSpawnFailed(String),
    GitCommandFailed {
        status_code: Option<i32>,
        stderr: String,
    },
    OriginNotConfigured(String),
    NonGitHubOrigin(String),
    InvalidGitHubOrigin(String),
}

impl fmt::Display for GitHubRemoteResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandSpawnFailed(error) => {
                write!(f, "git remote get-url origin could not be started: {error}")
            }
            Self::GitCommandFailed {
                status_code,
                stderr,
            } => {
                let status = status_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                write!(
                    f,
                    "git remote get-url origin failed with exit status {status}: {stderr}"
                )
            }
            Self::OriginNotConfigured(detail) => {
                write!(f, "Git origin remote is not configured: {detail}")
            }
            Self::NonGitHubOrigin(remote_url) => {
                write!(f, "Git origin remote is not a GitHub URL: {remote_url}")
            }
            Self::InvalidGitHubOrigin(remote_url) => {
                write!(f, "GitHub origin remote URL is invalid: {remote_url}")
            }
        }
    }
}

impl std::error::Error for GitHubRemoteResolutionError {}

pub fn github_remote_owner_and_repo(
    repo_path: &Path,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    let output = gwt_core::process::hidden_command("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .map_err(|error| GitHubRemoteResolutionError::CommandSpawnFailed(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    github_remote_owner_and_repo_from_get_url_output(
        output.status.success(),
        output.status.code(),
        &stdout,
        &stderr,
    )
}

pub fn parse_github_remote_url(remote_url: &str) -> Option<(String, String)> {
    let path = remote_url
        .strip_prefix("https://github.com/")
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
        .or_else(|| remote_url.strip_prefix("git@github.com:"))
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn github_remote_owner_and_repo_from_get_url_output(
    success: bool,
    status_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    let stdout = stdout.trim();
    let stderr = cleaned_process_text(stderr);
    if !success {
        if stderr.to_ascii_lowercase().contains("no such remote") && stderr.contains("origin") {
            return Err(GitHubRemoteResolutionError::OriginNotConfigured(stderr));
        }
        return Err(GitHubRemoteResolutionError::GitCommandFailed {
            status_code,
            stderr,
        });
    }
    if stdout.is_empty() {
        return Err(GitHubRemoteResolutionError::OriginNotConfigured(
            "git remote get-url origin returned an empty URL".to_string(),
        ));
    }
    if let Some(owner_repo) = parse_github_remote_url(stdout) {
        return Ok(owner_repo);
    }
    if has_supported_github_remote_prefix(stdout) {
        return Err(GitHubRemoteResolutionError::InvalidGitHubOrigin(
            stdout.to_string(),
        ));
    }
    Err(GitHubRemoteResolutionError::NonGitHubOrigin(
        stdout.to_string(),
    ))
}

fn cleaned_process_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "no stderr".to_string()
    } else {
        trimmed.to_string()
    }
}

fn has_supported_github_remote_prefix(remote_url: &str) -> bool {
    [
        "https://github.com/",
        "http://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ]
    .iter()
    .any(|prefix| remote_url.starts_with(prefix))
}

#[allow(dead_code)]
fn _assert_inbox_item_is_send_sync(_: IssueMonitorInboxItem) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueMonitorConfig, MonitorInboxState};
    use gwt_github::{Cache, FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};

    fn issue(number: u64) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-improve".to_string()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
        }
    }

    fn github_issue(number: u64) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: String::new(),
            labels: vec![],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments: vec![],
        }
    }

    #[test]
    fn payloads_keep_queue_when_no_gui_is_connected() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_claimed(issue(42), "claim-a");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, false);

        assert!(payloads.iter().any(|payload| payload.event == "status"));
        assert!(payloads.iter().any(|payload| payload.event == "inbox"));
        assert!(!payloads
            .iter()
            .any(|payload| payload.event == "launch_request"));
        assert_eq!(monitor.queue_len(), 1);
        assert_eq!(monitor.active_issue_number(), None);
    }

    #[test]
    fn payloads_emit_launch_request_when_gui_is_connected() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);

        assert!(payloads.iter().any(|payload| {
            payload.event == "launch_request" && payload.payload["issue_number"] == 42
        }));
        assert_eq!(monitor.active_issue_number(), Some(42));
        assert_eq!(
            monitor.inbox_item(42).expect("inbox item").state,
            MonitorInboxState::Launching
        );
    }

    #[test]
    fn payloads_emit_launch_request_before_launching_snapshot_when_gui_is_connected() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);
        let launch_index = payloads
            .iter()
            .position(|payload| payload.event == "launch_request")
            .expect("launch request payload");
        let first_status_index = payloads
            .iter()
            .position(|payload| payload.event == "status")
            .expect("status payload");

        assert!(
            launch_index < first_status_index,
            "the agent window launch request must reach the GUI before the monitor renders Launching"
        );
    }

    #[test]
    fn payloads_emit_all_pending_launch_requests_when_parallel_capacity_allows() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        client.seed(github_issue(43));
        client.seed(github_issue(44));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 3,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.record_candidate(issue(43));
        monitor.record_candidate(issue(44));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);
        let launch_numbers: Vec<u64> = payloads
            .iter()
            .filter(|payload| payload.event == "launch_request")
            .filter_map(|payload| payload.payload["issue_number"].as_u64())
            .collect();

        assert_eq!(launch_numbers, vec![42, 43, 44]);
        assert_eq!(monitor.active_count(), 3);
    }

    #[test]
    fn cached_issue_candidates_load_from_issue_cache_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let mut spec = github_issue(3165);
        spec.title = "SPEC: Issue auto-improve monitor".to_string();
        spec.labels = vec!["gwt-spec".to_string()];
        let mut closed = github_issue(3000);
        closed.title = "Closed issue".to_string();
        closed.state = IssueState::Closed;
        cache.write_snapshot(&spec).expect("write spec");
        cache.write_snapshot(&closed).expect("write closed issue");

        let candidates = load_cached_issue_monitor_candidates(dir.path()).expect("load cache");

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].number, 3000);
        assert_eq!(candidates[0].state, IssueMonitorIssueState::Closed);
        assert_eq!(candidates[1].number, 3165);
        assert_eq!(candidates[1].title, "SPEC: Issue auto-improve monitor");
        assert_eq!(candidates[1].labels, vec!["gwt-spec"]);
        assert_eq!(candidates[1].state, IssueMonitorIssueState::Open);
    }

    #[test]
    fn parse_github_remote_url_accepts_https_and_ssh_forms() {
        assert_eq!(
            parse_github_remote_url("https://github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("https://example.com/owner/repo"),
            None
        );
    }

    #[test]
    fn github_remote_output_resolves_valid_origin() {
        assert_eq!(
            github_remote_owner_and_repo_from_get_url_output(
                true,
                Some(0),
                "https://github.com/owner/repo.git\n",
                ""
            )
            .expect("valid origin"),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn github_remote_output_classifies_missing_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            false,
            Some(2),
            "",
            "error: No such remote 'origin'\n",
        )
        .expect_err("missing origin");

        assert_eq!(
            error.to_string(),
            "Git origin remote is not configured: error: No such remote 'origin'"
        );
    }

    #[test]
    fn github_remote_output_classifies_git_failure() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            false,
            Some(128),
            "",
            "fatal: not a git repository\n",
        )
        .expect_err("git failure");

        assert_eq!(
            error.to_string(),
            "git remote get-url origin failed with exit status 128: fatal: not a git repository"
        );
    }

    #[test]
    fn github_remote_output_classifies_non_github_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            true,
            Some(0),
            "https://example.com/owner/repo.git\n",
            "",
        )
        .expect_err("non GitHub origin");

        assert_eq!(
            error.to_string(),
            "Git origin remote is not a GitHub URL: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn github_remote_output_classifies_invalid_github_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            true,
            Some(0),
            "https://github.com/owner\n",
            "",
        )
        .expect_err("invalid GitHub origin");

        assert_eq!(
            error.to_string(),
            "GitHub origin remote URL is invalid: https://github.com/owner"
        );
    }
}
