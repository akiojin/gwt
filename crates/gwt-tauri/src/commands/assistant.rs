#![allow(dead_code)]
//! Assistant Mode Tauri commands

use std::path::{Path, PathBuf};

use gwt_core::{
    git::{self, get_spec_issue_detail, graphql, Branch, PrCache, WorkflowRunInfo},
    logging::{log_flow_failure, log_flow_start, log_flow_success},
    process::command as process_command,
    terminal::pane::PaneStatus,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use tracing::{instrument, warn};

use crate::{
    assistant_engine::{AssistantEngine, AssistantStartupStatus},
    assistant_monitor::{self, MonitorEvent, MonitorSnapshot, PaneSnapshot},
    commands::sessions::get_branch_session_summary_for_assistant,
    state::{AppState, AssistantActiveRunKind, AssistantContext, AssistantRuntimeState},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub role: String,
    pub kind: String,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStateResponse {
    pub messages: Vec<AssistantMessage>,
    pub ai_ready: bool,
    pub is_thinking: bool,
    pub session_id: Option<String>,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
    pub startup_status: String,
    pub startup_summary_ready: bool,
    pub startup_failure_kind: Option<String>,
    pub startup_failure_detail: Option<String>,
    pub startup_recovery_hints: Vec<String>,
    pub working_goal: Option<String>,
    pub goal_confidence: Option<String>,
    pub current_status: Option<String>,
    pub blockers: Vec<String>,
    pub recommended_next_actions: Vec<String>,
    pub queued_message_count: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AssistantDeliveryMode {
    Interrupt,
    Queue,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneDashboard {
    pub pane_id: String,
    pub agent_name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDashboard {
    pub branch: String,
    pub uncommitted_count: u32,
    pub unpushed_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecProgressSummary {
    pub issue_number: u64,
    pub title: String,
    pub phase: String,
    pub tasks_total: u32,
    pub tasks_completed: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CiStatusSummary {
    pub pr_number: u64,
    pub pr_title: String,
    pub check_status: String,
    pub failing_checks: Vec<String>,
    pub review_status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardResponse {
    pub panes: Vec<PaneDashboard>,
    pub git: GitDashboard,
    pub spec_progress: Option<SpecProgressSummary>,
    pub ci_status: Option<CiStatusSummary>,
    pub consultation_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StartupAnalysisFingerprint {
    branch: String,
    head_revision: String,
    worktree_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StartupAnalysisCacheEntry {
    fingerprint: StartupAnalysisFingerprint,
    summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoalResolution {
    goal: Option<String>,
    confidence: Option<String>,
}

#[derive(Debug, Clone)]
struct PrCiInsight {
    pr_number: u64,
    pr_title: String,
    pr_url: String,
    merge_state_status: Option<String>,
    failing_required_checks: Vec<String>,
    pending_required_checks: Vec<String>,
    changes_requested_by: Vec<String>,
}

#[derive(Debug, Clone)]
struct TerminalSummaryInsight {
    short_summary: String,
    highlights: Vec<String>,
    source_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StartupRecoveryInfo {
    kind: Option<String>,
    detail: Option<String>,
    hints: Vec<String>,
}
#[instrument(skip_all, fields(command = "assistant_get_state"))]
#[tauri::command]
pub async fn assistant_get_state(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<AssistantStateResponse, String> {
    let window_label = window.label().to_string();
    let engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    if let Some(engine) = engine_guard.get(&window_label) {
        let context = current_assistant_context(&state, &window_label);
        let runtime = state.assistant_runtime_snapshot(&window_label);
        let startup_status_message = current_startup_status_message(&state, &window_label);
        return Ok(build_assistant_state_response(
            engine,
            Some(window_label),
            &context,
            &runtime,
            startup_status_message.as_deref(),
        ));
    }

    drop(engine_guard);

    let startup_guard = state
        .assistant_startup_inflight
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(status) = startup_guard.get(&window_label) {
        return Ok(build_startup_inflight_state_response(
            window_label,
            status.clone(),
        ));
    }

    Ok(build_empty_assistant_state_response())
}

#[instrument(skip_all, fields(command = "assistant_send_message"))]
#[tauri::command]
pub async fn assistant_send_message(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    input: String,
    delivery_mode: Option<AssistantDeliveryMode>,
) -> Result<AssistantStateResponse, String> {
    log_flow_start("assistant", "assistant_send_message");
    let window_label = window.label().to_string();
    let input = input.trim().to_string();
    if input.is_empty() {
        log_flow_failure(
            "assistant",
            "assistant_send_message",
            "Empty message rejected",
        );
        return Err("Message cannot be empty".to_string());
    }
    let delivery_mode = delivery_mode.unwrap_or(AssistantDeliveryMode::Interrupt);
    let project_path = state.project_for_window(&window_label).ok_or_else(|| {
        gwt_core::logging::log_incident(
            "assistant",
            "assistant_send_message",
            Some("ASSISTANT_NO_PROJECT"),
            "No project opened",
        );
        "No project opened. Open a project first.".to_string()
    })?;

    let runtime_before = state.assistant_runtime_snapshot(&window_label);
    if runtime_before.active_kind.is_some() && delivery_mode == AssistantDeliveryMode::Queue {
        state.enqueue_assistant_input(&window_label, input);
        let engine = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .get(&window_label)
            .cloned()
            .unwrap_or_else(|| {
                AssistantEngine::new(PathBuf::from(&project_path), window_label.clone())
            });
        let context = current_assistant_context(&state, &window_label);
        let runtime = state.assistant_runtime_snapshot(&window_label);
        let startup_status_message = current_startup_status_message(&state, &window_label);
        log_flow_success("assistant", "assistant_send_message");
        return Ok(build_assistant_state_response(
            &engine,
            Some(window_label),
            &context,
            &runtime,
            startup_status_message.as_deref(),
        ));
    }

    clear_startup_status(&state, &window_label);
    let base_engine = ensure_assistant_engine_snapshot(&state, &window_label, &project_path)?;
    let generation = state.begin_assistant_session_for_window(&window_label);
    state.set_assistant_active_run(&window_label, generation, AssistantActiveRunKind::User);

    let app_handle = window.app_handle().clone();
    spawn_assistant_user_run(
        &app_handle,
        &window_label,
        &project_path,
        generation,
        input.clone(),
        base_engine.clone(),
        false,
    );

    let context = current_assistant_context(&state, &window_label);
    let runtime = state.assistant_runtime_snapshot(&window_label);
    log_flow_success("assistant", "assistant_send_message");
    Ok(build_pending_user_send_state_response(
        &base_engine,
        &window_label,
        &context,
        &runtime,
        &input,
    ))
}

#[instrument(skip_all, fields(command = "assistant_start"))]
#[tauri::command]
pub async fn assistant_start(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<AssistantStateResponse, String> {
    log_flow_start("assistant", "assistant_start");
    let window_label = window.label().to_string();
    let project_path = state.project_for_window(&window_label).ok_or_else(|| {
        gwt_core::logging::log_incident(
            "assistant",
            "assistant_start",
            Some("ASSISTANT_NO_PROJECT"),
            "No project opened",
        );
        "No project opened. Open a project first.".to_string()
    })?;

    state.clear_assistant_session_for_window(&window_label);
    let session_generation = state.begin_assistant_session_for_window(&window_label);
    state.set_assistant_active_run(
        &window_label,
        session_generation,
        AssistantActiveRunKind::Startup,
    );
    {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        engine_guard.insert(
            window_label.clone(),
            AssistantEngine::new(PathBuf::from(&project_path), window_label.clone()),
        );
    }
    set_startup_status(
        &state,
        &window_label,
        "Inspecting repository state...".to_string(),
    )
    .map_err(|e| format!("Lock error: {}", e))?;

    let app_handle = window.app_handle().clone();
    let window_label_for_task = window_label.clone();
    let project_path_for_task = project_path.clone();
    tokio::spawn(async move {
        let state = app_handle.state::<AppState>();
        if !state.is_current_assistant_session(&window_label_for_task, session_generation) {
            return;
        }
        let _ = emit_startup_status_if_current(
            &app_handle,
            &window_label_for_task,
            session_generation,
            "Inspecting repository state...".to_string(),
        );
        let mut engine = state
            .assistant_engine
            .lock()
            .ok()
            .and_then(|map| map.get(&window_label_for_task).cloned())
            .unwrap_or_else(|| {
                AssistantEngine::new(
                    PathBuf::from(&project_path_for_task),
                    window_label_for_task.clone(),
                )
            });

        let fingerprint = match resolve_startup_analysis_fingerprint(&state, &window_label_for_task)
        {
            Ok(fingerprint) => fingerprint,
            Err(err) => {
                gwt_core::logging::log_incident(
                    "assistant",
                    "assistant_start",
                    Some("ASSISTANT_REPO_INSPECTION_FAILED"),
                    &err,
                );
                engine.apply_startup_failure_message(format!(
                    "Assistant started, but repository inspection failed: {err}"
                ));
                finish_startup_session(
                    &app_handle,
                    &window_label_for_task,
                    &project_path_for_task,
                    session_generation,
                    engine,
                );
                return;
            }
        };
        if !state.is_current_assistant_session(&window_label_for_task, session_generation) {
            return;
        }

        let context_path = assistant_context_cache_path(&project_path_for_task);
        let context = match resolve_assistant_context(&state, &window_label_for_task) {
            Ok(context) => {
                store_assistant_context(&state, &window_label_for_task, context.clone());
                let _ = save_assistant_context_cache(&context_path, &context);
                context
            }
            Err(err) => {
                gwt_core::logging::log_incident(
                    "assistant",
                    "assistant_start",
                    Some("ASSISTANT_CONTEXT_RESOLVE_FAILED"),
                    &err,
                );
                let context = AssistantContext {
                    current_status: Some("blocked".to_string()),
                    blockers: vec![format!(
                        "起動時にプロジェクト文脈を解決できませんでした: {err}"
                    )],
                    recommended_next_actions: vec![
                        "README / CLAUDE.md / issue の整合性を確認する".to_string()
                    ],
                    ..AssistantContext::default()
                };
                store_assistant_context(&state, &window_label_for_task, context.clone());
                let _ = save_assistant_context_cache(&context_path, &context);
                engine.push_visible_assistant_message(format_assistant_context_message(&context));
                finish_startup_session(
                    &app_handle,
                    &window_label_for_task,
                    &project_path_for_task,
                    session_generation,
                    engine,
                );
                return;
            }
        };
        if !state.is_current_assistant_session(&window_label_for_task, session_generation) {
            return;
        }

        if context.current_status.as_deref() == Some("awaiting_goal_confirmation") {
            engine.push_visible_assistant_message(format_assistant_context_message(&context));
            finish_startup_session(
                &app_handle,
                &window_label_for_task,
                &project_path_for_task,
                session_generation,
                engine,
            );
            return;
        }

        let cache_path = startup_analysis_cache_path(&project_path_for_task);
        let _ = emit_startup_status_if_current(
            &app_handle,
            &window_label_for_task,
            session_generation,
            "Checking startup analysis cache...".to_string(),
        );
        if let Some(cache) = load_startup_analysis_cache(&cache_path) {
            if cache.fingerprint == fingerprint {
                let _ = emit_startup_status_if_current(
                    &app_handle,
                    &window_label_for_task,
                    session_generation,
                    "Using cached startup analysis...".to_string(),
                );
                engine.apply_cached_startup_summary(cache.summary);
                if let Some(cached_context) = load_assistant_context_cache(&context_path) {
                    store_assistant_context(&state, &window_label_for_task, cached_context);
                }
                finish_startup_session(
                    &app_handle,
                    &window_label_for_task,
                    &project_path_for_task,
                    session_generation,
                    engine,
                );
                return;
            }
        }
        if !state.is_current_assistant_session(&window_label_for_task, session_generation) {
            return;
        }

        let _ = emit_startup_status_if_current(
            &app_handle,
            &window_label_for_task,
            session_generation,
            "Running startup analysis...".to_string(),
        );
        match engine.handle_startup_with_cancel(&state, || {
            !state.is_current_assistant_session(&window_label_for_task, session_generation)
        }) {
            Ok(true) => {
                if engine.startup_summary_ready() {
                    if let Some(summary) =
                        engine
                            .conversation()
                            .iter()
                            .rev()
                            .find_map(|item| match item {
                                gwt_core::ai::ConversationItem::Message { role, content }
                                    if role == "assistant" =>
                                {
                                    Some(content.clone())
                                }
                                _ => None,
                            })
                    {
                        let cache = StartupAnalysisCacheEntry {
                            fingerprint,
                            summary,
                        };
                        let _ = save_startup_analysis_cache(&cache_path, &cache);
                    }
                }
                if should_emit_context_follow_up(&context) {
                    engine
                        .push_visible_assistant_message(format_assistant_context_message(&context));
                }
            }
            Ok(false) => {
                state.clear_assistant_active_run_if_generation(
                    &window_label_for_task,
                    session_generation,
                );
                clear_startup_status(&state, &window_label_for_task);
                return;
            }
            Err(err) => {
                gwt_core::logging::log_incident(
                    "assistant",
                    "assistant_start",
                    Some("ASSISTANT_INITIAL_ANALYSIS_FAILED"),
                    &err.to_string(),
                );
                engine.apply_startup_failure_message(format!(
                    "Assistant started, but the initial analysis failed: {err}"
                ));
                engine.push_visible_assistant_message(format_assistant_context_message(&context));
            }
        }

        finish_startup_session(
            &app_handle,
            &window_label_for_task,
            &project_path_for_task,
            session_generation,
            engine,
        );
    });

    log_flow_success("assistant", "assistant_start");
    Ok(build_startup_inflight_state_response(
        window_label,
        "Inspecting repository state...".to_string(),
    ))
}

#[instrument(skip_all, fields(command = "assistant_stop"))]
#[tauri::command]
pub async fn assistant_stop(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    log_flow_start("assistant", "assistant_stop");
    let window_label = window.label().to_string();
    state.clear_assistant_session_for_window(&window_label);

    log_flow_success("assistant", "assistant_stop");
    Ok(())
}

#[instrument(skip_all, fields(command = "assistant_get_dashboard"))]
#[tauri::command]
pub async fn assistant_get_dashboard(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<DashboardResponse, String> {
    build_dashboard_response(&state, window.label())
}

fn build_dashboard_response(
    state: &AppState,
    window_label: &str,
) -> Result<DashboardResponse, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Ok(DashboardResponse {
            panes: Vec::new(),
            git: build_git_dashboard(state, window_label)?,
            spec_progress: None,
            ci_status: None,
            consultation_count: 0,
        });
    };
    let repo_path = project_path.as_str();
    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(repo_path))?;
    let panes = {
        let mut mgr = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        mgr.panes_mut()
            .iter_mut()
            .filter(|pane| pane.project_root() == repo_path)
            .map(|pane| {
                let _ = pane.check_status();
                PaneDashboard {
                    pane_id: pane.pane_id().to_string(),
                    agent_name: pane.agent_name().to_string(),
                    status: normalize_pane_status(pane.status()),
                }
            })
            .collect::<Vec<_>>()
    };

    let git = build_git_dashboard(state, window_label)?;
    let spec_progress = resolve_spec_progress(&repo_path, &git.branch, &project_path);
    let ci_status = resolve_ci_status(&repo_path, &git.branch);

    Ok(DashboardResponse {
        panes,
        git,
        spec_progress,
        ci_status,
        consultation_count: 0,
    })
}

fn build_assistant_state_response(
    engine: &AssistantEngine,
    session_id: Option<String>,
    context: &AssistantContext,
    runtime: &AssistantRuntimeState,
    startup_status_message: Option<&str>,
) -> AssistantStateResponse {
    let recovery = derive_startup_recovery_info(engine);
    let mut messages = build_messages_from_conversation(engine);
    if matches!(runtime.active_kind, Some(AssistantActiveRunKind::Startup)) {
        if let Some(status_message) = startup_status_message {
            messages.push(AssistantMessage {
                role: "assistant".to_string(),
                kind: "text".to_string(),
                content: status_message.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
            });
        }
    }
    AssistantStateResponse {
        messages,
        ai_ready: true,
        is_thinking: runtime.active_kind.is_some()
            || engine.startup_status() == AssistantStartupStatus::Analyzing,
        session_id,
        llm_call_count: engine.llm_call_count,
        estimated_tokens: engine.estimated_tokens,
        startup_status: if matches!(runtime.active_kind, Some(AssistantActiveRunKind::Startup)) {
            AssistantStartupStatus::Analyzing.as_str().to_string()
        } else {
            engine.startup_status().as_str().to_string()
        },
        startup_summary_ready: engine.startup_summary_ready(),
        startup_failure_kind: recovery.kind,
        startup_failure_detail: recovery.detail,
        startup_recovery_hints: recovery.hints,
        working_goal: context.working_goal.clone(),
        goal_confidence: context.goal_confidence.clone(),
        current_status: context.current_status.clone(),
        blockers: context.blockers.clone(),
        recommended_next_actions: context.recommended_next_actions.clone(),
        queued_message_count: runtime.queued_inputs.len(),
    }
}

fn build_empty_assistant_state_response() -> AssistantStateResponse {
    AssistantStateResponse {
        messages: Vec::new(),
        ai_ready: check_ai_configured(),
        is_thinking: false,
        session_id: None,
        llm_call_count: 0,
        estimated_tokens: 0,
        startup_status: AssistantStartupStatus::Idle.as_str().to_string(),
        startup_summary_ready: false,
        startup_failure_kind: None,
        startup_failure_detail: None,
        startup_recovery_hints: Vec::new(),
        working_goal: None,
        goal_confidence: None,
        current_status: None,
        blockers: Vec::new(),
        recommended_next_actions: Vec::new(),
        queued_message_count: 0,
    }
}

fn build_startup_inflight_state_response(
    session_id: String,
    status_message: String,
) -> AssistantStateResponse {
    AssistantStateResponse {
        messages: vec![AssistantMessage {
            role: "assistant".to_string(),
            kind: "text".to_string(),
            content: status_message,
            timestamp: chrono::Utc::now().timestamp(),
        }],
        ai_ready: check_ai_configured(),
        is_thinking: true,
        session_id: Some(session_id),
        llm_call_count: 0,
        estimated_tokens: 0,
        startup_status: AssistantStartupStatus::Analyzing.as_str().to_string(),
        startup_summary_ready: false,
        startup_failure_kind: None,
        startup_failure_detail: None,
        startup_recovery_hints: Vec::new(),
        working_goal: None,
        goal_confidence: None,
        current_status: Some("analyzing".to_string()),
        blockers: Vec::new(),
        recommended_next_actions: Vec::new(),
        queued_message_count: 0,
    }
}

fn build_pending_user_send_state_response(
    engine: &AssistantEngine,
    session_id: &str,
    context: &AssistantContext,
    runtime: &AssistantRuntimeState,
    input: &str,
) -> AssistantStateResponse {
    let mut response = build_assistant_state_response(
        engine,
        Some(session_id.to_string()),
        context,
        runtime,
        None,
    );
    response.is_thinking = true;
    response.messages.push(AssistantMessage {
        role: "user".to_string(),
        kind: "text".to_string(),
        content: input.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
    });
    response
}

fn set_startup_status(
    state: &AppState,
    window_label: &str,
    status_message: String,
) -> Result<(), String> {
    let mut guard = state
        .assistant_startup_inflight
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    guard.insert(window_label.to_string(), status_message);
    Ok(())
}

fn emit_startup_status(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    status_message: String,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    set_startup_status(&state, window_label, status_message.clone())?;

    if let Some(window) = app_handle.get_webview_window(window_label) {
        let response =
            build_startup_inflight_state_response(window_label.to_string(), status_message);
        let _ = window.emit("assistant-state-updated", &response);
    }

    Ok(())
}

fn clear_startup_status(state: &AppState, window_label: &str) {
    if let Ok(mut guard) = state.assistant_startup_inflight.lock() {
        guard.remove(window_label);
    }
}

fn current_startup_status_message(state: &AppState, window_label: &str) -> Option<String> {
    state
        .assistant_startup_inflight
        .lock()
        .ok()
        .and_then(|guard| guard.get(window_label).cloned())
}

fn emit_startup_status_if_current(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    session_generation: u64,
    status_message: String,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    if !state.is_current_assistant_session(window_label, session_generation) {
        clear_startup_status(&state, window_label);
        return Ok(());
    }
    emit_startup_status(app_handle, window_label, status_message)
}

fn finish_startup_session(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    project_path: &str,
    session_generation: u64,
    engine: AssistantEngine,
) {
    let state = app_handle.state::<AppState>();
    clear_startup_status(&state, window_label);
    state.clear_assistant_active_run_if_generation(window_label, session_generation);
    if !state.is_current_assistant_session(window_label, session_generation) {
        return;
    }

    if let Ok(response) = finalize_started_engine(&state, window_label, project_path, engine) {
        start_assistant_monitor(app_handle, window_label, project_path);
        if let Some(window) = app_handle.get_webview_window(window_label) {
            let _ = window.emit("assistant-state-updated", &response);
        }
    }
}

fn finalize_started_engine(
    state: &AppState,
    window_label: &str,
    project_path: &str,
    engine: AssistantEngine,
) -> Result<AssistantStateResponse, String> {
    let current_project_path = state.project_for_window(window_label);
    let context = current_assistant_context(state, window_label);
    let runtime = state.assistant_runtime_snapshot(window_label);
    let mut engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    if current_project_path.as_deref() != Some(project_path) {
        return Ok(engine_guard
            .get(window_label)
            .map(|current| {
                build_assistant_state_response(
                    current,
                    Some(window_label.to_string()),
                    &context,
                    &runtime,
                    None,
                )
            })
            .unwrap_or_else(build_empty_assistant_state_response));
    }

    engine_guard.insert(window_label.to_string(), engine);
    let inserted = engine_guard
        .get(window_label)
        .ok_or_else(|| "Assistant session disappeared after startup.".to_string())?;

    Ok(build_assistant_state_response(
        inserted,
        Some(window_label.to_string()),
        &context,
        &runtime,
        None,
    ))
}

fn ensure_assistant_engine_snapshot(
    state: &AppState,
    window_label: &str,
    project_path: &str,
) -> Result<AssistantEngine, String> {
    let mut engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(engine) = engine_guard.get(window_label) {
        return Ok(engine.clone());
    }

    let engine = AssistantEngine::new(PathBuf::from(project_path), window_label.to_string());
    engine_guard.insert(window_label.to_string(), engine.clone());
    Ok(engine)
}

fn ensure_assistant_monitor_started(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    project_path: &str,
) {
    let state = app_handle.state::<AppState>();
    let already_running = state
        .assistant_monitor_handle
        .lock()
        .ok()
        .is_some_and(|handles| handles.contains_key(window_label));
    if !already_running {
        start_assistant_monitor(app_handle, window_label, project_path);
    }
}

fn spawn_assistant_user_run(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    project_path: &str,
    generation: u64,
    input: String,
    base_engine: AssistantEngine,
    emit_initial_state: bool,
) {
    let app_handle = app_handle.clone();
    let window_label = window_label.to_string();
    let project_path = project_path.to_string();

    tokio::spawn(async move {
        let state = app_handle.state::<AppState>();
        if !state.is_current_assistant_session(&window_label, generation) {
            return;
        }

        if emit_initial_state {
            let context = current_assistant_context(&state, &window_label);
            let runtime = state.assistant_runtime_snapshot(&window_label);
            let response = build_pending_user_send_state_response(
                &base_engine,
                &window_label,
                &context,
                &runtime,
                &input,
            );
            if let Some(window) = app_handle.get_webview_window(&window_label) {
                let _ = window.emit("assistant-state-updated", &response);
            }
        }

        let mut engine = base_engine;
        let run_result = engine.handle_user_message_with_cancel(&input, &state, || {
            !state.is_current_assistant_session(&window_label, generation)
        });

        let should_commit = state.is_current_assistant_session(&window_label, generation);
        if !should_commit {
            return;
        }

        match run_result {
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(err) => {
                gwt_core::logging::log_incident(
                    "assistant",
                    "assistant_send_message",
                    Some("ASSISTANT_REQUEST_FAILED"),
                    &err.to_string(),
                );
                engine.push_visible_assistant_message(format!("Assistant request failed: {err}"));
            }
        }

        if let Ok(mut engine_guard) = state.assistant_engine.lock() {
            engine_guard.insert(window_label.clone(), engine.clone());
        }
        state.clear_assistant_active_run_if_generation(&window_label, generation);

        ensure_assistant_monitor_started(&app_handle, &window_label, &project_path);

        let context = current_assistant_context(&state, &window_label);
        let runtime = state.assistant_runtime_snapshot(&window_label);
        let response = build_assistant_state_response(
            &engine,
            Some(window_label.clone()),
            &context,
            &runtime,
            None,
        );

        if let Some(window) = app_handle.get_webview_window(&window_label) {
            let _ = window.emit("assistant-state-updated", &response);
        }
        emit_dashboard_update(&app_handle, &window_label);

        if let Some(next_input) = state.dequeue_assistant_input(&window_label) {
            let next_generation = state.begin_assistant_session_for_window(&window_label);
            state.set_assistant_active_run(
                &window_label,
                next_generation,
                AssistantActiveRunKind::User,
            );
            let next_engine =
                match ensure_assistant_engine_snapshot(&state, &window_label, &project_path) {
                    Ok(engine) => engine,
                    Err(_) => return,
                };
            spawn_assistant_user_run(
                &app_handle,
                &window_label,
                &project_path,
                next_generation,
                next_input,
                next_engine,
                true,
            );
        }
    });
}

fn current_assistant_context(state: &AppState, window_label: &str) -> AssistantContext {
    state
        .assistant_context
        .lock()
        .ok()
        .and_then(|map| map.get(window_label).cloned())
        .unwrap_or_default()
}

fn derive_startup_recovery_info(engine: &AssistantEngine) -> StartupRecoveryInfo {
    if engine.startup_status() != AssistantStartupStatus::Failed {
        return StartupRecoveryInfo::default();
    }

    let detail = engine
        .conversation()
        .iter()
        .rev()
        .filter_map(|item| match item {
            gwt_core::ai::ConversationItem::Message { role, content } if role == "assistant" => {
                Some(content.as_str())
            }
            _ => None,
        })
        .find_map(|content| {
            let lower = content.to_ascii_lowercase();
            (lower.contains("failed")
                || lower.contains("error")
                || lower.contains("guardrails")
                || lower.contains("insufficient system resources")
                || lower.contains("ai is not configured"))
            .then(|| content.to_string())
        });
    let detail_lower = detail
        .as_deref()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    let (kind, hints) = if detail_lower.contains("ai is not configured")
        || detail_lower.contains("please configure ai settings first")
    {
        (
            Some("ai_not_configured".to_string()),
            vec![
                "Open Settings and configure the active AI profile.".to_string(),
                "Save the profile and retry the Assistant startup.".to_string(),
            ],
        )
    } else if detail_lower.contains("insufficient system resources")
        || detail_lower.contains("guardrails")
        || detail_lower.contains("requires approximately")
    {
        (
            Some("resource_guard".to_string()),
            vec![
                "Choose a smaller model in Settings.".to_string(),
                "Switch to a remote inference endpoint if available.".to_string(),
                "Only relax model loading guardrails if you accept freeze risk.".to_string(),
            ],
        )
    } else if detail_lower.contains("llm call failed") {
        (
            Some("llm_error".to_string()),
            vec![
                "Retry the Assistant startup.".to_string(),
                "Open Settings and verify the active model and endpoint.".to_string(),
            ],
        )
    } else {
        (
            Some("unknown".to_string()),
            vec![
                "Retry the Assistant startup.".to_string(),
                "Open Settings and review the active AI model and endpoint.".to_string(),
            ],
        )
    };

    StartupRecoveryInfo {
        kind,
        detail,
        hints,
    }
}
fn store_assistant_context(state: &AppState, window_label: &str, context: AssistantContext) {
    if let Ok(mut map) = state.assistant_context.lock() {
        map.insert(window_label.to_string(), context);
    }
}

fn assistant_context_cache_path(project_path: &str) -> PathBuf {
    Path::new(project_path)
        .join(".gwt")
        .join("assistant")
        .join("context.json")
}

fn load_assistant_context_cache(path: &Path) -> Option<AssistantContext> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_assistant_context_cache(path: &Path, context: &AssistantContext) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid assistant context cache path".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create assistant cache dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(context)
        .map_err(|e| format!("Failed to serialize assistant context: {e}"))?;
    std::fs::write(&tmp, content)
        .map_err(|e| format!("Failed to write assistant context tmp file: {e}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("Failed to finalize assistant context cache: {e}"))?;
    Ok(())
}

fn start_assistant_monitor(app_handle: &tauri::AppHandle, window_label: &str, project_path: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut handles) = state.assistant_monitor_handle.lock() {
        if let Some(handle) = handles.remove(window_label) {
            tokio::spawn(async move {
                handle.stop().await;
            });
        }
    }

    let (event_tx, mut event_rx) = mpsc::channel::<MonitorEvent>(8);
    let handle = assistant_monitor::start_monitor(
        app_handle.clone(),
        window_label.to_string(),
        project_path.to_string(),
        event_tx,
    );
    if let Ok(mut handles) = state.assistant_monitor_handle.lock() {
        handles.insert(window_label.to_string(), handle);
    }

    let app_handle = app_handle.clone();
    let window_label = window_label.to_string();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            handle_assistant_monitor_event(&app_handle, &window_label, event).await;
        }
    });
}

async fn handle_assistant_monitor_event(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    event: MonitorEvent,
) {
    let MonitorEvent::SnapshotChanged(snapshot) = event;
    let state = app_handle.state::<AppState>();

    let previous_context = current_assistant_context(&state, window_label);
    let ah = app_handle.clone();
    let wl = window_label.to_string();
    let snap = snapshot.clone();
    let next_context = match tokio::task::spawn_blocking(move || {
        let st = ah.state::<AppState>();
        resolve_assistant_context_with_snapshot(&st, &wl, &snap)
    })
    .await
    {
        Ok(Ok(context)) => context,
        Ok(Err(err)) => AssistantContext {
            current_status: Some("blocked".to_string()),
            blockers: vec![format!("監視状態の更新に失敗しました: {err}")],
            recommended_next_actions: vec![
                "プロジェクト状態を再読み込みし、Assistant を再起動する".to_string(),
            ],
            ..AssistantContext::default()
        },
        Err(join_err) => {
            warn!(window = %window_label, error = %join_err, "context resolution panicked");
            return;
        }
    };

    let project_path = state.project_for_window(window_label).unwrap_or_default();
    let context_path = assistant_context_cache_path(&project_path);
    store_assistant_context(&state, window_label, next_context.clone());
    if !project_path.is_empty() {
        let _ = save_assistant_context_cache(&context_path, &next_context);
    }

    if previous_context == next_context {
        emit_dashboard_update(app_handle, window_label);
        return;
    }

    let mut engine = {
        let mut engine_guard = match state.assistant_engine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let Some(engine) = engine_guard.remove(window_label) else {
            emit_dashboard_update(app_handle, window_label);
            return;
        };
        engine
    };

    engine.push_visible_assistant_message(format_assistant_context_message(&next_context));
    let runtime = state.assistant_runtime_snapshot(window_label);
    let response = build_assistant_state_response(
        &engine,
        Some(window_label.to_string()),
        &next_context,
        &runtime,
        None,
    );

    if let Ok(mut engine_guard) = state.assistant_engine.lock() {
        engine_guard.insert(window_label.to_string(), engine);
    }
    if let Some(window) = app_handle.get_webview_window(window_label) {
        let _ = window.emit("assistant-state-updated", &response);
    }
    emit_dashboard_update(app_handle, window_label);
}

fn emit_dashboard_update(app_handle: &tauri::AppHandle, window_label: &str) {
    let state = app_handle.state::<AppState>();
    if let Some(window) = app_handle.get_webview_window(window_label) {
        if let Ok(dashboard) = build_dashboard_response(&state, window_label) {
            let _ = window.emit("assistant-dashboard-updated", &dashboard);
        }
    }
}

fn resolve_assistant_context(
    state: &AppState,
    window_label: &str,
) -> Result<AssistantContext, String> {
    let snapshot = collect_current_monitor_snapshot(state, window_label)?;
    resolve_assistant_context_with_snapshot(state, window_label, &snapshot)
}

fn resolve_assistant_context_with_snapshot(
    state: &AppState,
    window_label: &str,
    snapshot: &MonitorSnapshot,
) -> Result<AssistantContext, String> {
    let project_path = state
        .project_for_window(window_label)
        .ok_or_else(|| "No project opened for assistant context.".to_string())?;
    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;

    let docs_goal = resolve_goal_from_docs(Path::new(&project_path));
    let issue_goal = resolve_goal_from_issue(&repo_path, &snapshot.git.branch);
    let cached_pr = PrCache::fetch_latest_for_branch(&repo_path, &snapshot.git.branch);
    let pr_ref = cached_pr
        .as_ref()
        .map(|pr| (format!("#{} {}", pr.number, pr.title), pr.state.clone()));
    let pr_ci_insight = resolve_pr_ci_insight_with_pr(&repo_path, cached_pr);
    let terminal_summary_insight =
        resolve_terminal_summary_insight(&project_path, &snapshot.git.branch, state);
    let current_branch_panes = snapshot
        .panes
        .iter()
        .filter(|pane| pane.branch == snapshot.git.branch)
        .cloned()
        .collect::<Vec<_>>();

    let working_goal = issue_goal
        .as_ref()
        .and_then(|goal| goal.goal.clone())
        .or_else(|| docs_goal.as_ref().and_then(|goal| goal.goal.clone()));
    let goal_confidence = if issue_goal.is_some() && docs_goal.is_some() {
        Some("high".to_string())
    } else {
        issue_goal
            .as_ref()
            .and_then(|goal| goal.confidence.clone())
            .or_else(|| docs_goal.as_ref().and_then(|goal| goal.confidence.clone()))
    };

    let mut blockers = Vec::new();
    if working_goal.is_none() {
        blockers.push(
            "README / CLAUDE.md / 現在の branch から、着手中のゴールを一意に特定できません。"
                .to_string(),
        );
    }

    let stopped_panes = current_branch_panes
        .iter()
        .filter(|pane| pane.status != "running")
        .cloned()
        .collect::<Vec<_>>();
    if !stopped_panes.is_empty() {
        let summaries = stopped_panes
            .iter()
            .map(|pane| format!("{} ({})", pane.agent_name, pane.status))
            .collect::<Vec<_>>()
            .join(", ");
        blockers.push(format!(
            "現在のブランチ `{}` の agent が停止しています: {summaries}",
            snapshot.git.branch
        ));
    }

    let has_running_current_branch_agent = current_branch_panes
        .iter()
        .any(|pane| pane.status == "running");
    if working_goal.is_some()
        && !snapshot.git.branch.is_empty()
        && !has_running_current_branch_agent
        && !snapshot.git.branch.eq_ignore_ascii_case("main")
        && !snapshot.git.branch.eq_ignore_ascii_case("develop")
    {
        blockers.push(format!(
            "現在のブランチ `{}` に稼働中の agent がありません。",
            snapshot.git.branch
        ));
    }
    if let Some(insight) = pr_ci_insight.as_ref() {
        if !insight.failing_required_checks.is_empty() {
            blockers.push(format!(
                "PR #{} `{}` の required check が失敗しています: {}",
                insight.pr_number,
                insight.pr_title,
                insight.failing_required_checks.join(", ")
            ));
        }
        if !insight.changes_requested_by.is_empty() {
            blockers.push(format!(
                "PR #{} `{}` に changes requested があります: {}",
                insight.pr_number,
                insight.pr_title,
                insight.changes_requested_by.join(", ")
            ));
        }
        if insight.merge_state_status.as_deref() == Some("BEHIND") {
            blockers.push(format!(
                "PR #{} `{}` は base branch への追従が必要です。",
                insight.pr_number, insight.pr_title
            ));
        }
    }
    if let Some(insight) = terminal_summary_insight.as_ref() {
        blockers.push(format!(
            "terminal summary indicates a likely failure: {}",
            insight.short_summary
        ));
    }

    let mut recommended_next_actions = Vec::new();
    if working_goal.is_none() {
        push_unique(
            &mut recommended_next_actions,
            "現在のゴールを一文で確認し、必要なら issue または README に明記する".to_string(),
        );
    }
    if !stopped_panes.is_empty() {
        push_unique(
            &mut recommended_next_actions,
            "停止した pane の scrollback を確認し、再開するか新しい agent を起動する".to_string(),
        );
    } else if working_goal.is_some() && !has_running_current_branch_agent {
        push_unique(
            &mut recommended_next_actions,
            format!(
                "ブランチ `{}` で agent を起動して作業を再開する",
                snapshot.git.branch
            ),
        );
    }
    if snapshot.git.uncommitted_count > 0 {
        push_unique(
            &mut recommended_next_actions,
            format!(
                "未コミット変更 {} 件を確認し、現在ゴールとの整合を見直す",
                snapshot.git.uncommitted_count
            ),
        );
    } else if let Some(goal) = issue_goal.as_ref().and_then(|goal| goal.goal.as_ref()) {
        push_unique(
            &mut recommended_next_actions,
            format!("issue に沿って次の実装単位を決める: {goal}"),
        );
    }
    if let Some(insight) = pr_ci_insight.as_ref() {
        if !insight.failing_required_checks.is_empty() {
            push_unique(
                &mut recommended_next_actions,
                format!(
                    "PR #{} の失敗 check を確認して修正する: {}",
                    insight.pr_number,
                    insight.failing_required_checks.join(", ")
                ),
            );
        }
        if !insight.pending_required_checks.is_empty() {
            push_unique(
                &mut recommended_next_actions,
                format!(
                    "PR #{} の required check 実行状況を確認する: {}",
                    insight.pr_number,
                    insight.pending_required_checks.join(", ")
                ),
            );
        }
        if !insight.changes_requested_by.is_empty() {
            push_unique(
                &mut recommended_next_actions,
                format!(
                    "PR #{} の review 指摘に対応する: {}",
                    insight.pr_number,
                    insight.changes_requested_by.join(", ")
                ),
            );
        }
        if insight.merge_state_status.as_deref() == Some("BEHIND") {
            push_unique(
                &mut recommended_next_actions,
                format!(
                    "PR #{} を merge 可能にするため base branch を取り込む",
                    insight.pr_number
                ),
            );
        }
        push_unique(
            &mut recommended_next_actions,
            format!(
                "PR #{} の詳細を確認する: {}",
                insight.pr_number, insight.pr_url
            ),
        );
    }
    if let Some(insight) = terminal_summary_insight.as_ref() {
        push_unique(
            &mut recommended_next_actions,
            format!(
                "Review the latest terminal summary and recover: {}",
                insight.short_summary
            ),
        );
        for highlight in insight.highlights.iter().take(2) {
            push_unique(
                &mut recommended_next_actions,
                format!("Check terminal highlight: {highlight}"),
            );
        }
        if insight.source_type.as_deref() == Some("scrollback") {
            push_unique(
                &mut recommended_next_actions,
                "Open the pane scrollback and inspect the latest abnormal log".to_string(),
            );
        }
    }
    if let Some((title, state_label)) = pr_ref {
        if state_label.eq_ignore_ascii_case("MERGED") {
            push_unique(
                &mut recommended_next_actions,
                format!("関連 PR `{title}` は merge 済みなので worktree cleanup を検討する"),
            );
        } else if state_label.eq_ignore_ascii_case("OPEN") {
            push_unique(
                &mut recommended_next_actions,
                format!("open PR `{title}` の状態とレビュー待ち項目を確認する"),
            );
        }
    }
    recommended_next_actions.truncate(3);

    let current_status = if working_goal.is_none() {
        Some("awaiting_goal_confirmation".to_string())
    } else if blockers.is_empty() {
        Some("monitoring".to_string())
    } else {
        Some("blocked".to_string())
    };

    Ok(AssistantContext {
        working_goal,
        goal_confidence,
        current_status,
        blockers,
        recommended_next_actions,
        dispatched_tasks: Vec::new(),
    })
}

fn collect_current_monitor_snapshot(
    state: &AppState,
    window_label: &str,
) -> Result<MonitorSnapshot, String> {
    let git = build_git_dashboard(state, window_label)?;
    let panes = collect_current_pane_snapshots(state, window_label)?;
    let pending_consultations = state
        .project_for_window(window_label)
        .map(|p| crate::consultation::count_pending_consultations(Path::new(&p)))
        .unwrap_or(0);
    Ok(MonitorSnapshot {
        panes,
        git: assistant_monitor::GitStatusSnapshot {
            branch: git.branch,
            uncommitted_count: git.uncommitted_count,
            unpushed_count: git.unpushed_count,
        },
        pending_consultations,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

fn collect_current_pane_snapshots(
    state: &AppState,
    window_label: &str,
) -> Result<Vec<PaneSnapshot>, String> {
    let project_path = state
        .project_for_window(window_label)
        .ok_or_else(|| "No project opened for pane snapshot.".to_string())?;
    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
    Ok(manager
        .panes_mut()
        .iter_mut()
        .filter(|pane| pane.project_root() == repo_path)
        .map(|pane| {
            let _ = pane.check_status();
            PaneSnapshot {
                pane_id: pane.pane_id().to_string(),
                agent_name: pane.agent_name().to_string(),
                branch: pane.branch_name().to_string(),
                status: normalize_pane_status(pane.status()),
                scrollback_hash: 0,
            }
        })
        .collect())
}

fn resolve_goal_from_docs(project_root: &Path) -> Option<GoalResolution> {
    for relative_path in ["README.md", "CLAUDE.md"] {
        let path = project_root.join(relative_path);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(goal) = extract_goal_from_markdown(&content) {
            return Some(GoalResolution {
                goal: Some(goal),
                confidence: Some("medium".to_string()),
            });
        }
    }
    None
}

fn extract_goal_from_markdown(content: &str) -> Option<String> {
    let normalized = content.replace("\r\n", "\n");
    for block in normalized.split("\n\n") {
        let cleaned = block
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty()
                    && !line.starts_with('#')
                    && !line.starts_with('[')
                    && !line.starts_with("```")
                    && !line.starts_with('|')
            })
            .collect::<Vec<_>>()
            .join(" ");
        let cleaned = cleaned.trim();
        if cleaned.is_empty() || cleaned.len() < 30 {
            continue;
        }
        let truncated = cleaned.chars().take(220).collect::<String>();
        return Some(truncated);
    }
    None
}

fn resolve_goal_from_issue(repo_path: &Path, branch: &str) -> Option<GoalResolution> {
    let issue_number = extract_issue_number_from_branch(branch)?;
    let detail = get_spec_issue_detail(repo_path, issue_number).ok()?;
    Some(GoalResolution {
        goal: Some(format!("#{} {}", detail.number, detail.title)),
        confidence: Some("high".to_string()),
    })
}

fn resolve_pr_ci_insight_with_pr(
    repo_path: &Path,
    pr: Option<gwt_core::git::PullRequest>,
) -> Option<PrCiInsight> {
    let pr = pr?;
    if !pr.state.eq_ignore_ascii_case("OPEN") {
        return None;
    }

    let detail = graphql::fetch_pr_detail(repo_path, pr.number).ok()?;

    Some(PrCiInsight {
        pr_number: detail.number,
        pr_title: detail.title,
        pr_url: detail.url,
        merge_state_status: detail.merge_state_status,
        failing_required_checks: failing_required_check_names(&detail.check_suites),
        pending_required_checks: pending_required_check_names(&detail.check_suites),
        changes_requested_by: detail
            .reviews
            .iter()
            .filter(|review| review.state == "CHANGES_REQUESTED")
            .map(|review| review.reviewer.clone())
            .collect(),
    })
}

fn resolve_terminal_summary_insight(
    project_path: &str,
    branch: &str,
    state: &AppState,
) -> Option<TerminalSummaryInsight> {
    let summary = get_branch_session_summary_for_assistant(project_path, branch, state)?;
    if summary.status != "ok" {
        return None;
    }

    let short_summary = summary
        .short_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| {
            summary
                .bullet_points
                .iter()
                .map(|value| value.trim().trim_start_matches("- ").trim())
                .find(|value| !value.is_empty())
                .map(|value| value.to_string())
        })?;

    let highlights = summary
        .bullet_points
        .iter()
        .map(|value| value.trim().trim_start_matches("- ").trim().to_string())
        .filter(|value| contains_terminal_anomaly_signal(value))
        .collect::<Vec<_>>();

    if !contains_terminal_anomaly_signal(&short_summary) && highlights.is_empty() {
        return None;
    }

    Some(TerminalSummaryInsight {
        short_summary,
        highlights,
        source_type: summary.source_type,
    })
}

fn failing_required_check_names(checks: &[WorkflowRunInfo]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| {
            check.is_required == Some(true)
                && matches!(
                    check.conclusion.as_deref(),
                    Some(
                        "failure"
                            | "cancelled"
                            | "timed_out"
                            | "action_required"
                            | "startup_failure"
                    )
                )
        })
        .map(|check| check.workflow_name.clone())
        .collect()
}

fn contains_terminal_anomaly_signal(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    [
        "error",
        "errors",
        "failed",
        "failure",
        "panic",
        "exception",
        "traceback",
        "timed out",
        "timeout",
        "segmentation fault",
        "test failed",
        "build failed",
        "compile error",
        "fatal",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword))
        || [
            "エラー",
            "失敗",
            "例外",
            "タイムアウト",
            "ビルド失敗",
            "テスト失敗",
            "panic",
        ]
        .iter()
        .any(|keyword| text.contains(keyword))
}

fn pending_required_check_names(checks: &[WorkflowRunInfo]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| {
            check.is_required == Some(true)
                && (check.status != "completed" || check.conclusion.is_none())
        })
        .map(|check| check.workflow_name.clone())
        .collect()
}

fn resolve_spec_progress(
    repo_path: &Path,
    branch: &str,
    _project_path: &str,
) -> Option<SpecProgressSummary> {
    let issue_number = extract_issue_number_from_branch(branch)?;
    let detail = get_spec_issue_detail(repo_path, issue_number).ok()?;

    let tasks_section = &detail.sections.tasks;
    let tasks_total = tasks_section.matches("- [").count() as u32;
    let tasks_completed = tasks_section.matches("- [x]").count() as u32;

    let phase = if tasks_section.is_empty() && detail.sections.plan.is_empty() {
        "draft"
    } else if tasks_section.is_empty() {
        "planned"
    } else if tasks_completed == tasks_total && tasks_total > 0 {
        "done"
    } else {
        "in-progress"
    };

    Some(SpecProgressSummary {
        issue_number: detail.number,
        title: detail.title,
        phase: phase.to_string(),
        tasks_total,
        tasks_completed,
    })
}

fn resolve_ci_status(repo_path: &Path, branch: &str) -> Option<CiStatusSummary> {
    let pr = PrCache::fetch_latest_for_branch(repo_path, branch);
    let insight = resolve_pr_ci_insight_with_pr(repo_path, pr)?;

    let check_status = if !insight.failing_required_checks.is_empty() {
        "failing"
    } else if !insight.pending_required_checks.is_empty() {
        "pending"
    } else {
        "passing"
    };

    let review_status = if !insight.changes_requested_by.is_empty() {
        "changes_requested"
    } else {
        "pending"
    };

    Some(CiStatusSummary {
        pr_number: insight.pr_number,
        pr_title: insight.pr_title,
        check_status: check_status.to_string(),
        failing_checks: insight.failing_required_checks,
        review_status: review_status.to_string(),
    })
}

fn extract_issue_number_from_branch(branch: &str) -> Option<u64> {
    branch.split('/').find_map(|segment| {
        let lower = segment.to_ascii_lowercase();
        let rest = lower.strip_prefix("issue-")?;
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.is_empty() {
            None
        } else {
            digits.parse::<u64>().ok()
        }
    })
}

fn should_emit_context_follow_up(context: &AssistantContext) -> bool {
    !context.blockers.is_empty() || !context.recommended_next_actions.is_empty()
}

fn format_assistant_context_message(context: &AssistantContext) -> String {
    let mut lines = Vec::new();
    lines.push("## Assistant PM Update".to_string());
    match context.working_goal.as_deref() {
        Some(goal) => lines.push(format!("- Current goal: {goal}")),
        None => lines.push("- Current goal: not confirmed yet".to_string()),
    }
    if let Some(status) = context.current_status.as_deref() {
        lines.push(format!("- Status: {}", format_status_label(status)));
    }
    if !context.blockers.is_empty() {
        lines.push(String::new());
        lines.push("### Blockers".to_string());
        for blocker in &context.blockers {
            lines.push(format!("- {blocker}"));
        }
    }
    if !context.recommended_next_actions.is_empty() {
        lines.push(String::new());
        lines.push("### Recommended Next Actions".to_string());
        for action in &context.recommended_next_actions {
            lines.push(format!("- {action}"));
        }
    }
    if context.current_status.as_deref() == Some("awaiting_goal_confirmation") {
        lines.push(String::new());
        lines.push("現在の作業ゴールを一文で確認してください。必要なら issue / README への反映も提案します。".to_string());
    }
    lines.join("\n")
}

fn format_status_label(status: &str) -> &'static str {
    match status {
        "analyzing" => "analyzing",
        "awaiting_goal_confirmation" => "awaiting goal confirmation",
        "blocked" => "blocked",
        "monitoring" => "monitoring",
        _ => "unknown",
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn normalize_pane_status(status: &PaneStatus) -> String {
    match status {
        PaneStatus::Running => "running".to_string(),
        PaneStatus::Completed(code) => format!("completed({code})"),
        PaneStatus::Error(message) => format!("error: {message}"),
    }
}

fn startup_analysis_cache_path(project_path: &str) -> PathBuf {
    Path::new(project_path)
        .join(".gwt")
        .join("assistant")
        .join("startup-analysis.json")
}

fn load_startup_analysis_cache(path: &Path) -> Option<StartupAnalysisCacheEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_startup_analysis_cache(
    path: &Path,
    entry: &StartupAnalysisCacheEntry,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid startup analysis cache path".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create cache dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(entry)
        .map_err(|e| format!("Failed to serialize startup cache: {e}"))?;
    std::fs::write(&tmp, content).map_err(|e| format!("Failed to write cache tmp file: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to finalize startup cache: {e}"))?;
    Ok(())
}

fn resolve_startup_analysis_fingerprint(
    state: &AppState,
    window_label: &str,
) -> Result<StartupAnalysisFingerprint, String> {
    let project_path = state
        .project_for_window(window_label)
        .ok_or_else(|| "No project opened for startup analysis.".to_string())?;

    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;

    let current_branch = Branch::current(&repo_path)
        .map_err(|e| format!("Failed to resolve current branch: {e}"))?;
    let branch = current_branch
        .as_ref()
        .map(|branch| branch.name.clone())
        .unwrap_or_default();

    let worktree_path = resolve_dashboard_worktree_path(&repo_path, current_branch.as_ref())
        .unwrap_or_else(|| repo_path.clone());
    let head_revision = run_git_text(&worktree_path, &["rev-parse", "HEAD"])?;
    let worktree_status = run_git_text(&worktree_path, &["status", "--short"])?;

    Ok(StartupAnalysisFingerprint {
        branch,
        head_revision,
        worktree_status,
    })
}

fn run_git_text(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = process_command("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", args.join(" "), e))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn build_git_dashboard(state: &AppState, window_label: &str) -> Result<GitDashboard, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Ok(GitDashboard {
            branch: String::new(),
            uncommitted_count: 0,
            unpushed_count: 0,
        });
    };

    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
            .map_err(|e| format!("Failed to resolve repository path: {}", e))?;

    let current_branch = Branch::current(&repo_path)
        .map_err(|e| format!("Failed to resolve current branch: {}", e))?;

    let (branch, unpushed_count) = current_branch
        .as_ref()
        .map(|branch| {
            (
                branch.name.clone(),
                branch.ahead.min(u32::MAX as usize) as u32,
            )
        })
        .unwrap_or_else(|| (String::new(), 0));

    let uncommitted_count = resolve_dashboard_worktree_path(&repo_path, current_branch.as_ref())
        .and_then(|path| git::get_working_tree_status(&path).ok())
        .map(|entries| entries.len().min(u32::MAX as usize) as u32)
        .unwrap_or(0);

    Ok(GitDashboard {
        branch,
        uncommitted_count,
        unpushed_count,
    })
}

fn resolve_dashboard_worktree_path(
    repo_path: &Path,
    _current_branch: Option<&Branch>,
) -> Option<PathBuf> {
    Some(repo_path.to_path_buf())
}

fn build_messages_from_conversation(engine: &AssistantEngine) -> Vec<AssistantMessage> {
    let now = chrono::Utc::now().timestamp();
    engine
        .conversation()
        .iter()
        .filter_map(|item| match item {
            gwt_core::ai::ConversationItem::Message { role, content } => {
                if role == "system" {
                    return None;
                }
                Some(AssistantMessage {
                    role: role.clone(),
                    kind: "text".to_string(),
                    content: content.clone(),
                    timestamp: now,
                })
            }
            gwt_core::ai::ConversationItem::FunctionCall { name, .. } => Some(AssistantMessage {
                role: "assistant".to_string(),
                kind: "tool_use".to_string(),
                content: format!("Tool call: {}", name),
                timestamp: now,
            }),
            gwt_core::ai::ConversationItem::FunctionCallOutput { .. } => None,
        })
        .collect()
}

fn check_ai_configured() -> bool {
    gwt_core::config::ProfilesConfig::load()
        .ok()
        .map(|profiles| profiles.resolve_active_ai_settings().resolved.is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn assistant_build_messages_from_conversation_hides_system_messages() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        engine.push_hidden_system_message_for_test("hidden startup prompt");
        engine.push_visible_assistant_message("visible guidance");

        let messages = build_messages_from_conversation(&engine);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].content, "visible guidance");
    }

    #[test]
    fn assistant_startup_inflight_state_exposes_session_id_and_thinking() {
        let response = build_startup_inflight_state_response(
            "main".to_string(),
            "Checking startup analysis cache...".to_string(),
        );

        assert_eq!(response.session_id.as_deref(), Some("main"));
        assert!(response.is_thinking);
        assert_eq!(response.messages.len(), 1);
        assert_eq!(
            response.messages[0].content,
            "Checking startup analysis cache..."
        );
        assert_eq!(response.current_status.as_deref(), Some("analyzing"));
    }

    #[test]
    fn assistant_startup_analysis_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup-analysis.json");
        let entry = StartupAnalysisCacheEntry {
            fingerprint: StartupAnalysisFingerprint {
                branch: "main".to_string(),
                head_revision: "abc123".to_string(),
                worktree_status: " M src/lib.rs".to_string(),
            },
            summary: "Cached startup summary".to_string(),
        };

        save_startup_analysis_cache(&path, &entry).unwrap();
        let loaded = load_startup_analysis_cache(&path).unwrap();

        assert_eq!(loaded, entry);
    }

    #[test]
    fn clear_startup_status_removes_inflight_entry() {
        let state = AppState::new();
        set_startup_status(&state, "main", "Running startup analysis...".to_string()).unwrap();

        clear_startup_status(&state, "main");

        let guard = state.assistant_startup_inflight.lock().unwrap();
        assert!(!guard.contains_key("main"));
    }

    #[test]
    fn finalize_started_engine_skips_stale_project_startup() {
        let state = AppState::new();
        state
            .claim_project_for_window_with_identity(
                "main",
                "/tmp/current".to_string(),
                "/tmp/current-id".to_string(),
            )
            .unwrap();

        let stale_engine = AssistantEngine::new(PathBuf::from("/tmp/stale"), "main".to_string());
        let response = finalize_started_engine(&state, "main", "/tmp/stale", stale_engine).unwrap();

        assert!(state.assistant_engine.lock().unwrap().is_empty());
        assert_eq!(response.session_id, None);
    }

    #[test]
    fn finalize_started_engine_keeps_existing_session() {
        let state = AppState::new();
        state
            .claim_project_for_window_with_identity(
                "main",
                "/tmp/current".to_string(),
                "/tmp/current-id".to_string(),
            )
            .unwrap();

        state.assistant_engine.lock().unwrap().insert(
            "main".to_string(),
            AssistantEngine::new(PathBuf::from("/tmp/current"), "main".to_string()),
        );

        let stale_engine = AssistantEngine::new(PathBuf::from("/tmp/stale"), "main".to_string());
        let response = finalize_started_engine(&state, "main", "/tmp/stale", stale_engine).unwrap();

        let stored = state.assistant_engine.lock().unwrap();
        let current = stored.get("main").unwrap();
        assert_eq!(current.project_path(), Path::new("/tmp/current"));
        assert_eq!(response.session_id.as_deref(), Some("main"));
    }

    #[test]
    fn assistant_context_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.json");
        let entry = AssistantContext {
            working_goal: Some("#1636 Assistant Mode".to_string()),
            goal_confidence: Some("high".to_string()),
            current_status: Some("monitoring".to_string()),
            blockers: vec!["none".to_string()],
            recommended_next_actions: vec!["do the work".to_string()],
            ..AssistantContext::default()
        };

        save_assistant_context_cache(&path, &entry).unwrap();
        let loaded = load_assistant_context_cache(&path).unwrap();

        assert_eq!(loaded, entry);
    }

    #[test]
    fn assistant_extract_goal_from_markdown_skips_heading_and_tables() {
        let markdown = "# Title\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nThis project manages Git worktrees and launches coding agents for development.";
        let goal = extract_goal_from_markdown(markdown).unwrap();
        assert!(goal.contains("manages Git worktrees"));
    }

    #[test]
    fn assistant_resolve_goal_from_docs_prefers_readme() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("README.md"),
            "# Test\n\nThis repository exists to help developers manage worktrees and run coding agents.",
        )
        .unwrap();
        fs::write(
            dir.path().join("CLAUDE.md"),
            "# CLAUDE\n\nFallback goal text that should not be used first.",
        )
        .unwrap();

        let goal = resolve_goal_from_docs(dir.path()).unwrap();
        assert_eq!(goal.confidence.as_deref(), Some("medium"));
        assert!(goal
            .goal
            .as_deref()
            .unwrap_or_default()
            .contains("manage worktrees"));
    }

    #[test]
    fn assistant_format_context_message_includes_goal_and_actions() {
        let context = AssistantContext {
            working_goal: Some("#1636 Assistant Mode".to_string()),
            goal_confidence: Some("high".to_string()),
            current_status: Some("blocked".to_string()),
            blockers: vec!["Agent stopped".to_string()],
            recommended_next_actions: vec!["Restart the agent".to_string()],
            ..AssistantContext::default()
        };

        let message = format_assistant_context_message(&context);

        assert!(message.contains("Current goal: #1636 Assistant Mode"));
        assert!(message.contains("Blockers"));
        assert!(message.contains("Restart the agent"));
    }

    #[test]
    fn assistant_failing_required_check_names_detects_failures() {
        let checks = vec![
            WorkflowRunInfo {
                workflow_name: "CI".to_string(),
                run_id: 1,
                status: "completed".to_string(),
                conclusion: Some("failure".to_string()),
                is_required: Some(true),
            },
            WorkflowRunInfo {
                workflow_name: "Docs".to_string(),
                run_id: 2,
                status: "completed".to_string(),
                conclusion: Some("failure".to_string()),
                is_required: Some(false),
            },
        ];

        assert_eq!(
            failing_required_check_names(&checks),
            vec!["CI".to_string()]
        );
    }

    #[test]
    fn assistant_pending_required_check_names_detects_in_progress_checks() {
        let checks = vec![
            WorkflowRunInfo {
                workflow_name: "CI".to_string(),
                run_id: 1,
                status: "in_progress".to_string(),
                conclusion: None,
                is_required: Some(true),
            },
            WorkflowRunInfo {
                workflow_name: "Docs".to_string(),
                run_id: 2,
                status: "queued".to_string(),
                conclusion: None,
                is_required: Some(false),
            },
        ];

        assert_eq!(
            pending_required_check_names(&checks),
            vec!["CI".to_string()]
        );
    }
    #[test]
    fn assistant_recovery_info_detects_resource_guard_failures() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        engine.apply_startup_failure_message(
            r#"LLM call failed: Failed to load model due to insufficient system resources. Adjust guardrails in settings."#,
        );

        let recovery = derive_startup_recovery_info(&engine);

        assert_eq!(recovery.kind.as_deref(), Some("resource_guard"));
        assert!(recovery
            .hints
            .iter()
            .any(|hint| hint.contains("smaller model")));
    }

    #[test]
    fn assistant_recovery_info_detects_ai_not_configured() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        engine.apply_startup_failure_message(
            "AI is not configured. Please configure AI settings first.",
        );

        let recovery = derive_startup_recovery_info(&engine);

        assert_eq!(recovery.kind.as_deref(), Some("ai_not_configured"));
        assert!(recovery
            .hints
            .iter()
            .any(|hint| hint.contains("Open Settings")));
    }
}
