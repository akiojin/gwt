//! Opt-in hook timing diagnostics.
//!
//! Hook stdout is part of the Claude/Codex protocol, so diagnostics must never
//! write there. When `GWT_HOOK_PROFILE_PATH` is set, handlers append compact
//! JSONL timing records to that path and otherwise stay silent.

use std::{
    cell::{Cell, RefCell},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use serde_json::json;

const GWT_HOOK_PROFILE_PATH_ENV: &str = "GWT_HOOK_PROFILE_PATH";

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

    /// Handler records buffered until the event finishes.
    ///
    /// Issue #3777: profiling used to open, append to, and close the profile
    /// file once per handler. On a profiled replay that I/O lands inside the
    /// very measurement it is supposed to explain — the handler records summed
    /// to 630ms of a 900ms `event-total`, and the 270ms residue was the
    /// profile writes themselves. Buffering the event's records and writing
    /// them once keeps the profiled replay comparable to an unprofiled one.
    static PENDING_RECORDS: RefCell<Vec<serde_json::Value>> = const { RefCell::new(Vec::new()) };

    /// Whether a dispatcher event is in flight, so its records get buffered.
    /// A direct handler call outside the dispatcher writes immediately, since
    /// nothing would ever flush its buffer.
    static EVENT_IN_FLIGHT: Cell<bool> = const { Cell::new(false) };
}

pub fn begin_event() {
    CURRENT_EVENT_METRICS.with(|metrics| metrics.set(HookEventMetrics::default()));
    PENDING_RECORDS.with(|records| records.borrow_mut().clear());
    EVENT_IN_FLIGHT.with(|in_flight| in_flight.set(true));
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
        "event": normalized_event(event),
        "handler": normalized_handler(handler),
        "status": normalized_status(status),
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "occurred_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    if EVENT_IN_FLIGHT.with(Cell::get) {
        PENDING_RECORDS.with(|records| records.borrow_mut().push(record));
        return;
    }
    write_records(&path, &[record]);
}

pub fn record_event_total(
    event: &str,
    duration: Duration,
    status: &str,
    metrics: HookEventMetrics,
) {
    // The event ends here whether or not anything is being profiled, so the
    // buffer and the in-flight flag are cleared on every path.
    EVENT_IN_FLIGHT.with(|in_flight| in_flight.set(false));
    let mut records = PENDING_RECORDS.with(|records| std::mem::take(&mut *records.borrow_mut()));
    let Some(path) = profile_path() else {
        return;
    };
    records.push(json!({
        "event": normalized_event(event),
        "handler": "event-total",
        "status": normalized_status(status),
        "duration_ms": duration.as_secs_f64() * 1000.0,
        "additional_context_bytes": metrics.additional_context_bytes,
        "provider_read_count": metrics.provider_read_count,
        "history_materialization_count": metrics.history_materialization_count,
        "projection_load_count": metrics.projection_load_count,
        "occurred_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    }));
    write_records(&path, &records);
}

fn normalized_event(value: &str) -> &'static str {
    match value {
        "SessionStart" => "SessionStart",
        "UserPromptSubmit" => "UserPromptSubmit",
        "PreToolUse" => "PreToolUse",
        "PostToolUse" => "PostToolUse",
        "Stop" => "Stop",
        _ => "invalid",
    }
}

fn normalized_status(value: &str) -> &'static str {
    match value {
        "ok" => "ok",
        "error" => "error",
        _ => "unknown",
    }
}

fn normalized_handler(value: &str) -> &'static str {
    match value {
        "runtime-state" => "runtime-state",
        // Issue #3777: `runtime-state` substages. Each name is a fixed literal
        // so the profile stays content-free while showing which read or
        // durable write consumes the UserPromptSubmit budget.
        "runtime-state/session-load" => "runtime-state/session-load",
        "runtime-state/session-metadata" => "runtime-state/session-metadata",
        "runtime-state/pending-resume" => "runtime-state/pending-resume",
        "runtime-state/state-write" => "runtime-state/state-write",
        "runtime-state/live-emit" => "runtime-state/live-emit",
        "forward" => "forward",
        "coordination-event" => "coordination-event",
        "board-reminder" => "board-reminder",
        // Issue #3777: `board-reminder` substages, same fixed-literal contract
        // as the `runtime-state` ones above.
        "board-reminder/context" => "board-reminder/context",
        "board-reminder/reminders-load" => "board-reminder/reminders-load",
        "board-reminder/board-read" => "board-reminder/board-read",
        "board-reminder/suppression" => "board-reminder/suppression",
        "board-reminder/reminders-write" => "board-reminder/reminders-write",
        "workspace-registration" => "workspace-registration",
        "workspace-identity" => "workspace-identity",
        "action-obligation-record" => "action-obligation-record",
        "pm-delivery-ack" => "pm-delivery-ack",
        "pm-loop-reset" => "pm-loop-reset",
        "discussion-goal-start" => "discussion-goal-start",
        "workflow-policy" => "workflow-policy",
        "autonomous-question-guard" => "autonomous-question-guard",
        "autonomous-answer-receipt" => "autonomous-answer-receipt",
        "session-start-session-id-diagnostic" => "session-start-session-id-diagnostic",
        "blocked-stop-runtime-state" => "blocked-stop-runtime-state",
        "completed-stop" => "completed-stop",
        _ => "other",
    }
}

fn profile_path() -> Option<PathBuf> {
    std::env::var_os(GWT_HOOK_PROFILE_PATH_ENV).map(PathBuf::from)
}

fn write_records(path: &Path, records: &[serde_json::Value]) {
    if records.is_empty() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let mut buffer = Vec::new();
    for record in records {
        let _ = serde_json::to_writer(&mut buffer, record);
        buffer.push(b'\n');
    }
    let _ = file.write_all(&buffer);
}
