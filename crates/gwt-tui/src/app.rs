//! App — Update and View functions for the Elm Architecture.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
#[cfg(test)]
use std::fs;
use std::hash::{Hash, Hasher};
#[cfg(test)]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use chrono::{DateTime, Utc};
use gwt_agent::{
    custom::CustomAgentType, persist_session_status, runtime_state_path, AgentDetector, AgentId,
    AgentLaunchBuilder, CustomCodingAgent, DetectedAgent, LaunchConfig, LaunchRuntimeTarget,
    Session as AgentSession, SessionMode, SessionRuntimeState, VersionCache, GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV,
};
use gwt_ai::{suggest_branch_name, AIClient};
use gwt_config::{AISettings, ConfigError, Settings, VoiceConfig};
use gwt_core::logging::{LogEvent as Notification, LogLevel as Severity};
use gwt_core::paths::{gwt_cache_dir, gwt_sessions_dir};
use gwt_skills::{
    distribute_to_worktree, generate_codex_hooks, generate_settings_local, update_git_exclude,
};
use gwt_terminal::protocol::{
    build_paste_input_bytes, key_event_to_bytes, screen_requests_bracketed_paste,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::{
    custom_agents::load_custom_agents,
    input::voice::VoiceInputMessage,
    input_trace,
    message::{GridSessionDirection, Message},
    model::{
        ActiveLayer, ActiveProfileSummary, BranchDetailQueue, DockerProgressEvent,
        DockerProgressQueue, FocusPane, ManagementTab, Model, PendingSessionConversion,
        ScrollbackStrategy, SessionLayout, SessionTabType, TerminalCell, TerminalSelection,
    },
    screens, theme,
};

#[cfg(test)]
use crate::custom_agents::{load_custom_agents_from_path, DISABLE_GLOBAL_CUSTOM_AGENTS_ENV};

static WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
/// Cap branch-detail preload event application per tick so one refresh burst
/// cannot monopolize the UI thread.
const BRANCH_DETAIL_EVENTS_PER_TICK_BUDGET: usize = 8;
const DEFAULT_NEW_BRANCH_BASE_BRANCH: &str = "develop";
const MANAGED_BUNDLE_TRIGGER_ROOTS: &[&str] = &[
    ".claude/skills",
    ".claude/commands",
    // Retired root kept here so startup refresh can still detect and prune
    // stale Claude gwt hook scripts left by older worktrees.
    ".claude/hooks/scripts",
    ".codex/skills",
    // Retired root kept here so startup refresh can still detect and prune
    // stale Codex gwt hook scripts left by older worktrees.
    ".codex/hooks/scripts",
];

// ---------------------------------------------------------------------------
// PTY lifecycle helpers
// ---------------------------------------------------------------------------

/// Spawn a background thread that reads PTY output and sends it to the channel.
fn spawn_pty_reader(
    session_id: String,
    mut reader: Box<dyn std::io::Read + Send>,
    tx: std::sync::mpsc::Sender<(String, Vec<u8>)>,
) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send((session_id.clone(), buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// Spawn a PTY process, start a reader thread, and register the handle on
/// the model.  On failure the error is returned so the caller can notify.
///
/// **Logging policy:** This helper is shared between **shell** and
/// **agent** spawn paths, so it intentionally does NOT log the agent
/// launch event or the env map. Agent-specific spawns must call
/// [`emit_agent_launch_event`] from the agent code path before calling
/// this helper. The trace inside this function is limited to safe
/// metadata (session_id, command name).
#[tracing::instrument(
    name = "spawn_pty",
    skip(model, config),
    fields(session_id = %session_id, command = %config.command)
)]
pub fn spawn_pty_for_session(
    model: &mut Model,
    session_id: &str,
    config: gwt_terminal::pty::SpawnConfig,
) -> Result<(), String> {
    let pty = gwt_terminal::PtyHandle::spawn(config).map_err(|e| {
        tracing::error!(session_id = session_id, error = %e, "PTY spawn failed");
        e.to_string()
    })?;
    let reader = pty.reader().map_err(|e| {
        tracing::error!(session_id = session_id, error = %e, "PTY reader failed");
        e.to_string()
    })?;
    spawn_pty_reader(session_id.to_string(), reader, model.pty_output_tx.clone());
    model.pty_handles.insert(session_id.to_string(), pty);
    tracing::info!(session_id = session_id, "PTY spawned successfully");
    Ok(())
}

/// Emit a structured agent-launch audit event (SPEC-6 FR-020 / FR-016 /
/// reviewer comment B2).
///
/// Called from the agent-only code path right before
/// [`spawn_pty_for_session`]. The event lands in
/// `~/.gwt/logs/gwt.log.YYYY-MM-DD` alongside every other tracing event,
/// and the Logs tab picks it up via the file watcher.
///
/// The env map is **not** included in the event. Custom-agent
/// configurations may inject API keys / tokens into `pty_env` and the
/// log file is world-readable on shared hosts (see B7 file permission
/// hardening). Recording only a presence flag and a count is enough to
/// audit that an agent was launched without persisting secrets.
pub fn emit_agent_launch_event(
    repo_path: &Path,
    session_id: &str,
    config: &gwt_terminal::pty::SpawnConfig,
) {
    tracing::info!(
        target: "gwt_tui::agent::launch",
        repo_path = %repo_path.display(),
        session_id = session_id,
        command = %config.command,
        args = ?config.args,
        cwd = ?config.cwd.as_ref().map(|p| p.display().to_string()),
        env_keys = config.env.len(),
        custom_env = !config.env.is_empty(),
        "agent launch"
    );
}

/// Compute the session pane content size `(cols, rows)` for PTY/VtState
/// initialization.  Falls back to `model.terminal_size` when the layout
/// geometry is not yet available (e.g. during early startup).
pub fn session_content_size(model: &Model) -> (u16, u16) {
    active_session_content_area(model)
        .map(|r| (r.width, r.height))
        .unwrap_or(model.terminal_size)
}

fn sync_session_viewports(model: &mut Model) {
    let Some(session_area) = visible_session_area(model) else {
        return;
    };

    for session_idx in 0..model.sessions.len() {
        let Some(content) = session_content_area_for_index(model, session_area, session_idx) else {
            continue;
        };
        let render_width = model
            .sessions
            .get(session_idx)
            .map(|session| session_text_area(session, content).width)
            .unwrap_or(content.width);

        let session_id = model.sessions[session_idx].id.clone();
        if let Some(pty) = model.pty_handles.get(&session_id) {
            let _ = pty.resize(render_width, content.height);
        }

        let session = &mut model.sessions[session_idx];
        let current_scrollback = session.vt.scrollback();
        session.vt.resize(content.height, render_width);
        session
            .vt
            .set_scrollback(current_scrollback.min(session.vt.max_scrollback()));
    }

    if let Some(session) = model.active_session_tab() {
        let content = active_session_content_area(model).unwrap_or(session_area);
        let render_width = session_text_area(session, content).width;
        crate::scroll_debug::log_lazy(|| {
            format!(
            "event=viewport_sync session={} content_width={} content_height={} render_width={} vt_rows={} vt_cols={} scrollback={} max_scrollback={} follow_live={}",
            session.id,
            content.width,
            content.height,
            render_width,
            session.vt.rows(),
            session.vt.cols(),
            session.vt.scrollback(),
            session.vt.max_scrollback(),
            session.vt.follow_live(),
        )
        });
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct AgentTranscriptSource {
    path: PathBuf,
    modified: Option<SystemTime>,
    started_at: Option<SystemTime>,
    session_key: Option<String>,
}

#[cfg(test)]
fn sync_active_agent_transcript_scrollback_with(
    model: &mut Model,
    sessions_dir: &Path,
    claude_projects_root: &Path,
    codex_sessions_root: &Path,
) {
    let Some(session_tab) = model.active_session_tab() else {
        return;
    };
    let SessionTabType::Agent { agent_id, .. } = &session_tab.tab_type else {
        return;
    };
    let persisted_path = sessions_dir.join(format!("{}.toml", session_tab.id));
    let Ok(persisted) = AgentSession::load(&persisted_path) else {
        return;
    };
    let agent_id = match agent_id.as_str() {
        "claude" => AgentId::ClaudeCode,
        "codex" => AgentId::Codex,
        _ => return,
    };
    let source = match agent_id {
        AgentId::ClaudeCode => resolve_claude_transcript_source(&persisted, claude_projects_root),
        AgentId::Codex => resolve_codex_transcript_source(&persisted, codex_sessions_root),
        _ => None,
    };
    let _ = source
        .as_ref()
        .and_then(|source| read_transcript_lines_for_agent(&agent_id, &source.path));
}

#[cfg(test)]
#[allow(dead_code)]
fn file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
#[allow(dead_code)]
fn parse_rfc3339_system_time(value: &str) -> Option<SystemTime> {
    let parsed = DateTime::parse_from_rfc3339(value).ok()?;
    Some(parsed.with_timezone(&Utc).into())
}

#[cfg(test)]
#[allow(dead_code)]
fn transcript_source_started_at(path: &Path) -> Option<SystemTime> {
    file_modified_time(path)
}

#[cfg(test)]
#[allow(dead_code)]
fn transcript_source_selection_distance(
    source: &AgentTranscriptSource,
    session_started_at: SystemTime,
) -> Option<Duration> {
    let started_at = source
        .started_at
        .or_else(|| transcript_source_started_at(&source.path))?;
    if started_at >= session_started_at {
        started_at.duration_since(session_started_at).ok()
    } else {
        session_started_at.duration_since(started_at).ok()
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn agent_session_started_at(session: &AgentSession) -> Option<SystemTime> {
    let secs = session.created_at.timestamp();
    let nanos = session.created_at.timestamp_subsec_nanos() as u64;
    if secs >= 0 {
        Some(UNIX_EPOCH + Duration::from_secs(secs as u64) + Duration::from_nanos(nanos))
    } else {
        let offset = Duration::from_secs(secs.unsigned_abs()) + Duration::from_nanos(nanos);
        UNIX_EPOCH.checked_sub(offset)
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn select_transcript_source_for_session(
    session: &AgentSession,
    candidates: Vec<AgentTranscriptSource>,
) -> Option<AgentTranscriptSource> {
    if let Some(session_key) = session.agent_session_id.as_deref() {
        if let Some(exact) = candidates
            .iter()
            .find(|candidate| candidate.session_key.as_deref() == Some(session_key))
            .cloned()
        {
            return Some(exact);
        }
    }

    let session_started_at = agent_session_started_at(session);
    if let Some(session_started_at) = session_started_at {
        let mut best: Option<(Duration, AgentTranscriptSource)> = None;
        for candidate in candidates.iter().cloned() {
            let Some(distance) =
                transcript_source_selection_distance(&candidate, session_started_at)
            else {
                continue;
            };
            let replace = best
                .as_ref()
                .map(|(best_distance, best_source)| {
                    distance < *best_distance
                        || (distance == *best_distance
                            && transcript_source_newer_than(&candidate, best_source))
                })
                .unwrap_or(true);
            if replace {
                best = Some((distance, candidate));
            }
        }
        if let Some((_, candidate)) = best {
            return Some(candidate);
        }
    }

    candidates.into_iter().max_by(|left, right| {
        let left_modified = left.modified.unwrap_or(UNIX_EPOCH);
        let right_modified = right.modified.unwrap_or(UNIX_EPOCH);
        left_modified.cmp(&right_modified)
    })
}

#[cfg(test)]
#[allow(dead_code)]
fn resolve_claude_transcript_source(
    session: &AgentSession,
    claude_projects_root: &Path,
) -> Option<AgentTranscriptSource> {
    let encoded_worktree = session.worktree_path.to_string_lossy().replace('/', "-");
    let dir = claude_projects_root.join(encoded_worktree);
    let entries = fs::read_dir(dir).ok()?;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = file_modified_time(&path);
        if modified.is_none() {
            continue;
        }
        let started_at = claude_transcript_started_at(&path);
        let session_key = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string);
        candidates.push(AgentTranscriptSource {
            path,
            modified,
            started_at,
            session_key,
        });
    }
    select_transcript_source_for_session(session, candidates)
}

#[cfg(test)]
#[allow(dead_code)]
fn resolve_codex_transcript_source(
    session: &AgentSession,
    codex_sessions_root: &Path,
) -> Option<AgentTranscriptSource> {
    let mut candidates = Vec::new();
    for candidate in collect_jsonl_files(codex_sessions_root) {
        let Some(metadata) = codex_transcript_metadata(&candidate) else {
            continue;
        };
        if metadata.cwd != session.worktree_path {
            continue;
        }
        let modified = file_modified_time(&candidate);
        if modified.is_none() {
            continue;
        }
        candidates.push(AgentTranscriptSource {
            path: candidate,
            modified,
            started_at: metadata.started_at,
            session_key: metadata.session_key,
        });
    }
    select_transcript_source_for_session(session, candidates)
}

#[cfg(test)]
#[allow(dead_code)]
fn transcript_source_newer_than(
    candidate: &AgentTranscriptSource,
    current: &AgentTranscriptSource,
) -> bool {
    let candidate_modified = candidate.modified.unwrap_or(UNIX_EPOCH);
    let current_modified = current.modified.unwrap_or(UNIX_EPOCH);
    candidate_modified > current_modified
}

#[cfg(test)]
#[allow(dead_code)]
fn claude_transcript_started_at(path: &Path) -> Option<SystemTime> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for raw in reader.lines().take(16).map_while(Result::ok) {
        let event: Value = serde_json::from_str(&raw).ok()?;
        if let Some(timestamp) = event.get("timestamp").and_then(Value::as_str) {
            return parse_rfc3339_system_time(timestamp);
        }
    }
    None
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct CodexTranscriptMetadata {
    cwd: PathBuf,
    started_at: Option<SystemTime>,
    session_key: Option<String>,
}

#[cfg(test)]
#[allow(dead_code)]
fn codex_transcript_metadata(path: &Path) -> Option<CodexTranscriptMetadata> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).ok()? == 0 {
        return None;
    }
    let payload: Value = serde_json::from_str(first_line.trim_end()).ok()?;
    if payload.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let inner = payload.get("payload")?;
    Some(CodexTranscriptMetadata {
        cwd: inner
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)?,
        started_at: inner
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_system_time),
        session_key: inner.get("id").and_then(Value::as_str).map(str::to_string),
    })
}

#[cfg(test)]
#[allow(dead_code)]
fn collect_jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_files_recursive(root, 0, 4, &mut files);
    files
}

#[cfg(test)]
#[allow(dead_code)]
fn collect_jsonl_files_recursive(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files_recursive(&path, depth + 1, max_depth, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn read_transcript_lines_for_agent(agent_id: &AgentId, path: &Path) -> Option<Vec<String>> {
    match agent_id {
        AgentId::ClaudeCode => read_claude_transcript_lines(path),
        AgentId::Codex => read_codex_transcript_lines(path),
        _ => None,
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn read_codex_transcript_lines(path: &Path) -> Option<Vec<String>> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for raw in reader.lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = event.get("payload") else {
            continue;
        };
        match payload.get("type").and_then(Value::as_str) {
            Some("message") => append_codex_message_lines(&mut lines, payload),
            Some("function_call_output") => {
                if let Some(output) = payload.get("output").and_then(Value::as_str) {
                    append_transcript_raw_lines(&mut lines, output);
                }
            }
            _ => {}
        }
    }
    Some(lines)
}

#[cfg(test)]
#[allow(dead_code)]
fn read_claude_transcript_lines(path: &Path) -> Option<Vec<String>> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for raw in reader.lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(role) = event.get("type").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        append_claude_event_lines(&mut lines, role, &event);
    }
    Some(lines)
}

#[cfg(test)]
#[allow(dead_code)]
fn append_codex_message_lines(lines: &mut Vec<String>, payload: &Value) {
    let Some(role) = payload.get("role").and_then(Value::as_str) else {
        return;
    };
    if !matches!(role, "user" | "assistant") {
        return;
    }
    let Some(content) = payload.get("content").and_then(Value::as_array) else {
        return;
    };
    let mut merged = Vec::new();
    for item in content {
        let Some(item_type) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(item_type, "input_text" | "output_text" | "text") {
            continue;
        }
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            merged.push(text);
        }
    }
    let text = merged.join("\n").trim().to_string();
    if text.is_empty() {
        return;
    }
    append_transcript_message_lines(lines, role, &text);
}

#[cfg(test)]
#[allow(dead_code)]
fn append_claude_event_lines(lines: &mut Vec<String>, role: &str, event: &Value) {
    let Some(message) = event.get("message") else {
        return;
    };
    let Some(content) = message.get("content") else {
        return;
    };
    if let Some(text) = content.as_str() {
        let text = text.trim();
        if !text.is_empty() {
            append_transcript_message_lines(lines, role, text);
        }
        return;
    }

    let Some(items) = content.as_array() else {
        return;
    };
    let mut merged = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    merged.push(text.to_string());
                }
            }
            Some("tool_result") => {
                if !merged.is_empty() {
                    let text = merged.join("\n");
                    append_transcript_message_lines(lines, role, text.trim());
                    merged.clear();
                }
                if let Some(text) = item.get("content").and_then(Value::as_str) {
                    append_transcript_raw_lines(lines, text);
                }
            }
            _ => {}
        }
    }
    if !merged.is_empty() {
        let text = merged.join("\n");
        let text = text.trim();
        if !text.is_empty() {
            append_transcript_message_lines(lines, role, text);
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn append_transcript_message_lines(lines: &mut Vec<String>, role: &str, text: &str) {
    for (index, raw_line) in text.lines().enumerate() {
        if index == 0 {
            lines.push(format!("{role}: {raw_line}"));
        } else {
            lines.push(format!("  {raw_line}"));
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn append_transcript_raw_lines(lines: &mut Vec<String>, text: &str) {
    for raw_line in text.lines() {
        lines.push(raw_line.to_string());
    }
}

/// Drain buffered PTY input and write it to the corresponding PTY handles.
fn drain_pending_pty_inputs(model: &mut Model) {
    while let Some(input) = model.pending_pty_inputs.pop_front() {
        if let Some(pty) = model.pty_handles.get(&input.session_id) {
            if let Err(e) = pty.write_input(&input.bytes) {
                tracing::warn!("PTY write error for {}: {e}", input.session_id);
            }
        }
    }
}

/// Poll live PTY handles for process exit and notify the user.
fn check_pty_exits(model: &mut Model) {
    check_pty_exits_with(model, &gwt_sessions_dir());
}

fn focus_session_by_id(model: &mut Model, session_id: &str) -> bool {
    if let Some(index) = model
        .sessions
        .iter()
        .position(|session| session.id == session_id)
    {
        model.active_layer = ActiveLayer::Main;
        model.active_session = index;
        model.active_focus = FocusPane::Terminal;
        true
    } else {
        false
    }
}

fn check_pty_exits_with(model: &mut Model, sessions_dir: &Path) {
    let exited: Vec<String> = model
        .pty_handles
        .iter()
        .filter_map(|(id, pty)| match pty.try_wait() {
            Ok(Some(_)) => Some(id.clone()),
            _ => None,
        })
        .collect();

    for id in exited {
        model.pty_handles.remove(&id);
        if let Some(index) = model.sessions.iter().position(|session| session.id == id) {
            if matches!(model.sessions[index].tab_type, SessionTabType::Agent { .. }) {
                persist_agent_session_stopped(sessions_dir, &id);
            }
            model.sessions.remove(index);
            if model.sessions.is_empty() {
                model.active_session = 0;
            } else if model.active_session >= model.sessions.len() {
                model.active_session = model.sessions.len() - 1;
            } else if index < model.active_session {
                model.active_session = model.active_session.saturating_sub(1);
            }
        }
        apply_notification(
            model,
            Notification::new(
                Severity::Info,
                "session",
                format!("Session {id} exited and closed"),
            ),
        );
    }

    refresh_branch_live_session_summaries_with(model, sessions_dir);
}

fn persist_agent_session_stopped(sessions_dir: &Path, session_id: &str) {
    if let Err(err) =
        persist_session_status(sessions_dir, session_id, gwt_agent::AgentStatus::Stopped)
    {
        tracing::warn!(session_id, error = %err, "failed to persist stopped agent session");
    }
}

fn bootstrap_agent_session_waiting_input(sessions_dir: &Path, session_id: &str) {
    let runtime_path = runtime_state_path(sessions_dir, session_id);
    if runtime_path.exists() {
        return;
    }

    let mut runtime = SessionRuntimeState::new(gwt_agent::AgentStatus::WaitingInput);
    runtime.source_event = Some("LaunchBootstrap".to_string());
    if let Err(err) = runtime.save(&runtime_path) {
        tracing::warn!(session_id, error = %err, "failed to bootstrap waiting runtime state");
    }
}

fn inject_agent_hook_runtime_env(
    env: &mut HashMap<String, String>,
    sessions_dir: &Path,
    session_id: &str,
) {
    env.insert(GWT_SESSION_ID_ENV.to_string(), session_id.to_string());
    env.insert(
        GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
        runtime_state_path(sessions_dir, session_id)
            .to_string_lossy()
            .into_owned(),
    );
}

fn augment_agent_hook_runtime_launch_config(
    config: &mut LaunchConfig,
    sessions_dir: &Path,
    session_id: &str,
) {
    if config.agent_id != AgentId::Codex {
        return;
    }

    let Some(runtime_dir) = runtime_state_path(sessions_dir, session_id)
        .parent()
        .map(|dir| dir.to_string_lossy().into_owned())
    else {
        return;
    };

    if config
        .args
        .windows(2)
        .any(|pair| pair[0] == "--add-dir" && pair[1] == runtime_dir)
    {
        return;
    }

    config.args.push("--add-dir".to_string());
    config.args.push(runtime_dir);
}

fn refresh_branch_live_session_summaries(model: &mut Model) {
    refresh_branch_live_session_summaries_with(model, &gwt_sessions_dir());
}

fn refresh_branch_live_session_summaries_with(model: &mut Model, sessions_dir: &Path) {
    model.branches.live_session_summaries = branch_live_session_summaries_with(model, sessions_dir);
}

/// Process a message and update the model (Elm: update).
pub fn update(model: &mut Model, msg: Message) {
    let previous_active_session = model.active_session;
    let previous_active_focus = model.active_focus;
    let previous_session_count = model.sessions.len();
    let previous_session_layout = model.session_layout;

    match msg {
        Message::Quit => {
            model.quit = true;
        }
        Message::TerminalLost => {
            tracing::warn!("Controlling terminal lost — shutting down gracefully");
            model.quit = true;
        }
        Message::ToggleLayer => {
            match model.active_layer {
                ActiveLayer::Initialization => {} // blocked
                ActiveLayer::Main => {
                    model.active_layer = ActiveLayer::Management;
                    model.active_focus = FocusPane::TabContent;
                    sync_session_viewports(model);
                }
                ActiveLayer::Management => {
                    model.active_layer = ActiveLayer::Main;
                    model.active_focus = FocusPane::Terminal;
                    sync_session_viewports(model);
                }
            }
        }
        Message::FocusNext => {
            if !is_in_text_input_mode(model) {
                cycle_focus_with_shortcut(model, false);
            }
        }
        Message::FocusPrev => {
            if !is_in_text_input_mode(model) {
                cycle_focus_with_shortcut(model, true);
            }
        }
        Message::SwitchManagementTab(tab) => {
            switch_management_tab(model, tab);
        }
        Message::NextSession => {
            if !model.sessions.is_empty() {
                model.active_session = (model.active_session + 1) % model.sessions.len();
            }
        }
        Message::PrevSession => {
            if !model.sessions.is_empty() {
                model.active_session = if model.active_session == 0 {
                    model.sessions.len() - 1
                } else {
                    model.active_session - 1
                };
            }
        }
        Message::SwitchSession(idx) => {
            if idx < model.sessions.len() {
                model.active_session = idx;
            }
        }
        Message::MoveGridSession(direction) => {
            move_grid_session(model, direction);
        }
        Message::ToggleSessionLayout => {
            model.session_layout = match model.session_layout {
                SessionLayout::Tab => SessionLayout::Grid,
                SessionLayout::Grid => SessionLayout::Tab,
            };
        }
        Message::NewShell => {
            let idx = model.sessions.len();
            let session = crate::model::SessionTab {
                id: format!("shell-{idx}"),
                name: format!("Shell {}", idx + 1),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            };
            let session_id = session.id.clone();
            model.sessions.push(session);
            model.active_session = idx;

            // Use actual pane content area for PTY size.
            let (cols, rows) = session_content_size(model);

            // Resize VtState to match.
            if let Some(s) = model.sessions.last_mut() {
                s.vt.resize(rows, cols);
            }

            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            let (env, remove_env) = spawn_env_with_active_profile(HashMap::new());
            let config = gwt_terminal::pty::SpawnConfig {
                command: shell,
                args: vec![],
                cols,
                rows,
                env,
                remove_env,
                cwd: Some(model.repo_path.clone()),
            };
            if let Err(e) = spawn_pty_for_session(model, &session_id, config) {
                apply_notification(
                    model,
                    Notification::new(Severity::Error, "pty", format!("Shell spawn failed: {e}")),
                );
            }
        }
        Message::CloseSession => {
            close_active_session_with(model, &gwt_sessions_dir());
        }
        Message::Resize(w, h) => {
            model.terminal_size = (w, h);
            sync_session_viewports(model);
        }
        Message::PtyOutput(pane_id, data) => {
            if let Some(session) = model.session_tab_mut(&pane_id) {
                session.vt.process(&data);
                crate::scroll_debug::log_lazy(|| {
                    format!(
                    "event=pty_output session={} bytes={} vt_rows={} vt_cols={} scrollback={} max_scrollback={} follow_live={}",
                    pane_id,
                    data.len(),
                    session.vt.rows(),
                    session.vt.cols(),
                    session.vt.scrollback(),
                    session.vt.max_scrollback(),
                    session.vt.follow_live(),
                )
                });
            }
            if model
                .active_session_tab()
                .is_some_and(|session| session.id == pane_id)
            {
                sync_session_viewports(model);
            }
        }
        Message::PushError(err) => {
            model
                .error_queue
                .push_back(Notification::new(Severity::Error, "app", err));
        }
        Message::PushErrorNotification(notification) => {
            model.error_queue.push_back(notification);
        }
        Message::Notify(notification) => {
            apply_notification(model, notification);
        }
        Message::ToggleHelp => {
            model.help_visible = !model.help_visible;
        }
        Message::ShowNotification(notification) => match notification.severity {
            Severity::Info => {
                model.current_notification = Some(notification);
                model.current_notification_ttl = Some(Duration::from_secs(5));
            }
            Severity::Warn => {
                model.current_notification = Some(notification);
                model.current_notification_ttl = None;
            }
            Severity::Debug | Severity::Error => {}
        },
        Message::DismissNotification => {
            model.current_notification = None;
            model.current_notification_ttl = None;
        }
        Message::DismissError => {
            model.error_queue.pop_front();
        }
        Message::Tick => {
            drain_notification_bus(model);
            model.drain_logs_watcher();
            drain_ui_log_events(model);
            drain_docker_progress_events(model);
            drain_branch_detail_events(model);
            drain_cleanup_events(model);
            drain_merge_state_events(model);
            if model.branches.has_computing_branches() {
                model.branches.tick_merge_spinner();
            }
            tick_notification(model);
            check_pty_exits(model);
            model.branches.session_animation_tick =
                model.branches.session_animation_tick.wrapping_add(1);
            refresh_branch_live_session_summaries(model);
            // Forward tick to wizard (AI suggest spinner) when active
            if let Some(ref mut wizard) = model.wizard {
                if wizard.ai_suggest.loading {
                    screens::wizard::update(wizard, screens::wizard::WizardMessage::Tick);
                }
            }
            // Forward tick to voice input when recording/transcribing
            if model.voice.is_active() {
                crate::input::voice::update(&mut model.voice, VoiceInputMessage::Tick);
            }
        }
        Message::KeyInput(key) => {
            if route_overlay_key(model, key) {
            } else if model.active_layer == ActiveLayer::Initialization {
                route_key_to_initialization(model, key);
            } else if model.active_layer == ActiveLayer::Management {
                // Dispatch based on focused pane
                match model.active_focus {
                    FocusPane::TabContent => route_key_to_management(model, key),
                    FocusPane::BranchDetail => route_key_to_branch_detail(model, key),
                    FocusPane::Terminal => forward_key_to_active_session(model, key),
                }

                // Check pending actions after key dispatch
                check_branch_pending_actions(model);
            } else {
                forward_key_to_active_session(model, key);
            }
        }
        Message::MouseInput(mouse) => {
            handle_mouse_input(model, mouse);
        }
        Message::Branches(msg) => {
            screens::branches::update(&mut model.branches, msg);
            check_branch_pending_actions(model);
            if let Some(action) = model.branches.pending_docker_action.take() {
                handle_pending_branch_docker_action(model, action);
            }
        }
        Message::Profiles(msg) => {
            screens::profiles::update(&mut model.profiles, msg);
        }
        Message::Issues(msg) => {
            handle_issues_message(model, msg);
        }
        Message::GitView(msg) => {
            screens::git_view::update(&mut model.git_view, msg);
        }
        Message::PrDashboard(msg) => {
            screens::pr_dashboard::update(&mut model.pr_dashboard, msg);
        }
        Message::Settings(msg) => {
            screens::settings::update(&mut model.settings, msg);
        }
        Message::Logs(msg) => {
            screens::logs::update(&mut model.logs, msg);
        }
        Message::Versions(msg) => {
            screens::versions::update(&mut model.versions, msg);
        }
        Message::Wizard(msg) => {
            let (launch_config, focus_session_id) = if let Some(ref mut wizard) = model.wizard {
                screens::wizard::update(wizard, msg);
                let project_root = wizard
                    .worktree_path
                    .clone()
                    .unwrap_or_else(|| model.repo_path.clone());
                sync_wizard_docker_status(wizard, &project_root);
                maybe_start_wizard_branch_suggestions(wizard);
                let completed = wizard.completed;
                let focus_session_id = if completed {
                    wizard.focus_session_id.clone()
                } else {
                    None
                };
                let launch_config = if completed && focus_session_id.is_none() {
                    Some(build_launch_config_from_wizard(wizard))
                } else {
                    None
                };
                if wizard.completed || wizard.cancelled {
                    model.wizard = None;
                }
                (launch_config, focus_session_id)
            } else {
                (None, None)
            };
            if let Some(session_id) = focus_session_id {
                if !focus_session_by_id(model, &session_id) {
                    apply_notification(
                        model,
                        Notification::new(
                            Severity::Warn,
                            "session",
                            format!("Session {session_id} is no longer available"),
                        ),
                    );
                }
            } else if let Some(config) = launch_config {
                model.pending_launch_config = Some(config);
                materialize_pending_launch(model);
                model.active_focus = FocusPane::Terminal;
            }
        }
        Message::DockerProgress(msg) => {
            let should_create = matches!(
                msg,
                screens::docker_progress::DockerProgressMessage::SetStage { .. }
                    | screens::docker_progress::DockerProgressMessage::Advance
                    | screens::docker_progress::DockerProgressMessage::SetError(_)
            );
            if model.docker_progress.is_none() && should_create {
                let mut state = screens::docker_progress::DockerProgressState::default();
                state.show();
                model.docker_progress = Some(state);
            }
            if let Some(ref mut state) = model.docker_progress {
                let hide_after = matches!(
                    msg,
                    screens::docker_progress::DockerProgressMessage::Hide
                        | screens::docker_progress::DockerProgressMessage::Reset
                );
                screens::docker_progress::update(state, msg);
                if hide_after || !state.visible {
                    model.docker_progress = None;
                }
            }
        }
        Message::ServiceSelect(msg) => {
            let selected_conversion =
                if matches!(msg, screens::service_select::ServiceSelectMessage::Select) {
                    model.service_select.as_ref().and_then(|state| {
                        state
                            .current_selection()
                            .map(|(service, value)| PendingSessionConversion {
                                session_index: model.active_session,
                                target_agent_id: value.to_string(),
                                target_display_name: service.to_string(),
                            })
                    })
                } else {
                    None
                };
            let cancelled = matches!(msg, screens::service_select::ServiceSelectMessage::Cancel);
            if let Some(ref mut state) = model.service_select {
                screens::service_select::update(state, msg);
                if !state.visible {
                    model.service_select = None;
                }
            }
            if cancelled {
                model.pending_session_conversion = None;
            }
            if let Some(pending) = selected_conversion {
                model.confirm = screens::confirm::ConfirmState::with_message(format!(
                    "Convert session to {}?",
                    pending.target_display_name
                ));
                model.pending_session_conversion = Some(pending);
            }
        }
        Message::PortSelect(msg) => {
            if let Some(ref mut state) = model.port_select {
                screens::port_select::update(state, msg);
                if !state.visible {
                    model.port_select = None;
                }
            }
        }
        Message::Confirm(msg) => {
            handle_confirm_message(model, msg);
        }
        Message::CleanupConfirm(msg) => {
            handle_cleanup_confirm_message(model, msg);
        }
        Message::CleanupProgress(msg) => {
            handle_cleanup_progress_message(model, msg);
        }
        Message::Voice(msg) => {
            let voice_config = Settings::load()
                .map(|settings| settings.voice)
                .unwrap_or_default();
            let mut runtime = std::mem::take(&mut model.voice_runtime);
            handle_voice_message_with_config_and_runtime(model, msg, &voice_config, &mut runtime);
            model.voice_runtime = runtime;
        }
        Message::Initialization(msg) => {
            use screens::initialization::InitializationMessage;
            if let Some(ref mut state) = model.initialization {
                match msg {
                    InitializationMessage::Exit => {
                        model.quit = true;
                    }
                    InitializationMessage::StartClone => {
                        let url = state.url_input.clone();
                        let target = model.repo_path.clone();
                        state.clone_status = screens::initialization::CloneStatus::Cloning;
                        match gwt_git::clone_repo(&url, &target) {
                            Ok(path) => {
                                let _ = gwt_git::install_develop_protection(&path);
                                let workspace_warning = gwt_git::initialize_workspace(&path)
                                    .err()
                                    .map(workspace_initialization_warning);
                                model.reset(path);
                                load_initial_data(model);
                                if let Some(notification) = workspace_warning {
                                    apply_notification(model, notification);
                                }
                            }
                            Err(e) => {
                                state.clone_status =
                                    screens::initialization::CloneStatus::Error(e.to_string());
                            }
                        }
                    }
                    other => {
                        screens::initialization::update(state, other);
                    }
                }
            }
        }
        Message::PasteInput(text) => route_paste_input(model, text),
        Message::OpenSessionConversion => {
            open_session_conversion(model);
        }
        Message::OpenWizardWithSpec(spec_context) => {
            open_wizard(model, Some(spec_context));
        }
        Message::OpenWizardWithIssue(issue_number) => {
            open_wizard_with_issue(model, issue_number);
        }
        Message::CloseWizard => {
            model.wizard = None;
        }
    }

    clear_terminal_trackpad_scroll_row_if_context_changed(
        model,
        previous_active_session,
        previous_active_focus,
    );

    if previous_session_count != model.sessions.len()
        || previous_session_layout != model.session_layout
    {
        sync_session_viewports(model);
    }
    if previous_active_session != model.active_session {
        refresh_branch_live_session_summaries(model);
    }

    // Flush buffered PTY input after every message so keystrokes reach the PTY
    // without waiting for the next Tick.
    drain_pending_pty_inputs(model);
}

/// Load initial data from the repository into the model.
///
/// Populates branches, version tags, and worktree mappings.  Each section is
/// best-effort: failures are silently ignored so the TUI still starts.
pub fn load_initial_data(model: &mut Model) {
    load_initial_data_with(model, fetch_current_pr_link, gwt_git::fetch_pr_list);
}

fn load_initial_data_with<P, F>(model: &mut Model, load_pr_link: P, load_prs: F)
where
    P: FnOnce(&std::path::Path) -> gwt_core::Result<Option<String>>,
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
{
    schedule_startup_version_cache_refresh();
    let has_git_remote = repo_has_git_remote(&model.repo_path);
    let issue_cache_root = default_issue_cache_root(&model.repo_path);
    if let Err(err) = crate::issue_cache::sync_issue_cache_from_remote_if_missing(
        &model.repo_path,
        &issue_cache_root,
    ) {
        tracing::warn!("startup issue cache sync failed: {err}");
    }
    reload_cached_issues_with_paths(
        model,
        issue_cache_root.clone(),
        default_issue_linkage_store_path(&model.repo_path),
    );
    model.specs.cache_root = issue_cache_root.clone();
    model.specs.reload_from_cache();
    if !issue_cache_root.exists() {
        model.specs.last_error = None;
    }

    // -- Branches --
    if let Ok(branches) = gwt_git::branch::list_branches(&model.repo_path) {
        let items: Vec<screens::branches::BranchItem> = branches
            .iter()
            .map(|b| screens::branches::BranchItem {
                name: b.name.clone(),
                is_head: b.is_head,
                is_local: b.is_local,
                category: screens::branches::categorize_branch(&b.name),
                worktree_path: None,
                upstream: b.upstream.clone(),
            })
            .collect();
        screens::branches::update(
            &mut model.branches,
            screens::branches::BranchesMessage::SetBranches(items),
        );
    }

    // -- Version tags --
    let repo_str = model.repo_path.to_string_lossy().to_string();
    if let Ok(output) = gwt_core::process::run_command(
        "git",
        &[
            "-C",
            &repo_str,
            "tag",
            "-l",
            "--sort=-v:refname",
            "--format=%(refname:short)\t%(creatordate:short)\t%(subject)",
        ],
    ) {
        let tags: Vec<screens::versions::VersionTag> = output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                screens::versions::VersionTag {
                    name: parts.first().unwrap_or(&"").to_string(),
                    date: parts.get(1).unwrap_or(&"").to_string(),
                    message: parts.get(2).unwrap_or(&"").to_string(),
                }
            })
            .collect();
        screens::versions::update(
            &mut model.versions,
            screens::versions::VersionsMessage::SetTags(tags),
        );
    }

    // -- Worktree → branch mapping --
    if let Ok(worktrees) = gwt_git::WorktreeManager::new(&model.repo_path).list() {
        // Track every branch that any worktree currently checks out so the
        // Branch Cleanup flow can refuse to delete them (FR-018b).
        let mut checked_out: std::collections::HashSet<String> = std::collections::HashSet::new();
        for wt in &worktrees {
            if let Some(ref branch_name) = wt.branch {
                checked_out.insert(branch_name.clone());
                // Match worktree branch to existing BranchItem
                if let Some(item) = model
                    .branches
                    .branches
                    .iter_mut()
                    .find(|b| &b.name == branch_name)
                {
                    item.worktree_path = Some(wt.path.clone());
                }
            }
        }
        model.branches.checked_out_branches = checked_out;
    }
    refresh_managed_gwt_assets_for_repo_worktrees(model);

    // Refresh the protection inputs the Cleanup gutter consults. The HEAD
    // branch tracks the gwt-tui process itself; active session branches are
    // filled in by the session/PTY pipeline elsewhere.
    model.branches.current_head_branch = model
        .branches
        .branches
        .iter()
        .find(|b| b.is_head)
        .map(|b| b.name.clone());
    if let Some(head_branch_name) = model.branches.current_head_branch.clone() {
        if let Some(item) = model
            .branches
            .branches
            .iter_mut()
            .find(|branch| branch.name == head_branch_name && branch.worktree_path.is_none())
        {
            item.worktree_path = Some(model.repo_path.clone());
        }
    }
    refresh_active_session_branches(model);

    // Compute Branch Cleanup merge state (FR-018a/d). This is currently a
    // synchronous walk; for large repositories it can be moved into the
    // existing branch-detail preload pipeline in a follow-up.
    refresh_cleanup_merge_state(model);

    schedule_branch_detail_prefetch(model);

    // -- Git View --
    load_git_view_with(
        model,
        gwt_git::diff::get_status,
        |repo_path| gwt_git::commit::recent_commits(repo_path, 10),
        gwt_git::branch::list_branches,
        |repo_path| {
            if has_git_remote {
                load_pr_link(repo_path)
            } else {
                Ok(None)
            }
        },
    );

    if has_git_remote {
        load_pr_dashboard_with(model, load_prs);
    }
}

fn repo_has_git_remote(repo_path: &std::path::Path) -> bool {
    let output = match Command::new("git")
        .args(["remote"])
        .current_dir(repo_path)
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };

    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

fn load_git_view_with<S, C, B, P>(
    model: &mut Model,
    load_status: S,
    load_commits: C,
    load_branches: B,
    load_pr_link: P,
) where
    S: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::diff::FileEntry>>,
    C: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::commit::CommitEntry>>,
    B: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::Branch>>,
    P: FnOnce(&std::path::Path) -> gwt_core::Result<Option<String>>,
{
    if let Ok(entries) = load_status(&model.repo_path) {
        let files = entries
            .into_iter()
            .map(|entry| {
                let diff_preview = entry.diff_content(&model.repo_path).unwrap_or_default();
                screens::git_view::GitFileItem {
                    path: entry.path.display().to_string(),
                    status: match entry.status {
                        gwt_git::diff::FileStatus::Staged => screens::git_view::FileStatus::Staged,
                        gwt_git::diff::FileStatus::Unstaged => {
                            screens::git_view::FileStatus::Unstaged
                        }
                        gwt_git::diff::FileStatus::Untracked => {
                            screens::git_view::FileStatus::Untracked
                        }
                    },
                    diff_preview,
                }
            })
            .collect();
        screens::git_view::update(
            &mut model.git_view,
            screens::git_view::GitViewMessage::SetFiles(files),
        );
    }

    if let Ok(entries) = load_commits(&model.repo_path) {
        let commits = entries
            .into_iter()
            .map(|entry| screens::git_view::GitCommitItem {
                hash: entry.hash,
                subject: entry.subject,
                author: entry.author,
                date: entry.timestamp.chars().take(10).collect(),
            })
            .collect();
        screens::git_view::update(
            &mut model.git_view,
            screens::git_view::GitViewMessage::SetCommits(commits),
        );
    }

    let divergence_summary = load_branches(&model.repo_path)
        .ok()
        .and_then(|branches| git_view_divergence_summary(&branches));
    let pr_link = load_pr_link(&model.repo_path).ok().flatten();
    screens::git_view::update(
        &mut model.git_view,
        screens::git_view::GitViewMessage::SetMetadata {
            divergence_summary,
            pr_link,
        },
    );
}

fn refresh_managed_gwt_assets_for_repo_worktrees(model: &Model) {
    let mut paths = std::collections::HashSet::new();
    paths.insert(model.repo_path().to_path_buf());
    paths.extend(model.active_worktree_paths());

    for worktree in paths {
        if !path_is_git_repo_or_worktree(&worktree) {
            continue;
        }
        refresh_managed_gwt_assets_for_worktree(
            &worktree,
            "startup refresh",
            worktree_has_managed_asset_state(&worktree),
        );
    }
}

fn path_is_git_repo_or_worktree(worktree: &Path) -> bool {
    match Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
    {
        Ok(output) => {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        }
        Err(_) => false,
    }
}

fn worktree_has_managed_asset_state(worktree: &Path) -> bool {
    MANAGED_BUNDLE_TRIGGER_ROOTS
        .iter()
        .any(|relative| worktree.join(relative).exists())
        || git_tracks_managed_assets(worktree)
}

fn git_tracks_managed_assets(worktree: &Path) -> bool {
    match Command::new("git")
        .arg("-C")
        .arg(worktree)
        .arg("ls-files")
        .arg("-z")
        .arg("--")
        .args(MANAGED_BUNDLE_TRIGGER_ROOTS)
        .output()
    {
        Ok(output) => output.status.success() && !output.stdout.is_empty(),
        Err(_) => false,
    }
}

fn refresh_managed_gwt_assets_for_worktree(
    worktree: &Path,
    context: &str,
    materialize_bundle: bool,
) {
    if materialize_bundle {
        if let Err(error) = distribute_to_worktree(worktree) {
            tracing::warn!(
                worktree = %worktree.display(),
                error = %error,
                context,
                "failed to distribute gwt managed assets"
            );
        }
    }
    if let Err(error) = update_git_exclude(worktree) {
        tracing::warn!(
            worktree = %worktree.display(),
            error = %error,
            context,
            "failed to update gwt managed excludes"
        );
    }
    if let Err(error) = generate_settings_local(worktree) {
        tracing::warn!(
            worktree = %worktree.display(),
            error = %error,
            context,
            "failed to regenerate Claude hook settings"
        );
    }
    if let Err(error) = generate_codex_hooks(worktree) {
        tracing::warn!(
            worktree = %worktree.display(),
            error = %error,
            context,
            "failed to regenerate Codex hook settings"
        );
    }
}

fn git_view_divergence_summary(branches: &[gwt_git::Branch]) -> Option<String> {
    let current = branches
        .iter()
        .find(|branch| branch.is_head && branch.is_local)?;
    current.upstream.as_ref()?;

    match (current.ahead, current.behind) {
        (0, 0) => Some("Up to date".to_string()),
        (ahead, 0) => Some(format!("Ahead {ahead}")),
        (0, behind) => Some(format!("Behind {behind}")),
        (ahead, behind) => Some(format!("Ahead {ahead} Behind {behind}")),
    }
}

fn fetch_current_pr_link(repo_path: &std::path::Path) -> gwt_core::Result<Option<String>> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "url"])
        .current_dir(repo_path)
        .output()
        .map_err(|err| gwt_core::GwtError::Git(format!("gh pr view: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.contains("no pull requests found")
            || lowered.contains("no pull request found")
            || lowered.contains("could not resolve to a pull request")
        {
            return Ok(None);
        }
        return Err(gwt_core::GwtError::Git(format!("gh pr view: {trimmed}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_current_pr_link_json(&stdout)
}

fn parse_current_pr_link_json(json: &str) -> gwt_core::Result<Option<String>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| gwt_core::GwtError::Other(format!("gh pr view JSON: {err}")))?;
    Ok(value
        .get("url")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

struct LoadedProfileSettings {
    settings: Settings,
    status: ActiveProfileSummary,
}

fn validation_error(reason: impl Into<String>) -> ConfigError {
    ConfigError::ValidationError {
        reason: reason.into(),
    }
}

fn load_settings_with_active_profile_fallback() -> LoadedProfileSettings {
    match Settings::load() {
        Ok(mut settings) => {
            let configured_active = settings.profiles.active.clone();
            let resolution = settings.profiles.normalize_active_profile();
            if resolution.fallback {
                tracing::warn!(
                    configured_active = ?configured_active,
                    resolved_active = %resolution.name,
                    "active profile missing or invalid; falling back to default"
                );
            }
            LoadedProfileSettings {
                settings,
                status: ActiveProfileSummary {
                    name: resolution.name,
                    fallback: resolution.fallback,
                },
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "failed to load settings; falling back to default profile");
            LoadedProfileSettings {
                settings: Settings::default(),
                status: ActiveProfileSummary {
                    name: "default".to_string(),
                    fallback: true,
                },
            }
        }
    }
}

fn profiles_state_from_settings(
    settings: &Settings,
    previous: Option<&screens::profiles::ProfilesState>,
    preferred_selection: Option<&str>,
) -> screens::profiles::ProfilesState {
    let base_env: BTreeMap<String, String> = std::env::vars().collect();
    let profiles: Vec<screens::profiles::ProfileItem> = settings
        .profiles
        .profiles
        .iter()
        .map(|profile| {
            let mut env_vars: Vec<screens::profiles::EnvVarItem> = profile
                .env_vars
                .iter()
                .map(|(key, value)| screens::profiles::EnvVarItem {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect();
            env_vars.sort_by(|left, right| left.key.cmp(&right.key));
            let mut disabled_env = profile.disabled_env.clone();
            disabled_env.sort();
            let merged_preview = profile
                .merged_env_pairs(std::env::vars())
                .into_iter()
                .map(|(key, value)| screens::profiles::EnvVarItem { key, value })
                .collect();
            let env_rows = profile_env_rows(profile, &base_env);

            screens::profiles::ProfileItem {
                name: profile.name.clone(),
                active: settings.profiles.active.as_deref() == Some(profile.name.as_str()),
                env_count: profile.env_vars.len(),
                description: profile.description.clone(),
                env_vars,
                disabled_env,
                env_rows,
                merged_preview,
                deletable: profile.name != "default",
            }
        })
        .collect();

    let previous_selection = previous
        .and_then(|state| state.selected_profile())
        .map(|profile| profile.name.as_str());
    let selected = preferred_selection
        .and_then(|name| profiles.iter().position(|profile| profile.name == name))
        .or_else(|| {
            previous_selection
                .and_then(|name| profiles.iter().position(|profile| profile.name == name))
        })
        .or_else(|| profiles.iter().position(|profile| profile.active))
        .unwrap_or(0);

    let mut state = screens::profiles::ProfilesState {
        profiles,
        selected,
        focus: previous.map(|state| state.focus).unwrap_or_default(),
        env_selected: previous.map(|state| state.env_selected).unwrap_or_default(),
        disabled_selected: previous
            .map(|state| state.disabled_selected)
            .unwrap_or_default(),
        ..Default::default()
    };
    state.clamp_selection();
    state
}

pub fn refresh_active_profile_state(model: &mut Model) {
    model.active_profile = load_settings_with_active_profile_fallback().status;
}

fn sync_profiles_state_from_settings(model: &mut Model, preferred_selection: Option<&str>) {
    let loaded = load_settings_with_active_profile_fallback();
    model.active_profile = loaded.status;
    model.profiles =
        profiles_state_from_settings(&loaded.settings, Some(&model.profiles), preferred_selection);
}

pub fn spawn_env_with_active_profile(
    mut base_env: HashMap<String, String>,
) -> (HashMap<String, String>, Vec<String>) {
    let loaded = load_settings_with_active_profile_fallback();
    let Some(profile) = loaded.settings.profiles.active_profile() else {
        return (base_env, Vec::new());
    };

    let mut remove_env = Vec::new();
    for key in &profile.disabled_env {
        if !base_env.contains_key(key) && !profile.env_vars.contains_key(key) {
            remove_env.push(key.clone());
        }
    }
    remove_env.sort();
    remove_env.dedup();

    for (key, value) in &profile.env_vars {
        base_env.insert(key.clone(), value.clone());
    }

    (base_env, remove_env)
}

fn apply_profiles_warning(
    model: &mut Model,
    summary: impl Into<String>,
    detail: impl Into<String>,
) {
    apply_notification(
        model,
        Notification::new(Severity::Warn, "profiles", summary.into()).with_detail(detail.into()),
    );
}

fn apply_profiles_info(model: &mut Model, message: impl Into<String>) {
    apply_notification(
        model,
        Notification::new(Severity::Info, "profiles", message.into()),
    );
}

fn current_profile_name(model: &Model) -> Option<String> {
    model
        .profiles
        .selected_profile()
        .map(|profile| profile.name.clone())
}

fn profile_env_rows(
    profile: &gwt_config::Profile,
    base_env: &BTreeMap<String, String>,
) -> Vec<screens::profiles::ProfileEnvRow> {
    let mut keys: BTreeSet<String> = base_env.keys().cloned().collect();
    keys.extend(profile.env_vars.keys().cloned());
    keys.extend(profile.disabled_env.iter().cloned());

    keys.into_iter()
        .map(|key| {
            let (kind, value) = if let Some(value) = profile.env_vars.get(&key) {
                (
                    screens::profiles::ProfileEnvRowKind::Override,
                    Some(value.clone()),
                )
            } else if profile.disabled_env.iter().any(|item| item == &key) {
                (
                    screens::profiles::ProfileEnvRowKind::Disabled,
                    base_env.get(&key).cloned(),
                )
            } else {
                (
                    screens::profiles::ProfileEnvRowKind::Base,
                    base_env.get(&key).cloned(),
                )
            };

            screens::profiles::ProfileEnvRow { key, value, kind }
        })
        .collect()
}

fn refresh_profiles_with_focus(
    model: &mut Model,
    preferred_selection: Option<&str>,
    focus: screens::profiles::ProfilesFocus,
    env_key: Option<&str>,
    disabled_key: Option<&str>,
) {
    sync_profiles_state_from_settings(model, preferred_selection);
    model.profiles.focus = focus;
    if let Some(key) = env_key.or(disabled_key) {
        if let Some(profile) = model.profiles.selected_profile() {
            if let Some(index) = profile.env_rows.iter().position(|row| row.key == key) {
                model.profiles.env_selected = index;
                model.profiles.disabled_selected = index;
            }
        }
    }
    model.profiles.clamp_selection();
}

fn submit_profiles_form(model: &mut Model) {
    use screens::profiles::{ProfileMode, ProfilesFocus};

    match model.profiles.mode {
        ProfileMode::CreateProfile => {
            let name = model.profiles.input_name.trim().to_string();
            let description = model.profiles.input_description.clone();
            match Settings::update_global(|settings| {
                let mut profile = gwt_config::Profile::new(&name);
                profile.description = description.clone();
                settings.profiles.add(profile).map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&name),
                        ProfilesFocus::ProfileList,
                        None,
                        None,
                    );
                    apply_profiles_info(model, format!("Created profile '{name}'"));
                }
                Err(err) => {
                    apply_profiles_warning(model, "Failed to create profile", err.to_string());
                }
            }
        }
        ProfileMode::EditProfile => {
            let Some(current_name) = current_profile_name(model) else {
                return;
            };
            let new_name = model.profiles.input_name.trim().to_string();
            let description = model.profiles.input_description.clone();
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .update_profile(&current_name, &new_name, &description)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&new_name),
                        ProfilesFocus::ProfileList,
                        None,
                        None,
                    );
                    apply_profiles_info(model, format!("Updated profile '{new_name}'"));
                }
                Err(err) => {
                    apply_profiles_warning(model, "Failed to update profile", err.to_string());
                }
            }
        }
        ProfileMode::CreateEnvVar => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            let key = model.profiles.input_key.trim().to_string();
            let value = model.profiles.input_value.clone();
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .set_env_var(&profile_name, &key, &value)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        ProfilesFocus::Environment,
                        Some(&key),
                        None,
                    );
                    apply_profiles_info(
                        model,
                        format!("Saved environment variable '{key}' in '{profile_name}'"),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to save environment variable",
                        err.to_string(),
                    );
                }
            }
        }
        ProfileMode::EditEnvVar => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            let Some(current_key) = model.profiles.selected_env_var().map(|env| env.key.clone())
            else {
                return;
            };
            let new_key = model.profiles.input_key.trim().to_string();
            let new_value = model.profiles.input_value.clone();
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .update_env_var(&profile_name, &current_key, &new_key, &new_value)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        ProfilesFocus::Environment,
                        Some(&new_key),
                        None,
                    );
                    apply_profiles_info(
                        model,
                        format!("Updated environment variable '{new_key}' in '{profile_name}'"),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to update environment variable",
                        err.to_string(),
                    );
                }
            }
        }
        ProfileMode::CreateDisabledEnv => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            let key = model.profiles.input_key.trim().to_string();
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .add_disabled_env(&profile_name, &key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        ProfilesFocus::Environment,
                        None,
                        Some(&key),
                    );
                    apply_profiles_info(
                        model,
                        format!("Blocked OS environment variable '{key}' in '{profile_name}'"),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to block OS environment variable",
                        err.to_string(),
                    );
                }
            }
        }
        ProfileMode::EditDisabledEnv => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            let Some(current_key) = model.profiles.selected_disabled_env().map(str::to_string)
            else {
                return;
            };
            let new_key = model.profiles.input_key.trim().to_string();
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .update_disabled_env(&profile_name, &current_key, &new_key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        ProfilesFocus::Environment,
                        None,
                        Some(&new_key),
                    );
                    apply_profiles_info(
                        model,
                        format!("Updated blocked OS environment variable '{new_key}' in '{profile_name}'"),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to update blocked OS environment variable",
                        err.to_string(),
                    );
                }
            }
        }
        _ => {}
    }
}

fn delete_profiles_selection(model: &mut Model) {
    use screens::profiles::{ProfileMode, ProfilesFocus};

    match model.profiles.mode {
        ProfileMode::ConfirmDeleteProfile => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .delete_profile(&profile_name)
                    .map(|_| ())
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    let preferred = if profile_name == "default" {
                        Some("default")
                    } else {
                        None
                    };
                    refresh_profiles_with_focus(
                        model,
                        preferred,
                        ProfilesFocus::ProfileList,
                        None,
                        None,
                    );
                    apply_profiles_info(model, format!("Deleted profile '{profile_name}'"));
                }
                Err(err) => {
                    apply_profiles_warning(model, "Failed to delete profile", err.to_string());
                    model.profiles.mode = ProfileMode::List;
                }
            }
        }
        ProfileMode::ConfirmDeleteEnvVar => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            let Some(key) = model.profiles.selected_env_var().map(|env| env.key.clone()) else {
                return;
            };
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .remove_env_var(&profile_name, &key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        ProfilesFocus::Environment,
                        None,
                        None,
                    );
                    apply_profiles_info(
                        model,
                        format!("Deleted environment variable '{key}' from '{profile_name}'"),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to delete environment variable",
                        err.to_string(),
                    );
                    model.profiles.mode = ProfileMode::List;
                }
            }
        }
        ProfileMode::ConfirmDeleteDisabledEnv => {
            let Some(profile_name) = current_profile_name(model) else {
                return;
            };
            let Some(key) = model.profiles.selected_disabled_env().map(str::to_string) else {
                return;
            };
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .remove_disabled_env(&profile_name, &key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        ProfilesFocus::Environment,
                        None,
                        None,
                    );
                    apply_profiles_info(
                        model,
                        format!(
                            "Removed blocked OS environment variable '{key}' from '{profile_name}'"
                        ),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to remove blocked OS environment variable",
                        err.to_string(),
                    );
                    model.profiles.mode = ProfileMode::List;
                }
            }
        }
        _ => {}
    }
}

fn delete_profiles_environment_row(model: &mut Model) {
    let Some(profile_name) = current_profile_name(model) else {
        return;
    };
    let Some(row) = model.profiles.selected_env_row().cloned() else {
        return;
    };

    match row.kind {
        screens::profiles::ProfileEnvRowKind::Base => {
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .add_disabled_env(&profile_name, &row.key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        screens::profiles::ProfilesFocus::Environment,
                        None,
                        Some(&row.key),
                    );
                    apply_profiles_info(
                        model,
                        format!(
                            "Disabled OS environment variable '{}' in '{}'",
                            row.key, profile_name
                        ),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to disable OS environment variable",
                        err.to_string(),
                    );
                }
            }
        }
        screens::profiles::ProfileEnvRowKind::Override => {
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .remove_env_var(&profile_name, &row.key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        screens::profiles::ProfilesFocus::Environment,
                        Some(&row.key),
                        None,
                    );
                    apply_profiles_info(
                        model,
                        format!(
                            "Removed profile override '{}' from '{}'",
                            row.key, profile_name
                        ),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to remove profile override",
                        err.to_string(),
                    );
                }
            }
        }
        screens::profiles::ProfileEnvRowKind::Disabled => {
            match Settings::update_global(|settings| {
                settings
                    .profiles
                    .remove_disabled_env(&profile_name, &row.key)
                    .map_err(validation_error)
            }) {
                Ok(()) => {
                    refresh_profiles_with_focus(
                        model,
                        Some(&profile_name),
                        screens::profiles::ProfilesFocus::Environment,
                        Some(&row.key),
                        None,
                    );
                    apply_profiles_info(
                        model,
                        format!(
                            "Restored OS environment variable '{}' in '{}'",
                            row.key, profile_name
                        ),
                    );
                }
                Err(err) => {
                    apply_profiles_warning(
                        model,
                        "Failed to restore OS environment variable",
                        err.to_string(),
                    );
                }
            }
        }
    }
}

fn switch_active_profile_from_profiles_tab(model: &mut Model) {
    let Some(profile_name) = model
        .profiles
        .selected_profile()
        .map(|profile| profile.name.clone())
    else {
        return;
    };

    match Settings::update_global(|settings| {
        settings
            .profiles
            .switch(&profile_name)
            .map_err(validation_error)
    }) {
        Ok(()) => sync_profiles_state_from_settings(model, Some(&profile_name)),
        Err(err) => {
            apply_notification(
                model,
                Notification::new(
                    Severity::Warn,
                    "profiles",
                    "Failed to switch active profile",
                )
                .with_detail(err.to_string()),
            );
            sync_profiles_state_from_settings(model, Some(&profile_name));
        }
    }
}

fn switch_management_tab(model: &mut Model, tab: ManagementTab) {
    switch_management_tab_with(
        model,
        tab,
        gwt_git::fetch_pr_list,
        fetch_pr_dashboard_detail_report,
    );
}

fn switch_management_tab_with<F, D>(
    model: &mut Model,
    tab: ManagementTab,
    fetch_prs: F,
    fetch_detail: D,
) where
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
    D: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    model.management_tab = tab;
    model.active_layer = ActiveLayer::Management;
    model.active_focus = FocusPane::TabContent;
    if tab == ManagementTab::Settings && model.settings.fields.is_empty() {
        model.settings.load_category_fields();
    }
    if tab == ManagementTab::Profiles {
        let preferred_selection = model
            .profiles
            .selected_profile()
            .map(|profile| profile.name.clone());
        sync_profiles_state_from_settings(model, preferred_selection.as_deref());
    }
    if tab == ManagementTab::Issues {
        reload_cached_issues(model);
    }
    if tab == ManagementTab::PrDashboard {
        refresh_pr_dashboard_with(model, fetch_prs, fetch_detail);
    }
}

fn refresh_pr_dashboard_with<F, D>(model: &mut Model, fetch_prs: F, fetch_detail: D)
where
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
    D: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    load_pr_dashboard_with(model, fetch_prs);
    if model.pr_dashboard.detail_view {
        load_pr_dashboard_detail_with(model, fetch_detail);
    }
}

fn load_pr_dashboard_with<F>(model: &mut Model, fetch_prs: F)
where
    F: FnOnce(&std::path::Path) -> gwt_core::Result<Vec<gwt_git::PrStatus>>,
{
    let Ok(prs) = fetch_prs(&model.repo_path) else {
        return;
    };
    let items = prs.into_iter().map(map_pr_item).collect();
    screens::pr_dashboard::update(
        &mut model.pr_dashboard,
        screens::pr_dashboard::PrDashboardMessage::SetPrs(items),
    );
}

fn load_pr_dashboard_detail_with<F>(model: &mut Model, fetch_detail: F)
where
    F: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    let Some(pr) = model.pr_dashboard.selected_pr() else {
        return;
    };

    let Ok(detail) = fetch_detail(&model.repo_path, pr.number) else {
        return;
    };

    screens::pr_dashboard::update(
        &mut model.pr_dashboard,
        screens::pr_dashboard::PrDashboardMessage::SetDetailReport(Some(detail)),
    );
}

fn fetch_pr_dashboard_detail_report(
    repo_path: &std::path::Path,
    number: u32,
) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport> {
    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &number.to_string(),
            "--json",
            "title,state,mergeable,reviewDecision,statusCheckRollup",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|err| gwt_core::GwtError::Git(format!("gh pr view: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(gwt_core::GwtError::Git(format!(
            "gh pr view: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_dashboard_detail_report_json(&stdout)
}

fn parse_pr_dashboard_detail_report_json(
    json: &str,
) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| gwt_core::GwtError::Other(format!("gh pr view JSON: {err}")))?;

    let ci_status = match value.get("statusCheckRollup") {
        Some(serde_json::Value::Array(checks)) if checks.is_empty() => "pending".to_string(),
        Some(serde_json::Value::Array(checks)) => {
            let any_fail = checks.iter().any(|check| {
                check
                    .get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| matches!(s, "FAILURE" | "CANCELLED" | "TIMED_OUT"))
            });
            let all_pass = checks.iter().all(|check| {
                check
                    .get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| matches!(s, "SUCCESS" | "NEUTRAL" | "SKIPPED"))
            });
            if any_fail {
                "failing".to_string()
            } else if all_pass {
                "passing".to_string()
            } else {
                "pending".to_string()
            }
        }
        _ => "unknown".to_string(),
    };

    let merge_status = match value.get("mergeable").and_then(|v| v.as_str()) {
        Some("MERGEABLE") => "ready".to_string(),
        Some("CONFLICTING") => "conflicts".to_string(),
        Some(_) => "blocked".to_string(),
        None => "unknown".to_string(),
    };

    let review_status = match value.get("reviewDecision").and_then(|v| v.as_str()) {
        Some("APPROVED") => "approved".to_string(),
        Some("CHANGES_REQUESTED") => "changes_requested".to_string(),
        Some("REVIEW_REQUIRED") => "pending".to_string(),
        _ => "unknown".to_string(),
    };

    let checks = value
        .get("statusCheckRollup")
        .and_then(|v| v.as_array())
        .map(|checks| {
            checks
                .iter()
                .map(|check| {
                    let name = check
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let conclusion = check
                        .get("conclusion")
                        .and_then(|v| v.as_str())
                        .or_else(|| check.get("status").and_then(|v| v.as_str()))
                        .unwrap_or("UNKNOWN")
                        .to_ascii_lowercase();
                    format!("{name}: {conclusion}")
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(screens::pr_dashboard::PrDetailReport {
        summary: format!("CI {ci_status}, merge {merge_status}, review {review_status}"),
        ci_status,
        merge_status,
        review_status,
        checks,
    })
}

fn map_pr_item(pr: gwt_git::PrStatus) -> screens::pr_dashboard::PrItem {
    let state = match pr.state {
        gwt_git::pr_status::PrState::Open => screens::pr_dashboard::PrState::Open,
        gwt_git::pr_status::PrState::Closed => screens::pr_dashboard::PrState::Closed,
        gwt_git::pr_status::PrState::Merged => screens::pr_dashboard::PrState::Merged,
    };
    let ci_status = pr.ci_status.to_ascii_lowercase();
    let review_status = pr.review_status.to_ascii_lowercase();
    let mergeable = matches!(pr.mergeable.as_str(), "MERGEABLE" | "mergeable");

    screens::pr_dashboard::PrItem {
        number: pr.number as u32,
        title: pr.title,
        state,
        ci_status,
        mergeable,
        review_status,
    }
}

/// Route a key event to the initialization screen.
fn route_key_to_initialization(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::initialization::InitializationMessage;

    let msg = match key.code {
        KeyCode::Esc => Some(Message::Initialization(InitializationMessage::Exit)),
        KeyCode::Enter => Some(Message::Initialization(InitializationMessage::StartClone)),
        KeyCode::Backspace => Some(Message::Initialization(InitializationMessage::Backspace)),
        KeyCode::Char(ch) => Some(Message::Initialization(InitializationMessage::InputChar(
            ch,
        ))),
        _ => None,
    };

    if let Some(m) = msg {
        update(model, m);
    }
}

/// Whether a periodic `Tick` still needs a terminal redraw after state updates.
///
/// This keeps non-terminal surfaces animated while allowing terminal-focused
/// IME composition to proceed without idle repaints.
fn visible_branch_live_indicator_rows(
    model: &Model,
) -> Vec<crate::screens::branches::VisibleBranchLiveIndicatorRow> {
    visible_branches_list_area(model)
        .map(|area| model.branches.visible_live_indicator_rows(area))
        .unwrap_or_default()
}

pub fn visible_branch_live_indicator_signature(model: &Model) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for row in visible_branch_live_indicator_rows(model) {
        row.branch_name.hash(&mut hasher);
        for indicator in row.indicators {
            branch_live_indicator_kind_tag(indicator.kind).hash(&mut hasher);
            branch_live_indicator_status_tag(indicator.status).hash(&mut hasher);
            branch_live_indicator_color_tag(indicator.color).hash(&mut hasher);
            indicator.active.hash(&mut hasher);
        }
    }
    hasher.finish()
}

pub fn should_render_after_tick_with_visible_branch_signature(
    visible_branch_signature_before: u64,
    model: &Model,
) -> bool {
    visible_branch_signature_before != visible_branch_live_indicator_signature(model)
        || tick_redraw_required(model)
}

pub fn tick_redraw_required(model: &Model) -> bool {
    if model.active_focus != FocusPane::Terminal {
        return true;
    }

    if model.active_layer == ActiveLayer::Management
        && model.management_tab == ManagementTab::Branches
        && visible_branches_list_area(model)
            .is_some_and(|area| model.branches.has_running_live_sessions(area))
    {
        return true;
    }

    model
        .wizard
        .as_ref()
        .is_some_and(|wizard| wizard.ai_suggest.loading)
        || model
            .docker_progress
            .as_ref()
            .is_some_and(|progress| progress.visible)
        || model.cleanup_progress.visible
        || model.voice.is_active()
}

fn visible_branches_list_area(model: &Model) -> Option<Rect> {
    if model.active_layer != ActiveLayer::Management
        || model.management_tab != ManagementTab::Branches
    {
        return None;
    }

    let management = visible_management_area(model)?;
    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(management)[0];
    let list_inner = pane_block(management_tab_title(model, top.width), false).inner(top);
    Some(
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(list_inner)[1],
    )
}

fn branch_live_indicator_status_tag(status: gwt_agent::AgentStatus) -> u8 {
    match status {
        gwt_agent::AgentStatus::Unknown => 0,
        gwt_agent::AgentStatus::Running => 1,
        gwt_agent::AgentStatus::WaitingInput => 2,
        gwt_agent::AgentStatus::Stopped => 3,
    }
}

fn branch_live_indicator_kind_tag(
    kind: crate::screens::branches::BranchLiveSessionIndicatorKind,
) -> u8 {
    match kind {
        crate::screens::branches::BranchLiveSessionIndicatorKind::Agent => 0,
        crate::screens::branches::BranchLiveSessionIndicatorKind::Shell => 1,
    }
}

fn branch_live_indicator_color_tag(color: crate::model::AgentColor) -> u8 {
    match color {
        crate::model::AgentColor::Green => 0,
        crate::model::AgentColor::Blue => 1,
        crate::model::AgentColor::Cyan => 2,
        crate::model::AgentColor::Yellow => 3,
        crate::model::AgentColor::Magenta => 4,
        crate::model::AgentColor::Gray => 5,
    }
}

fn route_overlay_key(model: &mut Model, key: crossterm::event::KeyEvent) -> bool {
    if model.help_visible {
        if key.code == KeyCode::Esc {
            update(model, Message::ToggleHelp);
        }
        return true;
    }

    // Wizard overlay takes priority (fullscreen modal)
    if model.wizard.is_some() {
        let msg = match key.code {
            KeyCode::Down => Some(screens::wizard::WizardMessage::MoveDown),
            KeyCode::Up => Some(screens::wizard::WizardMessage::MoveUp),
            KeyCode::Enter => Some(screens::wizard::WizardMessage::Select),
            KeyCode::Esc => Some(screens::wizard::WizardMessage::Back),
            KeyCode::Backspace => Some(screens::wizard::WizardMessage::Backspace),
            KeyCode::Char(ch) => Some(screens::wizard::WizardMessage::InputChar(ch)),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::Wizard(msg));
        }
        return true; // Always consume keys when wizard is open
    }

    // Error overlay
    if !model.error_queue.is_empty() {
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            update(model, Message::DismissError);
        }
        return true;
    }

    if model.help_visible {
        if key.code == KeyCode::Esc {
            update(model, Message::ToggleHelp);
        }
        return true;
    }

    if model.service_select.is_some() {
        let msg = match key.code {
            KeyCode::Down => Some(screens::service_select::ServiceSelectMessage::MoveDown),
            KeyCode::Up => Some(screens::service_select::ServiceSelectMessage::MoveUp),
            KeyCode::Enter => Some(screens::service_select::ServiceSelectMessage::Select),
            KeyCode::Esc => Some(screens::service_select::ServiceSelectMessage::Cancel),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::ServiceSelect(msg));
            return true;
        }
    }
    if model
        .docker_progress
        .as_ref()
        .is_some_and(|progress| progress.visible)
        && key.code == KeyCode::Esc
    {
        update(
            model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::Hide),
        );
        return true;
    }
    if model.confirm.visible {
        let msg = match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                Some(screens::confirm::ConfirmMessage::Toggle)
            }
            KeyCode::Enter => Some(screens::confirm::ConfirmMessage::Accept),
            KeyCode::Esc => Some(screens::confirm::ConfirmMessage::Cancel),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::Confirm(msg));
            return true;
        }
    }

    // Branch Cleanup progress modal — captures all input while running, and
    // accepts only Enter / Esc to dismiss after completion (FR-018g/h).
    if model.cleanup_progress.visible {
        if model.cleanup_progress.is_running() {
            // Hard input block during the run.
            return true;
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            update(
                model,
                Message::CleanupProgress(
                    screens::cleanup_progress::CleanupProgressMessage::Dismiss,
                ),
            );
        }
        return true;
    }

    // Branch Cleanup confirm modal — Enter confirms, Esc cancels (FR-018e).
    if model.cleanup_confirm.visible {
        let msg = match key.code {
            KeyCode::Char('r')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                Some(screens::cleanup_confirm::CleanupConfirmMessage::ToggleRemote)
            }
            KeyCode::Enter => Some(screens::cleanup_confirm::CleanupConfirmMessage::Confirm),
            KeyCode::Esc => Some(screens::cleanup_confirm::CleanupConfirmMessage::Cancel),
            _ => None,
        };
        if let Some(msg) = msg {
            update(model, Message::CleanupConfirm(msg));
            return true;
        }
        // Other keys are swallowed so they don't leak into Branches list.
        return true;
    }

    false
}

/// Route a key event to the branch detail pane (sections, session handoff, launch agent).
fn route_key_to_branch_detail(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::branches::BranchesMessage;

    let msg = match key.code {
        KeyCode::Left => Some(BranchesMessage::PrevDetailSection),
        KeyCode::Right => Some(BranchesMessage::NextDetailSection),
        KeyCode::Char(' ') => {
            toggle_cleanup_selection_for_selected_branch(model);
            return;
        }
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                && model.branches.detail_section != 2
                && selected_branch_has_worktree(model) =>
        {
            Some(BranchesMessage::OpenShell)
        }
        KeyCode::Up if model.branches.detail_section == 0 => Some(BranchesMessage::DockerServiceUp),
        KeyCode::Down if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerServiceDown)
        }
        KeyCode::Up if model.branches.detail_section == 2 => {
            let len = branch_session_matches(model).len();
            if len > 0 {
                model.branches.clamp_detail_session_selected(len);
                model.branches.detail_session_selected =
                    if model.branches.detail_session_selected == 0 {
                        len - 1
                    } else {
                        model.branches.detail_session_selected - 1
                    };
            }
            None
        }
        KeyCode::Down if model.branches.detail_section == 2 => {
            let len = branch_session_matches(model).len();
            if len > 0 {
                model.branches.clamp_detail_session_selected(len);
                model.branches.detail_session_selected =
                    (model.branches.detail_session_selected + 1) % len;
            }
            None
        }
        KeyCode::Enter if model.branches.detail_section == 2 => {
            let sessions = branch_session_matches(model);
            model.branches.clamp_detail_session_selected(sessions.len());
            if let Some(selected) = sessions.get(model.branches.detail_session_selected) {
                model.active_session = selected.session_index;
                model.active_focus = FocusPane::Terminal;
            }
            None
        }
        KeyCode::Enter => Some(BranchesMessage::LaunchAgent),
        KeyCode::Char('s') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerServiceStart)
        }
        KeyCode::Char('t') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerServiceStop)
        }
        KeyCode::Char('r') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerServiceRestart)
        }
        KeyCode::Char('c') if model.branches.detail_section == 0 => {
            Some(BranchesMessage::DockerServiceRecreate)
        }
        KeyCode::Char('m') => {
            screens::branches::update(&mut model.branches, BranchesMessage::ToggleView);
            None
        }
        KeyCode::Char('v') => {
            update(model, Message::SwitchManagementTab(ManagementTab::GitView));
            None
        }
        KeyCode::Char('f') | KeyCode::Char('/') => {
            screens::branches::update(&mut model.branches, BranchesMessage::SearchStart);
            model.active_focus = FocusPane::TabContent;
            None
        }
        KeyCode::Char('?') | KeyCode::Char('h') => {
            update(model, Message::ToggleHelp);
            None
        }
        KeyCode::Esc => {
            model.active_focus = FocusPane::TabContent;
            return;
        }
        _ => None,
    };
    if let Some(m) = msg {
        update(model, Message::Branches(m));
    } else if key.code == KeyCode::Esc {
        dismiss_warn_notification(model);
    }
}

fn toggle_cleanup_selection_for_selected_branch(model: &mut Model) {
    let Some(name) = model
        .branches
        .selected_branch()
        .map(|branch| branch.name.clone())
    else {
        return;
    };
    match model.branches.toggle_cleanup_selection(&name) {
        screens::branches::CleanupSelectionToggle::Selected
        | screens::branches::CleanupSelectionToggle::Deselected => {}
        screens::branches::CleanupSelectionToggle::Blocked(reason) => {
            apply_notification(
                model,
                Notification::new(Severity::Info, "cleanup", reason.toast_message()),
            );
        }
    }
}

/// Route a key event to the active management tab's screen message.
fn route_key_to_management(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::branches::BranchesMessage;
    use screens::git_view::GitViewMessage;
    use screens::issues::IssuesMessage;
    use screens::logs::LogsMessage;
    use screens::profiles::ProfilesMessage;
    use screens::settings::SettingsMessage;
    use screens::versions::VersionsMessage;

    // Left/Right switches tabs when not in text input mode.
    // Ctrl+Left/Right is reserved for sub-tab switching within individual tabs.
    if !is_in_text_input_mode(model) && !key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Right => {
                let next = model.management_tab.next();
                switch_management_tab(model, next);
                return;
            }
            KeyCode::Left => {
                let prev = model.management_tab.prev();
                switch_management_tab(model, prev);
                return;
            }
            _ => {}
        }
    }

    // Tab-specific key routing
    match model.management_tab {
        ManagementTab::Branches => {
            if model.branches.search_active {
                let msg = match key.code {
                    KeyCode::Esc => Some(BranchesMessage::SearchClear),
                    KeyCode::Backspace => Some(BranchesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(BranchesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::branches::update(&mut model.branches, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Some(BranchesMessage::OpenShell)
                }
                KeyCode::Down => Some(BranchesMessage::MoveDown),
                KeyCode::Up => Some(BranchesMessage::MoveUp),
                // Space: toggle Branch Cleanup selection on the focused row
                // (FR-018c). Blocked rows stay focused and surface a
                // short-lived Info toast that explains why cleanup selection
                // is not currently allowed.
                KeyCode::Char(' ') => {
                    toggle_cleanup_selection_for_selected_branch(model);
                    return;
                }
                // Shift+C: open the Cleanup Confirm modal (FR-018e).
                KeyCode::Char('C') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    open_cleanup_confirm_for_selection(model);
                    return;
                }
                // `a`: select every visible cleanable branch (FR-018c).
                KeyCode::Char('a')
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    let visible: Vec<screens::branches::BranchItem> = model
                        .branches
                        .filtered_branches()
                        .into_iter()
                        .cloned()
                        .collect();
                    let refs: Vec<&screens::branches::BranchItem> = visible.iter().collect();
                    model.branches.select_all_visible_cleanable(&refs);
                    return;
                }
                KeyCode::Enter => Some(BranchesMessage::Select),
                KeyCode::Char('s') => Some(BranchesMessage::ToggleSort),
                KeyCode::Char('m') => Some(BranchesMessage::ToggleView),
                KeyCode::Char('v') => {
                    update(model, Message::SwitchManagementTab(ManagementTab::GitView));
                    return;
                }
                KeyCode::Char('f') | KeyCode::Char('/') => Some(BranchesMessage::SearchStart),
                KeyCode::Char('?') | KeyCode::Char('h') => {
                    update(model, Message::ToggleHelp);
                    return;
                }
                KeyCode::Char('r') => {
                    refresh_branches(model);
                    return;
                }
                // FR-018c: Esc clears the cleanup selection when one exists
                // before falling through to the generic Terminal-focus
                // escape. This wires up the `Esc:clear` footer hint.
                KeyCode::Esc if model.branches.cleanup_selection_count() > 0 => {
                    model.branches.clear_selection_after_cleanup();
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                let should_prefetch_detail = matches!(
                    m,
                    BranchesMessage::MoveUp
                        | BranchesMessage::MoveDown
                        | BranchesMessage::ToggleSort
                        | BranchesMessage::ToggleView
                );
                screens::branches::update(&mut model.branches, m);
                if should_prefetch_detail {
                    schedule_branch_detail_prefetch(model);
                }
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
        ManagementTab::Issues => {
            if key.code == KeyCode::Enter
                && key.modifiers.contains(KeyModifiers::SHIFT)
                && model.issues.detail_view
            {
                if let Some(issue) = model.issues.selected_issue() {
                    update(model, Message::OpenWizardWithIssue(issue.number.into()));
                }
                return;
            }

            if model.issues.search_active {
                let msg = match key.code {
                    KeyCode::Esc => Some(IssuesMessage::SearchClear),
                    KeyCode::Backspace => Some(IssuesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(IssuesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    update(model, Message::Issues(m));
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Down => Some(IssuesMessage::MoveDown),
                KeyCode::Up => Some(IssuesMessage::MoveUp),
                KeyCode::Enter => Some(IssuesMessage::ToggleDetail),
                KeyCode::Char('/') => Some(IssuesMessage::SearchStart),
                KeyCode::Char('r') => Some(IssuesMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                update(model, Message::Issues(m));
            } else if key.code == KeyCode::Esc && model.issues.detail_view {
                update(model, Message::Issues(IssuesMessage::ToggleDetail));
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
        ManagementTab::Settings => {
            if model.settings.editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SettingsMessage::EndEdit),
                    KeyCode::Esc => Some(SettingsMessage::CancelEdit),
                    KeyCode::Backspace => Some(SettingsMessage::Backspace),
                    KeyCode::Char(ch) => Some(SettingsMessage::InputChar(ch)),
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                } else if key.code == KeyCode::Esc {
                    fallback_management_escape(model);
                }
            } else {
                let msg = match key.code {
                    KeyCode::Down => Some(SettingsMessage::MoveDown),
                    KeyCode::Up => Some(SettingsMessage::MoveUp),
                    KeyCode::Enter => Some(SettingsMessage::StartEdit),
                    KeyCode::Char(' ') => Some(SettingsMessage::ToggleBool),
                    KeyCode::Char('S') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        Some(SettingsMessage::Save)
                    }
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(SettingsMessage::PrevCategory)
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(SettingsMessage::NextCategory)
                    }
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                } else if key.code == KeyCode::Esc {
                    fallback_management_escape(model);
                }
            }
        }
        ManagementTab::Logs => {
            let msg = match key.code {
                KeyCode::Down => Some(LogsMessage::MoveDown),
                KeyCode::Up => Some(LogsMessage::MoveUp),
                KeyCode::Enter => Some(LogsMessage::ToggleDetail),
                KeyCode::Char('f') => Some(LogsMessage::SetFilter(next_logs_filter_level(
                    model.logs.filter_level,
                ))),
                KeyCode::Char('d') => Some(LogsMessage::SetFilter(toggle_logs_debug_filter(
                    model.logs.filter_level,
                ))),
                KeyCode::Char('r') => Some(LogsMessage::Refresh),
                KeyCode::Char('l') => Some(LogsMessage::CycleLogLevel),
                KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => Some(
                    LogsMessage::SetFilter(next_logs_filter_level(model.logs.filter_level)),
                ),
                KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => Some(
                    LogsMessage::SetFilter(prev_logs_filter_level(model.logs.filter_level)),
                ),
                _ => None,
            };
            if let Some(m) = msg {
                handle_logs_message(model, m);
            } else if key.code == KeyCode::Esc && model.logs.detail_view {
                screens::logs::update(&mut model.logs, LogsMessage::ToggleDetail);
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
        ManagementTab::Versions => {
            let msg = match key.code {
                KeyCode::Down => Some(VersionsMessage::MoveDown),
                KeyCode::Up => Some(VersionsMessage::MoveUp),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::versions::update(&mut model.versions, m);
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
        ManagementTab::GitView => {
            let msg = match key.code {
                KeyCode::Down => Some(GitViewMessage::MoveDown),
                KeyCode::Up => Some(GitViewMessage::MoveUp),
                KeyCode::Enter => Some(GitViewMessage::ToggleExpand),
                KeyCode::Char('r') => {
                    load_initial_data(model);
                    return;
                }
                _ => None,
            };
            if let Some(m) = msg {
                screens::git_view::update(&mut model.git_view, m);
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
        ManagementTab::PrDashboard => {
            if matches!(
                key.code,
                KeyCode::Down | KeyCode::Up | KeyCode::Enter | KeyCode::Char('r')
            ) {
                route_key_to_management_pr_dashboard_with(
                    model,
                    key,
                    fetch_pr_dashboard_detail_report,
                );
            } else if key.code == KeyCode::Esc && model.pr_dashboard.detail_view {
                screens::pr_dashboard::update(
                    &mut model.pr_dashboard,
                    screens::pr_dashboard::PrDashboardMessage::ToggleDetail,
                );
            } else if key.code == KeyCode::Esc {
                fallback_management_escape(model);
            }
        }
        ManagementTab::Profiles => {
            use screens::profiles::{ProfileMode, ProfilesFocus};

            match model.profiles.mode {
                ProfileMode::List => {
                    let msg = match key.code {
                        KeyCode::BackTab => Some(ProfilesMessage::FocusLeft),
                        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                            Some(ProfilesMessage::FocusLeft)
                        }
                        KeyCode::Tab => Some(ProfilesMessage::FocusRight),
                        KeyCode::Down => Some(ProfilesMessage::MoveDown),
                        KeyCode::Up => Some(ProfilesMessage::MoveUp),
                        KeyCode::Enter => match model.profiles.focus {
                            ProfilesFocus::ProfileList => Some(ProfilesMessage::ToggleActive),
                            ProfilesFocus::Environment
                                if model.profiles.selected_env_row().is_some() =>
                            {
                                Some(ProfilesMessage::StartEdit)
                            }
                            _ => None,
                        },
                        KeyCode::Char('n') => Some(ProfilesMessage::StartCreate),
                        KeyCode::Char('e') => Some(ProfilesMessage::StartEdit),
                        KeyCode::Char('d') => None,
                        _ => None,
                    };
                    if key.code == KeyCode::Char('d') {
                        if model.profiles.focus == ProfilesFocus::ProfileList {
                            if model
                                .profiles
                                .selected_profile()
                                .is_some_and(|profile| !profile.deletable)
                            {
                                apply_profiles_warning(
                                    model,
                                    "Default profile cannot be deleted",
                                    "The permanent default profile remains available even when no custom profiles exist.",
                                );
                            } else {
                                screens::profiles::update(
                                    &mut model.profiles,
                                    ProfilesMessage::StartDelete,
                                );
                            }
                        } else if model.profiles.focus == ProfilesFocus::Environment {
                            delete_profiles_environment_row(model);
                        }
                    } else if let Some(m) = msg {
                        if matches!(m, ProfilesMessage::ToggleActive) {
                            switch_active_profile_from_profiles_tab(model);
                        } else {
                            screens::profiles::update(&mut model.profiles, m);
                        }
                    } else if key.code == KeyCode::Esc {
                        fallback_management_escape(model);
                    }
                }
                ProfileMode::CreateProfile
                | ProfileMode::EditProfile
                | ProfileMode::CreateEnvVar
                | ProfileMode::EditEnvVar
                | ProfileMode::CreateDisabledEnv
                | ProfileMode::EditDisabledEnv => {
                    let msg = match key.code {
                        KeyCode::Enter => Some(ProfilesMessage::Confirm),
                        KeyCode::Esc => Some(ProfilesMessage::Cancel),
                        KeyCode::Backspace => Some(ProfilesMessage::Backspace),
                        KeyCode::Tab => Some(ProfilesMessage::NextField),
                        KeyCode::Char(ch) => Some(ProfilesMessage::InputChar(ch)),
                        _ => None,
                    };
                    if let Some(m) = msg {
                        match m {
                            ProfilesMessage::Confirm => submit_profiles_form(model),
                            _ => screens::profiles::update(&mut model.profiles, m),
                        }
                    } else if key.code == KeyCode::Esc {
                        fallback_management_escape(model);
                    }
                }
                ProfileMode::ConfirmDeleteProfile
                | ProfileMode::ConfirmDeleteEnvVar
                | ProfileMode::ConfirmDeleteDisabledEnv => match key.code {
                    KeyCode::Enter => delete_profiles_selection(model),
                    KeyCode::Esc => {
                        screens::profiles::update(&mut model.profiles, ProfilesMessage::Cancel);
                    }
                    _ => {}
                },
            }
        }
        ManagementTab::Specs => {
            // SPEC-12 Phase 9: Specs tab is cache-only. `r` triggers a
            // local cache reload from `~/.gwt/cache/issues/<repo-hash>/`.
            match key.code {
                KeyCode::Char('r') => {
                    model.specs.reload_from_cache();
                }
                KeyCode::Down => {
                    let len = model.specs.items.len();
                    if len > 0 {
                        model.specs.selected = (model.specs.selected + 1) % len;
                    }
                }
                KeyCode::Up => {
                    let len = model.specs.items.len();
                    if len > 0 {
                        model.specs.selected = if model.specs.selected == 0 {
                            len - 1
                        } else {
                            model.specs.selected - 1
                        };
                    }
                }
                KeyCode::Esc => {
                    fallback_management_escape(model);
                }
                _ => {}
            }
        }
    }
}

fn route_key_to_management_pr_dashboard_with<F>(
    model: &mut Model,
    key: crossterm::event::KeyEvent,
    fetch_detail: F,
) where
    F: FnOnce(&std::path::Path, u32) -> gwt_core::Result<screens::pr_dashboard::PrDetailReport>,
{
    use screens::pr_dashboard::PrDashboardMessage;

    let msg = match key.code {
        KeyCode::Down => Some(PrDashboardMessage::MoveDown),
        KeyCode::Up => Some(PrDashboardMessage::MoveUp),
        KeyCode::Enter => Some(PrDashboardMessage::ToggleDetail),
        KeyCode::Char('r') => Some(PrDashboardMessage::Refresh),
        _ => None,
    };

    if let Some(m) = msg {
        let should_open_detail =
            matches!(m, PrDashboardMessage::ToggleDetail) && !model.pr_dashboard.detail_view;
        let should_refresh = matches!(m, PrDashboardMessage::Refresh);
        let should_reload_detail_selection = model.pr_dashboard.detail_view
            && matches!(m, PrDashboardMessage::MoveUp | PrDashboardMessage::MoveDown);
        screens::pr_dashboard::update(&mut model.pr_dashboard, m);
        if (should_open_detail && model.pr_dashboard.detail_view) || should_reload_detail_selection
        {
            load_pr_dashboard_detail_with(model, fetch_detail);
        } else if should_refresh {
            refresh_pr_dashboard_with(model, gwt_git::fetch_pr_list, fetch_detail);
        }
    }
}

/// Check and consume pending branch actions (Wizard launch, shell open).
fn check_branch_pending_actions(model: &mut Model) {
    if model.branches.pending_launch_agent {
        model.branches.pending_launch_agent = false;
        if let Some(branch) = model.branches.selected_branch() {
            let branch_name = branch.name.clone();
            let worktree_path = branch.worktree_path.clone();
            let quick_start_root = worktree_path
                .clone()
                .unwrap_or_else(|| model.repo_path.clone());
            open_wizard(model, None);
            if let Some(mut wizard) = model.wizard.take() {
                wizard.worktree_path = worktree_path;
                configure_existing_branch_wizard_with_sessions(
                    &mut wizard,
                    model,
                    &quick_start_root,
                    &gwt_sessions_dir(),
                    &branch_name,
                );
                model.wizard = Some(wizard);
            }
        }
    }
    if model.branches.pending_open_shell {
        model.branches.pending_open_shell = false;
        if let Some(branch) = model.branches.selected_branch() {
            if let Some(ref wt_path) = branch.worktree_path {
                let idx = model.sessions.len();
                let session = crate::model::SessionTab {
                    id: format!("shell-{idx}"),
                    name: format!("Shell: {}", branch.name),
                    tab_type: crate::model::SessionTabType::Shell,
                    vt: crate::model::VtState::new(24, 80),
                    created_at: std::time::Instant::now(),
                };
                let session_id = session.id.clone();
                model.sessions.push(session);
                model.active_session = idx;
                model.active_focus = FocusPane::Terminal;

                let (cols, rows) = session_content_size(model);
                if let Some(s) = model.sessions.last_mut() {
                    s.vt.resize(rows, cols);
                }

                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                let (env, remove_env) = spawn_env_with_active_profile(HashMap::new());
                let config = gwt_terminal::pty::SpawnConfig {
                    command: shell,
                    args: vec![],
                    cols,
                    rows,
                    env,
                    remove_env,
                    cwd: Some(wt_path.clone()),
                };
                if let Err(e) = spawn_pty_for_session(model, &session_id, config) {
                    apply_notification(
                        model,
                        Notification::new(
                            Severity::Error,
                            "pty",
                            format!("Branch shell spawn failed: {e}"),
                        ),
                    );
                }
            }
        }
    }
}

fn selected_branch_has_worktree(model: &Model) -> bool {
    model
        .branches
        .selected_branch()
        .is_some_and(|branch| branch.worktree_path.is_some())
}

/// Build lightweight summaries of active sessions associated with the selected branch.
fn branch_session_summaries(model: &Model) -> Vec<screens::branches::DetailSessionSummary> {
    branch_session_matches(model)
        .into_iter()
        .map(|entry| entry.summary)
        .collect()
}

#[cfg_attr(not(test), allow(dead_code))]
fn branch_live_session_summaries_with(
    model: &Model,
    sessions_dir: &Path,
) -> HashMap<String, screens::branches::BranchLiveSessionSummary> {
    let mut summaries: HashMap<String, screens::branches::BranchLiveSessionSummary> =
        HashMap::new();
    let active_session_id = model.active_session_tab().map(|session| session.id.clone());

    for session in &model.sessions {
        match &session.tab_type {
            SessionTabType::Agent { agent_id, color } => {
                let path = sessions_dir.join(format!("{}.toml", session.id));
                let Ok(persisted) = AgentSession::load(&path) else {
                    continue;
                };
                let runtime_status =
                    agent_session_runtime_status(sessions_dir, &session.id, &persisted);
                let is_active = active_session_id.as_deref() == Some(session.id.as_str());
                let status = if matches!(
                    runtime_status,
                    gwt_agent::AgentStatus::Running | gwt_agent::AgentStatus::WaitingInput
                ) {
                    runtime_status
                } else if is_active {
                    gwt_agent::AgentStatus::WaitingInput
                } else {
                    continue;
                };

                let candidate = screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status,
                    color: branch_spinner_palette_color(agent_id, *color),
                    active: is_active,
                };
                summaries
                    .entry(persisted.branch.clone())
                    .or_insert_with(|| screens::branches::BranchLiveSessionSummary {
                        indicators: Vec::new(),
                    })
                    .indicators
                    .push(candidate);
            }
            SessionTabType::Shell => {}
        }
    }

    if let Some(active_session) = model.active_session_tab() {
        if matches!(&active_session.tab_type, SessionTabType::Shell) {
            if let Some(branch) = active_session.name.strip_prefix("Shell: ") {
                summaries
                    .entry(branch.to_string())
                    .or_insert_with(|| screens::branches::BranchLiveSessionSummary {
                        indicators: Vec::new(),
                    })
                    .indicators
                    .push(screens::branches::BranchLiveSessionIndicator {
                        kind: screens::branches::BranchLiveSessionIndicatorKind::Shell,
                        status: gwt_agent::AgentStatus::WaitingInput,
                        color: crate::model::AgentColor::Gray,
                        active: true,
                    });
            }
        }
    }

    for summary in summaries.values_mut() {
        summary.indicators.sort_by_key(|indicator| {
            (
                std::cmp::Reverse(u8::from(indicator.active)),
                std::cmp::Reverse(branch_live_session_priority(indicator.status)),
            )
        });
    }

    summaries
}

fn agent_session_runtime_status(
    sessions_dir: &Path,
    session_id: &str,
    persisted: &AgentSession,
) -> gwt_agent::AgentStatus {
    SessionRuntimeState::load(&runtime_state_path(sessions_dir, session_id))
        .map(|runtime| runtime.status)
        .unwrap_or(persisted.status)
}

fn branch_live_session_priority(status: gwt_agent::AgentStatus) -> u8 {
    match status {
        gwt_agent::AgentStatus::Running => 2,
        gwt_agent::AgentStatus::WaitingInput => 1,
        gwt_agent::AgentStatus::Unknown | gwt_agent::AgentStatus::Stopped => 0,
    }
}

fn branch_spinner_palette_color(
    agent_id: &str,
    fallback: crate::model::AgentColor,
) -> crate::model::AgentColor {
    match agent_id {
        "claude" => crate::model::AgentColor::Yellow,
        "codex" => crate::model::AgentColor::Cyan,
        "gemini" => crate::model::AgentColor::Magenta,
        _ => fallback,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn branch_session_summaries_with(
    model: &Model,
    sessions_dir: &Path,
) -> Vec<screens::branches::DetailSessionSummary> {
    branch_session_matches_with(model, sessions_dir)
        .into_iter()
        .map(|entry| entry.summary)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BranchSessionMatch {
    session_index: usize,
    summary: screens::branches::DetailSessionSummary,
}

fn branch_session_matches(model: &Model) -> Vec<BranchSessionMatch> {
    branch_session_matches_with(model, &gwt_sessions_dir())
}

fn branch_session_matches_with(model: &Model, sessions_dir: &Path) -> Vec<BranchSessionMatch> {
    let Some(branch) = model.branches.selected_branch() else {
        return Vec::new();
    };

    branch_session_matches_for(
        model,
        sessions_dir,
        branch.name.as_str(),
        branch.worktree_path.as_deref().unwrap_or(model.repo_path()),
    )
}

fn branch_session_matches_for(
    model: &Model,
    sessions_dir: &Path,
    branch_name: &str,
    branch_worktree: &Path,
) -> Vec<BranchSessionMatch> {
    let branch_shell_name = format!("Shell: {branch_name}");

    model
        .sessions
        .iter()
        .enumerate()
        .filter_map(|(index, session)| match &session.tab_type {
            SessionTabType::Shell if session.name == branch_shell_name => {
                Some(BranchSessionMatch {
                    session_index: index,
                    summary: screens::branches::DetailSessionSummary {
                        kind: "Shell",
                        name: session.name.clone(),
                        detail: None,
                        active: index == model.active_session,
                        launch_summary: Vec::new(),
                        launch_command_line: None,
                    },
                })
            }
            SessionTabType::Agent { .. } => {
                let path = sessions_dir.join(format!("{}.toml", session.id));
                let persisted = AgentSession::load(&path).ok()?;
                if persisted.branch != branch_name || persisted.worktree_path != branch_worktree {
                    return None;
                }

                let detail = match (
                    persisted.model.as_deref(),
                    persisted.reasoning_level.as_deref(),
                ) {
                    (Some(model), Some(reasoning)) => Some(format!("{model} · {reasoning}")),
                    (Some(model), None) => Some(model.to_string()),
                    (None, Some(reasoning)) => Some(reasoning.to_string()),
                    (None, None) => None,
                };

                Some(BranchSessionMatch {
                    session_index: index,
                    summary: screens::branches::DetailSessionSummary {
                        kind: "Agent",
                        name: session.name.clone(),
                        detail,
                        active: index == model.active_session,
                        launch_summary: agent_session_launch_summary(&persisted),
                        launch_command_line: format_launch_command_line(
                            &persisted.launch_command,
                            &persisted.launch_args,
                        ),
                    },
                })
            }
            _ => None,
        })
        .collect()
}

fn branch_live_session_entries_with(
    model: &Model,
    sessions_dir: &Path,
    branch_name: &str,
    branch_worktree: &Path,
) -> Vec<screens::wizard::LiveSessionEntry> {
    branch_session_matches_for(model, sessions_dir, branch_name, branch_worktree)
        .into_iter()
        .map(|entry| screens::wizard::LiveSessionEntry {
            session_id: model.sessions[entry.session_index].id.clone(),
            kind: entry.summary.kind.to_string(),
            name: entry.summary.name,
            detail: entry.summary.detail,
            active: entry.summary.active,
        })
        .collect()
}

fn agent_session_launch_summary(session: &AgentSession) -> Vec<String> {
    let mut summary = Vec::new();
    if let Some(model) = session.model.as_deref() {
        summary.push(format!("Model: {model}"));
    }
    if let Some(reasoning) = session.reasoning_level.as_deref() {
        summary.push(format!("Reasoning: {reasoning}"));
    }
    if let Some(version) = session.tool_version.as_deref() {
        summary.push(format!("Version: {version}"));
    }
    if let Some(resume_session_id) = session.agent_session_id.as_deref() {
        summary.push(format!("Resume session: {resume_session_id}"));
    }
    summary.push(format!(
        "Permissions: {}",
        if session.skip_permissions {
            "Skip confirmations"
        } else {
            "Standard"
        }
    ));
    if session.codex_fast_mode {
        summary.push("Codex fast mode: Enabled".to_string());
    }
    summary.push(format!("Runtime: {:?}", session.runtime_target));
    if let Some(service) = session.docker_service.as_deref() {
        summary.push(format!("Docker service: {service}"));
    }
    summary
}

fn format_launch_command_line(command: &str, args: &[String]) -> Option<String> {
    if command.trim().is_empty() {
        return None;
    }

    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_quote_launch_token(command));
    parts.extend(args.iter().map(|arg| shell_quote_launch_token(arg)));
    Some(parts.join(" "))
}

fn shell_quote_launch_token(token: &str) -> String {
    if !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':' | '='))
    {
        token.to_string()
    } else {
        let mut quoted = String::with_capacity(token.len() + 2);
        quoted.push('\'');
        for ch in token.chars() {
            if ch == '\'' {
                quoted.push_str("'\"'\"'");
            } else {
                quoted.push(ch);
            }
        }
        quoted.push('\'');
        quoted
    }
}

fn search_input_char(key: &crossterm::event::KeyEvent) -> Option<char> {
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }

    match key.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn forward_key_to_active_session(model: &mut Model, key: crossterm::event::KeyEvent) {
    reset_active_session_scrollback_for_input(model);
    let Some(bytes) = key_event_to_bytes(key) else {
        return;
    };
    let Some(session_id) = model.active_session_tab().map(|session| session.id.clone()) else {
        return;
    };
    input_trace::trace_pty_forward(key, &session_id, &bytes);
    model
        .pending_pty_inputs
        .push_back(crate::model::PendingPtyInput { session_id, bytes });
}

fn reset_active_session_scrollback_for_input(model: &mut Model) {
    let Some(session) = model.active_session_tab_mut() else {
        return;
    };
    if session.vt.viewing_history() {
        session.vt.clear_selection();
        session.vt.set_follow_live(true);
    }
}

fn apply_notification(model: &mut Model, notification: Notification) {
    // SPEC-6 Phase 5: when the Logs-tab file watcher is attached
    // (production), the `notification_router::route()` call below
    // emits a `tracing::*!` event that reaches `LogsState` via the
    // file path. Mirroring into `notification_log` + `LogsState`
    // synchronously would cause each notification to appear twice.
    //
    // In tests where no watcher is attached we still populate the
    // in-memory mirror so that assertions on `model.logs.entries`
    // remain valid without having to spawn a real file tail.
    if model.logs_watcher_rx.is_none() {
        model.notification_log.push(notification.clone());
        let entries = notification_log_snapshot(model);
        screens::logs::update(
            &mut model.logs,
            screens::logs::LogsMessage::SetEntries(entries),
        );
    }

    if let Some(msg) = crate::notification_router::route(&notification) {
        update(model, msg);
    }
}

fn mirror_log_event_without_watcher(model: &mut Model, event: Notification) {
    if model.logs_watcher_rx.is_none() {
        screens::logs::update(
            &mut model.logs,
            screens::logs::LogsMessage::AppendEntries(vec![event]),
        );
    }
}

fn emit_log_event(event: &Notification) {
    let detail = event.detail.as_deref().unwrap_or("");
    let action = event
        .fields
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let service = event
        .fields
        .get("service")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let stream = event
        .fields
        .get("stream")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    match event.severity {
        Severity::Debug => {
            tracing::debug!(
                target: "docker",
                detail = %detail,
                action = %action,
                service = %service,
                stream = %stream,
                "{}",
                event.message
            );
        }
        Severity::Info => {
            tracing::info!(
                target: "docker",
                detail = %detail,
                action = %action,
                service = %service,
                stream = %stream,
                "{}",
                event.message
            );
        }
        Severity::Warn => {
            tracing::warn!(
                target: "docker",
                detail = %detail,
                action = %action,
                service = %service,
                stream = %stream,
                "{}",
                event.message
            );
        }
        Severity::Error => {
            tracing::error!(
                target: "docker",
                detail = %detail,
                action = %action,
                service = %service,
                stream = %stream,
                "{}",
                event.message
            );
        }
    }
}

fn record_log_event(model: &mut Model, event: Notification) {
    mirror_log_event_without_watcher(model, event.clone());
    emit_log_event(&event);
}

fn workspace_initialization_warning<E: ToString>(err: E) -> Notification {
    let detail = err.to_string();
    if gwt_core::runtime::project_index_runtime_error_kind(&detail).is_some() {
        return Notification::new(Severity::Warn, "index", "Project index runtime unavailable")
            .with_detail(gwt_core::runtime::project_index_runtime_error_detail(
                &detail,
            ));
    }

    Notification::new(
        Severity::Warn,
        "workspace",
        "Workspace initialization incomplete",
    )
    .with_detail(detail)
}

fn notification_log_snapshot(model: &Model) -> Vec<screens::logs::LogEntry> {
    model.notification_log.entries().to_vec()
}

fn tick_notification(model: &mut Model) {
    let Some(ttl) = model.current_notification_ttl else {
        return;
    };

    let step = Duration::from_millis(100);
    if ttl <= step {
        model.current_notification = None;
        model.current_notification_ttl = None;
    } else {
        model.current_notification_ttl = Some(ttl - step);
    }
}

fn dismiss_warn_notification(model: &mut Model) {
    if matches!(
        model
            .current_notification
            .as_ref()
            .map(|notification| notification.severity),
        Some(Severity::Warn)
    ) {
        update(model, Message::DismissNotification);
    }
}

fn fallback_management_escape(model: &mut Model) {
    if matches!(
        model
            .current_notification
            .as_ref()
            .map(|notification| notification.severity),
        Some(Severity::Warn)
    ) {
        update(model, Message::DismissNotification);
    } else {
        model.active_focus = FocusPane::Terminal;
    }
}

fn next_management_focus(model: &Model, reverse: bool) -> FocusPane {
    if model.management_tab == ManagementTab::Branches {
        return if reverse {
            model.active_focus.prev()
        } else {
            model.active_focus.next()
        };
    }

    match (model.active_focus, reverse) {
        (FocusPane::Terminal, false) => FocusPane::TabContent,
        (FocusPane::TabContent, false) => FocusPane::Terminal,
        (FocusPane::BranchDetail, false) => FocusPane::Terminal,
        (FocusPane::Terminal, true) => FocusPane::TabContent,
        (FocusPane::TabContent, true) => FocusPane::Terminal,
        (FocusPane::BranchDetail, true) => FocusPane::TabContent,
    }
}

fn cycle_focus_with_shortcut(model: &mut Model, reverse: bool) {
    match model.active_layer {
        ActiveLayer::Initialization => {}
        ActiveLayer::Main => {
            model.active_layer = ActiveLayer::Management;
            model.active_focus = next_management_focus(model, reverse);
            sync_session_viewports(model);
        }
        ActiveLayer::Management => {
            model.active_focus = next_management_focus(model, reverse);
        }
    }
}

fn handle_pending_branch_docker_action(
    model: &mut Model,
    action: screens::branches::PendingDockerAction,
) {
    if model.docker_progress_events.is_some() {
        update(
            model,
            Message::ShowNotification(Notification::new(
                Severity::Warn,
                "docker",
                "Docker action already running",
            )),
        );
        return;
    }

    let service_label = action.service.clone();

    emit_branch_docker_progress(
        model,
        screens::docker_progress::DockerStage::StartingContainer,
        format!(
            "{} service {service_label}",
            start_message_for_action(action.action)
        ),
    );

    let events = Arc::new(Mutex::new(VecDeque::new()));
    model.docker_progress_events = Some(events.clone());
    spawn_docker_progress_worker(events, action, service_label);
}

fn emit_branch_docker_progress(
    model: &mut Model,
    stage: screens::docker_progress::DockerStage,
    message: String,
) {
    update(
        model,
        Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetStage {
            stage,
            message,
        }),
    );
}

fn push_docker_progress_event(events: &DockerProgressQueue, event: DockerProgressEvent) {
    if let Ok(mut queue) = events.lock() {
        queue.push_back(event);
    }
}

fn spawn_docker_progress_worker(
    events: DockerProgressQueue,
    action: screens::branches::PendingDockerAction,
    service_label: String,
) {
    use screens::branches::DockerLifecycleAction;

    thread::spawn(move || {
        let outcome = match action.action {
            DockerLifecycleAction::Start => {
                gwt_docker::compose_up(&action.compose_file, &action.service).map(|()| {
                    DockerProgressEvent::BranchCompleted {
                        message: format!("Started service {service_label}"),
                    }
                })
            }
            DockerLifecycleAction::Stop => {
                gwt_docker::compose_stop(&action.compose_file, &action.service).map(|()| {
                    DockerProgressEvent::BranchCompleted {
                        message: format!("Stopped service {service_label}"),
                    }
                })
            }
            DockerLifecycleAction::Restart => {
                gwt_docker::compose_restart(&action.compose_file, &action.service).map(|()| {
                    DockerProgressEvent::BranchCompleted {
                        message: format!("Restarted service {service_label}"),
                    }
                })
            }
            DockerLifecycleAction::Recreate => {
                gwt_docker::compose_up_force_recreate(&action.compose_file, &action.service).map(
                    |()| DockerProgressEvent::BranchCompleted {
                        message: format!("Recreated service {service_label}"),
                    },
                )
            }
        };

        let event = match outcome {
            Ok(result) => result,
            Err(err) => DockerProgressEvent::BranchFailed {
                message: format!(
                    "Failed to {} service {service_label}",
                    verb_for_action(action.action)
                ),
                detail: err.to_string(),
            },
        };

        push_docker_progress_event(&events, event);
    });
}

fn drain_docker_progress_events(model: &mut Model) {
    let Some(events) = model.docker_progress_events.as_ref().cloned() else {
        return;
    };

    let event = events.lock().ok().and_then(|mut queue| queue.pop_front());
    let Some(event) = event else {
        return;
    };

    match event {
        DockerProgressEvent::Stage { stage, message } => {
            emit_branch_docker_progress(model, stage, message);
        }
        DockerProgressEvent::Log { entry } => {
            record_log_event(model, entry);
        }
        DockerProgressEvent::BranchCompleted { message } => {
            model.docker_progress_events = None;
            emit_branch_docker_progress(
                model,
                screens::docker_progress::DockerStage::Ready,
                message.clone(),
            );
            schedule_branch_detail_prefetch(model);
            update(
                model,
                Message::Notify(Notification::new(Severity::Info, "docker", message)),
            );
        }
        DockerProgressEvent::BranchFailed { message, detail } => {
            model.docker_progress_events = None;
            update(
                model,
                Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetError(
                    format!("{message}: {detail}"),
                )),
            );
            update(
                model,
                Message::Notify(
                    Notification::new(Severity::Error, "docker", message).with_detail(detail),
                ),
            );
        }
        DockerProgressEvent::LaunchReady { config } => {
            model.docker_progress_events = None;
            let result = persist_and_spawn_launch(model, &gwt_sessions_dir(), *config);
            update(
                model,
                Message::DockerProgress(screens::docker_progress::DockerProgressMessage::Hide),
            );
            if let Err(err) = result {
                update(
                    model,
                    Message::PushErrorNotification(
                        Notification::new(Severity::Error, "session", "Agent launch failed")
                            .with_detail(err),
                    ),
                );
            }
        }
        DockerProgressEvent::LaunchFailed { message, detail } => {
            model.docker_progress_events = None;
            update(
                model,
                Message::DockerProgress(screens::docker_progress::DockerProgressMessage::Hide),
            );
            update(
                model,
                Message::PushErrorNotification(
                    Notification::new(Severity::Error, "docker", message).with_detail(detail),
                ),
            );
        }
    }
}

fn refresh_branches(model: &mut Model) {
    if let Ok(branches) = gwt_git::branch::list_branches(&model.repo_path) {
        let items: Vec<screens::branches::BranchItem> = branches
            .iter()
            .map(|branch| screens::branches::BranchItem {
                name: branch.name.clone(),
                is_head: branch.is_head,
                is_local: branch.is_local,
                category: screens::branches::categorize_branch(&branch.name),
                worktree_path: None,
                upstream: branch.upstream.clone(),
            })
            .collect();
        screens::branches::update(
            &mut model.branches,
            screens::branches::BranchesMessage::SetBranches(items),
        );
    }

    let mut checked_out: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(worktrees) = gwt_git::WorktreeManager::new(&model.repo_path).list() {
        for wt in &worktrees {
            if let Some(ref branch_name) = wt.branch {
                checked_out.insert(branch_name.clone());
                if let Some(item) = model
                    .branches
                    .branches
                    .iter_mut()
                    .find(|branch| &branch.name == branch_name)
                {
                    item.worktree_path = Some(wt.path.clone());
                }
            }
        }
    }
    model.branches.checked_out_branches = checked_out;
    model.branches.current_head_branch = model
        .branches
        .branches
        .iter()
        .find(|b| b.is_head)
        .map(|b| b.name.clone());
    if let Some(head_branch_name) = model.branches.current_head_branch.clone() {
        if let Some(item) = model
            .branches
            .branches
            .iter_mut()
            .find(|branch| branch.name == head_branch_name && branch.worktree_path.is_none())
        {
            item.worktree_path = Some(model.repo_path.clone());
        }
    }
    refresh_active_session_branches(model);
    refresh_cleanup_merge_state(model);

    let synced_branches = model.branches.branches.clone();
    screens::branches::update(
        &mut model.branches,
        screens::branches::BranchesMessage::SetBranches(synced_branches),
    );
    schedule_branch_detail_prefetch(model);
}

/// Spawn the background merge-state worker for every local branch
/// (FR-018a/d). The model immediately resets `merged_state` so the list
/// renders the `⋯` spinner glyph until the worker pushes results into the
/// queue drained by the tick loop.
fn refresh_cleanup_merge_state(model: &mut Model) {
    use screens::branches::MergeState;
    use std::sync::atomic::AtomicBool;

    let local_names: Vec<String> = model
        .branches
        .branches
        .iter()
        .filter(|b| b.is_local)
        .map(|b| b.name.clone())
        .collect();

    // Reset every local branch to Computing so the gutter shows the spinner
    // until the worker reports the new value.
    for name in &local_names {
        model
            .branches
            .set_merge_state(name.clone(), MergeState::Computing);
    }

    if local_names.is_empty() {
        model.merge_state_events = None;
        return;
    }

    let queue: crate::model::MergeStateQueue =
        Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let finished = Arc::new(AtomicBool::new(false));
    model.merge_state_events = Some(crate::model::MergeStateChannel {
        queue: queue.clone(),
        finished: finished.clone(),
    });
    let repo_path = model.repo_path.clone();

    std::thread::spawn(move || {
        let bases = [
            ("origin/main", gwt_git::MergeTarget::Main),
            ("origin/develop", gwt_git::MergeTarget::Develop),
        ];
        let gone = gwt_git::list_gone_branches(&repo_path).unwrap_or_default();
        for branch in local_names {
            let target = gwt_git::detect_cleanable_target(&repo_path, &branch, &bases, &gone)
                .unwrap_or(None);
            let state = match target {
                Some(t) => MergeState::Cleanable(t),
                None => MergeState::NotMerged,
            };
            queue
                .lock()
                .unwrap()
                .push_back(crate::model::MergeStateEvent { branch, state });
        }
        // Mark the worker finished AFTER the last event is enqueued so the
        // drain helper cannot race the loop.
        finished.store(true, std::sync::atomic::Ordering::Release);
    });
}

/// Maximum number of merge-state events drained per tick. Capping the
/// drain rate keeps the `⋯` spinner glyph on screen long enough for the
/// user to actually see it on small repositories where the worker would
/// otherwise complete in a single frame (FR-018d).
const MERGE_STATE_DRAIN_PER_TICK: usize = 2;

/// Drain pending merge-state events into `BranchesState::merged_state`,
/// at most [`MERGE_STATE_DRAIN_PER_TICK`] events per call. The shared
/// channel handle is dropped only after the worker has explicitly
/// signalled completion AND the queue is empty, so a momentarily empty
/// queue between two single-event pushes cannot tear the worker handle
/// down prematurely.
fn drain_merge_state_events(model: &mut Model) {
    use std::sync::atomic::Ordering;

    let Some(channel) = model.merge_state_events.clone() else {
        return;
    };
    let events: Vec<crate::model::MergeStateEvent> = {
        let mut guard = channel.queue.lock().unwrap();
        let take = MERGE_STATE_DRAIN_PER_TICK.min(guard.len());
        guard.drain(..take).collect()
    };
    for event in events {
        model.branches.set_merge_state(event.branch, event.state);
    }

    // Tear the channel down only when the worker is finished AND the queue
    // has been fully drained. Without the explicit `finished` flag the
    // queue can be empty between two single-event pushes, which would
    // strand the remaining branches in `Computing` forever.
    let worker_done = channel.finished.load(Ordering::Acquire);
    if worker_done {
        let queue_empty = channel.queue.lock().unwrap().is_empty();
        if queue_empty {
            model.merge_state_events = None;
        }
    }
}

/// Refresh the set of branches that have at least one running session pane
/// bound to them. Used by the Branch Cleanup protection guards (FR-018b).
///
/// For agent sessions the branch name is read from the persisted
/// [`gwt_agent::AgentSession`] metadata rather than guessed from the tab
/// title, because launched agent tabs are created with the agent's display
/// name and do not carry the branch name in `SessionTab::name`. Callers
/// should invoke this any time `model.sessions` changes so the guard cannot
/// go stale between branch reloads.
fn refresh_active_session_branches(model: &mut Model) {
    refresh_active_session_branches_with(model, &gwt_sessions_dir());
}

fn refresh_active_session_branches_with(model: &mut Model, sessions_dir: &Path) {
    use std::collections::HashSet;

    let mut active: HashSet<String> = HashSet::new();
    for session in &model.sessions {
        match &session.tab_type {
            SessionTabType::Shell => {
                if let Some(branch) = session.name.strip_prefix("Shell: ") {
                    active.insert(branch.to_string());
                }
            }
            SessionTabType::Agent { .. } => {
                let path = sessions_dir.join(format!("{}.toml", session.id));
                if let Ok(persisted) = AgentSession::load(&path) {
                    if !matches!(
                        agent_session_runtime_status(sessions_dir, &session.id, &persisted),
                        gwt_agent::AgentStatus::Stopped
                    ) {
                        active.insert(persisted.branch.clone());
                    }
                } else if let Some((_, branch)) = session.name.split_once(": ") {
                    // Fall back to the tab title only when no persisted
                    // metadata exists (e.g., freshly spawned session before
                    // the sidecar write lands).
                    active.insert(branch.to_string());
                }
            }
        }
    }
    model.branches.active_session_branches = active;
}

fn schedule_branch_detail_prefetch(model: &mut Model) {
    let (generation, branches) = model.branches.begin_detail_refresh();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = spawn_branch_detail_worker(
        events.clone(),
        cancel.clone(),
        generation,
        branches,
        branch_detail_docker_snapshotter(model),
    );

    if let Some(worker) = model.branch_detail_worker.as_mut() {
        worker.replace(events, cancel, handle);
    } else {
        model.branch_detail_worker = Some(crate::model::BranchDetailWorker::new(
            events, cancel, handle,
        ));
    }
}

fn spawn_branch_detail_worker(
    events: BranchDetailQueue,
    cancel: Arc<AtomicBool>,
    generation: u64,
    branches: Vec<screens::branches::BranchItem>,
    docker_snapshotter: Arc<dyn Fn() -> Vec<screens::branches::DockerServiceInfo> + Send + Sync>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let docker_services = docker_snapshotter();
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        run_branch_detail_worker(
            events,
            cancel,
            generation,
            branches,
            docker_services,
            screens::branches::load_branch_detail,
        );
    })
}

fn branch_detail_docker_snapshotter(
    model: &Model,
) -> Arc<dyn Fn() -> Vec<screens::branches::DockerServiceInfo> + Send + Sync> {
    #[cfg(test)]
    if let Some(snapshotter) = model.branch_detail_docker_snapshotter.as_ref() {
        return snapshotter.clone();
    }

    let branches = model.branches.branches.clone();
    Arc::new(move || snapshot_branch_detail_docker_services(&branches))
}

fn snapshot_branch_detail_docker_services(
    branches: &[screens::branches::BranchItem],
) -> Vec<screens::branches::DockerServiceInfo> {
    let mut services = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for branch in branches {
        let Some(project_root) = branch.worktree_path.as_ref() else {
            continue;
        };
        let files = gwt_docker::detect_docker_files(project_root);
        let Some(compose_file) = docker_compose_file_for_launch(project_root, &files)
            .ok()
            .flatten()
        else {
            continue;
        };
        let Ok(parsed_services) = gwt_docker::parse_compose_file(&compose_file) else {
            continue;
        };

        for service in parsed_services {
            let service_name = service.name;
            if !seen.insert((project_root.clone(), service_name.clone())) {
                continue;
            }
            let status = gwt_docker::compose_service_status(&compose_file, &service_name)
                .unwrap_or(gwt_docker::ComposeServiceStatus::NotFound);
            services.push(screens::branches::DockerServiceInfo {
                project_root: project_root.clone(),
                compose_file: compose_file.clone(),
                name: service_name,
                status,
                ports: service.ports.join(", "),
            });
        }
    }

    services
}

#[cfg(test)]
fn spawn_branch_detail_worker_with_loader<F>(
    events: BranchDetailQueue,
    cancel: Arc<AtomicBool>,
    generation: u64,
    branches: Vec<screens::branches::BranchItem>,
    docker_services: Vec<screens::branches::DockerServiceInfo>,
    loader: F,
) -> thread::JoinHandle<()>
where
    F: Fn(
            &screens::branches::BranchItem,
            &[screens::branches::DockerServiceInfo],
        ) -> screens::branches::BranchDetailData
        + Send
        + 'static,
{
    thread::spawn(move || {
        run_branch_detail_worker(
            events,
            cancel,
            generation,
            branches,
            docker_services,
            loader,
        );
    })
}

fn run_branch_detail_worker<F>(
    events: BranchDetailQueue,
    cancel: Arc<AtomicBool>,
    generation: u64,
    branches: Vec<screens::branches::BranchItem>,
    docker_services: Vec<screens::branches::DockerServiceInfo>,
    loader: F,
) where
    F: Fn(
        &screens::branches::BranchItem,
        &[screens::branches::DockerServiceInfo],
    ) -> screens::branches::BranchDetailData,
{
    for branch in branches {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let data = loader(&branch, &docker_services);
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(mut queue) = events.lock() {
            queue.push_back(screens::branches::BranchDetailLoadResult {
                generation,
                branch_name: branch.name.clone(),
                data,
            });
        }
    }
}

fn drain_branch_detail_events(model: &mut Model) {
    let Some(worker) = model.branch_detail_worker.as_mut() else {
        return;
    };
    worker.reap_finished();
    let events = worker.events();

    for _ in 0..BRANCH_DETAIL_EVENTS_PER_TICK_BUDGET {
        let event = events.lock().ok().and_then(|mut queue| queue.pop_front());
        let Some(event) = event else {
            return;
        };

        if event.generation != model.branches.detail_generation {
            continue;
        }
        if !model.branches.knows_branch(&event.branch_name) {
            continue;
        }

        model.branches.cache_detail(event.branch_name, event.data);
    }
}

fn start_message_for_action(action: screens::branches::DockerLifecycleAction) -> &'static str {
    use screens::branches::DockerLifecycleAction;

    match action {
        DockerLifecycleAction::Start => "Starting",
        DockerLifecycleAction::Stop => "Stopping",
        DockerLifecycleAction::Restart => "Restarting",
        DockerLifecycleAction::Recreate => "Recreating",
    }
}

fn verb_for_action(action: screens::branches::DockerLifecycleAction) -> &'static str {
    use screens::branches::DockerLifecycleAction;

    match action {
        DockerLifecycleAction::Start => "start",
        DockerLifecycleAction::Stop => "stop",
        DockerLifecycleAction::Restart => "restart",
        DockerLifecycleAction::Recreate => "recreate",
    }
}

fn next_logs_filter_level(level: screens::logs::FilterLevel) -> screens::logs::FilterLevel {
    level.next()
}

fn prev_logs_filter_level(level: screens::logs::FilterLevel) -> screens::logs::FilterLevel {
    level.prev()
}

fn toggle_logs_debug_filter(level: screens::logs::FilterLevel) -> screens::logs::FilterLevel {
    use screens::logs::FilterLevel;
    if level == FilterLevel::DebugUp {
        FilterLevel::All
    } else {
        FilterLevel::DebugUp
    }
}

fn drain_notification_bus(model: &mut Model) {
    for notification in model.drain_notifications() {
        update(model, Message::Notify(notification));
    }
}

/// Apply a `LogsMessage`, intercepting `CycleLogLevel` so the
/// `tracing_subscriber::reload::Handle` is invoked alongside the
/// state update (SPEC-6 FR-011).
fn handle_logs_message(model: &mut Model, msg: screens::logs::LogsMessage) {
    if matches!(msg, screens::logs::LogsMessage::CycleLogLevel) {
        let next = screens::logs::next_log_level(model.logs.current_log_level);
        match model.apply_log_level(next) {
            Ok(()) => {
                tracing::info!(
                    target: "gwt_tui::logging",
                    from = %model.logs.current_log_level,
                    to = %next,
                    "log level changed"
                );
                screens::logs::update(
                    &mut model.logs,
                    screens::logs::LogsMessage::SetLogLevel(next),
                );
            }
            Err(err) => {
                tracing::warn!(
                    target: "gwt_tui::logging",
                    error = %err,
                    "log level change failed"
                );
            }
        }
        return;
    }
    screens::logs::update(&mut model.logs, msg);
}

/// Drain the UI log bridge channel and dispatch user-facing events
/// as toast / error modal messages.
///
/// **Filter policy (reviewer comment B3):** the bridge ONLY forwards
/// events whose `target` starts with `gwt_tui::ui::` — a dedicated
/// namespace reserved for "this is intended for a user-visible
/// notification surface". Internal traces (`gwt_tui::main`,
/// `gwt_tui::agent::launch`, `gwt_tui::index`, etc.) are persisted to
/// the file but are NOT pushed as toasts. This prevents the bridge
/// from spamming the status bar with internal info logs and from
/// double-firing notifications that the legacy `apply_notification`
/// path already enqueues.
///
/// To surface a warn/error from any crate as a toast/modal **without**
/// going through `apply_notification`, emit a tracing event with
/// `target: "gwt_tui::ui::<area>"`.
fn drain_ui_log_events(model: &mut Model) {
    let Some(rx) = model.ui_log_rx.as_ref() else {
        return;
    };
    let mut pending = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if !event.source.starts_with("gwt_tui::ui::") {
            continue;
        }
        pending.push(event);
    }
    for event in pending {
        match event.severity {
            gwt_core::logging::LogLevel::Error => {
                update(model, Message::PushErrorNotification(event));
            }
            gwt_core::logging::LogLevel::Warn | gwt_core::logging::LogLevel::Info => {
                update(model, Message::ShowNotification(event));
            }
            gwt_core::logging::LogLevel::Debug => {}
        }
    }
}

fn push_input_to_active_session(model: &mut Model, bytes: Vec<u8>) {
    let Some(session_id) = model.active_session_tab().map(|session| session.id.clone()) else {
        return;
    };

    model
        .pending_pty_inputs
        .push_back(crate::model::PendingPtyInput { session_id, bytes });
}

fn handle_voice_message(model: &mut Model, msg: VoiceInputMessage, voice_enabled: bool) {
    if matches!(msg, VoiceInputMessage::StartRecording) && !voice_enabled {
        return;
    }

    let transcription = match &msg {
        VoiceInputMessage::TranscriptionResult(text) => Some(text.clone()),
        _ => None,
    };
    crate::input::voice::update(&mut model.voice, msg);
    if let Some(text) = transcription.filter(|text| !text.trim().is_empty()) {
        push_input_to_active_session(model, text.into_bytes());
    }
}

fn route_paste_input(model: &mut Model, text: String) {
    if model.help_visible
        || !model.error_queue.is_empty()
        || model.service_select.is_some()
        || model.confirm.visible
        || model.cleanup_confirm.visible
        || model.cleanup_progress.visible
        || model
            .docker_progress
            .as_ref()
            .is_some_and(|progress| progress.visible)
    {
        return;
    }

    if route_non_terminal_paste(model, &text) {
        return;
    }

    match model.active_layer {
        ActiveLayer::Initialization => {}
        ActiveLayer::Management => {
            if matches!(model.active_focus, FocusPane::Terminal) {
                handle_paste_input(model, text);
            }
        }
        _ => handle_paste_input(model, text),
    }
}

fn route_non_terminal_paste(model: &mut Model, text: &str) -> bool {
    if let Some(wizard) = model.wizard.as_mut() {
        paste_text_input_chars(text, |ch| {
            screens::wizard::update(wizard, screens::wizard::WizardMessage::InputChar(ch));
        });
        return true;
    }

    match model.active_layer {
        ActiveLayer::Initialization => {
            if let Some(state) = model.initialization.as_mut() {
                paste_text_input_chars(text, |ch| {
                    screens::initialization::update(
                        state,
                        screens::initialization::InitializationMessage::InputChar(ch),
                    );
                });
                return true;
            }
            false
        }
        ActiveLayer::Management if !matches!(model.active_focus, FocusPane::Terminal) => {
            match model.management_tab {
                ManagementTab::Branches if model.branches.search_active => {
                    paste_text_input_chars(text, |ch| {
                        screens::branches::update(
                            &mut model.branches,
                            screens::branches::BranchesMessage::SearchInput(ch),
                        );
                    });
                    true
                }
                ManagementTab::Issues if model.issues.search_active => {
                    paste_text_input_chars(text, |ch| {
                        screens::issues::update(
                            &mut model.issues,
                            screens::issues::IssuesMessage::SearchInput(ch),
                        );
                    });
                    true
                }
                ManagementTab::Settings if model.settings.editing => {
                    paste_text_input_chars(text, |ch| {
                        screens::settings::update(
                            &mut model.settings,
                            screens::settings::SettingsMessage::InputChar(ch),
                        );
                    });
                    true
                }
                ManagementTab::Profiles
                    if model.profiles.mode != screens::profiles::ProfileMode::List =>
                {
                    paste_text_input_chars(text, |ch| {
                        screens::profiles::update(
                            &mut model.profiles,
                            screens::profiles::ProfilesMessage::InputChar(ch),
                        );
                    });
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn paste_text_input_chars(text: &str, mut push_char: impl FnMut(char)) {
    for ch in text.chars() {
        if matches!(ch, '\r' | '\n') {
            continue;
        }
        push_char(ch);
    }
}

trait VoiceRuntime {
    fn configure(&mut self, config: &VoiceConfig);
    fn start_recording(&mut self) -> Result<(), String>;
    fn stop_and_transcribe(&mut self) -> Result<String, String>;
    fn reset(&mut self);
}

impl VoiceRuntime for crate::model::VoiceRuntimeState {
    fn configure(&mut self, config: &VoiceConfig) {
        crate::model::VoiceRuntimeState::configure(self, config);
    }

    fn start_recording(&mut self) -> Result<(), String> {
        crate::model::VoiceRuntimeState::start_recording(self)
    }

    fn stop_and_transcribe(&mut self) -> Result<String, String> {
        crate::model::VoiceRuntimeState::stop_and_transcribe(self)
    }

    fn reset(&mut self) {
        crate::model::VoiceRuntimeState::reset(self);
    }
}

#[cfg(test)]
fn handle_voice_message_with_runtime<R>(
    model: &mut Model,
    msg: VoiceInputMessage,
    voice_enabled: bool,
    runtime: &mut R,
) where
    R: VoiceRuntime,
{
    let voice_config = VoiceConfig {
        enabled: voice_enabled,
        ..VoiceConfig::default()
    };
    handle_voice_message_with_config_and_runtime(model, msg, &voice_config, runtime);
}

fn handle_voice_message_with_config_and_runtime<R>(
    model: &mut Model,
    msg: VoiceInputMessage,
    voice_config: &VoiceConfig,
    runtime: &mut R,
) where
    R: VoiceRuntime,
{
    runtime.configure(voice_config);

    match msg {
        VoiceInputMessage::StartRecording if !voice_config.enabled => {}
        VoiceInputMessage::StartRecording
            if model.voice.status == crate::input::voice::VoiceStatus::Recording =>
        {
            complete_voice_transcription(model, runtime);
        }
        VoiceInputMessage::StartRecording => match runtime.start_recording() {
            Ok(()) => {
                crate::input::voice::update(&mut model.voice, VoiceInputMessage::StartRecording)
            }
            Err(err) => {
                runtime.reset();
                crate::input::voice::update(
                    &mut model.voice,
                    VoiceInputMessage::TranscriptionError(err),
                );
            }
        },
        VoiceInputMessage::StopRecording => {
            if model.voice.status == crate::input::voice::VoiceStatus::Recording {
                complete_voice_transcription(model, runtime);
            } else {
                runtime.reset();
                crate::input::voice::update(
                    &mut model.voice,
                    VoiceInputMessage::TranscriptionError("Not currently recording".into()),
                );
            }
        }
        other => handle_voice_message(model, other, voice_config.enabled),
    }
}

fn complete_voice_transcription<R>(model: &mut Model, runtime: &mut R)
where
    R: VoiceRuntime,
{
    crate::input::voice::update(&mut model.voice, VoiceInputMessage::StopRecording);
    match runtime.stop_and_transcribe() {
        Ok(text) => {
            crate::input::voice::update(
                &mut model.voice,
                VoiceInputMessage::TranscriptionResult(text.clone()),
            );
            if !text.trim().is_empty() {
                push_input_to_active_session(model, text.into_bytes());
            }
        }
        Err(err) => {
            runtime.reset();
            crate::input::voice::update(
                &mut model.voice,
                VoiceInputMessage::TranscriptionError(err),
            );
        }
    }
}

fn maybe_start_wizard_branch_suggestions(wizard: &mut screens::wizard::WizardState) {
    maybe_start_wizard_branch_suggestions_with(wizard, request_branch_suggestions);
}

fn maybe_start_wizard_branch_suggestions_with<F>(
    wizard: &mut screens::wizard::WizardState,
    request: F,
) where
    F: FnOnce(&str) -> Result<Vec<String>, String>,
{
    if wizard.step != screens::wizard::WizardStep::AIBranchSuggest
        || !wizard.ai_suggest.loading
        || wizard.ai_suggest.tick_counter != 0
        || !wizard.ai_suggest.suggestions.is_empty()
        || wizard.ai_suggest.error.is_some()
    {
        return;
    }

    let context = wizard_branch_suggestion_context(wizard);
    let msg = match request(&context) {
        Ok(suggestions) => screens::wizard::WizardMessage::SetBranchSuggestions(suggestions),
        Err(err) => screens::wizard::WizardMessage::SetBranchSuggestError(err),
    };
    screens::wizard::update(wizard, msg);
}

fn wizard_branch_suggestion_context(wizard: &screens::wizard::WizardState) -> String {
    let mut parts = Vec::new();
    if let Some(summary) = wizard.spec_context_summary() {
        parts.push(format!("SPEC: {summary}"));
    }
    if let Some(spec_context) = wizard.spec_context.as_ref() {
        let spec_body = spec_context.spec_body.trim();
        if !spec_body.is_empty() {
            parts.push(format!("SPEC body:\n{spec_body}"));
        }
    }
    if !wizard.branch_name.trim().is_empty() {
        parts.push(format!(
            "Current branch seed: {}",
            wizard.branch_name.trim()
        ));
    }
    if !wizard.issue_id.trim().is_empty() {
        parts.push(format!("Issue: {}", wizard.issue_id.trim()));
    }
    if parts.is_empty() {
        "Create a concise git branch name for a new worktree task.".to_string()
    } else {
        parts.join("\n")
    }
}

fn request_branch_suggestions(context: &str) -> Result<Vec<String>, String> {
    let client = branch_suggestion_client()?;
    suggest_branch_name(&client, context).map_err(|err| err.to_string())
}

fn branch_suggestion_client() -> Result<AIClient, String> {
    let loaded = load_settings_with_active_profile_fallback();
    if let Some(ai_settings) = loaded
        .settings
        .profiles
        .active_profile()
        .and_then(|profile| profile.ai_settings.as_ref())
    {
        if ai_settings.is_enabled() {
            return ai_client_from_settings(ai_settings);
        }
    }

    let endpoint = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let model = std::env::var("OPENAI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "AI branch suggestion requires active profile AI settings or OPENAI_MODEL".to_string()
        })?;
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();

    AIClient::new(&endpoint, &api_key, &model).map_err(|err| err.to_string())
}

fn ai_client_from_settings(settings: &AISettings) -> Result<AIClient, String> {
    AIClient::new(
        &settings.endpoint,
        settings.api_key.as_deref().unwrap_or(""),
        &settings.model,
    )
    .map_err(|err| err.to_string())
}

/// Build a LaunchConfig from the wizard's accumulated selections.
fn build_launch_config_from_wizard(wizard: &screens::wizard::WizardState) -> LaunchConfig {
    let custom_agents = load_custom_agents();
    build_launch_config_from_wizard_with_custom_agents(wizard, &custom_agents)
}

fn build_launch_config_from_wizard_with_custom_agents(
    wizard: &screens::wizard::WizardState,
    custom_agents: &[CustomCodingAgent],
) -> LaunchConfig {
    if let Some(custom_agent) = custom_agents
        .iter()
        .find(|agent| agent.id == wizard.agent_id)
    {
        return build_custom_launch_config_from_wizard(wizard, custom_agent);
    }

    let agent_id = match wizard.agent_id.as_str() {
        "claude" => AgentId::ClaudeCode,
        "codex" => AgentId::Codex,
        "gemini" => AgentId::Gemini,
        "opencode" => AgentId::OpenCode,
        "gh" => AgentId::Copilot,
        other => AgentId::Custom(other.to_string()),
    };

    let mut builder = AgentLaunchBuilder::new(agent_id);

    if !wizard.is_new_branch {
        if let Some(ref wt) = wizard.worktree_path {
            builder = builder.working_dir(wt);
        }
    }

    if !wizard.branch_name.is_empty() {
        builder = builder.branch(&wizard.branch_name);
    }
    if let Some(base_branch) = wizard_launch_base_branch(wizard) {
        builder = builder.base_branch(base_branch);
    }

    if is_explicit_model_selection(&wizard.model) {
        builder = builder.model(&wizard.model);
    }

    if !wizard.version.is_empty() {
        builder = builder.version(&wizard.version);
    }

    if let Some(reasoning_level) = wizard_reasoning_level_for_launch(wizard) {
        builder = builder.reasoning_level(reasoning_level);
    }

    if wizard.agent_id == "codex" && wizard.codex_fast_mode {
        builder = builder.fast_mode(true);
    }

    if wizard.skip_perms {
        builder = builder.skip_permissions(true);
    }
    builder = builder.runtime_target(wizard.runtime_target);
    if let Some(docker_service) = wizard.docker_service.as_deref() {
        builder = builder.docker_service(docker_service);
    }
    builder = builder.docker_lifecycle_intent(wizard.docker_lifecycle_intent);
    let session_mode = match wizard.mode.as_str() {
        "continue" => SessionMode::Continue,
        "resume" if wizard.resume_session_id.is_some() => SessionMode::Resume,
        "resume" => SessionMode::Continue,
        _ => SessionMode::Normal,
    };
    builder = builder.session_mode(session_mode);
    if let Some(resume_session_id) = wizard.resume_session_id.as_deref() {
        builder = builder.resume_session_id(resume_session_id);
    }

    let mut config = builder.build();
    if !wizard.version.is_empty() {
        config.tool_version = Some(wizard.version.clone());
    }
    if let Some(reasoning_level) = wizard_reasoning_level_for_launch(wizard) {
        config.reasoning_level = Some(reasoning_level.to_string());
    } else if wizard.agent_id == "codex" && !wizard.reasoning.is_empty() {
        config.reasoning_level = Some(wizard.reasoning.clone());
    }
    config.linked_issue_number = wizard.issue_id.parse::<u64>().ok();
    config
}

fn build_custom_launch_config_from_wizard(
    wizard: &screens::wizard::WizardState,
    custom_agent: &CustomCodingAgent,
) -> LaunchConfig {
    let session_mode = match wizard.mode.as_str() {
        "continue" => SessionMode::Continue,
        "resume" if wizard.resume_session_id.is_some() => SessionMode::Resume,
        "resume" => SessionMode::Continue,
        _ => SessionMode::Normal,
    };

    let mut args = custom_agent.default_args.clone();
    if let Some(mode_args) = &custom_agent.mode_args {
        match session_mode {
            SessionMode::Normal => args.extend(mode_args.normal.clone()),
            SessionMode::Continue => args.extend(mode_args.continue_mode.clone()),
            SessionMode::Resume => args.extend(mode_args.resume.clone()),
        }
    }
    if wizard.skip_perms {
        args.extend(custom_agent.skip_permissions_args.clone());
    }

    let command = match custom_agent.agent_type {
        CustomAgentType::Command | CustomAgentType::Path => custom_agent.command.clone(),
        CustomAgentType::Bunx => {
            if gwt_core::process::command_exists("bunx") {
                args.insert(0, custom_agent.command.clone());
                "bunx".to_string()
            } else {
                args.insert(0, custom_agent.command.clone());
                args.insert(0, "--yes".to_string());
                "npx".to_string()
            }
        }
    };

    let mut env_vars = HashMap::new();
    env_vars.insert("TERM".to_string(), "xterm-256color".to_string());
    env_vars.extend(custom_agent.env.clone());

    let agent_id = AgentId::Custom(custom_agent.id.clone());
    LaunchConfig {
        color: agent_id.default_color(),
        agent_id,
        command,
        args,
        env_vars,
        working_dir: (!wizard.is_new_branch)
            .then(|| wizard.worktree_path.clone())
            .flatten(),
        branch: (!wizard.branch_name.is_empty()).then(|| wizard.branch_name.clone()),
        base_branch: wizard_launch_base_branch(wizard),
        display_name: custom_agent.display_name.clone(),
        model: None,
        tool_version: None,
        reasoning_level: None,
        session_mode,
        resume_session_id: wizard.resume_session_id.clone(),
        skip_permissions: wizard.skip_perms,
        codex_fast_mode: false,
        runtime_target: wizard.runtime_target,
        docker_service: wizard.docker_service.clone(),
        docker_lifecycle_intent: wizard.docker_lifecycle_intent,
        linked_issue_number: wizard.issue_id.parse::<u64>().ok(),
    }
}

fn is_explicit_model_selection(model: &str) -> bool {
    !model.is_empty() && !model.starts_with("Default")
}

fn wizard_reasoning_level_for_launch(wizard: &screens::wizard::WizardState) -> Option<&str> {
    match wizard.agent_id.as_str() {
        "codex" if !wizard.reasoning.is_empty() => Some(wizard.reasoning.as_str()),
        "claude"
            if !wizard.reasoning.is_empty()
                && matches!(
                    wizard.model.as_str(),
                    "Default (Opus 4.6)" | "opus" | "sonnet"
                ) =>
        {
            Some(wizard.reasoning.as_str())
        }
        _ => None,
    }
}

fn wizard_launch_base_branch(wizard: &screens::wizard::WizardState) -> Option<String> {
    if !wizard.is_new_branch {
        None
    } else {
        Some(
            wizard
                .base_branch_name
                .clone()
                .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string()),
        )
    }
}

fn materialize_pending_launch(model: &mut Model) {
    if model
        .pending_launch_config
        .as_ref()
        .is_some_and(|config| config.runtime_target == LaunchRuntimeTarget::Docker)
    {
        if model.docker_progress_events.is_some() {
            update(
                model,
                Message::PushErrorNotification(Notification::new(
                    Severity::Error,
                    "docker",
                    "Docker launch already running",
                )),
            );
            return;
        }
        if let Some(config) = model.pending_launch_config.take() {
            if let Err(err) = link_selected_issue_to_branch(&model.repo_path, &config) {
                update(
                    model,
                    Message::PushErrorNotification(
                        Notification::new(Severity::Error, "session", "Agent launch failed")
                            .with_detail(err),
                    ),
                );
                return;
            }
            if let Err(err) = persist_issue_linkage(&model.repo_path, &config) {
                tracing::warn!("issue linkage store update failed: {err}");
            } else if config.linked_issue_number.is_some() {
                reload_cached_issues(model);
            }
            start_async_docker_launch(model, config);
        }
        return;
    }

    if let Err(err) = materialize_pending_launch_with(model, &gwt_sessions_dir()) {
        update(
            model,
            Message::PushErrorNotification(
                Notification::new(Severity::Error, "session", "Agent launch failed")
                    .with_detail(err),
            ),
        );
    }
}

#[tracing::instrument(
    name = "materialize_pending_launch",
    skip(model, sessions_dir),
    fields(repo_path = %model.repo_path().display())
)]
fn materialize_pending_launch_with(
    model: &mut Model,
    sessions_dir: &std::path::Path,
) -> Result<(), String> {
    materialize_pending_launch_with_hooks(
        model,
        sessions_dir,
        link_selected_issue_to_branch,
        resolve_launch_worktree,
    )
}

fn materialize_pending_launch_with_hooks<Link, Resolve>(
    model: &mut Model,
    sessions_dir: &std::path::Path,
    link_issue: Link,
    resolve_worktree: Resolve,
) -> Result<(), String>
where
    Link: FnOnce(&std::path::Path, &LaunchConfig) -> Result<(), String>,
    Resolve: FnOnce(&std::path::Path, &mut LaunchConfig) -> Result<(), String>,
{
    let Some(mut config) = model.pending_launch_config.take() else {
        return Ok(());
    };

    link_issue(&model.repo_path, &config)?;
    resolve_worktree(&model.repo_path, &mut config)?;
    if let Err(err) = persist_issue_linkage(&model.repo_path, &config) {
        tracing::warn!("issue linkage store update failed: {err}");
    } else if config.linked_issue_number.is_some() {
        reload_cached_issues(model);
    }
    if let Err(err) = apply_docker_runtime_to_launch_config(&model.repo_path, &mut config) {
        update(
            model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetError(
                err.clone(),
            )),
        );
        update(
            model,
            Message::PushErrorNotification(
                Notification::new(Severity::Error, "docker", "Docker launch failed")
                    .with_detail(err),
            ),
        );
        return Ok(());
    }
    persist_and_spawn_launch(model, sessions_dir, config)
}

fn start_async_docker_launch(model: &mut Model, config: LaunchConfig) {
    update(
        model,
        Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetStage {
            stage: screens::docker_progress::DockerStage::DetectingFiles,
            message: "Preparing Docker launch".to_string(),
        }),
    );

    let events = Arc::new(Mutex::new(VecDeque::new()));
    model.docker_progress_events = Some(events.clone());
    spawn_docker_launch_worker(events, model.repo_path.clone(), config);
}

fn spawn_docker_launch_worker(
    events: DockerProgressQueue,
    repo_path: PathBuf,
    mut config: LaunchConfig,
) {
    thread::spawn(move || {
        let outcome = (|| -> Result<LaunchConfig, String> {
            resolve_launch_worktree(&repo_path, &mut config)?;
            apply_docker_runtime_to_launch_config_with_progress(
                &repo_path,
                &mut config,
                |stage, message| {
                    push_docker_progress_event(
                        &events,
                        DockerProgressEvent::Stage { stage, message },
                    );
                },
                |entry| {
                    push_docker_progress_event(&events, DockerProgressEvent::Log { entry });
                },
            )?;
            Ok(config)
        })();

        let event = match outcome {
            Ok(config) => DockerProgressEvent::LaunchReady {
                config: Box::new(config),
            },
            Err(detail) => DockerProgressEvent::LaunchFailed {
                message: "Docker launch failed".to_string(),
                detail,
            },
        };
        push_docker_progress_event(&events, event);
    });
}

fn persist_and_spawn_launch(
    model: &mut Model,
    sessions_dir: &std::path::Path,
    mut config: LaunchConfig,
) -> Result<(), String> {
    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| model.repo_path.clone());
    let mut session = AgentSession::new(
        worktree,
        config.branch.clone().unwrap_or_default(),
        config.agent_id.clone(),
    );
    session.model = config
        .model
        .clone()
        .filter(|model| is_explicit_model_selection(model));
    session.reasoning_level = config.reasoning_level.clone();
    session.tool_version = config.tool_version.clone();
    session.agent_session_id = config.resume_session_id.clone();
    session.skip_permissions = config.skip_permissions;
    session.codex_fast_mode = config.codex_fast_mode;
    session.runtime_target = config.runtime_target;
    session.docker_service = config.docker_service.clone();
    session.docker_lifecycle_intent = config.docker_lifecycle_intent;
    session.launch_command = config.command.clone();
    session.launch_args = config.args.clone();
    session.display_name = config.display_name.clone();
    session.save(sessions_dir).map_err(|err| err.to_string())?;
    augment_agent_hook_runtime_launch_config(&mut config, sessions_dir, &session.id);

    let mut vt = crate::model::VtState::new(24, 80);
    vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);
    let tab = crate::model::SessionTab {
        id: session.id.clone(),
        name: config.display_name.clone(),
        tab_type: SessionTabType::Agent {
            agent_id: config.agent_id.command().to_string(),
            color: tui_agent_color(config.color),
        },
        vt,
        created_at: std::time::Instant::now(),
    };
    let tab_id = tab.id.clone();
    model.sessions.push(tab);
    model.active_session = model.sessions.len().saturating_sub(1);
    model.active_layer = ActiveLayer::Main;

    let (cols, rows) = session_content_size(model);
    if let Some(s) = model.sessions.last_mut() {
        s.vt.resize(rows, cols);
    }

    let worktree: std::path::PathBuf = config
        .working_dir
        .clone()
        .unwrap_or_else(|| model.repo_path.clone());
    refresh_managed_gwt_assets_for_worktree(&worktree, "agent launch", true);

    let (mut pty_env, mut remove_env) = spawn_env_with_active_profile(config.env_vars.clone());
    inject_agent_hook_runtime_env(&mut pty_env, sessions_dir, &session.id);
    remove_env.retain(|key| !pty_env.contains_key(key));
    let pty_config = gwt_terminal::pty::SpawnConfig {
        command: config.command.clone(),
        args: config.args.clone(),
        cols,
        rows,
        env: pty_env,
        remove_env,
        cwd: config.working_dir.clone(),
    };
    let repo_path_for_watcher = model.repo_path.clone();
    emit_agent_launch_event(&model.repo_path, &tab_id, &pty_config);
    if let Err(e) = spawn_pty_for_session(model, &tab_id, pty_config) {
        apply_notification(
            model,
            Notification::new(
                Severity::Error,
                "pty",
                format!("Agent PTY spawn failed: {e}"),
            ),
        );
    } else {
        bootstrap_agent_session_waiting_input(sessions_dir, &session.id);
        // Phase 8: ensure a watcher is running for this Worktree so live
        // SPEC/file edits feed the incremental indexer.
        crate::index_worker::ensure_watcher(&repo_path_for_watcher, &worktree);
        crate::index_worker::kick_initial_build_for_worktree(&repo_path_for_watcher, &worktree);
    }

    refresh_branch_live_session_summaries_with(model, sessions_dir);

    apply_notification(
        model,
        Notification::new(
            Severity::Info,
            "session",
            format!("Created session for {}", config.display_name),
        ),
    );

    Ok(())
}

fn link_selected_issue_to_branch(
    repo_path: &std::path::Path,
    config: &LaunchConfig,
) -> Result<(), String> {
    link_selected_issue_to_branch_with(repo_path, config, |cwd, args| {
        let output = Command::new("gh")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|err| format!("gh issue develop: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "gh issue develop: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    })
}

fn link_selected_issue_to_branch_with<Run>(
    repo_path: &std::path::Path,
    config: &LaunchConfig,
    run: Run,
) -> Result<(), String>
where
    Run: FnOnce(&std::path::Path, &[String]) -> Result<(), String>,
{
    let Some(issue_number) = config.linked_issue_number else {
        return Ok(());
    };
    let branch_name = config
        .branch
        .as_deref()
        .ok_or_else(|| "issue linkage requires a branch name".to_string())?;
    let base_branch = config
        .base_branch
        .as_deref()
        .unwrap_or(DEFAULT_NEW_BRANCH_BASE_BRANCH);
    let args = vec![
        "issue".to_string(),
        "develop".to_string(),
        issue_number.to_string(),
        "--name".to_string(),
        branch_name.to_string(),
        "--base".to_string(),
        base_branch.to_string(),
    ];
    run(repo_path, &args)
}

fn close_active_session_with(model: &mut Model, sessions_dir: &Path) {
    if model.sessions.len() <= 1 {
        return;
    }

    let id = model.sessions[model.active_session].id.clone();
    let is_agent = matches!(
        model.sessions[model.active_session].tab_type,
        SessionTabType::Agent { .. }
    );
    if is_agent {
        persist_agent_session_stopped(sessions_dir, &id);
    }
    if let Some(pty) = model.pty_handles.remove(&id) {
        let _ = pty.kill();
    }
    model.sessions.remove(model.active_session);
    if model.active_session >= model.sessions.len() {
        model.active_session = model.sessions.len() - 1;
    }
    refresh_branch_live_session_summaries_with(model, sessions_dir);
}

fn resolve_launch_worktree(repo_path: &Path, config: &mut LaunchConfig) -> Result<(), String> {
    let Some(branch_name) = config.branch.clone() else {
        return Ok(());
    };
    if config.working_dir.is_some() {
        return Ok(());
    }

    let current_branch = current_git_branch(repo_path);
    if current_branch.is_err() && config.base_branch.is_none() {
        // Keep best-effort behavior for non-repo tests and ad-hoc launches that
        // don't provide an explicit base branch.
        return Ok(());
    }
    if current_branch
        .as_ref()
        .is_ok_and(|current| current == &branch_name)
    {
        config.working_dir = Some(repo_path.to_path_buf());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            repo_path.display().to_string(),
        );
        return Ok(());
    }

    let main_repo_path =
        gwt_git::worktree::main_worktree_root(repo_path).map_err(|err| err.to_string())?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let worktrees = manager.list().map_err(|err| err.to_string())?;
    if let Some(existing_worktree) = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_name.as_str()))
        .map(|worktree| worktree.path.clone())
    {
        config.working_dir = Some(existing_worktree.clone());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            existing_worktree.display().to_string(),
        );
        return Ok(());
    }

    let base_branch = config
        .base_branch
        .clone()
        .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string());
    let remote_base_ref = origin_remote_ref(&base_branch);
    let remote_branch_ref = origin_remote_ref(&branch_name);

    manager
        .fetch_origin()
        .map_err(|err| format!("failed to fetch origin: {err}"))?;

    if !manager
        .remote_branch_exists(&remote_base_ref)
        .map_err(|err| format!("failed to verify remote base branch {remote_base_ref}: {err}"))?
    {
        return Err(format!(
            "remote base branch does not exist: {remote_base_ref}"
        ));
    }

    if !manager
        .remote_branch_exists(&remote_branch_ref)
        .map_err(|err| format!("failed to verify remote branch {remote_branch_ref}: {err}"))?
    {
        manager
            .create_remote_branch_from_base(&remote_base_ref, &branch_name)
            .map_err(|err| {
                format!(
                    "failed to create remote branch {remote_branch_ref} from {remote_base_ref}: {err}"
                )
            })?;
        manager
            .fetch_origin()
            .map_err(|err| format!("failed to refresh origin refs after push: {err}"))?;
    }

    let preferred_worktree_path =
        gwt_git::worktree::sibling_worktree_path(&main_repo_path, &branch_name);
    let worktree_path = first_available_worktree_path(&preferred_worktree_path, &worktrees)
        .ok_or_else(|| {
            format!("failed to resolve available worktree path for branch {branch_name}")
        })?;
    if worktree_path != preferred_worktree_path {
        tracing::warn!(
            branch = branch_name,
            preferred = %preferred_worktree_path.display(),
            selected = %worktree_path.display(),
            "preferred worktree path is occupied; using suffixed fallback"
        );
    }
    if local_branch_exists(&main_repo_path, &branch_name)? {
        manager
            .create(&branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    } else {
        manager
            .create_from_remote(&remote_branch_ref, &branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    }

    config.working_dir = Some(worktree_path.clone());
    config.env_vars.insert(
        "GWT_PROJECT_ROOT".to_string(),
        worktree_path.display().to_string(),
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct DockerLaunchPlan {
    compose_file: PathBuf,
    service: String,
    container_cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerExecProgram {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerPackageRunnerCandidate {
    executable: &'static str,
    base_args: Vec<String>,
}

#[derive(Debug, Clone)]
struct DevContainerLaunchDefaults {
    service: Option<String>,
    workspace_folder: Option<String>,
    compose_file: Option<PathBuf>,
}

fn apply_docker_runtime_to_launch_config(
    repo_path: &Path,
    config: &mut LaunchConfig,
) -> Result<(), String> {
    apply_docker_runtime_to_launch_config_with_progress(repo_path, config, |_, _| {}, |_| {})
}

fn apply_docker_runtime_to_launch_config_with_progress<F, G>(
    repo_path: &Path,
    config: &mut LaunchConfig,
    mut emit_progress: F,
    mut emit_log: G,
) -> Result<(), String>
where
    F: FnMut(screens::docker_progress::DockerStage, String),
    G: FnMut(Notification),
{
    if config.runtime_target != LaunchRuntimeTarget::Docker {
        return Ok(());
    }

    emit_progress(
        screens::docker_progress::DockerStage::DetectingFiles,
        "Resolving Docker launch configuration".to_string(),
    );
    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    ensure_docker_launch_runtime_ready()?;
    ensure_docker_launch_service_ready(
        &launch,
        config.docker_lifecycle_intent,
        &mut emit_progress,
        &mut emit_log,
    )?;
    maybe_inject_docker_sandbox_env(&launch, config)?;
    emit_progress(
        screens::docker_progress::DockerStage::WaitingForServices,
        format!(
            "Checking launch command in Docker service {}",
            launch.service
        ),
    );
    let runtime_program = resolve_docker_exec_program(&launch, config)?;

    let mut args = vec![
        "compose".to_string(),
        "-f".to_string(),
        launch.compose_file.display().to_string(),
        "exec".to_string(),
        "-w".to_string(),
        launch.container_cwd.clone(),
    ];
    args.extend(docker_compose_exec_env_args(&config.env_vars));
    args.push(launch.service.clone());
    args.push(runtime_program.executable);
    args.extend(runtime_program.args);

    config.command = docker_binary_for_launch();
    config.args = args;
    config
        .env_vars
        .insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    config.docker_service = Some(launch.service);

    Ok(())
}

fn docker_compose_up_log_entry(
    service: &str,
    stream: Option<gwt_docker::CommandOutputStream>,
    message: impl Into<String>,
) -> Notification {
    let stream_label = stream.map(|stream| match stream {
        gwt_docker::CommandOutputStream::Stdout => "stdout",
        gwt_docker::CommandOutputStream::Stderr => "stderr",
    });
    let rendered = match stream_label {
        Some(label) => format!("[{service}][{label}] {}", message.into()),
        None => format!("[{service}] {}", message.into()),
    };
    let severity = match stream {
        Some(gwt_docker::CommandOutputStream::Stderr) => Severity::Warn,
        _ => Severity::Info,
    };
    let mut event = Notification::new(severity, "docker", rendered)
        .with_field(
            "action",
            serde_json::Value::String("compose_up".to_string()),
        )
        .with_field("service", serde_json::Value::String(service.to_string()));
    if let Some(label) = stream_label {
        event = event.with_field("stream", serde_json::Value::String(label.to_string()));
    }
    event
}

fn ensure_docker_launch_runtime_ready() -> Result<(), String> {
    if !gwt_docker::docker_available() {
        return Err("Docker is not installed or not available on PATH".to_string());
    }
    if !gwt_docker::compose_available() {
        return Err("docker compose is not available".to_string());
    }
    if !gwt_docker::daemon_running() {
        return Err("Docker daemon is not running".to_string());
    }
    Ok(())
}

fn maybe_inject_docker_sandbox_env(
    launch: &DockerLaunchPlan,
    config: &mut LaunchConfig,
) -> Result<(), String> {
    if cfg!(windows) || !matches!(config.agent_id, AgentId::ClaudeCode) || !config.skip_permissions
    {
        return Ok(());
    }

    let is_root = gwt_docker::compose_service_user_is_root(&launch.compose_file, &launch.service)
        .map_err(|err| {
        format!(
            "Failed to determine Docker user for service '{}': {err}",
            launch.service
        )
    })?;
    if is_root {
        config
            .env_vars
            .insert("IS_SANDBOX".to_string(), "1".to_string());
    }
    Ok(())
}

fn docker_compose_exec_env_args(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut keys = env_vars.keys().collect::<Vec<_>>();
    keys.sort();

    let mut args = Vec::new();
    for key in keys {
        let key = key.trim();
        if key.is_empty() || !is_valid_docker_env_key(key) {
            continue;
        }
        let value = env_vars.get(key).map(String::as_str).unwrap_or_default();
        args.push("-e".to_string());
        args.push(format!("{key}={value}"));
    }
    args
}

fn is_valid_docker_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn resolve_docker_exec_program(
    launch: &DockerLaunchPlan,
    config: &LaunchConfig,
) -> Result<DockerExecProgram, String> {
    let Some(version_spec) = docker_package_version_spec(config) else {
        ensure_docker_launch_command_ready(launch, &config.command)?;
        return Ok(DockerExecProgram {
            executable: config.command.clone(),
            args: config.args.clone(),
        });
    };

    resolve_docker_package_runner(launch, config, &version_spec)
}

fn docker_package_version_spec(config: &LaunchConfig) -> Option<String> {
    let package = config.agent_id.package_name()?;
    let version = config.tool_version.as_deref()?;
    if version == "installed" || version.is_empty() {
        return None;
    }

    Some(if version == "latest" {
        format!("{package}@latest")
    } else {
        format!("{package}@{version}")
    })
}

fn resolve_docker_package_runner(
    launch: &DockerLaunchPlan,
    config: &LaunchConfig,
    version_spec: &str,
) -> Result<DockerExecProgram, String> {
    let agent_args = strip_docker_package_runner_args(&config.args, version_spec);
    let candidates = vec![
        DockerPackageRunnerCandidate {
            executable: "bunx",
            base_args: vec![version_spec.to_string()],
        },
        DockerPackageRunnerCandidate {
            executable: "npx",
            base_args: vec!["--yes".to_string(), version_spec.to_string()],
        },
    ];
    let mut failures = Vec::new();

    for candidate in candidates {
        let output = gwt_docker::compose_service_exec_capture(
            &launch.compose_file,
            &launch.service,
            Some(&launch.container_cwd),
            &candidate.probe_args(),
        )
        .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(candidate.into_exec_program(agent_args.clone()));
        }
        failures.push(format!(
            "{}: {}",
            candidate.executable,
            docker_compose_exec_failure_detail(&output)
        ));
    }

    Err(format!(
        "Selected Docker runtime cannot launch {version_spec} in service '{}'. {}",
        launch.service,
        failures.join(" | ")
    ))
}

fn strip_docker_package_runner_args(args: &[String], version_spec: &str) -> Vec<String> {
    if args.first().is_some_and(|first| first == "--yes")
        && args.get(1).is_some_and(|arg| arg == version_spec)
    {
        return args[2..].to_vec();
    }
    if args.first().is_some_and(|arg| arg == version_spec) {
        return args[1..].to_vec();
    }
    args.to_vec()
}

fn docker_compose_exec_failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }

    output
        .status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string())
}

fn ensure_docker_launch_command_ready(
    launch: &DockerLaunchPlan,
    command: &str,
) -> Result<(), String> {
    let available =
        gwt_docker::compose_service_has_command(&launch.compose_file, &launch.service, command)
            .map_err(|err| err.to_string())?;
    if available {
        return Ok(());
    }

    Err(format!(
        "Command '{command}' is not available in Docker service '{}'",
        launch.service
    ))
}

impl DockerPackageRunnerCandidate {
    fn probe_args(&self) -> Vec<String> {
        let mut args = vec![self.executable.to_string()];
        args.extend(self.base_args.clone());
        args.push("--version".to_string());
        args
    }

    fn into_exec_program(self, mut agent_args: Vec<String>) -> DockerExecProgram {
        let mut args = self.base_args;
        args.append(&mut agent_args);
        DockerExecProgram {
            executable: self.executable.to_string(),
            args,
        }
    }
}

fn ensure_docker_launch_service_ready<F, G>(
    launch: &DockerLaunchPlan,
    intent: gwt_agent::DockerLifecycleIntent,
    emit_progress: &mut F,
    emit_log: &mut G,
) -> Result<(), String>
where
    F: FnMut(screens::docker_progress::DockerStage, String),
    G: FnMut(Notification),
{
    let status = gwt_docker::compose_service_status(&launch.compose_file, &launch.service)
        .map_err(|err| err.to_string())?;
    let action = normalize_docker_launch_action(intent, status);

    match action {
        DockerLaunchServiceAction::Connect => return Ok(()),
        DockerLaunchServiceAction::Start => {
            emit_progress(
                screens::docker_progress::DockerStage::BuildingImage,
                format!(
                    "Building image and starting Docker service {}",
                    launch.service
                ),
            );
            emit_log(docker_compose_up_log_entry(
                &launch.service,
                None,
                format!("Running docker compose up -d {}", launch.service),
            ));
            gwt_docker::compose_up_with_output(
                &launch.compose_file,
                &launch.service,
                |stream, line| {
                    emit_log(docker_compose_up_log_entry(
                        &launch.service,
                        Some(stream),
                        line.to_string(),
                    ));
                },
            )
            .map_err(|err| err.to_string())?;
            emit_log(docker_compose_up_log_entry(
                &launch.service,
                None,
                format!("docker compose up -d {} completed", launch.service),
            ));
        }
        DockerLaunchServiceAction::Restart => {
            emit_progress(
                screens::docker_progress::DockerStage::StartingContainer,
                format!("Restarting Docker service {}", launch.service),
            );
            emit_log(docker_compose_up_log_entry(
                &launch.service,
                None,
                format!("Running docker compose restart {}", launch.service),
            ));
            gwt_docker::compose_restart(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())?;
        }
        DockerLaunchServiceAction::Recreate => {
            emit_progress(
                screens::docker_progress::DockerStage::StartingContainer,
                format!("Recreating Docker service {}", launch.service),
            );
            emit_log(docker_compose_up_log_entry(
                &launch.service,
                None,
                format!(
                    "Running docker compose up -d --force-recreate {}",
                    launch.service
                ),
            ));
            gwt_docker::compose_up_force_recreate_with_output(
                &launch.compose_file,
                &launch.service,
                |stream, line| {
                    emit_log(docker_compose_up_log_entry(
                        &launch.service,
                        Some(stream),
                        line.to_string(),
                    ));
                },
            )
            .map_err(|err| err.to_string())?;
            emit_log(docker_compose_up_log_entry(
                &launch.service,
                None,
                format!(
                    "docker compose up -d --force-recreate {} completed",
                    launch.service
                ),
            ));
        }
    }

    emit_progress(
        screens::docker_progress::DockerStage::WaitingForServices,
        format!("Waiting for Docker service {}", launch.service),
    );
    let running = gwt_docker::compose_service_is_running(&launch.compose_file, &launch.service)
        .map_err(|err| err.to_string())?;
    if running {
        return Ok(());
    }

    let mut message = format!(
        "docker compose service '{}' is not running after startup.",
        launch.service
    );
    if let Ok(logs) = gwt_docker::compose_service_logs(&launch.compose_file, &launch.service) {
        let trimmed = logs.trim();
        if !trimmed.is_empty() {
            message.push_str("\n\n");
            message.push_str(trimmed);
        }
    }
    Err(message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerLaunchServiceAction {
    Connect,
    Start,
    Restart,
    Recreate,
}

fn normalize_docker_launch_action(
    intent: gwt_agent::DockerLifecycleIntent,
    status: gwt_docker::ComposeServiceStatus,
) -> DockerLaunchServiceAction {
    use gwt_agent::DockerLifecycleIntent;
    use gwt_docker::ComposeServiceStatus;

    match intent {
        DockerLifecycleIntent::Recreate => DockerLaunchServiceAction::Recreate,
        DockerLifecycleIntent::Restart if status == ComposeServiceStatus::Running => {
            DockerLaunchServiceAction::Restart
        }
        DockerLifecycleIntent::Connect
        | DockerLifecycleIntent::Start
        | DockerLifecycleIntent::Restart
        | DockerLifecycleIntent::CreateAndStart => match status {
            ComposeServiceStatus::Running => DockerLaunchServiceAction::Connect,
            ComposeServiceStatus::Stopped
            | ComposeServiceStatus::Exited
            | ComposeServiceStatus::NotFound => DockerLaunchServiceAction::Start,
        },
    }
}

fn resolve_docker_launch_plan(
    worktree: &Path,
    selected_service: Option<&str>,
) -> Result<DockerLaunchPlan, String> {
    let files = gwt_docker::detect_docker_files(worktree);
    let compose_file = docker_compose_file_for_launch(worktree, &files)?.ok_or_else(|| {
        "Docker launch requires a docker-compose.yml or devcontainer compose target".to_string()
    })?;
    let services = gwt_docker::parse_compose_file(&compose_file).map_err(|err| err.to_string())?;
    if services.is_empty() {
        return Err("Docker launch requires at least one compose service".to_string());
    }

    let devcontainer_defaults = docker_devcontainer_defaults(worktree, &files);
    let service_name = selected_service
        .map(str::to_string)
        .or_else(|| {
            devcontainer_defaults
                .as_ref()
                .and_then(|defaults| defaults.service.clone())
        })
        .or_else(|| {
            if services.len() == 1 {
                services.first().map(|service| service.name.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            "Multiple Docker services detected; select a Docker service in Launch Agent Wizard"
                .to_string()
        })?;

    let service = services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| {
            format!("Selected Docker service was not found in compose file: {service_name}")
        })?;

    let container_cwd = devcontainer_defaults
        .as_ref()
        .and_then(|defaults| defaults.workspace_folder.clone())
        .or_else(|| service.working_dir.clone())
        .or_else(|| compose_workspace_mount_target(worktree, service))
        .ok_or_else(|| {
            format!(
                "Docker service {} is missing working_dir/workspaceFolder and no project-root volume mount was detected",
                service.name
            )
        })?;

    Ok(DockerLaunchPlan {
        compose_file,
        service: service.name.clone(),
        container_cwd,
    })
}

fn docker_binary_for_launch() -> String {
    std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string())
}

fn docker_compose_file_for_launch(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Result<Option<PathBuf>, String> {
    Ok(docker_devcontainer_defaults(project_root, files)
        .and_then(|defaults| defaults.compose_file)
        .or_else(|| files.compose_file.clone()))
}

fn docker_devcontainer_defaults(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Option<DevContainerLaunchDefaults> {
    let devcontainer_dir = files.devcontainer_dir.as_ref()?;
    let path = devcontainer_dir.join("devcontainer.json");
    if !path.is_file() {
        return None;
    }

    let config = gwt_docker::DevContainerConfig::load(&path).ok()?;
    let compose_file = config
        .docker_compose_file
        .as_ref()
        .and_then(|value| {
            value
                .to_vec()
                .into_iter()
                .map(|candidate| devcontainer_dir.join(candidate))
                .find(|path| path.is_file())
        })
        .or_else(|| files.compose_file.clone())
        .or_else(|| {
            let fallback = project_root.join("docker-compose.yml");
            fallback.is_file().then_some(fallback)
        });

    Some(DevContainerLaunchDefaults {
        service: config.service,
        workspace_folder: config.workspace_folder,
        compose_file,
    })
}

fn compose_workspace_mount_target(
    project_root: &Path,
    service: &gwt_docker::ComposeService,
) -> Option<String> {
    service
        .volumes
        .iter()
        .find(|mount| mount_source_matches_project_root(&mount.source, project_root))
        .map(|mount| mount.target.clone())
}

fn mount_source_matches_project_root(source: &str, project_root: &Path) -> bool {
    let normalized = source
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim_end_matches("/.");

    if matches!(normalized, "." | "$PWD" | "${PWD}") {
        return true;
    }

    let source_path = Path::new(normalized);
    source_path.is_absolute() && same_worktree_path(source_path, project_root)
}

fn first_available_worktree_path(
    preferred_path: &Path,
    worktrees: &[gwt_git::WorktreeInfo],
) -> Option<PathBuf> {
    if !worktree_path_is_occupied(preferred_path, worktrees) && !preferred_path.exists() {
        return Some(preferred_path.to_path_buf());
    }

    for suffix in 2usize.. {
        let candidate = suffixed_worktree_path(preferred_path, suffix)?;
        if !worktree_path_is_occupied(&candidate, worktrees) && !candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn suffixed_worktree_path(path: &Path, suffix: usize) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let mut candidate = path.to_path_buf();
    candidate.set_file_name(format!("{file_name}-{suffix}"));
    Some(candidate)
}

fn worktree_path_is_occupied(path: &Path, worktrees: &[gwt_git::WorktreeInfo]) -> bool {
    worktrees
        .iter()
        .any(|worktree| same_worktree_path(&worktree.path, path))
}

fn same_worktree_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn origin_remote_ref(branch_name: &str) -> String {
    if let Some(ref_name) = branch_name.strip_prefix("refs/remotes/") {
        ref_name.to_string()
    } else if branch_name.starts_with("origin/") {
        branch_name.to_string()
    } else {
        format!("origin/{branch_name}")
    }
}

fn current_git_branch(repo_path: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_path)
        .output()
        .map_err(|err| format!("git branch --show-current: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git branch --show-current: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        Err("git branch --show-current returned an empty branch name".to_string())
    } else {
        Ok(branch)
    }
}

fn local_branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|err| format!("git show-ref --verify refs/heads/{branch_name}: {err}"))?;

    Ok(output.status.success())
}

fn configure_existing_branch_wizard_with_sessions(
    wizard: &mut screens::wizard::WizardState,
    model: &Model,
    repo_path: &std::path::Path,
    sessions_dir: &std::path::Path,
    branch_name: &str,
) {
    wizard.is_new_branch = false;
    wizard.branch_name = branch_name.to_string();
    apply_wizard_docker_context(wizard, repo_path);
    wizard.quick_start_entries = load_quick_start_entries(repo_path, sessions_dir, branch_name);
    wizard.live_session_entries =
        branch_live_session_entries_with(model, sessions_dir, branch_name, repo_path);
    wizard.has_quick_start =
        !wizard.quick_start_entries.is_empty() || !wizard.live_session_entries.is_empty();
    wizard.focus_session_id = None;
    wizard.step = if wizard.has_quick_start {
        screens::wizard::WizardStep::QuickStart
    } else {
        screens::wizard::WizardStep::BranchAction
    };
    wizard.selected = 0;
}

fn load_quick_start_entries(
    repo_path: &std::path::Path,
    sessions_dir: &std::path::Path,
    branch_name: &str,
) -> Vec<screens::wizard::QuickStartEntry> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    let mut latest_by_agent: HashMap<String, AgentSession> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Ok(session) = AgentSession::load(&path) else {
            continue;
        };
        if session.branch != branch_name || session.worktree_path != repo_path {
            continue;
        }

        let agent_key = session.agent_id.command().to_string();
        let should_replace = latest_by_agent
            .get(&agent_key)
            .map(|current| {
                session.updated_at > current.updated_at
                    || (session.updated_at == current.updated_at
                        && session.created_at > current.created_at)
            })
            .unwrap_or(true);
        if should_replace {
            latest_by_agent.insert(agent_key, session);
        }
    }

    let mut sessions = latest_by_agent.into_values().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    sessions
        .into_iter()
        .map(|session| screens::wizard::QuickStartEntry {
            agent_id: session.agent_id.command().to_string(),
            tool_label: session.display_name.clone(),
            model: session.model.clone(),
            reasoning: session.reasoning_level.clone(),
            version: session.tool_version.clone().or_else(|| {
                session
                    .agent_id
                    .package_name()
                    .map(|_| "installed".to_string())
            }),
            resume_session_id: session.agent_session_id.clone(),
            skip_permissions: session.skip_permissions,
            codex_fast_mode: session.codex_fast_mode,
            runtime_target: session.runtime_target,
            docker_service: session.docker_service.clone(),
            docker_lifecycle_intent: session.docker_lifecycle_intent,
        })
        .collect()
}

fn open_wizard(model: &mut Model, spec_context: Option<screens::wizard::SpecContext>) {
    open_wizard_with_prefill(model, spec_context, None);
}

fn open_wizard_with_issue(model: &mut Model, issue_number: u64) {
    open_wizard_with_prefill(model, None, Some(issue_number));
}

fn open_wizard_with_prefill(
    model: &mut Model,
    spec_context: Option<screens::wizard::SpecContext>,
    initial_issue_number: Option<u64>,
) {
    let cache_path = wizard_version_cache_path();
    let cache = VersionCache::load(&cache_path);
    let detected_agents = AgentDetector::detect_all();
    let (wizard, refresh_targets) = if let Some(issue_number) = initial_issue_number {
        prepare_wizard_startup_with_issue_cache_root(
            &model.repo_path,
            spec_context,
            Some(issue_number),
            detected_agents,
            &cache,
            default_issue_cache_root(&model.repo_path),
        )
    } else {
        prepare_wizard_startup(&model.repo_path, spec_context, detected_agents, &cache)
    };

    model.wizard = Some(wizard);
    schedule_wizard_version_cache_refresh(cache_path, refresh_targets);
}

fn open_session_conversion(model: &mut Model) {
    open_session_conversion_with(model, AgentDetector::detect_all());
}

fn open_session_conversion_with(model: &mut Model, detected_agents: Vec<DetectedAgent>) {
    let Some(session) = model.active_session_tab() else {
        return;
    };
    let SessionTabType::Agent { agent_id, .. } = &session.tab_type else {
        return;
    };

    let (services, values): (Vec<_>, Vec<_>) = detected_agents
        .into_iter()
        .filter(|detected| detected.agent_id.command() != agent_id)
        .map(|detected| {
            (
                detected.agent_id.display_name().to_string(),
                detected.agent_id.command().to_string(),
            )
        })
        .unzip();

    if services.is_empty() {
        return;
    }

    model.pending_session_conversion = None;
    model.service_select = Some(screens::service_select::ServiceSelectState::with_options(
        "Select Agent",
        services,
        values,
    ));
}

fn handle_confirm_message(model: &mut Model, msg: screens::confirm::ConfirmMessage) {
    handle_confirm_message_with(model, msg, AgentDetector::detect_all());
}

// ---------------- Branch Cleanup integration (FR-018) ----------------

fn open_cleanup_confirm_for_selection(model: &mut Model) {
    use screens::cleanup_confirm::CleanupConfirmRow;

    // FR-018c: the selection set persists across view-mode / sort / search
    // changes, so the confirm modal must walk the full branch list — not
    // `filtered_branches()` — or previously selected branches that are
    // currently hidden by a filter would be silently dropped from the run.
    let mut rows: Vec<CleanupConfirmRow> = model
        .branches
        .branches
        .iter()
        .filter_map(|branch| {
            if !model.branches.is_cleanup_selected(&branch.name) {
                return None;
            }
            let execution_branch = model.branches.cleanup_execution_branch(&branch.name)?;
            Some(CleanupConfirmRow {
                branch: branch.name.clone(),
                target: model.branches.cleanup_target(&branch.name),
                execution_branch,
                upstream: if branch.is_local {
                    branch.upstream.clone()
                } else {
                    Some(origin_remote_ref(&branch.name))
                },
                risks: model.branches.cleanup_selection_risks(&branch.name),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.branch.cmp(&b.branch));

    if rows.is_empty() {
        apply_notification(
            model,
            Notification::new(
                gwt_core::logging::LogLevel::Warn,
                "cleanup",
                "No cleanup branches selected",
            ),
        );
        return;
    }

    model
        .cleanup_confirm
        .show(rows, model.branches.cleanup_settings.delete_remote);
}

fn handle_cleanup_confirm_message(
    model: &mut Model,
    msg: screens::cleanup_confirm::CleanupConfirmMessage,
) {
    use screens::cleanup_confirm::CleanupConfirmOutcome;

    let outcome = screens::cleanup_confirm::update(&mut model.cleanup_confirm, msg);
    model.branches.cleanup_settings.delete_remote = model.cleanup_confirm.delete_remote;
    match outcome {
        CleanupConfirmOutcome::Pending => {}
        CleanupConfirmOutcome::Cancelled => {}
        CleanupConfirmOutcome::Confirmed => {
            start_cleanup_run(model);
        }
    }
}

fn handle_cleanup_progress_message(
    model: &mut Model,
    msg: screens::cleanup_progress::CleanupProgressMessage,
) {
    use screens::cleanup_progress::CleanupProgressMessage;

    let was_completed = matches!(msg, CleanupProgressMessage::Completed);
    let was_dismiss_attempt = matches!(msg, CleanupProgressMessage::Dismiss);
    let was_visible_before = model.cleanup_progress.visible;
    screens::cleanup_progress::update(&mut model.cleanup_progress, msg);
    if was_completed {
        let succeeded = model
            .cleanup_progress
            .run
            .as_ref()
            .map(|run| run.succeeded())
            .unwrap_or(0);
        let failed = model
            .cleanup_progress
            .run
            .as_ref()
            .map(|run| run.failed())
            .unwrap_or(0);
        let severity = if failed > 0 {
            gwt_core::logging::LogLevel::Warn
        } else {
            gwt_core::logging::LogLevel::Info
        };
        apply_notification(
            model,
            Notification::new(
                severity,
                "cleanup",
                format!("Cleaned {succeeded}, failed {failed}"),
            ),
        );
    }
    // FR-018g: tear down only when the modal actually transitioned out of
    // visible state. The modal swallows `Dismiss` while `Running`, so we
    // must consult the post-update visibility instead of trusting the raw
    // incoming message — otherwise a stray `Dismiss` mid-run would clear
    // the selection and drop the cleanup queue while the worker thread
    // was still deleting branches.
    let dismissed = was_dismiss_attempt && was_visible_before && !model.cleanup_progress.visible;
    if dismissed {
        model.branches.clear_selection_after_cleanup();
        model.cleanup_progress.run = None;
        model.cleanup_events = None;
        load_initial_data(model);
    }
}

fn start_cleanup_run(model: &mut Model) {
    use std::sync::{Arc, Mutex};

    let branches: Vec<String> = model
        .cleanup_confirm
        .rows
        .iter()
        .map(|row| row.branch.clone())
        .collect();
    if branches.is_empty() {
        return;
    }

    let rows = model.cleanup_confirm.rows.clone();
    let delete_remote = model.cleanup_confirm.delete_remote;
    model.cleanup_progress.show(rows.len(), delete_remote);

    let queue: crate::model::CleanupEventQueue =
        Arc::new(Mutex::new(std::collections::VecDeque::new()));
    model.cleanup_events = Some(queue.clone());

    let repo_path = model.repo_path.clone();
    let active_session_branches: std::collections::HashSet<String> =
        model.branches.active_session_branches.clone();
    let current_head_branch: Option<String> = model.branches.current_head_branch.clone();

    // Snapshot worktree paths so the worker can shut the per-worktree index
    // watcher down before git removes the directory (Phase 8 contract).
    let worktree_paths: std::collections::HashMap<String, std::path::PathBuf> = model
        .branches
        .branches
        .iter()
        .filter_map(|item| {
            item.worktree_path
                .as_ref()
                .map(|path| (item.name.clone(), path.clone()))
        })
        .collect();
    std::thread::spawn(move || {
        let manager = gwt_git::WorktreeManager::new(&repo_path);
        for row in rows {
            let branch = row.branch.clone();
            let execution_branch = row.execution_branch.clone();
            queue
                .lock()
                .unwrap()
                .push_back(crate::model::CleanupEvent::Started {
                    branch: branch.clone(),
                });

            // Revalidate FR-018b protections immediately before deletion.
            // Branches with their own worktree are still candidates — the
            // whole point of Branch Cleanup is to remove the worktree along
            // with the branch.
            let blocked_reason = if gwt_git::is_protected_branch(&execution_branch) {
                Some("protected branch".to_string())
            } else if current_head_branch.as_deref() == Some(execution_branch.as_str()) {
                Some("current HEAD".to_string())
            } else if active_session_branches.contains(&execution_branch) {
                Some("active session".to_string())
            } else {
                None
            };

            let (success, message) = if let Some(reason) = blocked_reason {
                (false, Some(reason))
            } else {
                match manager.cleanup_branch(&execution_branch) {
                    Ok(()) => {
                        // Phase 8: shut the per-worktree index watcher down
                        // and drop the on-disk index dir ONLY after git has
                        // confirmed the worktree was removed. If we tore it
                        // down beforehand and `cleanup_branch` later failed
                        // (dirty worktree, git error, ...), the surviving
                        // worktree would stop being indexed until something
                        // explicitly recreated the watcher.
                        if let Some(path) = worktree_paths.get(&execution_branch) {
                            let _ = crate::index_worker::shutdown_and_remove(&repo_path, path);
                        }
                        if delete_remote {
                            match manager
                                .delete_remote_branch(&execution_branch, row.upstream.as_deref())
                            {
                                Ok(gwt_git::RemoteDeleteOutcome::Deleted)
                                | Ok(gwt_git::RemoteDeleteOutcome::SkippedMissing) => (true, None),
                                Err(err) => (false, Some(format!("remote delete failed: {err}"))),
                            }
                        } else {
                            (true, None)
                        }
                    }
                    Err(err) => (false, Some(err.to_string())),
                }
            };

            queue
                .lock()
                .unwrap()
                .push_back(crate::model::CleanupEvent::Finished {
                    branch,
                    success,
                    message,
                });
        }
        queue
            .lock()
            .unwrap()
            .push_back(crate::model::CleanupEvent::Completed);
    });
}

fn drain_cleanup_events(model: &mut Model) {
    use crate::model::CleanupEvent;
    use screens::cleanup_progress::CleanupProgressMessage;

    let Some(queue) = model.cleanup_events.clone() else {
        return;
    };
    let events: Vec<CleanupEvent> = {
        let mut guard = queue.lock().unwrap();
        std::mem::take(&mut *guard).into_iter().collect()
    };
    for event in events {
        match event {
            CleanupEvent::Started { branch } => {
                update(
                    model,
                    Message::CleanupProgress(CleanupProgressMessage::Started { branch }),
                );
            }
            CleanupEvent::Finished {
                branch,
                success,
                message,
            } => {
                update(
                    model,
                    Message::CleanupProgress(CleanupProgressMessage::Finished {
                        branch,
                        success,
                        message,
                    }),
                );
            }
            CleanupEvent::Completed => {
                update(
                    model,
                    Message::CleanupProgress(CleanupProgressMessage::Completed),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------

fn handle_confirm_message_with(
    model: &mut Model,
    msg: screens::confirm::ConfirmMessage,
    detected_agents: Vec<DetectedAgent>,
) {
    let should_apply_session_conversion = matches!(msg, screens::confirm::ConfirmMessage::Accept)
        && model.confirm.accepted()
        && model.pending_session_conversion.is_some();
    let dismisses_session_conversion = matches!(msg, screens::confirm::ConfirmMessage::Cancel)
        || (matches!(msg, screens::confirm::ConfirmMessage::Accept) && !model.confirm.accepted());
    screens::confirm::update(&mut model.confirm, msg);
    if should_apply_session_conversion {
        if let Some(pending) = model.pending_session_conversion.take() {
            let target_display_name = pending.target_display_name.clone();
            match apply_pending_session_conversion_with(model, pending, detected_agents) {
                Ok(()) => apply_notification(
                    model,
                    Notification::new(
                        Severity::Info,
                        "session",
                        format!("Converted session to {target_display_name}"),
                    ),
                ),
                Err(err) => {
                    apply_notification(model, Notification::new(Severity::Error, "session", err))
                }
            }
        }
    } else if dismisses_session_conversion {
        model.pending_session_conversion = None;
    }
}

fn schedule_startup_version_cache_refresh() {
    schedule_startup_version_cache_refresh_with(
        wizard_version_cache_path(),
        AgentDetector::detect_all,
        |task| {
            let _ = thread::spawn(task);
        },
        schedule_wizard_version_cache_refresh,
    );
}

fn schedule_startup_version_cache_refresh_with<Detect, Spawn, Schedule>(
    cache_path: PathBuf,
    detect_agents: Detect,
    spawn_task: Spawn,
    schedule_refresh: Schedule,
) where
    Detect: FnOnce() -> Vec<DetectedAgent> + Send + 'static,
    Spawn: FnOnce(Box<dyn FnOnce() + Send>),
    Schedule: FnOnce(PathBuf, Vec<AgentId>) + Send + 'static,
{
    if STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    spawn_task(Box::new(move || {
        let cache = VersionCache::load(&cache_path);
        let (_, refresh_targets) = build_wizard_agent_options(detect_agents(), &cache);
        STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.store(false, Ordering::Release);
        if refresh_targets.is_empty() {
            return;
        }
        schedule_refresh(cache_path, refresh_targets);
    }));
}

fn prepare_wizard_startup(
    repo_path: &Path,
    spec_context: Option<screens::wizard::SpecContext>,
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
) -> (screens::wizard::WizardState, Vec<AgentId>) {
    prepare_wizard_startup_with_issue_cache_root(
        repo_path,
        spec_context,
        None,
        detected_agents,
        cache,
        default_issue_cache_root(repo_path),
    )
}

fn prepare_wizard_startup_with_issue_cache_root(
    repo_path: &Path,
    spec_context: Option<screens::wizard::SpecContext>,
    initial_issue_number: Option<u64>,
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
    issue_cache_root: PathBuf,
) -> (screens::wizard::WizardState, Vec<AgentId>) {
    let branch_name = spec_context
        .as_ref()
        .and_then(|ctx| ctx.branch_seed())
        .unwrap_or_default();
    let starts_new_branch = spec_context.is_some() || initial_issue_number.is_some();
    let (cached_issues, issue_load_error) = load_cached_wizard_issues(&issue_cache_root);

    let mut wizard = screens::wizard::WizardState {
        step: if starts_new_branch {
            screens::wizard::WizardStep::BranchTypeSelect
        } else {
            screens::wizard::WizardStep::BranchAction
        },
        is_new_branch: starts_new_branch,
        gh_cli_available: gwt_core::process::command_exists("gh"),
        ai_enabled: false,
        branch_name,
        issue_id: initial_issue_number
            .map(|number| number.to_string())
            .unwrap_or_default(),
        issue_picker: screens::wizard::IssuePickerState {
            issues: cached_issues,
            search_query: String::new(),
            search_active: false,
            load_error: issue_load_error,
        },
        spec_context,
        ..Default::default()
    };
    apply_wizard_docker_context(&mut wizard, repo_path);

    let (agents, refresh_targets) = build_wizard_agent_options(detected_agents, cache);
    if !agents.is_empty() {
        screens::wizard::update(
            &mut wizard,
            screens::wizard::WizardMessage::SetAgents(agents),
        );
    }

    (wizard, refresh_targets)
}

fn apply_wizard_docker_context(wizard: &mut screens::wizard::WizardState, project_root: &Path) {
    wizard.docker_context = detect_wizard_docker_context(project_root);
    wizard.runtime_target = if wizard.docker_context.is_some() {
        LaunchRuntimeTarget::Docker
    } else {
        LaunchRuntimeTarget::Host
    };
    if wizard.docker_service.is_none() {
        wizard.docker_service = wizard
            .docker_context
            .as_ref()
            .and_then(|context| context.suggested_service.clone());
    }
    sync_wizard_docker_status(wizard, project_root);
}

fn sync_wizard_docker_status(wizard: &mut screens::wizard::WizardState, project_root: &Path) {
    let status = detect_wizard_docker_service_status(wizard, project_root);
    wizard.docker_service_status = status;
    if !wizard_docker_lifecycle_supported(status, wizard.docker_lifecycle_intent) {
        wizard.docker_lifecycle_intent = default_docker_lifecycle_intent(status);
    }
}

fn detect_wizard_docker_service_status(
    wizard: &screens::wizard::WizardState,
    project_root: &Path,
) -> gwt_docker::ComposeServiceStatus {
    if wizard.runtime_target != LaunchRuntimeTarget::Docker {
        return gwt_docker::ComposeServiceStatus::NotFound;
    }

    let Some(service) = wizard.docker_service.as_deref().or_else(|| {
        wizard
            .docker_context
            .as_ref()
            .and_then(|context| context.suggested_service.as_deref())
    }) else {
        return gwt_docker::ComposeServiceStatus::NotFound;
    };

    let files = gwt_docker::detect_docker_files(project_root);
    let Some(compose_file) = docker_compose_file_for_launch(project_root, &files)
        .ok()
        .flatten()
    else {
        return gwt_docker::ComposeServiceStatus::NotFound;
    };

    gwt_docker::compose_service_status(&compose_file, service)
        .unwrap_or(gwt_docker::ComposeServiceStatus::NotFound)
}

fn default_docker_lifecycle_intent(
    status: gwt_docker::ComposeServiceStatus,
) -> gwt_agent::DockerLifecycleIntent {
    match status {
        gwt_docker::ComposeServiceStatus::Running => gwt_agent::DockerLifecycleIntent::Connect,
        gwt_docker::ComposeServiceStatus::Stopped | gwt_docker::ComposeServiceStatus::Exited => {
            gwt_agent::DockerLifecycleIntent::Start
        }
        gwt_docker::ComposeServiceStatus::NotFound => {
            gwt_agent::DockerLifecycleIntent::CreateAndStart
        }
    }
}

fn wizard_docker_lifecycle_supported(
    status: gwt_docker::ComposeServiceStatus,
    intent: gwt_agent::DockerLifecycleIntent,
) -> bool {
    match status {
        gwt_docker::ComposeServiceStatus::Running => matches!(
            intent,
            gwt_agent::DockerLifecycleIntent::Connect
                | gwt_agent::DockerLifecycleIntent::Restart
                | gwt_agent::DockerLifecycleIntent::Recreate
        ),
        gwt_docker::ComposeServiceStatus::Stopped | gwt_docker::ComposeServiceStatus::Exited => {
            matches!(
                intent,
                gwt_agent::DockerLifecycleIntent::Start
                    | gwt_agent::DockerLifecycleIntent::Recreate
            )
        }
        gwt_docker::ComposeServiceStatus::NotFound => {
            matches!(intent, gwt_agent::DockerLifecycleIntent::CreateAndStart)
        }
    }
}

fn detect_wizard_docker_context(
    project_root: &Path,
) -> Option<screens::wizard::DockerWizardContext> {
    let files = gwt_docker::detect_docker_files(project_root);

    let compose_file = docker_compose_file_for_launch(project_root, &files)
        .ok()
        .flatten()?;
    let services = gwt_docker::parse_compose_file(&compose_file).ok()?;
    if services.is_empty() {
        return None;
    }

    let suggested_service = docker_devcontainer_defaults(project_root, &files)
        .and_then(|defaults| defaults.service)
        .or_else(|| services.first().map(|service| service.name.clone()));

    Some(screens::wizard::DockerWizardContext {
        services: services.into_iter().map(|service| service.name).collect(),
        suggested_service,
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IssueBranchLinkStore {
    branches: HashMap<String, u64>,
}

fn default_issue_cache_root(repo_path: &std::path::Path) -> PathBuf {
    crate::issue_cache::issue_cache_root_for_repo_path_or_detached(repo_path)
}

fn default_issue_linkage_store_path(repo_path: &std::path::Path) -> Option<PathBuf> {
    let repo_hash = crate::index_worker::detect_repo_hash(repo_path)?;
    Some(
        gwt_cache_dir()
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str())),
    )
}

fn handle_issues_message(model: &mut Model, msg: screens::issues::IssuesMessage) {
    handle_issues_message_with_paths(
        model,
        msg,
        default_issue_cache_root(&model.repo_path),
        default_issue_linkage_store_path(&model.repo_path),
    );
}

fn handle_issues_message_with_paths(
    model: &mut Model,
    msg: screens::issues::IssuesMessage,
    issue_cache_root: PathBuf,
    linkage_store_path: Option<PathBuf>,
) {
    if matches!(msg, screens::issues::IssuesMessage::Refresh) {
        reload_cached_issues_with_paths(model, issue_cache_root, linkage_store_path);
        return;
    }

    screens::issues::update(&mut model.issues, msg);
}

fn reload_cached_issues(model: &mut Model) {
    reload_cached_issues_with_paths(
        model,
        default_issue_cache_root(&model.repo_path),
        default_issue_linkage_store_path(&model.repo_path),
    );
}

fn reload_cached_issues_with_paths(
    model: &mut Model,
    issue_cache_root: PathBuf,
    linkage_store_path: Option<PathBuf>,
) {
    let (issues, issue_load_error) =
        load_cached_issues_with_linkage(&issue_cache_root, linkage_store_path.as_deref());
    screens::issues::update(
        &mut model.issues,
        screens::issues::IssuesMessage::SetIssues(issues),
    );
    model.issues.last_error = issue_load_error;
    if model.issues.selected_issue().is_none() {
        model.issues.detail_view = false;
    }
}

fn load_cached_wizard_issues(
    cache_root: &std::path::Path,
) -> (Vec<screens::issues::IssueItem>, Option<String>) {
    load_cached_issues_with_linkage(cache_root, None)
}

fn load_cached_issues_with_linkage(
    cache_root: &std::path::Path,
    linkage_store_path: Option<&std::path::Path>,
) -> (Vec<screens::issues::IssueItem>, Option<String>) {
    let linked_branches_by_issue = load_issue_linkage_map(linkage_store_path);
    let dir = match std::fs::read_dir(cache_root) {
        Ok(dir) => dir,
        Err(err) => return (Vec::new(), Some(err.to_string())),
    };

    let mut issues = Vec::new();
    for entry in dir.flatten() {
        if !entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(number) = name.parse::<u32>() else {
            continue;
        };
        let meta_path = entry.path().join("meta.json");
        let Ok(meta_bytes) = std::fs::read(&meta_path) else {
            continue;
        };
        let Ok(meta): Result<serde_json::Value, _> = serde_json::from_slice(&meta_bytes) else {
            continue;
        };
        let title = meta
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let state = meta
            .get("state")
            .and_then(|value| value.as_str())
            .unwrap_or("open")
            .to_string();
        let labels = meta
            .get("labels")
            .and_then(|value| value.as_array())
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let body = std::fs::read_to_string(entry.path().join("body.md")).unwrap_or_default();
        let linked_branches = linked_branches_by_issue
            .get(&number)
            .cloned()
            .unwrap_or_default();

        issues.push(screens::issues::IssueItem {
            number,
            title,
            state,
            labels,
            body,
            linked_branches,
        });
    }

    issues.sort_by(|left, right| right.number.cmp(&left.number));
    (issues, None)
}

fn load_issue_linkage_map(
    linkage_store_path: Option<&std::path::Path>,
) -> HashMap<u32, Vec<String>> {
    let mut by_issue: HashMap<u32, Vec<String>> = HashMap::new();
    let Some(store_path) = linkage_store_path else {
        return by_issue;
    };
    let store = match read_issue_linkage_store(store_path) {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!("issue linkage store read failed: {err}");
            return by_issue;
        }
    };

    for (branch_name, issue_number) in store.branches {
        let Ok(issue_number) = u32::try_from(issue_number) else {
            continue;
        };
        by_issue.entry(issue_number).or_default().push(branch_name);
    }

    for branches in by_issue.values_mut() {
        branches.sort();
    }

    by_issue
}

fn persist_issue_linkage(repo_path: &std::path::Path, config: &LaunchConfig) -> Result<(), String> {
    let Some(issue_number) = config.linked_issue_number else {
        return Ok(());
    };
    let Some(branch_name) = config.branch.as_deref() else {
        return Ok(());
    };
    let Some(store_path) = default_issue_linkage_store_path(repo_path) else {
        return Ok(());
    };
    persist_issue_linkage_at_path(&store_path, issue_number, branch_name)
}

fn persist_issue_linkage_at_path(
    store_path: &std::path::Path,
    issue_number: u64,
    branch_name: &str,
) -> Result<(), String> {
    if branch_name.trim().is_empty() {
        return Ok(());
    }

    let mut store = read_issue_linkage_store(store_path)?;
    store.branches.insert(branch_name.to_string(), issue_number);

    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("create issue linkage store dir: {err}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&store)
        .map_err(|err| format!("serialize issue linkage store: {err}"))?;
    std::fs::write(store_path, bytes).map_err(|err| format!("write issue linkage store: {err}"))
}

fn read_issue_linkage_store(store_path: &std::path::Path) -> Result<IssueBranchLinkStore, String> {
    match std::fs::read(store_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|err| format!("parse issue linkage store {}: {err}", store_path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(IssueBranchLinkStore::default())
        }
        Err(err) => Err(format!(
            "read issue linkage store {}: {err}",
            store_path.display()
        )),
    }
}
/// All builtin agent IDs in display order.
const BUILTIN_AGENTS: [AgentId; 4] = [
    AgentId::ClaudeCode,
    AgentId::Codex,
    AgentId::Gemini,
    AgentId::Copilot,
];

fn build_wizard_agent_options(
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
) -> (Vec<screens::wizard::AgentOption>, Vec<AgentId>) {
    let custom_agents = load_custom_agents();
    build_wizard_agent_options_with_custom_agents(detected_agents, cache, &custom_agents)
}

fn build_wizard_agent_options_with_custom_agents(
    detected_agents: Vec<DetectedAgent>,
    cache: &VersionCache,
    custom_agents: &[CustomCodingAgent],
) -> (Vec<screens::wizard::AgentOption>, Vec<AgentId>) {
    let mut refresh_targets = Vec::new();
    let mut options = Vec::new();

    // Always list all builtin agents (installed or not), like old TUI
    for builtin_id in &BUILTIN_AGENTS {
        let detected = detected_agents.iter().find(|d| &d.agent_id == builtin_id);
        let available = detected.is_some();
        let installed_version = detected.and_then(|d| d.version.clone());

        let cached_versions = cached_agent_versions(cache, builtin_id);
        let cache_refreshable = builtin_id.package_name().is_some();
        let cache_outdated = cache_refreshable && cache.needs_refresh(builtin_id);
        if cache_outdated {
            refresh_targets.push(builtin_id.clone());
        }

        options.push(screens::wizard::AgentOption {
            id: builtin_id.command().to_string(),
            name: builtin_id.display_name().to_string(),
            available,
            installed_version,
            versions: cached_versions,
            cache_outdated,
        });
    }

    for custom_agent in custom_agents {
        options.push(screens::wizard::AgentOption {
            id: custom_agent.id.clone(),
            name: custom_agent.display_name.clone(),
            available: custom_agent_available(custom_agent),
            installed_version: None,
            versions: Vec::new(),
            cache_outdated: false,
        });
    }

    (options, refresh_targets)
}

fn custom_agent_available(agent: &CustomCodingAgent) -> bool {
    match agent.agent_type {
        CustomAgentType::Command => gwt_core::process::command_exists(&agent.command),
        CustomAgentType::Path => Path::new(&agent.command).is_file(),
        CustomAgentType::Bunx => {
            gwt_core::process::command_exists("bunx") || gwt_core::process::command_exists("npx")
        }
    }
}

fn cached_agent_versions(cache: &VersionCache, agent_id: &AgentId) -> Vec<String> {
    let key = version_cache_key(agent_id);
    cache
        .entries
        .get(&key)
        .map(|entry| entry.versions.clone())
        .unwrap_or_default()
}

fn version_cache_key(agent_id: &AgentId) -> String {
    match agent_id {
        AgentId::ClaudeCode => "claude-code".to_string(),
        AgentId::Codex => "codex".to_string(),
        AgentId::Gemini => "gemini".to_string(),
        AgentId::OpenCode => "opencode".to_string(),
        AgentId::Copilot => "copilot".to_string(),
        AgentId::Custom(name) => format!("custom-{name}"),
    }
}

fn wizard_version_cache_path() -> PathBuf {
    gwt_cache_dir().join("agent-versions.json")
}

fn schedule_wizard_version_cache_refresh(cache_path: PathBuf, refresh_targets: Vec<AgentId>) {
    schedule_wizard_version_cache_refresh_with(
        cache_path,
        refresh_targets,
        |task| {
            let _ = thread::spawn(task);
        },
        run_wizard_version_cache_refresh,
    );
}

fn schedule_wizard_version_cache_refresh_with<Spawn, Refresh>(
    cache_path: PathBuf,
    refresh_targets: Vec<AgentId>,
    spawn_task: Spawn,
    refresh_cache: Refresh,
) where
    Spawn: FnOnce(Box<dyn FnOnce() + Send>),
    Refresh: FnOnce(PathBuf, Vec<AgentId>) + Send + 'static,
{
    if refresh_targets.is_empty() {
        return;
    }

    if WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    spawn_task(Box::new(move || {
        refresh_cache(cache_path, refresh_targets);
        WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.store(false, Ordering::Release);
    }));
}

fn run_wizard_version_cache_refresh(cache_path: PathBuf, refresh_targets: Vec<AgentId>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();

    if let Ok(runtime) = runtime {
        runtime.block_on(async move {
            let mut cache = VersionCache::load(&cache_path);
            let mut changed = false;

            for agent_id in refresh_targets {
                if !cache.needs_refresh(&agent_id) {
                    continue;
                }

                if let Ok(Some(_versions)) = cache.refresh(&agent_id).await {
                    changed = true;
                }
            }

            if changed {
                let _ = cache.save(&cache_path);
            }
        });
    }
}

fn handle_paste_input(model: &mut Model, text: String) {
    let bracketed_paste_enabled = model
        .active_session_tab()
        .map(|session| screen_requests_bracketed_paste(session.vt.screen()))
        .unwrap_or(false);

    if let Some(bytes) = build_paste_input_bytes(&text, bracketed_paste_enabled) {
        push_input_to_active_session(model, bytes);
    }
}

fn apply_pending_session_conversion_with(
    model: &mut Model,
    pending: PendingSessionConversion,
    detected_agents: Vec<DetectedAgent>,
) -> Result<(), String> {
    let original_tab_type = model
        .sessions
        .get(pending.session_index)
        .map(|session| session.tab_type.clone())
        .ok_or_else(|| format!("Session index {} is out of bounds", pending.session_index))?;

    if !matches!(original_tab_type, SessionTabType::Agent { .. }) {
        return Err("Active session is not an agent session".to_string());
    }

    let detected = detected_agents
        .into_iter()
        .find(|candidate| candidate.agent_id.command() == pending.target_agent_id)
        .ok_or_else(|| {
            format!(
                "Target agent `{}` is not available",
                pending.target_agent_id
            )
        })?;

    let session = model
        .sessions
        .get_mut(pending.session_index)
        .ok_or_else(|| format!("Session index {} is out of bounds", pending.session_index))?;
    session.name = pending.target_display_name;
    session.tab_type = SessionTabType::Agent {
        agent_id: detected.agent_id.command().to_string(),
        color: tui_agent_color(detected.agent_id.default_color()),
    };
    session
        .vt
        .set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

    Ok(())
}

fn tui_agent_color(color: gwt_agent::AgentColor) -> crate::model::AgentColor {
    match color {
        gwt_agent::AgentColor::Green => crate::model::AgentColor::Green,
        gwt_agent::AgentColor::Blue => crate::model::AgentColor::Blue,
        gwt_agent::AgentColor::Cyan => crate::model::AgentColor::Cyan,
        gwt_agent::AgentColor::Yellow => crate::model::AgentColor::Yellow,
        gwt_agent::AgentColor::Magenta => crate::model::AgentColor::Magenta,
        gwt_agent::AgentColor::Gray => crate::model::AgentColor::Gray,
    }
}

/// Map `AgentColor` to a ratatui `Color` for rendering.
fn agent_color_to_ratatui(color: crate::model::AgentColor) -> Color {
    match color {
        crate::model::AgentColor::Green => Color::Green,
        crate::model::AgentColor::Blue => Color::Blue,
        crate::model::AgentColor::Cyan => Color::Cyan,
        crate::model::AgentColor::Yellow => Color::Yellow,
        crate::model::AgentColor::Magenta => Color::Magenta,
        crate::model::AgentColor::Gray => Color::Gray,
    }
}

/// Check if the active management screen is in a text input mode (search, edit).
fn is_in_text_input_mode(model: &Model) -> bool {
    match model.management_tab {
        ManagementTab::Branches => model.branches.search_active,
        ManagementTab::Issues => model.issues.search_active,
        ManagementTab::Settings => model.settings.editing,
        _ => false,
    }
}

fn workspace_main_area(model: &Model) -> Option<Rect> {
    if model.active_layer == ActiveLayer::Initialization {
        return None;
    }

    let (width, height) = model.terminal_size;
    if width == 0 || height == 0 {
        return None;
    }

    Some(Rect::new(0, 0, width, height.saturating_sub(1)))
}

fn visible_management_area(model: &Model) -> Option<Rect> {
    if model.active_layer != ActiveLayer::Management {
        return None;
    }
    Some(management_split(workspace_main_area(model)?)[0])
}

fn visible_session_area(model: &Model) -> Option<Rect> {
    let main_area = workspace_main_area(model)?;
    Some(if model.active_layer == ActiveLayer::Management {
        management_split(main_area)[1]
    } else {
        main_area
    })
}

fn mouse_hits_rect(mouse: MouseEvent, rect: Rect) -> bool {
    mouse.column >= rect.x
        && mouse.column < rect.right()
        && mouse.row >= rect.y
        && mouse.row < rect.bottom()
}

fn title_hit_label_index(
    title: &Line<'_>,
    labels: &[&str],
    area: Rect,
    mouse: MouseEvent,
) -> Option<usize> {
    if mouse.row != area.y {
        return None;
    }

    let mut x = area.x.saturating_add(1);
    for span in &title.spans {
        let content = span.content.as_ref();
        let width = content.chars().count() as u16;
        if mouse.column >= x && mouse.column < x.saturating_add(width) {
            let trimmed = content.trim();
            return labels.iter().position(|label| *label == trimmed);
        }
        x = x.saturating_add(width);
    }

    None
}

/// Detect which session tab label was clicked in a `build_session_title` Line.
///
/// Works by counting non-separator (`│`) spans — each corresponds to one
/// session in `model.sessions` order.  Returns `None` when the title is
/// compact (single span), when only one session exists, or when the click
/// falls outside any label span.
fn session_title_hit_tab_index(title: &Line<'_>, area: Rect, mouse: MouseEvent) -> Option<usize> {
    if mouse.row != area.y || title.spans.len() <= 1 {
        return None;
    }

    let mut x = area.x.saturating_add(1);
    let mut session_idx = 0;
    for span in &title.spans {
        let content = span.content.as_ref();
        let width = content.chars().count() as u16;
        if content.trim() == "│" {
            x = x.saturating_add(width);
            continue;
        }
        if mouse.column >= x && mouse.column < x.saturating_add(width) {
            return Some(session_idx);
        }
        x = x.saturating_add(width);
        session_idx += 1;
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridAxis {
    RowsFirst,
    ColumnsFirst,
}

fn responsive_grid_axis(area: Rect) -> GridAxis {
    if area.height > area.width {
        GridAxis::ColumnsFirst
    } else {
        GridAxis::RowsFirst
    }
}

fn grid_primary_count(count: usize) -> usize {
    (count as f64).sqrt().ceil() as usize
}

fn grid_session_pane_area(area: Rect, count: usize, target_index: usize) -> Option<Rect> {
    if count == 0 || target_index >= count {
        return None;
    }

    let primary = grid_primary_count(count);
    match responsive_grid_axis(area) {
        GridAxis::RowsFirst => {
            let rows = count.div_ceil(primary);
            let row_constraints: Vec<Constraint> = (0..rows)
                .map(|_| Constraint::Ratio(1, rows as u32))
                .collect();
            let row_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(row_constraints)
                .split(area);

            let target_row = target_index / primary;
            let start = target_row * primary;
            let end = (start + primary).min(count);
            let n = end - start;
            let col_constraints: Vec<Constraint> =
                (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
            let col_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(col_constraints)
                .split(row_chunks[target_row]);

            col_chunks.get(target_index - start).copied()
        }
        GridAxis::ColumnsFirst => {
            let cols = count.div_ceil(primary);
            let col_constraints: Vec<Constraint> = (0..cols)
                .map(|_| Constraint::Ratio(1, cols as u32))
                .collect();
            let col_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(col_constraints)
                .split(area);

            let target_col = target_index / primary;
            let start = target_col * primary;
            let end = (start + primary).min(count);
            let n = end - start;
            let row_constraints: Vec<Constraint> =
                (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
            let row_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(row_constraints)
                .split(col_chunks[target_col]);

            row_chunks.get(target_index - start).copied()
        }
    }
}

fn move_grid_session(model: &mut Model, direction: GridSessionDirection) {
    if model.session_layout != SessionLayout::Grid {
        return;
    }

    let Some(session_area) = visible_session_area(model) else {
        return;
    };
    let Some(next_session) = adjacent_grid_session_index(
        session_area,
        model.sessions.len(),
        model.active_session,
        direction,
    ) else {
        return;
    };

    model.active_session = next_session;
}

fn adjacent_grid_session_index(
    area: Rect,
    count: usize,
    active_index: usize,
    direction: GridSessionDirection,
) -> Option<usize> {
    let current_area = grid_session_pane_area(area, count, active_index)?;
    let current_center = rect_center_doubled(current_area);
    let mut aligned_best: Option<(i32, i32, usize)> = None;
    let mut fallback_best: Option<(i32, i32, usize)> = None;

    for candidate_index in 0..count {
        if candidate_index == active_index {
            continue;
        }
        let Some(candidate_area) = grid_session_pane_area(area, count, candidate_index) else {
            continue;
        };
        let candidate_center = rect_center_doubled(candidate_area);
        let Some((primary_distance, secondary_distance)) =
            directional_center_distances(current_center, candidate_center, direction)
        else {
            continue;
        };
        let key = (primary_distance, secondary_distance, candidate_index);
        let overlaps = match direction {
            GridSessionDirection::Left | GridSessionDirection::Right => {
                rects_overlap_vertically(current_area, candidate_area)
            }
            GridSessionDirection::Up | GridSessionDirection::Down => {
                rects_overlap_horizontally(current_area, candidate_area)
            }
        };
        let slot = if overlaps {
            &mut aligned_best
        } else {
            &mut fallback_best
        };
        if slot.as_ref().is_none_or(|best| key < *best) {
            *slot = Some(key);
        }
    }

    aligned_best
        .or(fallback_best)
        .map(|(_, _, candidate_index)| candidate_index)
}

fn rect_center_doubled(area: Rect) -> (i32, i32) {
    (
        i32::from(area.x) * 2 + i32::from(area.width),
        i32::from(area.y) * 2 + i32::from(area.height),
    )
}

fn directional_center_distances(
    current_center: (i32, i32),
    candidate_center: (i32, i32),
    direction: GridSessionDirection,
) -> Option<(i32, i32)> {
    match direction {
        GridSessionDirection::Left if candidate_center.0 < current_center.0 => Some((
            current_center.0 - candidate_center.0,
            (candidate_center.1 - current_center.1).abs(),
        )),
        GridSessionDirection::Right if candidate_center.0 > current_center.0 => Some((
            candidate_center.0 - current_center.0,
            (candidate_center.1 - current_center.1).abs(),
        )),
        GridSessionDirection::Up if candidate_center.1 < current_center.1 => Some((
            current_center.1 - candidate_center.1,
            (candidate_center.0 - current_center.0).abs(),
        )),
        GridSessionDirection::Down if candidate_center.1 > current_center.1 => Some((
            candidate_center.1 - current_center.1,
            (candidate_center.0 - current_center.0).abs(),
        )),
        _ => None,
    }
}

fn rects_overlap_vertically(left: Rect, right: Rect) -> bool {
    left.y < right.bottom() && right.y < left.bottom()
}

fn rects_overlap_horizontally(top: Rect, bottom: Rect) -> bool {
    top.x < bottom.right() && bottom.x < top.right()
}

fn handle_management_mouse_focus(model: &mut Model, mouse: MouseEvent) -> bool {
    let Some(management_area) = visible_management_area(model) else {
        return false;
    };
    if !mouse_hits_rect(mouse, management_area) {
        return false;
    }

    let management_labels: Vec<&str> = ManagementTab::ALL.iter().map(|tab| tab.label()).collect();

    if model.management_tab == ManagementTab::Branches {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(management_area);
        let list_pane = chunks[0];
        let detail_pane = chunks[1];

        if mouse_hits_rect(mouse, list_pane) {
            let title = management_tab_title(model, list_pane.width);
            if let Some(tab_idx) =
                title_hit_label_index(&title, &management_labels, list_pane, mouse)
            {
                update(
                    model,
                    Message::SwitchManagementTab(ManagementTab::ALL[tab_idx]),
                );
                model.active_focus = FocusPane::TabContent;
                return true;
            }

            let list_inner = pane_block(title, false).inner(list_pane);
            let list_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(list_inner);
            let list_area = list_chunks[1];
            if mouse_hits_rect(mouse, list_area) {
                let row = mouse.row.saturating_sub(list_area.y) as usize;
                model.branches.select_filtered_index(row);
            }
            model.active_focus = FocusPane::TabContent;
            return true;
        }

        if mouse_hits_rect(mouse, detail_pane) {
            let detail_title = branch_detail_title(model);
            if let Some(section_idx) = title_hit_label_index(
                &detail_title,
                screens::branches::detail_section_labels(),
                detail_pane,
                mouse,
            ) {
                model.branches.detail_section = section_idx;
            }
            model.active_focus = FocusPane::BranchDetail;
            return true;
        }

        return false;
    }

    let title = management_tab_title(model, management_area.width);
    if let Some(tab_idx) = title_hit_label_index(&title, &management_labels, management_area, mouse)
    {
        update(
            model,
            Message::SwitchManagementTab(ManagementTab::ALL[tab_idx]),
        );
        model.active_focus = FocusPane::TabContent;
        return true;
    }

    if model.management_tab == ManagementTab::Profiles
        && model.profiles.mode == screens::profiles::ProfileMode::List
    {
        let inner = pane_block(title, false).inner(management_area);
        let layout = screens::profiles::layout_areas(inner);

        if mouse_hits_rect(mouse, layout.list) {
            model.profiles.focus = screens::profiles::ProfilesFocus::ProfileList;
            if mouse_hits_rect(mouse, layout.list_content) {
                let row = mouse.row.saturating_sub(layout.list_content.y) as usize;
                if row < model.profiles.profiles.len() {
                    model.profiles.selected = row;
                    model.profiles.clamp_selection();
                }
            }
            model.active_focus = FocusPane::TabContent;
            return true;
        }

        if mouse_hits_rect(mouse, layout.env) {
            model.profiles.focus = screens::profiles::ProfilesFocus::Environment;
            if mouse_hits_rect(mouse, layout.env_content) {
                if let Some(profile) = model.profiles.selected_profile() {
                    let row = mouse.row.saturating_sub(layout.env_content.y) as usize;
                    if row < profile.env_rows.len() {
                        model.profiles.env_selected = row;
                    }
                }
            }
            model.active_focus = FocusPane::TabContent;
            return true;
        }
    }
    model.active_focus = FocusPane::TabContent;
    true
}

fn handle_session_mouse_focus(model: &mut Model, mouse: MouseEvent) -> bool {
    let Some(session_area) = visible_session_area(model) else {
        return false;
    };

    match model.session_layout {
        SessionLayout::Tab => {
            if !mouse_hits_rect(mouse, session_area) {
                return false;
            }
            let title = build_session_title(model, session_area.width);
            if let Some(tab_idx) = session_title_hit_tab_index(&title, session_area, mouse) {
                model.active_session = tab_idx;
                model.active_focus = FocusPane::Terminal;
                return true;
            }
            if active_session_content_area(model).is_some_and(|area| mouse_hits_rect(mouse, area)) {
                return false;
            }
            model.active_focus = FocusPane::Terminal;
            true
        }
        SessionLayout::Grid => {
            for session_idx in 0..model.sessions.len() {
                let Some(pane_area) =
                    grid_session_pane_area(session_area, model.sessions.len(), session_idx)
                else {
                    continue;
                };
                if !mouse_hits_rect(mouse, pane_area) {
                    continue;
                }
                if session_idx == model.active_session
                    && active_session_content_area(model)
                        .is_some_and(|area| mouse_hits_rect(mouse, area))
                {
                    return false;
                }
                model.active_session = session_idx;
                model.active_focus = FocusPane::Terminal;
                return true;
            }
            false
        }
    }
}

fn handle_mouse_input(model: &mut Model, mouse: MouseEvent) {
    // FR-018g: cleanup modals capture all input, including mouse events —
    // swallow here so clicks cannot fall through to the panes behind a
    // blocking cleanup dialog.
    if model.cleanup_confirm.visible || model.cleanup_progress.visible {
        return;
    }
    if let Err(err) = handle_mouse_input_with_tools(model, mouse, open_url, |text| {
        gwt_clipboard::ClipboardText::set_text(text).map_err(|err| err.to_string())
    }) {
        model.error_queue.push_back(
            Notification::new(Severity::Error, "terminal", "Mouse interaction failed")
                .with_detail(err),
        );
    }
}

#[cfg(test)]
fn handle_mouse_input_with<F>(
    model: &mut Model,
    mouse: MouseEvent,
    mut opener: F,
) -> Result<bool, String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    handle_mouse_input_with_tools(model, mouse, |url| opener(url), |_| Ok(()))
}

fn handle_mouse_input_with_tools<F, G>(
    model: &mut Model,
    mouse: MouseEvent,
    mut opener: F,
    mut clipboard_writer: G,
) -> Result<bool, String>
where
    F: FnMut(&str) -> Result<(), String>,
    G: FnMut(&str) -> Result<(), String>,
{
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        && (handle_management_mouse_focus(model, mouse) || handle_session_mouse_focus(model, mouse))
    {
        return Ok(true);
    }

    let hits_active_session = mouse_hits_active_session(model, mouse);
    crate::scroll_debug::log_lazy(|| {
        format!(
        "event=mouse kind={:?} column={} row={} modifiers={:?} hits_active_session={} active_focus={:?} active_layer={:?}",
        mouse.kind,
        mouse.column,
        mouse.row,
        mouse.modifiers,
        hits_active_session,
        model.active_focus,
        model.active_layer,
    )
    });

    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        && mouse.modifiers.contains(KeyModifiers::CONTROL)
    {
        let Some(url) = url_at_mouse_position(model, mouse) else {
            return Ok(false);
        };
        opener(&url)?;
        return Ok(true);
    }

    if !hits_active_session {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && scroll_target_session_index(model, mouse).is_some()
        {
            model.active_focus = FocusPane::Terminal;
        } else if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Right)) {
            model.terminal_trackpad_scroll_row = None;
        } else {
            return Ok(false);
        }
    }

    if matches!(
        mouse.kind,
        MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::Down(MouseButton::Right)
            | MouseEventKind::Drag(MouseButton::Right)
            | MouseEventKind::Up(MouseButton::Right)
            | MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left)
    ) {
        model.active_focus = FocusPane::Terminal;
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let Some(target_session) = scroll_target_session_index(model, mouse) else {
                return Ok(false);
            };
            let routing = session_scroll_routing_for_index(model, target_session);
            log_session_scroll_routing(model, target_session, routing, 1, "wheel");
            match routing {
                ScrollInputRouting::LocalViewport => {
                    Ok(scroll_session_by_rows(model, target_session, 1))
                }
                ScrollInputRouting::PtyMouse => {
                    Ok(queue_session_mouse_scroll(model, target_session, mouse, 1))
                }
            }
        }
        MouseEventKind::ScrollDown => {
            let Some(target_session) = scroll_target_session_index(model, mouse) else {
                return Ok(false);
            };
            let routing = session_scroll_routing_for_index(model, target_session);
            log_session_scroll_routing(model, target_session, routing, -1, "wheel");
            match routing {
                ScrollInputRouting::LocalViewport => {
                    Ok(scroll_session_by_rows(model, target_session, -1))
                }
                ScrollInputRouting::PtyMouse => {
                    Ok(queue_session_mouse_scroll(model, target_session, mouse, -1))
                }
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            model.terminal_trackpad_scroll_row = Some(mouse.row);
            Ok(false)
        }
        MouseEventKind::Drag(MouseButton::Right) => {
            let Some(previous_row) = model.terminal_trackpad_scroll_row.replace(mouse.row) else {
                return Ok(false);
            };
            let delta_rows = i32::from(mouse.row) - i32::from(previous_row);
            if delta_rows == 0 {
                return Ok(false);
            }
            let delta_rows = delta_rows.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            let routing = active_session_scroll_routing(model);
            log_session_scroll_routing(
                model,
                model.active_session,
                routing,
                delta_rows,
                "trackpad_drag",
            );
            match routing {
                ScrollInputRouting::LocalViewport => Ok(scroll_session_by_rows(
                    model,
                    model.active_session,
                    delta_rows,
                )),
                ScrollInputRouting::PtyMouse => Ok(queue_session_mouse_scroll(
                    model,
                    model.active_session,
                    mouse,
                    delta_rows,
                )),
            }
        }
        MouseEventKind::Up(MouseButton::Right) => {
            model.terminal_trackpad_scroll_row = None;
            Ok(false)
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if should_forward_click_to_pty(model, mouse) {
                return Ok(queue_session_mouse_click(
                    model,
                    model.active_session,
                    mouse,
                ));
            }
            let Some(cell) = mouse_terminal_cell(model, mouse) else {
                return Ok(false);
            };
            if let Some(session) = model.active_session_tab_mut() {
                session.vt.begin_selection(cell);
                return Ok(true);
            }
            Ok(false)
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if should_forward_click_to_pty(model, mouse) {
                return Ok(queue_session_mouse_click(
                    model,
                    model.active_session,
                    mouse,
                ));
            }
            let Some(cell) = mouse_terminal_cell(model, mouse) else {
                return Ok(false);
            };
            if let Some(session) = model.active_session_tab_mut() {
                session.vt.update_selection(cell);
                return Ok(true);
            }
            Ok(false)
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if should_forward_click_to_pty(model, mouse) {
                return Ok(queue_session_mouse_click(
                    model,
                    model.active_session,
                    mouse,
                ));
            }
            let Some(cell) = mouse_terminal_cell(model, mouse) else {
                return Ok(false);
            };
            let selection_text = if let Some(session) = model.active_session_tab_mut() {
                session.vt.update_selection(cell);
                selected_text(session)
            } else {
                None
            };
            if let Some(text) = selection_text.filter(|text| !text.is_empty()) {
                clipboard_writer(&text)?;
                return Ok(true);
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Decide whether a left-button mouse event should be forwarded to the PTY
/// (because the guest program has enabled SGR mouse reporting) or handled as
/// local text selection.  Shift held down acts as a bypass so users can still
/// select text in guests that captured the mouse, matching the behavior of
/// common terminal emulators (iTerm2, WezTerm, kitty, ...).
fn should_forward_click_to_pty(model: &Model, mouse: MouseEvent) -> bool {
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        return false;
    }
    matches!(
        session_scroll_routing_for_index(model, model.active_session),
        ScrollInputRouting::PtyMouse
    )
}

fn url_at_mouse_position(model: &Model, mouse: MouseEvent) -> Option<String> {
    let area = active_session_text_area(model)?;
    if mouse.column < area.x
        || mouse.column >= area.right()
        || mouse.row < area.y
        || mouse.row >= area.bottom()
    {
        return None;
    }

    let session = model.active_session_tab()?;
    session.vt.with_visible_screen(|screen| {
        let render_surface = session_render_surface(session, screen, area);
        render_surface
            .url_regions
            .into_iter()
            .find(|region| {
                let row = area.y + region.row;
                let start_col = area.x + region.start_col;
                let end_col = area.x + region.end_col;
                mouse.row == row && mouse.column >= start_col && mouse.column <= end_col
            })
            .map(|region| region.url)
    })
}

fn scroll_target_session_index(model: &Model, mouse: MouseEvent) -> Option<usize> {
    match model.session_layout {
        SessionLayout::Tab => {
            mouse_hits_active_session(model, mouse).then_some(model.active_session)
        }
        SessionLayout::Grid => {
            let session_area = visible_session_area(model)?;
            (0..model.sessions.len()).find(|&session_idx| {
                grid_session_pane_area(session_area, model.sessions.len(), session_idx)
                    .is_some_and(|pane_area| mouse_hits_rect(mouse, pane_area))
            })
        }
    }
}

fn mouse_hits_active_session(model: &Model, mouse: MouseEvent) -> bool {
    let Some(area) = active_session_content_area(model) else {
        return false;
    };
    mouse.column >= area.x
        && mouse.column < area.right()
        && mouse.row >= area.y
        && mouse.row < area.bottom()
}

fn mouse_terminal_cell(model: &Model, mouse: MouseEvent) -> Option<TerminalCell> {
    let area = active_session_text_area(model)?;
    if mouse.column < area.x
        || mouse.column >= area.right()
        || mouse.row < area.y
        || mouse.row >= area.bottom()
    {
        return None;
    }
    Some(TerminalCell {
        row: mouse.row.saturating_sub(area.y),
        col: mouse.column.saturating_sub(area.x),
    })
}

fn session_text_area_for_index(model: &Model, session_idx: usize) -> Option<Rect> {
    let session_area = visible_session_area(model)?;
    let area = session_content_area_for_index(model, session_area, session_idx)?;
    let session = model.sessions.get(session_idx)?;
    Some(session_text_area(session, area))
}

fn clamped_mouse_terminal_cell_for_index(
    model: &Model,
    mouse: MouseEvent,
    session_idx: usize,
) -> Option<TerminalCell> {
    let area = session_text_area_for_index(model, session_idx)?;
    if area.width == 0 || area.height == 0 || mouse.row < area.y || mouse.row >= area.bottom() {
        return None;
    }

    let max_col = area.right().saturating_sub(1);
    let clamped_col = mouse.column.clamp(area.x, max_col);
    Some(TerminalCell {
        row: mouse.row.saturating_sub(area.y),
        col: clamped_col.saturating_sub(area.x),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollInputRouting {
    LocalViewport,
    PtyMouse,
}

fn active_session_scroll_routing(model: &Model) -> ScrollInputRouting {
    let Some(session) = model.active_session_tab() else {
        return ScrollInputRouting::LocalViewport;
    };
    session_scroll_routing(session)
}

fn session_scroll_routing_for_index(model: &Model, session_idx: usize) -> ScrollInputRouting {
    let Some(session) = model.sessions.get(session_idx) else {
        return ScrollInputRouting::LocalViewport;
    };
    session_scroll_routing(session)
}

fn session_scroll_routing(session: &crate::model::SessionTab) -> ScrollInputRouting {
    if !matches!(session.tab_type, SessionTabType::Agent { .. }) {
        return ScrollInputRouting::LocalViewport;
    }

    if session.vt.accepts_mouse_scroll_input() {
        return ScrollInputRouting::PtyMouse;
    }

    ScrollInputRouting::LocalViewport
}

fn log_session_scroll_routing(
    model: &Model,
    session_idx: usize,
    routing: ScrollInputRouting,
    delta_rows: i16,
    source: &str,
) {
    let Some(session) = model.sessions.get(session_idx) else {
        return;
    };
    crate::scroll_debug::log_lazy(|| {
        format!(
        "event=scroll_route session={} source={} delta_rows={} routing={:?} alternate_screen={} uses_snapshot_scrollback={} max_scrollback={} snapshot_count={} mouse_reporting={} follow_live={}",
        session.id,
        source,
        delta_rows,
        routing,
        session.vt.screen().alternate_screen(),
        session.vt.uses_snapshot_scrollback(),
        session.vt.max_scrollback(),
        session.vt.snapshot_count(),
        session.vt.accepts_mouse_scroll_input(),
        session.vt.follow_live(),
    )
    });
}

fn push_input_to_session(model: &mut Model, session_id: String, bytes: Vec<u8>) {
    model
        .pending_pty_inputs
        .push_back(crate::model::PendingPtyInput { session_id, bytes });
}

fn reset_session_scrollback_for_input(model: &mut Model, session_idx: usize) {
    let Some(session) = model.sessions.get_mut(session_idx) else {
        return;
    };
    if session.vt.viewing_history() {
        session.vt.clear_selection();
        session.vt.set_follow_live(true);
    }
}

fn queue_session_mouse_scroll(
    model: &mut Model,
    session_idx: usize,
    mouse: MouseEvent,
    delta_rows: i16,
) -> bool {
    if delta_rows == 0 {
        return false;
    }

    let Some(cell) = clamped_mouse_terminal_cell_for_index(model, mouse, session_idx) else {
        return false;
    };

    reset_session_scrollback_for_input(model, session_idx);
    let steps = usize::from(delta_rows.unsigned_abs());
    let code = if delta_rows > 0 { 64 } else { 65 };
    let mut bytes = Vec::with_capacity(steps.saturating_mul(12));
    for _ in 0..steps {
        bytes.extend_from_slice(
            format!(
                "\x1b[<{code};{};{}M",
                cell.col.saturating_add(1),
                cell.row.saturating_add(1)
            )
            .as_bytes(),
        );
    }
    let Some(session_id) = model
        .sessions
        .get(session_idx)
        .map(|session| session.id.clone())
    else {
        return false;
    };
    push_input_to_session(model, session_id, bytes);
    true
}

/// Encode a mouse button event (press / drag / release) as an SGR 1006 report
/// and queue it for the PTY writer.  Mirrors `queue_session_mouse_scroll` but
/// for button events instead of wheel events.
///
/// Returns `true` when a byte sequence was queued, `false` when the event
/// does not map to a supported button (or the coordinate falls outside the
/// session's text area).
fn queue_session_mouse_click(model: &mut Model, session_idx: usize, mouse: MouseEvent) -> bool {
    let Some((base_code, final_byte)) = mouse_click_sgr_code(mouse.kind) else {
        return false;
    };
    let Some(cell) = clamped_mouse_terminal_cell_for_index(model, mouse, session_idx) else {
        return false;
    };

    reset_session_scrollback_for_input(model, session_idx);
    let code = base_code | sgr_modifier_bits(mouse.modifiers);
    let bytes = format!(
        "\x1b[<{code};{};{}{final_byte}",
        cell.col.saturating_add(1),
        cell.row.saturating_add(1),
    )
    .into_bytes();

    let Some(session_id) = model
        .sessions
        .get(session_idx)
        .map(|session| session.id.clone())
    else {
        return false;
    };
    push_input_to_session(model, session_id, bytes);
    true
}

/// Map a `MouseEventKind` to its SGR 1006 base button code and terminating
/// byte (`M` for press/motion, `m` for release).  Returns `None` for events
/// we do not forward (wheel events are encoded by `queue_session_mouse_scroll`
/// and moved events are filtered upstream).
fn mouse_click_sgr_code(kind: MouseEventKind) -> Option<(u16, char)> {
    match kind {
        MouseEventKind::Down(button) => Some((sgr_button_base(button), 'M')),
        MouseEventKind::Drag(button) => Some((sgr_button_base(button) | 32, 'M')),
        MouseEventKind::Up(button) => Some((sgr_button_base(button), 'm')),
        _ => None,
    }
}

fn sgr_button_base(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

fn sgr_modifier_bits(modifiers: KeyModifiers) -> u16 {
    let mut bits = 0u16;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits |= 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits |= 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits |= 16;
    }
    bits
}

fn selected_text(session: &crate::model::SessionTab) -> Option<String> {
    let selection = session.vt.selection()?;
    let (start, end) = normalize_selection(selection);
    session.vt.with_visible_screen(|screen| {
        let end_col = end.col.saturating_add(1).min(screen.size().1);
        Some(screen.contents_between(start.row, start.col, end.row, end_col))
    })
}

fn scroll_session_by_rows(model: &mut Model, session_idx: usize, delta_rows: i16) -> bool {
    let Some(session) = model.sessions.get_mut(session_idx) else {
        return false;
    };

    session.vt.clear_selection();
    let previous_scrollback = session.vt.scrollback();
    let previous_max_scrollback = session.vt.max_scrollback();
    let previous_snapshot_position = session.vt.snapshot_position();
    let previous_snapshot_count = session.vt.snapshot_count();
    let previous_follow_live = session.vt.follow_live();
    let mode = if session.vt.uses_snapshot_scrollback() {
        "snapshot"
    } else {
        "row"
    };
    let changed = session.vt.scroll_viewport_lines(delta_rows);
    if changed {
        crate::scroll_debug::log_lazy(|| {
            format!(
            "event=scroll delta_rows={} session={} mode={} previous_scrollback={} next_scrollback={} max_scrollback={} previous_snapshot_position={} next_snapshot_position={} previous_snapshot_count={} next_snapshot_count={} previous_follow_live={} next_follow_live={}",
            delta_rows,
            session.id,
            mode,
            previous_scrollback,
            session.vt.scrollback(),
            previous_max_scrollback,
            previous_snapshot_position,
            session.vt.snapshot_position(),
            previous_snapshot_count,
            session.vt.snapshot_count(),
            previous_follow_live,
            session.vt.follow_live(),
        )
        });
    }
    changed
}

fn clear_terminal_trackpad_scroll_row_if_context_changed(
    model: &mut Model,
    previous_active_session: usize,
    previous_active_focus: FocusPane,
) {
    if model.active_session != previous_active_session
        || (previous_active_focus == FocusPane::Terminal
            && model.active_focus != FocusPane::Terminal)
    {
        model.terminal_trackpad_scroll_row = None;
    }
}

fn normalize_selection(selection: TerminalSelection) -> (TerminalCell, TerminalCell) {
    if (selection.anchor.row, selection.anchor.col) <= (selection.focus.row, selection.focus.col) {
        (selection.anchor, selection.focus)
    } else {
        (selection.focus, selection.anchor)
    }
}

fn active_session_content_area(model: &Model) -> Option<Rect> {
    if model.active_layer == ActiveLayer::Initialization {
        return None;
    }

    let (width, height) = model.terminal_size;
    if width == 0 || height == 0 {
        return None;
    }

    let size = Rect::new(0, 0, width, height);
    let main_area = Rect {
        height: size.height.saturating_sub(1),
        ..size
    };
    let session_area = if model.active_layer == ActiveLayer::Management {
        management_split(main_area)[1]
    } else {
        main_area
    };

    session_content_area_for_index(model, session_area, model.active_session)
}

fn active_session_text_area(model: &Model) -> Option<Rect> {
    let area = active_session_content_area(model)?;
    let session = model.active_session_tab()?;
    Some(session_text_area(session, area))
}

fn session_text_area(session: &crate::model::SessionTab, area: Rect) -> Rect {
    let _ = session;
    area
}

#[cfg(test)]
fn session_has_scrollbar(session: &crate::model::SessionTab) -> bool {
    let _ = session;
    false
}

fn management_split(area: Rect) -> [Rect; 2] {
    let management_percentage = if area.width >= 120 { 40 } else { 50 };
    let lr = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(management_percentage),
            Constraint::Percentage(100 - management_percentage),
        ])
        .split(area);
    [lr[0], lr[1]]
}

fn session_content_area_for_index(
    model: &Model,
    session_area: Rect,
    session_idx: usize,
) -> Option<Rect> {
    match model.session_layout {
        SessionLayout::Tab => {
            model.sessions.get(session_idx)?;
            Some(
                pane_block(
                    build_session_title(model, session_area.width),
                    model.active_focus == FocusPane::Terminal
                        && session_idx == model.active_session,
                )
                .inner(session_area),
            )
        }
        SessionLayout::Grid => active_grid_session_content_area(model, session_area, session_idx),
    }
}

fn active_grid_session_content_area(model: &Model, area: Rect, session_idx: usize) -> Option<Rect> {
    let count = model.sessions.len();
    if count == 0 || session_idx >= count {
        return None;
    }

    let pane_area = grid_session_pane_area(area, count, session_idx)?;
    let session = model.sessions.get(session_idx)?;
    Some(
        grid_session_block(session_idx, session, session_idx == model.active_session)
            .inner(pane_area),
    )
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };

    command
        .status()
        .map_err(|err| format!("failed to spawn URL opener: {err}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("URL opener exited with status {status}"))
            }
        })
}

fn render_session_surface(
    session: &crate::model::SessionTab,
    frame: &mut Frame,
    area: Rect,
    show_cursor: bool,
) {
    let text_area = session_text_area(session, area);
    session.vt.with_visible_screen(|screen| {
        let render_surface = session_render_surface(session, screen, text_area);
        let render_screen = render_surface.screen(screen);

        if render_screen.contents().trim().is_empty() {
            match &session.tab_type {
                crate::model::SessionTabType::Agent { agent_id, color } => {
                    // Braille spinner driven by elapsed time (~5 fps via 100ms tick)
                    const SPINNER: [char; 6] = [
                        '\u{280B}', '\u{2819}', '\u{2838}', '\u{2834}', '\u{2826}', '\u{2807}',
                    ];
                    let elapsed = session.created_at.elapsed().as_millis() as usize;
                    let ch = SPINNER[(elapsed / 200) % SPINNER.len()];
                    let agent_fg = agent_color_to_ratatui(*color);

                    // Center the startup display vertically
                    let top_pad = area.height.saturating_sub(5) / 2;
                    let mut lines: Vec<Line<'_>> = Vec::new();
                    for _ in 0..top_pad {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{} ", theme::icon::SESSION_AGENT),
                            Style::default().fg(agent_fg),
                        ),
                        Span::styled(
                            session.name.clone(),
                            Style::default().fg(agent_fg).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled(format!("{ch} "), Style::default().fg(agent_fg)),
                        Span::styled(
                            format!("Starting {agent_id}..."),
                            Style::default().fg(theme::color::TEXT_SECONDARY),
                        ),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "Waiting for agent output",
                        Style::default().fg(theme::color::TEXT_DISABLED),
                    )));
                    let paragraph =
                        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center);
                    frame.render_widget(paragraph, text_area);
                }
                _ => {
                    let placeholder = Paragraph::new(format!(
                        "Session: {} ({}x{})",
                        session.name,
                        session.vt.cols(),
                        session.vt.rows()
                    ))
                    .style(Style::default().fg(theme::color::TEXT_DISABLED));
                    frame.render_widget(placeholder, text_area);
                }
            }
        } else {
            crate::renderer::render_vt_screen_with_selection_and_urls(
                render_screen,
                frame.buffer_mut(),
                text_area,
                session.vt.selection(),
                &render_surface.url_regions,
            );
        }

        if show_cursor && !session.vt.viewing_history() && !render_screen.hide_cursor() {
            let (cursor_row, cursor_col) = render_screen.cursor_position();
            let x = text_area.x + cursor_col;
            let y = text_area.y + cursor_row;
            if x < text_area.right() && y < text_area.bottom() {
                frame.set_cursor_position((x, y));
            }
        }
    });
}

struct SessionRenderSurface {
    normalized_parser: Option<vt100::Parser>,
    url_regions: Vec<crate::renderer::UrlRegion>,
}

impl SessionRenderSurface {
    fn screen<'a>(&'a self, fallback: &'a vt100::Screen) -> &'a vt100::Screen {
        self.normalized_parser
            .as_ref()
            .map(vt100::Parser::screen)
            .unwrap_or(fallback)
    }
}

fn session_render_surface(
    session: &crate::model::SessionTab,
    screen: &vt100::Screen,
    text_area: Rect,
) -> SessionRenderSurface {
    let area = Rect::new(0, 0, text_area.width, text_area.height);
    let normalized_parser = match &session.tab_type {
        crate::model::SessionTabType::Agent { agent_id, .. }
            if agent_id == "codex" && session.vt.selection().is_none() =>
        {
            normalized_codex_progress_parser(screen)
        }
        _ => None,
    };
    let url_regions = if let Some(parser) = normalized_parser.as_ref() {
        crate::renderer::collect_url_regions(parser.screen(), area)
    } else {
        session.vt.visible_url_regions(area)
    };
    SessionRenderSurface {
        normalized_parser,
        url_regions,
    }
}

fn normalized_codex_progress_parser(screen: &vt100::Screen) -> Option<vt100::Parser> {
    let (rows, cols) = screen.size();
    let visible_lines: Vec<String> = screen
        .rows(0, cols)
        .map(|line| line.trim_end_matches(' ').to_string())
        .collect();
    let formatted_rows: Vec<Vec<u8>> = screen.rows_formatted(0, cols).collect();
    if visible_lines.is_empty() || visible_lines.len() != formatted_rows.len() {
        return None;
    }
    let (cursor_row, cursor_col) = screen.cursor_position();
    let hide_cursor = screen.hide_cursor();

    let mut keep = vec![true; visible_lines.len()];
    let mut row = 0usize;
    let mut changed = false;
    while row < visible_lines.len() {
        if !is_codex_progress_separator(&visible_lines[row]) {
            row += 1;
            continue;
        }

        let separator_start = row;
        while row < visible_lines.len() && is_codex_progress_separator(&visible_lines[row]) {
            row += 1;
        }
        let separator_end = row;

        let previous_end = separator_start;
        let mut previous_start = previous_end;
        while previous_start > 0 && !is_codex_progress_separator(&visible_lines[previous_start - 1])
        {
            previous_start -= 1;
        }

        let next_start = separator_end;
        let mut next_end = next_start;
        while next_end < visible_lines.len()
            && !is_codex_progress_separator(&visible_lines[next_end])
        {
            next_end += 1;
        }

        let previous_block =
            codex_progress_block_lines(&visible_lines[previous_start..previous_end]);
        let next_block = codex_progress_block_lines(&visible_lines[next_start..next_end]);
        if previous_block
            .as_ref()
            .zip(next_block.as_ref())
            .is_some_and(|(previous, next)| codex_progress_block_repeated(previous, next))
        {
            for keep_row in keep.iter_mut().take(next_start).skip(previous_start) {
                *keep_row = false;
            }
            changed = true;
        }
    }

    if !changed {
        return None;
    }

    let rows_to_keep: Vec<usize> = keep
        .iter()
        .enumerate()
        .filter_map(|(index, keep_row)| keep_row.then_some(index))
        .collect();
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(b"\x1b[2J\x1b[H");
    for (dest_row, src_row) in rows_to_keep.iter().take(rows as usize).enumerate() {
        let mut positioned_row = format!("\x1b[{};1H\x1b[K", dest_row + 1).into_bytes();
        positioned_row.extend_from_slice(&formatted_rows[*src_row]);
        parser.process(&positioned_row);
    }
    if rows > 0 && cols > 0 {
        let cursor_row = usize::from(cursor_row).min(keep.len().saturating_sub(1));
        let adjusted_cursor_row = keep
            .iter()
            .take(cursor_row.saturating_add(1))
            .filter(|keep_row| **keep_row)
            .count()
            .saturating_sub(1)
            .min(rows_to_keep.len().saturating_sub(1));
        let adjusted_cursor_col = usize::from(cursor_col)
            .min(usize::from(cols).saturating_sub(1))
            .saturating_add(1);
        let cursor_sequence = format!(
            "\x1b[{};{}H",
            adjusted_cursor_row.saturating_add(1),
            adjusted_cursor_col
        );
        parser.process(cursor_sequence.as_bytes());
    }
    parser.process(if hide_cursor {
        b"\x1b[?25l"
    } else {
        b"\x1b[?25h"
    });
    Some(parser)
}

fn is_codex_progress_separator(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '─')
}

fn codex_progress_block_lines(lines: &[String]) -> Option<Vec<String>> {
    let first = lines.iter().position(|line| !line.trim().is_empty())?;
    let last = lines.iter().rposition(|line| !line.trim().is_empty())?;
    let mut normalized = Vec::new();
    let mut has_bullet = false;
    let mut has_child = false;

    for line in &lines[first..=last] {
        let trimmed = line.trim_end().to_string();
        if trimmed.trim().is_empty() {
            continue;
        }
        has_bullet |= trimmed.starts_with("• ");
        has_child |= trimmed.starts_with("  ");
        normalized.push(trimmed);
    }

    (has_bullet && has_child && normalized.len() >= 2).then_some(normalized)
}

fn codex_progress_block_repeated(previous: &[String], next: &[String]) -> bool {
    if previous.len() > next.len() || previous.len() < 2 {
        return false;
    }

    next.windows(previous.len())
        .any(|window| window == previous)
}

/// Render the full UI (Elm: view).
pub fn view(model: &Model, frame: &mut Frame) {
    let size = frame.area();

    // Initialization layer is fullscreen — no management panel or sessions
    if model.active_layer == ActiveLayer::Initialization {
        if let Some(ref init_state) = model.initialization {
            screens::initialization::render(init_state, frame, size);
        }
        // Error overlay on top
        if !model.error_queue.is_empty() {
            screens::error::render(&model.error_queue, frame, size);
        }
        return;
    }

    // Reserve 1 line at bottom for keybind hints
    let main_area = Rect {
        height: size.height.saturating_sub(1),
        ..size
    };
    let hint_area = Rect {
        y: size.height.saturating_sub(1),
        height: 1,
        ..size
    };

    if model.active_layer == ActiveLayer::Management {
        let lr = management_split(main_area);

        render_management_panes(model, frame, lr[0]);
        render_session_pane(model, frame, lr[1]);
    } else {
        render_session_pane(model, frame, main_area);
    }

    render_keybind_hints(model, frame, hint_area);

    // Overlays on top
    render_overlays(model, frame, size);
}

/// Build a bordered block with focus-aware border color (Cyan when focused, Gray otherwise).
fn pane_block(title: Line<'static>, is_focused: bool) -> Block<'static> {
    let (border_style, border_type) = theme::pane_border(is_focused);
    Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title(title)
}

/// Build the management tab title line for embedding in a pane border.
fn management_tab_title(model: &Model, width: u16) -> Line<'static> {
    let labels: Vec<&str> = ManagementTab::ALL.iter().map(|t| t.label()).collect();
    let active_idx = ManagementTab::ALL
        .iter()
        .position(|t| *t == model.management_tab)
        .unwrap_or(0);
    if should_compact_management_tab_title(width) {
        return compact_management_tab_title(&labels, active_idx, width);
    }
    screens::build_tab_title(&labels, active_idx)
}

fn should_compact_management_tab_title(width: u16) -> bool {
    let available_title_width = width.saturating_sub(2) as usize;
    let full_strip_width: usize = ManagementTab::ALL
        .iter()
        .map(|tab| tab.label().chars().count() + 2)
        .sum::<usize>()
        + ManagementTab::ALL.len().saturating_sub(1);
    full_strip_width > available_title_width
}

fn compact_management_tab_title(labels: &[&str], active_idx: usize, width: u16) -> Line<'static> {
    let available_title_width = width.saturating_sub(2) as usize;

    for window_len in (1..=labels.len().min(3)).rev() {
        let start = compact_tab_window_start(labels.len(), active_idx, window_len);
        let candidate =
            compact_management_tab_title_window(labels, active_idx, start, start + window_len);
        if title_line_width(&candidate) <= available_title_width {
            return candidate;
        }
    }

    compact_management_tab_title_window(labels, active_idx, active_idx, active_idx + 1)
}

fn compact_tab_window_start(total_tabs: usize, active_idx: usize, window_len: usize) -> usize {
    if total_tabs <= window_len {
        return 0;
    }
    let half_window = window_len / 2;
    let mut start = active_idx.saturating_sub(half_window);
    let max_start = total_tabs - window_len;
    if start > max_start {
        start = max_start;
    }
    start
}

fn compact_management_tab_title_window(
    labels: &[&str],
    active_idx: usize,
    start: usize,
    end: usize,
) -> Line<'static> {
    let mut spans = Vec::new();

    if start > 0 {
        spans.push(Span::styled("...", Style::default().fg(Color::DarkGray)));
        spans.push(Span::raw("│"));
    }

    for (idx, label) in labels[start..end].iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw("│"));
        }
        let tab_idx = start + idx;
        if tab_idx == active_idx {
            spans.push(Span::styled(
                format!(" {} ", label),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::Gray),
            ));
        }
    }

    if end < labels.len() {
        spans.push(Span::raw("│"));
        spans.push(Span::styled("...", Style::default().fg(Color::DarkGray)));
    }

    Line::from(spans)
}

fn title_line_width(title: &Line<'_>) -> usize {
    title
        .spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum()
}

/// Render the management panes (left side — 2 stacked for Branches, 1 for others).
fn render_management_panes(model: &Model, frame: &mut Frame, area: Rect) {
    if model.management_tab == ManagementTab::Branches {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Top pane: management tab names in title, branch list content
        let list_focused = model.active_focus == FocusPane::TabContent;
        let list_block = pane_block(management_tab_title(model, chunks[0].width), list_focused);
        let list_inner = list_block.inner(chunks[0]);
        frame.render_widget(list_block, chunks[0]);
        screens::branches::render_list(&model.branches, frame, list_inner);

        // Bottom pane: detail section names in title, detail content
        let detail_focused = model.active_focus == FocusPane::BranchDetail;
        let detail_title = branch_detail_title(model);
        let detail_block = pane_block(detail_title, detail_focused);
        let detail_inner = detail_block.inner(chunks[1]);
        frame.render_widget(detail_block, chunks[1]);
        let branch_sessions = branch_session_summaries(model);
        screens::branches::render_detail_content(
            &model.branches,
            frame,
            detail_inner,
            &branch_sessions,
        );
    } else {
        // Single pane for all other tabs
        let focused = model.active_focus == FocusPane::TabContent;
        let block = pane_block(management_tab_title(model, area.width), focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        render_management_tab_content(model, frame, inner);
    }
}

fn branch_detail_title(model: &Model) -> Line<'static> {
    let detail_labels: Vec<&str> = screens::branches::detail_section_labels().to_vec();
    let mut title = screens::build_tab_title(&detail_labels, model.branches.detail_section);
    title
        .spans
        .push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
    let branch_label = model
        .branches
        .selected_branch()
        .map(|branch| branch.name.clone())
        .unwrap_or_else(|| "No branch selected".to_string());
    title.spans.push(Span::styled(
        branch_label,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    title
}

/// Render the content of the active management tab (non-Branches).
fn render_management_tab_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.management_tab {
        ManagementTab::Branches => {
            // Handled by render_management_panes directly
        }
        ManagementTab::Issues => screens::issues::render(&model.issues, frame, area),
        ManagementTab::PrDashboard => {
            screens::pr_dashboard::render(&model.pr_dashboard, frame, area)
        }
        ManagementTab::Profiles => screens::profiles::render(&model.profiles, frame, area),
        ManagementTab::GitView => screens::git_view::render(&model.git_view, frame, area),
        ManagementTab::Versions => screens::versions::render(&model.versions, frame, area),
        ManagementTab::Settings => screens::settings::render(&model.settings, frame, area),
        ManagementTab::Logs => screens::logs::render(&model.logs, frame, area),
        ManagementTab::Specs => screens::specs::render(&model.specs, frame, area),
    }
}

/// Render the session pane (right side, or full screen).
fn render_session_pane(model: &Model, frame: &mut Frame, area: Rect) {
    let terminal_focused = model.active_focus == FocusPane::Terminal;
    match model.session_layout {
        SessionLayout::Tab => {
            if let Some(session) = model.active_session_tab() {
                let title = build_session_title(model, area.width);
                let block = pane_block(title, terminal_focused);
                let inner = block.inner(area);
                frame.render_widget(block, area);
                render_session_surface(session, frame, inner, terminal_focused);
            }
        }
        SessionLayout::Grid => {
            render_grid_sessions(model, frame, area);
        }
    }
}

/// Build session tab title line (same pattern as management tabs in Block title).
fn build_session_title(model: &Model, width: u16) -> Line<'static> {
    build_session_title_with(model, width, &gwt_sessions_dir())
}

fn build_session_title_with(model: &Model, width: u16, sessions_dir: &Path) -> Line<'static> {
    let pane_focused =
        model.active_layer == ActiveLayer::Main || model.active_focus == FocusPane::Terminal;
    let entries: Vec<(String, Style, &'static str)> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            (
                session_title_label(session, sessions_dir),
                session_title_style(session, i == model.active_session, pane_focused),
                session.tab_type.icon(),
            )
        })
        .collect();

    if should_compact_session_title(width, &entries) {
        if let Some((label, style, icon)) = entries.get(model.active_session) {
            let position = model.active_session.saturating_add(1);
            let total = model.sessions.len();
            let title = format!(" {position}/{total} {icon} {label} ");
            return Line::from(vec![Span::styled(title, *style)]);
        }
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (label, style, icon)) in entries.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(format!(" {icon} {label} "), style));
    }
    Line::from(spans)
}

fn session_title_label(session: &crate::model::SessionTab, sessions_dir: &Path) -> String {
    match &session.tab_type {
        SessionTabType::Agent { .. } => load_persisted_branch_label(&session.id, sessions_dir)
            .unwrap_or_else(|| session.name.clone()),
        SessionTabType::Shell => session.name.clone(),
    }
}

fn load_persisted_branch_label(session_id: &str, sessions_dir: &Path) -> Option<String> {
    let path = sessions_dir.join(format!("{session_id}.toml"));
    let persisted = AgentSession::load(&path).ok()?;
    let branch = persisted.branch.trim();
    if branch.is_empty() {
        None
    } else {
        Some(persisted.branch)
    }
}

fn session_title_style(
    session: &crate::model::SessionTab,
    is_active: bool,
    pane_focused: bool,
) -> Style {
    match &session.tab_type {
        SessionTabType::Shell => {
            if !pane_focused {
                Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
            } else if is_active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::Gray)
            }
        }
        SessionTabType::Agent { color, .. } => {
            let style = Style::default().fg(agent_color_to_ratatui(*color));
            if !pane_focused {
                style.add_modifier(Modifier::DIM)
            } else if is_active {
                style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                style.add_modifier(Modifier::DIM)
            }
        }
    }
}

fn should_compact_session_title(width: u16, entries: &[(String, Style, &'static str)]) -> bool {
    let available_title_width = width.saturating_sub(2) as usize;
    if available_title_width == 0 {
        return false;
    }

    let full_strip_width: usize = entries
        .iter()
        .enumerate()
        .map(|(i, (label, _, icon))| {
            let label_width = format!(" {icon} {label} ").len();
            if i == 0 {
                label_width
            } else {
                label_width + "│".len()
            }
        })
        .sum();

    full_strip_width > available_title_width
}

/// Render context-sensitive keybind hints at the bottom of the screen.
///
/// The status bar keeps session context visible and appends the relevant hints.
fn render_keybind_hints(model: &Model, frame: &mut Frame, area: Rect) {
    let compact = area.width <= 80;
    let hints = match model.active_focus {
        FocusPane::TabContent if model.management_tab == ManagementTab::Branches => {
            branches_list_hint_text_with_selection(
                compact,
                model.branches.cleanup_selection_count(),
            )
        }
        FocusPane::TabContent => management_hint_text(model, compact),
        FocusPane::BranchDetail => branch_detail_hint_text(model, compact),
        FocusPane::Terminal => terminal_hint_text(),
    };

    crate::widgets::status_bar::render_with_notification_and_hints(
        model,
        model.current_notification.as_ref(),
        Some(&hints),
        frame,
        area,
    );
}

fn terminal_hint_text() -> String {
    "Ctrl+G:b/i/s g c []/1-9 z ?  C-g Tab:focus  ^C×2".to_string()
}

fn branches_list_hint_text_with_selection(compact: bool, selection_count: usize) -> String {
    if compact {
        if selection_count > 0 {
            format!("↑↓  ↵ wiz  Sp sel({selection_count})  ⇧C clean  a all  Esc clr")
        } else {
            "↑↓ mv  ←→ tab  ↵ wiz  S↵ sh  Sp sel  ⇧C clean  a all  mvf?  Esc→T".to_string()
        }
    } else if selection_count > 0 {
        format!(
            "↑↓:move  Enter:wizard  Space:select({selection_count})  Shift+C:cleanup  a:all  Esc:clear"
        )
    } else {
        "↑↓:move  ←→:tab  Enter:wizard  Space:select  Shift+C:cleanup  a:all  m:view  v:git  f:search  Esc:term  ?:help".to_string()
    }
}

fn management_hint_text(model: &Model, compact: bool) -> String {
    match model.management_tab {
        ManagementTab::Branches => branches_list_hint_text_with_selection(
            compact,
            model.branches.cleanup_selection_count(),
        ),
        ManagementTab::Issues => issues_hint_text(model, compact),
        ManagementTab::Settings => {
            if model.settings.editing {
                settings_edit_hint_text(compact)
            } else {
                settings_list_hint_text(compact)
            }
        }
        ManagementTab::Logs => logs_hint_text(model, compact),
        ManagementTab::PrDashboard => pr_dashboard_hint_text(model, compact),
        ManagementTab::Profiles => profiles_hint_text(model, compact),
        ManagementTab::GitView => git_view_hint_text(compact),
        ManagementTab::Versions => versions_hint_text(compact),
        ManagementTab::Specs => {
            if compact {
                "↑↓ sel  r rfsh  Esc back".to_string()
            } else {
                "↑↓:select  r:reload from cache  Esc:back".to_string()
            }
        }
    }
}

fn issues_hint_text(model: &Model, compact: bool) -> String {
    if model.issues.detail_view {
        if compact {
            "↑↓ mv  ↵ close  r rfsh  C-g Tab  Esc back  ?".to_string()
        } else {
            "↑↓:move  Enter:close  r:refresh  Ctrl+G, Tab:focus  Esc:back  ?:help".to_string()
        }
    } else if compact {
        "↑↓ sel  ↵ dtl  / srch  r rfsh  C-g Tab  Esc term  ?".to_string()
    } else {
        "↑↓:select  Enter:detail  /:search  r:refresh  Ctrl+G, Tab:focus  Esc:term  ?:help"
            .to_string()
    }
}

fn generic_management_hint_text(
    compact: bool,
    include_sub_tab: bool,
    escape_action: &str,
) -> String {
    let compact_sub_tab = if include_sub_tab {
        "  C-←→ sub"
    } else {
        ""
    };
    let full_sub_tab = if include_sub_tab {
        "  Ctrl+←→:sub-tab"
    } else {
        ""
    };

    if compact {
        format!("↑↓ sel  ←→ tab{compact_sub_tab}  ↵ act  C-g Tab  Esc {escape_action}  ?")
    } else {
        format!(
            "↑↓:select  ←→:tab{full_sub_tab}  Enter:action  Ctrl+G, Tab:focus  Esc:{escape_action}  ?:help"
        )
    }
}

fn settings_list_hint_text(compact: bool) -> String {
    if compact {
        "↑↓ sel  ↵ edit  Sp tog  C-←→ sub  S save  C-g Tab  Esc term  ?".to_string()
    } else {
        "↑↓:select  Enter:edit  Space:toggle  Ctrl+←→:sub-tab  Shift+S:save  Ctrl+G, Tab:focus  Esc:term  ?:help".to_string()
    }
}

fn settings_edit_hint_text(compact: bool) -> String {
    if compact {
        "↵ save  ⌫ del  Esc cancel  ?".to_string()
    } else {
        "Enter:save  Backspace:delete  Esc:cancel  ?:help".to_string()
    }
}

fn logs_hint_text(model: &Model, compact: bool) -> String {
    if model.logs.detail_view {
        if compact {
            "↑↓ mv  ↵ close  f next  d dbg  r rfsh  C-←→ flt  Esc back".to_string()
        } else {
            "↑↓:move  Enter:close  f:next-filter  d:debug  r:refresh  Ctrl+←→:filter  Esc:back"
                .to_string()
        }
    } else if compact {
        "↑↓ sel  ↵ dtl  f next  d dbg  r rfsh  C-←→ flt  Esc term".to_string()
    } else {
        "↑↓:select  Enter:detail  f:next-filter  d:debug  r:refresh  Ctrl+←→:filter  Esc:term"
            .to_string()
    }
}

fn pr_dashboard_hint_text(model: &Model, compact: bool) -> String {
    if model.pr_dashboard.detail_view {
        if compact {
            "↑↓ mv  ↵ close  r rfsh  C-g Tab  Esc back  ?".to_string()
        } else {
            "↑↓:move  Enter:close  r:refresh  Ctrl+G, Tab:focus  Esc:back  ?:help".to_string()
        }
    } else if compact {
        "↑↓ sel  ↵ dtl  r rfsh  C-g Tab  Esc term  ?".to_string()
    } else {
        "↑↓:select  Enter:detail  r:refresh  Ctrl+G, Tab:focus  Esc:term  ?:help".to_string()
    }
}

fn profiles_hint_text(model: &Model, compact: bool) -> String {
    use screens::profiles::{ProfileMode, ProfilesFocus};

    if model.profiles.mode != ProfileMode::List {
        generic_management_hint_text(compact, false, "cancel")
    } else if compact {
        match model.profiles.focus {
            ProfilesFocus::ProfileList => {
                "Tab pane  S-Tab back  ↑↓ sel  ↵ act  n/e/d  Esc term".to_string()
            }
            ProfilesFocus::Environment => {
                "Tab pane  S-Tab back  ↑↓ env  ↵/e edit  n add  d del/rst  Esc term".to_string()
            }
        }
    } else {
        match model.profiles.focus {
            ProfilesFocus::ProfileList => {
                "Tab:next pane  Shift+Tab:prev pane  ↑↓:select  Enter:activate  n:new  e:edit  d:delete  Esc:term".to_string()
            }
            ProfilesFocus::Environment => {
                "Tab:next pane  Shift+Tab:prev pane  ↑↓:env  Enter/e:edit  n:add  d:delete/restore  Esc:term"
                    .to_string()
            }
        }
    }
}

fn git_view_hint_text(compact: bool) -> String {
    if compact {
        "↑↓ mv  ↵ exp  r rfsh  C-g Tab  Esc term  ?".to_string()
    } else {
        "↑↓:move  Enter:expand  r:refresh  Ctrl+G, Tab:focus  Esc:term  ?:help".to_string()
    }
}

fn versions_hint_text(compact: bool) -> String {
    if compact {
        "↑↓ mv  r rfsh  C-g Tab  Esc term  ?".to_string()
    } else {
        "↑↓:move  r:refresh  Ctrl+G, Tab:focus  Esc:term  ?:help".to_string()
    }
}

fn branch_detail_hint_text(model: &Model, compact: bool) -> String {
    let direct_action_hints = if selected_branch_has_worktree(model) {
        "  Shift+Enter:shell"
    } else {
        ""
    };
    let local_mnemonics = "  m:view  v:git  f:search  ?:help";
    if compact {
        let direct_action_hints = if selected_branch_has_worktree(model) {
            "  S↵ sh"
        } else {
            ""
        };
        let docker_hints = model
            .branches
            .docker_services
            .get(model.branches.docker_selected)
            .map(|service| match service.status {
                gwt_docker::ComposeServiceStatus::Running => "  T/R/C",
                gwt_docker::ComposeServiceStatus::Stopped
                | gwt_docker::ComposeServiceStatus::Exited => "  S/C",
                gwt_docker::ComposeServiceStatus::NotFound => "  S",
            })
            .unwrap_or("");
        return match model.branches.detail_section {
            0 => format!(
                "←→ sec  ↵ act{direct_action_hints}{docker_hints}  Sp sel  mvf?  C-g↔P  Esc←"
            ),
            2 => "↑↓ ses  ←→ sec  ↵ focus  Sp sel  mvf?  C-g↔P  Esc←".to_string(),
            _ => format!("←→ sec  ↵ act{direct_action_hints}  Sp sel  mvf?  C-g↔P  Esc←"),
        };
    }
    match model.branches.detail_section {
        0 => {
            let docker_hints = model
                .branches
                .docker_services
                .get(model.branches.docker_selected)
                .map(|service| match service.status {
                    gwt_docker::ComposeServiceStatus::Running => {
                        "  T:stop  R:restart  C:recreate"
                    }
                    gwt_docker::ComposeServiceStatus::Stopped
                    | gwt_docker::ComposeServiceStatus::Exited => "  S:start  C:recreate",
                    gwt_docker::ComposeServiceStatus::NotFound => "  S:create/start",
                })
                .unwrap_or("");
            format!(
                "←→:section  Enter:launch{direct_action_hints}{docker_hints}  Space:select{local_mnemonics}  Ctrl+G, Tab:focus  Esc:back"
            )
        }
        2 => format!(
            "↑↓:session  ←→:section  Enter:focus  Space:select{local_mnemonics}  Ctrl+G, Tab:focus  Esc:back"
        ),
        _ => format!(
            "←→:section  Enter:launch{direct_action_hints}  Space:select{local_mnemonics}  Ctrl+G, Tab:focus  Esc:back"
        ),
    }
}

/// Render all overlay widgets on top of the main layout.
fn render_overlays(model: &Model, frame: &mut Frame, size: Rect) {
    // Confirm dialog overlay
    screens::confirm::render(&model.confirm, frame, size);

    // Branch Cleanup confirm modal (FR-018e)
    screens::cleanup_confirm::render(&model.cleanup_confirm, frame, size);

    // Branch Cleanup progress modal (FR-018g/h)
    screens::cleanup_progress::render(&model.cleanup_progress, frame, size);

    // Docker progress overlay
    if let Some(ref docker) = model.docker_progress {
        screens::docker_progress::render(docker, frame, size);
    }

    // Service selection overlay
    if let Some(ref svc) = model.service_select {
        screens::service_select::render(svc, frame, size);
    }

    // Port selection overlay
    if let Some(ref port) = model.port_select {
        screens::port_select::render(port, frame, size);
    }

    // Wizard overlay (on top of everything except errors)
    if let Some(ref wizard) = model.wizard {
        screens::wizard::render(wizard, frame, size);
    }

    if model.help_visible {
        let bindings = crate::input::keybind::KeybindRegistry::new();
        screens::help::render(bindings.all_bindings(), frame, size);
    }

    // Error overlay on top of everything
    if !model.error_queue.is_empty() {
        screens::error::render(&model.error_queue, frame, size);
    }
}

/// Render sessions in a grid layout.
fn render_grid_sessions(model: &Model, frame: &mut Frame, area: Rect) {
    let count = model.sessions.len();
    if count == 0 {
        return;
    }

    let terminal_focused = model.active_focus == FocusPane::Terminal;
    for session_idx in 0..count {
        let Some(session) = model.sessions.get(session_idx) else {
            continue;
        };
        let Some(pane_area) = grid_session_pane_area(area, count, session_idx) else {
            continue;
        };
        let is_active = session_idx == model.active_session;
        let block = grid_session_block(session_idx, session, is_active && terminal_focused);
        let inner = block.inner(pane_area);
        frame.render_widget(block, pane_area);
        render_session_surface(session, frame, inner, is_active && terminal_focused);
    }
}

fn grid_session_block(
    session_idx: usize,
    session: &crate::model::SessionTab,
    is_active: bool,
) -> Block<'static> {
    pane_block(
        Line::from(grid_session_title(session_idx, session)),
        is_active,
    )
}

fn grid_session_title(session_idx: usize, session: &crate::model::SessionTab) -> String {
    grid_session_title_with(session_idx, session, &gwt_sessions_dir())
}

fn grid_session_title_with(
    session_idx: usize,
    session: &crate::model::SessionTab,
    sessions_dir: &Path,
) -> String {
    format!(
        " {}: {} {} ",
        session_idx.saturating_add(1),
        session.tab_type.icon(),
        session_title_label(session, sessions_dir)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use crossterm::event::{
        KeyEvent, KeyEventKind, KeyEventState, MouseButton, MouseEvent, MouseEventKind,
    };
    use gwt_agent::{
        custom::{CustomAgentType, ModeArgs},
        version_cache::VersionEntry,
        AgentId, CustomCodingAgent, DetectedAgent, VersionCache,
    };
    use gwt_core::logging::{LogEvent as Notification, LogLevel as Severity};
    use gwt_git::pr_status::PrState as GitPrState;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};
    use ratatui::text::Line;
    use ratatui::widgets::Widget;
    use ratatui::{buffer::Buffer, Terminal};
    use std::collections::HashMap;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Once;
    use tempfile::TempDir;

    static VERSION_CACHE_SCHEDULER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static INPUT_TRACE_ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static HOME_ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn disable_global_custom_agents_for_tests() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            std::env::set_var(DISABLE_GLOBAL_CUSTOM_AGENTS_ENV, "1");
        });
    }

    fn test_model() -> Model {
        disable_global_custom_agents_for_tests();
        Model::new(PathBuf::from("/tmp/test"))
    }

    struct HomeEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl HomeEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("HOME");
            std::env::set_var("HOME", path);
            Self { previous }
        }
    }

    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var("HOME", previous);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    fn with_temp_home<T>(run: impl FnOnce(&std::path::Path) -> T) -> T {
        let _guard = HOME_ENV_TEST_LOCK.lock().expect("lock HOME env");
        let home = tempfile::tempdir().expect("temp home dir");
        let _env = HomeEnvGuard::set(home.path());
        run(home.path())
    }

    fn wait_for_startup_version_cache_refresh_slot() {
        for _ in 0..100 {
            if !STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    type StartupVersionRefreshTask = Box<dyn FnOnce() + Send>;
    type ScheduledStartupVersionRefresh =
        std::sync::Arc<std::sync::Mutex<Option<(PathBuf, Vec<AgentId>)>>>;

    fn capture_startup_version_cache_refresh_task(
        cache_path: PathBuf,
        detect_agents: std::sync::Arc<dyn Fn() -> Vec<DetectedAgent> + Send + Sync>,
    ) -> (StartupVersionRefreshTask, ScheduledStartupVersionRefresh) {
        for _ in 0..5 {
            let spawned = std::cell::RefCell::new(None::<StartupVersionRefreshTask>);
            let scheduled = std::sync::Arc::new(std::sync::Mutex::new(None));
            let scheduled_capture = scheduled.clone();
            let detect_agents = detect_agents.clone();

            schedule_startup_version_cache_refresh_with(
                cache_path.clone(),
                move || detect_agents.as_ref()(),
                |task| {
                    *spawned.borrow_mut() = Some(task);
                },
                move |path, targets| {
                    *scheduled_capture.lock().unwrap() = Some((path, targets));
                },
            );

            if let Some(task) = spawned.borrow_mut().take() {
                return (task, scheduled);
            }

            wait_for_startup_version_cache_refresh_slot();
        }

        panic!("failed to capture startup version cache refresh task");
    }

    #[test]
    fn tick_redraw_required_stays_true_while_cleanup_progress_is_visible() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.cleanup_progress.show(1, false);

        update(&mut model, Message::Tick);

        assert!(
            tick_redraw_required(&model),
            "cleanup progress should keep tick-driven redraws alive until the modal reflects updates"
        );
    }

    #[derive(Debug)]
    struct FakeVoiceRuntime {
        start_result: Result<(), String>,
        stop_result: Result<String, String>,
    }

    impl FakeVoiceRuntime {
        fn success(transcript: &str) -> Self {
            Self {
                start_result: Ok(()),
                stop_result: Ok(transcript.to_string()),
            }
        }

        fn start_error(message: &str) -> Self {
            Self {
                start_result: Err(message.to_string()),
                stop_result: Ok(String::new()),
            }
        }

        fn stop_error(message: &str) -> Self {
            Self {
                start_result: Ok(()),
                stop_result: Err(message.to_string()),
            }
        }
    }

    impl VoiceRuntime for FakeVoiceRuntime {
        fn configure(&mut self, _config: &VoiceConfig) {}

        fn start_recording(&mut self) -> Result<(), String> {
            self.start_result.clone()
        }

        fn stop_and_transcribe(&mut self) -> Result<String, String> {
            self.stop_result.clone()
        }

        fn reset(&mut self) {}
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            let line = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>();
            text.push_str(line.trim_end());
            text.push('\n');
        }
        text
    }

    fn render_model_text(model: &Model, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| view(model, frame))
            .expect("draw model");
        buffer_text(terminal.backend().buffer())
    }

    fn render_model_buffer(model: &Model, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| view(model, frame))
            .expect("draw model");
        terminal.backend().buffer().clone()
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn title_label_click_x(title: &Line<'_>, area_x: u16, label: &str) -> u16 {
        let mut x = area_x + 1;
        for span in &title.spans {
            let content = span.content.as_ref();
            let width = content.chars().count() as u16;
            if content.trim() == label {
                return x + width.saturating_sub(1).min(1);
            }
            x = x.saturating_add(width);
        }
        panic!("label `{label}` not found in title `{}`", line_text(title));
    }

    fn test_main_area(model: &Model) -> Rect {
        let (width, height) = model.terminal_size;
        Rect::new(0, 0, width, height.saturating_sub(1))
    }

    fn branches_management_areas(model: &Model) -> (Rect, Rect, Rect) {
        let management = management_split(test_main_area(model))[0];
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(management);
        let top = split[0];
        let list_inner = pane_block(management_tab_title(model, top.width), false).inner(top);
        let list_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(list_inner);
        (top, split[1], list_chunks[1])
    }

    fn grid_session_area(area: Rect, count: usize, target_index: usize) -> Rect {
        let cols = (count as f64).sqrt().ceil() as usize;
        let rows = count.div_ceil(cols);
        let row_constraints: Vec<Constraint> = (0..rows)
            .map(|_| Constraint::Ratio(1, rows as u32))
            .collect();
        let row_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(area);
        let target_row = target_index / cols;
        let start = target_row * cols;
        let end = (start + cols).min(count);
        let n = end - start;
        let col_constraints: Vec<Constraint> =
            (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
        let col_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(row_chunks[target_row]);
        col_chunks[target_index - start]
    }

    fn persist_agent_tab(
        sessions_dir: &Path,
        branch: &str,
        agent_id: AgentId,
        color: crate::model::AgentColor,
    ) -> (crate::model::SessionTab, PathBuf) {
        fs::create_dir_all(sessions_dir).expect("create sessions dir");

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let session_id = format!("test-session-{}-{unique}", agent_id.command());

        let mut session = AgentSession::new("/tmp/test-worktree", branch, agent_id.clone());
        session.id = session_id.clone();
        session.display_name = agent_id.display_name().to_string();
        session.save(sessions_dir).expect("persist session");

        (
            crate::model::SessionTab {
                id: session_id.clone(),
                name: agent_id.display_name().to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: agent_id.command().to_string(),
                    color,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            sessions_dir.join(format!("{session_id}.toml")),
        )
    }

    fn append_session_line(model: &mut Model, session_id: &str, line: &str) {
        update(
            model,
            Message::PtyOutput(session_id.to_string(), format!("{line}\r\n").into_bytes()),
        );
    }

    fn join_terminal_lines(lines: &[&str]) -> String {
        lines.join("\r\n")
    }

    fn enter_alt_screen_with_text(model: &mut Model, session_id: &str, text: &str) {
        update(
            model,
            Message::PtyOutput(
                session_id.to_string(),
                format!("\x1b[?1049h\x1b[2J\x1b[H{text}").into_bytes(),
            ),
        );
    }

    fn enter_alt_screen_with_lines(model: &mut Model, session_id: &str, lines: &[&str]) {
        enter_alt_screen_with_text(model, session_id, &join_terminal_lines(lines));
    }

    fn replace_alt_screen_text(model: &mut Model, session_id: &str, text: &str) {
        update(
            model,
            Message::PtyOutput(
                session_id.to_string(),
                format!("\x1b[2J\x1b[H{text}").into_bytes(),
            ),
        );
    }

    fn replace_alt_screen_lines(model: &mut Model, session_id: &str, lines: &[&str]) {
        replace_alt_screen_text(model, session_id, &join_terminal_lines(lines));
    }

    fn detected_agent(agent_id: AgentId, version: Option<&str>) -> DetectedAgent {
        disable_global_custom_agents_for_tests();
        DetectedAgent {
            agent_id,
            version: version.map(|value| value.to_string()),
            path: PathBuf::from("/tmp/fake-agent"),
        }
    }

    fn agent_session_tab(
        name: &str,
        agent_id: &str,
        color: crate::model::AgentColor,
    ) -> crate::model::SessionTab {
        let mut vt = crate::model::VtState::new(30, 100);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);
        crate::model::SessionTab {
            id: "agent-0".to_string(),
            name: name.to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: agent_id.to_string(),
                color,
            },
            vt,
            created_at: std::time::Instant::now(),
        }
    }

    fn version_entry(versions: &[&str], age_seconds: i64) -> VersionEntry {
        VersionEntry {
            versions: versions.iter().map(|value| value.to_string()).collect(),
            updated_at: Utc::now() - Duration::seconds(age_seconds),
        }
    }

    fn sample_custom_agent(
        agent_type: CustomAgentType,
        command: impl Into<String>,
    ) -> CustomCodingAgent {
        CustomCodingAgent {
            id: "my-agent".to_string(),
            display_name: "My Agent".to_string(),
            agent_type,
            command: command.into(),
            default_args: vec!["--flag".to_string()],
            mode_args: Some(ModeArgs {
                normal: vec!["--normal".to_string()],
                continue_mode: vec!["--continue".to_string()],
                resume: vec!["--resume".to_string()],
            }),
            skip_permissions_args: vec!["--yolo".to_string()],
            env: HashMap::from([("CUSTOM_ENV".to_string(), "enabled".to_string())]),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_agent_session(
        dir: &std::path::Path,
        repo_path: &str,
        branch: &str,
        agent_id: AgentId,
        updated_at: chrono::DateTime<Utc>,
        model: Option<&str>,
        reasoning_level: Option<&str>,
        tool_version: Option<&str>,
        resume_session_id: Option<&str>,
        skip_permissions: bool,
        codex_fast_mode: bool,
    ) {
        let mut session = AgentSession::new(repo_path, branch, agent_id);
        session.model = model.map(str::to_string);
        session.reasoning_level = reasoning_level.map(str::to_string);
        session.tool_version = tool_version.map(str::to_string);
        session.agent_session_id = resume_session_id.map(str::to_string);
        session.skip_permissions = skip_permissions;
        session.codex_fast_mode = codex_fast_mode;
        session.updated_at = updated_at;
        session.created_at = updated_at;
        session.last_activity_at = updated_at;
        session.save(dir).expect("persist session");
    }

    fn docker_service(
        project_root: &std::path::Path,
        name: &str,
        status: gwt_docker::ComposeServiceStatus,
    ) -> screens::branches::DockerServiceInfo {
        screens::branches::DockerServiceInfo {
            project_root: project_root.to_path_buf(),
            compose_file: project_root.join("docker-compose.yml"),
            name: name.to_string(),
            status,
            ports: if name == "db" {
                "5432:5432".to_string()
            } else {
                "8080:80".to_string()
            },
        }
    }

    fn write_fake_docker(script_body: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let script_path = dir.path().join("docker");
        let mut file = fs::File::create(&script_path).expect("create fake docker");
        file.write_all(script_body.as_bytes())
            .expect("write fake docker");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata().expect("stat fake docker").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).expect("chmod fake docker");
        }

        (dir, script_path)
    }

    fn with_fake_docker<R>(script_body: &str, f: impl FnOnce() -> R) -> R {
        let _guard = crate::DOCKER_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, script_path) = write_fake_docker(script_body);
        let previous = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", &script_path);

        let result = f();

        match previous {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        result
    }

    fn with_docker_bin_override<R>(path: &std::path::Path, f: impl FnOnce() -> R) -> R {
        let _guard = crate::DOCKER_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", path);

        let result = f();

        match previous {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        result
    }

    fn write_docker_launch_compose_fixture(worktree: &std::path::Path) -> PathBuf {
        let compose_path = worktree.join("docker-compose.yml");
        fs::write(
            &compose_path,
            r#"
services:
  gwt:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose fixture");
        compose_path
    }

    #[cfg(unix)]
    fn write_fake_gh(script_body: &str) -> TempDir {
        let dir = tempfile::tempdir().expect("create temp dir");
        let script_path = dir.path().join("gh");
        let mut file = fs::File::create(&script_path).expect("create fake gh");
        file.write_all(script_body.as_bytes())
            .expect("write fake gh");

        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata().expect("stat fake gh").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod fake gh");

        dir
    }

    #[cfg(unix)]
    fn with_fake_gh<R>(script_body: &str, f: impl FnOnce() -> R) -> R {
        let _guard = crate::GH_PATH_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = write_fake_gh(script_body);
        let previous = std::env::var_os("PATH");
        let mut entries = vec![dir.path().to_path_buf()];
        if let Some(value) = previous.as_ref() {
            entries.extend(std::env::split_paths(value));
        }
        let new_path = std::env::join_paths(entries).expect("join fake gh PATH");
        std::env::set_var("PATH", &new_path);

        let result = f();

        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        result
    }

    fn drive_docker_worker_until(model: &mut Model, done: impl Fn(&Model) -> bool, context: &str) {
        for _ in 0..120 {
            update(model, Message::Tick);
            if done(model) {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }

        panic!("timed out waiting for docker worker: {context}");
    }

    fn drive_ticks_until(model: &mut Model, done: impl Fn(&Model) -> bool, context: &str) {
        for _ in 0..200 {
            update(model, Message::Tick);
            if done(model) {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }

        panic!("timed out waiting for ticks: {context}");
    }

    #[test]
    fn sync_active_agent_transcript_scrollback_with_ignores_session_logs_for_agent_panes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions_dir = temp.path().join("sessions");
        let claude_projects_root = temp.path().join("claude/projects");
        let codex_sessions_root = temp.path().join("codex/sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions dir");
        fs::create_dir_all(&claude_projects_root).expect("claude projects dir");
        fs::create_dir_all(codex_sessions_root.join("2026/04/07")).expect("codex day dir");

        let worktree = temp.path().join("wt-codex");
        fs::create_dir_all(&worktree).expect("worktree");

        let session_id = "agent-codex-test";
        let mut persisted = AgentSession::new(&worktree, "feature/transcript", AgentId::Codex);
        persisted.id = session_id.to_string();
        persisted
            .save(&sessions_dir)
            .expect("persist codex session");

        let transcript_path = codex_sessions_root
            .join("2026/04/07")
            .join("rollout-test.jsonl");
        let transcript_lines = [
            serde_json::json!({
                "type": "session_meta",
                "payload": { "cwd": worktree.to_string_lossy() }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "prompt-1" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "answer-1" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "prompt-2" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "answer-2" }]
                }
            })
            .to_string(),
        ];
        fs::write(&transcript_path, transcript_lines.join("\n")).expect("write codex transcript");
        let resolved = resolve_codex_transcript_source(&persisted, &codex_sessions_root)
            .expect("resolved codex transcript source");
        assert_eq!(resolved.path, transcript_path);
        let parsed_lines = read_transcript_lines_for_agent(&AgentId::Codex, &resolved.path)
            .expect("parsed codex transcript lines");
        assert!(
            parsed_lines.iter().any(|line| line.contains("prompt-1")),
            "test setup should exercise real transcript parsing before verifying runtime ignore behavior"
        );

        let mut model = Model::new(worktree.clone());
        model.sessions = vec![crate::model::SessionTab {
            id: session_id.to_string(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            },
            vt: crate::model::VtState::new(3, 80),
            created_at: std::time::Instant::now(),
        }];
        model.active_session = 0;

        sync_active_agent_transcript_scrollback_with(
            &mut model,
            &sessions_dir,
            &claude_projects_root,
            &codex_sessions_root,
        );

        let session = model.active_session_tab_mut().expect("active session");
        assert!(
            !session.vt.has_viewport_scrollback(),
            "agent pane scrollback should stay empty until PTY output arrives; session logs must not hydrate runtime history"
        );
        let text = session.vt.visible_screen_parser().screen().contents();
        assert!(!text.contains("prompt-1"));
        assert!(!text.contains("answer-1"));
    }

    #[test]
    fn pane_block_uses_yellow_double_border_when_focused() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);

        pane_block(Line::from("Focused"), true).render(area, &mut buffer);

        assert_eq!(buffer[(0, 0)].fg, Color::Yellow);
        assert_eq!(buffer[(0, 0)].symbol(), "╔");
    }

    #[test]
    fn pane_block_uses_gray_rounded_border_when_unfocused() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);

        pane_block(Line::from("Unfocused"), false).render(area, &mut buffer);

        assert_eq!(buffer[(0, 0)].fg, Color::Gray);
        assert_eq!(buffer[(0, 0)].symbol(), "╭");
    }

    #[test]
    fn management_split_uses_50_50_at_standard_width_and_40_60_at_wide_width() {
        let standard = Rect::new(0, 0, 100, 20);
        let [standard_management, standard_session] = management_split(standard);
        assert_eq!(standard_management, Rect::new(0, 0, 50, 20));
        assert_eq!(standard_session, Rect::new(50, 0, 50, 20));

        let wide = Rect::new(0, 0, 120, 20);
        let [wide_management, wide_session] = management_split(wide);
        assert_eq!(wide_management, Rect::new(0, 0, 48, 20));
        assert_eq!(wide_session, Rect::new(48, 0, 72, 20));
    }

    #[test]
    fn active_session_content_area_matches_responsive_management_split() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.terminal_size = (100, 24);

        let standard = active_session_content_area(&model).expect("active session content area");
        assert_eq!(standard, Rect::new(51, 1, 48, 21));

        model.terminal_size = (120, 24);
        let wide = active_session_content_area(&model).expect("active session content area");
        assert_eq!(wide, Rect::new(49, 1, 70, 21));
    }

    #[test]
    fn pty_output_renders_into_session_surface() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;

        update(
            &mut model,
            Message::PtyOutput("shell-0".to_string(), b"https://example.com".to_vec()),
        );

        let rendered = render_model_text(&model, 80, 24);
        assert!(rendered.contains("https://example.com"));
    }

    #[test]
    fn mouse_scroll_up_moves_terminal_into_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(18, 8));
        for i in 0..12 {
            append_session_line(&mut model, "shell-0", &format!("line-{i}"));
        }

        let before = render_model_text(&model, 18, 8);
        assert!(before.contains("line-11"));
        assert!(!before.contains("line-6"));

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let after = render_model_text(&model, 18, 8);
        assert!(
            after.contains("line-7"),
            "scrolling up should reveal an earlier line from scrollback"
        );
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .scrollback()
                > 0,
            "scrolling up should move the viewport away from live follow mode"
        );
    }

    #[test]
    fn mouse_scroll_up_over_session_focuses_terminal_and_scrolls() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;
        update(&mut model, Message::Resize(18, 8));
        for i in 0..12 {
            append_session_line(&mut model, "shell-0", &format!("line-{i}"));
        }

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(
            model.active_focus,
            FocusPane::Terminal,
            "session mouse scroll should move focus to the terminal pane"
        );
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .scrollback()
                > 0,
            "session mouse scroll should move the viewport away from live follow mode"
        );
    }

    #[test]
    fn mouse_click_management_tab_switches_tab_and_focuses_management_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(120, 30));

        let (top, _, _) = branches_management_areas(&model);
        let title = management_tab_title(&model, top.width);
        let issues_x = title_label_click_x(&title, top.x, "Issues");

        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: issues_x,
                row: top.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(model.management_tab, ManagementTab::Issues);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn mouse_click_session_tab_switches_active_session() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions.push(crate::model::SessionTab {
            id: "agent-0".to_string(),
            name: "Claude Code".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "claude".to_string(),
                color: crate::model::AgentColor::Green,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.active_session = 0;
        update(&mut model, Message::Resize(80, 24));

        // The session pane spans the full width in Main layer.
        let session_area = visible_session_area(&model).expect("session area");
        let title = build_session_title(&model, session_area.width);

        // Find the x position that falls within the second tab label.
        let mut x = session_area.x.saturating_add(1);
        let mut target_x = None;
        let mut session_idx = 0;
        for span in &title.spans {
            let content = span.content.as_ref();
            let width = content.chars().count() as u16;
            if content.trim() == "│" {
                x = x.saturating_add(width);
                continue;
            }
            if session_idx == 1 {
                target_x = Some(x + 1);
                break;
            }
            x = x.saturating_add(width);
            session_idx += 1;
        }
        let click_x = target_x.expect("second session tab label should exist in the title");

        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: click_x,
                row: session_area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(
            model.active_session, 1,
            "clicking the second session tab should switch to it"
        );
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn mouse_click_branch_list_row_selects_branch_and_focuses_list() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(120, 30));
        screens::branches::update(
            &mut model.branches,
            screens::branches::BranchesMessage::SetBranches(vec![
                screens::branches::BranchItem {
                    name: "feature/one".to_string(),
                    is_head: false,
                    is_local: true,
                    category: screens::branches::BranchCategory::Feature,
                    worktree_path: None,
                    upstream: None,
                },
                screens::branches::BranchItem {
                    name: "feature/two".to_string(),
                    is_head: false,
                    is_local: true,
                    category: screens::branches::BranchCategory::Feature,
                    worktree_path: None,
                    upstream: None,
                },
            ]),
        );

        let (_, _, list_area) = branches_management_areas(&model);
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: list_area.x + 2,
                row: list_area.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(
            model
                .branches
                .selected_branch()
                .expect("selected branch")
                .name,
            "feature/two"
        );
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn mouse_click_branch_detail_title_switches_section_and_focuses_detail() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        update(&mut model, Message::Resize(120, 30));
        screens::branches::update(
            &mut model.branches,
            screens::branches::BranchesMessage::SetBranches(vec![screens::branches::BranchItem {
                name: "feature/one".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: None,
                upstream: None,
            }]),
        );

        let (_, detail_area, _) = branches_management_areas(&model);
        let title = branch_detail_title(&model);
        let git_x = title_label_click_x(&title, detail_area.x, "Git");

        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: git_x,
                row: detail_area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(model.branches.detail_section, 1);
        assert_eq!(model.active_focus, FocusPane::BranchDetail);
    }

    #[test]
    fn mouse_click_grid_session_switches_active_session_and_focuses_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::TabContent;
        model.session_layout = SessionLayout::Grid;
        model.sessions.push(crate::model::SessionTab {
            id: "shell-1".to_string(),
            name: "Shell 2".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        update(&mut model, Message::Resize(120, 30));

        let second_area = grid_session_area(test_main_area(&model), model.sessions.len(), 1);
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: second_area.x + 2,
                row: second_area.y + 2,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(model.active_session, 1);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn render_model_text_grid_sessions_render_live_terminal_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/one"),
            shell_tab("shell-1", "Shell: feature/two"),
            shell_tab("shell-2", "Shell: feature/three"),
        ];

        update(&mut model, Message::Resize(120, 30));
        append_session_line(&mut model, "shell-0", "grid-line-one");
        append_session_line(&mut model, "shell-1", "grid-line-two");
        append_session_line(&mut model, "shell-2", "grid-line-three");

        let rendered = render_model_text(&model, 120, 30);

        assert!(
            rendered.contains("grid-line-one"),
            "grid mode should render the first session's live surface, not just its pane title"
        );
        assert!(
            rendered.contains("grid-line-two"),
            "grid mode should render the second session's live surface, not just its pane title"
        );
        assert!(
            rendered.contains("grid-line-three"),
            "grid mode should render the third session's live surface, not just its pane title"
        );
    }

    #[test]
    fn grid_session_area_stacks_two_sessions_vertically_in_tall_layout() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/one"),
            shell_tab("shell-1", "Shell: feature/two"),
        ];

        update(&mut model, Message::Resize(40, 80));

        let session_area = test_main_area(&model);
        let first = grid_session_pane_area(session_area, model.sessions.len(), 0)
            .expect("first grid session");
        let second = grid_session_pane_area(session_area, model.sessions.len(), 1)
            .expect("second grid session");

        assert_eq!(
            first.x, second.x,
            "tall layouts should transpose the two-session grid into a vertical stack"
        );
        assert!(
            second.y > first.y,
            "the second session should appear below the first in a tall layout"
        );
        assert_eq!(
            first.width, second.width,
            "transposed two-session layouts should preserve equal widths"
        );
    }

    #[test]
    fn resize_grid_syncs_each_session_to_its_own_cell_size() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/one"),
            shell_tab("shell-1", "Shell: feature/two"),
            shell_tab("shell-2", "Shell: feature/three"),
        ];
        model.active_session = 0;

        update(&mut model, Message::Resize(120, 30));

        let top_cols = model.sessions[0].vt.cols();
        let bottom_cols = model.sessions[2].vt.cols();

        assert!(
            bottom_cols > top_cols,
            "three-session grid layouts should resize the bottom full-width session wider than the top-row sessions"
        );
    }

    #[test]
    fn wheel_over_inactive_grid_session_scrolls_it_without_changing_active_session() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/one"),
            shell_tab("shell-1", "Shell: feature/two"),
        ];
        model.active_session = 0;
        update(&mut model, Message::Resize(120, 30));

        for i in 0..40 {
            append_session_line(&mut model, "shell-1", &format!("two-line-{i}"));
        }

        let second_area = grid_session_pane_area(test_main_area(&model), model.sessions.len(), 1)
            .expect("second grid session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: second_area.x + 2,
                row: second_area.y + 2,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(
            model.active_session, 0,
            "wheel scrolling another grid session should not steal active input ownership"
        );
        assert!(
            model.sessions[1].vt.viewing_history(),
            "wheel scrolling over an inactive grid session should scroll that session's local viewport"
        );
    }

    #[test]
    fn right_drag_over_session_scrolls_terminal_for_terminal_app_trackpad_fallback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;
        update(&mut model, Message::Resize(18, 8));
        for i in 0..12 {
            append_session_line(&mut model, "shell-0", &format!("line-{i}"));
        }

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Right),
                column: area.x,
                row: area.y + 3,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(
            model.active_focus,
            FocusPane::Terminal,
            "session right-drag fallback should move focus to the terminal pane"
        );
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .scrollback()
                > 0,
            "Terminal.app style right-drag fallback should move the viewport away from live follow mode"
        );
    }

    #[test]
    fn right_drag_state_clears_when_mouse_up_occurs_outside_active_session() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(18, 8));

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(model.terminal_trackpad_scroll_row, Some(area.y + 1));

        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Right),
                column: area.right(),
                row: area.bottom(),
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert!(
            model.terminal_trackpad_scroll_row.is_none(),
            "right-drag fallback state should clear even if mouse-up lands outside the session area"
        );
    }

    #[test]
    fn right_drag_state_clears_when_active_session_changes() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.sessions.push(crate::model::SessionTab {
            id: "shell-1".to_string(),
            name: "Shell 2".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        update(&mut model, Message::Resize(18, 8));

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(model.terminal_trackpad_scroll_row, Some(area.y + 1));

        update(&mut model, Message::SwitchSession(1));

        assert!(
            model.terminal_trackpad_scroll_row.is_none(),
            "changing the active session should clear right-drag fallback state"
        );
    }

    #[test]
    fn right_drag_state_clears_when_focus_changes() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(18, 8));

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(model.terminal_trackpad_scroll_row, Some(area.y + 1));

        update(&mut model, Message::FocusNext);

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(
            model.terminal_trackpad_scroll_row.is_none(),
            "focus changes should clear right-drag fallback state"
        );
    }

    #[test]
    fn render_model_text_terminal_history_never_reserves_scrollbar_chrome() {
        let mut overflow_model = test_model();
        overflow_model.active_layer = ActiveLayer::Main;
        overflow_model.active_focus = FocusPane::Terminal;
        update(&mut overflow_model, Message::Resize(18, 8));
        for i in 0..12 {
            append_session_line(&mut overflow_model, "shell-0", &format!("L{i}"));
        }
        let overflow_area =
            active_session_content_area(&overflow_model).expect("overflow session area");
        let overflow_buf = render_model_buffer(&overflow_model, 18, 8);
        let overflow_has_scrollbar = (overflow_area.y..overflow_area.bottom()).any(|y| {
            !overflow_buf[(overflow_area.right() - 1, y)]
                .symbol()
                .trim()
                .is_empty()
        });
        assert!(
            !overflow_has_scrollbar,
            "overflowing history should no longer reserve scrollbar chrome on the right edge"
        );

        let mut non_overflow_model = test_model();
        non_overflow_model.active_layer = ActiveLayer::Main;
        non_overflow_model.active_focus = FocusPane::Terminal;
        update(&mut non_overflow_model, Message::Resize(18, 8));
        append_session_line(&mut non_overflow_model, "shell-0", "short");
        let non_overflow_area =
            active_session_content_area(&non_overflow_model).expect("non-overflow session area");
        let non_overflow_buf = render_model_buffer(&non_overflow_model, 18, 8);
        let non_overflow_has_scrollbar =
            (non_overflow_area.y..non_overflow_area.bottom()).any(|y| {
                !non_overflow_buf[(non_overflow_area.right() - 1, y)]
                    .symbol()
                    .trim()
                    .is_empty()
            });
        assert!(
            !non_overflow_has_scrollbar,
            "non-overflowing history should also stay free of scrollbar chrome"
        );
    }

    #[test]
    fn drag_selection_reverses_selected_terminal_cells() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        append_session_line(&mut model, "shell-0", "alpha beta");

        let area = active_session_content_area(&model).expect("active session area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: area.x + 4,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let buf = render_model_buffer(&model, 24, 8);
        assert!(
            buf[(area.x, area.y)].modifier.contains(Modifier::REVERSED),
            "drag selection should reverse the selected terminal cells"
        );
    }

    #[test]
    fn selection_copy_uses_scrollback_viewport_coordinates() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(18, 8));
        for i in 0..12 {
            append_session_line(&mut model, "shell-0", &format!("line-{i}"));
        }

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let mut copied = None;
        handle_mouse_input_with_tools(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
            |text| {
                copied = Some(text.to_string());
                Ok(())
            },
        )
        .expect("selection down succeeds");
        handle_mouse_input_with_tools(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: area.x + 5,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
            |_| Ok(()),
        )
        .expect("selection drag succeeds");
        handle_mouse_input_with_tools(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: area.x + 5,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
            |text| {
                copied = Some(text.to_string());
                Ok(())
            },
        )
        .expect("selection up succeeds");

        assert_eq!(copied.as_deref(), Some("line-7"));
    }

    #[test]
    fn in_place_full_screen_redraw_keeps_previous_snapshot_history() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_text(&mut model, "shell-0", "frame-1");
        replace_alt_screen_text(&mut model, "shell-0", "frame-2");

        let session = model.active_session_tab().expect("active session");
        assert_eq!(session.vt.snapshot_count(), 2);
        assert!(session.vt.has_snapshot_scrollback());

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        let after = render_model_text(&model, 24, 8);
        assert!(after.contains("frame-1"));
        assert!(!after.contains("frame-2"));
    }

    #[test]
    fn style_only_redraw_flood_does_not_evict_meaningful_snapshot_history() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_text(&mut model, "shell-0", "frame-1");
        replace_alt_screen_text(&mut model, "shell-0", "frame-2");

        for index in 0..2200 {
            let style_redraw = if index % 2 == 0 {
                "\x1b[7m\x1b[1;1Hframe-2\x1b[0m"
            } else {
                "\x1b[4m\x1b[1;1Hframe-2\x1b[0m"
            };
            update(
                &mut model,
                Message::PtyOutput("shell-0".to_string(), style_redraw.as_bytes().to_vec()),
            );
        }

        let session = model.active_session_tab().expect("active session");
        assert!(
            session.vt.snapshot_count() <= 3,
            "style-only redraw flood should collapse into a tiny bounded history footprint"
        );
        assert!(session.vt.has_snapshot_scrollback());

        let area = active_session_text_area(&model).expect("active session text area");
        let mut reached_frame_1 = false;
        for _ in 0..3 {
            update(
                &mut model,
                Message::MouseInput(MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
            );
            let after = render_model_text(&model, 24, 8);
            if after.contains("frame-1") {
                reached_frame_1 = true;
                break;
            }
        }

        assert!(
            reached_frame_1,
            "bounded history should still preserve the oldest meaningful frame after redraw flood"
        );
    }

    #[test]
    fn snapshot_scrollback_works_in_alt_screen_after_main_output() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));

        for index in 0..20 {
            append_session_line(&mut model, "shell-0", &format!("seed-{index}"));
        }

        enter_alt_screen_with_text(&mut model, "shell-0", "frame-1");
        replace_alt_screen_text(&mut model, "shell-0", "frame-2");

        let session = model.active_session_tab().expect("active session");
        assert!(session.vt.uses_snapshot_scrollback());
        assert!(session.vt.has_snapshot_scrollback());

        let before = render_model_text(&model, 24, 8);
        assert!(before.contains("frame-2"));
        assert!(!before.contains("frame-1"));

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let after = render_model_text(&model, 24, 8);
        assert!(after.contains("frame-1"));
        assert!(!after.contains("frame-2"));
    }

    #[test]
    fn bottom_aligned_first_frame_does_not_leave_blank_snapshot_history() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));

        enter_alt_screen_with_text(&mut model, "shell-0", "");
        let rows = model
            .active_session_tab()
            .expect("active session")
            .vt
            .rows();
        replace_alt_screen_text(
            &mut model,
            "shell-0",
            &format!("\u{1b}[{};1Htail-frame", rows),
        );

        let session = model.active_session_tab().expect("active session");
        assert_eq!(
            session.vt.snapshot_count(),
            1,
            "first visible full-screen frame should replace the transient blank frame instead of extending history"
        );
        assert!(
            !session.vt.has_snapshot_scrollback(),
            "scrollback must stay disabled when only one meaningful frame exists"
        );

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        let text = render_model_text(&model, 24, 8);
        assert!(text.contains("tail-frame"));
    }

    #[test]
    fn snapshot_scrollback_reveals_previous_full_screen_viewport_after_line_shift() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "shell-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "shell-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );

        assert_eq!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .max_scrollback(),
            0,
            "full-screen updates should not create vt100 row scrollback"
        );

        let before = render_model_text(&model, 24, 8);
        assert!(before.contains("line-6"));
        assert!(!before.contains("line-1"));

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let after = render_model_text(&model, 24, 8);
        assert!(
            after.contains("line-1"),
            "snapshot scrollback should reveal the previous full-screen viewport when the content advanced vertically"
        );
        assert!(!after.contains("line-6"));
    }

    #[test]
    fn full_screen_snapshot_history_does_not_render_scrollbar_when_row_scrollback_is_zero() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "shell-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "shell-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );

        let area = active_session_content_area(&model).expect("active session area");
        let buffer = render_model_buffer(&model, 24, 8);
        let has_scrollbar = (area.y..area.bottom())
            .any(|y| !buffer[(area.right() - 1, y)].symbol().trim().is_empty());

        assert!(
            !has_scrollbar,
            "snapshot history should no longer reserve scrollbar chrome even without vt100 row scrollback"
        );
    }

    #[test]
    fn snapshot_history_keeps_full_terminal_width_without_scrollbar_gutter() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "shell-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "shell-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );

        let content = active_session_content_area(&model).expect("content area");
        let text = active_session_text_area(&model).expect("text area");

        assert_eq!(
            text.width, content.width,
            "snapshot-backed history should keep the full terminal width once scrollbar chrome is removed"
        );
    }

    #[test]
    fn selection_copy_uses_snapshot_viewport_surface_when_viewing_past_full_screen_frame() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "shell-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "shell-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let mut copied = None;
        handle_mouse_input_with_tools(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
            |_| Ok(()),
        )
        .expect("selection down succeeds");
        handle_mouse_input_with_tools(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: area.x + 6,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
            |_| Ok(()),
        )
        .expect("selection drag succeeds");
        handle_mouse_input_with_tools(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: area.x + 6,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
            |text| {
                copied = Some(text.to_string());
                Ok(())
            },
        )
        .expect("selection up succeeds");

        assert_eq!(
            copied.as_deref(),
            Some("line-1"),
            "selection copy should read from the visible snapshot surface instead of the live frame"
        );
    }

    #[test]
    fn snapshot_scrollback_stays_frozen_until_it_returns_to_live() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "shell-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "shell-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        replace_alt_screen_lines(
            &mut model,
            "shell-0",
            &["line-3", "line-4", "line-5", "line-6", "line-7"],
        );

        let frozen = render_model_text(&model, 24, 8);
        assert!(frozen.contains("line-1"));
        assert!(!frozen.contains("line-7"));

        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        let previous = render_model_text(&model, 24, 8);
        assert!(!previous.contains("line-1"));
        assert!(
            previous.contains("line-6") || previous.contains("line-7"),
            "scrolling down from the oldest cached viewport should leave the frozen history view and move toward the newest available content"
        );

        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        let live = render_model_text(&model, 24, 8);
        assert!(live.contains("line-7"));
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .follow_live(),
            "returning to the newest snapshot should restore live-follow mode"
        );
    }

    #[test]
    fn agent_memory_scrollback_uses_in_memory_snapshots_when_frames_do_not_overlap() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &["alpha-1", "alpha-2", "alpha-3", "alpha-4", "alpha-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "agent-0",
            &["beta-1", "beta-2", "beta-3", "beta-4", "beta-5"],
        );

        let session = model.active_session_tab_mut().expect("active session");
        assert!(
            session.vt.has_snapshot_scrollback(),
            "full-screen redraw agents should still keep snapshot history when consecutive frames do not share a vertical overlap that can be normalized into row scrollback"
        );
        assert!(session.vt.scroll_snapshot_up(1));

        let frozen = render_model_text(&model, 24, 8);
        assert!(frozen.contains("alpha-1"));
        assert!(!frozen.contains("beta-1"));
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .viewing_history(),
            "full-screen redraw agents should still enter history view when they truly need snapshot fallback"
        );
    }

    #[test]
    fn codex_status_churn_shift_uses_local_row_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &["status-a", "line-1", "line-2", "line-3", "line-4"],
        );
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[H\x1b[1;1Hstatus-b\x1b[2;1Hline-2\x1b[3;1Hline-3\x1b[4;1Hline-4\x1b[5;1Hline-5".to_vec(),
            ),
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            !session.vt.uses_snapshot_scrollback(),
            "Codex-like redraws with one changing status row should still derive local row history instead of falling back to page-sized snapshot scrolling"
        );
        assert!(
            session.vt.max_scrollback() > 0,
            "a vertical shift hidden by status churn should still contribute at least one local history row"
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        let frozen = render_model_text(&model, 24, 8);
        assert!(frozen.contains("line-1"));
        assert!(!frozen.contains("status-a"));
        assert!(!frozen.contains("line-5"));
    }

    #[test]
    fn codex_coalesced_home_repaints_use_local_row_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[2J\x1b[H\x1b[1;1Hheader\x1b[2;1Hline-1\x1b[3;1Hline-2\x1b[4;1Hline-3\x1b[5;1Hline-4\x1b[6;1Hline-5\x1b[7;1Hfooter".to_vec(),
            ),
        );
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                [
                    b"\x1b[H\x1b[1;1Hheader\x1b[2;1Hline-2\x1b[3;1Hline-3\x1b[4;1Hprogress\x1b[5;1Hline-5\x1b[6;1Hline-6\x1b[7;1Hfooter".as_slice(),
                    b"\x1b[H\x1b[1;1Hheader\x1b[2;1Hline-3\x1b[3;1Hprogress\x1b[4;1Hprogress\x1b[5;1Hline-6\x1b[6;1Hline-7\x1b[7;1Hfooter".as_slice(),
                ]
                .concat(),
            ),
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            !session.vt.uses_snapshot_scrollback(),
            "coalesced home-repaint redraws should still promote Codex panes into row-based local history"
        );
        assert_eq!(
            session.vt.max_scrollback(),
            2,
            "each repaint shift inside one payload should contribute its own scrolled-off line"
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        let frozen = render_model_text(&model, 24, 8);
        assert!(frozen.contains("line-2"));
        assert!(!frozen.contains("line-7"));
    }

    #[test]
    fn agent_memory_scrollback_preserves_coalesced_full_screen_redraw_frames() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[?1049h\x1b[2J\x1b[Hframe-1\x1b[2J\x1b[Hframe-2".to_vec(),
            ),
        );

        let session = model.active_session_tab_mut().expect("active session");
        assert!(
            session.vt.has_snapshot_scrollback(),
            "coalesced redraw capture should still populate snapshot history even when wheel routing is PTY-owned"
        );
        assert!(session.vt.scroll_snapshot_up(1));

        let frozen = render_model_text(&model, 24, 8);
        assert!(frozen.contains("frame-1"));
        assert!(!frozen.contains("frame-2"));
    }

    #[test]
    fn agent_mouse_wheel_forwards_to_pty_when_mouse_reporting_is_enabled() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[?1000h\x1b[?1006hframe-1".to_vec(),
            ),
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        let forwarded = model
            .pending_pty_inputs()
            .back()
            .expect("queued mouse input");
        assert_eq!(forwarded.session_id, "agent-0");
        assert_eq!(forwarded.bytes, b"\x1b[<64;1;1M".to_vec());
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .follow_live(),
            "PTY-driven agent scrolling should keep the local viewport pinned to live output"
        );
    }

    /// Helper that spins up a single Claude-Code-like agent session with SGR
    /// mouse reporting (DECSET 1000+1006) already negotiated.  Used by the
    /// left-click forwarding tests below to avoid copy-pasting fixture setup.
    fn model_with_agent_mouse_reporting_enabled() -> Model {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[?1000h\x1b[?1006hframe-1".to_vec(),
            ),
        );
        assert_eq!(
            active_session_scroll_routing(&model),
            ScrollInputRouting::PtyMouse,
            "fixture precondition: agent should be in PTY mouse routing",
        );
        model
    }

    #[test]
    fn agent_left_click_down_forwards_sgr_press_to_pty() {
        let mut model = model_with_agent_mouse_reporting_enabled();
        let area = active_session_text_area(&model).expect("active session text area");

        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        let forwarded = model
            .pending_pty_inputs()
            .back()
            .expect("queued mouse input");
        assert_eq!(forwarded.session_id, "agent-0");
        assert_eq!(forwarded.bytes, b"\x1b[<0;1;1M".to_vec());
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .selection()
                .is_none(),
            "PTY-forwarded clicks must not also start a local text selection",
        );
    }

    #[test]
    fn agent_left_click_up_forwards_sgr_release_to_pty() {
        let mut model = model_with_agent_mouse_reporting_enabled();
        let area = active_session_text_area(&model).expect("active session text area");

        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("left down should succeed");

        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("left up should succeed");

        assert!(handled);
        let forwarded = model
            .pending_pty_inputs()
            .back()
            .expect("queued mouse input");
        assert_eq!(forwarded.session_id, "agent-0");
        assert_eq!(
            forwarded.bytes,
            b"\x1b[<0;1;1m".to_vec(),
            "release must terminate with lowercase 'm' per SGR 1006",
        );
    }

    #[test]
    fn agent_left_drag_forwards_sgr_motion_event_to_pty() {
        let mut model = model_with_agent_mouse_reporting_enabled();
        let area = active_session_text_area(&model).expect("active session text area");

        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("left down should succeed");

        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: area.x.saturating_add(2),
                row: area.y.saturating_add(1),
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("left drag should succeed");

        assert!(handled);
        let forwarded = model
            .pending_pty_inputs()
            .back()
            .expect("queued mouse input");
        assert_eq!(forwarded.session_id, "agent-0");
        assert_eq!(
            forwarded.bytes,
            b"\x1b[<32;3;2M".to_vec(),
            "drag encodes base button 0 plus the motion flag 32",
        );
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .selection()
                .is_none(),
            "drag forwarding must not silently update the local selection either",
        );
    }

    #[test]
    fn shift_left_click_bypasses_pty_forwarding_and_starts_local_selection() {
        let mut model = model_with_agent_mouse_reporting_enabled();
        let area = active_session_text_area(&model).expect("active session text area");

        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::SHIFT,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        assert!(
            model.pending_pty_inputs().is_empty(),
            "Shift+click should act as a local selection bypass, never touching the PTY",
        );
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .selection()
                .is_some(),
            "Shift+click must start a local text selection for copy UX",
        );
    }

    #[test]
    fn agent_left_click_without_mouse_reporting_still_starts_local_selection() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        // No DECSET 1000/1006 → session_scroll_routing must stay LocalViewport.
        assert_eq!(
            active_session_scroll_routing(&model),
            ScrollInputRouting::LocalViewport,
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        assert!(
            model.pending_pty_inputs().is_empty(),
            "agents without mouse reporting should keep click handling on the local selection path",
        );
        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .selection()
                .is_some(),
            "local-mode left click must begin a text selection as before",
        );
    }

    #[test]
    fn agent_sessions_keep_full_terminal_width_without_scrollbar_overlay() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[?1000h\x1b[?1006h\x1b[?1049h\x1b[2J\x1b[Hframe-1".to_vec(),
            ),
        );
        update(
            &mut model,
            Message::PtyOutput("agent-0".to_string(), b"\x1b[2J\x1b[Hframe-2".to_vec()),
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            session.vt.has_viewport_scrollback(),
            "precondition: local snapshot history still exists even though wheel handling is delegated to PTY"
        );
        assert!(
            !session_has_scrollbar(session),
            "gwt should never expose a local scrollbar overlay for agent panes"
        );

        let content = active_session_content_area(&model).expect("content area");
        let text = active_session_text_area(&model).expect("text area");
        assert_eq!(
            text.width, content.width,
            "removing scrollbar chrome should keep the full pane width for terminal rendering"
        );
    }

    #[test]
    fn alternate_screen_agent_without_mouse_reporting_uses_local_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "agent-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );
        update(
            &mut model,
            Message::PtyOutput("agent-0".to_string(), b"\x1b[2J\x1b[Hframe-2".to_vec()),
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        assert!(
            model.pending_pty_inputs().is_empty(),
            "agents without mouse reporting should keep wheel input inside gwt's local viewport cache"
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            !session_has_scrollbar(session),
            "local viewport scrolling should still render without any scrollbar overlay"
        );
        assert!(
            session.vt.viewing_history(),
            "wheel input should move alternate-screen agents into local history when no PTY mouse capability was negotiated"
        );
    }

    #[test]
    fn non_alternate_screen_agent_without_mouse_reporting_uses_local_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                (1..=40)
                    .map(|line| format!("line-{line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into_bytes(),
            ),
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        assert!(
            model.pending_pty_inputs().is_empty(),
            "non alternate-screen agents should keep using local scrollback when no PTY scroll capability was negotiated"
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            !session_has_scrollbar(session),
            "local scrollback panes should also stay free of scrollbar overlay"
        );
        assert!(
            session.vt.viewing_history(),
            "wheel input should move non alternate-screen panes into local history"
        );
    }

    #[test]
    fn snapshot_backed_agent_without_mouse_reporting_uses_local_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[2J\x1b[Hline-1\r\nline-2\r\nline-3\r\nline-4\r\nline-5".to_vec(),
            ),
        );
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[2J\x1b[Hline-2\r\nline-3\r\nline-4\r\nline-5\r\nline-6".to_vec(),
            ),
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            !session.vt.uses_snapshot_scrollback(),
            "redraw-shift normalization should promote Codex-like panes into row-based local history before the first wheel step"
        );

        let area = active_session_text_area(&model).expect("active session text area");
        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("mouse input should succeed");

        assert!(handled);
        assert!(
            model.pending_pty_inputs().is_empty(),
            "snapshot-backed agents without mouse reporting should still scroll locally instead of synthesizing arrow-key input"
        );

        let session = model.active_session_tab().expect("active session");
        assert!(
            !session_has_scrollbar(session),
            "even row-based local history should not reintroduce a scrollbar overlay"
        );
        assert!(
            session.vt.viewing_history(),
            "wheel input should move the local viewport into history rather than injecting arrow keys into the PTY"
        );
    }

    #[test]
    fn render_model_text_codex_collapses_identical_progress_blocks() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(60, 14));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &[
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "────────────────────────────────────────",
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "• Working",
            ],
        );

        let text = render_model_text(&model, 80, 20);
        assert_eq!(text.matches("• Explored").count(), 1);
        assert_eq!(text.matches("  └ Search alpha").count(), 1);
    }

    #[test]
    fn render_model_text_codex_keeps_distinct_progress_blocks() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(60, 14));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &[
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "────────────────────────────────────────",
                "• cache summary updated",
                "",
                "• Explored",
                "  └ Read spec.mdsh",
                "",
                "• Working",
            ],
        );

        let text = render_model_text(&model, 80, 20);
        assert_eq!(text.matches("• Explored").count(), 2);
        assert!(text.contains("  └ Search alpha"));
        assert!(text.contains("  └ Read spec.mdsh"));
    }

    #[test]
    fn render_model_text_non_codex_keeps_identical_progress_blocks() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(60, 14));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &[
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "────────────────────────────────────────",
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "• Working",
            ],
        );

        let text = render_model_text(&model, 80, 20);
        assert_eq!(text.matches("• Explored").count(), 2);
        assert_eq!(text.matches("  └ Search alpha").count(), 2);
    }

    #[test]
    fn render_model_text_codex_with_selection_keeps_identical_progress_blocks() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(60, 14));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &[
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "────────────────────────────────────────",
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "• Working",
            ],
        );
        let session = model.active_session_tab_mut().expect("active session");
        session
            .vt
            .begin_selection(crate::model::TerminalCell { row: 0, col: 0 });
        session
            .vt
            .update_selection(crate::model::TerminalCell { row: 0, col: 5 });

        let text = render_model_text(&model, 80, 20);
        assert_eq!(text.matches("• Explored").count(), 2);
        assert_eq!(text.matches("  └ Search alpha").count(), 2);
    }

    #[test]
    fn normalized_codex_progress_parser_preserves_cursor_state() {
        let mut parser = vt100::Parser::new(14, 60, 0);
        parser.process(
            concat!(
                "• contract swapped\r\n",
                "\r\n",
                "• Explored\r\n",
                "  └ Search alpha\r\n",
                "\r\n",
                "────────────────────────────────────────\r\n",
                "• contract swapped\r\n",
                "\r\n",
                "• Explored\r\n",
                "  └ Search alpha\r\n",
                "\r\n",
                "• Working\r\n",
            )
            .as_bytes(),
        );
        parser.process(b"\x1b[12;5H\x1b[?25l");

        let normalized = normalized_codex_progress_parser(parser.screen()).expect("normalized");
        assert_eq!(normalized.screen().cursor_position(), (5, 4));
        assert!(normalized.screen().hide_cursor());
    }

    #[test]
    fn agent_trackpad_drag_forwards_repeated_mouse_wheel_steps_to_pty() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Claude Code",
            "claude",
            crate::model::AgentColor::Green,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[?1002h\x1b[?1006hframe-1".to_vec(),
            ),
        );

        let area = active_session_text_area(&model).expect("active session text area");
        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("right down should succeed");

        let handled = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Right),
                column: area.x,
                row: area.y.saturating_add(2),
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("right drag should succeed");

        assert!(handled);
        let forwarded = model
            .pending_pty_inputs()
            .back()
            .expect("queued wheel input");
        assert_eq!(forwarded.session_id, "agent-0");
        assert_eq!(forwarded.bytes, b"\x1b[<64;1;3M\x1b[<64;1;3M".to_vec());
    }

    #[test]
    fn alternate_screen_agent_trackpad_drag_routes_to_local_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &["line-1", "line-2", "line-3", "line-4", "line-5"],
        );
        replace_alt_screen_lines(
            &mut model,
            "agent-0",
            &["line-2", "line-3", "line-4", "line-5", "line-6"],
        );

        assert_eq!(
            active_session_scroll_routing(&model),
            ScrollInputRouting::LocalViewport,
            "agents without mouse reporting should keep trackpad drags on the local viewport path"
        );

        let area = active_session_text_area(&model).expect("active session text area");
        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("right down should succeed");

        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Right),
                column: area.x,
                row: area.bottom().saturating_sub(1),
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("right drag should succeed");

        assert!(
            model.pending_pty_inputs().is_empty(),
            "trackpad drag should not be translated into arrow-key input for local-scroll agents"
        );
    }

    #[test]
    fn snapshot_backed_agent_trackpad_drag_routes_to_local_scrollback() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;

        update(&mut model, Message::Resize(24, 8));
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[2J\x1b[Hline-1\r\nline-2\r\nline-3\r\nline-4\r\nline-5".to_vec(),
            ),
        );
        update(
            &mut model,
            Message::PtyOutput(
                "agent-0".to_string(),
                b"\x1b[2J\x1b[Hline-2\r\nline-3\r\nline-4\r\nline-5\r\nline-6".to_vec(),
            ),
        );

        assert_eq!(
            active_session_scroll_routing(&model),
            ScrollInputRouting::LocalViewport,
            "snapshot-backed agents without mouse reporting should keep trackpad drags on the local viewport path"
        );

        let area = active_session_text_area(&model).expect("active session text area");
        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("right down should succeed");

        handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Right),
                column: area.x,
                row: area.bottom().saturating_sub(1),
                modifiers: KeyModifiers::NONE,
            },
            |_| Ok(()),
        )
        .expect("right drag should succeed");

        assert!(
            model.pending_pty_inputs().is_empty(),
            "snapshot-backed agents should keep trackpad drags local instead of injecting arrow-key input"
        );
    }

    #[test]
    fn toggle_layer_resizes_active_terminal_viewport_immediately() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        update(&mut model, Message::Resize(100, 24));
        let before = model
            .active_session_tab()
            .expect("active session")
            .vt
            .clone();
        assert_eq!(before.cols(), 98);
        assert_eq!(before.rows(), 21);

        update(&mut model, Message::ToggleLayer);

        let after = &model.active_session_tab().expect("active session").vt;
        assert_eq!(after.cols(), 48);
        assert_eq!(after.rows(), 21);
    }

    #[test]
    fn exited_pty_sessions_are_removed_automatically() {
        let mut model = test_model();
        let session_id = "shell-exit".to_string();
        model.sessions.push(crate::model::SessionTab {
            id: session_id.clone(),
            name: "Ephemeral".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.active_session = 1;

        spawn_pty_for_session(
            &mut model,
            &session_id,
            gwt_terminal::pty::SpawnConfig {
                command: "/bin/echo".to_string(),
                args: vec!["done".to_string()],
                cols: 80,
                rows: 24,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: None,
            },
        )
        .expect("spawn exiting pty");

        drive_ticks_until(
            &mut model,
            |m| !m.pty_handles.contains_key(&session_id),
            "pty exit detection",
        );

        assert_eq!(model.session_count(), 1);
        assert_eq!(model.active_session, 0);
    }

    #[test]
    fn render_model_text_status_bar_keeps_branch_context_and_branch_hints() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.sessions[0] = crate::model::SessionTab {
            id: "shell-0".to_string(),
            name: "Shell: feature/status-bar".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        };

        let rendered = render_model_text(&model, 220, 24);
        assert!(rendered.contains("feature/status-bar"));
        assert!(rendered.contains("type: Shell"));
        assert!(rendered.contains("Enter:wizard"));
        assert!(rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_git_view_hints_include_escape_to_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::GitView;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);
        assert!(rendered.contains("Enter:expand"));
        assert!(rendered.contains("r:refresh"));
        assert!(rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_issues_detail_hints_show_escape_back_not_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;
        model.issues.detail_view = true;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Esc:back"));
        assert!(!rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_profiles_create_hints_show_escape_cancel_not_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;
        model.profiles.mode = screens::profiles::ProfileMode::CreateProfile;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Esc:cancel"));
        assert!(!rendered.contains("Esc:term"));
    }

    #[test]
    fn render_model_text_profiles_list_hints_prefer_tab_for_pane_navigation() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Tab:next pane"));
        assert!(rendered.contains("Shift+Tab:prev pane"));
        assert!(!rendered.contains("Ctrl+←→"));
    }

    #[test]
    fn render_model_text_settings_list_hints_include_sub_tab_controls() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Ctrl+←→:sub-tab"));
    }

    #[test]
    fn render_model_text_git_view_hints_omit_sub_tab_controls() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::GitView;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(!rendered.contains("Ctrl+←→:sub-tab"));
    }

    #[test]
    fn render_model_text_git_view_hints_show_expand_and_refresh_actions() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::GitView;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Enter:expand"));
        assert!(rendered.contains("r:refresh"));
        assert!(!rendered.contains("Enter:action"));
    }

    #[test]
    fn render_model_text_versions_hints_show_refresh_without_enter_action() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Versions;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("r:refresh"));
        assert!(!rendered.contains("Enter:action"));
    }

    #[test]
    fn render_model_text_issues_list_hints_show_search_and_refresh_actions() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("/:search"));
        assert!(rendered.contains("r:refresh"));
        assert!(rendered.contains("Enter:detail"));
    }

    #[test]
    fn render_model_text_pr_dashboard_detail_hints_show_close_and_refresh_actions() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::PrDashboard;
        model.active_focus = FocusPane::TabContent;
        model.pr_dashboard.detail_view = true;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Enter:close"));
        assert!(rendered.contains("r:refresh"));
        assert!(rendered.contains("Esc:back"));
    }

    #[test]
    fn render_model_text_management_omits_standalone_header_banner() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.repo_path = PathBuf::from("/tmp/demo/project-repo");
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/banner".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/demo/project-repo-feature-banner")),
            upstream: None,
        }];

        let rendered = render_model_text(&model, 120, 16);

        assert!(
            !rendered.contains(" gwt | "),
            "management should rely on pane titles instead of a standalone header banner"
        );
    }

    #[test]
    fn render_model_text_management_top_row_uses_pane_title_chrome() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/top-row".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/demo/project-repo-feature-top-row")),
            upstream: None,
        }];

        let rendered = render_model_text(&model, 120, 16);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(
            first_line.contains("Branches"),
            "top row should start with pane title chrome once the standalone header is removed"
        );
    }

    #[test]
    fn render_model_text_non_branches_management_top_row_uses_pane_title_chrome() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 120, 16);
        let mut lines = rendered.lines();
        let first_line = lines.next().unwrap_or_default();
        let second_line = lines.next().unwrap_or_default();

        assert!(
            !rendered.contains(" gwt | "),
            "non-Branches tabs should also omit the standalone management banner"
        );
        assert!(
            first_line.contains("Settings"),
            "non-Branches top row should keep the active pane title chrome visible"
        );
        assert!(
            second_line.contains("General"),
            "non-Branches content should start immediately below the pane title chrome"
        );
    }

    #[test]
    fn render_model_text_standard_width_branches_title_keeps_nearby_tabs_visible() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Branches"));
        assert!(
            first_line.contains("Issues"),
            "standard-width Branches title should keep the next nearby tab visible"
        );
        assert!(
            first_line.contains("PRs"),
            "standard-width Branches title should keep multiple nearby tabs visible"
        );
        assert!(
            !first_line.contains("Profiles"),
            "standard-width Branches title should not try to render the full strip"
        );
    }

    #[test]
    fn render_model_text_standard_width_non_branches_title_keeps_nearby_tabs_visible() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Branches"));
        assert!(first_line.contains("Issues"));
        assert!(
            first_line.contains("PRs"),
            "standard-width non-Branches title should keep the next nearby tab visible"
        );
        assert!(
            !first_line.contains("Profiles"),
            "standard-width non-Branches title should not try to render distant tabs"
        );
    }

    #[test]
    fn compact_tab_window_start_keeps_active_tab_visible_for_single_slot_window() {
        assert_eq!(compact_tab_window_start(8, 0, 1), 0);
        assert_eq!(compact_tab_window_start(8, 3, 1), 3);
        assert_eq!(compact_tab_window_start(8, 7, 1), 7);
    }

    #[test]
    fn render_model_text_medium_width_management_title_still_prefers_nearby_tabs() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 120, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Branches"));
        assert!(first_line.contains("Issues"));
        assert!(
            first_line.contains("PRs"),
            "when the full tab strip does not fit, medium-width panes should still keep nearby tabs visible"
        );
        assert!(
            !first_line.contains("Profiles"),
            "medium-width panes should still omit distant tabs until the full strip fits"
        );
    }

    #[test]
    fn render_model_text_extra_wide_management_title_keeps_tab_strip() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 220, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("Branches"));
        assert!(first_line.contains("Issues"));
        // SPEC-12 Phase 9: the Specs tab is now a top-level peer, so the
        // extra-wide tab strip should include it.
        assert!(first_line.contains("Specs"));
    }

    fn shell_tab(id: &str, name: &str) -> crate::model::SessionTab {
        crate::model::SessionTab {
            id: id.to_string(),
            name: name.to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn render_model_text_standard_width_session_title_collapses_to_active_session() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
            shell_tab("shell-3", "Shell: feature/session-four"),
        ];
        model.active_session = 2;

        let rendered = render_model_text(&model, 80, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("session-three"));
        assert!(
            first_line.contains("3/4"),
            "compact session title should keep the active index/count visible so multi-session context survives the collapse"
        );
        assert!(
            !first_line.contains("session-one"),
            "standard-width session title should collapse to the active session instead of truncating the strip"
        );
    }

    #[test]
    fn render_model_text_medium_width_session_title_still_collapses_when_strip_does_not_fit() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
            shell_tab("shell-3", "Shell: feature/session-four"),
        ];
        model.active_session = 1;

        let rendered = render_model_text(&model, 120, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("session-two"));
        assert!(
            first_line.contains("2/4"),
            "medium-width compact session title should also keep the active index/count visible"
        );
        assert!(
            !first_line.contains("session-one"),
            "medium-width session pane should still collapse when the full strip would truncate"
        );
    }

    #[test]
    fn render_model_text_extra_wide_session_title_keeps_full_strip() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
            shell_tab("shell-3", "Shell: feature/session-four"),
        ];
        model.active_session = 1;

        let rendered = render_model_text(&model, 220, 24);
        let first_line = rendered.lines().next().unwrap_or_default();

        assert!(first_line.contains("session-one"));
        assert!(first_line.contains("session-two"));
        assert!(first_line.contains("session-three"));
        assert!(first_line.contains("session-four"));
        assert!(
            !first_line.contains("2/4"),
            "extra-wide panes should keep the full strip rather than the compact index/count chrome"
        );
    }

    #[test]
    fn build_session_title_agent_tabs_prefer_persisted_branch_names_in_full_strip() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let (claude, claude_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/claude-branch",
            AgentId::ClaudeCode,
            crate::model::AgentColor::Yellow,
        );
        let (codex, codex_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/codex-branch",
            AgentId::Codex,
            crate::model::AgentColor::Cyan,
        );

        let mut model = test_model();
        model.sessions = vec![claude, codex];
        model.active_session = 0;

        let title = build_session_title_with(&model, 220, sessions_dir.path());
        let text = line_text(&title);

        assert!(text.contains("feature/claude-branch"));
        assert!(text.contains("feature/codex-branch"));
        assert!(!text.contains("Claude Code"));
        assert!(!text.contains("Codex"));

        let _ = fs::remove_file(claude_path);
        let _ = fs::remove_file(codex_path);
    }

    #[test]
    fn build_session_title_compact_agent_tabs_show_active_branch_name_and_count() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let mut cleanup = Vec::new();
        let mut sessions = Vec::new();

        for (branch, agent_id, color) in [
            (
                "feature/branch-one",
                AgentId::ClaudeCode,
                crate::model::AgentColor::Yellow,
            ),
            (
                "feature/branch-two",
                AgentId::Codex,
                crate::model::AgentColor::Cyan,
            ),
            (
                "feature/branch-three",
                AgentId::Gemini,
                crate::model::AgentColor::Magenta,
            ),
            (
                "feature/branch-four",
                AgentId::ClaudeCode,
                crate::model::AgentColor::Yellow,
            ),
            (
                "feature/branch-five",
                AgentId::Codex,
                crate::model::AgentColor::Cyan,
            ),
            (
                "feature/branch-six",
                AgentId::Gemini,
                crate::model::AgentColor::Magenta,
            ),
            (
                "feature/branch-seven",
                AgentId::ClaudeCode,
                crate::model::AgentColor::Yellow,
            ),
            (
                "feature/branch-eight",
                AgentId::Codex,
                crate::model::AgentColor::Cyan,
            ),
        ] {
            let (session, path) = persist_agent_tab(sessions_dir.path(), branch, agent_id, color);
            sessions.push(session);
            cleanup.push(path);
        }

        let mut model = test_model();
        model.sessions = sessions;
        model.active_session = 5;

        let title = build_session_title_with(&model, 40, sessions_dir.path());
        let text = line_text(&title);

        assert!(text.contains("6/8"));
        assert!(text.contains("feature/branch-six"));
        assert!(!text.contains("Gemini CLI"));

        for path in cleanup {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn build_session_title_agent_tabs_keep_identity_colors_and_use_modifiers_for_active_state() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let (claude, claude_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/claude-active",
            AgentId::ClaudeCode,
            crate::model::AgentColor::Yellow,
        );
        let (codex, codex_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/codex-idle",
            AgentId::Codex,
            crate::model::AgentColor::Cyan,
        );
        let (gemini, gemini_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/gemini-idle",
            AgentId::Gemini,
            crate::model::AgentColor::Magenta,
        );

        let mut model = test_model();
        model.sessions = vec![claude, codex, gemini];
        model.active_session = 0;
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        let title = build_session_title_with(&model, 220, sessions_dir.path());
        let claude_span = title
            .spans
            .iter()
            .find(|span| span.content.contains("feature/claude-active"))
            .expect("claude span");
        let codex_span = title
            .spans
            .iter()
            .find(|span| span.content.contains("feature/codex-idle"))
            .expect("codex span");
        let gemini_span = title
            .spans
            .iter()
            .find(|span| span.content.contains("feature/gemini-idle"))
            .expect("gemini span");

        assert_eq!(claude_span.style.fg, Some(Color::Yellow));
        assert!(claude_span.style.add_modifier.contains(Modifier::BOLD));
        assert!(claude_span
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(codex_span.style.fg, Some(Color::Cyan));
        assert!(codex_span.style.add_modifier.contains(Modifier::DIM));
        assert_eq!(gemini_span.style.fg, Some(Color::Magenta));
        assert!(gemini_span.style.add_modifier.contains(Modifier::DIM));

        let _ = fs::remove_file(claude_path);
        let _ = fs::remove_file(codex_path);
        let _ = fs::remove_file(gemini_path);
    }

    #[test]
    fn build_session_title_management_unfocused_dims_active_session_identity() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let (claude, claude_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/claude-active",
            AgentId::ClaudeCode,
            crate::model::AgentColor::Yellow,
        );
        let (codex, codex_path) = persist_agent_tab(
            sessions_dir.path(),
            "feature/codex-idle",
            AgentId::Codex,
            crate::model::AgentColor::Cyan,
        );

        let mut model = test_model();
        model.sessions = vec![claude, codex];
        model.active_session = 0;
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        let title = build_session_title_with(&model, 220, sessions_dir.path());
        let claude_span = title
            .spans
            .iter()
            .find(|span| span.content.contains("feature/claude-active"))
            .expect("claude span");

        assert_eq!(claude_span.style.fg, Some(Color::Yellow));
        assert!(claude_span.style.add_modifier.contains(Modifier::DIM));
        assert!(!claude_span.style.add_modifier.contains(Modifier::BOLD));
        assert!(!claude_span
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));

        let _ = fs::remove_file(claude_path);
        let _ = fs::remove_file(codex_path);
    }

    #[test]
    fn render_model_text_grid_session_titles_include_index_and_icon() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
            shell_tab("shell-2", "Shell: feature/session-three"),
        ];
        model.active_session = 1;

        let rendered = render_model_text(&model, 120, 24);

        assert!(
            rendered.contains("1:"),
            "grid pane titles should expose a stable numeric affordance for the first session"
        );
        assert!(
            rendered.contains("2:"),
            "grid pane titles should expose a stable numeric affordance for the active session"
        );
        assert!(
            rendered.contains("3:"),
            "grid pane titles should expose a stable numeric affordance for later sessions"
        );
        assert!(
            rendered.contains(crate::theme::icon::SESSION_SHELL),
            "grid pane titles should preserve the session-type icon instead of showing name-only chrome"
        );
    }

    #[test]
    fn render_model_buffer_grid_active_session_uses_unfocused_border_when_management_has_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
        ];
        model.active_session = 0;

        let size = Rect::new(0, 0, 120, 24);
        let [_, session_area] = management_split(size);
        let pane_area = grid_session_pane_area(session_area, model.sessions.len(), 0)
            .expect("active grid pane area");

        let buffer = render_model_buffer(&model, size.width, size.height);

        assert_eq!(buffer[(pane_area.x, pane_area.y)].symbol(), "╭");
        assert_eq!(buffer[(pane_area.x, pane_area.y)].fg, Color::Gray);
    }

    #[test]
    fn render_model_buffer_grid_active_session_keeps_focused_border_when_terminal_has_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.session_layout = SessionLayout::Grid;
        model.sessions = vec![
            shell_tab("shell-0", "Shell: feature/session-one"),
            shell_tab("shell-1", "Shell: feature/session-two"),
        ];
        model.active_session = 0;

        let size = Rect::new(0, 0, 120, 24);
        let [_, session_area] = management_split(size);
        let pane_area = grid_session_pane_area(session_area, model.sessions.len(), 0)
            .expect("active grid pane area");

        let buffer = render_model_buffer(&model, size.width, size.height);

        assert_eq!(buffer[(pane_area.x, pane_area.y)].symbol(), "╔");
        assert_eq!(buffer[(pane_area.x, pane_area.y)].fg, Color::Yellow);
    }

    #[test]
    fn branch_live_session_rendering_highlights_the_active_agent_indicator() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");

        let mut model = Model::new(repo_path.clone());
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
            upstream: None,
        }];

        let running = AgentSession::new(&selected_worktree, "feature/test", AgentId::Codex);
        running.save(dir.path()).expect("persist running session");
        SessionRuntimeState::from_hook_event("PostToolUse")
            .expect("running runtime")
            .save(&runtime_state_path(dir.path(), &running.id))
            .expect("persist running runtime");

        model.sessions = vec![crate::model::SessionTab {
            id: running.id.clone(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        }];
        model.active_session = 0;
        model.branches.live_session_summaries =
            branch_live_session_summaries_with(&model, dir.path());

        let rendered = render_model_buffer(&model, 80, 8);
        let active_indicator = rendered
            .content
            .iter()
            .find(|cell| matches!(cell.symbol(), "◐" | "◓" | "◑" | "◒"))
            .expect("active agent indicator");

        assert!(
            active_indicator.modifier.contains(Modifier::REVERSED),
            "the active agent's branch-row indicator should be visually highlighted"
        );
    }

    #[test]
    fn branch_live_session_rendering_adds_a_shell_indicator_for_the_active_branch_shell() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");

        let mut model = Model::new(repo_path.clone());
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
            upstream: None,
        }];
        model.sessions = vec![crate::model::SessionTab {
            id: "shell-0".to_string(),
            name: "Shell: feature/test".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        }];
        model.active_session = 0;
        model.branches.live_session_summaries =
            branch_live_session_summaries_with(&model, dir.path());

        let rendered = render_model_buffer(&model, 80, 8);
        let shell_indicator = rendered
            .content
            .iter()
            .find(|cell| cell.symbol() == "▸")
            .expect("active shell indicator");

        assert!(
            shell_indicator.modifier.contains(Modifier::REVERSED),
            "the active shell branch indicator should use the same highlighted treatment as the active agent indicator"
        );
    }

    #[test]
    fn render_model_text_terminal_hints_include_grouped_global_shortcuts() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("Ctrl+G:b/i/s g c []/1-9 z ?"));
    }

    #[test]
    fn render_model_text_terminal_hints_include_focus_and_quit_shortcuts() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        let rendered = render_model_text(&model, 220, 24);

        assert!(rendered.contains("C-g Tab:focus"));
        assert!(rendered.contains("^C×2"));
    }

    #[test]
    fn render_model_text_terminal_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions[0] = crate::model::SessionTab {
            id: "shell-0".to_string(),
            name: "Shell: feature/compact-footer".to_string(),
            tab_type: SessionTabType::Shell,
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        };

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("Ctrl+G:b/i/s g c []/1-9 z ?"));
        assert!(rendered.contains("C-g Tab:focus"));
        assert!(rendered.contains("^C×2"));
    }

    #[test]
    fn render_model_text_branches_list_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);

        // Compact hints at 80-col width
        assert!(rendered.contains("↑↓ mv"));
        assert!(rendered.contains("←→ tab"));
    }

    #[test]
    fn render_model_text_branch_detail_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/compact-detail".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from(
                "/tmp/demo/project-repo-feature-compact-detail",
            )),
            upstream: None,
        }];

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("←→ sec  ↵ act  S↵ sh"));
        assert!(rendered.contains("mvf?"));
        assert!(rendered.contains("C-g↔P  Esc←"));
    }

    #[test]
    fn render_model_text_issues_list_hints_remain_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        let rendered = render_model_text(&model, 80, 24);

        assert!(rendered.contains("↑↓ sel  ↵ dtl  / srch  r rfsh"));
        assert!(rendered.contains("C-g Tab"));
        assert!(rendered.contains("Esc term"));
    }

    #[test]
    fn render_model_text_branch_detail_title_includes_selected_branch_name() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/title-context".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-title-context")),
            upstream: None,
        }];

        let rendered = render_model_text(&model, 160, 24);
        let title_line = rendered
            .lines()
            .find(|line| line.contains("Overview") && line.contains("Sessions"))
            .expect("detail title line");
        assert!(
            title_line.contains("feature/title-context"),
            "detail title should keep the selected branch name visible"
        );
    }

    #[test]
    fn render_model_text_branch_detail_title_falls_back_without_selection() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches.clear();

        let rendered = render_model_text(&model, 160, 24);
        let title_line = rendered
            .lines()
            .find(|line| line.contains("Overview") && line.contains("Sessions"))
            .expect("detail title line");
        assert!(
            title_line.contains("No branch selected"),
            "detail title should fall back when no branch is selected"
        );
    }

    #[test]
    fn ctrl_click_on_url_invokes_opener_with_full_url() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.terminal_size = (80, 24);
        let expected_url = "https://example.com/docs";
        update(
            &mut model,
            Message::PtyOutput("shell-0".to_string(), expected_url.as_bytes().to_vec()),
        );
        let area = active_session_content_area(&model).expect("active session area");
        let region = crate::renderer::collect_url_regions(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .screen(),
            Rect::new(0, 0, area.width, area.height),
        )
        .into_iter()
        .find(|region| region.url == expected_url)
        .expect("url region");

        let mut opened = None;
        let opened_result = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + region.start_col,
                row: area.y + region.row,
                modifiers: KeyModifiers::CONTROL,
            },
            |url| {
                opened = Some(url.to_string());
                Ok(())
            },
        )
        .expect("mouse handler succeeds");

        assert!(opened_result);
        assert_eq!(opened.as_deref(), Some(expected_url));
    }

    #[test]
    fn ctrl_click_on_codex_url_uses_normalized_hit_testing() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        model.sessions = vec![agent_session_tab(
            "Codex",
            "codex",
            crate::model::AgentColor::Cyan,
        )];
        model.active_session = 0;
        let expected_url = "https://example.com/docs";

        update(&mut model, Message::Resize(80, 20));
        enter_alt_screen_with_lines(
            &mut model,
            "agent-0",
            &[
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                "────────────────────────────────────────",
                "• contract swapped",
                "",
                "• Explored",
                "  └ Search alpha",
                "",
                expected_url,
            ],
        );

        let area = active_session_text_area(&model).expect("active session area");
        let region = model
            .active_session_tab()
            .expect("active session")
            .vt
            .with_visible_screen(|screen| {
                let normalized = normalized_codex_progress_parser(screen).expect("normalized");
                crate::renderer::collect_url_regions(
                    normalized.screen(),
                    Rect::new(0, 0, area.width, area.height),
                )
                .into_iter()
                .find(|region| region.url == expected_url)
                .expect("url region")
            });

        let mut opened = None;
        let opened_result = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + region.start_col,
                row: area.y + region.row,
                modifiers: KeyModifiers::CONTROL,
            },
            |url| {
                opened = Some(url.to_string());
                Ok(())
            },
        )
        .expect("mouse handler succeeds");

        assert!(opened_result);
        assert_eq!(opened.as_deref(), Some(expected_url));
    }

    #[test]
    fn click_without_ctrl_does_not_invoke_opener_and_focuses_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::TabContent;
        let expected_url = "https://example.com";
        update(
            &mut model,
            Message::PtyOutput("shell-0".to_string(), expected_url.as_bytes().to_vec()),
        );
        let area = active_session_content_area(&model).expect("active session area");
        let region = crate::renderer::collect_url_regions(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .screen(),
            Rect::new(0, 0, area.width, area.height),
        )
        .into_iter()
        .find(|region| region.url == expected_url)
        .expect("url region");

        let mut opened = false;
        let opened_result = handle_mouse_input_with(
            &mut model,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + region.start_col,
                row: area.y + region.row,
                modifiers: KeyModifiers::NONE,
            },
            |_| {
                opened = true;
                Ok(())
            },
        )
        .expect("mouse handler succeeds");

        assert!(opened_result);
        assert!(!opened);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    fn init_git_repo(path: &std::path::Path) {
        let path_str = path.to_string_lossy().to_string();
        let init = std::process::Command::new("git")
            .args(["init", &path_str])
            .output()
            .expect("init git repo");
        assert!(init.status.success(), "git init failed: {:?}", init);

        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("set git email");
        assert!(email.status.success(), "git config user.email failed");

        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("set git name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn init_bare_git_repo(path: &std::path::Path) {
        let path_str = path.to_string_lossy().to_string();
        let init = std::process::Command::new("git")
            .args(["init", "--bare", &path_str])
            .output()
            .expect("init bare git repo");
        assert!(init.status.success(), "git init --bare failed: {:?}", init);
    }

    fn git_clone_repo(src: &std::path::Path, dst: &std::path::Path) {
        let output = std::process::Command::new("git")
            .args([
                "clone",
                src.to_str().expect("clone src"),
                dst.to_str().expect("clone dst"),
            ])
            .output()
            .expect("clone git repo");
        assert!(
            output.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_add_remote(path: &std::path::Path, name: &str, remote: &std::path::Path) {
        let output = std::process::Command::new("git")
            .args(["remote", "add", name, remote.to_str().expect("remote path")])
            .current_dir(path)
            .output()
            .expect("add git remote");
        assert!(
            output.status.success(),
            "git remote add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_push_branch(path: &std::path::Path, name: &str) {
        let output = std::process::Command::new("git")
            .args(["push", "-u", "origin", name])
            .current_dir(path)
            .output()
            .expect("push git branch");
        assert!(
            output.status.success(),
            "git push -u origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_commit_allow_empty(path: &std::path::Path, message: &str) {
        let output = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", message])
            .current_dir(path)
            .output()
            .expect("create git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_create_branch(path: &std::path::Path, name: &str) {
        let output = std::process::Command::new("git")
            .args(["branch", name])
            .current_dir(path)
            .output()
            .expect("create git branch");
        assert!(
            output.status.success(),
            "git branch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_resolved_exclude_path(path: &std::path::Path) -> PathBuf {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--git-path", "info/exclude"])
            .current_dir(path)
            .output()
            .expect("git rev-parse --git-path info/exclude");
        assert!(
            output.status.success(),
            "git rev-parse --git-path info/exclude failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let resolved = PathBuf::from(
            String::from_utf8(output.stdout)
                .expect("utf8 git-path")
                .trim(),
        );
        if resolved.is_absolute() {
            resolved
        } else {
            path.join(resolved)
        }
    }

    fn git_checkout_branch_or_create(path: &std::path::Path, name: &str) {
        let checkout = std::process::Command::new("git")
            .args(["checkout", name])
            .current_dir(path)
            .output()
            .expect("checkout git branch");
        if checkout.status.success() {
            return;
        }

        let output = std::process::Command::new("git")
            .args(["checkout", "-b", name])
            .current_dir(path)
            .output()
            .expect("checkout new git branch");
        assert!(
            output.status.success(),
            "git checkout/create failed: checkout stderr: {}; checkout -b stderr: {}",
            String::from_utf8_lossy(&checkout.stderr),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn update_quit_sets_flag() {
        let mut model = test_model();
        update(&mut model, Message::Quit);
        assert!(model.quit);
    }

    #[test]
    fn update_toggle_layer() {
        let mut model = test_model();
        assert_eq!(model.active_layer, ActiveLayer::Management);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Main);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_toggle_layer_shows_management_and_focuses_management_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        update(&mut model, Message::ToggleLayer);

        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn update_toggle_layer_hides_management_and_normalizes_tab_focus_to_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        update(&mut model, Message::ToggleLayer);

        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_toggle_layer_hides_management_and_normalizes_detail_focus_to_terminal() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::BranchDetail;

        update(&mut model, Message::ToggleLayer);

        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_switch_management_tab() {
        let mut model = test_model();
        update(
            &mut model,
            Message::SwitchManagementTab(ManagementTab::Settings),
        );
        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert_eq!(model.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_switch_management_tab_from_main_focuses_management_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        update(
            &mut model,
            Message::SwitchManagementTab(ManagementTab::Issues),
        );

        assert_eq!(model.management_tab, ManagementTab::Issues);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn update_next_prev_session() {
        let mut model = test_model();
        // Add a second session
        update(&mut model, Message::NewShell);
        assert_eq!(model.active_session, 1);

        update(&mut model, Message::PrevSession);
        assert_eq!(model.active_session, 0);

        update(&mut model, Message::PrevSession);
        // Wraps to last
        assert_eq!(model.active_session, 1);

        update(&mut model, Message::NextSession);
        assert_eq!(model.active_session, 0);
    }

    #[test]
    fn update_switch_session_by_index() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);

        update(&mut model, Message::SwitchSession(0));
        assert_eq!(model.active_session, 0);

        update(&mut model, Message::SwitchSession(2));
        assert_eq!(model.active_session, 2);

        // Out of range — no change
        update(&mut model, Message::SwitchSession(99));
        assert_eq!(model.active_session, 2);
    }

    #[test]
    fn update_toggle_session_layout() {
        let mut model = test_model();
        assert_eq!(model.session_layout, SessionLayout::Tab);

        update(&mut model, Message::ToggleSessionLayout);
        assert_eq!(model.session_layout, SessionLayout::Grid);

        update(&mut model, Message::ToggleSessionLayout);
        assert_eq!(model.session_layout, SessionLayout::Tab);
    }

    #[test]
    fn update_move_grid_session_switches_to_adjacent_wide_layout_cells() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);
        model.active_session = 0;
        model.session_layout = SessionLayout::Grid;
        update(&mut model, Message::Resize(120, 30));

        update(
            &mut model,
            Message::MoveGridSession(crate::message::GridSessionDirection::Right),
        );
        assert_eq!(model.active_session, 1);

        update(
            &mut model,
            Message::MoveGridSession(crate::message::GridSessionDirection::Down),
        );
        assert_eq!(model.active_session, 2);
    }

    #[test]
    fn update_move_grid_session_switches_to_adjacent_tall_layout_cells() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        model.active_session = 0;
        model.session_layout = SessionLayout::Grid;
        update(&mut model, Message::Resize(40, 80));

        update(
            &mut model,
            Message::MoveGridSession(crate::message::GridSessionDirection::Down),
        );
        assert_eq!(model.active_session, 1);

        update(
            &mut model,
            Message::MoveGridSession(crate::message::GridSessionDirection::Up),
        );
        assert_eq!(model.active_session, 0);
    }

    #[test]
    fn update_new_shell_adds_session() {
        let mut model = test_model();
        assert_eq!(model.session_count(), 1);

        update(&mut model, Message::NewShell);
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.active_session, 1);
        assert_eq!(model.sessions[1].name, "Shell 2");
    }

    #[test]
    fn update_close_session_removes_active() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);
        assert_eq!(model.session_count(), 3);
        assert_eq!(model.active_session, 2);

        update(&mut model, Message::CloseSession);
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.active_session, 1);
    }

    #[test]
    fn update_close_session_wont_remove_last() {
        let mut model = test_model();
        assert_eq!(model.session_count(), 1);

        update(&mut model, Message::CloseSession);
        assert_eq!(model.session_count(), 1);
    }

    #[test]
    fn update_resize() {
        let mut model = test_model();
        update(&mut model, Message::Resize(120, 40));
        assert_eq!(model.terminal_size, (120, 40));
    }

    #[test]
    fn load_initial_data_populates_git_view_from_repository_state() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let tracked = dir.path().join("tracked.txt");
        fs::write(&tracked, "before\n").expect("write tracked file");
        let add = std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .expect("git add tracked file");
        assert!(add.status.success(), "git add failed");
        git_commit_allow_empty(dir.path(), "add tracked file");

        fs::write(&tracked, "before\nafter\n").expect("modify tracked file");
        fs::write(dir.path().join("new.txt"), "new file\n").expect("write untracked file");

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);
        load_initial_data(&mut model);

        assert!(
            model
                .git_view
                .files
                .iter()
                .any(|item| item.path == "tracked.txt"),
            "tracked modified file should appear in Git View"
        );
        assert!(
            model
                .git_view
                .files
                .iter()
                .any(|item| item.path == "new.txt"),
            "untracked file should appear in Git View"
        );
        assert!(
            model
                .git_view
                .commits
                .iter()
                .any(|commit| commit.subject == "add tracked file"),
            "recent commits should populate Git View"
        );
    }

    #[test]
    fn load_initial_data_handles_empty_repo_git_view_gracefully() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        assert!(
            model.git_view.files.is_empty(),
            "empty repo should not produce file entries"
        );
        assert!(
            model.git_view.commits.is_empty(),
            "empty repo should not produce commit entries"
        );
    }

    #[test]
    fn load_initial_data_prefetches_branch_detail_async() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let mut model = Model::new(dir.path().to_path_buf());
        let project_root = dir.path().to_path_buf();
        model.set_branch_detail_docker_snapshotter(move || {
            thread::sleep(std::time::Duration::from_millis(250));
            vec![docker_service(
                &project_root,
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            )]
        });

        let start = std::time::Instant::now();
        load_initial_data(&mut model);
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(3000),
            "initial data load should not block on branch detail preload: {elapsed:?}"
        );
        assert!(
            model.branches.docker_services.is_empty(),
            "branch detail docker data should arrive asynchronously"
        );
        assert!(
            model.branch_detail_worker.is_some(),
            "branch detail preload should run in the background"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_initial_data_skips_github_cli_when_repo_has_no_remote() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");
        assert!(
            !repo_has_git_remote(dir.path()),
            "test repo should not have any git remotes"
        );

        let gh_count = dir.path().join("gh-count.txt");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ] || [ \"$2\" = \"--version\" ]; then\n  printf 'gh version test\\n'\n  exit 0\nfi\ncount_file=\"{}\"\ncount=0\nif [ -f \"$count_file\" ]; then\n  count=$(cat \"$count_file\")\nfi\necho $((count + 1)) > \"$count_file\"\nsleep 5\nprintf '{{\"url\":\"https://example.com/pr/1\"}}'\n",
            gh_count.display()
        );

        with_fake_gh(&script, || {
            let mut model = Model::new(dir.path().to_path_buf());

            let start = std::time::Instant::now();
            load_initial_data_with(
                &mut model,
                |_repo_path| Ok(None),
                |_repo_path| Ok(Vec::new()),
            );
            let elapsed = start.elapsed();
            let gh_calls = fs::read_to_string(&gh_count)
                .unwrap_or_else(|_| "0".to_string())
                .trim()
                .to_string();

            assert!(
                elapsed < std::time::Duration::from_millis(1500),
                "load_initial_data should skip gh lookups when the repo has no remote: {elapsed:?} (gh calls: {gh_calls})"
            );
            assert!(
                !gh_count.exists(),
                "gh should not be invoked for repos without remotes"
            );
            assert!(
                model.pr_dashboard.prs.is_empty(),
                "repos without remotes should not try to populate PR dashboard data"
            );
        });
    }

    #[test]
    fn load_initial_data_prefetches_docker_once_per_refresh() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");
        git_create_branch(dir.path(), "feature/one");
        git_create_branch(dir.path(), "feature/two");

        let docker_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let docker_calls_for_worker = docker_calls.clone();
        let mut model = Model::new(dir.path().to_path_buf());
        let project_root = dir.path().to_path_buf();
        model.set_branch_detail_docker_snapshotter(move || {
            docker_calls_for_worker.fetch_add(1, Ordering::SeqCst);
            vec![docker_service(
                &project_root,
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            )]
        });
        load_initial_data(&mut model);

        let docker_calls_for_assert = docker_calls.clone();
        drive_ticks_until(
            &mut model,
            |_| docker_calls_for_assert.load(Ordering::SeqCst) == 1,
            "branch detail preload docker snapshotter call",
        );

        let docker_calls = docker_calls.load(Ordering::SeqCst);
        assert_eq!(
            docker_calls, 1,
            "branch detail preload should snapshot docker state exactly once per refresh cycle"
        );
    }

    #[test]
    fn load_initial_data_prunes_stale_gwt_assets_from_repo_and_active_worktrees() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let worktree = dir.path().join("wt-feature-stale");
        let add_worktree = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/stale",
                worktree.to_str().expect("worktree path"),
            ])
            .current_dir(dir.path())
            .output()
            .expect("git worktree add");
        assert!(
            add_worktree.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&add_worktree.stderr)
        );

        let repo_stale = dir.path().join(".claude/commands/gwt-issue-search.md");
        let worktree_stale = worktree.join(".codex/skills/gwt-agent-read/SKILL.md");
        let unrelated_dir = dir.path().join("not-a-worktree");
        let unrelated_stale = unrelated_dir.join(".claude/commands/gwt-issue-search.md");

        fs::create_dir_all(repo_stale.parent().expect("repo stale parent"))
            .expect("repo stale dir");
        fs::create_dir_all(worktree_stale.parent().expect("worktree stale parent"))
            .expect("worktree stale dir");
        fs::create_dir_all(unrelated_stale.parent().expect("unrelated stale parent"))
            .expect("unrelated stale dir");
        fs::write(&repo_stale, "legacy repo command").expect("write repo stale asset");
        fs::write(&worktree_stale, "legacy worktree skill").expect("write worktree stale asset");
        fs::write(&unrelated_stale, "legacy unrelated command")
            .expect("write unrelated stale asset");

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        assert!(
            !repo_stale.exists(),
            "repo stale asset should be pruned during startup load"
        );
        assert!(
            !worktree_stale.exists(),
            "active worktree stale asset should be pruned during startup load"
        );
        assert!(
            unrelated_stale.exists(),
            "startup sweep should not touch non-worktree directories"
        );
    }

    #[test]
    fn load_initial_data_refreshes_managed_assets_for_repo_and_active_worktrees() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let tracked_command = dir.path().join(".claude/commands/gwt-spec-brainstorm.md");
        fs::create_dir_all(tracked_command.parent().expect("tracked command parent"))
            .expect("create tracked command dir");
        fs::write(&tracked_command, "tracked brainstorm command")
            .expect("write tracked brainstorm command");
        let add_tracked_command = std::process::Command::new("git")
            .args(["add", ".claude/commands/gwt-spec-brainstorm.md"])
            .current_dir(dir.path())
            .output()
            .expect("git add tracked brainstorm command");
        assert!(
            add_tracked_command.status.success(),
            "git add tracked brainstorm command failed: {}",
            String::from_utf8_lossy(&add_tracked_command.stderr)
        );
        fs::remove_file(&tracked_command).expect("delete tracked brainstorm command");

        let worktree = dir.path().join("wt-feature-sync");
        let add_worktree = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/sync",
                worktree.to_str().expect("worktree path"),
            ])
            .current_dir(dir.path())
            .output()
            .expect("git worktree add");
        assert!(
            add_worktree.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&add_worktree.stderr)
        );

        let legacy_hooks = serde_json::to_string_pretty(&serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "*",
                    "hooks": [{
                        "command": "GWT_MANAGED_HOOK=runtime-state '/tmp/gwt-tui' hook runtime-state SessionStart",
                        "type": "command"
                    }]
                }],
                "UserPromptSubmit": [{
                    "matcher": "*",
                    "hooks": [{
                        "command": "GWT_MANAGED_HOOK=runtime-state '/tmp/gwt-tui' hook runtime-state UserPromptSubmit",
                        "type": "command"
                    }]
                }],
                "PreToolUse": [
                    {
                        "matcher": "*",
                        "hooks": [{
                            "command": "GWT_MANAGED_HOOK=runtime-state '/tmp/gwt-tui' hook runtime-state PreToolUse",
                            "type": "command"
                        }]
                    },
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {
                                "command": "'/tmp/gwt-tui' hook block-git-branch-ops",
                                "type": "command"
                            },
                            {
                                "command": "'/tmp/gwt-tui' hook block-cd-command",
                                "type": "command"
                            },
                            {
                                "command": "'/tmp/gwt-tui' hook block-file-ops",
                                "type": "command"
                            },
                            {
                                "command": "'/tmp/gwt-tui' hook block-git-dir-override",
                                "type": "command"
                            }
                        ]
                    }
                ],
                "PostToolUse": [{
                    "matcher": "*",
                    "hooks": [{
                        "command": "GWT_MANAGED_HOOK=runtime-state '/tmp/gwt-tui' hook runtime-state PostToolUse",
                        "type": "command"
                    }]
                }],
                "Stop": [{
                    "matcher": "*",
                    "hooks": [{
                        "command": "GWT_MANAGED_HOOK=runtime-state '/tmp/gwt-tui' hook runtime-state Stop",
                        "type": "command"
                    }]
                }]
            }
        }))
        .expect("serialize legacy hooks");

        let codex_hooks = worktree.join(".codex/hooks.json");
        let claude_settings = worktree.join(".claude/settings.local.json");
        fs::create_dir_all(codex_hooks.parent().expect("codex hooks parent"))
            .expect("create codex dir");
        fs::create_dir_all(claude_settings.parent().expect("claude settings parent"))
            .expect("create claude dir");
        fs::write(&codex_hooks, &legacy_hooks).expect("write legacy codex hooks");
        fs::write(&claude_settings, &legacy_hooks).expect("write legacy claude settings");

        let add_codex_hooks = std::process::Command::new("git")
            .args(["add", ".codex/hooks.json"])
            .current_dir(&worktree)
            .output()
            .expect("git add codex hooks");
        assert!(
            add_codex_hooks.status.success(),
            "git add codex hooks failed: {}",
            String::from_utf8_lossy(&add_codex_hooks.stderr)
        );

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        let tracked_command_content =
            fs::read_to_string(&tracked_command).expect("tracked brainstorm command restored");
        assert!(
            tracked_command_content.contains("SPEC Brainstorm Command"),
            "startup refresh should restore missing tracked bundled commands"
        );

        let codex_content = fs::read_to_string(&codex_hooks).expect("read migrated codex hooks");
        assert!(
            codex_content.contains("block-bash-policy"),
            "startup refresh should consolidate Bash policy hooks"
        );
        assert!(
            !codex_content.contains("GWT_MANAGED_HOOK=runtime-state"),
            "startup refresh should remove the legacy runtime marker"
        );
        assert!(
            !codex_content.contains("block-git-branch-ops"),
            "startup refresh should replace split bash blockers"
        );

        let claude_content =
            fs::read_to_string(&claude_settings).expect("read migrated claude settings");
        assert!(
            claude_content.contains("block-bash-policy"),
            "startup refresh should regenerate Claude settings too"
        );
        assert!(
            !claude_content.contains("GWT_MANAGED_HOOK=runtime-state"),
            "startup refresh should remove the legacy runtime marker from Claude settings"
        );
    }

    #[test]
    fn load_initial_data_self_heals_pristine_git_repo_and_linked_worktree_without_materializing_bundles(
    ) {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let worktree = dir.path().join("wt-feature-pristine");
        let add_worktree = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/pristine",
                worktree.to_str().expect("worktree path"),
            ])
            .current_dir(dir.path())
            .output()
            .expect("git worktree add");
        assert!(
            add_worktree.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&add_worktree.stderr)
        );
        assert!(
            worktree.join(".git").is_file(),
            "linked worktree should expose .git as a file"
        );

        let mut model = Model::new(dir.path().to_path_buf());
        load_initial_data(&mut model);

        for path in [dir.path(), worktree.as_path()] {
            assert!(
                path.join(".claude/settings.local.json").exists(),
                "startup self-heal should generate Claude settings for {}",
                path.display()
            );
            assert!(
                path.join(".codex/hooks.json").exists(),
                "startup self-heal should generate Codex hooks for {}",
                path.display()
            );
            assert!(
                !path.join(".claude/skills/gwt-pr/SKILL.md").exists(),
                "pristine startup should not materialize bundled skills for {}",
                path.display()
            );
            assert!(
                !path
                    .join(".claude/commands/gwt-spec-brainstorm.md")
                    .exists(),
                "pristine startup should not materialize bundled commands for {}",
                path.display()
            );

            let exclude_path = git_resolved_exclude_path(path);
            let exclude = fs::read_to_string(&exclude_path).expect("read git exclude");
            assert!(
                exclude.contains("# gwt-managed-begin"),
                "startup self-heal should update git exclude for {}",
                path.display()
            );
            assert!(
                exclude.contains(".codex/hooks.json"),
                "startup self-heal should exclude managed hooks for {}",
                path.display()
            );
        }

        assert!(
            !worktree.join(".git/info/exclude").exists(),
            "linked worktree startup self-heal should not treat .git as a directory"
        );
    }

    #[test]
    fn load_git_view_with_populates_divergence_and_pr_link_metadata() {
        let mut model = test_model();

        load_git_view_with(
            &mut model,
            |_repo_path| {
                Ok(vec![gwt_git::diff::FileEntry {
                    path: std::path::PathBuf::from("tracked.txt"),
                    status: gwt_git::diff::FileStatus::Staged,
                }])
            },
            |_repo_path| {
                Ok(vec![gwt_git::commit::CommitEntry {
                    hash: "abcdef1".into(),
                    subject: "Initial commit".into(),
                    author: "Alice".into(),
                    timestamp: "2026-04-04T00:00:00Z".into(),
                }])
            },
            |_repo_path| {
                Ok(vec![gwt_git::Branch {
                    name: "feature/live-meta".into(),
                    is_local: true,
                    is_remote: false,
                    is_head: true,
                    upstream: Some("origin/feature/live-meta".into()),
                    ahead: 2,
                    behind: 1,
                    last_commit_date: None,
                }])
            },
            |_repo_path| Ok(Some("https://example.com/pr/42".into())),
        );

        assert_eq!(
            model.git_view.divergence_summary.as_deref(),
            Some("Ahead 2 Behind 1")
        );
        assert_eq!(
            model.git_view.pr_link.as_deref(),
            Some("https://example.com/pr/42")
        );
    }

    #[test]
    fn load_git_view_with_omits_divergence_without_upstream() {
        let mut model = test_model();

        load_git_view_with(
            &mut model,
            |_repo_path| Ok(Vec::new()),
            |_repo_path| Ok(Vec::new()),
            |_repo_path| {
                Ok(vec![gwt_git::Branch {
                    name: "feature/no-upstream".into(),
                    is_local: true,
                    is_remote: false,
                    is_head: true,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    last_commit_date: None,
                }])
            },
            |_repo_path| Ok(None),
        );

        assert!(model.git_view.divergence_summary.is_none());
        assert!(model.git_view.pr_link.is_none());
    }

    #[test]
    fn switch_management_tab_pr_dashboard_loads_prs_and_focuses_management_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        switch_management_tab_with(
            &mut model,
            ManagementTab::PrDashboard,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 42,
                    title: "Wire PR dashboard".into(),
                    state: GitPrState::Open,
                    url: "https://example.com/pr/42".into(),
                    ci_status: "SUCCESS".into(),
                    mergeable: "MERGEABLE".into(),
                    review_status: "APPROVED".into(),
                }])
            },
            |_repo_path, _number| panic!("detail loader should not run for list-only focus"),
        );

        assert_eq!(model.management_tab, ManagementTab::PrDashboard);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert_eq!(model.pr_dashboard.prs.len(), 1);
        assert_eq!(model.pr_dashboard.prs[0].number, 42);
        assert_eq!(model.pr_dashboard.prs[0].title, "Wire PR dashboard");
        assert_eq!(model.pr_dashboard.prs[0].ci_status, "success");
        assert_eq!(model.pr_dashboard.prs[0].review_status, "approved");
        assert!(model.pr_dashboard.prs[0].mergeable);
    }

    #[test]
    fn switch_management_tab_from_tab_content_lands_on_tab_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        switch_management_tab_with(
            &mut model,
            ManagementTab::Settings,
            |_repo_path| panic!("PR loader should not run for Settings"),
            |_repo_path, _number| panic!("detail loader should not run for Settings"),
        );

        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn switch_management_tab_from_branch_detail_lands_on_tab_content() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::BranchDetail;

        switch_management_tab_with(
            &mut model,
            ManagementTab::Issues,
            |_repo_path| panic!("PR loader should not run for Issues"),
            |_repo_path, _number| panic!("detail loader should not run for Issues"),
        );

        assert_eq!(model.management_tab, ManagementTab::Issues);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn switch_management_tab_pr_dashboard_reloads_detail_when_open() {
        let mut model = test_model();
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Existing detail".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "pending".into(),
                    mergeable: true,
                    review_status: "review_required".into(),
                },
            ]),
        );
        model.pr_dashboard.detail_view = true;

        switch_management_tab_with(
            &mut model,
            ManagementTab::PrDashboard,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 42,
                    title: "Existing detail".into(),
                    state: GitPrState::Open,
                    url: "https://example.com/pr/42".into(),
                    ci_status: "SUCCESS".into(),
                    mergeable: "MERGEABLE".into(),
                    review_status: "APPROVED".into(),
                }])
            },
            |_repo_path, number| {
                assert_eq!(number, 42);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "live detail".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["lint: success".into()],
                })
            },
        );

        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report refreshed on tab focus");
        assert_eq!(detail.summary, "live detail");
    }

    #[test]
    fn refresh_pr_dashboard_with_reloads_prs() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;

        load_pr_dashboard_with(&mut model, |_repo_path| {
            Ok(vec![gwt_git::PrStatus {
                number: 7,
                title: "Initial".into(),
                state: GitPrState::Open,
                url: "https://example.com/pr/7".into(),
                ci_status: "PENDING".into(),
                mergeable: "UNKNOWN".into(),
                review_status: "REVIEW_REQUIRED".into(),
            }])
        });
        assert_eq!(model.pr_dashboard.prs.len(), 1);
        assert_eq!(model.pr_dashboard.prs[0].number, 7);

        refresh_pr_dashboard_with(
            &mut model,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 8,
                    title: "Updated".into(),
                    state: GitPrState::Merged,
                    url: "https://example.com/pr/8".into(),
                    ci_status: "FAILURE".into(),
                    mergeable: "CONFLICTING".into(),
                    review_status: "CHANGES_REQUESTED".into(),
                }])
            },
            |_repo_path, _number| {
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "CI failing".into(),
                    ci_status: "failing".into(),
                    merge_status: "conflicts".into(),
                    review_status: "changes_requested".into(),
                    checks: vec!["lint: failure".into()],
                })
            },
        );

        assert_eq!(model.pr_dashboard.prs.len(), 1);
        assert_eq!(model.pr_dashboard.prs[0].number, 8);
        assert_eq!(model.pr_dashboard.prs[0].title, "Updated");
        assert_eq!(model.pr_dashboard.prs[0].ci_status, "failure");
        assert_eq!(model.pr_dashboard.prs[0].review_status, "changes_requested");
        assert!(!model.pr_dashboard.prs[0].mergeable);
        assert_eq!(
            model.pr_dashboard.prs[0].state,
            screens::pr_dashboard::PrState::Merged
        );
    }

    #[test]
    fn refresh_pr_dashboard_with_in_detail_view_updates_detail_report() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 8,
                    title: "Updated".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );
        model.pr_dashboard.detail_view = true;

        refresh_pr_dashboard_with(
            &mut model,
            |_repo_path| {
                Ok(vec![gwt_git::PrStatus {
                    number: 8,
                    title: "Updated".into(),
                    state: GitPrState::Open,
                    url: "https://example.com/pr/8".into(),
                    ci_status: "SUCCESS".into(),
                    mergeable: "MERGEABLE".into(),
                    review_status: "APPROVED".into(),
                }])
            },
            |_repo_path, number| {
                assert_eq!(number, 8);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "CI passing".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["test: success".into()],
                })
            },
        );

        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report refreshed");
        assert_eq!(detail.summary, "CI passing");
        assert_eq!(detail.checks, vec!["test: success"]);
    }

    #[test]
    fn parse_pr_dashboard_detail_report_json_extracts_checks_and_statuses() {
        let json = r#"{
            "title": "Add dashboard detail",
            "state": "OPEN",
            "mergeable": "CONFLICTING",
            "reviewDecision": "CHANGES_REQUESTED",
            "statusCheckRollup": [
                {"name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "test", "status": "COMPLETED", "conclusion": "FAILURE"}
            ]
        }"#;

        let detail = parse_pr_dashboard_detail_report_json(json).expect("detail report parsed");
        assert_eq!(detail.ci_status, "failing");
        assert_eq!(detail.merge_status, "conflicts");
        assert_eq!(detail.review_status, "changes_requested");
        assert_eq!(
            detail.checks,
            vec!["lint: success".to_string(), "test: failure".to_string()]
        );
    }

    #[test]
    fn route_key_to_management_pr_dashboard_enter_loads_detail_report() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Wire detail report".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );

        route_key_to_management_pr_dashboard_with(
            &mut model,
            key(KeyCode::Enter, KeyModifiers::NONE),
            |_repo_path, number| {
                assert_eq!(number, 42);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "CI passing".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["lint: success".into()],
                })
            },
        );

        assert!(model.pr_dashboard.detail_view);
        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report loaded");
        assert_eq!(detail.summary, "CI passing");
        assert_eq!(detail.checks, vec!["lint: success"]);
    }

    #[test]
    fn route_key_to_management_right_from_branches_switches_directly_to_issues() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;

        route_key_to_management(&mut model, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(model.management_tab, ManagementTab::Issues);

        route_key_to_management(&mut model, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(model.management_tab, ManagementTab::Branches);
    }

    #[test]
    fn route_key_to_management_pr_dashboard_move_in_detail_view_reloads_selected_pr_detail() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 41,
                    title: "First".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "pending".into(),
                    mergeable: true,
                    review_status: "review_required".into(),
                },
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Second".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );
        model.pr_dashboard.detail_view = true;

        route_key_to_management_pr_dashboard_with(
            &mut model,
            key(KeyCode::Down, KeyModifiers::NONE),
            |_repo_path, number| {
                assert_eq!(number, 42);
                Ok(screens::pr_dashboard::PrDetailReport {
                    summary: "second detail".into(),
                    ci_status: "passing".into(),
                    merge_status: "ready".into(),
                    review_status: "approved".into(),
                    checks: vec!["test: success".into()],
                })
            },
        );

        assert_eq!(model.pr_dashboard.selected, 1);
        let detail = model
            .pr_dashboard
            .detail_report
            .as_ref()
            .expect("detail report reloaded for moved selection");
        assert_eq!(detail.summary, "second detail");
    }

    #[test]
    fn route_key_to_management_pr_dashboard_esc_closes_detail_view_and_preserves_selection() {
        let mut model = test_model();
        model.management_tab = ManagementTab::PrDashboard;
        screens::pr_dashboard::update(
            &mut model.pr_dashboard,
            screens::pr_dashboard::PrDashboardMessage::SetPrs(vec![
                screens::pr_dashboard::PrItem {
                    number: 41,
                    title: "First".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "pending".into(),
                    mergeable: true,
                    review_status: "review_required".into(),
                },
                screens::pr_dashboard::PrItem {
                    number: 42,
                    title: "Second".into(),
                    state: screens::pr_dashboard::PrState::Open,
                    ci_status: "success".into(),
                    mergeable: true,
                    review_status: "approved".into(),
                },
            ]),
        );
        model.pr_dashboard.selected = 1;
        model.pr_dashboard.detail_view = true;
        model.pr_dashboard.detail_report = Some(screens::pr_dashboard::PrDetailReport {
            summary: "loaded".into(),
            ci_status: "passing".into(),
            merge_status: "ready".into(),
            review_status: "approved".into(),
            checks: vec!["lint: success".into()],
        });

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!model.pr_dashboard.detail_view);
        assert_eq!(model.pr_dashboard.selected, 1);
        assert_eq!(
            model.pr_dashboard.selected_pr().map(|pr| pr.number),
            Some(42)
        );
        assert!(model.pr_dashboard.detail_report.is_none());
    }

    #[test]
    fn update_error_queue() {
        let mut model = test_model();
        update(&mut model, Message::PushError("e1".into()));
        update(
            &mut model,
            Message::PushErrorNotification(Notification::new(Severity::Error, "core", "e2")),
        );
        assert_eq!(model.error_queue.len(), 2);

        update(&mut model, Message::DismissError);
        assert_eq!(model.error_queue.len(), 1);
        assert_eq!(model.error_queue.front().unwrap().message, "e2");
    }

    #[test]
    fn update_dismiss_empty_error_queue_is_noop() {
        let mut model = test_model();
        update(&mut model, Message::DismissError);
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn prepare_wizard_startup_prefills_spec_context_and_versions() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let mut cache = VersionCache::new();
        cache.entries.insert(
            "claude-code".into(),
            version_entry(&["1.0.54", "1.0.53"], 60),
        );
        cache
            .entries
            .insert("codex".into(), version_entry(&["0.5.0"], 90_000));

        let detected = vec![
            detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
            detected_agent(AgentId::Codex, Some("0.5.1")),
            detected_agent(AgentId::Gemini, Some("0.2.0")),
        ];

        let (wizard, refresh_targets) = prepare_wizard_startup(
            project_root.path(),
            Some(screens::wizard::SpecContext::new(
                "SPEC-42",
                "My Feature",
                "# SPEC-42\n\nBody\n",
            )),
            detected,
            &cache,
        );

        assert_eq!(wizard.branch_name, "feature/spec-42-my-feature");
        let ctx = wizard.spec_context.as_ref().unwrap();
        assert_eq!(ctx.spec_id, "SPEC-42");
        assert_eq!(ctx.title, "My Feature");
        assert_eq!(ctx.spec_body, "# SPEC-42\n\nBody\n");
        // All 4 builtins are always listed
        assert_eq!(wizard.detected_agents.len(), 4);
        // Claude Code: installed with cache
        assert_eq!(wizard.detected_agents[0].name, "Claude Code");
        assert!(wizard.detected_agents[0].available);
        assert_eq!(
            wizard.detected_agents[0].installed_version.as_deref(),
            Some("1.0.55")
        );
        assert_eq!(wizard.detected_agents[0].versions, vec!["1.0.54", "1.0.53"]);
        // Codex: installed, stale cache
        assert_eq!(wizard.detected_agents[1].name, "Codex");
        assert!(wizard.detected_agents[1].available);
        assert_eq!(
            wizard.detected_agents[1].installed_version.as_deref(),
            Some("0.5.1")
        );
        assert!(wizard.detected_agents[1].cache_outdated);
        // Gemini: installed, no cache
        assert_eq!(wizard.detected_agents[2].name, "Gemini CLI");
        assert!(wizard.detected_agents[2].available);
        assert_eq!(
            wizard.detected_agents[2].installed_version.as_deref(),
            Some("0.2.0")
        );
        assert!(wizard.detected_agents[2].cache_outdated);
        // Copilot: not installed (not in detected list)
        assert_eq!(wizard.detected_agents[3].name, "GitHub Copilot");
        assert!(!wizard.detected_agents[3].available);
        assert!(wizard.detected_agents[3].installed_version.is_none());
        assert_eq!(wizard.model, "Default (Opus 4.6)");
        assert_eq!(
            wizard
                .version_options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Installed", "latest", "1.0.54", "1.0.53"]
        );
        assert_eq!(wizard.version, "latest");
        assert_eq!(refresh_targets, vec![AgentId::Codex, AgentId::Gemini]);
    }

    fn write_issue_cache_meta(
        root: &std::path::Path,
        number: u64,
        title: &str,
        state: &str,
        labels: &[&str],
    ) {
        let dir = root.join(number.to_string());
        fs::create_dir_all(&dir).expect("create issue cache dir");
        fs::write(
            dir.join("meta.json"),
            serde_json::json!({
                "number": number,
                "title": title,
                "labels": labels,
                "state": state,
                "updated_at": "2026-04-09T00:00:00Z",
                "comment_ids": []
            })
            .to_string(),
        )
        .expect("write issue cache meta");
    }

    fn write_issue_cache_body(root: &std::path::Path, number: u64, body: &str) {
        let dir = root.join(number.to_string());
        fs::create_dir_all(&dir).expect("create issue cache dir for body");
        fs::write(dir.join("body.md"), body).expect("write issue cache body");
    }

    fn write_cached_spec(
        root: &std::path::Path,
        number: u64,
        title: &str,
        state: gwt_github::IssueState,
        labels: &[&str],
    ) {
        gwt_github::Cache::new(root.to_path_buf())
            .write_snapshot(&gwt_github::client::IssueSnapshot {
                number: gwt_github::IssueNumber(number),
                title: title.to_string(),
                body: format!(
                    "<!-- gwt-spec id={number} version=1 -->\n<!-- sections:\nspec=body\n-->\n<!-- artifact:spec BEGIN -->\nbody\n<!-- artifact:spec END -->\n"
                ),
                labels: labels.iter().map(|label| label.to_string()).collect(),
                state,
                updated_at: gwt_github::UpdatedAt::new("2026-04-12T00:00:00Z"),
                comments: vec![],
            })
            .expect("write cached spec");
    }

    #[test]
    fn prepare_wizard_startup_loads_cached_issues_and_prefills_selected_issue() {
        let cache = VersionCache::new();
        let issue_cache = tempfile::tempdir().expect("issue cache tempdir");
        write_issue_cache_meta(
            issue_cache.path(),
            42,
            "Fix login bug",
            "open",
            &["bug", "auth"],
        );
        write_issue_cache_meta(
            issue_cache.path(),
            1776,
            "Launch Agent issue linkage",
            "closed",
            &["ux"],
        );

        let (wizard, _) = prepare_wizard_startup_with_issue_cache_root(
            issue_cache.path(),
            None,
            Some(1776),
            vec![],
            &cache,
            issue_cache.path().to_path_buf(),
        );

        assert_eq!(wizard.step, screens::wizard::WizardStep::BranchTypeSelect);
        assert_eq!(wizard.issue_id, "1776");
        assert_eq!(
            wizard.current_options_for_step(screens::wizard::WizardStep::IssueSelect),
            vec![
                "Related to none".to_string(),
                "#1776 Launch Agent issue linkage (closed) [ux]".to_string(),
                "#42 Fix login bug (open) [bug, auth]".to_string(),
            ]
        );
    }

    #[test]
    fn load_cached_issues_with_linkage_merges_issue_cache_and_local_branch_store() {
        let issue_cache = tempfile::tempdir().expect("issue cache tempdir");
        write_issue_cache_meta(issue_cache.path(), 42, "Fix login bug", "open", &["bug"]);
        write_issue_cache_body(issue_cache.path(), 42, "Login fails on Safari.");
        write_issue_cache_meta(
            issue_cache.path(),
            1776,
            "Launch Agent issue linkage",
            "closed",
            &["ux"],
        );

        let linkage_path = issue_cache.path().join("issue-links.json");
        persist_issue_linkage_at_path(&linkage_path, 42, "feature/login-api")
            .expect("persist linkage");
        persist_issue_linkage_at_path(&linkage_path, 42, "feature/login-ui")
            .expect("persist linkage");
        persist_issue_linkage_at_path(&linkage_path, 1776, "feature/issue-link")
            .expect("persist linkage");

        let (issues, load_error) =
            load_cached_issues_with_linkage(issue_cache.path(), Some(linkage_path.as_path()));

        assert!(load_error.is_none());
        assert_eq!(
            issues.iter().map(|issue| issue.number).collect::<Vec<_>>(),
            vec![1776, 42]
        );
        assert_eq!(
            issues[0].linked_branches,
            vec!["feature/issue-link".to_string()]
        );
        assert_eq!(
            issues[1].linked_branches,
            vec![
                "feature/login-api".to_string(),
                "feature/login-ui".to_string()
            ]
        );
        assert_eq!(issues[1].body, "Login fails on Safari.");
    }

    #[test]
    fn prepare_wizard_startup_starts_spec_prefill_at_branch_type_select() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let cache = VersionCache::new();

        let (wizard, _) = prepare_wizard_startup(
            project_root.path(),
            Some(screens::wizard::SpecContext::new(
                "SPEC-42",
                "My Feature",
                "",
            )),
            vec![],
            &cache,
        );

        assert_eq!(wizard.step, screens::wizard::WizardStep::BranchTypeSelect);
        assert_eq!(wizard.branch_name, "feature/spec-42-my-feature");
    }

    #[test]
    fn prepare_wizard_startup_disables_ai_branch_suggestions_by_default() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let cache = VersionCache::new();

        let (wizard, _) = prepare_wizard_startup(
            project_root.path(),
            Some(screens::wizard::SpecContext::new(
                "SPEC-99",
                "AI-disabled flow",
                "",
            )),
            vec![],
            &cache,
        );

        assert!(!wizard.ai_enabled);
    }

    #[test]
    fn prepare_wizard_startup_uses_detected_version_when_cache_is_missing() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let cache = VersionCache::new();
        let detected = vec![detected_agent(AgentId::ClaudeCode, Some("1.0.55"))];

        let (wizard, refresh_targets) =
            prepare_wizard_startup(project_root.path(), None, detected, &cache);

        assert!(wizard.spec_context.is_none());
        assert!(wizard.branch_name.is_empty());
        // All 4 builtins listed; Claude installed, others not
        assert_eq!(wizard.detected_agents.len(), 4);
        assert!(wizard.detected_agents[0].available); // Claude installed
        assert_eq!(
            wizard.detected_agents[0].installed_version.as_deref(),
            Some("1.0.55")
        );
        assert!(wizard.detected_agents[0].versions.is_empty());
        assert_eq!(
            wizard
                .version_options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Installed", "latest"]
        );
        assert_eq!(wizard.version, "latest");
        assert!(wizard.detected_agents[0].cache_outdated);
        assert!(!wizard.detected_agents[1].available); // Codex not installed
        assert!(!wizard.detected_agents[2].available); // Gemini not installed
        assert!(!wizard.detected_agents[3].available); // Copilot not installed
                                                       // All npm agents need refresh (empty cache)
        assert!(refresh_targets.contains(&AgentId::ClaudeCode));
        assert!(refresh_targets.contains(&AgentId::Codex));
        assert!(refresh_targets.contains(&AgentId::Gemini));
    }

    #[test]
    fn prepare_wizard_startup_detects_compose_workflow_and_defaults_to_docker() {
        let project_root = tempfile::tempdir().expect("temp project root");
        fs::write(
            project_root.path().join("docker-compose.yml"),
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
  db:
    image: postgres:16
"#,
        )
        .expect("write compose");
        fs::create_dir_all(project_root.path().join(".devcontainer"))
            .expect("create .devcontainer");
        fs::write(
            project_root.path().join(".devcontainer/devcontainer.json"),
            r#"{
  "dockerComposeFile": "docker-compose.yml",
  "service": "web",
  "workspaceFolder": "/workspace"
}"#,
        )
        .expect("write devcontainer");
        let cache = VersionCache::new();

        let (wizard, _) = prepare_wizard_startup(project_root.path(), None, vec![], &cache);

        let docker_context = wizard.docker_context.as_ref().expect("docker context");
        assert_eq!(wizard.runtime_target, LaunchRuntimeTarget::Docker);
        assert_eq!(wizard.docker_service.as_deref(), Some("web"));
        assert_eq!(docker_context.suggested_service.as_deref(), Some("web"));
        assert!(docker_context
            .services
            .iter()
            .any(|service| service == "web"));
        assert!(docker_context
            .services
            .iter()
            .any(|service| service == "db"));
    }

    #[test]
    fn resolve_docker_launch_plan_prefers_devcontainer_compose_target() {
        let project_root = tempfile::tempdir().expect("temp project root");
        fs::write(
            project_root.path().join("docker-compose.yml"),
            r#"
services:
  root:
    image: node:22
    working_dir: /root-workspace
"#,
        )
        .expect("write root compose");
        fs::create_dir_all(project_root.path().join(".devcontainer"))
            .expect("create .devcontainer");
        let dev_compose = project_root
            .path()
            .join(".devcontainer/docker-compose.dev.yml");
        fs::write(
            &dev_compose,
            r#"
services:
  app:
    image: node:22
    working_dir: /workspace
"#,
        )
        .expect("write devcontainer compose");
        fs::write(
            project_root.path().join(".devcontainer/devcontainer.json"),
            r#"{
  "dockerComposeFile": "docker-compose.dev.yml",
  "service": "app",
  "workspaceFolder": "/workspace"
}"#,
        )
        .expect("write devcontainer");

        let plan = resolve_docker_launch_plan(project_root.path(), None).expect("launch plan");

        assert_eq!(plan.compose_file, dev_compose);
        assert_eq!(plan.service, "app");
        assert_eq!(plan.container_cwd, "/workspace");
    }

    #[test]
    fn configure_existing_branch_wizard_with_sessions_loads_newest_entry_per_agent() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let cache = VersionCache::new();
        let detected = vec![
            detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
            detected_agent(AgentId::Codex, Some("0.5.1")),
        ];
        let repo_path = PathBuf::from("/tmp/repo");
        let branch = "feature/test";
        let now = Utc::now();

        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            branch,
            AgentId::Codex,
            now - Duration::minutes(10),
            Some("gpt-5.2-codex"),
            Some("medium"),
            Some("0.5.0"),
            None,
            false,
            false,
        );
        let mut latest_codex =
            AgentSession::new(repo_path.to_str().unwrap(), branch, AgentId::Codex);
        latest_codex.model = Some("gpt-5.3-codex".to_string());
        latest_codex.reasoning_level = Some("high".to_string());
        latest_codex.tool_version = Some("latest".to_string());
        latest_codex.agent_session_id = Some("sess-new".to_string());
        latest_codex.skip_permissions = true;
        latest_codex.codex_fast_mode = true;
        latest_codex.runtime_target = LaunchRuntimeTarget::Docker;
        latest_codex.docker_service = Some("web".to_string());
        latest_codex.updated_at = now - Duration::minutes(1);
        latest_codex.created_at = latest_codex.updated_at;
        latest_codex.last_activity_at = latest_codex.updated_at;
        latest_codex
            .save(dir.path())
            .expect("persist latest codex session");
        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            branch,
            AgentId::ClaudeCode,
            now - Duration::minutes(5),
            Some("sonnet"),
            None,
            Some("1.0.54"),
            None,
            false,
            false,
        );
        persist_agent_session(
            dir.path(),
            repo_path.to_str().unwrap(),
            "feature/other",
            AgentId::Gemini,
            now - Duration::minutes(2),
            Some("gemini-2.5-pro"),
            None,
            Some("latest"),
            Some("sess-other"),
            false,
            false,
        );

        let (mut wizard, _) = prepare_wizard_startup(repo_path.as_path(), None, detected, &cache);
        let model = test_model();
        configure_existing_branch_wizard_with_sessions(
            &mut wizard,
            &model,
            &repo_path,
            dir.path(),
            branch,
        );

        assert_eq!(wizard.step, screens::wizard::WizardStep::QuickStart);
        assert!(wizard.has_quick_start);
        assert_eq!(wizard.branch_name, branch);
        assert_eq!(wizard.quick_start_entries.len(), 2);
        assert_eq!(wizard.quick_start_entries[0].agent_id, "codex");
        assert_eq!(
            wizard.quick_start_entries[0].model.as_deref(),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            wizard.quick_start_entries[0].resume_session_id.as_deref(),
            Some("sess-new")
        );
        assert!(wizard.quick_start_entries[0].skip_permissions);
        assert!(wizard.quick_start_entries[0].codex_fast_mode);
        assert_eq!(
            wizard.quick_start_entries[0].runtime_target,
            LaunchRuntimeTarget::Docker
        );
        assert_eq!(
            wizard.quick_start_entries[0].docker_service.as_deref(),
            Some("web")
        );
        assert_eq!(wizard.quick_start_entries[1].agent_id, "claude");
        assert!(!wizard.quick_start_entries[1].codex_fast_mode);
    }

    #[test]
    fn configure_existing_branch_wizard_with_sessions_loads_branch_live_sessions() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let cache = VersionCache::new();
        let repo_path = PathBuf::from("/tmp/repo");
        let worktree_path = repo_path.join("wt-feature-test");
        let branch = "feature/test";
        let now = Utc::now();

        let mut persisted =
            AgentSession::new(worktree_path.to_str().unwrap(), branch, AgentId::Codex);
        persisted.id = "agent-1".to_string();
        persisted.model = Some("gpt-5.3-codex".to_string());
        persisted.reasoning_level = Some("high".to_string());
        persisted.tool_version = Some("latest".to_string());
        persisted.agent_session_id = Some("sess-live".to_string());
        persisted.skip_permissions = true;
        persisted.codex_fast_mode = true;
        persisted.updated_at = now;
        persisted.created_at = now;
        persisted.last_activity_at = now;
        persisted.save(dir.path()).expect("persist live session");

        let detected = vec![detected_agent(AgentId::Codex, Some("0.5.1"))];
        let (mut wizard, _) = prepare_wizard_startup(repo_path.as_path(), None, detected, &cache);
        let mut model = test_model();
        model.sessions = vec![crate::model::SessionTab {
            id: "agent-1".to_string(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Cyan,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        }];
        model.active_session = 0;

        configure_existing_branch_wizard_with_sessions(
            &mut wizard,
            &model,
            &worktree_path,
            dir.path(),
            branch,
        );

        assert_eq!(wizard.live_session_entries.len(), 1);
        assert_eq!(wizard.live_session_entries[0].session_id, "agent-1");
        assert_eq!(wizard.live_session_entries[0].name, "Codex");
    }

    #[test]
    fn wizard_select_focus_existing_session_switches_active_session_without_launching() {
        let mut model = test_model();
        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: "agent-1".to_string(),
                name: "Codex".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "codex".to_string(),
                    color: crate::model::AgentColor::Cyan,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];
        model.active_session = 0;
        model.active_focus = FocusPane::BranchDetail;
        model.active_layer = ActiveLayer::Management;
        model.wizard = Some(screens::wizard::WizardState {
            step: screens::wizard::WizardStep::FocusExistingSession,
            live_session_entries: vec![screens::wizard::LiveSessionEntry {
                session_id: "agent-1".to_string(),
                kind: "Agent".to_string(),
                name: "Codex".to_string(),
                detail: Some("gpt-5.3-codex · high".to_string()),
                active: false,
            }],
            ..screens::wizard::WizardState::default()
        });

        update(
            &mut model,
            Message::Wizard(screens::wizard::WizardMessage::Select),
        );

        assert!(model.wizard.is_none());
        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert_eq!(model.active_session, 1);
        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert!(model.pending_launch_config.is_none());
    }

    #[test]
    fn branch_session_summaries_with_filters_to_selected_branch_and_marks_active() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        let other_worktree = repo_path.join("wt-feature-other");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");
        fs::create_dir_all(&other_worktree).expect("create other worktree");

        let mut model = Model::new(repo_path.clone());
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
            upstream: None,
        }];

        let mut matching = AgentSession::new(&selected_worktree, "feature/test", AgentId::Codex);
        matching.model = Some("gpt-5.3-codex".to_string());
        matching.reasoning_level = Some("high".to_string());
        matching.tool_version = Some("latest".to_string());
        matching.agent_session_id = Some("sess-abc".to_string());
        matching.skip_permissions = true;
        matching.runtime_target = LaunchRuntimeTarget::Docker;
        matching.docker_service = Some("web".to_string());
        matching.launch_command = "codex".to_string();
        matching.launch_args = vec![
            "--no-alt-screen".to_string(),
            "--model=gpt-5.3-codex".to_string(),
            "resume".to_string(),
            "sess-abc".to_string(),
            "--yolo".to_string(),
        ];
        matching.display_name = "Codex".to_string();
        matching.save(dir.path()).expect("persist matching session");

        let mut stale_branch =
            AgentSession::new(&selected_worktree, "feature/other", AgentId::ClaudeCode);
        stale_branch.display_name = "Claude Code".to_string();
        stale_branch.save(dir.path()).expect("persist stale branch");

        let mut stale_worktree =
            AgentSession::new(&other_worktree, "feature/test", AgentId::Gemini);
        stale_worktree.display_name = "Gemini CLI".to_string();
        stale_worktree
            .save(dir.path())
            .expect("persist stale worktree");

        model.sessions = vec![
            crate::model::SessionTab {
                id: matching.id.clone(),
                name: "Codex".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "codex".to_string(),
                    color: crate::model::AgentColor::Blue,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: stale_branch.id.clone(),
                name: "Claude Code".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "claude".to_string(),
                    color: crate::model::AgentColor::Green,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: stale_worktree.id.clone(),
                name: "Gemini CLI".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "gemini".to_string(),
                    color: crate::model::AgentColor::Cyan,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: "shell-branch".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];
        model.active_session = 0;

        let summaries = branch_session_summaries_with(&model, dir.path());

        assert_eq!(
            summaries,
            vec![
                screens::branches::DetailSessionSummary {
                    kind: "Agent",
                    name: "Codex".to_string(),
                    detail: Some("gpt-5.3-codex · high".to_string()),
                    active: true,
                    launch_summary: vec![
                        "Model: gpt-5.3-codex".to_string(),
                        "Reasoning: high".to_string(),
                        "Version: latest".to_string(),
                        "Resume session: sess-abc".to_string(),
                        "Permissions: Skip confirmations".to_string(),
                        "Runtime: Docker".to_string(),
                        "Docker service: web".to_string(),
                    ],
                    launch_command_line: Some(
                        "codex --no-alt-screen --model=gpt-5.3-codex resume sess-abc --yolo"
                            .to_string(),
                    ),
                },
                screens::branches::DetailSessionSummary {
                    kind: "Shell",
                    name: "Shell: feature/test".to_string(),
                    detail: None,
                    active: false,
                    launch_summary: Vec::new(),
                    launch_command_line: None,
                },
            ]
        );
    }

    #[test]
    fn branch_live_session_rendering_keeps_multiple_live_agents_for_same_branch() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");

        let mut model = Model::new(repo_path.clone());
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
            upstream: None,
        }];

        let running = AgentSession::new(&selected_worktree, "feature/test", AgentId::Codex);
        running.save(dir.path()).expect("persist running session");
        SessionRuntimeState::from_hook_event("PostToolUse")
            .expect("running runtime")
            .save(&runtime_state_path(dir.path(), &running.id))
            .expect("persist running runtime");

        let waiting = AgentSession::new(&selected_worktree, "feature/test", AgentId::ClaudeCode);
        waiting.save(dir.path()).expect("persist waiting session");
        SessionRuntimeState::from_hook_event("Stop")
            .expect("waiting runtime")
            .save(&runtime_state_path(dir.path(), &waiting.id))
            .expect("persist waiting runtime");

        model.sessions = vec![
            crate::model::SessionTab {
                id: waiting.id.clone(),
                name: "Claude Code".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "claude".to_string(),
                    color: crate::model::AgentColor::Green,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: running.id.clone(),
                name: "Codex".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "codex".to_string(),
                    color: crate::model::AgentColor::Blue,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];
        model.active_session = 1;

        let summaries = branch_live_session_summaries_with(&model, dir.path());
        let summary = summaries.get("feature/test").expect("branch live summary");
        assert_eq!(summary.indicators.len(), 2);
        assert_eq!(
            summary.indicators[0].status,
            gwt_agent::AgentStatus::Running
        );
        assert_eq!(summary.indicators[0].color, crate::model::AgentColor::Cyan);
        assert_eq!(
            summary.indicators[1].status,
            gwt_agent::AgentStatus::WaitingInput
        );
        assert_eq!(
            summary.indicators[1].color,
            crate::model::AgentColor::Yellow
        );
        model.branches.live_session_summaries = summaries;
        model.branches.session_animation_tick = 0;

        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                crate::screens::branches::render_list(&model.branches, frame, frame.area());
            })
            .expect("draw branches");

        let rendered = buffer_text(terminal.backend().buffer());
        let spinner_count = rendered
            .chars()
            .filter(|ch| matches!(ch, '◐' | '◓' | '◑' | '◒'))
            .count();
        let waiting_count = rendered.chars().filter(|ch| *ch == '●').count();

        assert_eq!(
            spinner_count, 1,
            "one live branch row should animate only the running agent session"
        );
        assert_eq!(
            waiting_count, 1,
            "one live branch row should keep the waiting agent visible with a static dot"
        );
        assert!(
            !rendered.contains("run ") && !rendered.contains("wait "),
            "branch rows should no longer render textual run/wait labels"
        );
    }

    #[test]
    fn branch_live_session_rendering_uses_agent_colors_for_running_and_waiting_indicators() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");

        let mut model = Model::new(repo_path.clone());
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
            upstream: None,
        }];

        let running = AgentSession::new(&selected_worktree, "feature/test", AgentId::Codex);
        running.save(dir.path()).expect("persist running session");
        SessionRuntimeState::from_hook_event("PostToolUse")
            .expect("running runtime")
            .save(&runtime_state_path(dir.path(), &running.id))
            .expect("persist running runtime");

        let waiting = AgentSession::new(&selected_worktree, "feature/test", AgentId::ClaudeCode);
        waiting.save(dir.path()).expect("persist waiting session");
        SessionRuntimeState::from_hook_event("Stop")
            .expect("waiting runtime")
            .save(&runtime_state_path(dir.path(), &waiting.id))
            .expect("persist waiting runtime");

        model.sessions = vec![
            crate::model::SessionTab {
                id: waiting.id.clone(),
                name: "Claude Code".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "claude".to_string(),
                    color: crate::model::AgentColor::Green,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: running.id.clone(),
                name: "Codex".to_string(),
                tab_type: SessionTabType::Agent {
                    agent_id: "codex".to_string(),
                    color: crate::model::AgentColor::Blue,
                },
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];
        model.active_session = 1;

        model.branches.live_session_summaries =
            branch_live_session_summaries_with(&model, dir.path());
        model.branches.session_animation_tick = 0;

        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                crate::screens::branches::render_list(&model.branches, frame, frame.area());
            })
            .expect("draw branches");

        let indicator_colors: Vec<Color> = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| matches!(cell.symbol(), "◐" | "◓" | "◑" | "◒" | "●"))
            .map(|cell| cell.fg)
            .collect();

        assert_eq!(
            indicator_colors,
            vec![Color::Cyan, Color::Yellow],
            "running and waiting indicators should keep per-agent colors so multiple agents remain distinguishable"
        );
    }

    #[test]
    fn tick_redraw_required_keeps_terminal_focus_animating_for_running_branch_indicators() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.management_tab = ManagementTab::Branches;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-test")),
            upstream: None,
        }];
        model.branches.live_session_summaries.insert(
            "feature/test".to_string(),
            screens::branches::BranchLiveSessionSummary {
                indicators: vec![screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status: gwt_agent::AgentStatus::Running,
                    color: crate::model::AgentColor::Cyan,
                    active: false,
                }],
            },
        );

        assert!(
            tick_redraw_required(&model),
            "Branches should keep redrawing while a running live-session indicator is visible, even when terminal focus owns input"
        );
    }

    #[test]
    fn tick_redraw_required_keeps_waiting_branch_indicators_static_under_terminal_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.management_tab = ManagementTab::Branches;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-test")),
            upstream: None,
        }];
        model.branches.live_session_summaries.insert(
            "feature/test".to_string(),
            screens::branches::BranchLiveSessionSummary {
                indicators: vec![screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status: gwt_agent::AgentStatus::WaitingInput,
                    color: crate::model::AgentColor::Yellow,
                    active: false,
                }],
            },
        );

        assert!(
            !tick_redraw_required(&model),
            "waiting-only live-session indicators should stay static and must not re-enable idle redraws under terminal focus"
        );
    }

    #[test]
    fn tick_redraw_required_ignores_filtered_out_running_branch_indicators() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.management_tab = ManagementTab::Branches;
        model.branches.view_mode = screens::branches::ViewMode::All;
        model.branches.branches = vec![
            screens::branches::BranchItem {
                name: "feature/visible".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: Some(PathBuf::from("/tmp/wt-feature-visible")),
                upstream: None,
            },
            screens::branches::BranchItem {
                name: "feature/hidden".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: Some(PathBuf::from("/tmp/wt-feature-hidden")),
                upstream: None,
            },
        ];
        model.branches.search_query = "visible".to_string();
        model.branches.live_session_summaries.insert(
            "feature/hidden".to_string(),
            screens::branches::BranchLiveSessionSummary {
                indicators: vec![screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status: gwt_agent::AgentStatus::Running,
                    color: crate::model::AgentColor::Blue,
                    active: false,
                }],
            },
        );

        assert!(
            !tick_redraw_required(&model),
            "running indicators on filtered-out rows must not force idle redraws under terminal focus"
        );
    }

    #[test]
    fn tick_redraw_required_ignores_running_branch_indicators_without_visible_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.management_tab = ManagementTab::Branches;
        model.terminal_size = (24, 8);
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/this-branch-name-is-too-wide".to_string(),
            is_head: true,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-wide")),
            upstream: None,
        }];
        model.branches.live_session_summaries.insert(
            "feature/this-branch-name-is-too-wide".to_string(),
            screens::branches::BranchLiveSessionSummary {
                indicators: vec![screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status: gwt_agent::AgentStatus::Running,
                    color: crate::model::AgentColor::Cyan,
                    active: false,
                }],
            },
        );

        assert!(
            !tick_redraw_required(&model),
            "running indicators with no visible summary strip must not re-enable idle redraws"
        );
    }

    #[test]
    fn should_render_after_tick_repaints_visible_branch_indicator_state_changes() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::Terminal;
        model.management_tab = ManagementTab::Branches;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-test")),
            upstream: None,
        }];
        model.branches.live_session_summaries.insert(
            "feature/test".to_string(),
            screens::branches::BranchLiveSessionSummary {
                indicators: vec![screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status: gwt_agent::AgentStatus::Running,
                    color: crate::model::AgentColor::Cyan,
                    active: false,
                }],
            },
        );
        let before = visible_branch_live_indicator_signature(&model);
        model.branches.live_session_summaries.insert(
            "feature/test".to_string(),
            screens::branches::BranchLiveSessionSummary {
                indicators: vec![screens::branches::BranchLiveSessionIndicator {
                    kind: screens::branches::BranchLiveSessionIndicatorKind::Agent,
                    status: gwt_agent::AgentStatus::WaitingInput,
                    color: crate::model::AgentColor::Cyan,
                    active: false,
                }],
            },
        );

        assert!(
            should_render_after_tick_with_visible_branch_signature(before, &model),
            "a visible running spinner must repaint once when it transitions to a static waiting dot"
        );
    }

    #[test]
    fn branch_live_session_rendering_uses_magenta_for_gemini_spinner() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let repo_path = dir.path().join("repo");
        let selected_worktree = repo_path.join("wt-feature-test");
        fs::create_dir_all(&selected_worktree).expect("create selected worktree");

        let mut model = Model::new(repo_path.clone());
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(selected_worktree.clone()),
            upstream: None,
        }];

        let running = AgentSession::new(&selected_worktree, "feature/test", AgentId::Gemini);
        running.save(dir.path()).expect("persist running session");
        SessionRuntimeState::from_hook_event("PostToolUse")
            .expect("running runtime")
            .save(&runtime_state_path(dir.path(), &running.id))
            .expect("persist running runtime");

        model.sessions = vec![crate::model::SessionTab {
            id: running.id.clone(),
            name: "Gemini CLI".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "gemini".to_string(),
                color: crate::model::AgentColor::Cyan,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        }];

        model.branches.live_session_summaries =
            branch_live_session_summaries_with(&model, dir.path());
        model.branches.session_animation_tick = 0;

        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                crate::screens::branches::render_list(&model.branches, frame, frame.area());
            })
            .expect("draw branches");

        let spinner_colors: Vec<Color> = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| matches!(cell.symbol(), "◐" | "◓" | "◑" | "◒"))
            .map(|cell| cell.fg)
            .collect();

        assert_eq!(
            spinner_colors,
            vec![Color::Magenta],
            "Gemini branch spinners should use the old-TUI magenta palette"
        );
    }

    #[test]
    fn load_custom_agents_from_path_parses_spec_schema() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[tools.customCodingAgents.my-agent]
id = "my-agent"
displayName = "My Agent"
agentType = "command"
command = "my-agent-cli"
defaultArgs = ["--flag"]

[tools.customCodingAgents.my-agent.modeArgs]
normal = ["--normal"]
continue = ["--continue"]
resume = ["--resume"]

[tools.customCodingAgents.my-agent.env]
CUSTOM_ENV = "enabled"
"#,
        )
        .expect("write config");

        let agents = load_custom_agents_from_path(&config_path).expect("load custom agents");

        assert_eq!(agents.len(), 1);
        let agent = &agents[0];
        assert_eq!(agent.id, "my-agent");
        assert_eq!(agent.display_name, "My Agent");
        assert_eq!(agent.agent_type, CustomAgentType::Command);
        assert_eq!(agent.command, "my-agent-cli");
        assert_eq!(agent.default_args, vec!["--flag"]);
        assert_eq!(
            agent
                .mode_args
                .as_ref()
                .map(|args| args.continue_mode.clone()),
            Some(vec!["--continue".to_string()])
        );
        assert_eq!(
            agent.env.get("CUSTOM_ENV").map(String::as_str),
            Some("enabled")
        );
    }

    #[test]
    fn build_wizard_agent_options_with_custom_agents_appends_settings_agents() {
        let dir = tempfile::tempdir().expect("temp custom path");
        let custom_path = dir.path().join("my-agent");
        fs::write(&custom_path, "#!/bin/sh\n").expect("write custom path");
        let cache = VersionCache::new();

        let (options, _) = build_wizard_agent_options_with_custom_agents(
            vec![detected_agent(AgentId::ClaudeCode, Some("1.0.55"))],
            &cache,
            &[sample_custom_agent(
                CustomAgentType::Path,
                custom_path.display().to_string(),
            )],
        );

        assert_eq!(options.len(), BUILTIN_AGENTS.len() + 1);
        let custom = options.last().expect("custom option");
        assert_eq!(custom.id, "my-agent");
        assert_eq!(custom.name, "My Agent");
        assert!(custom.available);
        assert!(custom.installed_version.is_none());
        assert!(custom.versions.is_empty());
        assert!(!custom.cache_outdated);
    }

    #[test]
    fn build_launch_config_from_wizard_omits_default_model_selection() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "Default (Opus 4.6)".to_string(),
            branch_name: "feature/spec-42".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.agent_id, AgentId::ClaudeCode);
        assert_eq!(config.branch.as_deref(), Some("feature/spec-42"));
        assert!(config.model.is_none());
        assert!(!config.args.iter().any(|arg| arg.contains("--model")));
    }

    #[test]
    fn build_launch_config_from_wizard_with_custom_agents_uses_custom_command_and_display_name() {
        let wizard = screens::wizard::WizardState {
            agent_id: "my-agent".to_string(),
            branch_name: "feature/custom-agent".to_string(),
            mode: "continue".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard_with_custom_agents(
            &wizard,
            &[sample_custom_agent(
                CustomAgentType::Command,
                "my-agent-cli",
            )],
        );

        assert_eq!(config.agent_id, AgentId::Custom("my-agent".to_string()));
        assert_eq!(config.command, "my-agent-cli");
        assert_eq!(
            config.args,
            vec!["--flag".to_string(), "--continue".to_string()]
        );
        assert_eq!(config.display_name, "My Agent");
        assert_eq!(config.branch.as_deref(), Some("feature/custom-agent"));
        assert_eq!(
            config.env_vars.get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
        assert_eq!(
            config.env_vars.get("CUSTOM_ENV").map(String::as_str),
            Some("enabled")
        );
        assert!(matches!(config.session_mode, SessionMode::Continue));
    }

    #[test]
    fn build_launch_config_from_wizard_carries_selected_base_branch_for_new_branch_flow() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("feature/source".to_string()),
            branch_name: "feature/child".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.branch.as_deref(), Some("feature/child"));
        assert_eq!(config.base_branch.as_deref(), Some("feature/source"));
    }

    #[test]
    fn build_launch_config_from_wizard_new_branch_ignores_selected_branch_worktree() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "feature/child".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/wt-develop")),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.branch.as_deref(), Some("feature/child"));
        assert_eq!(config.base_branch.as_deref(), Some("develop"));
        assert!(config.working_dir.is_none());
    }

    #[test]
    fn build_launch_config_from_wizard_defaults_spec_prefill_base_branch_to_develop() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            branch_name: "feature/spec-42-my-feature".to_string(),
            spec_context: Some(screens::wizard::SpecContext::new(
                "SPEC-42",
                "My Feature",
                "",
            )),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.base_branch.as_deref(), Some("develop"));
    }

    #[test]
    fn build_launch_config_from_wizard_keeps_selected_version() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            version: "latest".to_string(),
            branch_name: "feature/spec-42".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.agent_id, AgentId::ClaudeCode);
        assert_eq!(config.branch.as_deref(), Some("feature/spec-42"));
        assert_eq!(config.model.as_deref(), Some("sonnet"));
        assert_eq!(config.tool_version.as_deref(), Some("latest"));
    }

    #[test]
    fn build_launch_config_from_wizard_preserves_installed_version_mode() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            version: "installed".to_string(),
            branch_name: "feature/spec-42".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.agent_id, AgentId::ClaudeCode);
        assert_eq!(config.tool_version.as_deref(), Some("installed"));
        assert_eq!(config.command, "claude");
    }

    #[test]
    fn build_launch_config_from_wizard_carries_selected_issue_number() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            branch_name: "feature/issue-link".to_string(),
            issue_id: "1776".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.linked_issue_number, Some(1776));
    }

    #[test]
    fn build_launch_config_from_wizard_uses_resume_session_id_for_quick_start_resume() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            resume_session_id: Some("sess-123".to_string()),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.args.contains(&"--resume".to_string()));
        assert!(config.args.contains(&"sess-123".to_string()));
    }

    #[test]
    fn build_launch_config_from_wizard_codex_quick_start_resume_uses_resume_subcommand() {
        let wizard = screens::wizard::WizardState {
            agent_id: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            resume_session_id: Some("sess-123".to_string()),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.args.contains(&"resume".to_string()));
        assert!(config.args.contains(&"sess-123".to_string()));
    }

    #[test]
    fn build_launch_config_from_wizard_codex_disables_alternate_screen() {
        let wizard = screens::wizard::WizardState {
            agent_id: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            branch_name: "feature/spec-42".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(
            config.args.contains(&"--no-alt-screen".to_string()),
            "Codex launches should prefer inline mode so gwt can rely on PTY row scrollback instead of reconstructing page-sized snapshots: {:?}",
            config.args
        );
    }

    #[test]
    fn build_launch_config_from_wizard_falls_back_to_continue_without_resume_session_id() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.args.contains(&"--continue".to_string()));
        assert!(!config.args.contains(&"--resume".to_string()));
    }

    #[test]
    fn build_launch_config_from_wizard_codex_fast_mode_adds_service_tier_flag() {
        let wizard = screens::wizard::WizardState {
            agent_id: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            version: "0.113.0".to_string(),
            codex_fast_mode: true,
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.args.contains(&"-c".to_string()));
        assert!(config.args.contains(&"service_tier=fast".to_string()));
        assert!(config.codex_fast_mode);
    }

    #[test]
    fn build_launch_config_from_wizard_carries_docker_runtime_and_service_selection() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(
            config.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(config.docker_service.as_deref(), Some("web"));
        assert_eq!(
            config.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
    }

    #[test]
    fn resolve_docker_launch_plan_accepts_pwd_workspace_mount() {
        let project_root = tempfile::tempdir().expect("temp project root");
        fs::write(
            project_root.path().join("docker-compose.yml"),
            r#"
services:
  web:
    image: node:22
    volumes:
      - ${PWD}:/workspace
"#,
        )
        .expect("write compose");

        let plan =
            resolve_docker_launch_plan(project_root.path(), Some("web")).expect("launch plan");

        assert_eq!(plan.service, "web");
        assert_eq!(plan.container_cwd, "/workspace");
    }

    #[test]
    fn resolve_docker_launch_plan_accepts_absolute_project_root_workspace_mount() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let mount = format!("{}:/workspace", project_root.path().display());
        fs::write(
            project_root.path().join("docker-compose.yml"),
            format!(
                r#"
services:
  web:
    image: node:22
    volumes:
      - {mount}
"#
            ),
        )
        .expect("write compose");

        let plan =
            resolve_docker_launch_plan(project_root.path(), Some("web")).expect("launch plan");

        assert_eq!(plan.service, "web");
        assert_eq!(plan.container_cwd, "/workspace");
    }

    #[test]
    fn apply_docker_runtime_to_launch_config_wraps_command_for_selected_service() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let compose_path = project_root.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose");

        let mut config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: vec!["--print".to_string()],
            env_vars: HashMap::new(),
            working_dir: Some(project_root.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\\n'\n  exit 0\nfi\nexit 0\n",
            || {
            let expected_docker = std::env::var("GWT_DOCKER_BIN").expect("fake docker path");
            apply_docker_runtime_to_launch_config(project_root.path(), &mut config)
                .expect("apply docker runtime");

            assert_eq!(config.command, expected_docker);
            assert_eq!(
                config.args,
                vec![
                    "compose".to_string(),
                    "-f".to_string(),
                    compose_path.display().to_string(),
                    "exec".to_string(),
                    "-w".to_string(),
                    "/workspace".to_string(),
                    "web".to_string(),
                    "claude".to_string(),
                    "--print".to_string(),
                ]
            );
            assert_eq!(
                config.env_vars.get("GWT_PROJECT_ROOT").map(String::as_str),
                Some("/workspace")
            );
            assert_eq!(config.docker_service.as_deref(), Some("web"));
        },
        );
    }

    #[test]
    fn apply_docker_runtime_to_launch_config_forwards_valid_env_vars_into_exec_args() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let compose_path = project_root.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose");

        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CODE_EFFORT_LEVEL".to_string(), "high".to_string());
        env_vars.insert("TERM".to_string(), "xterm-256color".to_string());
        env_vars.insert("INVALID-KEY".to_string(), "ignored".to_string());
        let mut config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: vec!["--print".to_string()],
            env_vars,
            working_dir: Some(project_root.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: Some("high".to_string()),
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"web\" ] && [ \"$7\" = \"sh\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            || {
                apply_docker_runtime_to_launch_config(project_root.path(), &mut config)
                    .expect("apply docker runtime");

                assert_eq!(
                    config.args,
                    vec![
                        "compose".to_string(),
                        "-f".to_string(),
                        compose_path.display().to_string(),
                        "exec".to_string(),
                        "-w".to_string(),
                        "/workspace".to_string(),
                        "-e".to_string(),
                        "CLAUDE_CODE_EFFORT_LEVEL=high".to_string(),
                        "-e".to_string(),
                        "TERM=xterm-256color".to_string(),
                        "web".to_string(),
                        "claude".to_string(),
                        "--print".to_string(),
                    ]
                );
            },
        );
    }

    #[test]
    fn apply_docker_runtime_to_launch_config_adds_is_sandbox_for_root_claude_skip_permissions() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let compose_path = project_root.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose");

        let mut config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: vec!["--print".to_string()],
            env_vars: HashMap::new(),
            working_dir: Some(project_root.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: true,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"web\" ] && [ \"$7\" = \"sh\" ] && [ \"$8\" = \"-lc\" ] && [ \"$9\" = \"id -u\" ]; then\n  printf '0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"web\" ] && [ \"$7\" = \"sh\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            || {
                apply_docker_runtime_to_launch_config(project_root.path(), &mut config)
                    .expect("apply docker runtime");

                assert!(config.args.contains(&"-e".to_string()));
                assert!(config.args.contains(&"IS_SANDBOX=1".to_string()));
            },
        );
    }

    #[test]
    fn apply_docker_runtime_to_launch_config_omits_is_sandbox_for_non_root_claude() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let compose_path = project_root.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose");

        let mut config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: vec!["--print".to_string()],
            env_vars: HashMap::new(),
            working_dir: Some(project_root.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: true,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"web\" ] && [ \"$7\" = \"sh\" ] && [ \"$8\" = \"-lc\" ] && [ \"$9\" = \"id -u\" ]; then\n  printf '1000\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"web\" ] && [ \"$7\" = \"sh\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            || {
                apply_docker_runtime_to_launch_config(project_root.path(), &mut config)
                    .expect("apply docker runtime");

                assert!(!config.args.iter().any(|arg| arg == "IS_SANDBOX=1"));
            },
        );
    }

    #[test]
    fn apply_docker_runtime_to_launch_config_normalizes_package_runner_for_container_exec() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let compose_path = project_root.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose");

        let mut config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "/opt/homebrew/bin/bunx".to_string(),
            args: vec![
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ],
            env_vars: HashMap::new(),
            working_dir: Some(project_root.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: Some("latest".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"-w\" ] && [ \"$7\" = \"/workspace\" ] && [ \"$8\" = \"web\" ] && [ \"$9\" = \"bunx\" ] && [ \"${10}\" = \"@anthropic-ai/claude-code@latest\" ] && [ \"${11}\" = \"--version\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            || {
                let expected_docker = std::env::var("GWT_DOCKER_BIN").expect("fake docker path");
                apply_docker_runtime_to_launch_config(project_root.path(), &mut config)
                    .expect("apply docker runtime");

                assert_eq!(config.command, expected_docker);
                assert_eq!(
                    config.args,
                    vec![
                        "compose".to_string(),
                        "-f".to_string(),
                        compose_path.display().to_string(),
                        "exec".to_string(),
                        "-w".to_string(),
                        "/workspace".to_string(),
                        "web".to_string(),
                        "bunx".to_string(),
                        "@anthropic-ai/claude-code@latest".to_string(),
                        "--print".to_string(),
                    ]
                );
            },
        );
    }

    #[test]
    fn apply_docker_runtime_to_launch_config_falls_back_to_npx_when_bunx_package_exec_fails() {
        let project_root = tempfile::tempdir().expect("temp project root");
        let compose_path = project_root.path().join("docker-compose.yml");
        let log_path = project_root.path().join("docker-args.log");
        fs::write(
            &compose_path,
            r#"
services:
  web:
    image: node:22
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
        )
        .expect("write compose");

        let mut config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "/opt/homebrew/bin/bunx".to_string(),
            args: vec![
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ],
            env_vars: HashMap::new(),
            working_dir: Some(project_root.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: Some("latest".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("web".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"--version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\\trunning\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"web\" ] && [ \"$7\" = \"sh\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"-w\" ] && [ \"$7\" = \"/workspace\" ] && [ \"$8\" = \"web\" ] && [ \"$9\" = \"bunx\" ] && [ \"${{10}}\" = \"@anthropic-ai/claude-code@latest\" ] && [ \"${{11}}\" = \"--version\" ]; then\n  printf 'could not determine executable to run for package @anthropic-ai/claude-code\\n' >&2\n  exit 1\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"-w\" ] && [ \"$7\" = \"/workspace\" ] && [ \"$8\" = \"web\" ] && [ \"$9\" = \"npx\" ] && [ \"${{10}}\" = \"--yes\" ] && [ \"${{11}}\" = \"@anthropic-ai/claude-code@latest\" ] && [ \"${{12}}\" = \"--version\" ]; then\n  printf '2.1.100 (Claude Code)\\n'\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            log_path.display()
        );

        with_fake_docker(&script, || {
            let expected_docker = std::env::var("GWT_DOCKER_BIN").expect("fake docker path");
            apply_docker_runtime_to_launch_config(project_root.path(), &mut config)
                .expect("apply docker runtime");

            let log = fs::read_to_string(&log_path).expect("read invocation log");
            assert!(log.contains(" bunx @anthropic-ai/claude-code@latest --version"));
            assert!(log.contains(" npx --yes @anthropic-ai/claude-code@latest --version"));
            assert_eq!(config.command, expected_docker);
            assert_eq!(
                config.args,
                vec![
                    "compose".to_string(),
                    "-f".to_string(),
                    compose_path.display().to_string(),
                    "exec".to_string(),
                    "-w".to_string(),
                    "/workspace".to_string(),
                    "web".to_string(),
                    "npx".to_string(),
                    "--yes".to_string(),
                    "@anthropic-ai/claude-code@latest".to_string(),
                    "--print".to_string(),
                ]
            );
        });
    }

    #[test]
    fn materialize_pending_launch_with_missing_docker_routes_visible_error_without_session() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        write_docker_launch_compose_fixture(worktree.path());

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        with_docker_bin_override(&worktree.path().join("missing-docker"), || {
            materialize_pending_launch_with(&mut model, sessions_dir.path())
                .expect("materialize launch");
        });

        assert_eq!(model.session_count(), 1);
        assert_eq!(model.error_queue.len(), 1);
        let notification = model.error_queue.front().expect("error notification");
        assert_eq!(notification.source, "docker");
        assert_eq!(notification.message, "Docker launch failed");
        assert!(notification
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("Docker is not installed or not available on PATH"));
        assert!(
            fs::read_dir(sessions_dir.path())
                .expect("read sessions dir")
                .next()
                .is_none(),
            "failing Docker launch must not persist a session entry"
        );
    }

    #[test]
    fn materialize_pending_launch_with_missing_docker_runtime_command_routes_visible_error_without_session(
    ) {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        write_docker_launch_compose_fixture(worktree.path());

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: Some("installed".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'gwt\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"gwt\" ]; then\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            || {
                materialize_pending_launch_with(&mut model, sessions_dir.path())
                    .expect("materialize launch");
            },
        );

        assert_eq!(model.session_count(), 1);
        assert_eq!(model.error_queue.len(), 1);
        let notification = model.error_queue.front().expect("error notification");
        assert_eq!(notification.source, "docker");
        assert_eq!(notification.message, "Docker launch failed");
        assert!(notification
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("Command 'claude' is not available in Docker service 'gwt'"));
        assert!(
            fs::read_dir(sessions_dir.path())
                .expect("read sessions dir")
                .next()
                .is_none(),
            "launch must not persist a session when the Docker runtime command is missing"
        );
    }

    #[test]
    fn materialize_pending_launch_with_missing_docker_package_runner_routes_visible_error_without_session(
    ) {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        write_docker_launch_compose_fixture(worktree.path());

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "/opt/homebrew/bin/bunx".to_string(),
            args: vec![
                "@anthropic-ai/claude-code@latest".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ],
            env_vars: HashMap::new(),
            working_dir: Some(worktree.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: Some("latest".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: true,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        with_fake_docker(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'gwt\\trunning\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"gwt\" ] && [ \"$7\" = \"sh\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$9\" = \"bunx\" ]; then\n  printf 'could not determine executable to run for package @anthropic-ai/claude-code\\n' >&2\n  exit 1\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$9\" = \"npx\" ]; then\n  printf 'npm ERR! missing executable\\n' >&2\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            || {
                materialize_pending_launch_with(&mut model, sessions_dir.path())
                    .expect("materialize launch");
            },
        );

        assert_eq!(model.session_count(), 1);
        assert_eq!(model.error_queue.len(), 1);
        let notification = model.error_queue.front().expect("error notification");
        assert_eq!(notification.source, "docker");
        assert_eq!(notification.message, "Docker launch failed");
        let detail = notification.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("@anthropic-ai/claude-code@latest"));
        assert!(detail.contains("bunx"));
        assert!(detail.contains("npx"));
        assert!(
            fs::read_dir(sessions_dir.path())
                .expect("read sessions dir")
                .next()
                .is_none(),
            "launch must not persist a session when no Docker package runner can start the agent"
        );
    }

    #[test]
    fn materialize_pending_launch_async_docker_launch_shows_progress_and_hides_overlay_on_failure()
    {
        let worktree = tempfile::tempdir().expect("temp worktree");
        write_docker_launch_compose_fixture(worktree.path());
        let release_flag = worktree.path().join("release-compose-up");
        let running_flag = worktree.path().join("service-running");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: Some("installed".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  if [ -f '{}' ]; then\n    printf 'gwt\\n'\n  fi\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ] && [ \"$5\" = \"-d\" ] && [ \"$6\" = \"gwt\" ]; then\n  while [ ! -f '{}' ]; do\n    sleep 0.01\n  done\n  : > '{}'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"gwt\" ]; then\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            running_flag.display(),
            release_flag.display(),
            running_flag.display(),
        );

        with_fake_docker(&script, || {
            materialize_pending_launch(&mut model);

            assert!(model.docker_progress_events.is_some());
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::DetectingFiles
            );

            drive_docker_worker_until(
                &mut model,
                |model| {
                    model.docker_progress.as_ref().is_some_and(|progress| {
                        progress.stage == screens::docker_progress::DockerStage::BuildingImage
                    })
                },
                "docker build progress stage",
            );

            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress
                .message
                .contains("Building image and starting Docker service gwt"));

            fs::write(&release_flag, "").expect("release compose up");

            drive_docker_worker_until(
                &mut model,
                |model| model.docker_progress_events.is_none() && !model.error_queue.is_empty(),
                "docker launch failure",
            );

            assert!(model.docker_progress.is_none());
            assert_eq!(model.session_count(), 1);
            let notification = model.error_queue.front().expect("error notification");
            assert_eq!(notification.source, "docker");
            assert_eq!(notification.message, "Docker launch failed");
            assert!(notification
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("Command 'claude' is not available in Docker service 'gwt'"));
        });
    }

    #[test]
    fn materialize_pending_launch_async_docker_launch_streams_compose_output_into_logs() {
        let worktree = tempfile::tempdir().expect("temp worktree");
        write_docker_launch_compose_fixture(worktree.path());
        let release_flag = worktree.path().join("release-compose-up");
        let running_flag = worktree.path().join("service-running");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.path().to_path_buf()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: Some("installed".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  if [ -f '{}' ]; then\n    printf 'gwt\\n'\n  fi\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ] && [ \"$5\" = \"-d\" ] && [ \"$6\" = \"gwt\" ]; then\n  printf 'build step 1\\n'\n  printf 'build warning\\n' >&2\n  while [ ! -f '{}' ]; do\n    sleep 0.01\n  done\n  : > '{}'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"gwt\" ]; then\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            running_flag.display(),
            release_flag.display(),
            running_flag.display(),
        );

        with_fake_docker(&script, || {
            materialize_pending_launch(&mut model);

            drive_docker_worker_until(
                &mut model,
                |model| {
                    model.logs.entries.iter().any(|entry| {
                        entry.source == "docker"
                            && entry.message.contains("[gwt][stdout] build step 1")
                    }) && model.logs.entries.iter().any(|entry| {
                        entry.source == "docker"
                            && entry.message.contains("[gwt][stderr] build warning")
                    })
                },
                "docker compose output in logs",
            );

            assert!(model.current_notification.is_none());
            assert!(model.error_queue.is_empty());

            fs::write(&release_flag, "").expect("release compose up");

            drive_docker_worker_until(
                &mut model,
                |model| model.docker_progress_events.is_none() && !model.error_queue.is_empty(),
                "docker launch failure after logs",
            );
        });
    }

    #[test]
    fn materialize_pending_launch_with_stopped_docker_service_starts_it_before_exec() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        let compose_path = write_docker_launch_compose_fixture(worktree.path());
        let log_path = worktree.path().join("docker-args.log");
        let running_flag = worktree.path().join("service-running");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  if [ -f '{}' ]; then\n    printf 'gwt\\n'\n  fi\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ] && [ \"$5\" = \"-d\" ] && [ \"$6\" = \"gwt\" ]; then\n  : > '{}'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            log_path.display(),
            running_flag.display(),
            running_flag.display(),
        );

        with_fake_docker(&script, || {
            let mut model = test_model();
            model.pending_launch_config = Some(LaunchConfig {
                agent_id: AgentId::ClaudeCode,
                command: "/bin/echo".to_string(),
                args: vec!["agent-test".to_string()],
                env_vars: HashMap::new(),
                working_dir: Some(worktree.path().to_path_buf()),
                branch: Some("develop".to_string()),
                base_branch: None,
                display_name: "Claude Code".to_string(),
                color: AgentId::ClaudeCode.default_color(),
                model: None,
                tool_version: None,
                reasoning_level: None,
                session_mode: SessionMode::Normal,
                resume_session_id: None,
                runtime_target: LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Start,
                skip_permissions: false,
                codex_fast_mode: false,
                linked_issue_number: None,
            });

            materialize_pending_launch_with(&mut model, sessions_dir.path())
                .expect("materialize launch");

            let mut log = String::new();
            for _ in 0..50 {
                log = fs::read_to_string(&log_path).unwrap_or_default();
                if log.contains("compose -f")
                    && log.contains(" up -d gwt")
                    && log.contains(" exec -w /workspace gwt /bin/echo agent-test")
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            assert!(log.contains(&format!(
                "compose -f {} ps --all --format",
                compose_path.display()
            )));
            assert!(log.contains(&format!("compose -f {} up -d gwt", compose_path.display())));
            assert!(log.contains(&format!(
                "compose -f {} exec -w /workspace gwt /bin/echo agent-test",
                compose_path.display()
            )));
            assert_eq!(model.session_count(), 2);
            assert!(model.error_queue.is_empty());
        });
    }

    #[test]
    fn materialize_pending_launch_with_running_docker_service_restarts_before_exec() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        let compose_path = write_docker_launch_compose_fixture(worktree.path());
        let log_path = worktree.path().join("docker-args.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'gwt\\trunning\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"restart\" ] && [ \"$5\" = \"gwt\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            log_path.display(),
        );

        with_fake_docker(&script, || {
            let mut model = test_model();
            model.pending_launch_config = Some(LaunchConfig {
                agent_id: AgentId::ClaudeCode,
                command: "/bin/echo".to_string(),
                args: vec!["agent-test".to_string()],
                env_vars: HashMap::new(),
                working_dir: Some(worktree.path().to_path_buf()),
                branch: Some("develop".to_string()),
                base_branch: None,
                display_name: "Claude Code".to_string(),
                color: AgentId::ClaudeCode.default_color(),
                model: None,
                tool_version: None,
                reasoning_level: None,
                session_mode: SessionMode::Normal,
                resume_session_id: None,
                runtime_target: LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                skip_permissions: false,
                codex_fast_mode: false,
                linked_issue_number: None,
            });

            materialize_pending_launch_with(&mut model, sessions_dir.path())
                .expect("materialize launch");

            let mut log = String::new();
            for _ in 0..50 {
                log = fs::read_to_string(&log_path).unwrap_or_default();
                if log.contains(" compose -f ")
                    && log.contains(" restart gwt")
                    && log.contains(" exec -w /workspace gwt /bin/echo agent-test")
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            assert!(log.contains(&format!(
                "compose -f {} ps --all --format",
                compose_path.display()
            )));
            assert!(log.contains(&format!(
                "compose -f {} restart gwt",
                compose_path.display()
            )));
            assert!(log.contains(&format!(
                "compose -f {} exec -w /workspace gwt /bin/echo agent-test",
                compose_path.display()
            )));
            assert_eq!(model.session_count(), 2);
            assert!(model.error_queue.is_empty());
        });
    }

    #[test]
    fn materialize_pending_launch_with_running_docker_service_recreates_before_exec() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        let compose_path = write_docker_launch_compose_fixture(worktree.path());
        let log_path = worktree.path().join("docker-args.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'gwt\\trunning\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ] && [ \"$5\" = \"-d\" ] && [ \"$6\" = \"--force-recreate\" ] && [ \"$7\" = \"gwt\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            log_path.display(),
        );

        with_fake_docker(&script, || {
            let mut model = test_model();
            model.pending_launch_config = Some(LaunchConfig {
                agent_id: AgentId::ClaudeCode,
                command: "/bin/echo".to_string(),
                args: vec!["agent-test".to_string()],
                env_vars: HashMap::new(),
                working_dir: Some(worktree.path().to_path_buf()),
                branch: Some("develop".to_string()),
                base_branch: None,
                display_name: "Claude Code".to_string(),
                color: AgentId::ClaudeCode.default_color(),
                model: None,
                tool_version: None,
                reasoning_level: None,
                session_mode: SessionMode::Normal,
                resume_session_id: None,
                runtime_target: LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Recreate,
                skip_permissions: false,
                codex_fast_mode: false,
                linked_issue_number: None,
            });

            materialize_pending_launch_with(&mut model, sessions_dir.path())
                .expect("materialize launch");

            let mut log = String::new();
            for _ in 0..50 {
                log = fs::read_to_string(&log_path).unwrap_or_default();
                if log.contains(" up -d --force-recreate gwt")
                    && log.contains(" exec -w /workspace gwt /bin/echo agent-test")
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            assert!(log.contains(&format!(
                "compose -f {} up -d --force-recreate gwt",
                compose_path.display()
            )));
            assert!(log.contains(&format!(
                "compose -f {} exec -w /workspace gwt /bin/echo agent-test",
                compose_path.display()
            )));
            assert_eq!(model.session_count(), 2);
            assert!(model.error_queue.is_empty());
        });
    }

    #[test]
    fn materialize_pending_launch_with_service_that_never_starts_routes_error_without_session() {
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = tempfile::tempdir().expect("temp worktree");
        write_docker_launch_compose_fixture(worktree.path());
        let script = "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'Docker version 27.0.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"info\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$2\" = \"version\" ]; then\n  printf 'Docker Compose version v2.27.0\\n'\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ] && [ \"$5\" = \"-d\" ] && [ \"$6\" = \"gwt\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"logs\" ]; then\n  printf 'boot failed\\n'\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n";

        with_fake_docker(script, || {
            let mut model = test_model();
            model.pending_launch_config = Some(LaunchConfig {
                agent_id: AgentId::ClaudeCode,
                command: "claude".to_string(),
                args: Vec::new(),
                env_vars: HashMap::new(),
                working_dir: Some(worktree.path().to_path_buf()),
                branch: Some("develop".to_string()),
                base_branch: None,
                display_name: "Claude Code".to_string(),
                color: AgentId::ClaudeCode.default_color(),
                model: None,
                tool_version: None,
                reasoning_level: None,
                session_mode: SessionMode::Normal,
                resume_session_id: None,
                runtime_target: LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                skip_permissions: false,
                codex_fast_mode: false,
                linked_issue_number: None,
            });

            materialize_pending_launch_with(&mut model, sessions_dir.path())
                .expect("materialize launch");

            assert_eq!(model.session_count(), 1);
            assert_eq!(model.error_queue.len(), 1);
            let notification = model.error_queue.front().expect("error notification");
            assert_eq!(notification.source, "docker");
            assert_eq!(notification.message, "Docker launch failed");
            assert!(notification
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("docker compose service 'gwt' is not running after startup."));
            assert!(
                fs::read_dir(sessions_dir.path())
                    .expect("read sessions dir")
                    .next()
                    .is_none(),
                "launch must not persist a session when Docker service never becomes ready"
            );
        });
    }

    #[test]
    fn build_launch_config_from_wizard_codex_skip_permissions_does_not_imply_fast_mode() {
        let wizard = screens::wizard::WizardState {
            agent_id: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            skip_perms: true,
            codex_fast_mode: false,
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.skip_permissions);
        assert!(!config.codex_fast_mode);
        assert!(config.args.contains(&"--yolo".to_string()));
        assert!(!config.args.contains(&"service_tier=fast".to_string()));
    }

    #[test]
    fn build_launch_config_from_wizard_claude_skip_permissions_uses_dangerous_flag() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            skip_perms: true,
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.skip_permissions);
        assert!(config
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn build_launch_config_from_wizard_claude_without_skip_permissions_omits_dangerous_flag() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            skip_perms: false,
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(!config.skip_permissions);
        assert!(!config
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn link_selected_issue_to_branch_with_builds_gh_issue_develop_command() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        let config = LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: None,
            branch: Some("feature/issue-link".to_string()),
            base_branch: Some("develop".to_string()),
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            linked_issue_number: Some(1776),
        };

        let mut observed_cwd = None;
        let mut observed_args = Vec::new();
        link_selected_issue_to_branch_with(repo.path(), &config, |cwd, args| {
            observed_cwd = Some(cwd.to_path_buf());
            observed_args.extend(args.iter().cloned());
            Ok(())
        })
        .expect("link issue");

        assert_eq!(observed_cwd.as_deref(), Some(repo.path()));
        assert_eq!(
            observed_args,
            [
                "issue",
                "develop",
                "1776",
                "--name",
                "feature/issue-link",
                "--base",
                "develop",
            ]
        );
    }

    #[test]
    fn build_launch_config_from_wizard_claude_effort_auto_persists_without_env() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "opus".to_string(),
            reasoning: "auto".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.reasoning_level.as_deref(), Some("auto"));
        assert!(!config.env_vars.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
    }

    #[test]
    fn build_launch_config_from_wizard_claude_effort_medium_exports_env() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "sonnet".to_string(),
            reasoning: "medium".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.reasoning_level.as_deref(), Some("medium"));
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_EFFORT_LEVEL"),
            Some(&"medium".to_string())
        );
    }

    #[test]
    fn build_launch_config_from_wizard_claude_effort_high_exports_env() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "opus".to_string(),
            reasoning: "high".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert_eq!(config.reasoning_level.as_deref(), Some("high"));
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_EFFORT_LEVEL"),
            Some(&"high".to_string())
        );
    }

    #[test]
    fn build_launch_config_from_wizard_claude_haiku_ignores_effort_selection() {
        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            model: "haiku".to_string(),
            reasoning: "high".to_string(),
            ..Default::default()
        };

        let config = build_launch_config_from_wizard(&wizard);

        assert!(config.reasoning_level.is_none());
        assert!(!config.env_vars.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
    }

    // SPEC-6 Phase 5: `append_agent_launch_log_with` and its redaction
    // helper were removed. Agent launches now emit a structured
    // `tracing::info!(target: "gwt_tui::agent::launch", ...)` event
    // that lands in `~/.gwt/logs/gwt.log.YYYY-MM-DD` alongside every
    // other event. No redaction (FR-016).

    #[test]
    fn materialize_pending_launch_with_creates_agent_session_and_persists_metadata() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut model = test_model();
        model.pending_launch_config = Some(
            AgentLaunchBuilder::new(AgentId::ClaudeCode)
                .branch("feature/spec-42")
                .model("sonnet")
                .version("latest")
                .build(),
        );

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        assert!(model.pending_launch_config.is_none());
        assert_eq!(model.sessions.len(), 2);
        assert_eq!(model.active_layer, ActiveLayer::Main);
        let session_tab = model.active_session_tab().expect("active launched session");
        assert_eq!(session_tab.name, "Claude Code");
        assert_eq!(
            session_tab.tab_type,
            SessionTabType::Agent {
                agent_id: "claude".to_string(),
                color: crate::model::AgentColor::Yellow,
            }
        );

        let mut entries = fs::read_dir(dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(entries.len(), 1);

        let persisted = AgentSession::load(&entries[0]).expect("load persisted session");
        assert_eq!(persisted.agent_id, AgentId::ClaudeCode);
        assert_eq!(persisted.branch, "feature/spec-42");
        assert_eq!(persisted.model.as_deref(), Some("sonnet"));
        assert_eq!(persisted.tool_version.as_deref(), Some("latest"));
        assert_eq!(persisted.display_name, "Claude Code");
    }

    #[test]
    fn materialize_pending_launch_with_issue_link_failure_stops_before_session_creation() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::ClaudeCode,
            command: "claude".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: None,
            branch: Some("feature/issue-link".to_string()),
            base_branch: Some("develop".to_string()),
            display_name: "Claude Code".to_string(),
            color: AgentId::ClaudeCode.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: Some(1776),
        });

        let result = materialize_pending_launch_with_hooks(
            &mut model,
            dir.path(),
            |_repo_path, _config| Err("gh issue develop failed".to_string()),
            |_repo_path, _config| {
                panic!("resolve_launch_worktree should not run after link failure")
            },
        );

        assert_eq!(result, Err("gh issue develop failed".to_string()));
        assert_eq!(
            model.sessions.len(),
            1,
            "launch should stop before creating a new tab"
        );
        assert!(
            fs::read_dir(dir.path())
                .expect("read sessions dir")
                .next()
                .is_none(),
            "failed link should not persist a session"
        );
    }

    #[test]
    fn materialize_pending_launch_with_issue_link_persists_local_linkage_store() {
        with_temp_home(|_home| {
            let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
            let repo_path = workspace_dir.path().join("repo");
            fs::create_dir_all(&repo_path).expect("create repo path");
            init_git_repo(&repo_path);
            let remote_path = workspace_dir.path().join("origin.git");
            init_bare_git_repo(&remote_path);
            git_add_remote(&repo_path, "origin", &remote_path);

            let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
            let mut model = Model::new(repo_path.clone());
            model.pending_launch_config = Some(LaunchConfig {
                agent_id: AgentId::Custom("my-agent".to_string()),
                command: "gwt-missing-custom-agent-command".to_string(),
                args: Vec::new(),
                env_vars: HashMap::new(),
                working_dir: Some(repo_path.clone()),
                branch: Some("feature/issue-link".to_string()),
                base_branch: Some("develop".to_string()),
                display_name: "My Agent".to_string(),
                color: AgentId::Custom("my-agent".to_string()).default_color(),
                model: None,
                tool_version: None,
                reasoning_level: None,
                session_mode: SessionMode::Normal,
                resume_session_id: None,
                runtime_target: LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                skip_permissions: false,
                codex_fast_mode: false,
                linked_issue_number: Some(1776),
            });

            materialize_pending_launch_with_hooks(
                &mut model,
                sessions_dir.path(),
                |_repo_path, _config| Ok(()),
                |_repo_path, _config| Ok(()),
            )
            .expect("materialize launch");

            let linkage_store_path =
                default_issue_linkage_store_path(&repo_path).expect("repo hash-backed store path");
            let issue_branches = load_issue_linkage_map(Some(linkage_store_path.as_path()));
            assert_eq!(
                issue_branches.get(&1776),
                Some(&vec!["feature/issue-link".to_string()])
            );
        });
    }

    #[test]
    fn materialize_pending_launch_with_generates_claude_settings_local_hooks() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-spec-42");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Custom("my-agent".to_string()),
            command: "gwt-missing-custom-agent-command".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.clone()),
            branch: Some("feature/spec-42".to_string()),
            base_branch: None,
            display_name: "My Agent".to_string(),
            color: AgentId::Custom("my-agent".to_string()).default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let settings_path = worktree.join(".claude/settings.local.json");
        let content = fs::read_to_string(&settings_path).expect("read settings.local");
        let value: serde_json::Value = serde_json::from_str(&content).expect("parse settings");

        let command = value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
            .as_str()
            .expect("hook command");
        assert!(command.contains(" hook runtime-state UserPromptSubmit"));
        assert!(!command.contains("GWT_MANAGED_HOOK"));
        assert!(!command.contains("node"));
        assert_eq!(
            value["hooks"]["PreToolUse"][1]["matcher"],
            serde_json::Value::String("Bash".to_string())
        );
    }

    #[test]
    fn materialize_pending_launch_with_generates_codex_hooks() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-spec-42");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Codex,
            command: "gwt-missing-custom-agent-command".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.clone()),
            branch: Some("feature/spec-42".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let hooks_path = worktree.join(".codex/hooks.json");
        let content = fs::read_to_string(&hooks_path).expect("read codex hooks");
        let value: serde_json::Value = serde_json::from_str(&content).expect("parse codex hooks");
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("hook command");

        assert!(command.contains(" hook runtime-state SessionStart"));
        assert!(!command.contains("GWT_MANAGED_HOOK"));
        assert!(!command.contains("node"));
    }

    #[test]
    fn materialize_pending_launch_with_updates_git_resolved_exclude_for_linked_worktree() {
        let dir = tempfile::tempdir().expect("temp repo dir");
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        git_commit_allow_empty(&repo, "initial commit");

        let worktree = dir.path().join("wt-feature-spec-42");
        let add_worktree = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/spec-42",
                worktree.to_str().expect("worktree path"),
            ])
            .current_dir(&repo)
            .output()
            .expect("git worktree add");
        assert!(
            add_worktree.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&add_worktree.stderr)
        );
        assert!(
            worktree.join(".git").is_file(),
            "linked worktree should expose .git as a file"
        );

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Codex,
            command: "gwt-missing-custom-agent-command".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.clone()),
            branch: Some("feature/spec-42".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let exclude_path = git_resolved_exclude_path(&worktree);
        let exclude = fs::read_to_string(&exclude_path).expect("read git exclude");
        assert!(exclude.contains("# gwt-managed-begin"));
        assert!(exclude.contains(".codex/hooks.json"));
        assert!(
            !worktree.join(".git/info/exclude").exists(),
            "agent launch should not treat linked worktree .git as a directory"
        );
    }

    #[test]
    fn materialize_pending_launch_with_migrates_tracked_legacy_codex_runtime_hooks() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-develop");
        fs::create_dir_all(worktree.join(".codex")).expect("create .codex");
        fs::write(
            worktree.join(".codex/hooks.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {
                    "SessionStart": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "node \"$(git rev-parse --show-toplevel)/.codex/hooks/scripts/gwt-forward-hook.mjs\" SessionStart",
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .expect("serialize tracked hooks"),
        )
        .expect("write tracked hooks");
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg(&worktree)
            .status()
            .expect("git init")
            .success());
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&worktree)
            .arg("add")
            .arg(".codex/hooks.json")
            .status()
            .expect("git add tracked hooks")
            .success());

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Codex,
            command: "gwt-missing-custom-agent-command".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree.clone()),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let hooks_path = worktree.join(".codex/hooks.json");
        let content = fs::read_to_string(&hooks_path).expect("read migrated codex hooks");
        let value: serde_json::Value =
            serde_json::from_str(&content).expect("parse migrated codex hooks");
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("hook command");

        assert!(command.contains(" hook runtime-state SessionStart"));
        assert!(!command.contains("GWT_MANAGED_HOOK"));
        assert!(!content.contains("gwt-forward-hook.mjs"));
        assert!(!command.contains("node"));
    }

    #[test]
    fn materialize_pending_launch_with_prepares_claude_settings_before_agent_process_starts() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-spec-42");
        fs::create_dir_all(&worktree).expect("create worktree");
        let marker = dir.path().join("settings-check.txt");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Custom("my-agent".to_string()),
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "if [ -f .claude/settings.local.json ]; then printf present > \"$1\"; else printf missing > \"$1\"; fi".to_string(),
                "sh".to_string(),
                marker.to_string_lossy().into_owned(),
            ],
            env_vars: HashMap::new(),
            working_dir: Some(worktree),
            branch: Some("feature/spec-42".to_string()),
            base_branch: None,
            display_name: "My Agent".to_string(),
            color: AgentId::Custom("my-agent".to_string()).default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let mut observed = None;
        for _ in 0..50 {
            if let Ok(value) = fs::read_to_string(&marker) {
                if !value.is_empty() {
                    observed = Some(value);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(observed.as_deref(), Some("present"));
    }

    #[test]
    fn materialize_pending_launch_with_prepares_codex_hooks_before_agent_process_starts() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-spec-42");
        fs::create_dir_all(&worktree).expect("create worktree");
        let marker = dir.path().join("hooks-check.txt");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Codex,
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "if [ -f .codex/hooks.json ]; then printf present > \"$1\"; else printf missing > \"$1\"; fi".to_string(),
                "sh".to_string(),
                marker.to_string_lossy().into_owned(),
            ],
            env_vars: HashMap::new(),
            working_dir: Some(worktree),
            branch: Some("feature/spec-42".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let mut observed = None;
        for _ in 0..50 {
            if let Ok(value) = fs::read_to_string(&marker) {
                if !value.is_empty() {
                    observed = Some(value);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(observed.as_deref(), Some("present"));
    }

    #[test]
    fn materialize_pending_launch_with_prunes_stale_gwt_assets_before_agent_process_starts() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-spec-42");
        fs::create_dir_all(worktree.join(".claude/commands")).expect("create claude commands");
        fs::create_dir_all(worktree.join(".codex/skills/gwt-agent-read"))
            .expect("create stale codex skill");
        fs::create_dir_all(worktree.join(".claude/skills/gwt-pr/references"))
            .expect("create stale nested claude skill path");
        fs::write(
            worktree.join(".claude/commands/gwt-issue-search.md"),
            "legacy command",
        )
        .expect("write stale command");
        fs::write(
            worktree.join(".codex/skills/gwt-agent-read/SKILL.md"),
            "legacy skill",
        )
        .expect("write stale skill");
        fs::write(
            worktree.join(".claude/skills/gwt-pr/references/legacy.md"),
            "legacy nested skill file",
        )
        .expect("write stale nested skill file");
        let marker = dir.path().join("cleanup-check.txt");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Custom("my-agent".to_string()),
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "if [ ! -e .claude/commands/gwt-issue-search.md ] && [ ! -e .codex/skills/gwt-agent-read ] && [ ! -e .claude/skills/gwt-pr/references/legacy.md ]; then printf pruned > \"$1\"; else printf stale > \"$1\"; fi".to_string(),
                "sh".to_string(),
                marker.to_string_lossy().into_owned(),
            ],
            env_vars: HashMap::new(),
            working_dir: Some(worktree.clone()),
            branch: Some("feature/spec-42".to_string()),
            base_branch: None,
            display_name: "My Agent".to_string(),
            color: AgentId::Custom("my-agent".to_string()).default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let mut observed = None;
        for _ in 0..50 {
            if let Ok(value) = fs::read_to_string(&marker) {
                if !value.is_empty() {
                    observed = Some(value);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(observed.as_deref(), Some("pruned"));
        assert!(
            !worktree
                .join(".claude/commands/gwt-issue-search.md")
                .exists(),
            "stale command should be removed before spawn"
        );
        assert!(
            !worktree.join(".codex/skills/gwt-agent-read").exists(),
            "stale skill should be removed before spawn"
        );
        assert!(
            !worktree
                .join(".claude/skills/gwt-pr/references/legacy.md")
                .exists(),
            "stale nested skill file should be removed before spawn"
        );
    }

    #[test]
    fn materialize_pending_launch_with_bootstraps_waiting_runtime_sidecar_after_spawn() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-develop");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Codex,
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            env_vars: HashMap::new(),
            working_dir: Some(worktree),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let session_id = model
            .sessions
            .last()
            .expect("launched session tab")
            .id
            .clone();
        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &session_id))
            .expect("bootstrap runtime state");
        assert_eq!(runtime.status, gwt_agent::AgentStatus::WaitingInput);
        assert_eq!(runtime.source_event.as_deref(), Some("LaunchBootstrap"));
    }

    #[test]
    fn materialize_pending_launch_with_does_not_leave_bootstrap_runtime_sidecar_on_spawn_failure() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-develop");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut model = test_model();
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Codex,
            command: "gwt-missing-custom-agent-command".to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: Some(worktree),
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let session_id = model
            .sessions
            .last()
            .expect("launched session tab")
            .id
            .clone();
        assert!(
            !runtime_state_path(dir.path(), &session_id).exists(),
            "failed launches must not leave a stale running sidecar behind"
        );
    }

    #[test]
    fn inject_agent_hook_runtime_env_sets_session_identifiers() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut env = HashMap::from([(String::from("EXISTING"), String::from("1"))]);

        inject_agent_hook_runtime_env(&mut env, dir.path(), "session-123");

        assert_eq!(env.get("EXISTING").map(String::as_str), Some("1"));
        assert_eq!(
            env.get(gwt_agent::GWT_SESSION_ID_ENV).map(String::as_str),
            Some("session-123")
        );
        assert_eq!(
            env.get(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
                .map(String::as_str),
            Some(
                runtime_state_path(dir.path(), "session-123")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    #[test]
    fn augment_agent_hook_runtime_launch_config_adds_codex_runtime_namespace_after_session_id() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut config = LaunchConfig {
            agent_id: AgentId::Codex,
            command: "codex".to_string(),
            args: vec!["--enable".to_string(), "codex_hooks".to_string()],
            env_vars: HashMap::new(),
            working_dir: None,
            branch: Some("develop".to_string()),
            base_branch: None,
            display_name: "Codex".to_string(),
            color: AgentId::Codex.default_color(),
            model: None,
            tool_version: Some("latest".to_string()),
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        };

        augment_agent_hook_runtime_launch_config(&mut config, dir.path(), "session-123");

        let expected = runtime_state_path(dir.path(), "session-123")
            .parent()
            .expect("runtime parent")
            .to_string_lossy()
            .into_owned();
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--add-dir" && pair[1] == expected));
    }

    #[test]
    fn close_active_session_with_marks_agent_session_stopped() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-test");
        fs::create_dir_all(&worktree).expect("create worktree");

        let persisted = AgentSession::new(&worktree, "feature/test", AgentId::Codex);
        persisted.save(dir.path()).expect("persist agent session");
        SessionRuntimeState::from_hook_event("SessionStart")
            .expect("running runtime")
            .save(&runtime_state_path(dir.path(), &persisted.id))
            .expect("persist running runtime");

        let mut model = test_model();
        model.sessions.push(crate::model::SessionTab {
            id: persisted.id.clone(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.active_session = 1;

        close_active_session_with(&mut model, dir.path());

        assert_eq!(model.session_count(), 1);
        let persisted = AgentSession::load(&dir.path().join(format!("{}.toml", persisted.id)))
            .expect("load stopped agent session");
        assert_eq!(persisted.status, gwt_agent::AgentStatus::Stopped);
        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &persisted.id))
            .expect("load stopped runtime");
        assert_eq!(runtime.status, gwt_agent::AgentStatus::Stopped);
    }

    #[test]
    fn check_pty_exits_with_marks_agent_session_stopped() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-test");
        fs::create_dir_all(&worktree).expect("create worktree");

        let persisted = AgentSession::new(&worktree, "feature/test", AgentId::Codex);
        persisted.save(dir.path()).expect("persist agent session");

        let mut model = test_model();
        model.sessions.push(crate::model::SessionTab {
            id: persisted.id.clone(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.active_session = 1;

        spawn_pty_for_session(
            &mut model,
            &persisted.id,
            gwt_terminal::pty::SpawnConfig {
                command: "/bin/sh".to_string(),
                args: vec!["-lc".to_string(), "exit 0".to_string()],
                cols: 80,
                rows: 24,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
            },
        )
        .expect("spawn short-lived PTY");

        for _ in 0..50 {
            let exited = model
                .pty_handles
                .get(&persisted.id)
                .and_then(|pty| pty.try_wait().ok().flatten())
                .is_some();
            if exited {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        check_pty_exits_with(&mut model, dir.path());

        assert_eq!(model.session_count(), 1);
        let persisted = AgentSession::load(&dir.path().join(format!("{}.toml", persisted.id)))
            .expect("load stopped agent session");
        assert_eq!(persisted.status, gwt_agent::AgentStatus::Stopped);
    }

    #[test]
    fn refresh_active_session_branches_with_ignores_stopped_agent_sessions() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-test");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut persisted = AgentSession::new(&worktree, "feature/test", AgentId::Codex);
        persisted.update_status(gwt_agent::AgentStatus::Stopped);
        persisted
            .save(dir.path())
            .expect("persist stopped agent session");

        let mut model = test_model();
        model.sessions.push(crate::model::SessionTab {
            id: persisted.id.clone(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(worktree.clone()),
            upstream: None,
        }];
        model.branches.set_merge_state(
            "feature/test",
            screens::branches::MergeState::Cleanable(gwt_git::MergeTarget::Develop),
        );

        refresh_active_session_branches_with(&mut model, dir.path());

        assert!(
            !model
                .branches
                .active_session_branches
                .contains("feature/test"),
            "stopped agent sessions must not block cleanup selection"
        );
        assert_eq!(
            model.branches.toggle_cleanup_selection("feature/test"),
            screens::branches::CleanupSelectionToggle::Selected
        );
    }

    #[test]
    fn refresh_active_session_branches_with_keeps_running_agent_sessions_blocked() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let worktree = dir.path().join("wt-feature-test");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut persisted = AgentSession::new(&worktree, "feature/test", AgentId::Codex);
        persisted.update_status(gwt_agent::AgentStatus::Running);
        persisted
            .save(dir.path())
            .expect("persist running agent session");

        let mut model = test_model();
        model.sessions.push(crate::model::SessionTab {
            id: persisted.id.clone(),
            name: "Codex".to_string(),
            tab_type: SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Blue,
            },
            vt: crate::model::VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(worktree),
            upstream: None,
        }];
        model.branches.set_merge_state(
            "feature/test",
            screens::branches::MergeState::Cleanable(gwt_git::MergeTarget::Develop),
        );

        refresh_active_session_branches_with(&mut model, dir.path());

        assert!(model
            .branches
            .active_session_branches
            .contains("feature/test"));
        assert_eq!(
            model.branches.toggle_cleanup_selection("feature/test"),
            screens::branches::CleanupSelectionToggle::Blocked(
                screens::branches::CleanupSelectionBlockedReason::ActiveSession
            )
        );
    }

    #[test]
    fn materialize_pending_launch_with_persists_quick_start_restore_fields() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut model = test_model();
        let wizard = screens::wizard::WizardState {
            agent_id: "codex".to_string(),
            model: "gpt-5.3-codex".to_string(),
            reasoning: "high".to_string(),
            version: "latest".to_string(),
            branch_name: "feature/spec-42".to_string(),
            mode: "resume".to_string(),
            resume_session_id: Some("sess-abc".to_string()),
            skip_perms: true,
            codex_fast_mode: true,
            ..Default::default()
        };
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        let entry = fs::read_dir(dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&entry).expect("load persisted session");
        assert_eq!(persisted.reasoning_level.as_deref(), Some("high"));
        assert!(persisted.skip_permissions);
        assert!(persisted.codex_fast_mode);
        assert_eq!(persisted.agent_session_id.as_deref(), Some("sess-abc"));
        assert!(
            persisted.launch_command.ends_with("bunx") || persisted.launch_command.ends_with("npx"),
            "latest tool versions should persist the resolved runner command"
        );
        assert!(
            persisted
                .launch_args
                .first()
                .is_some_and(|arg| arg == "@openai/codex@latest" || arg == "--yes"),
            "runner-prefixed argv should persist the package launcher prefix"
        );
        assert!(
            persisted
                .launch_args
                .iter()
                .any(|arg| arg == "--no-alt-screen"),
            "launch args should persist the finalized argv"
        );
        assert!(
            persisted
                .launch_args
                .iter()
                .any(|arg| arg == "@openai/codex@latest"),
            "package version selection should be persisted in the final argv"
        );
        assert!(
            persisted
                .launch_args
                .iter()
                .any(|arg| arg == "--model=gpt-5.3-codex"),
            "model override should be reflected in the persisted argv"
        );
        assert!(
            persisted.launch_args.iter().any(|arg| arg == "resume"),
            "resume subcommand should be persisted"
        );
        assert!(
            persisted.launch_args.iter().any(|arg| arg == "sess-abc"),
            "resume target should be persisted"
        );
        assert!(
            persisted.launch_args.iter().any(|arg| arg == "--yolo"),
            "skip permissions flag should be reflected in persisted argv"
        );
    }

    #[test]
    fn materialize_pending_launch_with_new_branch_creates_worktree_and_persists_actual_path() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_branch_or_create(&repo_path, "develop");
        git_push_branch(&repo_path, "develop");

        let mut model = Model::new(repo_path.clone());
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Custom("my-agent".to_string()),
            command: "/bin/echo".to_string(),
            args: vec!["agent-test".to_string()],
            env_vars: HashMap::new(),
            working_dir: None,
            branch: Some("feature/alpha/beta".to_string()),
            base_branch: None,
            display_name: "My Agent".to_string(),
            color: AgentId::Custom("my-agent".to_string()).default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, sessions_dir.path())
            .expect("materialize launch");

        let expected_worktree = workspace_dir
            .path()
            .join("feature")
            .join("alpha")
            .join("beta");
        assert!(expected_worktree.exists(), "new worktree should exist");
        let expected_worktree =
            std::fs::canonicalize(&expected_worktree).expect("canonicalize expected worktree");

        let branch_output = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&expected_worktree)
            .output()
            .expect("read worktree branch");
        assert!(
            branch_output.status.success(),
            "git branch --show-current failed: {}",
            String::from_utf8_lossy(&branch_output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&branch_output.stdout).trim(),
            "feature/alpha/beta"
        );

        let remote_output = std::process::Command::new("git")
            .args([
                "ls-remote",
                "--exit-code",
                "--heads",
                "origin",
                "feature/alpha/beta",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("read remote branch");
        assert!(
            remote_output.status.success(),
            "remote branch must exist: {}",
            String::from_utf8_lossy(&remote_output.stderr)
        );

        let session_entry = fs::read_dir(sessions_dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&session_entry).expect("load persisted session");
        assert_eq!(persisted.branch, "feature/alpha/beta");
        assert_eq!(persisted.worktree_path, expected_worktree);
    }

    #[test]
    fn materialize_pending_launch_with_new_branch_from_selected_branch_creates_new_worktree() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_branch_or_create(&repo_path, "develop");
        git_push_branch(&repo_path, "develop");

        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "feature/launch-from-selected".to_string(),
            worktree_path: Some(repo_path.clone()),
            ..Default::default()
        };

        let mut model = Model::new(repo_path.clone());
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, sessions_dir.path())
            .expect("materialize launch");

        let expected_worktree = workspace_dir
            .path()
            .join("feature")
            .join("launch-from-selected");
        assert!(expected_worktree.exists(), "new worktree should exist");
        let expected_worktree =
            std::fs::canonicalize(&expected_worktree).expect("canonicalize expected worktree");

        let branch_output = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&expected_worktree)
            .output()
            .expect("read worktree branch");
        assert!(
            branch_output.status.success(),
            "git branch --show-current failed: {}",
            String::from_utf8_lossy(&branch_output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&branch_output.stdout).trim(),
            "feature/launch-from-selected"
        );

        let remote_output = std::process::Command::new("git")
            .args([
                "ls-remote",
                "--exit-code",
                "--heads",
                "origin",
                "feature/launch-from-selected",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("read remote branch");
        assert!(
            remote_output.status.success(),
            "remote branch must exist: {}",
            String::from_utf8_lossy(&remote_output.stderr)
        );

        let session_entry = fs::read_dir(sessions_dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&session_entry).expect("load persisted session");
        assert_eq!(persisted.branch, "feature/launch-from-selected");
        assert_eq!(persisted.worktree_path, expected_worktree);
    }

    #[test]
    fn materialize_pending_launch_with_linked_worktree_uses_main_repo_branch_layout() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let develop_worktree = workspace_dir.path().join("develop");
        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "develop",
                develop_worktree.to_str().expect("develop worktree path"),
            ])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add -b");
        assert!(
            output.status.success(),
            "git worktree add -b failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        git_push_branch(&repo_path, "develop");

        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "feature/test".to_string(),
            worktree_path: Some(develop_worktree.clone()),
            ..Default::default()
        };

        let mut model = Model::new(develop_worktree.clone());
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, sessions_dir.path())
            .expect("materialize launch");

        let expected_worktree = workspace_dir.path().join("feature").join("test");
        let expected_worktree =
            std::fs::canonicalize(&expected_worktree).expect("canonicalize expected worktree");
        assert!(
            expected_worktree.exists(),
            "new sibling worktree should exist"
        );
        assert!(
            !workspace_dir.path().join("develop-feature-test").exists(),
            "linked worktree name must not be used as sibling-layout repo prefix"
        );

        let branch_output = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&expected_worktree)
            .output()
            .expect("read worktree branch");
        assert!(
            branch_output.status.success(),
            "git branch --show-current failed: {}",
            String::from_utf8_lossy(&branch_output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&branch_output.stdout).trim(),
            "feature/test"
        );

        let session_entry = std::fs::read_dir(sessions_dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&session_entry).expect("load persisted session");
        assert_eq!(persisted.worktree_path, expected_worktree);
    }

    #[test]
    fn materialize_pending_launch_with_bare_workspace_linked_worktree_uses_branch_hierarchy_layout()
    {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let bare_repo_path = workspace_dir.path().join("gwt.git");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        init_bare_git_repo(&bare_repo_path);

        let bootstrap_path = workspace_dir.path().join("bootstrap");
        git_clone_repo(&bare_repo_path, &bootstrap_path);
        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&bootstrap_path)
            .output()
            .expect("set git email");
        assert!(email.status.success(), "git config user.email failed");
        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&bootstrap_path)
            .output()
            .expect("set git name");
        assert!(name.status.success(), "git config user.name failed");
        git_checkout_branch_or_create(&bootstrap_path, "develop");
        git_commit_allow_empty(&bootstrap_path, "initial commit");
        git_push_branch(&bootstrap_path, "develop");
        git_add_remote(&bare_repo_path, "origin", &bare_repo_path);

        let develop_worktree = workspace_dir.path().join("develop");
        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                develop_worktree.to_str().expect("develop worktree path"),
                "develop",
            ])
            .current_dir(&bare_repo_path)
            .output()
            .expect("git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "feature/test".to_string(),
            worktree_path: Some(develop_worktree.clone()),
            ..Default::default()
        };

        let mut model = Model::new(develop_worktree);
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, sessions_dir.path())
            .expect("materialize launch");

        let expected_worktree = workspace_dir.path().join("feature").join("test");
        let expected_worktree =
            std::fs::canonicalize(&expected_worktree).expect("canonicalize expected worktree");
        assert!(
            expected_worktree.exists(),
            "new sibling worktree should exist for bare workspace layout"
        );
        assert!(
            !workspace_dir.path().join("develop-feature-test").exists(),
            "bare workspace linked worktree name must not be used as repo prefix"
        );

        let session_entry = std::fs::read_dir(sessions_dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&session_entry).expect("load persisted session");
        assert_eq!(persisted.worktree_path, expected_worktree);
    }

    #[test]
    fn materialize_pending_launch_with_existing_branch_worktree_reuses_previous_path() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        let develop_worktree = workspace_dir.path().join("develop");
        let develop_output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "develop",
                develop_worktree.to_str().expect("develop worktree path"),
            ])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add -b develop");
        assert!(
            develop_output.status.success(),
            "git worktree add -b develop failed: {}",
            String::from_utf8_lossy(&develop_output.stderr)
        );

        let stale_worktree = workspace_dir.path().join("develop-feature-test");
        let stale_output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/test",
                stale_worktree.to_str().expect("stale worktree path"),
                "develop",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add -b feature/test");
        assert!(
            stale_output.status.success(),
            "git worktree add -b feature/test failed: {}",
            String::from_utf8_lossy(&stale_output.stderr)
        );
        let stale_worktree =
            std::fs::canonicalize(&stale_worktree).expect("canonicalize stale worktree");

        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "feature/test".to_string(),
            worktree_path: Some(develop_worktree.clone()),
            ..Default::default()
        };

        let mut model = Model::new(develop_worktree);
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, sessions_dir.path())
            .expect("materialize launch");

        assert!(
            !workspace_dir.path().join("feature").join("test").exists(),
            "launch should reuse the existing branch worktree instead of trying to create a new sibling path"
        );

        let session_entry = std::fs::read_dir(sessions_dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&session_entry).expect("load persisted session");
        assert_eq!(persisted.branch, "feature/test");
        assert_eq!(persisted.worktree_path, stale_worktree);
    }

    #[test]
    fn materialize_pending_launch_with_occupied_preferred_worktree_path_uses_suffixed_fallback() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");

        git_checkout_branch_or_create(&repo_path, "main");
        git_push_branch(&repo_path, "main");
        git_checkout_branch_or_create(&repo_path, "develop");
        git_push_branch(&repo_path, "develop");
        git_checkout_branch_or_create(&repo_path, "main");

        let occupied_path = workspace_dir.path().join("develop");
        let occupied_output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "dependabot/npm_and_yarn/test",
                occupied_path.to_str().expect("occupied worktree path"),
                "develop",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add occupied path");
        assert!(
            occupied_output.status.success(),
            "git worktree add occupied path failed: {}",
            String::from_utf8_lossy(&occupied_output.stderr)
        );

        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "develop".to_string(),
            worktree_path: Some(repo_path.clone()),
            ..Default::default()
        };

        let mut model = Model::new(repo_path);
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        materialize_pending_launch_with(&mut model, sessions_dir.path())
            .expect("materialize launch with occupied preferred path");

        let expected_worktree = workspace_dir.path().join("develop-2");
        let expected_worktree =
            std::fs::canonicalize(&expected_worktree).expect("canonicalize fallback worktree");
        assert!(
            expected_worktree.exists(),
            "launch should create a suffixed fallback path when the canonical branch path is occupied by another worktree"
        );

        let session_entry = std::fs::read_dir(sessions_dir.path())
            .expect("read sessions dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .expect("session entry");
        let persisted = AgentSession::load(&session_entry).expect("load persisted session");
        assert_eq!(persisted.branch, "develop");
        assert_eq!(persisted.worktree_path, expected_worktree);
    }

    #[test]
    fn materialize_pending_launch_with_missing_remote_base_branch_returns_error() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        let sessions_dir = tempfile::tempdir().expect("temp sessions dir");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_checkout_branch_or_create(&repo_path, "main");
        git_push_branch(&repo_path, "main");

        let wizard = screens::wizard::WizardState {
            agent_id: "claude".to_string(),
            is_new_branch: true,
            base_branch_name: Some("develop".to_string()),
            branch_name: "feature/needs-base".to_string(),
            worktree_path: Some(repo_path.clone()),
            ..Default::default()
        };

        let mut model = Model::new(repo_path);
        model.pending_launch_config = Some(build_launch_config_from_wizard(&wizard));

        let result = materialize_pending_launch_with(&mut model, sessions_dir.path());
        assert!(
            result.is_err(),
            "launch should fail when origin/develop is missing"
        );
        let err = result.expect_err("error");
        assert!(
            err.contains("remote base branch does not exist: origin/develop"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn materialize_pending_launch_with_spawn_failure_mentions_command() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut model = test_model();
        let missing_command = "gwt-missing-custom-agent-command";
        model.pending_launch_config = Some(LaunchConfig {
            agent_id: AgentId::Custom("my-agent".to_string()),
            command: missing_command.to_string(),
            args: Vec::new(),
            env_vars: HashMap::new(),
            working_dir: None,
            branch: Some("feature/custom-agent".to_string()),
            base_branch: None,
            display_name: "My Agent".to_string(),
            color: AgentId::Custom("my-agent".to_string()).default_color(),
            model: None,
            tool_version: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            skip_permissions: false,
            codex_fast_mode: false,
            linked_issue_number: None,
        });

        materialize_pending_launch_with(&mut model, dir.path()).expect("materialize launch");

        assert_eq!(model.error_queue.len(), 1);
        let notification = model.error_queue.front().expect("error notification");
        assert!(notification.message.contains(missing_command));
    }

    #[test]
    fn schedule_startup_version_cache_refresh_with_schedules_stale_refreshable_agents() {
        let _guard = VERSION_CACHE_SCHEDULER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.store(false, Ordering::Release);

        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("agent-versions.json");
        let mut cache = VersionCache::new();
        cache.entries.insert(
            "claude-code".into(),
            version_entry(&["1.0.54", "1.0.53"], 90_000),
        );
        cache
            .entries
            .insert("codex".into(), version_entry(&["0.5.0"], 60));
        cache.save(&cache_path).unwrap();

        let (task, scheduled) = capture_startup_version_cache_refresh_task(
            cache_path.clone(),
            std::sync::Arc::new(|| {
                vec![
                    detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
                    detected_agent(AgentId::Codex, Some("0.5.1")),
                    detected_agent(AgentId::OpenCode, Some("0.2.0")),
                ]
            }),
        );
        task();
        let (scheduled_path, targets) = scheduled.lock().unwrap().take().unwrap();
        assert_eq!(scheduled_path, cache_path);
        assert!(targets.contains(&AgentId::ClaudeCode));
        assert!(!targets.contains(&AgentId::Codex));
        assert!(targets.contains(&AgentId::Gemini));
    }

    #[test]
    fn schedule_startup_version_cache_refresh_with_schedules_missing_cache_entries() {
        let _guard = VERSION_CACHE_SCHEDULER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.store(false, Ordering::Release);

        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("agent-versions.json");
        let (task, scheduled) = capture_startup_version_cache_refresh_task(
            cache_path.clone(),
            std::sync::Arc::new(|| {
                vec![
                    detected_agent(AgentId::Gemini, Some("0.2.0")),
                    detected_agent(AgentId::OpenCode, Some("0.4.0")),
                ]
            }),
        );
        task();
        let (scheduled_path, targets) = scheduled.lock().unwrap().take().unwrap();
        assert_eq!(scheduled_path, cache_path);
        assert!(targets.contains(&AgentId::ClaudeCode));
        assert!(targets.contains(&AgentId::Codex));
        assert!(targets.contains(&AgentId::Gemini));
    }

    #[test]
    fn schedule_startup_version_cache_refresh_with_defers_detection_until_spawned_task_runs() {
        let _guard = VERSION_CACHE_SCHEDULER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.store(false, Ordering::Release);

        let cache_path = PathBuf::from("/tmp/agent-versions.json");
        let detected = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let detected_flag = detected.clone();
        let (task, scheduled) = capture_startup_version_cache_refresh_task(
            cache_path.clone(),
            std::sync::Arc::new(move || {
                detected_flag.store(true, Ordering::Release);
                vec![detected_agent(AgentId::ClaudeCode, Some("1.0.55"))]
            }),
        );

        assert!(!detected.load(Ordering::Acquire));
        assert!(scheduled.lock().unwrap().is_none());
        assert!(STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.load(Ordering::Acquire));
        task();

        assert!(detected.load(Ordering::Acquire));
        let (scheduled_path, targets) = scheduled.lock().unwrap().clone().unwrap();
        assert_eq!(scheduled_path, cache_path);
        assert!(targets.contains(&AgentId::ClaudeCode));
        assert!(!STARTUP_VERSION_CACHE_REFRESH_DISPATCH_IN_FLIGHT.load(Ordering::Acquire));
    }

    #[test]
    fn schedule_wizard_version_cache_refresh_with_defers_refresh_until_spawned_task_runs() {
        let _guard = VERSION_CACHE_SCHEDULER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.store(false, Ordering::Release);

        let spawned = std::cell::RefCell::new(None::<Box<dyn FnOnce() + Send>>);
        let refreshed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let refreshed_flag = refreshed.clone();

        schedule_wizard_version_cache_refresh_with(
            PathBuf::from("/tmp/agent-versions.json"),
            vec![AgentId::ClaudeCode],
            |task| {
                *spawned.borrow_mut() = Some(task);
            },
            move |_, _| {
                refreshed_flag.store(true, Ordering::Release);
            },
        );

        assert!(!refreshed.load(Ordering::Acquire));
        assert!(spawned.borrow().is_some());
        assert!(WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.load(Ordering::Acquire));

        let task = spawned.borrow_mut().take().unwrap();
        task();

        assert!(refreshed.load(Ordering::Acquire));
        assert!(!WIZARD_VERSION_CACHE_REFRESH_IN_FLIGHT.load(Ordering::Acquire));
    }

    #[test]
    fn open_session_conversion_with_opens_picker_for_alternative_agents() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);

        open_session_conversion_with(
            &mut model,
            vec![
                detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
                detected_agent(AgentId::Codex, Some("0.5.1")),
                detected_agent(AgentId::Gemini, Some("0.2.0")),
            ],
        );

        let picker = model.service_select.as_ref().unwrap();
        assert!(picker.visible);
        assert_eq!(picker.title, "Select Agent");
        assert_eq!(
            picker.services,
            vec!["Codex".to_string(), "Gemini CLI".to_string()]
        );
        assert_eq!(
            picker.values,
            vec!["codex".to_string(), "gemini".to_string()]
        );
    }

    #[test]
    fn apply_pending_session_conversion_with_updates_active_session_and_preserves_repo_path() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        let original_repo_path = model.repo_path.clone();

        apply_pending_session_conversion_with(
            &mut model,
            PendingSessionConversion {
                session_index: 0,
                target_agent_id: "codex".to_string(),
                target_display_name: "Codex".to_string(),
            },
            vec![detected_agent(AgentId::Codex, Some("0.5.1"))],
        )
        .unwrap();

        let converted = &model.sessions[0];
        assert_eq!(converted.name, "Codex");
        assert_eq!(
            converted.tab_type,
            SessionTabType::Agent {
                agent_id: "codex".to_string(),
                color: crate::model::AgentColor::Cyan,
            }
        );
        assert_eq!(converted.vt.rows(), 30);
        assert_eq!(converted.vt.cols(), 100);
        assert_eq!(model.repo_path, original_repo_path);
    }

    #[test]
    fn apply_pending_session_conversion_with_preserves_original_session_on_failure() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        let original_name = model.sessions[0].name.clone();
        let original_tab_type = model.sessions[0].tab_type.clone();

        let err = apply_pending_session_conversion_with(
            &mut model,
            PendingSessionConversion {
                session_index: 0,
                target_agent_id: "gemini".to_string(),
                target_display_name: "Gemini CLI".to_string(),
            },
            vec![detected_agent(AgentId::Codex, Some("0.5.1"))],
        )
        .unwrap_err();

        assert!(err.contains("gemini"));
        assert_eq!(model.sessions[0].name, original_name);
        assert_eq!(model.sessions[0].tab_type, original_tab_type);
    }

    #[test]
    fn update_open_session_conversion_for_agent_session_opens_picker() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);

        open_session_conversion_with(
            &mut model,
            vec![
                detected_agent(AgentId::ClaudeCode, Some("1.0.55")),
                detected_agent(AgentId::Codex, Some("0.5.1")),
            ],
        );

        assert!(model.service_select.is_some());
        assert!(model.pending_session_conversion.is_none());
    }

    #[test]
    fn update_service_select_select_sets_pending_conversion_and_opens_confirm() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        model.service_select = Some(screens::service_select::ServiceSelectState::with_options(
            "Select Agent",
            vec!["Codex".to_string()],
            vec!["codex".to_string()],
        ));

        update(
            &mut model,
            Message::ServiceSelect(screens::service_select::ServiceSelectMessage::Select),
        );

        assert!(model.service_select.is_none());
        assert_eq!(
            model.pending_session_conversion,
            Some(PendingSessionConversion {
                session_index: 0,
                target_agent_id: "codex".to_string(),
                target_display_name: "Codex".to_string(),
            })
        );
        assert!(model.confirm.visible);
        assert_eq!(model.confirm.message, "Convert session to Codex?");
    }

    #[test]
    fn handle_confirm_message_with_accept_applies_pending_session_conversion_and_logs_info() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        model.pending_session_conversion = Some(PendingSessionConversion {
            session_index: 0,
            target_agent_id: "codex".to_string(),
            target_display_name: "Codex".to_string(),
        });
        model.confirm = screens::confirm::ConfirmState::with_message("Convert?");
        model.confirm.selected = screens::confirm::ConfirmChoice::Yes;

        let target = detected_agent(AgentId::Codex, Some("0.5.1"));
        let target_name = target.agent_id.display_name().to_string();
        let target_command = target.agent_id.command().to_string();
        let target_color = tui_agent_color(target.agent_id.default_color());
        handle_confirm_message_with(
            &mut model,
            screens::confirm::ConfirmMessage::Accept,
            vec![target],
        );

        assert_eq!(model.sessions[0].name, target_name);
        assert_eq!(
            model.sessions[0].tab_type,
            SessionTabType::Agent {
                agent_id: target_command,
                color: target_color,
            }
        );
        assert_eq!(model.logs.entries.last().unwrap().source, "session");
        assert!(model.current_notification.is_some());
        assert!(model.pending_session_conversion.is_none());
    }

    #[test]
    fn handle_confirm_message_with_failure_routes_error_queue() {
        let mut model = test_model();
        model.sessions[0] =
            agent_session_tab("Claude Code", "claude", crate::model::AgentColor::Green);
        let original_name = model.sessions[0].name.clone();
        let original_tab_type = model.sessions[0].tab_type.clone();
        model.pending_session_conversion = Some(PendingSessionConversion {
            session_index: 0,
            target_agent_id: "missing-agent".to_string(),
            target_display_name: "Missing Agent".to_string(),
        });
        model.confirm = screens::confirm::ConfirmState::with_message("Convert?");
        model.confirm.selected = screens::confirm::ConfirmChoice::Yes;

        handle_confirm_message_with(&mut model, screens::confirm::ConfirmMessage::Accept, vec![]);

        assert_eq!(model.sessions[0].name, original_name);
        assert_eq!(model.sessions[0].tab_type, original_tab_type);
        assert_eq!(model.error_queue.len(), 1);
        assert!(model
            .logs
            .entries
            .last()
            .unwrap()
            .message
            .contains("missing-agent"));
        assert!(model.pending_session_conversion.is_none());
    }

    // Legacy single-branch worktree delete via Ctrl+C → Confirm has been
    // removed in favor of the multi-select Branch Cleanup flow (FR-018).
    // The end-to-end coverage now lives in `cleanup_run_*` tests below and
    // in `screens::branches::tests::*` plus `screens::cleanup_*::tests::*`.

    #[test]
    fn maybe_start_wizard_branch_suggestions_with_applies_result() {
        let mut wizard = screens::wizard::WizardState::default();
        wizard.step = screens::wizard::WizardStep::AIBranchSuggest;
        wizard.ai_suggest.loading = true;
        wizard.spec_context = Some(screens::wizard::SpecContext::new(
            "SPEC-42",
            "My Feature",
            "# SPEC-42\n\nDetailed implementation notes",
        ));

        maybe_start_wizard_branch_suggestions_with(&mut wizard, |_| {
            Ok(vec!["feature/spec-42-my-feature".into()])
        });

        assert!(!wizard.ai_suggest.loading);
        assert_eq!(
            wizard.ai_suggest.suggestions,
            vec!["feature/spec-42-my-feature".to_string()]
        );
    }

    #[test]
    fn maybe_start_wizard_branch_suggestions_with_applies_error() {
        let mut wizard = screens::wizard::WizardState::default();
        wizard.step = screens::wizard::WizardStep::AIBranchSuggest;
        wizard.ai_suggest.loading = true;

        maybe_start_wizard_branch_suggestions_with(&mut wizard, |_| {
            Err("missing AI configuration".to_string())
        });

        assert!(!wizard.ai_suggest.loading);
        assert_eq!(
            wizard.ai_suggest.error.as_deref(),
            Some("missing AI configuration")
        );
    }

    #[test]
    fn wizard_branch_suggestion_context_includes_spec_and_branch_seed() {
        let mut wizard = screens::wizard::WizardState::default();
        wizard.branch_name = "feature/spec-7-voice".into();
        wizard.issue_id = "1776".into();
        wizard.spec_context = Some(screens::wizard::SpecContext::new(
            "SPEC-7",
            "Voice settings",
            "# Voice settings\n\nCapture the selected microphone and language.\n",
        ));

        let context = wizard_branch_suggestion_context(&wizard);

        assert!(context.contains("SPEC: SPEC-7 - Voice settings"));
        assert!(context.contains("SPEC body:"));
        assert!(context.contains("Capture the selected microphone and language."));
        assert!(context.contains("Current branch seed: feature/spec-7-voice"));
        assert!(context.contains("Issue: 1776"));
    }

    #[test]
    fn update_key_input_in_main_layer_queues_pty_bytes() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;

        // Call key_event_to_bytes + push directly to verify conversion,
        // because update() now drains pending inputs immediately.
        let bytes = key_event_to_bytes(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(bytes, Some(vec![0x03]));

        push_input_to_active_session(&mut model, bytes.unwrap());
        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(forwarded.bytes, vec![0x03]);
    }

    #[test]
    fn forward_key_to_active_session_returns_row_scrollback_to_live() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(18, 8));
        for i in 0..12 {
            append_session_line(&mut model, "shell-0", &format!("line-{i}"));
        }

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .viewing_history(),
            "precondition: shell session should be browsing history before key input"
        );

        forward_key_to_active_session(&mut model, key(KeyCode::Char('a'), KeyModifiers::NONE));

        let session = model.active_session_tab().expect("active session");
        assert!(session.vt.follow_live());
        assert!(!session.vt.viewing_history());
        assert_eq!(session.vt.scrollback(), 0);
        let forwarded = model.pending_pty_inputs().back().expect("queued key input");
        assert_eq!(forwarded.bytes, b"a".to_vec());
    }

    #[test]
    fn forward_key_to_active_session_returns_snapshot_history_to_live() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;
        update(&mut model, Message::Resize(24, 8));
        enter_alt_screen_with_text(&mut model, "shell-0", "frame-1");
        replace_alt_screen_text(&mut model, "shell-0", "frame-2");

        let area = active_session_text_area(&model).expect("active session text area");
        update(
            &mut model,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert!(
            model
                .active_session_tab()
                .expect("active session")
                .vt
                .viewing_history(),
            "precondition: full-screen session should be browsing snapshot history before key input"
        );

        forward_key_to_active_session(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

        let session = model.active_session_tab().expect("active session");
        assert!(session.vt.follow_live());
        assert!(!session.vt.viewing_history());
        let visible = session.vt.visible_screen_parser().screen().contents();
        assert!(visible.contains("frame-2"));
        assert!(!visible.contains("frame-1"));
        let forwarded = model
            .pending_pty_inputs()
            .back()
            .expect("queued enter input");
        assert_eq!(forwarded.bytes, vec![b'\r']);
    }

    #[test]
    fn key_event_to_bytes_maps_backtab_to_escape_sequence() {
        let bytes = key_event_to_bytes(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(bytes, Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn update_voice_transcription_result_queues_pty_bytes() {
        let mut model = test_model();
        handle_voice_message(
            &mut model,
            VoiceInputMessage::TranscriptionResult("git status".into()),
            true,
        );

        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(forwarded.bytes, b"git status".to_vec());
        assert_eq!(model.voice.buffer, "git status");
    }

    #[test]
    fn update_voice_transcription_result_ignores_empty_text() {
        let mut model = test_model();
        handle_voice_message(
            &mut model,
            VoiceInputMessage::TranscriptionResult("   ".into()),
            true,
        );

        assert!(model.pending_pty_inputs().is_empty());
        assert_eq!(model.voice.buffer, "   ");
    }

    #[test]
    fn handle_voice_start_recording_is_noop_when_disabled() {
        let mut model = test_model();

        handle_voice_message(&mut model, VoiceInputMessage::StartRecording, false);

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Idle);
        assert!(model.pending_pty_inputs().is_empty());
    }

    #[test]
    fn handle_voice_start_recording_transitions_when_enabled() {
        let mut model = test_model();

        handle_voice_message(&mut model, VoiceInputMessage::StartRecording, true);

        assert_eq!(
            model.voice.status,
            crate::input::voice::VoiceStatus::Recording
        );
    }

    #[test]
    fn handle_voice_start_recording_with_runtime_error_sets_error_state() {
        let mut model = test_model();

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut FakeVoiceRuntime::start_error("backend missing"),
        );

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Error);
        assert_eq!(
            model.voice.error_message.as_deref(),
            Some("backend missing")
        );
    }

    #[test]
    fn handle_voice_start_recording_toggle_stops_and_injects_transcript() {
        let mut model = test_model();
        let mut runtime = FakeVoiceRuntime::success("git status");

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut runtime,
        );
        assert_eq!(
            model.voice.status,
            crate::input::voice::VoiceStatus::Recording
        );

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut runtime,
        );

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Idle);
        assert_eq!(model.voice.buffer, "git status");
        let pending = model
            .pending_pty_inputs
            .pop_front()
            .expect("pty input queued");
        assert_eq!(pending.bytes, b"git status".to_vec());
    }

    #[test]
    fn handle_voice_stop_recording_with_runtime_error_sets_error_state() {
        let mut model = test_model();
        let mut runtime = FakeVoiceRuntime::stop_error("transcription failed");

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StartRecording,
            true,
            &mut runtime,
        );
        assert_eq!(
            model.voice.status,
            crate::input::voice::VoiceStatus::Recording
        );

        handle_voice_message_with_runtime(
            &mut model,
            VoiceInputMessage::StopRecording,
            true,
            &mut runtime,
        );

        assert_eq!(model.voice.status, crate::input::voice::VoiceStatus::Error);
        assert_eq!(
            model.voice.error_message.as_deref(),
            Some("transcription failed")
        );
    }

    #[test]
    fn build_paste_input_bytes_wraps_payload_when_bracketed_paste_is_enabled() {
        let bytes = build_paste_input_bytes("git status\npwd", true).unwrap();
        assert_eq!(bytes, b"\x1b[200~git status\npwd\x1b[201~".to_vec());
    }

    #[test]
    fn build_paste_input_bytes_keeps_plain_text_when_bracketed_paste_is_disabled() {
        let bytes = build_paste_input_bytes("echo hello", false).unwrap();
        assert_eq!(bytes, b"echo hello".to_vec());
    }

    #[test]
    fn build_paste_input_bytes_ignores_empty_payload() {
        assert!(build_paste_input_bytes("", false).is_none());
    }

    #[test]
    fn build_paste_input_bytes_preserves_whitespace_payload() {
        let bytes = build_paste_input_bytes("   \n", false).unwrap();
        assert_eq!(bytes, b"   \n".to_vec());
    }

    #[test]
    fn vt_state_reports_bracketed_paste_when_requested_by_session() {
        let mut vt = crate::model::VtState::new(24, 80);
        assert!(!screen_requests_bracketed_paste(vt.screen()));

        vt.process(b"\x1b[?2004h");
        assert!(screen_requests_bracketed_paste(vt.screen()));

        vt.process(b"\x1b[?2004l");
        assert!(!screen_requests_bracketed_paste(vt.screen()));
    }

    #[test]
    fn handle_paste_input_queues_bracketed_payload_for_active_session() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model
            .active_session_tab_mut()
            .expect("active session")
            .vt
            .process(b"\x1b[?2004h");

        handle_paste_input(&mut model, "git status\npwd".into());

        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(
            forwarded.bytes,
            b"\x1b[200~git status\npwd\x1b[201~".to_vec()
        );
    }

    #[test]
    fn handle_paste_input_ignores_empty_text() {
        let mut model = test_model();

        handle_paste_input(&mut model, "".into());

        assert!(model.pending_pty_inputs().is_empty());
    }

    #[test]
    fn route_paste_input_ignores_management_paste_when_terminal_is_not_focused() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;

        route_paste_input(&mut model, "git status".into());

        assert!(model.pending_pty_inputs().is_empty());
    }

    #[test]
    fn route_paste_input_ignores_paste_when_wizard_is_open() {
        let mut model = test_model();
        model.wizard = Some(screens::wizard::WizardState::default());

        route_paste_input(&mut model, "git status".into());

        assert!(model.pending_pty_inputs().is_empty());
    }

    #[test]
    fn route_paste_input_initialization_appends_url_input() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Initialization;
        model.initialization = Some(crate::screens::initialization::InitializationState::default());

        route_paste_input(&mut model, "https://example.com/repo.git".into());

        assert_eq!(
            model.initialization.as_ref().unwrap().url_input,
            "https://example.com/repo.git"
        );
    }

    #[test]
    fn route_paste_input_wizard_branch_name_appends_text() {
        let mut model = test_model();
        let mut wizard = screens::wizard::WizardState::default();
        wizard.step = screens::wizard::WizardStep::BranchNameInput;
        model.wizard = Some(wizard);

        route_paste_input(&mut model, "feature/paste".into());

        assert_eq!(model.wizard.as_ref().unwrap().branch_name, "feature/paste");
    }

    #[test]
    fn route_paste_input_branches_search_appends_query() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;
        model.management_tab = ManagementTab::Branches;
        model.branches.search_active = true;

        route_paste_input(&mut model, "feat".into());

        assert_eq!(model.branches.search_query, "feat");
    }

    #[test]
    fn route_paste_input_settings_edit_appends_buffer() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;
        model.management_tab = ManagementTab::Settings;
        model.settings.load_category_fields();
        model.settings.editing = true;
        model.settings.edit_buffer.clear();

        route_paste_input(&mut model, "dark".into());

        assert_eq!(model.settings.edit_buffer, "dark");
    }

    #[test]
    fn update_notify_info_sets_status_notification_and_log() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Info, "core", "Started");

        update(&mut model, Message::Notify(notification));

        assert!(model.current_notification.is_some());
        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].message, "Started");
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn update_notify_warn_persists_across_ticks() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Warn, "git", "Detached HEAD");

        update(&mut model, Message::Notify(notification));
        for _ in 0..60 {
            update(&mut model, Message::Tick);
        }

        assert!(model.current_notification.is_some());
        assert_eq!(
            model.current_notification.as_ref().unwrap().message,
            "Detached HEAD"
        );
    }

    #[test]
    fn update_key_input_esc_dismisses_warn_notification_when_unclaimed() {
        let mut model = test_model();
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_key_input_esc_preserves_warn_notification_during_branch_search() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.search_active = true;
        model.branches.search_query = "detached".into();
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_some());
        assert!(!model.branches.search_active);
        assert!(model.branches.search_query.is_empty());
    }

    #[test]
    fn update_key_input_esc_preserves_warn_notification_during_settings_edit() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Settings;
        model.settings.editing = true;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_some());
        assert!(!model.settings.editing);
    }

    #[test]
    fn update_key_input_esc_preserves_warn_notification_during_issue_search() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.search_active = true;
        model.issues.search_query = "warn".into();
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert!(model.current_notification.is_some());
        assert!(!model.issues.search_active);
        assert!(model.issues.search_query.is_empty());
    }

    #[test]
    fn update_notify_info_auto_dismisses_after_timeout() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Info, "core", "Started");

        update(&mut model, Message::Notify(notification));
        for _ in 0..50 {
            update(&mut model, Message::Tick);
        }

        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_notify_error_routes_to_error_queue_and_log() {
        let mut model = test_model();
        let notification =
            Notification::new(Severity::Error, "pty", "Crashed").with_detail("stack trace");

        update(&mut model, Message::Notify(notification));

        assert_eq!(model.error_queue.len(), 1);
        let queued = model.error_queue.front().unwrap();
        assert_eq!(queued.severity, Severity::Error);
        assert_eq!(queued.source, "pty");
        assert_eq!(queued.message, "Crashed");
        assert_eq!(queued.detail.as_deref(), Some("stack trace"));
        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].source, "pty");
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn update_notify_debug_logs_without_ui_surface() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Debug, "pty", "raw bytes");

        update(&mut model, Message::Notify(notification));

        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].severity, Severity::Debug);
        assert!(model.current_notification.is_none());
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn route_key_to_management_logs_f_cycles_filter_levels() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;

        route_key_to_management(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(
            model.logs.filter_level,
            screens::logs::FilterLevel::ErrorOnly
        );

        route_key_to_management(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::WarnUp);
    }

    #[test]
    fn route_key_to_management_logs_d_toggles_debug_filter() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;

        route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::DebugUp);

        route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::All);
    }

    #[test]
    fn route_key_to_management_logs_esc_closes_detail_view_and_preserves_selection() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;
        model.logs.entries = vec![
            Notification::new(Severity::Info, "core", "first"),
            Notification::new(Severity::Warn, "core", "second"),
        ];
        model.logs.selected = 1;
        model.logs.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!model.logs.detail_view);
        assert_eq!(model.logs.selected, 1);
        assert_eq!(
            model
                .logs
                .selected_entry()
                .map(|entry| entry.message.as_str()),
            Some("second")
        );
    }

    #[test]
    fn route_key_to_management_logs_filter_controls_still_work_after_detail_close_support() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;

        route_key_to_management(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(
            model.logs.filter_level,
            screens::logs::FilterLevel::ErrorOnly
        );

        route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(model.logs.filter_level, screens::logs::FilterLevel::DebugUp);
    }

    #[test]
    fn route_key_to_management_logs_esc_without_warn_returns_terminal_focus() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;
        model.active_focus = FocusPane::TabContent;
        model.logs.entries = vec![
            Notification::new(Severity::Info, "core", "first"),
            Notification::new(Severity::Warn, "core", "second"),
        ];
        model.logs.selected = 1;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(model.management_tab, ManagementTab::Logs);
        assert_eq!(model.logs.selected, 1);
    }

    #[test]
    fn route_key_to_management_logs_esc_with_warn_still_dismisses_warning() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Logs;
        model.active_focus = FocusPane::TabContent;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn route_key_to_management_profiles_esc_without_warn_returns_terminal_focus_in_list_mode() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(model.profiles.mode, screens::profiles::ProfileMode::List);
    }

    #[test]
    fn route_key_to_management_profiles_esc_with_warn_still_dismisses_warning_in_list_mode() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_none());
        assert_eq!(model.profiles.mode, screens::profiles::ProfileMode::List);
    }

    #[test]
    fn route_key_to_management_profiles_esc_in_create_mode_still_cancels_form() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;
        model.profiles.mode = screens::profiles::ProfileMode::CreateProfile;
        model.profiles.input_name = "demo".into();

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert_eq!(model.profiles.mode, screens::profiles::ProfileMode::List);
        assert!(model.profiles.input_name.is_empty());
    }

    #[test]
    fn route_key_to_management_profiles_enter_persists_active_profile_and_updates_status_bar() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev"))
                .unwrap();
            settings.profiles.switch("default").unwrap();
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            switch_management_tab(&mut model, ManagementTab::Profiles);
            assert_eq!(model.profiles.profiles.len(), 2);
            model.profiles.selected = 1;

            route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

            let reloaded = gwt_config::Settings::load().expect("reload settings");
            assert_eq!(reloaded.profiles.active.as_deref(), Some("dev"));

            let rendered = render_model_text(&model, 220, 24);
            assert!(rendered.contains("profile: dev"), "{rendered}");
        });
    }

    #[test]
    fn refresh_active_profile_state_renders_default_fallback_when_active_is_invalid() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev"))
                .unwrap();
            settings.profiles.active = Some("missing".to_string());
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            refresh_active_profile_state(&mut model);

            let rendered = render_model_text(&model, 220, 24);
            assert!(
                rendered.contains("profile: default (fallback)"),
                "{rendered}"
            );
        });
    }

    #[test]
    fn spawn_env_with_active_profile_overrides_and_removes_inherited_keys() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev").with_env("API_URL", "https://example.test"))
                .expect("add profile");
            settings
                .profiles
                .update_disabled_env("dev", "MISSING", "HOME")
                .expect("disable home");
            settings.profiles.switch("dev").expect("switch active");
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut explicit_env = HashMap::new();
            explicit_env.insert("HOME".to_string(), "/tmp/override-home".to_string());
            explicit_env.insert("GWT_KEEP".to_string(), "1".to_string());
            let (env, remove_env) = spawn_env_with_active_profile(explicit_env);

            assert_eq!(
                env.get("API_URL").map(String::as_str),
                Some("https://example.test")
            );
            assert_eq!(
                env.get("HOME").map(String::as_str),
                Some("/tmp/override-home")
            );
            assert!(remove_env.is_empty());

            let (env, remove_env) = spawn_env_with_active_profile(HashMap::new());
            assert_eq!(
                env.get("API_URL").map(String::as_str),
                Some("https://example.test")
            );
            assert!(remove_env.contains(&"HOME".to_string()));
        });
    }

    #[test]
    fn route_key_to_management_profiles_create_profile_persists_metadata() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");
            gwt_config::Settings::default()
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            switch_management_tab(&mut model, ManagementTab::Profiles);

            route_key_to_management(&mut model, key(KeyCode::Char('n'), KeyModifiers::NONE));
            for ch in "dev".chars() {
                route_key_to_management(&mut model, key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            route_key_to_management(&mut model, key(KeyCode::Tab, KeyModifiers::NONE));
            for ch in "Dev profile".chars() {
                route_key_to_management(&mut model, key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

            let reloaded = gwt_config::Settings::load().expect("reload settings");
            let dev = reloaded.profiles.get("dev").expect("dev profile");
            assert_eq!(dev.description, "Dev profile");
        });
    }

    #[test]
    fn route_key_to_management_profiles_tab_cycles_internal_focus() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;

        route_key_to_management(&mut model, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            model.profiles.focus,
            screens::profiles::ProfilesFocus::Environment
        );

        route_key_to_management(&mut model, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            model.profiles.focus,
            screens::profiles::ProfilesFocus::ProfileList
        );

        route_key_to_management(&mut model, key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(
            model.profiles.focus,
            screens::profiles::ProfilesFocus::Environment
        );
    }

    #[test]
    fn route_key_to_management_profiles_ctrl_arrow_is_noop() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Profiles;
        model.active_focus = FocusPane::TabContent;

        route_key_to_management(&mut model, key(KeyCode::Right, KeyModifiers::CONTROL));

        assert_eq!(
            model.profiles.focus,
            screens::profiles::ProfilesFocus::ProfileList
        );
    }

    #[test]
    fn route_key_to_management_profiles_add_env_var_persists_and_renders_preview() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev"))
                .expect("add profile");
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            switch_management_tab(&mut model, ManagementTab::Profiles);
            model.profiles.selected = 1;

            route_key_to_management(&mut model, key(KeyCode::Tab, KeyModifiers::NONE));
            route_key_to_management(&mut model, key(KeyCode::Char('n'), KeyModifiers::NONE));
            for ch in "API_URL".chars() {
                route_key_to_management(&mut model, key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            route_key_to_management(&mut model, key(KeyCode::Tab, KeyModifiers::NONE));
            for ch in "https://example.test".chars() {
                route_key_to_management(&mut model, key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

            let reloaded = gwt_config::Settings::load().expect("reload settings");
            let dev = reloaded.profiles.get("dev").expect("dev profile");
            assert_eq!(
                dev.env_vars.get("API_URL").map(String::as_str),
                Some("https://example.test")
            );

            let rendered = render_model_text(&model, 220, 24);
            assert!(rendered.contains("API_URL"), "{rendered}");
            assert!(rendered.contains("https://example.test"), "{rendered}");
        });
    }

    #[test]
    fn profiles_mouse_click_selects_profile_row_and_focuses_env_pane() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev").with_env("API_URL", "https://example.test"))
                .expect("add profile");
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            update(&mut model, Message::Resize(80, 24));
            switch_management_tab(&mut model, ManagementTab::Profiles);
            model.active_focus = FocusPane::TabContent;

            let management = visible_management_area(&model).expect("management area");
            let outer = pane_block(management_tab_title(&model, management.width), false);
            let inner = outer.inner(management);
            let layout = screens::profiles::layout_areas(inner);

            let handled = handle_mouse_input_with(
                &mut model,
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: layout.list_content.x,
                    row: layout.list_content.y.saturating_add(1),
                    modifiers: KeyModifiers::NONE,
                },
                |_| Ok(()),
            )
            .expect("profile click succeeds");
            assert!(handled);
            assert_eq!(
                model.profiles.focus,
                screens::profiles::ProfilesFocus::ProfileList
            );
            assert_eq!(
                model
                    .profiles
                    .selected_profile()
                    .map(|profile| profile.name.as_str()),
                Some("dev")
            );
            let expected_env_key = model
                .profiles
                .selected_profile()
                .expect("selected profile")
                .env_rows
                .first()
                .expect("first env row")
                .key
                .clone();

            let handled = handle_mouse_input_with(
                &mut model,
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: layout.env_content.x,
                    row: layout.env_content.y,
                    modifiers: KeyModifiers::NONE,
                },
                |_| Ok(()),
            )
            .expect("env click succeeds");
            assert!(handled);
            assert_eq!(
                model.profiles.focus,
                screens::profiles::ProfilesFocus::Environment
            );
            assert_eq!(
                model
                    .profiles
                    .selected_env_row()
                    .map(|env| env.key.as_str()),
                Some(expected_env_key.as_str())
            );
        });
    }

    #[test]
    fn route_key_to_management_profiles_delete_base_row_toggles_disabled_env() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let base_key = "GWT_PROFILE_DELETE_ME";
            let previous = std::env::var_os(base_key);
            std::env::set_var(base_key, "from-os");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev"))
                .expect("add profile");
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            switch_management_tab(&mut model, ManagementTab::Profiles);
            model.profiles.selected = 1;
            model.profiles.focus = screens::profiles::ProfilesFocus::Environment;
            model.profiles.env_selected = model.profiles.profiles[1]
                .env_rows
                .iter()
                .position(|row| row.key == base_key)
                .expect("base env row");

            route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));

            let reloaded = gwt_config::Settings::load().expect("reload settings");
            assert_eq!(
                reloaded.profiles.get("dev").unwrap().disabled_env,
                vec![base_key.to_string()]
            );

            route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));

            let restored = gwt_config::Settings::load().expect("reload settings again");
            assert!(restored
                .profiles
                .get("dev")
                .unwrap()
                .disabled_env
                .is_empty());

            if let Some(previous) = previous {
                std::env::set_var(base_key, previous);
            } else {
                std::env::remove_var(base_key);
            }
        });
    }

    #[test]
    fn route_key_to_management_profiles_delete_active_profile_switches_to_default() {
        with_temp_home(|home| {
            let config_dir = home.join(".gwt");
            std::fs::create_dir_all(&config_dir).expect("create config dir");

            let mut settings = gwt_config::Settings::default();
            settings
                .profiles
                .add(gwt_config::Profile::new("dev"))
                .expect("add profile");
            settings.profiles.switch("dev").expect("switch active");
            settings
                .save(&config_dir.join("config.toml"))
                .expect("save settings");

            let mut model = test_model();
            switch_management_tab(&mut model, ManagementTab::Profiles);
            model.profiles.selected = 1;

            route_key_to_management(&mut model, key(KeyCode::Char('d'), KeyModifiers::NONE));
            route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

            let reloaded = gwt_config::Settings::load().expect("reload settings");
            assert!(reloaded.profiles.get("dev").is_none());
            assert_eq!(reloaded.profiles.active.as_deref(), Some("default"));

            let rendered = render_model_text(&model, 220, 24);
            assert!(rendered.contains("profile: default"), "{rendered}");
        });
    }

    #[test]
    fn update_tick_drains_notification_bus_into_notify_flow() {
        let mut model = test_model();
        let notification = Notification::new(Severity::Info, "bus", "Queued");

        assert!(model.notification_bus_handle().send(notification).is_ok());

        update(&mut model, Message::Tick);

        assert_eq!(model.logs.entries.len(), 1);
        assert_eq!(model.logs.entries[0].message, "Queued");
        assert!(model.current_notification.is_some());
        assert!(model.drain_notifications().is_empty());
    }

    #[test]
    fn update_tick_drains_branch_detail_events_in_small_batches() {
        let mut model = test_model();
        let total_events = 12usize;
        let generation = model.branches.detail_generation.wrapping_add(1);
        model.branches.detail_generation = generation;
        model.branches.branches = (0..total_events)
            .map(|index| screens::branches::BranchItem {
                name: format!("feature/{index}"),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: None,
                upstream: None,
            })
            .collect();

        let events = Arc::new(Mutex::new(VecDeque::new()));
        {
            let mut queue = events.lock().expect("lock branch detail queue");
            for index in 0..total_events {
                queue.push_back(screens::branches::BranchDetailLoadResult {
                    generation,
                    branch_name: format!("feature/{index}"),
                    data: screens::branches::BranchDetailData::default(),
                });
            }
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn(|| {});
        model.branch_detail_worker = Some(crate::model::BranchDetailWorker::new(
            events.clone(),
            cancel,
            handle,
        ));

        update(&mut model, Message::Tick);

        let remaining = events
            .lock()
            .expect("lock branch detail queue after tick")
            .len();
        assert_eq!(
            remaining, 4,
            "tick should leave work queued so branch detail preload cannot monopolize one frame"
        );
        assert_eq!(model.branches.detail_cache.len(), 8);
    }

    #[test]
    fn route_key_to_management_routes_search_input_for_issues() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.search_active = true;

        route_key_to_management(&mut model, key(KeyCode::Char('b'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Char('u'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(model.issues.search_query, "b");
    }

    #[test]
    fn route_key_to_management_issues_esc_closes_detail_view_and_preserves_selection() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.issues = vec![
            screens::issues::IssueItem {
                number: 1,
                title: "First".into(),
                state: "open".into(),
                labels: vec!["ux".into()],
                body: "First body".into(),
                linked_branches: vec![],
            },
            screens::issues::IssueItem {
                number: 2,
                title: "Second".into(),
                state: "open".into(),
                labels: vec!["bug".into()],
                body: "Second body".into(),
                linked_branches: vec![],
            },
        ];
        model.issues.selected = 1;
        model.issues.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!model.issues.detail_view);
        assert_eq!(model.issues.selected, 1);
        assert_eq!(
            model.issues.selected_issue().map(|issue| issue.number),
            Some(2)
        );
    }

    #[test]
    fn route_key_to_management_issues_shift_enter_opens_wizard_with_prefilled_issue() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.active_focus = FocusPane::TabContent;
        model.management_tab = ManagementTab::Issues;
        model.issues.issues = vec![screens::issues::IssueItem {
            number: 1776,
            title: "Launch Agent issue linkage".into(),
            state: "open".into(),
            labels: vec!["ux".into()],
            body: "Wizard should link a selected issue".into(),
            linked_branches: vec![],
        }];
        model.issues.detail_view = true;

        route_key_to_management(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        let wizard = model.wizard.expect("wizard should open from issue detail");
        assert_eq!(wizard.step, screens::wizard::WizardStep::BranchTypeSelect);
        assert_eq!(wizard.issue_id, "1776");
    }

    #[test]
    fn route_key_to_management_issues_refresh_reloads_issue_cache() {
        with_temp_home(|home| {
            let repo_url = "https://github.com/example/repo.git";
            let issue_cache_root = home
                .join(".gwt")
                .join("cache")
                .join("issues")
                .join(gwt_core::repo_hash::compute_repo_hash(repo_url).as_str());
            write_issue_cache_meta(&issue_cache_root, 42, "Fix login bug", "open", &["bug"]);

            let dir = tempfile::tempdir().expect("temp repo");
            init_git_repo(dir.path());
            let add_remote = std::process::Command::new("git")
                .args(["remote", "add", "origin", repo_url])
                .current_dir(dir.path())
                .output()
                .expect("add github remote");
            assert!(add_remote.status.success(), "git remote add failed");

            let mut model = Model::new(dir.path().to_path_buf());
            model.management_tab = ManagementTab::Issues;
            model.issues.last_error = Some("stale".to_string());

            route_key_to_management(&mut model, key(KeyCode::Char('r'), KeyModifiers::NONE));

            assert_eq!(model.issues.issues.len(), 1);
            assert_eq!(model.issues.issues[0].number, 42);
            assert!(model.issues.last_error.is_none());
        });
    }

    #[test]
    fn route_key_to_management_git_view_refresh_reloads_repository_data() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");
        fs::write(dir.path().join("tracked.txt"), "one\n").expect("write tracked file");
        let add = std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .expect("git add tracked file");
        assert!(add.status.success(), "git add failed");
        git_commit_allow_empty(dir.path(), "add tracked file");

        let mut model = Model::new(dir.path().to_path_buf());
        model.management_tab = ManagementTab::GitView;
        load_initial_data(&mut model);
        assert!(model.git_view.files.is_empty());

        fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").expect("modify tracked file");

        route_key_to_management(&mut model, key(KeyCode::Char('r'), KeyModifiers::NONE));

        assert_eq!(model.git_view.files.len(), 1);
        assert_eq!(model.git_view.files[0].path, "tracked.txt");
    }

    #[test]
    fn load_initial_data_populates_issues_from_issue_cache() {
        with_temp_home(|home| {
            let repo_url = "https://github.com/example/repo.git";
            let repo_hash = gwt_core::repo_hash::compute_repo_hash(repo_url);
            let issue_cache_root = home
                .join(".gwt")
                .join("cache")
                .join("issues")
                .join(repo_hash.as_str());
            let other_cache_root = home.join(".gwt").join("cache").join("issues").join(
                gwt_core::repo_hash::compute_repo_hash("https://github.com/example/other.git")
                    .as_str(),
            );
            write_issue_cache_meta(
                &issue_cache_root,
                1776,
                "Launch Agent issue linkage",
                "open",
                &["ux"],
            );
            write_issue_cache_meta(&other_cache_root, 42, "Other repo issue", "open", &["bug"]);

            let dir = tempfile::tempdir().expect("temp repo");
            init_git_repo(dir.path());
            let add_remote = std::process::Command::new("git")
                .args(["remote", "add", "origin", repo_url])
                .current_dir(dir.path())
                .output()
                .expect("add github remote");
            assert!(add_remote.status.success(), "git remote add failed");

            let mut model = Model::new(dir.path().to_path_buf());
            load_initial_data_with(&mut model, |_| Ok(None), |_| Ok(vec![]));

            assert_eq!(model.issues.issues.len(), 1);
            assert_eq!(model.issues.issues[0].number, 1776);
            assert!(model.issues.last_error.is_none());
        });
    }

    #[test]
    fn load_initial_data_syncs_repo_scoped_issue_cache_when_missing() {
        with_temp_home(|_home| {
            let dir = tempfile::tempdir().expect("temp repo");
            init_git_repo(dir.path());
            let add_remote = std::process::Command::new("git")
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/example/repo.git",
                ])
                .current_dir(dir.path())
                .output()
                .expect("add github remote");
            assert!(add_remote.status.success(), "git remote add failed");

            let script = r#"#!/bin/sh
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  printf '[{"number":1776,"title":"Launch Agent issue linkage","body":"Body from startup sync","labels":[{"name":"ux"}],"state":"OPEN","url":"https://github.com/example/repo/issues/1776","updatedAt":"2026-04-13T00:00:00Z"}]'
  exit 0
fi
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 1
"#;

            with_fake_gh(script, || {
                let mut model = Model::new(dir.path().to_path_buf());
                load_initial_data_with(&mut model, |_| Ok(None), |_| Ok(vec![]));

                assert_eq!(model.issues.issues.len(), 1);
                assert_eq!(model.issues.issues[0].number, 1776);
                assert_eq!(model.issues.issues[0].title, "Launch Agent issue linkage");
                assert_eq!(model.issues.issues[0].body, "Body from startup sync");
            });
        });
    }

    #[test]
    fn model_new_loads_specs_from_repo_scoped_cache_only() {
        with_temp_home(|home| {
            let repo_url = "https://github.com/example/specs.git";
            let repo_hash = gwt_core::repo_hash::compute_repo_hash(repo_url);
            let relevant_root = home
                .join(".gwt")
                .join("cache")
                .join("issues")
                .join(repo_hash.as_str());
            let other_root = home.join(".gwt").join("cache").join("issues").join(
                gwt_core::repo_hash::compute_repo_hash(
                    "https://github.com/example/other-specs.git",
                )
                .as_str(),
            );
            write_cached_spec(
                &relevant_root,
                1776,
                "Current repo spec",
                gwt_github::IssueState::Open,
                &["gwt-spec", "phase/draft"],
            );
            write_cached_spec(
                &other_root,
                42,
                "Other repo spec",
                gwt_github::IssueState::Open,
                &["gwt-spec", "phase/draft"],
            );

            let dir = tempfile::tempdir().expect("temp repo");
            init_git_repo(dir.path());
            let add_remote = std::process::Command::new("git")
                .args(["remote", "add", "origin", repo_url])
                .current_dir(dir.path())
                .output()
                .expect("add github remote");
            assert!(add_remote.status.success(), "git remote add failed");

            let model = Model::new(dir.path().to_path_buf());
            assert_eq!(model.specs.items.len(), 1);
            assert_eq!(model.specs.items[0].number, 1776);
            assert_eq!(model.specs.items[0].title, "Current repo spec");
        });
    }

    #[test]
    fn route_key_to_management_branches_refresh_does_not_block_on_detail_reload() {
        let dir = tempfile::tempdir().expect("temp repo");
        init_git_repo(dir.path());
        git_commit_allow_empty(dir.path(), "initial commit");

        let mut model = Model::new(dir.path().to_path_buf());
        model.management_tab = ManagementTab::Branches;
        let project_root = dir.path().to_path_buf();
        model.set_branch_detail_docker_snapshotter(move || {
            thread::sleep(std::time::Duration::from_millis(250));
            vec![docker_service(
                &project_root,
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            )]
        });

        let start = std::time::Instant::now();
        route_key_to_management(&mut model, key(KeyCode::Char('r'), KeyModifiers::NONE));
        let elapsed = start.elapsed();

        // Tight enough to still prove non-blocking behavior (the mock
        // snapshotter sleeps 250ms, so anything under that disproves "waited
        // for it"), loose enough to survive CPU contention when the full test
        // suite runs in parallel — the previous 150ms bound flaked whenever
        // nearby tests were added.
        assert!(
            elapsed < std::time::Duration::from_millis(230),
            "Branches refresh should not block on branch detail reload: {elapsed:?}"
        );
        assert!(
            model.branches.docker_services.is_empty(),
            "detail refresh should update docker data asynchronously"
        );
        assert!(
            model.branch_detail_worker.is_some(),
            "branch detail refresh should run in the background"
        );
    }

    #[test]
    fn update_toggle_help_flips_overlay_visibility() {
        let mut model = test_model();
        assert!(!model.help_visible);

        update(&mut model, Message::ToggleHelp);
        assert!(model.help_visible);

        update(&mut model, Message::ToggleHelp);
        assert!(!model.help_visible);
    }

    #[test]
    fn route_overlay_key_escape_closes_help_overlay() {
        let mut model = test_model();
        model.help_visible = true;

        assert!(route_overlay_key(
            &mut model,
            key(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(!model.help_visible);
    }

    #[test]
    fn route_overlay_key_escape_hides_docker_progress_overlay() {
        let mut model = test_model();
        model.docker_progress = Some(screens::docker_progress::DockerProgressState {
            visible: true,
            stage: screens::docker_progress::DockerStage::StartingContainer,
            message: "Starting container web".into(),
            error: None,
        });

        assert!(route_overlay_key(
            &mut model,
            key(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(model.docker_progress.is_none());
    }

    #[test]
    fn render_help_overlay_lists_all_registered_keybindings_only() {
        let mut model = test_model();
        model.help_visible = true;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("create terminal");
        terminal
            .draw(|frame| view(&model, frame))
            .expect("render help overlay");

        let text = buffer_text(terminal.backend().buffer());
        let registry = crate::input::keybind::KeybindRegistry::new();

        for binding in registry.all_bindings() {
            assert!(
                text.contains(&binding.keys),
                "expected help overlay to contain binding {}",
                binding.keys
            );
            assert!(
                text.contains(&binding.description),
                "expected help overlay to contain description {}",
                binding.description
            );
        }
    }

    #[test]
    fn update_toggle_layer_blocked_in_initialization() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        assert_eq!(model.active_layer, ActiveLayer::Initialization);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Initialization); // stays
    }

    #[test]
    fn update_initialization_exit_quits() {
        use crate::screens::initialization::InitializationMessage;

        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        update(
            &mut model,
            Message::Initialization(InitializationMessage::Exit),
        );
        assert!(model.quit);
    }

    #[test]
    fn route_key_to_initialization_esc_exits() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        route_key_to_initialization(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(model.quit);
    }

    #[test]
    fn route_key_to_initialization_char_input() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        route_key_to_initialization(&mut model, key(KeyCode::Char('h'), KeyModifiers::NONE));
        let init = model.initialization.as_ref().unwrap();
        assert_eq!(init.url_input, "h");
    }

    #[test]
    fn route_key_to_initialization_backspace() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        route_key_to_initialization(&mut model, key(KeyCode::Char('a'), KeyModifiers::NONE));
        route_key_to_initialization(&mut model, key(KeyCode::Char('b'), KeyModifiers::NONE));
        route_key_to_initialization(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));
        let init = model.initialization.as_ref().unwrap();
        assert_eq!(init.url_input, "a");
    }

    #[test]
    fn update_key_input_routes_to_service_select_overlay() {
        let mut model = test_model();
        model.service_select = Some(screens::service_select::ServiceSelectState {
            title: "Select Agent".into(),
            services: vec!["claude".into(), "codex".into()],
            values: vec!["claude".into(), "codex".into()],
            selected: 0,
            visible: true,
        });

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Down, KeyModifiers::NONE)),
        );

        assert_eq!(model.service_select.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn route_key_to_branch_detail_overview_moves_docker_selection() {
        let mut model = test_model();
        model.branches.detail_section = 0;
        model.branches.docker_services = vec![
            docker_service(
                std::path::Path::new("/tmp/test"),
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            ),
            docker_service(
                std::path::Path::new("/tmp/test"),
                "db",
                gwt_docker::ComposeServiceStatus::Stopped,
            ),
        ];

        route_key_to_branch_detail(&mut model, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(model.branches.docker_selected, 1);

        route_key_to_branch_detail(&mut model, key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(model.branches.docker_selected, 0);
    }

    #[test]
    fn route_key_to_management_branches_down_does_not_block_on_detail_reload() {
        let wt_a = tempfile::tempdir().expect("worktree a");
        let wt_b = tempfile::tempdir().expect("worktree b");
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        let snapshot_root = wt_b.path().to_path_buf();
        model.set_branch_detail_docker_snapshotter(move || {
            thread::sleep(std::time::Duration::from_millis(250));
            vec![docker_service(
                &snapshot_root,
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            )]
        });
        model.branches.branches = vec![
            screens::branches::BranchItem {
                name: "feature/a".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: Some(wt_a.path().to_path_buf()),
                upstream: None,
            },
            screens::branches::BranchItem {
                name: "feature/b".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: Some(wt_b.path().to_path_buf()),
                upstream: None,
            },
        ];

        let start = std::time::Instant::now();
        route_key_to_management(&mut model, key(KeyCode::Down, KeyModifiers::NONE));
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(150),
            "Branches cursor movement should not block on detail reload: {elapsed:?}"
        );
        assert_eq!(model.branches.selected, 1);
    }

    #[test]
    fn spawn_branch_detail_worker_with_loader_stops_after_cancel() {
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let branches = vec![
            screens::branches::BranchItem {
                name: "feature/a".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: None,
                upstream: None,
            },
            screens::branches::BranchItem {
                name: "feature/b".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: None,
                upstream: None,
            },
        ];

        let handle = spawn_branch_detail_worker_with_loader(
            events,
            cancel.clone(),
            7,
            branches,
            Vec::new(),
            move |branch, _docker_containers| {
                started_tx
                    .send(branch.name.clone())
                    .expect("signal branch load start");
                if branch.name == "feature/a" {
                    release_rx.recv().expect("release first branch load");
                }
                screens::branches::BranchDetailData::default()
            },
        );

        let first_branch = started_rx
            .recv_timeout(std::time::Duration::from_millis(200))
            .expect("first branch should start loading");
        assert_eq!(first_branch, "feature/a");

        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        release_tx.send(()).expect("release canceled worker");
        handle.join().expect("join worker");

        assert!(
            started_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "canceled worker should not continue into later branches"
        );
    }

    #[test]
    fn route_key_to_branch_detail_sessions_moves_selection() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-test")),
            upstream: None,
        }];
        model.branches.detail_section = 2;

        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: "shell-1".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];

        route_key_to_branch_detail(&mut model, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(model.branches.detail_session_selected, 1);

        route_key_to_branch_detail(&mut model, key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(model.branches.detail_session_selected, 0);
    }

    #[test]
    fn route_key_to_branch_detail_sessions_enter_focuses_selected_session() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-test")),
            upstream: None,
        }];
        model.branches.detail_section = 2;
        model.active_focus = FocusPane::BranchDetail;

        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: "shell-1".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];
        model.branches.detail_session_selected = 1;

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(model.active_session, 1);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn route_key_to_branch_detail_sessions_enter_clamps_stale_selection() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/test".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-test")),
            upstream: None,
        }];
        model.branches.detail_section = 2;
        model.active_focus = FocusPane::BranchDetail;

        model.sessions = vec![
            crate::model::SessionTab {
                id: "shell-0".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
            crate::model::SessionTab {
                id: "shell-1".to_string(),
                name: "Shell: feature/test".to_string(),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
                created_at: std::time::Instant::now(),
            },
        ];
        model.branches.detail_session_selected = 99;

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(model.branches.detail_session_selected, 1);
        assert_eq!(model.active_session, 1);
        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn route_key_to_branch_detail_shift_enter_opens_shell_for_selected_branch() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/direct-actions".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-direct-actions")),
            upstream: None,
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;
        let initial_sessions = model.sessions.len();

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        assert_eq!(model.sessions.len(), initial_sessions + 1);
        assert_eq!(
            model.active_session, initial_sessions,
            "new shell session should become active"
        );
        assert_eq!(model.active_focus, FocusPane::Terminal);
        assert_eq!(
            model.sessions.last().map(|session| session.name.as_str()),
            Some("Shell: feature/direct-actions")
        );
    }

    #[test]
    fn route_key_to_branch_detail_ctrl_c_does_not_open_delete_confirm() {
        // FR-018: the legacy single-branch Ctrl+C delete-worktree shortcut
        // has been removed. Ctrl+C on the Branch Detail pane must no longer
        // open any confirmation modal — deletions go through the multi-select
        // Cleanup flow instead.
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/direct-actions".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-direct-actions")),
            upstream: None,
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(!model.confirm.visible);
    }

    #[test]
    fn route_key_to_branch_detail_shift_enter_ignores_branches_without_worktree() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/no-worktree".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
            upstream: None,
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;
        let initial_sessions = model.sessions.len();

        route_key_to_branch_detail(&mut model, key(KeyCode::Enter, KeyModifiers::SHIFT));

        assert_eq!(model.sessions.len(), initial_sessions);
        assert_eq!(model.active_focus, FocusPane::BranchDetail);
        assert!(!model.branches.pending_open_shell);
    }

    #[test]
    fn route_key_to_branch_detail_ctrl_c_is_noop_for_branches_without_worktree() {
        // FR-018: see `route_key_to_branch_detail_ctrl_c_does_not_open_delete_confirm`.
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/no-worktree".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
            upstream: None,
        }];
        model.branches.detail_section = 0;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(!model.confirm.visible);
    }

    #[test]
    fn route_key_to_branch_detail_esc_returns_to_tab_content_focus() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
            upstream: None,
        }];
        model.branches.selected = 0;
        model.branches.detail_section = 2;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn route_key_to_branch_detail_esc_preserves_detail_context() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
            upstream: None,
        }];
        model.branches.selected = 0;
        model.branches.detail_section = 2;
        model.branches.detail_session_selected = 4;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.branches.selected, 0);
        assert_eq!(model.branches.detail_section, 2);
        assert_eq!(model.branches.detail_session_selected, 4);
        assert_eq!(
            model
                .branches
                .selected_branch()
                .map(|branch| branch.name.as_str()),
            Some("feature/esc-back")
        );
    }

    #[test]
    fn route_key_to_branch_detail_esc_with_warn_preserves_notification_and_returns_to_list() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
            upstream: None,
        }];
        model.branches.selected = 0;
        model.branches.detail_section = 2;
        model.branches.detail_session_selected = 4;
        model.active_focus = FocusPane::BranchDetail;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_some());
        assert_eq!(model.branches.selected, 0);
        assert_eq!(model.branches.detail_section, 2);
        assert_eq!(model.branches.detail_session_selected, 4);
    }

    #[test]
    fn route_key_to_branch_detail_esc_with_warn_allows_second_escape_to_dismiss_from_list() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/esc-back".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-esc-back")),
            upstream: None,
        }];
        model.active_focus = FocusPane::BranchDetail;
        update(
            &mut model,
            Message::Notify(Notification::new(Severity::Warn, "git", "Detached HEAD")),
        );

        route_key_to_branch_detail(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn route_key_to_branch_detail_m_toggles_view_mode() {
        let mut model = test_model();
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/view-mode".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-view-mode")),
            upstream: None,
        }];
        model.active_focus = FocusPane::BranchDetail;
        model.branches.detail_section = 0;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('m'), KeyModifiers::NONE));

        assert_eq!(
            model.branches.view_mode,
            screens::branches::ViewMode::Remote
        );
        assert_eq!(model.active_focus, FocusPane::BranchDetail);
    }

    #[test]
    fn route_key_to_branch_detail_v_switches_to_git_view() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.detail_section = 2;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('v'), KeyModifiers::NONE));

        assert_eq!(model.management_tab, ManagementTab::GitView);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn route_key_to_branch_detail_f_starts_search_and_returns_to_list_focus() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.detail_section = 1;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('f'), KeyModifiers::NONE));

        assert!(model.branches.search_active);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn route_key_to_branch_detail_h_toggles_help_overlay() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;

        route_key_to_branch_detail(&mut model, key(KeyCode::Char('h'), KeyModifiers::NONE));

        assert!(model.help_visible);
    }

    #[test]
    fn render_model_text_branch_detail_hints_are_section_sensitive() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/direct-actions".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-direct-actions")),
            upstream: None,
        }];
        model.branches.detail_section = 0;
        model.branches.docker_services = vec![docker_service(
            std::path::Path::new("/tmp/test/wt-feature-direct-actions"),
            "web",
            gwt_docker::ComposeServiceStatus::Running,
        )];

        let overview = render_model_text(&model, 200, 24);
        assert!(overview.contains("Shift+Enter:shell"));
        assert!(
            !overview.contains("Ctrl+C:delete"),
            "FR-018: single-branch Ctrl+C delete-worktree hint must be gone"
        );
        assert!(overview.contains("T:stop"));

        model.branches.branches[0].worktree_path = None;
        let no_worktree = render_model_text(&model, 200, 24);
        assert!(!no_worktree.contains("Shift+Enter:shell"));
        assert!(!no_worktree.contains("Ctrl+C:delete"));
        assert!(no_worktree.contains("Enter:launch"));

        model.branches.detail_section = 2;
        let sessions = render_model_text(&model, 200, 24);
        assert!(sessions.contains("↑↓:session"));
        assert!(sessions.contains("Enter:focus"));
    }

    #[test]
    fn render_model_text_branch_detail_hints_include_branch_local_mnemonics() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/mnemonics".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-mnemonics")),
            upstream: None,
        }];

        let rendered = render_model_text(&model, 220, 24);
        assert!(rendered.contains("Space:select"));
        assert!(rendered.contains("m:view"));
        assert!(rendered.contains("v:git"));
        assert!(rendered.contains("f:search"));
        assert!(rendered.contains("?:help"));
    }

    #[test]
    fn route_key_to_management_branches_h_toggles_help_overlay() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        route_key_to_management(&mut model, key(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(model.help_visible);
    }

    #[test]
    fn route_key_to_management_branches_space_on_unmerged_branch_selects_without_toast() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/not-merged".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
            upstream: None,
        }];
        model.branches.set_merge_state(
            "feature/not-merged",
            screens::branches::MergeState::NotMerged,
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char(' '), KeyModifiers::NONE)),
        );

        assert_eq!(model.active_focus, FocusPane::TabContent);
        assert!(model.branches.is_cleanup_selected("feature/not-merged"));
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn route_key_to_branch_detail_space_on_unmerged_branch_selects_without_toast() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::BranchDetail;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/not-merged".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/test/wt-feature-not-merged")),
            upstream: None,
        }];
        model.branches.set_merge_state(
            "feature/not-merged",
            screens::branches::MergeState::NotMerged,
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char(' '), KeyModifiers::NONE)),
        );

        assert_eq!(model.active_focus, FocusPane::BranchDetail);
        assert!(model.branches.is_cleanup_selected("feature/not-merged"));
        assert!(model.current_notification.is_none());
    }

    #[test]
    fn render_model_text_branches_blocked_cleanup_toast_is_visible_at_standard_width() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/computing".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
            upstream: None,
        }];

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char(' '), KeyModifiers::NONE)),
        );

        let rendered = render_model_text(&model, 80, 24);
        assert!(
            rendered.contains("Cannot select: merge check running"),
            "{rendered}"
        );
    }

    #[test]
    fn route_key_input_shift_c_on_unmerged_selection_opens_confirm_with_warning() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/not-merged".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
            upstream: None,
        }];
        model.branches.set_merge_state(
            "feature/not-merged",
            screens::branches::MergeState::NotMerged,
        );

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char(' '), KeyModifiers::NONE)),
        );
        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char('C'), KeyModifiers::SHIFT)),
        );

        assert!(model.cleanup_confirm.visible);
        assert_eq!(model.cleanup_confirm.rows.len(), 1);
        assert_eq!(
            model.cleanup_confirm.rows[0].risks,
            vec![screens::branches::CleanupSelectionRisk::Unmerged]
        );
    }

    #[test]
    fn update_key_input_cleanup_confirm_r_then_enter_deletes_remote_branch() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_create_branch(&repo_path, "feature/cleanup-remote");
        git_push_branch(&repo_path, "feature/cleanup-remote");

        let mut model = Model::new(repo_path.clone());
        model.cleanup_confirm.show(
            vec![screens::cleanup_confirm::CleanupConfirmRow {
                branch: "feature/cleanup-remote".to_string(),
                target: Some(gwt_git::MergeTarget::Main),
                execution_branch: "feature/cleanup-remote".to_string(),
                upstream: Some("origin/feature/cleanup-remote".to_string()),
                risks: Vec::new(),
            }],
            false,
        );
        model.branches.branches = vec![screens::branches::BranchItem {
            name: "feature/cleanup-remote".to_string(),
            is_head: false,
            is_local: true,
            category: screens::branches::BranchCategory::Feature,
            worktree_path: None,
            upstream: Some("origin/feature/cleanup-remote".to_string()),
        }];
        model.branches.current_head_branch = Some("master".to_string());

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char('r'), KeyModifiers::NONE)),
        );
        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Enter, KeyModifiers::NONE)),
        );

        drive_ticks_until(
            &mut model,
            |model| {
                model
                    .cleanup_progress
                    .run
                    .as_ref()
                    .is_some_and(|run| run.phase == screens::branches::CleanupRunPhase::Done)
            },
            "cleanup progress completion",
        );

        let remote_output = std::process::Command::new("git")
            .args([
                "ls-remote",
                "--exit-code",
                "--heads",
                "origin",
                "feature/cleanup-remote",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("read remote branch");
        assert!(
            !remote_output.status.success(),
            "remote branch should be deleted: {}",
            String::from_utf8_lossy(&remote_output.stderr)
        );
    }

    #[test]
    fn update_key_input_cleanup_confirm_enter_on_remote_tracking_row_deletes_local_only() {
        let workspace_dir = tempfile::tempdir().expect("temp workspace dir");
        let repo_path = workspace_dir.path().join("gwt");
        let remote_path = workspace_dir.path().join("origin.git");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");
        init_git_repo(&repo_path);
        init_bare_git_repo(&remote_path);
        git_add_remote(&repo_path, "origin", &remote_path);
        git_commit_allow_empty(&repo_path, "initial commit");
        git_create_branch(&repo_path, "feature/cleanup-remote");
        git_push_branch(&repo_path, "feature/cleanup-remote");

        let mut model = Model::new(repo_path.clone());
        model.cleanup_confirm.show(
            vec![screens::cleanup_confirm::CleanupConfirmRow {
                branch: "origin/feature/cleanup-remote".to_string(),
                target: Some(gwt_git::MergeTarget::Main),
                execution_branch: "feature/cleanup-remote".to_string(),
                upstream: Some("origin/feature/cleanup-remote".to_string()),
                risks: vec![screens::branches::CleanupSelectionRisk::RemoteTracking],
            }],
            false,
        );
        model.branches.branches = vec![
            screens::branches::BranchItem {
                name: "feature/cleanup-remote".to_string(),
                is_head: false,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: None,
                upstream: Some("origin/feature/cleanup-remote".to_string()),
            },
            screens::branches::BranchItem {
                name: "origin/feature/cleanup-remote".to_string(),
                is_head: false,
                is_local: false,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: None,
                upstream: None,
            },
        ];
        model.branches.current_head_branch = Some("master".to_string());

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Enter, KeyModifiers::NONE)),
        );

        drive_ticks_until(
            &mut model,
            |model| {
                model
                    .cleanup_progress
                    .run
                    .as_ref()
                    .is_some_and(|run| run.phase == screens::branches::CleanupRunPhase::Done)
            },
            "cleanup progress completion",
        );

        let local_output = std::process::Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/feature/cleanup-remote",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("read local branch");
        assert!(
            !local_output.status.success(),
            "local branch should be deleted"
        );

        let remote_output = std::process::Command::new("git")
            .args([
                "ls-remote",
                "--exit-code",
                "--heads",
                "origin",
                "feature/cleanup-remote",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("read remote branch");
        assert!(
            remote_output.status.success(),
            "remote branch should remain when delete_remote is off: {}",
            String::from_utf8_lossy(&remote_output.stderr)
        );
    }

    #[test]
    fn update_branches_docker_stop_executes_and_refreshes_detail() {
        let tmp = tempfile::tempdir().expect("temp worktree");
        fs::write(
            tmp.path().join("docker-compose.yml"),
            "services:\n  web:\n    image: nginx:latest\n",
        )
        .expect("compose");

        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"stop\" ] && [ \"$5\" = \"web\" ]; then\n  sleep 0.1\n  exit 0\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\texited\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, || {
            let mut model = test_model();
            model.branches.branches = vec![screens::branches::BranchItem {
                name: "feature/docker".into(),
                is_head: true,
                is_local: true,
                category: screens::branches::BranchCategory::Feature,
                worktree_path: Some(tmp.path().to_path_buf()),
                upstream: None,
            }];
            model.branches.docker_services = vec![docker_service(
                tmp.path(),
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            )];

            update(
                &mut model,
                Message::Branches(screens::branches::BranchesMessage::DockerServiceStop),
            );

            assert!(model.branches.pending_docker_action.is_none());
            assert!(model.docker_progress_events.is_some());
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::StartingContainer
            );
            assert_eq!(docker_progress.message, "Stopping service web");
            assert_eq!(
                model.branches.docker_services[0].status,
                gwt_docker::ComposeServiceStatus::Running
            );

            drive_docker_worker_until(
                &mut model,
                |model| {
                    model.docker_progress_events.is_none() && model.current_notification.is_some()
                },
                "docker stop completion",
            );

            drive_ticks_until(
                &mut model,
                |model| {
                    model
                        .branches
                        .docker_services
                        .first()
                        .is_some_and(|service| {
                            service.status == gwt_docker::ComposeServiceStatus::Exited
                        })
                },
                "branch detail refresh after docker stop",
            );

            assert_eq!(model.branches.docker_services.len(), 1);
            assert_eq!(
                model.branches.docker_services[0].status,
                gwt_docker::ComposeServiceStatus::Exited
            );
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::Ready
            );
            assert_eq!(docker_progress.message, "Stopped service web");
            assert!(docker_progress.error.is_none());
            let notification = model
                .current_notification
                .as_ref()
                .expect("status notification");
            assert_eq!(notification.source, "docker");
            assert_eq!(notification.message, "Stopped service web");
            assert!(model.error_queue.is_empty());
        });
    }

    #[test]
    fn update_branches_docker_restart_failure_routes_error_notification() {
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"restart\" ] && [ \"$5\" = \"web\" ]; then\n  sleep 0.1\n  printf 'permission denied' >&2\n  exit 1\nfi\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'web\trunning\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, || {
            let mut model = test_model();
            model.branches.docker_services = vec![docker_service(
                std::path::Path::new("/tmp/test"),
                "web",
                gwt_docker::ComposeServiceStatus::Running,
            )];

            update(
                &mut model,
                Message::Branches(screens::branches::BranchesMessage::DockerServiceRestart),
            );

            assert!(model.docker_progress_events.is_some());
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::StartingContainer
            );
            assert_eq!(docker_progress.message, "Restarting service web");

            drive_docker_worker_until(
                &mut model,
                |model| model.docker_progress_events.is_none() && !model.error_queue.is_empty(),
                "docker restart failure",
            );

            assert!(model.current_notification.is_none());
            assert_eq!(model.error_queue.len(), 1);
            let docker_progress = model.docker_progress.as_ref().expect("docker progress");
            assert!(docker_progress.visible);
            assert_eq!(
                docker_progress.stage,
                screens::docker_progress::DockerStage::Failed
            );
            assert!(docker_progress
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("Failed to restart service web"));
            let notification = model.error_queue.front().unwrap();
            assert_eq!(notification.source, "docker");
            assert_eq!(notification.message, "Failed to restart service web");
            assert!(notification
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("permission denied"));
        });
    }

    #[test]
    fn update_docker_progress_set_stage_creates_overlay_when_missing() {
        let mut model = test_model();
        assert!(model.docker_progress.is_none());

        update(
            &mut model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::SetStage {
                stage: screens::docker_progress::DockerStage::BuildingImage,
                message: "Building image".into(),
            }),
        );

        let state = model.docker_progress.as_ref().expect("docker progress");
        assert!(state.visible);
        assert_eq!(
            state.stage,
            screens::docker_progress::DockerStage::BuildingImage
        );
        assert_eq!(state.message, "Building image");
    }

    #[test]
    fn update_docker_progress_hide_drops_overlay() {
        let mut model = test_model();
        model.docker_progress = Some(screens::docker_progress::DockerProgressState {
            visible: true,
            stage: screens::docker_progress::DockerStage::WaitingForServices,
            message: "Waiting".into(),
            error: None,
        });

        update(
            &mut model,
            Message::DockerProgress(screens::docker_progress::DockerProgressMessage::Hide),
        );

        assert!(model.docker_progress.is_none());
    }

    #[test]
    fn update_key_input_routes_to_confirm_overlay() {
        let mut model = test_model();
        model.confirm = screens::confirm::ConfirmState::with_message("Convert?");

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Tab, KeyModifiers::NONE)),
        );

        assert!(model.confirm.accepted());
    }

    #[test]
    fn update_focus_next_on_non_branches_management_skips_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        update(&mut model, Message::FocusNext);

        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_focus_prev_on_non_branches_management_skips_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Logs;
        model.active_focus = FocusPane::Terminal;

        update(&mut model, Message::FocusPrev);

        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn update_focus_next_from_main_reveals_management_and_targets_next_pane() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::Terminal;

        update(&mut model, Message::FocusNext);

        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn update_focus_prev_from_main_on_branches_targets_branch_detail() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::Terminal;

        update(&mut model, Message::FocusPrev);

        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.active_focus, FocusPane::BranchDetail);
    }

    #[test]
    fn update_focus_next_on_branches_still_cycles_into_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Branches;
        model.active_focus = FocusPane::TabContent;

        update(&mut model, Message::FocusNext);

        assert_eq!(model.active_focus, FocusPane::BranchDetail);
    }

    #[test]
    fn update_focus_next_on_non_branches_management_normalizes_stale_branch_detail_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::BranchDetail;

        update(&mut model, Message::FocusNext);

        assert_eq!(model.active_focus, FocusPane::Terminal);
    }

    #[test]
    fn update_key_input_tab_no_longer_cycles_focus() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Issues;
        model.active_focus = FocusPane::TabContent;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Tab, KeyModifiers::NONE)),
        );

        assert_eq!(model.active_focus, FocusPane::TabContent);
    }

    #[test]
    fn update_key_input_terminal_tab_still_forwards_to_pty() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;
        model.active_focus = FocusPane::Terminal;

        forward_key_to_active_session(&mut model, key(KeyCode::Tab, KeyModifiers::NONE));

        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(forwarded.bytes, b"\t".to_vec());
    }

    #[test]
    fn forward_key_to_active_session_appends_opt_in_trace_record() {
        let _guard = INPUT_TRACE_ENV_TEST_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("input-trace.jsonl");
        let previous = std::env::var_os(crate::input_trace::INPUT_TRACE_PATH_ENV);
        std::env::set_var(crate::input_trace::INPUT_TRACE_PATH_ENV, &path);

        let mut model = test_model();
        forward_key_to_active_session(&mut model, key(KeyCode::Enter, KeyModifiers::NONE));

        match previous {
            Some(value) => std::env::set_var(crate::input_trace::INPUT_TRACE_PATH_ENV, value),
            None => std::env::remove_var(crate::input_trace::INPUT_TRACE_PATH_ENV),
        }

        let text = std::fs::read_to_string(&path).expect("read trace file");
        assert!(text.contains("\"stage\":\"pty_forward\""));
        assert!(text.contains("\"session_id\":\"shell-0\""));
        assert!(text.contains("\"bytes_hex\":\"0d\""));
    }

    #[test]
    fn switch_management_tab_settings_loads_fields() {
        let mut model = test_model();
        assert!(model.settings.fields.is_empty());

        update(
            &mut model,
            Message::SwitchManagementTab(ManagementTab::Settings),
        );

        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert!(!model.settings.fields.is_empty());
    }

    #[test]
    fn update_settings_skills_shows_bundled_count() {
        let mut model = test_model();
        model.settings.category = screens::settings::SettingsCategory::Skills;
        model.settings.load_category_fields();
        assert_eq!(model.settings.fields.len(), 1);
        assert_eq!(model.settings.fields[0].label, "Bundled skills");
        let count: usize = model.settings.fields[0].value.parse().unwrap_or(0);
        assert!(count > 0, "should have bundled skills");
    }

    #[test]
    fn workspace_initialization_warning_is_warn_notification() {
        let notification = workspace_initialization_warning("runtime setup failed");
        assert_eq!(notification.severity, Severity::Warn);
        assert_eq!(notification.source, "workspace");
        assert_eq!(notification.message, "Workspace initialization incomplete");
        assert_eq!(notification.detail.as_deref(), Some("runtime setup failed"));
    }

    #[test]
    fn workspace_initialization_warning_uses_project_index_guidance_when_python_is_missing() {
        let notification = workspace_initialization_warning("[gwt-project-index-python-install] Project index runtime requires Python 3.9+ on PATH. Install Python and ensure `python` or `py -3` works before reopening gwt.");
        assert_eq!(notification.severity, Severity::Warn);
        assert_eq!(notification.source, "index");
        assert_eq!(notification.message, "Project index runtime unavailable");
        assert!(notification
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("py -3"));
    }

    #[test]
    fn workspace_initialization_warning_uses_project_index_source_for_runtime_failures() {
        let notification =
            workspace_initialization_warning("[gwt-project-index-runtime] pip install -r failed");
        assert_eq!(notification.severity, Severity::Warn);
        assert_eq!(notification.source, "index");
        assert_eq!(notification.message, "Project index runtime unavailable");
        assert_eq!(
            notification.detail.as_deref(),
            Some("pip install -r failed")
        );
    }
}
