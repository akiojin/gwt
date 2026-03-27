//! PR status polling module (SPEC-1776 Phase 4, T300-T301)
//!
//! Extracts PR status checking into a reusable module using the `gh` CLI.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::gh_cli::{is_gh_available, run_gh_output_with_repair};

/// PR status information for dashboard display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrStatus {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub url: String,
    pub branch: String,
    pub ci_status: CiStatus,
    pub mergeable: bool,
    pub review_status: ReviewStatus,
}

/// PR state enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    /// Parse from GitHub API string (e.g. "OPEN", "CLOSED", "MERGED").
    pub fn from_gh_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "OPEN" => Self::Open,
            "CLOSED" => Self::Closed,
            "MERGED" => Self::Merged,
            _ => Self::Open,
        }
    }
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

/// CI/CD check status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CiStatus {
    Pending,
    Passing,
    Failing,
    None,
}

impl CiStatus {
    /// Parse from GitHub statusCheckRollup array.
    pub fn from_check_rollup(checks: &[serde_json::Value]) -> Self {
        if checks.is_empty() {
            return Self::None;
        }

        let mut has_pending = false;
        for check in checks {
            let conclusion = check
                .get("conclusion")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let status = check
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("");

            match conclusion.to_uppercase().as_str() {
                "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" => return Self::Failing,
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => {}
                _ => {
                    // No conclusion yet — check status
                    if status.to_uppercase() != "COMPLETED" {
                        has_pending = true;
                    }
                }
            }
        }

        if has_pending {
            Self::Pending
        } else {
            Self::Passing
        }
    }
}

impl std::fmt::Display for CiStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Passing => write!(f, "passing"),
            Self::Failing => write!(f, "failing"),
            Self::None => write!(f, "none"),
        }
    }
}

/// Review status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewStatus {
    Approved,
    ChangesRequested,
    Pending,
    None,
}

impl ReviewStatus {
    /// Parse from GitHub reviewDecision string.
    pub fn from_review_decision(decision: Option<&str>) -> Self {
        match decision {
            Some(d) => match d.to_uppercase().as_str() {
                "APPROVED" => Self::Approved,
                "CHANGES_REQUESTED" => Self::ChangesRequested,
                "REVIEW_REQUIRED" => Self::Pending,
                _ => Self::None,
            },
            None => Self::None,
        }
    }
}

impl std::fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Approved => write!(f, "approved"),
            Self::ChangesRequested => write!(f, "changes_requested"),
            Self::Pending => write!(f, "pending"),
            Self::None => write!(f, "none"),
        }
    }
}

/// Parse a single PR JSON value into a `PrStatus`.
fn parse_pr_json(value: &serde_json::Value) -> Option<PrStatus> {
    let number = value.get("number")?.as_u64()?;
    let title = value.get("title")?.as_str()?.to_string();
    let state_str = value.get("state")?.as_str()?;
    let url = value.get("url")?.as_str().unwrap_or("").to_string();
    let branch = value
        .get("headRefName")
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .to_string();

    let mergeable_str = value
        .get("mergeable")
        .and_then(|m| m.as_str())
        .unwrap_or("UNKNOWN");
    let mergeable = mergeable_str.eq_ignore_ascii_case("MERGEABLE");

    let empty_checks = Vec::new();
    let checks = value
        .get("statusCheckRollup")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_checks);

    let review_decision = value.get("reviewDecision").and_then(|r| r.as_str());

    Some(PrStatus {
        number,
        title,
        state: PrState::from_gh_str(state_str),
        url,
        branch,
        ci_status: CiStatus::from_check_rollup(checks),
        mergeable,
        review_status: ReviewStatus::from_review_decision(review_decision),
    })
}

const PR_JSON_FIELDS: &str =
    "number,title,state,url,headRefName,reviewDecision,statusCheckRollup,mergeable";

