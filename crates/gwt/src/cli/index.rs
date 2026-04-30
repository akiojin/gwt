use std::{
    fs::OpenOptions,
    io,
    io::Write,
    path::{Path, PathBuf},
};

use gwt_core::{repo_hash::RepoHash, worktree_hash::compute_worktree_hash};
use gwt_github::{client::ApiError, SpecOpsError};
use serde_json::{json, Map, Value};

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
        let log_dir = audit_log_dir(&context);
        let _ = audit_status_failure(
            &log_dir,
            &context,
            &format_runner_failure(&output),
            output.status.code().unwrap_or(1),
        );
        return Ok(1);
    }

    let payload = parse_runner_json(&output.stdout)?;
    render_index_status(out, &report, &payload);
    let log_dir = audit_log_dir(&context);
    let _ = audit_status(&log_dir, &context, &report, &payload, 0);
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
    let log_dir = audit_log_dir(&context);
    for action in rebuild_actions(scope) {
        let _ = audit_rebuild_start(&log_dir, &context, action.label);
        let output = run_runner_rebuild(&context, action)?;
        let _ = audit_runner_progress(&log_dir, &context, action.label, &output.stderr);
        let _ = audit_rebuild_result(&log_dir, &context, action.label, &output);
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
    let mut command = gwt_core::process::hidden_command(&context.python);
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

fn audit_log_dir(context: &IndexContext) -> PathBuf {
    gwt_core::paths::gwt_project_logs_dir_for_project_path(&context.project_root)
}

fn audit_status(
    log_dir: &Path,
    context: &IndexContext,
    report: &gwt_core::runtime::ProjectIndexRuntimeReport,
    payload: &Value,
    exit_code: i32,
) -> io::Result<()> {
    let mut fields = base_audit_fields(context, "project_index_status", "index status");
    let health = audit_health(payload);
    fields.insert("status".to_string(), json!(health.status));
    fields.insert("repair_required".to_string(), json!(health.repair_required));
    if !health.unhealthy_scopes.is_empty() {
        fields.insert(
            "unhealthy_scopes".to_string(),
            json!(health.unhealthy_scopes),
        );
    }
    fields.insert("exit_code".to_string(), json!(exit_code));
    fields.insert(
        "runtime_repaired".to_string(),
        json!({
            "runner": report.runner_updated,
            "requirements": report.requirements_updated,
            "manifest": report.manifest_updated,
            "venv_created": report.venv_created,
            "venv_rebuilt": report.venv_rebuilt,
            "dependencies": report.dependencies_installed,
        }),
    );
    fields.insert(
        "runner_hash".to_string(),
        json!(report.runner_hash.as_str()),
    );
    if let Some(runtime) = payload.get("runtime") {
        fields.insert("runtime".to_string(), runtime.clone());
    }
    if let Some(status) = payload.get("status") {
        fields.insert("scopes".to_string(), status.clone());
    }
    append_index_audit_event(log_dir, Value::Object(fields))
}

struct AuditHealth {
    status: &'static str,
    repair_required: bool,
    unhealthy_scopes: Vec<String>,
}

fn audit_health(payload: &Value) -> AuditHealth {
    let Some(status) = payload.get("status").and_then(Value::as_object) else {
        return AuditHealth {
            status: "unknown",
            repair_required: false,
            unhealthy_scopes: Vec::new(),
        };
    };

    let mut unhealthy_scopes = Vec::new();
    for scope in ["issues", "specs", "files", "files-docs"] {
        let Some(scope_status) = status.get(scope) else {
            continue;
        };
        let healthy = scope_status
            .get("healthy")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let repair_required = scope_status
            .get("repair_required")
            .and_then(Value::as_bool)
            .unwrap_or(!healthy);
        if !healthy || repair_required {
            unhealthy_scopes.push(scope.to_string());
        }
    }

    AuditHealth {
        status: if unhealthy_scopes.is_empty() {
            "ready"
        } else {
            "repair_required"
        },
        repair_required: !unhealthy_scopes.is_empty(),
        unhealthy_scopes,
    }
}

fn audit_status_failure(
    log_dir: &Path,
    context: &IndexContext,
    detail: &str,
    exit_code: i32,
) -> io::Result<()> {
    let mut fields = base_audit_fields(context, "project_index_status", "index status");
    fields.insert("status".to_string(), json!("runner_error"));
    fields.insert("exit_code".to_string(), json!(exit_code));
    fields.insert("detail".to_string(), json!(detail.trim()));
    append_index_audit_event(log_dir, Value::Object(fields))
}

fn audit_rebuild_start(log_dir: &Path, context: &IndexContext, scope: &str) -> io::Result<()> {
    let mut fields = base_audit_fields(context, "project_index_rebuild", "index rebuild");
    fields.insert("stage".to_string(), json!("start"));
    fields.insert("scope".to_string(), json!(scope));
    append_index_audit_event(log_dir, Value::Object(fields))
}

fn audit_rebuild_result(
    log_dir: &Path,
    context: &IndexContext,
    scope: &str,
    output: &std::process::Output,
) -> io::Result<()> {
    let mut fields = base_audit_fields(context, "project_index_rebuild", "index rebuild");
    fields.insert("stage".to_string(), json!("result"));
    fields.insert("scope".to_string(), json!(scope));
    fields.insert("success".to_string(), json!(output.status.success()));
    fields.insert(
        "exit_code".to_string(),
        json!(output.status.code().unwrap_or(1)),
    );
    if output.status.success() {
        if let Ok(payload) = serde_json::from_slice::<Value>(&output.stdout) {
            fields.insert("result".to_string(), payload);
        }
    } else {
        fields.insert(
            "detail".to_string(),
            json!(format_runner_failure(output).trim()),
        );
    }
    append_index_audit_event(log_dir, Value::Object(fields))
}

fn audit_runner_progress(
    log_dir: &Path,
    context: &IndexContext,
    action_scope: &str,
    stderr: &[u8],
) -> io::Result<()> {
    let stderr = String::from_utf8_lossy(stderr);
    for line in stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(progress) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let mut fields =
            base_audit_fields(context, "project_index_runner_progress", "index rebuild");
        fields.insert("action_scope".to_string(), json!(action_scope));
        if let Some(object) = progress.as_object() {
            for key in ["phase", "scope", "mode", "done", "total", "indexed"] {
                if let Some(value) = object.get(key) {
                    fields.insert(key.to_string(), value.clone());
                }
            }
        }
        fields.insert("progress".to_string(), progress);
        append_index_audit_event(log_dir, Value::Object(fields))?;
    }
    Ok(())
}

