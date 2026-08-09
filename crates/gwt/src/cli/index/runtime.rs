//! Project index runtime helpers (SPEC-1942 SC-027 split).
//!
//! Hosts the [`IndexContext`] and every helper that drives the embedded
//! Python runner: discovering the venv path, invoking the runner with
//! `--action status` / `--action index-*`, parsing its JSON payload, and
//! rendering the status output. Error helpers ([`io_error`]) live alongside
//! since they only matter for runner-bound errors.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use gwt_core::{repo_hash::RepoHash, worktree_hash::compute_worktree_hash};
use gwt_github::{client::ApiError, SpecOpsError};
use serde_json::Value;

use super::IndexScope;

#[derive(Debug, Clone)]
pub(crate) struct IndexContext {
    pub project_root: PathBuf,
    pub repo_hash: RepoHash,
    pub worktree_hash: String,
    pub python: PathBuf,
    pub runner: PathBuf,
}

pub(crate) fn resolve_index_context(repo_path: &Path) -> Result<IndexContext, SpecOpsError> {
    let project_root = crate::index_worker::default_project_index_worktree_root(repo_path)
        .unwrap_or_else(|| {
            repo_path
                .canonicalize()
                .unwrap_or_else(|_| repo_path.to_path_buf())
        });
    let repo_hash = crate::index_worker::detect_repo_hash(repo_path)
        .or_else(|| crate::index_worker::detect_repo_hash(&project_root))
        .ok_or_else(|| {
            SpecOpsError::from(ApiError::Unexpected(
                "could not resolve project index repo hash from git origin".to_string(),
            ))
        })?;
    let worktree_hash = compute_worktree_hash(&project_root)
        .map_err(|err| SpecOpsError::from(ApiError::Unexpected(err.to_string())))?
        .to_string();
    Ok(IndexContext {
        project_root,
        repo_hash,
        worktree_hash,
        python: project_index_python_path(),
        runner: gwt_core::runtime::project_index_runner_path(),
    })
}

fn project_index_python_path() -> PathBuf {
    gwt_core::runtime::project_index_python_path()
}

pub(crate) fn run_runner_status(
    context: &IndexContext,
) -> Result<std::process::Output, SpecOpsError> {
    gwt_core::process::hidden_command(&context.python)
        .arg(&context.runner)
        .arg("--action")
        .arg("status")
        .arg("--repo-hash")
        .arg(context.repo_hash.as_str())
        .arg("--worktree-hash")
        .arg(&context.worktree_hash)
        .current_dir(&context.project_root)
        .output()
        .map_err(io_error)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RebuildAction {
    pub label: &'static str,
    pub action: &'static str,
    pub scope: Option<&'static str>,
    pub needs_worktree_hash: bool,
}

pub(crate) fn rebuild_actions(scope: IndexScope) -> Vec<RebuildAction> {
    let all = vec![
        RebuildAction {
            label: "issues",
            action: "index-issues",
            scope: None,
            needs_worktree_hash: false,
        },
        RebuildAction {
            label: "specs",
            action: "index-specs",
            scope: None,
            needs_worktree_hash: false,
        },
        RebuildAction {
            label: "memory",
            action: "index-memory",
            scope: None,
            needs_worktree_hash: false,
        },
        RebuildAction {
            label: "discussions",
            action: "index-discussions",
            scope: None,
            needs_worktree_hash: false,
        },
        RebuildAction {
            label: "board",
            action: "index-board",
            scope: None,
            needs_worktree_hash: false,
        },
        RebuildAction {
            label: "files",
            action: "index-files",
            scope: Some("files"),
            needs_worktree_hash: true,
        },
        RebuildAction {
            label: "files-docs",
            action: "index-files",
            scope: Some("files-docs"),
            needs_worktree_hash: true,
        },
    ];
    match scope {
        IndexScope::All => all,
        IndexScope::Issues => all.into_iter().filter(|a| a.label == "issues").collect(),
        IndexScope::Specs => all.into_iter().filter(|a| a.label == "specs").collect(),
        IndexScope::Memory => all.into_iter().filter(|a| a.label == "memory").collect(),
        IndexScope::Discussions => all
            .into_iter()
            .filter(|a| a.label == "discussions")
            .collect(),
        IndexScope::Board => all.into_iter().filter(|a| a.label == "board").collect(),
        IndexScope::Files => all.into_iter().filter(|a| a.label == "files").collect(),
        IndexScope::FilesDocs => all
            .into_iter()
            .filter(|a| a.label == "files-docs")
            .collect(),
    }
}

pub(crate) fn run_runner_rebuild(
    context: &IndexContext,
    action: RebuildAction,
    qos: &str,
) -> Result<std::process::Output, SpecOpsError> {
    #[cfg(test)]
    LEGACY_REBUILD_RUNNER_CALLS.fetch_add(1, Ordering::Relaxed);
    let args = rebuild_runner_args(context, action, qos);
    let mut command = gwt_core::process::hidden_command(&context.python);
    command.args(args).current_dir(&context.project_root);
    command.output().map_err(io_error)
}

pub(crate) fn rebuild_runner_args(
    context: &IndexContext,
    action: RebuildAction,
    qos: &str,
) -> Vec<OsString> {
    #[cfg(test)]
    if std::env::var_os("GWT_INDEX_TEST_REBUILD_FAILURE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        return vec![
            OsString::from("--ignored"),
            OsString::from("--exact"),
            OsString::from("index_worker::tests::rebuild_runner_failure_fixture"),
            OsString::from("--nocapture"),
        ];
    }
    #[cfg(test)]
    if std::env::var_os("GWT_INDEX_TEST_REBUILD_HANG").as_deref() == Some(std::ffi::OsStr::new("1"))
    {
        return vec![
            OsString::from("--ignored"),
            OsString::from("--exact"),
            OsString::from("index_worker::tests::rebuild_runner_hang_fixture"),
            OsString::from("--nocapture"),
        ];
    }

    let mut args = vec![
        context.runner.clone().into_os_string(),
        OsString::from("--action"),
        OsString::from(action.action),
        OsString::from("--repo-hash"),
        OsString::from(context.repo_hash.as_str()),
        OsString::from("--project-root"),
        context.project_root.clone().into_os_string(),
        OsString::from("--mode"),
        OsString::from("full"),
        OsString::from("--qos"),
        OsString::from(qos),
    ];
    if action.needs_worktree_hash {
        args.extend([
            OsString::from("--worktree-hash"),
            OsString::from(&context.worktree_hash),
        ]);
    }
    if let Some(scope) = action.scope {
        args.extend([OsString::from("--scope"), OsString::from(scope)]);
    }
    args
}

