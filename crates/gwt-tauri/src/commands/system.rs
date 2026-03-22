//! Tauri commands for system info and statistics.

use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use gwt_core::{
    config::stats::Stats,
    system_info::{GpuDynamicInfo, GpuStaticInfo},
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tracing::{instrument, warn};

use crate::state::AppState;

const GET_SYSTEM_INFO_WARN_THRESHOLD: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupDiagnostics {
    pub startup_trace: bool,
    pub disable_tray: bool,
    pub disable_login_shell_capture: bool,
    pub disable_heartbeat_watchdog: bool,
    pub disable_session_watcher: bool,
    pub disable_startup_update_check: bool,
    pub disable_profiling: bool,
    pub disable_tab_restore: bool,
    pub disable_window_session_restore: bool,
}

fn parse_env_flag(value: Option<std::ffi::OsString>) -> bool {
    value
        .and_then(|value| value.into_string().ok())
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn startup_diagnostics_from_env() -> StartupDiagnostics {
    StartupDiagnostics {
        startup_trace: parse_env_flag(std::env::var_os("GWT_DIAG_STARTUP_TRACE")),
        disable_tray: parse_env_flag(std::env::var_os("GWT_DIAG_DISABLE_TRAY")),
        disable_login_shell_capture: parse_env_flag(std::env::var_os(
            "GWT_DIAG_DISABLE_LOGIN_SHELL_CAPTURE",
        )),
        disable_heartbeat_watchdog: parse_env_flag(std::env::var_os(
            "GWT_DIAG_DISABLE_HEARTBEAT_WATCHDOG",
        )),
        disable_session_watcher: parse_env_flag(std::env::var_os(
            "GWT_DIAG_DISABLE_SESSION_WATCHER",
        )),
        disable_startup_update_check: parse_env_flag(std::env::var_os(
            "GWT_DIAG_DISABLE_STARTUP_UPDATE_CHECK",
        )),
        disable_profiling: parse_env_flag(std::env::var_os("GWT_DIAG_DISABLE_PROFILING")),
        disable_tab_restore: parse_env_flag(std::env::var_os("GWT_DIAG_DISABLE_TAB_RESTORE")),
        disable_window_session_restore: parse_env_flag(std::env::var_os(
            "GWT_DIAG_DISABLE_WINDOW_SESSION_RESTORE",
        )),
    }
}

// --- T030: SystemInfoResponse / GpuInfo ---

#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_bytes: Option<u64>,
    pub usage_percent: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfoResponse {
    pub cpu_usage_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub gpus: Vec<GpuInfo>,
}

fn normalize_gpu_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn dynamic_to_gpu_info(dynamic: GpuDynamicInfo) -> GpuInfo {
    let fallback_name = "NVIDIA GPU".to_string();
    let name = if dynamic.name.trim().is_empty() {
        fallback_name
    } else {
        dynamic.name
    };
    GpuInfo {
        name,
        vram_total_bytes: Some(dynamic.vram_total_bytes),
        vram_used_bytes: Some(dynamic.vram_used_bytes),
        usage_percent: Some(dynamic.usage_percent),
    }
}

fn build_gpu_infos(
    static_infos: Vec<GpuStaticInfo>,
    dynamic_infos: Vec<GpuDynamicInfo>,
) -> Vec<GpuInfo> {
    let mut gpus = Vec::with_capacity(static_infos.len().max(dynamic_infos.len()));
    let mut remaining_dynamic: Vec<Option<GpuDynamicInfo>> =
        dynamic_infos.into_iter().map(Some).collect();

    for static_info in static_infos {
        let static_key = normalize_gpu_name(&static_info.name);
        let dynamic_match_index = remaining_dynamic.iter().position(|dynamic| {
            dynamic
                .as_ref()
                .map(|info| normalize_gpu_name(&info.name) == static_key)
                .unwrap_or(false)
        });
        let dynamic_match = dynamic_match_index.and_then(|idx| remaining_dynamic[idx].take());

        gpus.push(GpuInfo {
            name: static_info.name,
            vram_total_bytes: dynamic_match
                .as_ref()
                .map(|info| info.vram_total_bytes)
                .or(static_info.vram_total_bytes),
            vram_used_bytes: dynamic_match.as_ref().map(|info| info.vram_used_bytes),
            usage_percent: dynamic_match.as_ref().map(|info| info.usage_percent),
        });
    }

    for dynamic in remaining_dynamic.into_iter().flatten() {
        gpus.push(dynamic_to_gpu_info(dynamic));
    }

    gpus
}

// --- T031: StatsResponse / StatsEntryResponse / AgentStatEntry / RepoStatsEntry ---

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatEntry {
    pub agent_id: String,
    pub model: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsEntryResponse {
    pub agents: Vec<AgentStatEntry>,
    pub worktrees_created: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoStatsEntry {
    pub repo_path: String,
    pub stats: StatsEntryResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsResponse {
    pub global: StatsEntryResponse,
    pub repos: Vec<RepoStatsEntry>,
}

// --- T033: get_system_info command ---

fn get_system_info_impl(state: &AppState) -> SystemInfoResponse {
    let mut monitor = state.system_monitor.lock().unwrap();
    monitor.refresh();
    let cpu = monitor.cpu_usage();
    let (mem_used, mem_total) = monitor.memory_info();
    let gpus = build_gpu_infos(monitor.gpu_static_infos(), monitor.gpu_dynamic_info());
    SystemInfoResponse {
        cpu_usage_percent: cpu,
        memory_used_bytes: mem_used,
        memory_total_bytes: mem_total,
        gpus,
    }
}

#[instrument(skip_all, fields(command = "get_system_info"))]
#[tauri::command]
pub async fn get_system_info(app_handle: AppHandle) -> SystemInfoResponse {
    let started = Instant::now();
    let info = tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        get_system_info_impl(&state)
    })
    .await
    .unwrap_or_else(|_| SystemInfoResponse {
        cpu_usage_percent: 0.0,
        memory_used_bytes: 0,
        memory_total_bytes: 0,
        gpus: Vec::new(),
    });
    let elapsed = started.elapsed();
    if elapsed > GET_SYSTEM_INFO_WARN_THRESHOLD {
        warn!(
            category = "system",
            elapsed_ms = elapsed.as_millis(),
            "get_system_info took longer than expected"
        );
    }
    info
}

// --- T034: get_stats command ---

/// Convert a `HashMap<String, u64>` agent map to `Vec<AgentStatEntry>`.
///
/// Keys are `"{agent_id}.{model}"`. Split on the first `.` only so that
/// agent IDs containing dots (e.g. "claude-code") work correctly.
fn agents_map_to_vec(agents: &std::collections::HashMap<String, u64>) -> Vec<AgentStatEntry> {
    let mut result: Vec<AgentStatEntry> = agents
        .iter()
        .map(|(key, &count)| {
            let (agent_id, model) = match key.find('.') {
                Some(pos) => (key[..pos].to_string(), key[pos + 1..].to_string()),
                None => (key.clone(), "default".to_string()),
            };
            AgentStatEntry {
                agent_id,
                model,
                count,
            }
        })
        .collect();
    result.sort_by(|a, b| b.count.cmp(&a.count));
    result
}

fn stats_entry_to_response(entry: &gwt_core::config::stats::StatsEntry) -> StatsEntryResponse {
    StatsEntryResponse {
        agents: agents_map_to_vec(&entry.agents),
        worktrees_created: entry.worktrees_created,
    }
}

#[instrument(skip_all, fields(command = "get_stats"))]
#[tauri::command]
pub fn get_stats() -> StatsResponse {
    let stats = Stats::load().unwrap_or_default();

    let mut repos: Vec<RepoStatsEntry> = stats
        .repos
        .iter()
        .map(|(path, entry)| RepoStatsEntry {
            repo_path: path.clone(),
            stats: stats_entry_to_response(entry),
        })
        .collect();
    repos.sort_by(|a, b| a.repo_path.cmp(&b.repo_path));

    StatsResponse {
        global: stats_entry_to_response(&stats.global),
        repos,
    }
}

#[instrument(skip_all, fields(command = "get_startup_diagnostics"))]
#[tauri::command]
pub fn get_startup_diagnostics() -> StartupDiagnostics {
    startup_diagnostics_from_env()
}

// --- Freeze detection: heartbeat + frontend metrics ---

#[instrument(skip_all, fields(command = "heartbeat"))]
#[tauri::command]
pub fn heartbeat(state: tauri::State<'_, AppState>, app_handle: AppHandle) {
    if let Ok(mut slot) = state.last_heartbeat.lock() {
        *slot = Some(Instant::now());
    }

    if startup_diagnostics_from_env().disable_heartbeat_watchdog {
        return;
    }

    if state
        .heartbeat_watchdog_started
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                let state = app_handle.state::<AppState>();
                let elapsed = {
                    let slot = state.last_heartbeat.lock().ok();
                    slot.and_then(|guard| guard.map(|ts| ts.elapsed()))
                };
                if let Some(elapsed) = elapsed {
                    if elapsed > Duration::from_secs(3) {
                        warn!(
                            category = "freeze_detection",
                            elapsed_ms = elapsed.as_millis(),
                            "Frontend heartbeat stale – possible UI freeze"
                        );
                        let _ = app_handle.emit("freeze-detected", elapsed.as_millis() as u64);
                    }
                }
            }
        });

        tracing::info!(
            category = "startup_diag",
            checkpoint = "heartbeat_watchdog.ready",
            "Frontend heartbeat watchdog armed after first heartbeat"
        );
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendMetric {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub duration_ms: f64,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub startup_token: Option<String>,
    #[serde(default)]
    pub success: Option<bool>,
}

#[instrument(skip_all, fields(command = "report_frontend_metrics"))]
#[tauri::command]
pub fn report_frontend_metrics(metrics: Vec<FrontendMetric>) {
    for m in &metrics {
        tracing::info!(
            target: "frontend",
            kind = %m.kind.as_deref().unwrap_or("invoke"),
            command = %m.command.as_deref().unwrap_or(""),
            name = %m.name.as_deref().unwrap_or(""),
            duration_ms = m.duration_ms,
            timestamp = m.timestamp.unwrap_or_default(),
            startup_token = %m.startup_token.as_deref().unwrap_or(""),
            success = m.success.unwrap_or(true),
            "Frontend invoke metric"
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn build_gpu_infos_returns_dynamic_payload_without_static_info() {
        let gpus = build_gpu_infos(
            vec![],
            vec![GpuDynamicInfo {
                name: "NVIDIA RTX 4090".to_string(),
                usage_percent: 71.0,
                vram_used_bytes: 1024,
                vram_total_bytes: 2048,
            }],
        );

        assert_eq!(gpus.len(), 1);
        let gpu = &gpus[0];
        assert_eq!(gpu.name, "NVIDIA RTX 4090");
        assert_eq!(gpu.vram_total_bytes, Some(2048));
        assert_eq!(gpu.vram_used_bytes, Some(1024));
        assert_eq!(gpu.usage_percent, Some(71.0));
    }

    #[test]
    fn build_gpu_infos_merges_dynamic_values_into_matching_static_entry() {
        let gpus = build_gpu_infos(
            vec![GpuStaticInfo {
                name: "NVIDIA GeForce RTX 4090".to_string(),
                vram_total_bytes: Some(4096),
            }],
            vec![GpuDynamicInfo {
                name: "NVIDIA GeForce RTX 4090".to_string(),
                usage_percent: 33.0,
                vram_used_bytes: 2048,
                vram_total_bytes: 24564,
            }],
        );

        assert_eq!(gpus.len(), 1);
        let gpu = &gpus[0];
        assert_eq!(gpu.name, "NVIDIA GeForce RTX 4090");
        // Prefer NVML total VRAM over potentially stale static values.
        assert_eq!(gpu.vram_total_bytes, Some(24564));
        assert_eq!(gpu.vram_used_bytes, Some(2048));
        assert_eq!(gpu.usage_percent, Some(33.0));
    }

    #[test]
    fn build_gpu_infos_keeps_static_entries_without_dynamic_metrics() {
        let gpus = build_gpu_infos(
            vec![GpuStaticInfo {
                name: "Intel(R) UHD Graphics".to_string(),
                vram_total_bytes: Some(1073741824),
            }],
            vec![],
        );

        assert_eq!(gpus.len(), 1);
        let gpu = &gpus[0];
        assert_eq!(gpu.name, "Intel(R) UHD Graphics");
        assert_eq!(gpu.vram_total_bytes, Some(1073741824));
        assert_eq!(gpu.vram_used_bytes, None);
        assert_eq!(gpu.usage_percent, None);
    }

    #[test]
    fn build_gpu_infos_returns_empty_without_any_gpu_data() {
        assert!(build_gpu_infos(vec![], vec![]).is_empty());
    }

    #[test]
    fn build_gpu_infos_preserves_unmatched_dynamic_entries() {
        let gpus = build_gpu_infos(
            vec![GpuStaticInfo {
                name: "Intel(R) UHD Graphics".to_string(),
                vram_total_bytes: Some(1073741824),
            }],
            vec![GpuDynamicInfo {
                name: "NVIDIA GeForce RTX 4090".to_string(),
                usage_percent: 50.0,
                vram_used_bytes: 4096,
                vram_total_bytes: 24564,
            }],
        );

        assert_eq!(gpus.len(), 2);
        let intel = &gpus[0];
        let nvidia = &gpus[1];
        assert_eq!(intel.name, "Intel(R) UHD Graphics");
        assert_eq!(intel.usage_percent, None);
        assert_eq!(nvidia.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(nvidia.vram_total_bytes, Some(24564));
        assert_eq!(nvidia.vram_used_bytes, Some(4096));
        assert_eq!(nvidia.usage_percent, Some(50.0));
    }

    #[test]
    fn build_gpu_infos_uses_fallback_name_when_dynamic_name_missing() {
        let gpus = build_gpu_infos(
            vec![],
            vec![GpuDynamicInfo {
                name: "   ".to_string(),
                usage_percent: 11.0,
                vram_used_bytes: 128,
                vram_total_bytes: 256,
            }],
        );

        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "NVIDIA GPU");
    }

    #[test]
    fn parse_env_flag_accepts_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "on"] {
            assert!(parse_env_flag(Some(std::ffi::OsString::from(value))));
        }
    }

    #[test]
    fn parse_env_flag_rejects_falsey_values() {
        assert!(!parse_env_flag(None));
        for value in ["0", "false", "FALSE", "no", "off", ""] {
            assert!(!parse_env_flag(Some(std::ffi::OsString::from(value))));
        }
    }

    #[test]
    fn frontend_metric_accepts_legacy_invoke_shape() {
        let metric: FrontendMetric = serde_json::from_value(json!({
            "command": "open_project",
            "durationMs": 12.5
        }))
        .expect("legacy invoke metric should deserialize");

        assert_eq!(metric.command.as_deref(), Some("open_project"));
        assert_eq!(metric.kind.as_deref(), None);
        assert_eq!(metric.name.as_deref(), None);
        assert_eq!(metric.duration_ms, 12.5);
    }

    #[test]
    fn frontend_metric_accepts_startup_shape() {
        let metric: FrontendMetric = serde_json::from_value(json!({
            "kind": "startup",
            "name": "project_start.open_project.total",
            "durationMs": 432.1,
            "timestamp": 1711111111u64,
            "startupToken": "startup-1",
            "success": true
        }))
        .expect("startup metric should deserialize");

        assert_eq!(metric.kind.as_deref(), Some("startup"));
        assert_eq!(
            metric.name.as_deref(),
            Some("project_start.open_project.total")
        );
        assert_eq!(metric.startup_token.as_deref(), Some("startup-1"));
        assert_eq!(metric.success, Some(true));
        assert_eq!(metric.duration_ms, 432.1);
    }
}
