//! `errors.list` JSON operation (Issue #3778).

use std::{collections::HashMap, path::Path};

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

/// Explicit ledger selection; the default query selects the calling project.
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
            let caller_project =
                (scope == ErrorListScope::Project && project_root.is_none()).then(|| {
                    let root = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
                    gwt_core::paths::project_scope_hash(&root)
                });
            let mut matching_roots = HashMap::new();
            let errors: Vec<_> = errors
                .into_iter()
                .filter(|record| scope.includes(record, project_root.as_deref()))
                .filter(|record| {
                    caller_project.as_ref().is_none_or(|caller_project| {
                        record.target.project_root.as_deref().is_some_and(|root| {
                            *matching_roots.entry(root.to_owned()).or_insert_with(|| {
                                let root =
                                    gwt_core::paths::resolve_current_worktree_root(Path::new(root));
                                gwt_core::paths::project_scope_hash(&root) == *caller_project
                            })
                        })
                    })
                })
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
    use gwt_core::process::{resolved_command, ProcessPlanRequest};
    use gwt_core::test_support::ScopedGwtHome;

    #[test]
    fn errors_list_defaults_to_repository_from_subdirectory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        let repo = dir.path().join("repo");
        let cwd = repo.join("crates/gwt");
        std::fs::create_dir_all(&cwd).expect("subdirectory");
        assert!(resolved_command(ProcessPlanRequest::new("git"))
            .expect("resolve git")
            .current_dir(&repo)
            .args(["init", "-q"])
            .status()
            .expect("git init")
            .success());
        let record = ErrorRecord::new(
            ErrorKind::HookFailure,
            "project fault",
            ErrorTarget {
                project_root: Some(repo.display().to_string()),
                ..Default::default()
            },
        );
        gwt_core::error_ledger::record(record.clone()).expect("record");
        let nested_record = ErrorRecord::new(
            ErrorKind::HookFailure,
            "nested project fault",
            ErrorTarget {
                project_root: Some(cwd.display().to_string()),
                ..Default::default()
            },
        );
        gwt_core::error_ledger::record(nested_record.clone()).expect("nested record");
        for caller in [&repo, &cwd] {
            let mut env = TestEnv::new(caller.clone());
            let mut out = String::new();
            run(
                &mut env,
                ErrorsCommand::List {
                    since: None,
                    scope: ErrorListScope::Project,
                    project_root: None,
                },
                &mut out,
            )
            .expect("run");
            let payload: serde_json::Value = serde_json::from_str(out.trim()).expect("json");
            assert_eq!(payload["count"], 2, "{}", caller.display());
            let errors = payload["errors"].as_array().expect("errors");
            assert!(errors.iter().any(|row| row["id"] == record.id));
            assert!(errors.iter().any(|row| row["id"] == nested_record.id));
        }
    }

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
        let ledger_path = gwt_core::paths::gwt_error_ledger_dir()
            .join(format!("errors.{}.jsonl", newer.recorded_at.date_naive()));
        let mut ledger = std::fs::OpenOptions::new()
            .append(true)
            .open(ledger_path)
            .expect("ledger");
        std::io::Write::write_all(&mut ledger, b"{malformed\n").expect("malformed row");

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
        assert_eq!(payload["count"], 2);
        let errors = payload["errors"].as_array().expect("errors");
        assert_eq!(errors.len(), 2);
        let valid = errors
            .iter()
            .find(|row| row["id"] == newer.id)
            .expect("valid newer row");
        assert_eq!(valid["kind"], "operation_refusal");
        assert_eq!(valid["message"], "board.post refused");
        assert_eq!(valid["target"]["issue"], 3778);
        assert!(errors.iter().any(|row| row["kind"] == "ledger_corruption"));
    }
}
