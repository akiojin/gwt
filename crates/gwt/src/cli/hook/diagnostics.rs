//! Opt-in hook timing diagnostics.
//!
//! Hook stdout is part of the Claude/Codex protocol, so diagnostics must never
//! write there. When `GWT_HOOK_PROFILE_PATH` is set, handlers append compact
//! JSONL timing records to that path and otherwise stay silent.

use std::{fs::OpenOptions, io::Write, path::PathBuf, time::Duration};

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
