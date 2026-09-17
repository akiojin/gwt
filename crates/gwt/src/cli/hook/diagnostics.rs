//! Opt-in hook timing diagnostics and durable Stop-gate decision records.
//!
//! Hook stdout is part of the Claude/Codex protocol, so diagnostics must never
//! write there. When `GWT_HOOK_PROFILE_PATH` is set, handlers append compact
//! JSONL timing records to that path and otherwise stay silent.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use gwt_agent::{GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV};
use serde_json::json;

const GWT_HOOK_PROFILE_PATH_ENV: &str = "GWT_HOOK_PROFILE_PATH";

pub fn record_handler_duration(event: &str, handler: &str, duration: Duration, status: &str) {
    let Some(path) = std::env::var_os(GWT_HOOK_PROFILE_PATH_ENV) else {
        return;
    };
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let record = json!({
        "event": event,
        "handler": handler,
        "status": status,
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "gwt_session_id": std::env::var(GWT_SESSION_ID_ENV).ok(),
        "runtime_path": std::env::var(GWT_SESSION_RUNTIME_PATH_ENV).ok(),
        "forward_url_set": std::env::var_os(GWT_HOOK_FORWARD_URL_ENV).is_some(),
        "occurred_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    });

    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = serde_json::to_writer(&mut file, &record);
    let _ = file.write_all(b"\n");
}

/// Record a Stop-gate decision in the project's canonical log
/// (`~/.gwt/projects/<hash>/logs/gwt.log.YYYY-MM-DD`).
///
/// Issue #4454 AC-2: when a gate releases Stop because the session holds no
/// authority over the Work, the grounds must outlive the session. A refusal
/// text the agent reads and discards is not evidence, and the exemption is
/// silent by construction — nothing else would record that it happened.
///
/// Hook processes run the CLI before the GUI installs the tracing subscriber,
/// so `tracing::*` reaches nobody here. Append the JSONL record straight to
/// the canonical log instead, exactly as the index audit trail does. Every
/// step is fail-open: logging never decides whether Stop is released.
pub fn record_stop_gate_decision(worktree: &Path, fields: serde_json::Value) {
    let log_dir = gwt_core::paths::gwt_project_logs_dir_for_project_path(worktree);
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let record = json!({
        "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "level": "WARN",
        "target": "gwt::hook::stop_gate",
        "fields": fields,
    });
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(gwt_core::logging::current_log_file(&log_dir))
    else {
        return;
    };
    let _ = serde_json::to_writer(&mut file, &record);
    let _ = file.write_all(b"\n");
}
