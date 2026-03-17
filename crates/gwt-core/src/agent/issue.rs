//! Project issue definitions

use serde::{Deserialize, Serialize};

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
    pub tasks: Vec<Task>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_status_serialize_pending() {
        let json = serde_json::to_string(&IssueStatus::Pending).unwrap();
        assert_eq!(json, r#""pending""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::Pending);
    }

    #[test]
    fn test_issue_status_serialize_completed() {
        let json = serde_json::to_string(&IssueStatus::Completed).unwrap();
        assert_eq!(json, r#""completed""#);
        let deserialized: IssueStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, IssueStatus::Completed);
    }

    #[test]
    fn test_project_issue_roundtrip() {
        let issue = ProjectIssue {
            id: "issue-10".to_string(),
            github_issue_number: 10,
            github_issue_url: "https://github.com/owner/repo/issues/10".to_string(),
            title: "Login feature".to_string(),
            status: IssueStatus::InProgress,
            tasks: Vec::new(),
        };

        let json = serde_json::to_string_pretty(&issue).unwrap();
        let deserialized: ProjectIssue = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "issue-10");
        assert_eq!(deserialized.github_issue_number, 10);
        assert_eq!(deserialized.title, "Login feature");
        assert_eq!(deserialized.status, IssueStatus::InProgress);
        assert!(deserialized.tasks.is_empty());
    }
}
