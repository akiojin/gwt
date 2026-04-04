//! Pull Request status tracking via GitHub CLI

use std::path::Path;

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// Pull Request state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "OPEN"),
            Self::Closed => write!(f, "CLOSED"),
            Self::Merged => write!(f, "MERGED"),
        }
    }
}

/// Status of a Pull Request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrStatus {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub url: String,
    /// Overall CI status: "SUCCESS", "FAILURE", "PENDING", or "UNKNOWN".
    pub ci_status: String,
    /// Whether the PR can be merged: "MERGEABLE", "CONFLICTING", "UNKNOWN".
    pub mergeable: String,
    /// Review verdict: "APPROVED", "CHANGES_REQUESTED", "REVIEW_REQUIRED", or "UNKNOWN".
    pub review_status: String,
}

/// Fetch the status of a PR by number using `gh pr view --json`.
///
/// The `repo_slug` should be in "owner/repo" format.
pub fn fetch_pr_status(repo_slug: &str, number: u64) -> Result<PrStatus> {
    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            repo_slug,
            "--json",
            "number,title,state,url,mergeable,statusCheckRollup,reviewDecision",
        ])
        .output()
        .map_err(|e| GwtError::Git(format!("gh pr view: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("gh pr view: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_status_json(&stdout)
}

/// Parse `gh pr view --json` output.
pub fn parse_pr_status_json(json: &str) -> Result<PrStatus> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| GwtError::Other(e.to_string()))?;

    let number = v["number"].as_u64().unwrap_or(0);
    let title = v["title"].as_str().unwrap_or("").to_string();
    let state_str = v["state"].as_str().unwrap_or("OPEN");
    let state = match state_str {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    };
    let url = v["url"].as_str().unwrap_or("").to_string();
    let mergeable = v["mergeable"].as_str().unwrap_or("UNKNOWN").to_string();

    // Determine CI status from statusCheckRollup
    let ci_status = v["statusCheckRollup"]
        .as_array()
        .map(|checks| {
            if checks.is_empty() {
                return "UNKNOWN".to_string();
            }
            let any_failure = checks.iter().any(|c| {
                c["conclusion"].as_str() == Some("FAILURE")
                    || c["conclusion"].as_str() == Some("failure")
            });
            let any_pending = checks.iter().any(|c| {
                c["status"].as_str() == Some("IN_PROGRESS")
                    || c["status"].as_str() == Some("QUEUED")
                    || c["conclusion"].is_null()
            });
            if any_failure {
                "FAILURE".to_string()
            } else if any_pending {
                "PENDING".to_string()
            } else {
                "SUCCESS".to_string()
            }
        })
        .unwrap_or_else(|| "UNKNOWN".to_string());

    let review_status = v["reviewDecision"]
        .as_str()
        .unwrap_or("UNKNOWN")
        .to_string();

    Ok(PrStatus {
        number,
        title,
        state,
        url,
        ci_status,
        mergeable,
        review_status,
    })
}

/// Fetch a list of open PRs for the repository at `repo_path`.
///
/// Uses the GitHub CLI's `pr list --json` surface as the primary path and
/// falls back to the REST pulls endpoint when that surface is unavailable.
pub fn fetch_pr_list(repo_path: &Path) -> Result<Vec<PrStatus>> {
    fetch_pr_list_with(repo_path, run_gh_command)
}

/// Parse `gh pr list --json` output (a JSON array) into a `Vec<PrStatus>`.
pub fn parse_pr_list_json(json: &str) -> Result<Vec<PrStatus>> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr list JSON: {e}")))?;

    let mut results = Vec::with_capacity(arr.len());
    for v in &arr {
        // Reuse the single-PR parser by serializing back to string
        let single_json = serde_json::to_string(v).map_err(|e| GwtError::Other(e.to_string()))?;
        results.push(parse_pr_status_json(&single_json)?);
    }
    Ok(results)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GhCliOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

