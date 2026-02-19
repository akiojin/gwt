//! Project issue definitions for agent mode

use serde::{Deserialize, Serialize};

use super::coordinator::CoordinatorState;
use super::task::Task;

/// Issue lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Pending,
    Planned,
    InProgress,
    CiFail,
    Completed,
    Failed,
}

/// A project issue linked to a GitHub Issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIssue {
    pub id: String,
    pub github_issue_number: u64,
    pub github_issue_url: String,
    pub title: String,
    pub status: IssueStatus,
    pub coordinator: Option<CoordinatorState>,
    pub tasks: Vec<Task>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::coordinator::{CoordinatorState, CoordinatorStatus};
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_issue_status_serialize_pending() {
        let json = serde_json::to_string(&IssueStatus::Pending).unwrap();
        assert_eq!(json, r#""pending""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::Pending);
    }

    #[test]
    fn test_issue_status_serialize_planned() {
        let json = serde_json::to_string(&IssueStatus::Planned).unwrap();
        assert_eq!(json, r#""planned""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::Planned);
    }

    #[test]
    fn test_issue_status_serialize_in_progress() {
        let json = serde_json::to_string(&IssueStatus::InProgress).unwrap();
        assert_eq!(json, r#""in_progress""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::InProgress);
    }

    #[test]
    fn test_issue_status_serialize_ci_fail() {
        let json = serde_json::to_string(&IssueStatus::CiFail).unwrap();
        assert_eq!(json, r#""ci_fail""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::CiFail);
    }

    #[test]
    fn test_issue_status_serialize_completed() {
        let json = serde_json::to_string(&IssueStatus::Completed).unwrap();
        assert_eq!(json, r#""completed""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::Completed);
    }

    #[test]
    fn test_issue_status_serialize_failed() {
        let json = serde_json::to_string(&IssueStatus::Failed).unwrap();
        assert_eq!(json, r#""failed""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::Failed);
    }

    #[test]
    fn test_project_issue_roundtrip() {
        let issue = ProjectIssue {
            id: "issue-10".to_string(),
            github_issue_number: 10,
            github_issue_url: "https://github.com/owner/repo/issues/10".to_string(),
            title: "Login feature".to_string(),
            status: IssueStatus::InProgress,
            coordinator: Some(CoordinatorState {
                pane_id: "coord-1".to_string(),
                pid: Some(12345),
                status: CoordinatorStatus::Running,
                started_at: Utc.with_ymd_and_hms(2026, 2, 19, 10, 0, 0).unwrap(),
                github_issue_number: 10,
                crash_count: 0,
            }),
            tasks: Vec::new(),
        };

        let json = serde_json::to_string_pretty(&issue).unwrap();
        let deserialized: ProjectIssue = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "issue-10");
        assert_eq!(deserialized.github_issue_number, 10);
        assert_eq!(
            deserialized.github_issue_url,
            "https://github.com/owner/repo/issues/10"
        );
        assert_eq!(deserialized.title, "Login feature");
        assert_eq!(deserialized.status, IssueStatus::InProgress);
        assert!(deserialized.coordinator.is_some());
        assert!(deserialized.tasks.is_empty());
    }

    #[test]
    fn test_project_issue_without_coordinator() {
        let issue = ProjectIssue {
            id: "issue-20".to_string(),
            github_issue_number: 20,
            github_issue_url: "https://github.com/owner/repo/issues/20".to_string(),
            title: "Pending issue".to_string(),
            status: IssueStatus::Pending,
            coordinator: None,
            tasks: Vec::new(),
        };

        let json = serde_json::to_string(&issue).unwrap();
        let deserialized: ProjectIssue = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "issue-20");
        assert_eq!(deserialized.status, IssueStatus::Pending);
        assert!(deserialized.coordinator.is_none());
        assert!(deserialized.tasks.is_empty());
    }
}
