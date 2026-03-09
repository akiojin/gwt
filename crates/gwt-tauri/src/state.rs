use crate::agent_master::ProjectModeState;
use gwt_core::agent::SessionStore;
use gwt_core::ai::SessionSummaryCache;
use gwt_core::config::os_env::EnvSource;
use gwt_core::config::SkillRegistrationStatus;
use gwt_core::terminal::manager::PaneManager;
use gwt_core::update::UpdateManager;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct AgentVersionsCache {
    pub tags: Vec<String>,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PaneLaunchMeta {
    pub agent_id: String,
    pub branch: String,
    pub repo_path: PathBuf,
    pub worktree_path: PathBuf,
    pub tool_label: String,
    pub tool_version: String,
    pub mode: String,
    pub model: Option<String>,
    pub reasoning_level: Option<String>,
    pub skip_permissions: bool,
    pub collaboration_modes: bool,
    pub docker_service: Option<String>,
    pub docker_force_host: Option<bool>,
    pub docker_recreate: Option<bool>,
    pub docker_build: Option<bool>,
    pub docker_keep: Option<bool>,
    pub docker_container_name: Option<String>,
    pub docker_compose_args: Option<Vec<String>>,
    pub started_at_millis: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionHistoryCacheEntry {
    pub label: String,
    pub range_from: Option<String>,
    pub range_to: String,
    pub range_from_oid: Option<String>,
    pub range_to_oid: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTabMenuState {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowAgentTabsState {
    pub tabs: Vec<AgentTabMenuState>,
    pub active_tab_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMigrationState {
    pub job_id: String,
    pub source_root: String,
}

const WINDOW_SESSION_RESTORE_LEAD_TTL_MS: u64 = 15_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSessionRestoreLeaderState {
    pub label: String,
    pub expires_at_millis: u64,
}

pub struct AppState {
    /// System resource monitor for CPU/memory/GPU queries.
    pub system_monitor: Mutex<gwt_core::system_info::SystemMonitor>,
    /// Project root path per window label.
    ///
    /// Only stores windows that currently have a project opened.
    pub window_projects: Mutex<HashMap<String, String>>,
    /// Canonical project identity per window label (used for duplicate project detection).
    pub window_project_identities: Mutex<HashMap<String, String>>,
    /// One-shot permission to allow a window to actually close (instead of always hiding it).
    /// Used by close-event flows that explicitly permit destruction.
    pub windows_allowed_to_close: Mutex<HashSet<String>>,
    /// Agent tab state per window label for native Window menu rendering.
    pub window_agent_tabs: Mutex<HashMap<String, WindowAgentTabsState>>,
    /// Migration status per window label for native Window menu rendering.
    pub window_migrations: Mutex<HashMap<String, WindowMigrationState>>,
    /// Project Mode conversation state per window label.
    pub window_project_modes: Mutex<HashMap<String, ProjectModeState>>,
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
    /// Semaphore to limit concurrent AI summary generation (max 3).
    pub version_history_semaphore: Arc<Semaphore>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    /// Single-process leader lock for startup multi-window restore.
    pub window_session_restore_leader: Mutex<Option<WindowSessionRestoreLeaderState>>,
    /// Launch job cancellation flags keyed by job id.
    pub launch_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Completed launch results stored for polling retrieval (fallback when
    /// Tauri events are lost).
    pub launch_results: Mutex<HashMap<String, crate::commands::terminal::LaunchFinishedPayload>>,
    pub is_quitting: AtomicBool,
    /// Timestamp of the first Cmd+Q press for "Press ⌘Q again to quit" flow.
    pub quit_confirm_requested_at: Mutex<Option<Instant>>,
    /// Prevent multiple exit confirmation dialogs from showing at once.
    #[cfg(not(test))]
    pub exit_confirm_inflight: AtomicBool,
    pub os_env: Arc<RwLock<HashMap<String, String>>>,
    pub os_env_source: Arc<RwLock<EnvSource>>,
    pub os_env_ready: Arc<AtomicBool>,
    pub os_env_capture_inflight: Arc<AtomicBool>,
    /// Last observed skill registration health snapshot.
    pub skill_registration_status: Arc<Mutex<SkillRegistrationStatus>>,
    pub update_manager: UpdateManager,
    /// Whether `gh` CLI is authenticated (gwt-spec issue T009).
    pub gh_available: AtomicBool,
    /// MRU (most-recently-used) window focus history. Front = most recent.
    pub window_focus_history: Mutex<Vec<String>>,
    /// Persistent storage for agent sessions (Branch Mode & Project Mode).
    pub session_store: SessionStore,
}

impl AppState {
    pub fn new() -> Self {
        let initial_env = std::env::vars().collect();
        Self {
            system_monitor: Mutex::new(gwt_core::system_info::SystemMonitor::new()),
            window_projects: Mutex::new(HashMap::new()),
            window_project_identities: Mutex::new(HashMap::new()),
            windows_allowed_to_close: Mutex::new(HashSet::new()),
            window_agent_tabs: Mutex::new(HashMap::new()),
            window_migrations: Mutex::new(HashMap::new()),
            window_project_modes: Mutex::new(HashMap::new()),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            session_summary_cache: Mutex::new(HashMap::new()),
            session_summary_inflight: Mutex::new(HashSet::new()),
            session_summary_rebuild_inflight: Mutex::new(HashSet::new()),
            project_version_history_cache: Mutex::new(HashMap::new()),
            project_version_history_inflight: Mutex::new(HashSet::new()),
            project_issue_list_cache: Mutex::new(HashMap::new()),
            project_issue_list_inflight: Mutex::new(HashSet::new()),
            version_history_semaphore: Arc::new(Semaphore::new(3)),
            pane_launch_meta: Mutex::new(HashMap::new()),
            window_session_restore_leader: Mutex::new(None),
            launch_jobs: Mutex::new(HashMap::new()),
            launch_results: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
            quit_confirm_requested_at: Mutex::new(None),
            #[cfg(not(test))]
            exit_confirm_inflight: AtomicBool::new(false),
            os_env: Arc::new(RwLock::new(initial_env)),
            os_env_source: Arc::new(RwLock::new(EnvSource::ProcessEnv)),
            os_env_ready: Arc::new(AtomicBool::new(true)),
            os_env_capture_inflight: Arc::new(AtomicBool::new(false)),
            skill_registration_status: Arc::new(Mutex::new(SkillRegistrationStatus::default())),
            update_manager: UpdateManager::new(),
            gh_available: AtomicBool::new(false),
            window_focus_history: Mutex::new(Vec::new()),
            session_store: SessionStore::new(),
        }
    }

    /// Whether OS environment capture has completed.
    pub fn is_os_env_ready(&self) -> bool {
        self.os_env_ready.load(Ordering::SeqCst)
    }

    /// Wait briefly for OS environment capture to complete.
    ///
    /// This avoids non-deterministic launches when the UI requests a session before
    /// the startup capture task finishes.
    pub fn wait_os_env_ready(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while !self.is_os_env_ready() && start.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.is_os_env_ready()
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn begin_os_env_capture(&self) -> bool {
        let started = self
            .os_env_capture_inflight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if !started {
            return false;
        }
        self.os_env_ready.store(false, Ordering::SeqCst);
        true
    }

    pub fn set_os_env_snapshot(&self, env: HashMap<String, String>, source: EnvSource) {
        if let Ok(mut slot) = self.os_env.write() {
            *slot = env;
        }
        if let Ok(mut source_slot) = self.os_env_source.write() {
            *source_slot = source;
        }
        self.os_env_ready.store(true, Ordering::SeqCst);
        self.os_env_capture_inflight.store(false, Ordering::SeqCst);
    }

    /// Used on Windows to set OS environment from process env.
    #[allow(dead_code)]
    pub fn set_os_env_process_env_snapshot(&self) {
        self.set_os_env_snapshot(std::env::vars().collect(), EnvSource::ProcessEnv);
    }

    pub fn os_env_snapshot(&self) -> HashMap<String, String> {
        self.os_env
            .read()
            .map(|env| env.clone())
            .unwrap_or_default()
    }

    pub fn os_env_source_snapshot(&self) -> Option<EnvSource> {
        self.os_env_source.read().ok().map(|source| source.clone())
    }

    /// Atomically claim a project identity for a window and persist its project path.
    ///
    /// Returns `Err(existing_window_label)` when another window already owns the
    /// same project identity.
    pub fn claim_project_for_window_with_identity(
        &self,
        window_label: &str,
        project_path: String,
        project_identity: String,
    ) -> Result<(), String> {
        let Ok(mut projects) = self.window_projects.lock() else {
            return Ok(());
        };
        let Ok(mut identities) = self.window_project_identities.lock() else {
            projects.insert(window_label.to_string(), project_path);
            return Ok(());
        };

        if let Some(existing_window_label) = identities.iter().find_map(|(label, identity)| {
            if identity == &project_identity && label.as_str() != window_label {
                Some(label.clone())
            } else {
                None
            }
        }) {
            return Err(existing_window_label);
        }

        projects.insert(window_label.to_string(), project_path);
        identities.insert(window_label.to_string(), project_identity);
        Ok(())
    }

    pub fn clear_project_for_window(&self, window_label: &str) {
        if let Ok(mut map) = self.window_projects.lock() {
            map.remove(window_label);
        }
        if let Ok(mut map) = self.window_project_identities.lock() {
            map.remove(window_label);
        }
        if let Ok(mut map) = self.window_agent_tabs.lock() {
            map.remove(window_label);
        }
        if let Ok(mut map) = self.window_project_modes.lock() {
            map.remove(window_label);
        }
        self.remove_window_from_history(window_label);
    }

    pub fn clear_window_state(&self, window_label: &str) {
        self.clear_project_for_window(window_label);
        if let Ok(mut map) = self.window_migrations.lock() {
            map.remove(window_label);
        }
    }

    pub fn set_skill_registration_status(&self, status: SkillRegistrationStatus) {
        if let Ok(mut slot) = self.skill_registration_status.lock() {
            *slot = status;
        }
    }

    pub fn get_skill_registration_status(&self) -> SkillRegistrationStatus {
        self.skill_registration_status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn project_for_window(&self, window_label: &str) -> Option<String> {
        let map = self.window_projects.lock().ok()?;
        map.get(window_label).cloned()
    }

    fn now_millis() -> u64 {
        let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return 0;
        };
        duration.as_millis() as u64
    }

    pub fn try_acquire_window_session_restore_leader(&self, label: &str) -> bool {
        self.try_acquire_window_session_restore_leader_at(label, Self::now_millis())
    }

    fn try_acquire_window_session_restore_leader_at(&self, label: &str, now_millis: u64) -> bool {
        let normalized_label = label.trim();
        if normalized_label != "main" {
            return false;
        }

        let Ok(mut slot) = self.window_session_restore_leader.lock() else {
            return false;
        };

        if let Some(existing) = slot.as_ref() {
            if existing.label != normalized_label && existing.expires_at_millis > now_millis {
                return false;
            }
        }

        *slot = Some(WindowSessionRestoreLeaderState {
            label: normalized_label.to_string(),
            expires_at_millis: now_millis + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
        });
        true
    }

    pub fn release_window_session_restore_leader(&self, label: &str) {
        let normalized_label = label.trim();
        if normalized_label.is_empty() {
            return;
        }

        let Ok(mut slot) = self.window_session_restore_leader.lock() else {
            return;
        };
        let should_clear = slot
            .as_ref()
            .map(|existing| existing.label == normalized_label)
            .unwrap_or(false);
        if should_clear {
            *slot = None;
        }
    }

    pub fn set_window_agent_tabs(
        &self,
        window_label: &str,
        tabs: Vec<AgentTabMenuState>,
        active_tab_id: Option<String>,
    ) {
        let normalized_active = active_tab_id.filter(|id| tabs.iter().any(|t| &t.id == id));
        if let Ok(mut map) = self.window_agent_tabs.lock() {
            map.insert(
                window_label.to_string(),
                WindowAgentTabsState {
                    tabs,
                    active_tab_id: normalized_active,
                },
            );
        }
    }

    pub fn set_window_agent_active_tab(&self, window_label: &str, active_tab_id: Option<String>) {
        if let Ok(mut map) = self.window_agent_tabs.lock() {
            let Some(state) = map.get_mut(window_label) else {
                return;
            };
            let normalized_active = active_tab_id
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .filter(|id| state.tabs.iter().any(|tab| tab.id == *id));
            state.active_tab_id = normalized_active;
        }
    }

    pub fn window_agent_tabs_for_window(&self, window_label: &str) -> WindowAgentTabsState {
        let map = match self.window_agent_tabs.lock() {
            Ok(m) => m,
            Err(_) => return WindowAgentTabsState::default(),
        };
        map.get(window_label).cloned().unwrap_or_default()
    }

    pub fn set_window_migration(&self, window_label: &str, job_id: String, source_root: String) {
        if let Ok(mut map) = self.window_migrations.lock() {
            map.insert(
                window_label.to_string(),
                WindowMigrationState {
                    job_id,
                    source_root,
                },
            );
        }
    }

    pub fn clear_window_migration_if_job(&self, window_label: &str, job_id: &str) {
        if let Ok(mut map) = self.window_migrations.lock() {
            let remove = map
                .get(window_label)
                .map(|migration| migration.job_id == job_id)
                .unwrap_or(false);
            if remove {
                map.remove(window_label);
            }
        }
    }

    pub fn window_migrations_snapshot(&self) -> HashMap<String, WindowMigrationState> {
        self.window_migrations
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn allow_window_close(&self, window_label: &str) {
        if let Ok(mut set) = self.windows_allowed_to_close.lock() {
            set.insert(window_label.to_string());
        }
    }

    pub fn consume_window_close_permission(&self, window_label: &str) -> bool {
        if let Ok(mut set) = self.windows_allowed_to_close.lock() {
            return set.remove(window_label);
        }
        false
    }

    pub fn request_quit(&self) {
        self.is_quitting.store(true, Ordering::SeqCst);
    }

    /// Begin the quit-confirm window ("Press ⌘Q again to quit").
    ///
    /// Returns `true` if this call arms a new confirmation window, `false` if
    /// the existing confirmation is still active within `timeout`.
    pub fn begin_quit_confirm(&self, timeout: Duration) -> bool {
        let Ok(mut slot) = self.quit_confirm_requested_at.lock() else {
            return false;
        };
        if let Some(requested_at) = *slot {
            if requested_at.elapsed() < timeout {
                return false;
            }
        }
        *slot = Some(Instant::now());
        true
    }

    /// Returns `true` if quit confirm was started within `timeout` duration.
    pub fn is_quit_confirm_active(&self, timeout: Duration) -> bool {
        let Ok(slot) = self.quit_confirm_requested_at.lock() else {
            return false;
        };
        match *slot {
            Some(requested_at) => requested_at.elapsed() < timeout,
            None => false,
        }
    }

    /// Reset the quit-confirm state (clears the timestamp).
    pub fn cancel_quit_confirm(&self) {
        if let Ok(mut slot) = self.quit_confirm_requested_at.lock() {
            *slot = None;
        }
    }

    /// Push window to front of MRU list. If already present, move to front.
    pub fn push_window_focus(&self, label: &str) {
        if let Ok(mut history) = self.window_focus_history.lock() {
            history.retain(|l| l != label);
            history.insert(0, label.to_string());
        }
    }

    /// Get next window in rotation order (rotate list left).
    /// Returns None if only one or zero windows.
    pub fn next_window(&self) -> Option<String> {
        let mut history = self.window_focus_history.lock().ok()?;
        if history.len() <= 1 {
            return None;
        }
        let front = history.remove(0);
        history.push(front);
        Some(history[0].clone())
    }

    /// Get previous window in rotation order (rotate list right).
    /// Returns None if only one or zero windows.
    pub fn previous_window(&self) -> Option<String> {
        let mut history = self.window_focus_history.lock().ok()?;
        if history.len() <= 1 {
            return None;
        }
        let back = history.pop().unwrap();
        history.insert(0, back.clone());
        Some(back)
    }

    /// Get the most recently focused window without rotating history.
    pub fn most_recent_window(&self) -> Option<String> {
        let history = self.window_focus_history.lock().ok()?;
        history.first().cloned()
    }

    /// Get the least recently focused window without rotating history.
    pub fn least_recent_window(&self) -> Option<String> {
        let history = self.window_focus_history.lock().ok()?;
        history.last().cloned()
    }

    /// Remove window from MRU history (on window destroy).
    pub fn remove_window_from_history(&self, label: &str) {
        if let Ok(mut history) = self.window_focus_history.lock() {
            history.retain(|l| l != label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_history_semaphore_has_three_permits() {
        let state = AppState::new();
        // The semaphore should allow exactly 3 concurrent permits.
        let p1 = state.version_history_semaphore.try_acquire();
        assert!(p1.is_ok(), "1st permit should succeed");
        let p2 = state.version_history_semaphore.try_acquire();
        assert!(p2.is_ok(), "2nd permit should succeed");
        let p3 = state.version_history_semaphore.try_acquire();
        assert!(p3.is_ok(), "3rd permit should succeed");
        let p4 = state.version_history_semaphore.try_acquire();
        assert!(p4.is_err(), "4th permit should fail (max 3)");
    }

    #[test]
    fn window_projects_set_get_clear() {
        let state = AppState::new();
        assert_eq!(state.project_for_window("main"), None);

        let claim = state.claim_project_for_window_with_identity(
            "main",
            "/tmp/repo".to_string(),
            "/tmp/repo-canonical".to_string(),
        );
        assert_eq!(claim, Ok(()));
        assert_eq!(
            state.project_for_window("main"),
            Some("/tmp/repo".to_string())
        );

        state.clear_project_for_window("main");
        assert_eq!(state.project_for_window("main"), None);
    }

    #[test]
    fn claim_project_for_window_with_identity_rejects_duplicates() {
        let state = AppState::new();

        let first = state.claim_project_for_window_with_identity(
            "main",
            "/tmp/repo".to_string(),
            "/tmp/repo-canonical".to_string(),
        );
        assert_eq!(first, Ok(()));

        let second = state.claim_project_for_window_with_identity(
            "project-2",
            "/tmp/repo".to_string(),
            "/tmp/repo-canonical".to_string(),
        );
        assert_eq!(second, Err("main".to_string()));

        state.clear_project_for_window("main");
        let third = state.claim_project_for_window_with_identity(
            "project-2",
            "/tmp/repo".to_string(),
            "/tmp/repo-canonical".to_string(),
        );
        assert_eq!(third, Ok(()));
    }

    #[test]
    fn window_close_permission_is_one_shot() {
        let state = AppState::new();
        assert!(!state.consume_window_close_permission("main"));

        state.allow_window_close("main");
        assert!(state.consume_window_close_permission("main"));
        assert!(!state.consume_window_close_permission("main"));
    }

    #[test]
    fn window_agent_tabs_set_get_clear() {
        let state = AppState::new();
        assert_eq!(
            state.window_agent_tabs_for_window("main"),
            WindowAgentTabsState::default()
        );

        state.set_window_agent_tabs(
            "main",
            vec![
                AgentTabMenuState {
                    id: "agent-pane-1".to_string(),
                    label: "feature/one".to_string(),
                },
                AgentTabMenuState {
                    id: "agent-pane-2".to_string(),
                    label: "feature/two".to_string(),
                },
            ],
            Some("agent-pane-2".to_string()),
        );

        let tabs = state.window_agent_tabs_for_window("main");
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(tabs.active_tab_id, Some("agent-pane-2".to_string()));

        state.clear_project_for_window("main");
        assert_eq!(
            state.window_agent_tabs_for_window("main"),
            WindowAgentTabsState::default()
        );
    }

    #[test]
    fn window_agent_tabs_active_is_cleared_when_missing() {
        let state = AppState::new();
        state.set_window_agent_tabs(
            "main",
            vec![AgentTabMenuState {
                id: "agent-pane-1".to_string(),
                label: "feature/one".to_string(),
            }],
            Some("agent-pane-999".to_string()),
        );

        let tabs = state.window_agent_tabs_for_window("main");
        assert_eq!(tabs.active_tab_id, None);
    }

    #[test]
    fn window_agent_tabs_active_can_be_updated_without_replacing_tabs() {
        let state = AppState::new();
        state.set_window_agent_tabs(
            "main",
            vec![
                AgentTabMenuState {
                    id: "agent-pane-1".to_string(),
                    label: "feature/one".to_string(),
                },
                AgentTabMenuState {
                    id: "agent-pane-2".to_string(),
                    label: "feature/two".to_string(),
                },
            ],
            Some("agent-pane-1".to_string()),
        );

        state.set_window_agent_active_tab("main", Some("agent-pane-2".to_string()));

        let tabs = state.window_agent_tabs_for_window("main");
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(tabs.active_tab_id, Some("agent-pane-2".to_string()));
    }

    #[test]
    fn window_agent_tabs_active_update_ignores_unknown_id() {
        let state = AppState::new();
        state.set_window_agent_tabs(
            "main",
            vec![AgentTabMenuState {
                id: "agent-pane-1".to_string(),
                label: "feature/one".to_string(),
            }],
            Some("agent-pane-1".to_string()),
        );

        state.set_window_agent_active_tab("main", Some("agent-pane-999".to_string()));

        let tabs = state.window_agent_tabs_for_window("main");
        assert_eq!(tabs.active_tab_id, None);
    }

    #[test]
    fn window_migration_set_get_and_clear_matching_job() {
        let state = AppState::new();

        state.set_window_migration("main", "job-1".to_string(), "/tmp/repo".to_string());

        let migrations = state.window_migrations_snapshot();
        assert_eq!(migrations.len(), 1);
        assert_eq!(
            migrations.get("main"),
            Some(&WindowMigrationState {
                job_id: "job-1".to_string(),
                source_root: "/tmp/repo".to_string(),
            })
        );

        // Non-matching job id must not clear a newer/other job.
        state.clear_window_migration_if_job("main", "job-other");
        assert!(state.window_migrations_snapshot().contains_key("main"));

        state.clear_window_migration_if_job("main", "job-1");
        assert!(!state.window_migrations_snapshot().contains_key("main"));
    }

    #[test]
    fn clear_project_for_window_keeps_migration_state() {
        let state = AppState::new();
        let claim = state.claim_project_for_window_with_identity(
            "main",
            "/tmp/repo".to_string(),
            "/tmp/repo-canonical".to_string(),
        );
        assert_eq!(claim, Ok(()));
        state.set_window_agent_tabs(
            "main",
            vec![AgentTabMenuState {
                id: "agent-pane-1".to_string(),
                label: "feature/one".to_string(),
            }],
            Some("agent-pane-1".to_string()),
        );
        state.set_window_migration("main", "job-1".to_string(), "/tmp/repo".to_string());

        state.clear_project_for_window("main");

        assert_eq!(state.project_for_window("main"), None);
        assert_eq!(
            state.window_agent_tabs_for_window("main"),
            WindowAgentTabsState::default()
        );
        assert!(state.window_migrations_snapshot().contains_key("main"));
    }

    #[test]
    fn clear_project_for_window_removes_window_from_mru_history() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]
        state.clear_project_for_window("B");
        let history = state.window_focus_history.lock().unwrap();
        assert_eq!(*history, vec!["A", "C"]);
    }

    #[test]
    fn window_rotation_skips_window_after_project_close() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]
        state.clear_project_for_window("B");
        assert_eq!(state.next_window(), Some("C".to_string()));
        assert_eq!(state.next_window(), Some("A".to_string()));
    }

    #[test]
    fn mru_push_moves_to_front() {
        let state = AppState::new();
        state.push_window_focus("A");
        state.push_window_focus("B");
        state.push_window_focus("C");
        // History: [C, B, A]
        state.push_window_focus("A");
        // History: [A, C, B]
        let history = state.window_focus_history.lock().unwrap();
        assert_eq!(*history, vec!["A", "C", "B"]);
    }

    #[test]
    fn mru_next_returns_second_entry() {
        let state = AppState::new();
        state.push_window_focus("A");
        state.push_window_focus("B");
        state.push_window_focus("C");
        // History: [C, B, A] → rotate left → [B, A, C] → next = B
        assert_eq!(state.next_window(), Some("B".to_string()));
    }

    #[test]
    fn mru_next_returns_none_for_single() {
        let state = AppState::new();
        state.push_window_focus("A");
        assert_eq!(state.next_window(), None);
    }

    #[test]
    fn mru_previous_returns_last_entry() {
        let state = AppState::new();
        state.push_window_focus("A");
        state.push_window_focus("B");
        state.push_window_focus("C");
        // History: [C, B, A] → rotate right → [A, C, B] → previous = A
        assert_eq!(state.previous_window(), Some("A".to_string()));
    }

    #[test]
    fn mru_peek_recent_and_oldest_without_rotation() {
        let state = AppState::new();
        state.push_window_focus("A");
        state.push_window_focus("B");
        state.push_window_focus("C");
        // History: [C, B, A]
        assert_eq!(state.most_recent_window(), Some("C".to_string()));
        assert_eq!(state.least_recent_window(), Some("A".to_string()));
        let history = state.window_focus_history.lock().unwrap();
        assert_eq!(*history, vec!["C", "B", "A"]);
    }

    #[test]
    fn mru_remove_cleans_up() {
        let state = AppState::new();
        state.push_window_focus("A");
        state.push_window_focus("B");
        state.push_window_focus("C");
        state.remove_window_from_history("B");
        let history = state.window_focus_history.lock().unwrap();
        assert_eq!(*history, vec!["C", "A"]);
    }

    #[test]
    fn next_window_cycles_all_three() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]
        assert_eq!(state.next_window(), Some("B".to_string()));
        assert_eq!(state.next_window(), Some("C".to_string()));
        assert_eq!(state.next_window(), Some("A".to_string()));
        // Full cycle back to start
        assert_eq!(state.next_window(), Some("B".to_string()));
    }

    #[test]
    fn previous_window_cycles_reverse() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]
        assert_eq!(state.previous_window(), Some("C".to_string()));
        assert_eq!(state.previous_window(), Some("B".to_string()));
        assert_eq!(state.previous_window(), Some("A".to_string()));
        // Full cycle back to start
        assert_eq!(state.previous_window(), Some("C".to_string()));
    }

    #[test]
    fn push_focus_after_rotation_is_idempotent() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]
        assert_eq!(state.next_window(), Some("B".to_string()));
        // Simulate OS focus event for the rotated-to window
        state.push_window_focus("B");
        // B is already at front, so push_window_focus is idempotent
        assert_eq!(state.next_window(), Some("C".to_string()));
        state.push_window_focus("C");
        assert_eq!(state.next_window(), Some("A".to_string()));
    }

    #[test]
    fn two_windows_toggle() {
        let state = AppState::new();
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B]
        assert_eq!(state.next_window(), Some("B".to_string()));
        assert_eq!(state.next_window(), Some("A".to_string()));
        assert_eq!(state.next_window(), Some("B".to_string()));
    }

    #[test]
    fn clear_window_state_removes_project_tabs_mode_and_migration() {
        let state = AppState::new();
        let claim = state.claim_project_for_window_with_identity(
            "main",
            "/tmp/repo".to_string(),
            "/tmp/repo-canonical".to_string(),
        );
        assert_eq!(claim, Ok(()));
        state.set_window_agent_tabs(
            "main",
            vec![AgentTabMenuState {
                id: "agent-pane-1".to_string(),
                label: "feature/one".to_string(),
            }],
            Some("agent-pane-1".to_string()),
        );
        state.set_window_migration("main", "job-1".to_string(), "/tmp/repo".to_string());

        if let Ok(mut map) = state.window_project_modes.lock() {
            map.insert("main".to_string(), ProjectModeState::new());
        }

        state.clear_window_state("main");

        assert_eq!(state.project_for_window("main"), None);
        assert_eq!(
            state.window_agent_tabs_for_window("main"),
            WindowAgentTabsState::default()
        );
        assert!(!state.window_migrations_snapshot().contains_key("main"));
        assert!(state
            .window_project_modes
            .lock()
            .map(|m| !m.contains_key("main"))
            .unwrap_or(false));
    }

    #[test]
    fn window_session_restore_leader_only_allows_main() {
        let state = AppState::new();
        assert!(!state.try_acquire_window_session_restore_leader_at("project-1", 1_000));
        assert!(!state.try_acquire_window_session_restore_leader_at("  ", 1_000));

        assert!(state.try_acquire_window_session_restore_leader_at("main", 1_000));
        let leader = state
            .window_session_restore_leader
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        assert_eq!(
            leader,
            Some(WindowSessionRestoreLeaderState {
                label: "main".to_string(),
                expires_at_millis: 1_000 + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
            })
        );
    }

    #[test]
    fn window_session_restore_leader_blocks_active_other_label() {
        let state = AppState::new();
        if let Ok(mut slot) = state.window_session_restore_leader.lock() {
            *slot = Some(WindowSessionRestoreLeaderState {
                label: "other".to_string(),
                expires_at_millis: 20_000,
            });
        }

        assert!(!state.try_acquire_window_session_restore_leader_at("main", 10_000));
        let leader = state
            .window_session_restore_leader
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        assert_eq!(
            leader,
            Some(WindowSessionRestoreLeaderState {
                label: "other".to_string(),
                expires_at_millis: 20_000,
            })
        );
    }

    #[test]
    fn window_session_restore_leader_reacquires_after_expiration_and_releases_by_label() {
        let state = AppState::new();
        if let Ok(mut slot) = state.window_session_restore_leader.lock() {
            *slot = Some(WindowSessionRestoreLeaderState {
                label: "old".to_string(),
                expires_at_millis: 5_000,
            });
        }

        assert!(state.try_acquire_window_session_restore_leader_at("main", 6_000));
        let leader = state
            .window_session_restore_leader
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        assert_eq!(
            leader,
            Some(WindowSessionRestoreLeaderState {
                label: "main".to_string(),
                expires_at_millis: 6_000 + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
            })
        );

        state.release_window_session_restore_leader("project-1");
        let leader_after_wrong_release = state
            .window_session_restore_leader
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        assert_eq!(
            leader_after_wrong_release,
            Some(WindowSessionRestoreLeaderState {
                label: "main".to_string(),
                expires_at_millis: 6_000 + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
            })
        );

        state.release_window_session_restore_leader("main");
        let cleared = state
            .window_session_restore_leader
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        assert_eq!(cleared, None);
    }

    // ── quit_confirm state management ──

    const QUIT_CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);

    #[test]
    fn quit_confirm_begin_returns_true_first_time() {
        let state = AppState::new();
        assert!(
            state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT),
            "first call should return true"
        );
    }

    #[test]
    fn quit_confirm_begin_returns_false_when_already_active() {
        let state = AppState::new();
        assert!(state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT));
        assert!(
            !state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT),
            "second call should return false while confirm is active"
        );
    }

    #[test]
    fn quit_confirm_is_active_within_timeout() {
        let state = AppState::new();
        state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT);
        assert!(
            state.is_quit_confirm_active(Duration::from_secs(5)),
            "should be active within generous timeout"
        );
    }

    #[test]
    fn quit_confirm_is_not_active_after_timeout() {
        let state = AppState::new();
        state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT);
        // A zero-duration timeout means anything that already elapsed is expired.
        assert!(
            !state.is_quit_confirm_active(Duration::ZERO),
            "should not be active after zero-duration timeout"
        );
    }

    #[test]
    fn quit_confirm_cancel_resets_state() {
        let state = AppState::new();
        assert!(state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT));
        state.cancel_quit_confirm();
        assert!(
            state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT),
            "after cancel, begin should return true again"
        );
    }

    #[test]
    fn quit_confirm_cancel_when_not_active_is_noop() {
        let state = AppState::new();
        // Should not panic when called on a fresh state.
        state.cancel_quit_confirm();
        // State should still be usable afterwards.
        assert!(state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT));
    }

    #[test]
    fn quit_confirm_begin_rearms_when_stale() {
        let state = AppState::new();
        if let Ok(mut slot) = state.quit_confirm_requested_at.lock() {
            *slot = Some(Instant::now() - Duration::from_secs(30));
        }
        assert!(
            state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT),
            "expired confirm state should be re-armed"
        );
        assert!(
            state.is_quit_confirm_active(QUIT_CONFIRM_TIMEOUT),
            "re-armed confirm should be active"
        );
    }

    #[test]
    fn window_hidden_removed_from_cycling() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]

        // Simulate closing window B (CloseRequested → hide → remove from history)
        state.remove_window_from_history("B");

        // B should not appear in cycling
        assert_eq!(state.next_window(), Some("C".to_string()));
        assert_eq!(state.next_window(), Some("A".to_string()));
        assert_eq!(state.next_window(), Some("C".to_string()));
    }

    #[test]
    fn window_refocused_after_hide_readded() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]

        // Simulate closing B
        state.remove_window_from_history("B");
        // History: [A, C]

        // Simulate B being reopened and focused
        state.push_window_focus("B");
        // History: [B, A, C]

        assert_eq!(state.next_window(), Some("A".to_string()));
        assert_eq!(state.next_window(), Some("C".to_string()));
        assert_eq!(state.next_window(), Some("B".to_string()));
    }

    #[test]
    fn hide_all_but_one_prevents_cycling() {
        let state = AppState::new();
        state.push_window_focus("C");
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B, C]

        // Close B and C
        state.remove_window_from_history("B");
        state.remove_window_from_history("C");
        // History: [A]

        assert_eq!(state.next_window(), None);
    }

    #[test]
    fn most_recent_window_excludes_hidden() {
        let state = AppState::new();
        state.push_window_focus("B");
        state.push_window_focus("A");
        // History: [A, B]

        assert_eq!(state.most_recent_window(), Some("A".to_string()));

        // Close A (most recent)
        state.remove_window_from_history("A");
        // History: [B]

        assert_eq!(state.most_recent_window(), Some("B".to_string()));
    }
}
