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
            let repo_path = gwt_core::paths::resolve_main_worktree_root(&repo_path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn pm_status_reads_the_main_worktree_registry_from_a_linked_worktree() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&repo).expect("repo dir");
        run_git(&repo, &["init"]);
        run_git(&repo, &["config", "user.email", "gwt@example.invalid"]);
        run_git(&repo, &["config", "user.name", "gwt test"]);
        std::fs::write(repo.join("seed"), "seed\n").expect("seed file");
        run_git(&repo, &["add", "seed"]);
        run_git(&repo, &["commit", "-m", "seed"]);
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "--detach",
                worktree.to_str().expect("utf8 worktree"),
                "HEAD",
            ],
        );
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&repo);
        pm_registry::save_pm_prefs(
            &prefs_path,
            &pm_registry::PmPrefs {
                registration: Some(pm_registry::PmRegistration {
                    session_id: "pm-session".to_string(),
                    agent_id: "codex".to_string(),
                    worktree_path: worktree.to_string_lossy().into_owned(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..pm_registry::PmPrefs::default()
            },
        )
        .expect("seed main-worktree PM prefs");
        let mut env = crate::cli::TestEnv::new(worktree);
        let mut output = String::new();

        let status = run(
            &mut env,
            PmCommand::Status { project_root: None },
            &mut output,
        )
        .expect("pm.status from linked worktree");
        let report: serde_json::Value = serde_json::from_str(&output).expect("status JSON");

        assert_eq!(status, 0);
        assert_eq!(report["registered"], true);
        assert_eq!(report["registration"]["session_id"], "pm-session");
    }
}
