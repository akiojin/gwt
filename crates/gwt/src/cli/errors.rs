//! `errors.list` JSON operation (Issue #3778).

use chrono::{DateTime, Utc};
use gwt_core::error_ledger::{ErrorRecord, ErrorScope};
use gwt_github::{client::ApiError, SpecOpsError};
use serde::Serialize;

use crate::cli::{CliEnv, CliParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorsCommand {
    List {
        since: Option<String>,
        scope: ErrorListScope,
        project_root: Option<String>,
    },
}

/// Explicit ledger selection; the default query excludes shared and unowned faults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorListScope {
    #[default]
    Project,
    Host,
    Unknown,
    All,
}

impl ErrorListScope {
    pub(crate) fn parse(raw: &str) -> Result<Self, CliParseError> {
        match raw {
            "project" => Ok(Self::Project),
            "host" => Ok(Self::Host),
            "unknown" => Ok(Self::Unknown),
            "all" => Ok(Self::All),
            _ => Err(CliParseError::InvalidValue {
                flag: "scope",
                reason: "must be project, host, unknown, or all",
            }),
        }
    }

    fn includes(self, record: &ErrorRecord, project_root: Option<&str>) -> bool {
        if let Some(root) = project_root {
            return record.scope == ErrorScope::Project
                && record.target.project_root.as_deref() == Some(root);
        }
        match self {
            Self::Project => record.scope == ErrorScope::Project,
            Self::Host => record.scope == ErrorScope::Host,
            Self::Unknown => record.scope == ErrorScope::Unknown,
            Self::All => true,
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorsListPayload {
    schema_version: u32,
    since: Option<String>,
    count: usize,
    errors: Vec<gwt_core::error_ledger::ErrorRecord>,
}

pub fn run<E: CliEnv>(
    env: &mut E,
    command: ErrorsCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let _ = env;
    match command {
        ErrorsCommand::List {
            since,
            scope,
            project_root,
        } => {
            let cutoff = since
                .as_deref()
                .map(parse_since)
                .transpose()
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            let errors = gwt_core::error_ledger::list_since(cutoff)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            let errors: Vec<_> = errors
                .into_iter()
                .filter(|record| scope.includes(record, project_root.as_deref()))
                .collect();
            let payload = ErrorsListPayload {
                schema_version: gwt_core::error_ledger::SCHEMA_VERSION,
                since,
                count: errors.len(),
                errors,
            };
            let rendered = serde_json::to_string_pretty(&payload)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            out.push_str(&rendered);
            out.push('\n');
            Ok(0)
        }
    }
}

pub(crate) fn parse_since(raw: &str) -> Result<DateTime<Utc>, CliParseError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| CliParseError::InvalidValue {
            flag: "since",
            reason: "must be RFC3339",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::TestEnv;
    use chrono::TimeZone;
    use gwt_core::error_ledger::{ErrorKind, ErrorRecord, ErrorTarget};
    use gwt_core::test_support::ScopedGwtHome;

    #[test]
    fn errors_list_returns_rows_recorded_since_cutoff() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        let older = {
            let mut record =
                ErrorRecord::new(ErrorKind::HookFailure, "old hook", ErrorTarget::default());
            record.recorded_at = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            record
        };
        let newer = ErrorRecord::new(
            ErrorKind::OperationRefusal,
            "board.post refused",
            ErrorTarget {
                issue: Some(3778),
                ..ErrorTarget::default()
            },
        );
        gwt_core::error_ledger::record(older).expect("older");
        gwt_core::error_ledger::record(newer.clone()).expect("newer");

        let mut env = TestEnv::new(dir.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            ErrorsCommand::List {
                since: Some("2026-08-01T00:00:00Z".into()),
                scope: ErrorListScope::All,
                project_root: None,
            },
            &mut out,
        )
        .expect("run");
        assert_eq!(code, 0);
        let payload: serde_json::Value = serde_json::from_str(out.trim()).expect("json");
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["errors"][0]["id"], newer.id);
        assert_eq!(payload["errors"][0]["kind"], "operation_refusal");
        assert_eq!(payload["errors"][0]["message"], "board.post refused");
        assert_eq!(payload["errors"][0]["target"]["issue"], 3778);
    }
}