fn fetch_pr_list_with<F>(repo_path: &Path, mut run_gh: F) -> Result<Vec<PrStatus>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let primary = run_gh(
        repo_path,
        &[
            "pr",
            "list",
            "--json",
            "number,title,state,url,statusCheckRollup,mergeable,reviewDecision",
            "--limit",
            "20",
        ],
    );

    if let Ok(output) = primary {
        if output.success {
            if let Ok(prs) = parse_pr_list_json(&output.stdout) {
                return Ok(prs);
            }
        }
    }

    let rest = run_gh(repo_path, &["api", "repos/{owner}/{repo}/pulls?state=open&per_page=20"])?;
    if !rest.success {
        return Err(GwtError::Git(format!("gh api pulls: {}", rest.stderr.trim())));
    }
    parse_rest_pr_list_json(&rest.stdout)
}

fn run_gh_command(repo_path: &Path, args: &[&str]) -> Result<GhCliOutput> {
    let output = std::process::Command::new("gh")
        .args(args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("gh {}: {e}", args.join(" "))))?;

    Ok(GhCliOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn parse_rest_pr_list_json(json: &str) -> Result<Vec<PrStatus>> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| GwtError::Other(format!("gh api pulls JSON: {e}")))?;

    Ok(arr
        .into_iter()
        .map(|v| {
            let state = match v.get("state").and_then(|s| s.as_str()).unwrap_or("open") {
                "closed" => PrState::Closed,
                "merged" => PrState::Merged,
                _ => PrState::Open,
            };
            PrStatus {
                number: v.get("number").and_then(|n| n.as_u64()).unwrap_or(0),
                title: v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                state,
                url: v
                    .get("html_url")
                    .or_else(|| v.get("url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                ci_status: "UNKNOWN".to_string(),
                mergeable: "UNKNOWN".to_string(),
                review_status: "UNKNOWN".to_string(),
            }
        })
        .collect())
}

// ── Extended PR check report ──

/// PR status check states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Passing,
    Failing,
    Pending,
    Unknown,
}

/// Merge readiness states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    Ready,
    Blocked,
    Conflicts,
    Unknown,
}

/// Review states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Approved,
    ChangesRequested,
    Pending,
    Unknown,
}

/// Extended PR status report.
#[derive(Debug, Clone)]
pub struct PrCheckReport {
    pub ci: CiStatus,
    pub merge: MergeStatus,
    pub review: ReviewStatus,
    pub summary: String,
}

