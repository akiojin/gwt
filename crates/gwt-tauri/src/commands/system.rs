//! Tauri commands for system info and statistics.

use crate::state::AppState;
use gwt_core::config::stats::Stats;
use gwt_core::system_info::{GpuDynamicInfo, GpuStaticInfo};
use serde::Serialize;
use tauri::State;

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
    pub gpu: Option<GpuInfo>,
}

fn build_gpu_info(
    static_info: Option<GpuStaticInfo>,
    dynamic_info: Option<GpuDynamicInfo>,
) -> Option<GpuInfo> {
    match (static_info, dynamic_info) {
        (None, None) => None,
        (static_info, dynamic_info) => Some(GpuInfo {
            name: static_info
                .as_ref()
                .map(|info| info.name.clone())
                .unwrap_or_else(|| "NVIDIA GPU".to_string()),
            vram_total_bytes: static_info
                .as_ref()
                .and_then(|info| info.vram_total_bytes)
                .or(dynamic_info.as_ref().map(|info| info.vram_total_bytes)),
            vram_used_bytes: dynamic_info.as_ref().map(|info| info.vram_used_bytes),
            usage_percent: dynamic_info.as_ref().map(|info| info.usage_percent),
        }),
    }
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

#[tauri::command]
pub fn get_system_info(state: State<'_, AppState>) -> SystemInfoResponse {
    let mut monitor = state.system_monitor.lock().unwrap();
    monitor.refresh();
    let cpu = monitor.cpu_usage();
    let (mem_used, mem_total) = monitor.memory_info();
    let gpu = build_gpu_info(monitor.gpu_static_info(), monitor.gpu_dynamic_info());
    SystemInfoResponse {
        cpu_usage_percent: cpu,
        memory_used_bytes: mem_used,
        memory_total_bytes: mem_total,
        gpu,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_gpu_info_returns_dynamic_payload_without_static_info() {
        let gpu = build_gpu_info(
            None,
            Some(GpuDynamicInfo {
                usage_percent: 71.0,
                vram_used_bytes: 1024,
                vram_total_bytes: 2048,
            }),
        )
        .expect("dynamic GPU info should produce payload");

        assert_eq!(gpu.name, "NVIDIA GPU");
        assert_eq!(gpu.vram_total_bytes, Some(2048));
        assert_eq!(gpu.vram_used_bytes, Some(1024));
        assert_eq!(gpu.usage_percent, Some(71.0));
    }

    #[test]
    fn build_gpu_info_returns_none_without_any_gpu_data() {
        assert!(build_gpu_info(None, None).is_none());
    }
}
