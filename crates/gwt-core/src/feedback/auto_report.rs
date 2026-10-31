//! Closed public vocabulary for unattended feedback. No caller text belongs here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::FeedbackState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownOperation {
    WorkspaceUpdate,
    VerifyPlan,
    VerifyRun,
    ExecutionStatus,
    ExecutionComplete,
    ExecutionReopen,
    BuildStart,
    BuildComplete,
    HookHealth,
    HookDoctor,
    Search,
    IndexStatus,
    PaneLaunch,
    PaneRead,
    IssueView,
    IssueSpecEdit,
    IssueMonitorStatus,
    IssueMonitorLaunch,
    PrCreate,
    PrReady,
    PrChecks,
    FeedbackCreate,
    Unknown,
}

impl KnownOperation {
    /// Exact allowlist lookup; unrecognized text is discarded, never retained.
    pub fn classify(name: &str) -> Self {
        match name {
            "workspace.update" => Self::WorkspaceUpdate,
            "verify.plan" => Self::VerifyPlan,
            "verify.run" => Self::VerifyRun,
            "execution.status" => Self::ExecutionStatus,
            "execution.complete" => Self::ExecutionComplete,
            "execution.reopen" => Self::ExecutionReopen,
            "build.start" => Self::BuildStart,
            "build.complete" => Self::BuildComplete,
            "hook.health" => Self::HookHealth,
            "hook.doctor" => Self::HookDoctor,
            "search" => Self::Search,
            "index.status" => Self::IndexStatus,
            "pane.launch" => Self::PaneLaunch,
            "pane.read" => Self::PaneRead,
            "issue.view" => Self::IssueView,
            "issue.spec.edit" => Self::IssueSpecEdit,
            "issue.monitor.status" => Self::IssueMonitorStatus,
            "issue.monitor.launch_now" => Self::IssueMonitorLaunch,
            "pr.create" => Self::PrCreate,
            "pr.ready" => Self::PrReady,
            "pr.checks" => Self::PrChecks,
            "feedback.create" => Self::FeedbackCreate,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum NumericVersion {
    Release { major: u16, minor: u16, patch: u16 },
    Unknown,
}

impl NumericVersion {
    fn trusted_package(version: &str) -> Self {
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            return Self::Unknown;
        }
        match (parts[0].parse(), parts[1].parse(), parts[2].parse()) {
            (Ok(major), Ok(minor), Ok(patch)) => Self::Release {
                major,
                minor,
                patch,
            },
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OsFamily {
    Windows,
    Macos,
    Linux,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Architecture {
    X86_64,
    Aarch64,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutoSchema {
    SafeAutoV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticError {
    Unknown,
}

/// Every field has a finite vocabulary or bounded numeric components. The
/// constructor accepts only a known operation, never versions or free text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeAutoReport {
    schema: AutoSchema,
    operation: KnownOperation,
    error: DiagnosticError,
    gwt_version: NumericVersion,
    gwtd_version: NumericVersion,
    os: OsFamily,
    architecture: Architecture,
}

impl SafeAutoReport {
    pub fn current(operation: KnownOperation) -> Self {
        Self {
            schema: AutoSchema::SafeAutoV1,
            operation,
            error: DiagnosticError::Unknown,
            gwt_version: NumericVersion::trusted_package(env!("CARGO_PKG_VERSION")),
            gwtd_version: NumericVersion::trusted_package(env!("CARGO_PKG_VERSION")),
            os: match std::env::consts::OS {
                "windows" => OsFamily::Windows,
                "macos" => OsFamily::Macos,
                "linux" => OsFamily::Linux,
                _ => OsFamily::Unknown,
            },
            architecture: match std::env::consts::ARCH {
                "x86_64" => Architecture::X86_64,
                "aarch64" => Architecture::Aarch64,
                _ => Architecture::Unknown,
            },
        }
    }

    pub fn sendable(&self) -> bool {
        self.operation != KnownOperation::Unknown
    }

    pub fn title(&self) -> String {
        // The serialized enum contains no caller-selected characters.
        format!(
            "gwt automatic feedback: {}",
            serde_json::to_value(self.operation)
                .expect("closed enum")
                .as_str()
                .expect("enum string")
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoReason {
    InsufficientInformation,
    PolicyUnavailable,
    PolicyRevoked,
    WorkerUnavailable,
    AuthUnavailable,
    PreSubmitFailed,
    RemoteOutcomeUnknown,
    SenderInterrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutoMode {
    Auto,
}

/// Immutable snapshot shared by storage, fingerprinting and the typed sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeAutoSnapshot {
    #[serde(with = "request_id")]
    id: uuid::Uuid,
    report: SafeAutoReport,
}

impl SafeAutoSnapshot {
    pub fn id(&self) -> String {
        self.id.to_string()
    }
    pub fn report(&self) -> &SafeAutoReport {
        &self.report
    }
    pub fn marker(&self) -> String {
        format!("<!-- gwt-feedback:{} -->", self.id)
    }
    pub fn render_issue(&self) -> String {
        format!("{}\n<!-- gwt-feedback-kind:friction -->\n\n## Automatic structured report\n\n```json\n{}\n```\n\nFree text is excluded from automatic feedback.\n", self.marker(), serde_json::to_string_pretty(&self.report).expect("closed report"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoFeedbackRecord {
    mode: AutoMode,
    pub snapshot: SafeAutoSnapshot,
    pub created_at: DateTime<Utc>,
    pub state: FeedbackState,
    pub issue_number: Option<u64>,
    pub error: Option<AutoReason>,
}

impl AutoFeedbackRecord {
    pub(super) fn new(report: SafeAutoReport, created_at: DateTime<Utc>) -> Self {
        let sendable = report.sendable();
        Self {
            mode: AutoMode::Auto,
            snapshot: SafeAutoSnapshot {
                id: uuid::Uuid::new_v4(),
                report,
            },
            created_at,
            state: if sendable {
                FeedbackState::Queued
            } else {
                FeedbackState::Withheld
            },
            issue_number: None,
            error: (!sendable).then_some(AutoReason::InsufficientInformation),
        }
    }
}

mod request_id {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(id: &uuid::Uuid, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&id.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<uuid::Uuid, D::Error> {
        uuid::Uuid::parse_str(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_versions_are_bounded_and_never_publish_suffixes() {
        assert_eq!(
            NumericVersion::trusted_package("65535.0.1"),
            NumericVersion::Release {
                major: 65535,
                minor: 0,
                patch: 1
            }
        );
        for version in [
            "65536.0.0",
            "1.2.3-private",
            "1.2.3+private",
            "1.2",
            "1.2.3.4",
            "1..3",
            "private",
        ] {
            assert_eq!(
                NumericVersion::trusted_package(version),
                NumericVersion::Unknown
            );
        }
    }

    #[test]
    fn safe_report_schema_cannot_deserialize_free_text_or_unknown_fields() {
        let report = SafeAutoReport::current(KnownOperation::VerifyRun);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 7);
        for field in [
            "body",
            "title",
            "environment",
            "project_hash",
            "active_agents",
            "source",
            "labels",
        ] {
            let mut injected = value.clone();
            injected[field] = serde_json::json!("unmarked private content");
            assert!(serde_json::from_value::<SafeAutoReport>(injected).is_err());
        }
        let mut injected = value;
        injected["gwt_version"] =
            serde_json::json!({"release":{"major":65536,"minor":0,"patch":0}});
        assert!(serde_json::from_value::<SafeAutoReport>(injected).is_err());
        assert!(!SafeAutoReport::current(KnownOperation::classify("private ui action")).sendable());
    }
}
