use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU16},
        Arc, Mutex, RwLock,
    },
};

use gwt_core::{
    agent::SessionStore,
    ai::SessionSummaryCache,
    config::{os_env::EnvSource, SkillRegistrationStatus},
    terminal::manager::PaneManager,
    update::UpdateManager,
};
use tokio::sync::Semaphore;

use crate::ws::EventBroadcaster;

/// Shared application state for the gwt-server process.
///
/// This is a Tauri-free version of the state previously managed by
/// `tauri::State<AppState>`. All fields that referenced Tauri types
/// (AppHandle, emit, etc.) have been replaced with the EventBroadcaster.
pub struct AppState {
    pub system_monitor: Mutex<gwt_core::system_info::SystemMonitor>,
    pub window_projects: Mutex<HashMap<String, String>>,
    pub pane_manager: Mutex<PaneManager>,
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub session_summary_cache: Mutex<HashMap<String, SessionSummaryCache>>,
    pub session_summary_inflight: Mutex<HashSet<String>>,
    pub session_summary_rebuild_inflight: Mutex<HashSet<String>>,
    pub project_version_history_cache:
        Mutex<HashMap<String, HashMap<String, VersionHistoryCacheEntry>>>,
    pub project_version_history_inflight: Mutex<HashSet<String>>,
    pub project_issue_list_cache: Mutex<HashMap<String, HashMap<String, IssueListCacheEntry>>>,
    pub project_issue_list_inflight: Mutex<HashSet<String>>,
    pub project_branch_inventory_snapshot_cache:
        Mutex<HashMap<String, BranchInventorySnapshotCacheEntry>>,
    pub project_branch_inventory_snapshot_inflight: Mutex<HashSet<String>>,
    pub version_history_semaphore: Arc<Semaphore>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    pub pane_runtime_contexts: Mutex<HashMap<String, PaneRuntimeContext>>,
    pub launch_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub launch_results: Mutex<HashMap<String, serde_json::Value>>,
    pub is_quitting: AtomicBool,
    pub os_env: Arc<RwLock<HashMap<String, String>>>,
    pub os_env_source: Arc<RwLock<EnvSource>>,
    pub os_env_ready: Arc<AtomicBool>,
    pub os_env_capture_inflight: Arc<AtomicBool>,
    pub skill_registration_status: Arc<Mutex<SkillRegistrationStatus>>,
    pub update_manager: UpdateManager,
    pub gh_available: AtomicBool,
    pub session_store: SessionStore,
    pub last_heartbeat: Mutex<Option<std::time::Instant>>,
    pub http_port: AtomicU16,

    /// Event broadcaster for WebSocket clients (replaces Tauri emit).
    pub broadcaster: EventBroadcaster,
}

impl AppState {
    pub fn new(broadcaster: EventBroadcaster) -> Self {
        let initial_env = std::env::vars().collect();
        Self {
            system_monitor: Mutex::new(gwt_core::system_info::SystemMonitor::new()),
            window_projects: Mutex::new(HashMap::new()),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            session_summary_cache: Mutex::new(HashMap::new()),
            session_summary_inflight: Mutex::new(HashSet::new()),
            session_summary_rebuild_inflight: Mutex::new(HashSet::new()),
            project_version_history_cache: Mutex::new(HashMap::new()),
            project_version_history_inflight: Mutex::new(HashSet::new()),
            project_issue_list_cache: Mutex::new(HashMap::new()),
            project_issue_list_inflight: Mutex::new(HashSet::new()),
            project_branch_inventory_snapshot_cache: Mutex::new(HashMap::new()),
            project_branch_inventory_snapshot_inflight: Mutex::new(HashSet::new()),
            version_history_semaphore: Arc::new(Semaphore::new(3)),
            pane_launch_meta: Mutex::new(HashMap::new()),
            pane_runtime_contexts: Mutex::new(HashMap::new()),
            launch_jobs: Mutex::new(HashMap::new()),
            launch_results: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
            os_env: Arc::new(RwLock::new(initial_env)),
            os_env_source: Arc::new(RwLock::new(EnvSource::ProcessEnv)),
            os_env_ready: Arc::new(AtomicBool::new(true)),
            os_env_capture_inflight: Arc::new(AtomicBool::new(false)),
            skill_registration_status: Arc::new(Mutex::new(SkillRegistrationStatus::default())),
            update_manager: UpdateManager::new(),
            gh_available: AtomicBool::new(false),
            session_store: SessionStore::new(),
            last_heartbeat: Mutex::new(None),
            http_port: AtomicU16::new(0),
            broadcaster,
        }
    }
}

// Re-export helper types used by handlers.

#[derive(Debug, Clone)]
pub struct AgentVersionsCache {
    pub tags: Vec<String>,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PaneLaunchMeta {
    pub agent_id: String,
    pub branch: String,
    pub repo_path: std::path::PathBuf,
    pub worktree_path: std::path::PathBuf,
    pub tool_label: String,
    pub tool_version: String,
    pub mode: String,
    pub model: Option<String>,
    pub started_at_millis: i64,
}

#[derive(Debug, Clone)]
pub struct PaneRuntimeContext {
    pub launch_workdir: std::path::PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionHistoryCacheEntry {
    pub label: String,
    pub range_from: Option<String>,
    pub range_to: String,
    pub commit_count: u32,
    pub language: String,
    pub summary_markdown: String,
    pub changelog_markdown: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IssueListCacheEntry {
    pub fetched_at_millis: i64,
    pub response_json: String,
}

#[derive(Debug, Clone)]
pub struct BranchInventorySnapshotCacheEntry {
    pub refresh_key: u64,
    pub entries: Vec<serde_json::Value>,
}