/// Generate an extended PR status report by inspecting the repository.
///
/// Runs `gh pr view` to gather CI, merge, and review states. Falls back
/// to `Unknown` states when `gh` is unavailable or the repo has no open PR.
pub fn pr_check_report(repo_path: &Path) -> Result<PrCheckReport> {
    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "statusCheckRollup,mergeable,reviewDecision,state,title",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("gh pr view: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(PrCheckReport {
            ci: CiStatus::Unknown,
            merge: MergeStatus::Unknown,
            review: ReviewStatus::Unknown,
            summary: format!("No open PR or gh error: {}", stderr.trim()),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_check_report_json(&stdout)
}

/// Parse `gh pr view --json` output into an extended PR check report.
pub fn parse_pr_check_report_json(json: &str) -> Result<PrCheckReport> {
    let json: serde_json::Value =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr view JSON: {e}")))?;

    let ci = match json.get("statusCheckRollup") {
        Some(serde_json::Value::Array(checks)) => {
            let all_pass = checks.iter().all(|c| {
                c.get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "SUCCESS" || s == "NEUTRAL" || s == "SKIPPED")
            });
            let any_fail = checks.iter().any(|c| {
                c.get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "FAILURE" || s == "CANCELLED" || s == "TIMED_OUT")
            });
            if checks.is_empty() {
                CiStatus::Pending
            } else if any_fail {
                CiStatus::Failing
            } else if all_pass {
                CiStatus::Passing
            } else {
                CiStatus::Pending
            }
        }
        _ => CiStatus::Unknown,
    };

    let merge = match json.get("mergeable").and_then(|v| v.as_str()) {
        Some("MERGEABLE") => MergeStatus::Ready,
        Some("CONFLICTING") => MergeStatus::Conflicts,
        Some(_) => MergeStatus::Blocked,
        None => MergeStatus::Unknown,
    };

    let review = match json.get("reviewDecision").and_then(|v| v.as_str()) {
        Some("APPROVED") => ReviewStatus::Approved,
        Some("CHANGES_REQUESTED") => ReviewStatus::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewStatus::Pending,
        _ => ReviewStatus::Unknown,
    };

    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(untitled)");

    let summary = format!(
        "PR: {} | CI: {:?} | Merge: {:?} | Review: {:?}",
        title, ci, merge, review
    );

    Ok(PrCheckReport {
        ci,
        merge,
        review,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_status_open() {
        let json = r#"{
            "number": 123,
            "title": "Add feature",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/pull/123",
            "mergeable": "MERGEABLE",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"}
            ],
            "reviewDecision": "APPROVED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.number, 123);
        assert_eq!(pr.title, "Add feature");
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.ci_status, "SUCCESS");
        assert_eq!(pr.mergeable, "MERGEABLE");
        assert_eq!(pr.review_status, "APPROVED");
    }

    #[test]
    fn parse_pr_status_merged() {
        let json = r#"{
            "number": 456,
            "title": "Fix bug",
            "state": "MERGED",
            "url": "https://github.com/owner/repo/pull/456",
            "mergeable": "UNKNOWN",
            "statusCheckRollup": [],
            "reviewDecision": "APPROVED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.state, PrState::Merged);
        assert_eq!(pr.ci_status, "UNKNOWN");
    }

    #[test]
    fn parse_pr_status_ci_failure() {
        let json = r#"{
            "number": 789,
            "title": "Broken PR",
            "state": "OPEN",
            "url": "",
            "mergeable": "CONFLICTING",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "lint", "status": "COMPLETED", "conclusion": "FAILURE"}
            ],
            "reviewDecision": "CHANGES_REQUESTED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.ci_status, "FAILURE");
        assert_eq!(pr.mergeable, "CONFLICTING");
        assert_eq!(pr.review_status, "CHANGES_REQUESTED");
    }

    #[test]
    fn parse_pr_status_ci_pending() {
        let json = r#"{
            "number": 101,
            "title": "WIP",
            "state": "OPEN",
            "url": "",
            "mergeable": "UNKNOWN",
            "statusCheckRollup": [
                {"name": "ci", "status": "IN_PROGRESS", "conclusion": null}
            ],
            "reviewDecision": ""
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.ci_status, "PENDING");
    }

    #[test]
    fn parse_pr_status_invalid_json() {
        let result = parse_pr_status_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_pr_check_report_structured_statuses() {
        let json = r#"{
            "title": "Add feature",
            "mergeable": "MERGEABLE",
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "lint", "status": "COMPLETED", "conclusion": "NEUTRAL"}
            ]
        }"#;

        let report = parse_pr_check_report_json(json).unwrap();

        assert_eq!(report.ci, CiStatus::Passing);
        assert_eq!(report.merge, MergeStatus::Ready);
        assert_eq!(report.review, ReviewStatus::Approved);
        assert_eq!(
            report.summary,
            "PR: Add feature | CI: Passing | Merge: Ready | Review: Approved"
        );
    }

    #[test]
    fn parse_pr_check_report_empty_checks() {
        let json = r#"{
            "title": "Waiting on CI",
            "mergeable": "CONFLICTING",
            "reviewDecision": "REVIEW_REQUIRED",
            "statusCheckRollup": []
        }"#;

        let report = parse_pr_check_report_json(json).unwrap();

        assert_eq!(report.ci, CiStatus::Pending);
        assert_eq!(report.merge, MergeStatus::Conflicts);
        assert_eq!(report.review, ReviewStatus::Pending);
        assert_eq!(
            report.summary,
            "PR: Waiting on CI | CI: Pending | Merge: Conflicts | Review: Pending"
        );
    }

    #[test]
    fn parse_pr_list_empty() {
        let prs = parse_pr_list_json("[]").unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn parse_pr_list_multiple() {
        let json = r#"[
            {
                "number": 1,
                "title": "First PR",
                "state": "OPEN",
                "url": "https://github.com/o/r/pull/1",
                "mergeable": "MERGEABLE",
                "statusCheckRollup": [],
                "reviewDecision": "APPROVED"
            },
            {
                "number": 2,
                "title": "Second PR",
                "state": "OPEN",
                "url": "https://github.com/o/r/pull/2",
                "mergeable": "CONFLICTING",
                "statusCheckRollup": [
                    {"name": "ci", "status": "COMPLETED", "conclusion": "FAILURE"}
                ],
                "reviewDecision": "CHANGES_REQUESTED"
            }
        ]"#;

        let prs = parse_pr_list_json(json).unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0].number, 1);
        assert_eq!(prs[0].title, "First PR");
        assert_eq!(prs[1].number, 2);
        assert_eq!(prs[1].ci_status, "FAILURE");
    }

    #[test]
    fn parse_pr_list_invalid_json() {
        assert!(parse_pr_list_json("not json").is_err());
    }

    #[test]
    fn parse_rest_pr_list_json_sets_missing_ci_merge_review_fields_to_unknown() {
        let json = r#"[
            {
                "number": 11,
                "title": "REST fallback PR",
                "state": "open",
                "html_url": "https://github.com/o/r/pull/11"
            }
        ]"#;

        let prs = parse_rest_pr_list_json(json).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 11);
        assert_eq!(prs[0].title, "REST fallback PR");
        assert_eq!(prs[0].state, PrState::Open);
        assert_eq!(prs[0].url, "https://github.com/o/r/pull/11");
        assert_eq!(prs[0].ci_status, "UNKNOWN");
        assert_eq!(prs[0].mergeable, "UNKNOWN");
        assert_eq!(prs[0].review_status, "UNKNOWN");
    }

    #[test]
    fn fetch_pr_list_with_uses_primary_pr_list_when_available() {
        let repo_path = Path::new("/tmp/repo");
        let mut calls = Vec::new();

        let prs = fetch_pr_list_with(repo_path, |path, args| {
            assert_eq!(path, repo_path);
            calls.push(args[..2].join(" "));
            match args {
                ["pr", "list", ..] => Ok(GhCliOutput {
                    success: true,
                    stdout: r#"[
                        {
                            "number": 7,
                            "title": "Primary transport",
                            "state": "OPEN",
                            "url": "https://github.com/o/r/pull/7",
                            "mergeable": "MERGEABLE",
                            "statusCheckRollup": [],
                            "reviewDecision": "APPROVED"
                        }
                    ]"#
                    .to_string(),
                    stderr: String::new(),
                }),
                other => panic!("unexpected gh invocation: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(calls, vec!["pr list"]);
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].review_status, "APPROVED");
    }

    #[test]
    fn fetch_pr_list_with_falls_back_to_rest_when_pr_list_call_fails() {
        let repo_path = Path::new("/tmp/repo");
        let mut calls = Vec::new();

        let prs = fetch_pr_list_with(repo_path, |path, args| {
            assert_eq!(path, repo_path);
            calls.push(args[..2].join(" "));
            match args {
                ["pr", "list", ..] => Ok(GhCliOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "pr list unavailable".to_string(),
                }),
                ["api", "repos/{owner}/{repo}/pulls?state=open&per_page=20"] => Ok(GhCliOutput {
                    success: true,
                    stdout: r#"[
                        {
                            "number": 21,
                            "title": "REST fallback",
                            "state": "open",
                            "html_url": "https://github.com/o/r/pull/21"
                        }
                    ]"#
                    .to_string(),
                    stderr: String::new(),
                }),
                other => panic!("unexpected gh invocation: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(
            calls,
            vec!["pr list", "api repos/{owner}/{repo}/pulls?state=open&per_page=20"]
        );
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 21);
        assert_eq!(prs[0].ci_status, "UNKNOWN");
    }

    #[test]
    fn pr_state_display() {
        assert_eq!(PrState::Open.to_string(), "OPEN");
        assert_eq!(PrState::Closed.to_string(), "CLOSED");
        assert_eq!(PrState::Merged.to_string(), "MERGED");
    }
}
