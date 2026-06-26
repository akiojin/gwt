use std::path::Path;

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

/// Why [`resolve_github_remote`] could not produce a GitHub `(owner, repo)`.
///
/// The Issue Monitor previously collapsed every one of these into the single
/// opaque banner "GitHub origin remote is unavailable", which made a transient
/// process-start failure indistinguishable from an origin that genuinely is not
/// a GitHub remote (Issue #3190). Keep the variants distinct so the scan error
/// names the real cause.
#[derive(Debug)]
pub enum RemoteResolveError {
    /// `git remote get-url origin` could not be started at all — e.g. `git` is
    /// not on `PATH`, or the process/file-descriptor table is exhausted.
    Spawn(String),
    /// `git` ran but exited non-zero. `stderr` carries the real reason (no
    /// `origin` remote configured, not a git repository, lock contention, …).
    GitFailed { code: Option<i32>, stderr: String },
    /// `origin` resolved, but its URL is not a recognizable GitHub URL.
    NotGitHub { url: String },
}

impl RemoteResolveError {
    /// English, user-facing message recorded as the Issue Monitor scan error.
    /// (TUI/GUI surfaces are English-only.)
    pub fn user_message(&self) -> String {
        match self {
            RemoteResolveError::Spawn(detail) => {
                format!("Could not run `git remote get-url origin` (git failed to start): {detail}")
            }
            RemoteResolveError::GitFailed { code, stderr } => {
                let code = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                let stderr = stderr.trim();
                if stderr.is_empty() {
                    format!("`git remote get-url origin` failed (exit {code})")
                } else {
                    format!("`git remote get-url origin` failed (exit {code}): {stderr}")
                }
            }
            RemoteResolveError::NotGitHub { url } => {
                format!("origin remote is not a GitHub URL: {url}")
            }
        }
    }
}

/// Resolve the GitHub `(owner, repo)` for the repository at `repo_path`,
/// surfacing the specific failure mode instead of a bare `None`.
pub fn resolve_github_remote(repo_path: &Path) -> Result<(String, String), RemoteResolveError> {
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Git,
        "git",
        &["remote", "get-url", "origin"],
        gwt_core::process_console::SpawnOptions::new("git remote get-url origin")
            .current_dir(repo_path),
    )
    .map_err(|error| RemoteResolveError::Spawn(error.to_string()))?;
    classify_github_remote(
        output.success(),
        output.exit_code,
        &output.stdout,
        &output.stderr,
    )
}

/// Pure classifier for a `git remote get-url origin` result. Split out so the
/// failure-mode → message mapping is unit-testable without spawning git.
pub fn classify_github_remote(
    success: bool,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<(String, String), RemoteResolveError> {
    if !success {
        return Err(RemoteResolveError::GitFailed {
            code: exit_code,
            stderr: stderr.trim().to_string(),
        });
    }
    let url = stdout.trim();
    parse_github_remote_url(url).ok_or_else(|| RemoteResolveError::NotGitHub {
        url: url.to_string(),
    })
}

pub fn github_remote_owner_and_repo(repo_path: &Path) -> Option<(String, String)> {
    resolve_github_remote(repo_path).ok()
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
    fn classify_github_remote_ok_for_github_url() {
        assert_eq!(
            classify_github_remote(true, Some(0), "https://github.com/akiojin/gwt\n", "")
                .expect("github url resolves"),
            ("akiojin".to_string(), "gwt".to_string())
        );
    }

    #[test]
    fn classify_github_remote_non_github_url_is_not_github_error() {
        let err = classify_github_remote(true, Some(0), "https://gitlab.com/x/y\n", "")
            .expect_err("non-github url is rejected");
        assert!(matches!(err, RemoteResolveError::NotGitHub { .. }));
        let msg = err.user_message();
        assert!(msg.contains("not a GitHub"), "msg: {msg}");
        assert!(
            msg.contains("gitlab.com"),
            "msg should name the offending url: {msg}"
        );
    }

    #[test]
    fn classify_github_remote_missing_origin_is_git_failed_with_stderr() {
        let err = classify_github_remote(false, Some(2), "", "error: No such remote 'origin'\n")
            .expect_err("missing origin fails");
        assert!(matches!(
            err,
            RemoteResolveError::GitFailed { code: Some(2), .. }
        ));
        let msg = err.user_message();
        assert!(msg.contains("No such remote 'origin'"), "msg: {msg}");
        assert!(msg.contains('2'), "msg should include the exit code: {msg}");
    }

    #[test]
    fn classify_github_remote_not_a_repo_is_git_failed() {
        let err = classify_github_remote(false, Some(128), "", "fatal: not a git repository\n")
            .expect_err("non-repo fails");
        assert!(matches!(
            err,
            RemoteResolveError::GitFailed {
                code: Some(128),
                ..
            }
        ));
        assert!(err.user_message().contains("not a git repository"));
    }

    #[test]
    fn remote_resolve_error_messages_are_distinct_per_failure_mode() {
        // The historical bug collapsed spawn failure, git non-zero exit, and a
        // non-GitHub origin URL into the single opaque string "GitHub origin
        // remote is unavailable". Each failure mode must now map to a different,
        // self-explaining message.
        let spawn = RemoteResolveError::Spawn("No such file or directory (os error 2)".to_string())
            .user_message();
        let git_failed = RemoteResolveError::GitFailed {
            code: Some(128),
            stderr: "fatal: not a git repository".to_string(),
        }
        .user_message();
        let not_github = RemoteResolveError::NotGitHub {
            url: "https://gitlab.com/x/y".to_string(),
        }
        .user_message();

        assert_ne!(spawn, git_failed);
        assert_ne!(git_failed, not_github);
        assert_ne!(spawn, not_github);
        // A spawn failure (git could not start, e.g. fd exhaustion) must read as
        // a process-start problem, never as "origin is not a GitHub remote".
        assert!(
            spawn.contains("failed to start"),
            "spawn message should name a start failure: {spawn}"
        );
    }
}
