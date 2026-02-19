//! Coordinator state for agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Coordinator process status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorStatus {
    Starting,
    Running,
    Completed,
    Crashed,
    Restarting,
}

/// State of a coordinator managing a single GitHub Issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorState {
    pub pane_id: String,
    pub pid: Option<u32>,
    pub status: CoordinatorStatus,
    pub started_at: DateTime<Utc>,
    pub github_issue_number: u64,
    pub crash_count: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_coordinator_status_serialize_starting() {
        let json = serde_json::to_string(&CoordinatorStatus::Starting).unwrap();
        assert_eq!(json, r#""starting""#);
        let deserialized: CoordinatorStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CoordinatorStatus::Starting);
    }

    #[test]
    fn test_coordinator_status_serialize_running() {
        let json = serde_json::to_string(&CoordinatorStatus::Running).unwrap();
        assert_eq!(json, r#""running""#);
        let deserialized: CoordinatorStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CoordinatorStatus::Running);
    }

    #[test]
    fn test_coordinator_status_serialize_completed() {
        let json = serde_json::to_string(&CoordinatorStatus::Completed).unwrap();
        assert_eq!(json, r#""completed""#);
        let deserialized: CoordinatorStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CoordinatorStatus::Completed);
    }

    #[test]
    fn test_coordinator_status_serialize_crashed() {
        let json = serde_json::to_string(&CoordinatorStatus::Crashed).unwrap();
        assert_eq!(json, r#""crashed""#);
        let deserialized: CoordinatorStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CoordinatorStatus::Crashed);
    }

    #[test]
    fn test_coordinator_status_serialize_restarting() {
        let json = serde_json::to_string(&CoordinatorStatus::Restarting).unwrap();
        assert_eq!(json, r#""restarting""#);
        let deserialized: CoordinatorStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CoordinatorStatus::Restarting);
    }

    #[test]
    fn test_coordinator_state_roundtrip() {
        let state = CoordinatorState {
            pane_id: "coord-1".to_string(),
            pid: Some(12345),
            status: CoordinatorStatus::Running,
            started_at: Utc.with_ymd_and_hms(2026, 2, 19, 10, 0, 0).unwrap(),
            github_issue_number: 10,
            crash_count: 0,
        };

        let json = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: CoordinatorState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.pane_id, "coord-1");
        assert_eq!(deserialized.pid, Some(12345));
        assert_eq!(deserialized.status, CoordinatorStatus::Running);
        assert_eq!(deserialized.github_issue_number, 10);
        assert_eq!(deserialized.crash_count, 0);
    }

    #[test]
    fn test_coordinator_state_with_none_pid() {
        let state = CoordinatorState {
            pane_id: "coord-2".to_string(),
            pid: None,
            status: CoordinatorStatus::Starting,
            started_at: Utc.with_ymd_and_hms(2026, 2, 19, 11, 0, 0).unwrap(),
            github_issue_number: 42,
            crash_count: 3,
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: CoordinatorState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.pid, None);
        assert_eq!(deserialized.crash_count, 3);
    }
}