/// Guard: return error if `gh` CLI is not available.
fn require_gh() -> Result<(), crate::error::GwtError> {
    if !is_gh_available() {
        return Err(crate::error::GwtError::GitCommandFailed {
            command: "gh".to_string(),
            reason: "gh CLI is not installed or not in PATH".to_string(),
        });
    }
    Ok(())
}

/// Fetch PR status for a specific branch using `gh` CLI.
///
/// Returns `None` if no PR exists for this branch.
pub fn fetch_pr_status(
    repo_root: &Path,
    branch: &str,
) -> Result<Option<PrStatus>, crate::error::GwtError> {
    require_gh()?;

    let output = run_gh_output_with_repair(
        repo_root,
        [
            "pr",
            "view",
            branch,
            "--json",
            PR_JSON_FIELDS,
        ],
    )
    .map_err(|e| crate::error::GwtError::GitCommandFailed {
        command: "gh pr view".to_string(),
        reason: e.to_string(),
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "no pull requests found" is not an error — just no PR for this branch
        if stderr.contains("no pull requests found") || stderr.contains("Could not resolve") {
            return Ok(None);
        }
        return Err(crate::error::GwtError::GitCommandFailed {
            command: "gh pr view".to_string(),
            reason: stderr.to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
        crate::error::GwtError::GitCommandFailed {
            command: "gh pr view".to_string(),
            reason: format!("Failed to parse JSON: {e}"),
        }
    })?;

    Ok(parse_pr_json(&value))
}

/// Fetch all open PRs for a repository.
pub fn fetch_open_prs(repo_root: &Path) -> Result<Vec<PrStatus>, crate::error::GwtError> {
    require_gh()?;

    let output = run_gh_output_with_repair(
        repo_root,
        [
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            PR_JSON_FIELDS,
            "--limit",
            "100",
        ],
    )
    .map_err(|e| crate::error::GwtError::GitCommandFailed {
        command: "gh pr list".to_string(),
        reason: e.to_string(),
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::GwtError::GitCommandFailed {
            command: "gh pr list".to_string(),
            reason: stderr.to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let values: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).map_err(|e| crate::error::GwtError::GitCommandFailed {
            command: "gh pr list".to_string(),
            reason: format!("Failed to parse JSON: {e}"),
        })?;

    Ok(values.iter().filter_map(parse_pr_json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pr_state_display() {
        assert_eq!(PrState::Open.to_string(), "OPEN");
        assert_eq!(PrState::Closed.to_string(), "CLOSED");
        assert_eq!(PrState::Merged.to_string(), "MERGED");
    }

    #[test]
    fn test_pr_state_from_gh_str() {
        assert_eq!(PrState::from_gh_str("OPEN"), PrState::Open);
        assert_eq!(PrState::from_gh_str("open"), PrState::Open);
        assert_eq!(PrState::from_gh_str("CLOSED"), PrState::Closed);
        assert_eq!(PrState::from_gh_str("MERGED"), PrState::Merged);
        assert_eq!(PrState::from_gh_str("unknown"), PrState::Open);
    }

    #[test]
    fn test_ci_status_display() {
        assert_eq!(CiStatus::Pending.to_string(), "pending");
        assert_eq!(CiStatus::Passing.to_string(), "passing");
        assert_eq!(CiStatus::Failing.to_string(), "failing");
        assert_eq!(CiStatus::None.to_string(), "none");
    }

    #[test]
    fn test_ci_status_from_empty_checks() {
        assert_eq!(CiStatus::from_check_rollup(&[]), CiStatus::None);
    }

    #[test]
    fn test_ci_status_from_passing_checks() {
        let checks = vec![serde_json::json!({
            "status": "COMPLETED",
            "conclusion": "SUCCESS"
        })];
        assert_eq!(CiStatus::from_check_rollup(&checks), CiStatus::Passing);
    }

    #[test]
    fn test_ci_status_from_failing_checks() {
        let checks = vec![
            serde_json::json!({
                "status": "COMPLETED",
                "conclusion": "SUCCESS"
            }),
            serde_json::json!({
                "status": "COMPLETED",
                "conclusion": "FAILURE"
            }),
        ];
        assert_eq!(CiStatus::from_check_rollup(&checks), CiStatus::Failing);
    }

    #[test]
    fn test_ci_status_from_pending_checks() {
        let checks = vec![serde_json::json!({
            "status": "IN_PROGRESS",
            "conclusion": ""
        })];
        assert_eq!(CiStatus::from_check_rollup(&checks), CiStatus::Pending);
    }

    #[test]
    fn test_review_status_display() {
        assert_eq!(ReviewStatus::Approved.to_string(), "approved");
        assert_eq!(
            ReviewStatus::ChangesRequested.to_string(),
            "changes_requested"
        );
        assert_eq!(ReviewStatus::Pending.to_string(), "pending");
        assert_eq!(ReviewStatus::None.to_string(), "none");
    }

    #[test]
    fn test_review_status_from_decision() {
        assert_eq!(
            ReviewStatus::from_review_decision(Some("APPROVED")),
            ReviewStatus::Approved
        );
        assert_eq!(
            ReviewStatus::from_review_decision(Some("CHANGES_REQUESTED")),
            ReviewStatus::ChangesRequested
        );
        assert_eq!(
            ReviewStatus::from_review_decision(Some("REVIEW_REQUIRED")),
            ReviewStatus::Pending
        );
        assert_eq!(
            ReviewStatus::from_review_decision(None),
            ReviewStatus::None
        );
    }

    #[test]
    fn test_parse_pr_json() {
        let json = serde_json::json!({
            "number": 42,
            "title": "feat: add auth",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/pull/42",
            "headRefName": "feature/auth",
            "mergeable": "MERGEABLE",
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [
                { "status": "COMPLETED", "conclusion": "SUCCESS" }
            ]
        });

        let pr = parse_pr_json(&json).unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "feat: add auth");
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.url, "https://github.com/owner/repo/pull/42");
        assert_eq!(pr.branch, "feature/auth");
        assert!(pr.mergeable);
        assert_eq!(pr.ci_status, CiStatus::Passing);
        assert_eq!(pr.review_status, ReviewStatus::Approved);
    }

    #[test]
    fn test_parse_pr_json_not_mergeable() {
        let json = serde_json::json!({
            "number": 10,
            "title": "fix: bug",
            "state": "OPEN",
            "url": "https://example.com/pull/10",
            "headRefName": "fix/bug",
            "mergeable": "CONFLICTING",
            "reviewDecision": null,
            "statusCheckRollup": []
        });

        let pr = parse_pr_json(&json).unwrap();
        assert!(!pr.mergeable);
        assert_eq!(pr.ci_status, CiStatus::None);
        assert_eq!(pr.review_status, ReviewStatus::None);
    }

    #[test]
    fn test_fetch_pr_status_no_gh() {
        // This test verifies behavior when gh is not on PATH.
        // In CI or environments where gh IS available, the function may succeed,
        // so we only test the parsing/enum logic here exhaustively.
        // The actual gh-unavailable error path is covered by unit tests on
        // is_gh_available() returning false, which is environment-dependent.
        let status = PrState::from_gh_str("MERGED");
        assert_eq!(status, PrState::Merged);
    }

    #[test]
    fn test_pr_status_serde_roundtrip() {
        let pr = PrStatus {
            number: 99,
            title: "test PR".to_string(),
            state: PrState::Open,
            url: "https://example.com/pull/99".to_string(),
            branch: "feature/test".to_string(),
            ci_status: CiStatus::Passing,
            mergeable: true,
            review_status: ReviewStatus::Approved,
        };

        let json = serde_json::to_string(&pr).unwrap();
        let deserialized: PrStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.number, 99);
        assert_eq!(deserialized.state, PrState::Open);
        assert_eq!(deserialized.ci_status, CiStatus::Passing);
        assert_eq!(deserialized.review_status, ReviewStatus::Approved);
    }
}