fn base_audit_fields(context: &IndexContext, message: &str, command: &str) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("message".to_string(), json!(message));
    fields.insert("event".to_string(), json!(message));
    fields.insert("command".to_string(), json!(command));
    fields.insert("repo_hash".to_string(), json!(context.repo_hash.as_str()));
    fields.insert("worktree_hash".to_string(), json!(context.worktree_hash));
    fields.insert(
        "project_root".to_string(),
        json!(context.project_root.to_string_lossy().to_string()),
    );
    fields
}

fn append_index_audit_event(log_dir: &Path, fields: Value) -> io::Result<()> {
    std::fs::create_dir_all(log_dir)?;
    let path = gwt_core::logging::current_log_file(log_dir);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let event = json!({
        "timestamp": chrono::Local::now().to_rfc3339(),
        "level": "INFO",
        "target": "gwt::index",
        "fields": fields,
    });
    serde_json::to_writer(&mut file, &event)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    file.write_all(b"\n")?;
    file.flush()
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
    fn audit_log_dir_uses_project_scoped_gwt_log_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("repo");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_hash = gwt_core::repo_hash::compute_path_hash(&project_root);
        let context = IndexContext {
            project_root,
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };

        let log_dir = audit_log_dir(&context);

        assert!(log_dir.ends_with(
            PathBuf::from("projects")
                .join(project_hash.as_str())
                .join("logs")
        ));
        assert!(!log_dir.ends_with(PathBuf::from(".gwt").join("logs")));
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

    #[test]
    fn audit_status_writes_to_unified_gwt_log_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let context = IndexContext {
            project_root: dir.path().join("repo"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            runner_updated: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "runtime": {
                "reason": "ready",
                "asset_hash": "aaaaaaaaaaaaaaaa",
                "smoke_test": "passed"
            },
            "status": {
                "files-docs": {
                    "healthy": true,
                    "repair_required": false,
                    "reason": "ready",
                    "document_count": 16,
                    "last_repair_at": "2026-04-24T06:15:20Z"
                }
            }
        });

        audit_status(dir.path(), &context, &report, &payload, 0).unwrap();

        let log_path = gwt_core::logging::current_log_file(dir.path());
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(content.contains("\"target\":\"gwt::index\""), "{content}");
        assert!(
            content.contains("\"message\":\"project_index_status\""),
            "{content}"
        );
        assert!(content.contains("\"status\":\"ready\""), "{content}");
        assert!(content.contains("\"repair_required\":false"), "{content}");
        assert!(content.contains("\"runner\":true"), "{content}");
        assert!(content.contains("\"files-docs\""), "{content}");
        assert!(
            content.contains("\"last_repair_at\":\"2026-04-24T06:15:20Z\""),
            "{content}"
        );
        assert!(!dir.path().join("index.log").exists());
    }

    #[test]
    fn audit_status_marks_unhealthy_scope_as_repair_required() {
        let dir = tempfile::tempdir().unwrap();
        let context = IndexContext {
            project_root: dir.path().join("repo"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "status": {
                "specs": {
                    "healthy": false,
                    "repair_required": true,
                    "reason": "count_mismatch",
                    "document_count": 1
                },
                "files": {
                    "healthy": true,
                    "repair_required": false,
                    "reason": "ready",
                    "document_count": 310
                }
            }
        });

        audit_status(dir.path(), &context, &report, &payload, 0).unwrap();

        let log_path = gwt_core::logging::current_log_file(dir.path());
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(
            content.contains("\"status\":\"repair_required\""),
            "{content}"
        );
        assert!(content.contains("\"repair_required\":true"), "{content}");
        assert!(
            content.contains("\"unhealthy_scopes\":[\"specs\"]"),
            "{content}"
        );
        assert!(
            content.contains("\"reason\":\"count_mismatch\""),
            "{content}"
        );
        assert!(!dir.path().join("index.log").exists());
    }

    #[test]
    fn audit_runner_progress_translates_stderr_ndjson_to_unified_log() {
        let dir = tempfile::tempdir().unwrap();
        let context = IndexContext {
            project_root: dir.path().join("repo"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let stderr = br#"{"phase":"indexing","scope":"files-docs","done":0,"total":16}
not json
{"phase":"complete","scope":"files-docs","indexed":16,"total":16}
"#;

        audit_runner_progress(dir.path(), &context, "files-docs", stderr).unwrap();

        let log_path = gwt_core::logging::current_log_file(dir.path());
        let content = std::fs::read_to_string(log_path).unwrap();
        assert_eq!(content.lines().count(), 2);
        assert!(content.contains("\"phase\":\"indexing\""), "{content}");
        assert!(content.contains("\"phase\":\"complete\""), "{content}");
        assert!(content.contains("\"scope\":\"files-docs\""), "{content}");
        assert!(!content.contains("not json"), "{content}");
        assert!(!dir.path().join("index.log").exists());
    }
}
