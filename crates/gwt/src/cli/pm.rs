//! `pm.*` JSON operations (SPEC-3431): PM agent diagnostics.
//!
//! `pm.status` is the read-only diagnostic surface for the per-project PM
//! singleton (FR-001): it reports the durable registration, the auto-start
//! opt-out (FR-002), and a stale hint derived from the durable session store.
//! It is registered in the workflow-policy read-only and ownerless-safe
//! allowlists so any session can diagnose PM state before an owner is linked.

use std::path::PathBuf;

use gwt_core::paths::gwt_sessions_dir;
use gwt_github::{ApiError, SpecOpsError};

use crate::cli::env::CliEnv;
use crate::pm_registry;

/// Parsed `pm.*` command surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmCommand {
    /// `pm.status` — optional explicit `project_root`; defaults to the
    /// current repository path (container/bare setups must pass it
    /// explicitly, same convention as the Issue Monitor queue operations).
    Status { project_root: Option<String> },
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: PmCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        PmCommand::Status { project_root } => {
            let repo_path = project_root
                .map(PathBuf::from)
                .unwrap_or_else(|| env.repo_path().to_path_buf());
            let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&repo_path);
            let prefs = pm_registry::load_pm_prefs(&prefs_path).map_err(|error| {
                SpecOpsError::from(ApiError::Unexpected(format!(
                    "failed to load PM prefs from {}: {error}",
                    prefs_path.display()
                )))
            })?;
            // SPEC-3431 FR-009 diagnostic visibility: report whether THIS
            // caller holds PM privilege, so a refused Issue Monitor ON has a
            // one-command explanation.
            let caller_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let report = pm_registry::pm_status_report_for_caller(
                &prefs,
                |session_id| {
                    gwt_sessions_dir()
                        .join(format!("{session_id}.toml"))
                        .exists()
                },
                caller_session.as_deref(),
            );
            let rendered = serde_json::to_string_pretty(&report).map_err(|error| {
                SpecOpsError::from(ApiError::Unexpected(format!(
                    "failed to serialize pm.status report: {error}"
                )))
            })?;
            out.push_str(&rendered);
            out.push('\n');
            Ok(0)
        }
    }
}