#[cfg(test)]
static LEGACY_REBUILD_RUNNER_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_legacy_rebuild_runner_calls() {
    LEGACY_REBUILD_RUNNER_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn legacy_rebuild_runner_calls() -> usize {
    LEGACY_REBUILD_RUNNER_CALLS.load(Ordering::Relaxed)
}

pub(crate) fn parse_runner_json(stdout: &[u8]) -> Result<Value, SpecOpsError> {
    serde_json::from_slice(stdout).map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(format!(
            "project index status returned invalid JSON: {err}"
        )))
    })
}

pub fn render_index_status(
    out: &mut String,
    report: &gwt_core::runtime::ProjectIndexRuntimeReport,
    payload: &Value,
) {
    let runtime = payload.get("runtime").unwrap_or(&Value::Null);
    let reason = runtime
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("ready");
    let asset_hash = runtime
        .get("asset_hash")
        .and_then(Value::as_str)
        .unwrap_or(report.runner_hash.as_str());
    out.push_str(&format!(
        "runtime: {reason} asset={asset_hash} smoke={}\n",
        runtime.get("smoke_test").and_then(Value::as_str).unwrap_or(
            if report.runner_smoke_tested {
                "passed"
            } else {
                "skipped"
            }
        )
    ));
    out.push_str(&format!(
        "runtime_repaired: runner={} requirements={} manifest={} venv_created={} venv_rebuilt={} dependencies={}\n",
        report.runner_updated,
        report.requirements_updated,
        report.manifest_updated,
        report.venv_created,
        report.venv_rebuilt,
        report.dependencies_installed
    ));
    if let Some(status) = payload.get("status").and_then(Value::as_object) {
        for scope in [
            "issues",
            "specs",
            "memory",
            "discussions",
            "board",
            "files",
            "files-docs",
        ] {
            if let Some(scope_status) = status.get(scope) {
                let healthy = scope_status
                    .get("healthy")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let repair = scope_status
                    .get("repair_required")
                    .and_then(Value::as_bool)
                    .unwrap_or(!healthy);
                let reason = scope_status
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let count = scope_status
                    .get("document_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                out.push_str(&format!(
                    "{scope}: {} reason={reason} documents={count} repair_required={repair}\n",
                    if healthy { "ready" } else { "unhealthy" }
                ));
            }
        }
    }
}

pub(crate) fn format_runner_failure(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = match (stderr.is_empty(), stdout.is_empty()) {
        (true, true) => String::new(),
        (true, false) => stdout,
        (false, true) => stderr,
        (false, false) => format!("{stderr}; stdout={stdout}"),
    };
    format!("runner exit={} detail={detail}\n", output.status)
}

fn io_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_runner_args_are_shared_for_worktree_scoped_actions() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = IndexContext {
            project_root: PathBuf::from("project-root"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://example.com/shared-args.git",
            ),
            worktree_hash: "worktree-hash".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let action = RebuildAction {
            label: "files-docs",
            action: "index-files",
            scope: Some("files-docs"),
            needs_worktree_hash: true,
        };

        let args = rebuild_runner_args(&context, action, "background");

        assert_eq!(
            args,
            [
                "runner.py",
                "--action",
                "index-files",
                "--repo-hash",
                context.repo_hash.as_str(),
                "--project-root",
                "project-root",
                "--mode",
                "full",
                "--qos",
                "background",
                "--worktree-hash",
                "worktree-hash",
                "--scope",
                "files-docs",
            ]
            .map(OsString::from)
        );
    }
}
