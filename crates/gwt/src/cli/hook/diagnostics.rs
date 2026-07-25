//! Opt-in hook timing diagnostics.
//!
//! Hook stdout is part of the Claude/Codex protocol, so diagnostics must never
//! write there. When `GWT_HOOK_PROFILE_PATH` is set, handlers append compact
//! JSONL timing records to that path and otherwise stay silent.

use std::{
    cell::Cell,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use gwt_agent::{GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV};
use serde_json::json;

const GWT_HOOK_PROFILE_PATH_ENV: &str = "GWT_HOOK_PROFILE_PATH";

/// Content-free aggregate metrics for one managed hook event.
///
/// Provider/history and projection counters can join this numeric-only
/// boundary once their read APIs are stable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HookEventMetrics {
    pub additional_context_bytes: usize,
    pub provider_read_count: usize,
    pub history_materialization_count: usize,
    pub projection_load_count: usize,
}

thread_local! {
    static CURRENT_EVENT_METRICS: Cell<HookEventMetrics> =
        const { Cell::new(HookEventMetrics {
            additional_context_bytes: 0,
            provider_read_count: 0,
            history_materialization_count: 0,
            projection_load_count: 0,
        }) };
}

pub fn begin_event() {
    CURRENT_EVENT_METRICS.with(|metrics| metrics.set(HookEventMetrics::default()));
}

pub fn record_prompt_board_read() {
    CURRENT_EVENT_METRICS.with(|metrics| {
        let mut current = metrics.get();
        current.provider_read_count = current.provider_read_count.saturating_add(1);
        current.history_materialization_count =
            current.history_materialization_count.saturating_add(1);
        metrics.set(current);
    });
}

pub fn record_projection_load() {
    CURRENT_EVENT_METRICS.with(|metrics| {
        let mut current = metrics.get();
        current.projection_load_count = current.projection_load_count.saturating_add(1);
        metrics.set(current);
    });
}

pub fn event_metrics(additional_context_bytes: usize) -> HookEventMetrics {
    CURRENT_EVENT_METRICS.with(|metrics| {
        let mut current = metrics.get();
        current.additional_context_bytes = additional_context_bytes;
        current
    })
}

pub fn record_handler_duration(event: &str, handler: &str, duration: Duration, status: &str) {
    let Some(path) = profile_path() else {
        return;
    };
    let record = json!({
        "event": event,
        "handler": handler,
        "status": status,
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "gwt_session_id": std::env::var(GWT_SESSION_ID_ENV).ok(),
        "forward_url_set": std::env::var_os(GWT_HOOK_FORWARD_URL_ENV).is_some(),
        "occurred_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    write_record(&path, &record);
}

pub fn record_event_total(
    event: &str,
    duration: Duration,
    status: &str,
    metrics: HookEventMetrics,
) {
    let Some(path) = profile_path() else {
        return;
    };
    let record = json!({
        "event": event,
        "handler": "event-total",
        "status": status,
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "additional_context_bytes": metrics.additional_context_bytes,
        "provider_read_count": metrics.provider_read_count,
        "history_materialization_count": metrics.history_materialization_count,
        "projection_load_count": metrics.projection_load_count,
        "gwt_session_id": std::env::var(GWT_SESSION_ID_ENV).ok(),
        "forward_url_set": std::env::var_os(GWT_HOOK_FORWARD_URL_ENV).is_some(),
        "occurred_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    write_record(&path, &record);
}

fn profile_path() -> Option<PathBuf> {
    std::env::var_os(GWT_HOOK_PROFILE_PATH_ENV).map(PathBuf::from)
}

fn write_record(path: &Path, record: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = serde_json::to_writer(&mut file, record);
    let _ = file.write_all(b"\n");
}
