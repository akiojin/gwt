use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

use gwt_core::{repo_hash::RepoHash, worktree_hash::compute_worktree_hash};
use gwt_github::{client::ApiError, SpecOpsError};
use serde_json::Value;

use super::{CliCommand, CliEnv, CliParseError, IndexScope};

pub fn parse(args: &[String]) -> Result<CliCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "status" => {
            if !rest.is_empty() {
                return Err(CliParseError::Usage);
            }
            Ok(CliCommand::IndexStatus)
        }
        "rebuild" => Ok(CliCommand::IndexRebuild {
            scope: parse_rebuild_scope(rest)?,
        }),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_rebuild_scope(args: &[String]) -> Result<IndexScope, CliParseError> {
    if args.is_empty() {
        return Ok(IndexScope::All);
    }
    if args.len() != 2 || args[0] != "--scope" {
        return Err(CliParseError::Usage);
    }
    match args[1].as_str() {
        "all" => Ok(IndexScope::All),
        "issues" => Ok(IndexScope::Issues),
        "specs" => Ok(IndexScope::Specs),
        "files" => Ok(IndexScope::Files),
        "files-docs" => Ok(IndexScope::FilesDocs),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

pub fn run<E: CliEnv>(env: &mut E, cmd: CliCommand, out: &mut String) -> Result<i32, SpecOpsError> {
    match cmd {
        CliCommand::IndexStatus => run_status(env, out),
        CliCommand::IndexRebuild { scope } => run_rebuild(env, scope, out),
        _ => unreachable!("index::run only accepts index commands"),
    }
}

fn run_status<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let context = resolve_index_context(env.repo_path())?;
    let report = gwt_core::runtime::ensure_project_index_runtime().map_err(runtime_error)?;
    let output = run_runner_status(&context)?;
    if !output.status.success() {
        out.push_str("runtime: error\n");
        out.push_str(&format_runner_failure(&output));
        return Ok(1);
    }

    let payload = parse_runner_json(&output.stdout)?;
    render_index_status(out, &report, &payload);
    Ok(0)
}

fn run_rebuild<E: CliEnv>(
    env: &mut E,
    scope: IndexScope,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let context = resolve_index_context(env.repo_path())?;
    let report = gwt_core::runtime::ensure_project_index_runtime().map_err(runtime_error)?;
    out.push_str(&format!(
        "runtime: ready asset={} smoke={}\n",
        report.runner_hash,
        if report.runner_smoke_tested {
            "passed"
        } else {
            "skipped"
        }
    ));

    let mut ok = true;
    for action in rebuild_actions(scope) {
        let output = run_runner_rebuild(&context, action)?;
        if output.status.success() {
            out.push_str(&format!("{}: ok\n", action.label));
        } else {
            ok = false;
            out.push_str(&format!("{}: error\n", action.label));
            out.push_str(&format_runner_failure(&output));
        }
    }

    Ok(if ok { 0 } else { 1 })
}

#[derive(Debug, Clone)]
struct IndexContext {
    project_root: PathBuf,
    repo_hash: RepoHash,
    worktree_hash: String,
    python: PathBuf,
    runner: PathBuf,
}

fn resolve_index_context(repo_path: &Path) -> Result<IndexContext, SpecOpsError> {
    let project_root = repo_path
        .canonicalize()
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let repo_hash = crate::index_worker::detect_repo_hash(&project_root).ok_or_else(|| {
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
        runner: gwt_core::paths::gwt_runtime_runner_path(),
    })
}

fn project_index_python_path() -> PathBuf {
    let venv = gwt_core::paths::gwt_project_index_venv_dir();
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python3")
    }
}

fn run_runner_status(context: &IndexContext) -> Result<std::process::Output, SpecOpsError> {
    Command::new(&context.python)
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
struct RebuildAction {
    label: &'static str,
    action: &'static str,
    scope: Option<&'static str>,
    needs_worktree_hash: bool,
}

fn rebuild_actions(scope: IndexScope) -> Vec<RebuildAction> {
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
        IndexScope::Files => all.into_iter().filter(|a| a.label == "files").collect(),
        IndexScope::FilesDocs => all
            .into_iter()
            .filter(|a| a.label == "files-docs")
            .collect(),
    }
}

fn run_runner_rebuild(
    context: &IndexContext,
    action: RebuildAction,
) -> Result<std::process::Output, SpecOpsError> {
    let mut command = Command::new(&context.python);
    command
        .arg(&context.runner)
        .arg("--action")
        .arg(action.action)
        .arg("--repo-hash")
        .arg(context.repo_hash.as_str())
        .arg("--project-root")
        .arg(&context.project_root)
        .arg("--mode")
        .arg("full")
        .current_dir(&context.project_root);
    if action.needs_worktree_hash {
        command.arg("--worktree-hash").arg(&context.worktree_hash);
    }
    if let Some(scope) = action.scope {
        command.arg("--scope").arg(scope);
    }
    command.output().map_err(io_error)
}

fn parse_runner_json(stdout: &[u8]) -> Result<Value, SpecOpsError> {
    serde_json::from_slice(stdout).map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(format!(
            "project index status returned invalid JSON: {err}"
        )))
    })
}

pub(crate) fn render_index_status(
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
        for scope in ["issues", "specs", "files", "files-docs"] {
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

fn format_runner_failure(output: &std::process::Output) -> String {
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

fn io_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

fn runtime_error(err: gwt_core::GwtError) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn parses_index_status_and_rebuild_scope() {
        assert_eq!(parse(&s(&["status"])).unwrap(), CliCommand::IndexStatus);
        assert_eq!(
            parse(&s(&["rebuild", "--scope", "files-docs"])).unwrap(),
            CliCommand::IndexRebuild {
                scope: IndexScope::FilesDocs
            }
        );
        assert_eq!(
            parse(&s(&["rebuild"])).unwrap(),
            CliCommand::IndexRebuild {
                scope: IndexScope::All
            }
        );
    }

    #[test]
    fn renders_runtime_and_scope_health() {
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "runtime": {
                "healthy": true,
                "reason": "ready",
                "asset_hash": "cccccccccccccccc",
                "smoke_test": "passed"
            },
            "status": {
                "files": {
                    "healthy": false,
                    "repair_required": true,
                    "reason": "manifest_missing",
                    "document_count": 3
                }
            }
        });
        let mut out = String::new();
        render_index_status(&mut out, &report, &payload);
        assert!(out.contains("runtime: ready asset=cccccccccccccccc smoke=passed"));
        assert!(out
            .contains("files: unhealthy reason=manifest_missing documents=3 repair_required=true"));
    }
}
