//! Index audit logging (SPEC-1942 SC-027 split).
//!
//! Append-only JSONL audit events for `gwtd index status` / `gwtd index
//! rebuild`: status snapshots, rebuild start / progress / result, and runner
//! failure detail. Each event is written under
//! `gwt_project_logs_dir_for_project_path/<date>.log`.

use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};

use super::runtime::{format_runner_failure, IndexContext};

pub(super) fn audit_log_dir(context: &IndexContext) -> PathBuf {
    gwt_core::paths::gwt_project_logs_dir_for_project_path(&context.project_root)
}

pub(super) fn audit_status(
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

pub(super) fn audit_status_failure(
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

pub(super) fn audit_rebuild_start(
    log_dir: &Path,
    context: &IndexContext,
    scope: &str,
) -> io::Result<()> {
    let mut fields = base_audit_fields(context, "project_index_rebuild", "index rebuild");
    fields.insert("stage".to_string(), json!("start"));
    fields.insert("scope".to_string(), json!(scope));
    append_index_audit_event(log_dir, Value::Object(fields))
}

pub(super) fn audit_rebuild_result(
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

pub(super) fn audit_runner_progress(
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
