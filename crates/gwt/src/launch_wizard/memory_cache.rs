//! Shared Session ledger and agent-detection snapshots for launch consumers.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    thread,
};

use crate::session_launch::durable_launch_recovery_exists;

/// SPEC-3864 T-006: install detection runs off the startup critical path.
/// The slot starts as a background thread and is joined on first wizard
/// access; clones share the same slot so a joined result is reused.
#[derive(Debug)]
enum AgentOptionsSlot {
    Loading(Option<std::thread::JoinHandle<Vec<crate::AgentOption>>>),
    Ready(Vec<crate::AgentOption>),
}

#[derive(Debug)]
pub struct LaunchWizardMemoryCache {
    sessions_dir: PathBuf,
    // Issue #4377: the ledger holds every Session ever launched, stopped ones
    // included (1,124 files on one host), so it is parsed on a background
    // thread and joined on first read instead of inside `AppRuntime::new`.
    sessions: Arc<OnceLock<Vec<gwt_agent::Session>>>,
    pending_sessions: Arc<Mutex<Option<thread::JoinHandle<Vec<gwt_agent::Session>>>>>,
    agent_options: Arc<Mutex<AgentOptionsSlot>>,
    // SPEC-3170 FR-001: Claude capability detection may read settings and run
    // `claude --version` once per process. The wizard stores the booleans at
    // cache load time and reuses them on every open.
    claude_ultracode_supported: bool,
    claude_workflows_enabled: bool,
}

impl Clone for LaunchWizardMemoryCache {
    fn clone(&self) -> Self {
        Self {
            sessions_dir: self.sessions_dir.clone(),
            sessions: self.sessions.clone(),
            pending_sessions: self.pending_sessions.clone(),
            agent_options: self.agent_options.clone(),
            claude_ultracode_supported: self.claude_ultracode_supported,
            claude_workflows_enabled: self.claude_workflows_enabled,
        }
    }
}

impl LaunchWizardMemoryCache {
    pub fn load(sessions_dir: &Path) -> Self {
        let claude_capabilities = gwt_agent::claude_capability_snapshot();
        Self {
            sessions_dir: sessions_dir.to_path_buf(),
            sessions: Arc::new(OnceLock::new()),
            pending_sessions: Arc::new(Self::spawn_session_load(sessions_dir)),
            agent_options: Self::spawn_agent_options_detection(),
            claude_ultracode_supported: claude_capabilities.ultracode_supported,
            claude_workflows_enabled: claude_capabilities.workflows_enabled,
        }
    }

    /// Load with a caller-supplied agent detection snapshot.
    #[doc(hidden)]
    pub fn load_with_agent_options(
        sessions_dir: &Path,
        agent_options: Vec<crate::AgentOption>,
    ) -> Self {
        Self::load_with_agent_options_and_capabilities(sessions_dir, agent_options, false, false)
    }

    /// Load with caller-supplied agent and Claude capability snapshots.
    #[doc(hidden)]
    pub fn load_with_agent_options_and_capabilities(
        sessions_dir: &Path,
        agent_options: Vec<crate::AgentOption>,
        claude_ultracode_supported: bool,
        claude_workflows_enabled: bool,
    ) -> Self {
        Self {
            sessions_dir: sessions_dir.to_path_buf(),
            sessions: Arc::new(OnceLock::new()),
            pending_sessions: Arc::new(Self::spawn_session_load(sessions_dir)),
            agent_options: Arc::new(Mutex::new(AgentOptionsSlot::Ready(agent_options))),
            claude_ultracode_supported,
            claude_workflows_enabled,
        }
    }

    fn load_sessions(sessions_dir: &Path) -> Vec<gwt_agent::Session> {
        gwt_agent::session_ledger::load_sessions(sessions_dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|session| !durable_launch_recovery_exists(sessions_dir, &session.id))
            .collect()
    }

    /// A thread that cannot be spawned leaves the slot empty, and the first
    /// read then loads inline, so the ledger is never silently empty.
    fn spawn_session_load(
        sessions_dir: &Path,
    ) -> Mutex<Option<thread::JoinHandle<Vec<gwt_agent::Session>>>> {
        let sessions_dir = sessions_dir.to_path_buf();
        let handle = thread::Builder::new()
            .name("gwt-session-ledger".to_string())
            .spawn(move || Self::load_sessions(&sessions_dir));
        Mutex::new(match handle {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(error = %error, "session ledger thread unavailable; loading on first read");
                None
            }
        })
    }

    pub fn sessions(&self) -> &Vec<gwt_agent::Session> {
        self.sessions.get_or_init(|| {
            self.pending_sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .and_then(|handle| handle.join().ok())
                .unwrap_or_else(|| Self::load_sessions(&self.sessions_dir))
        })
    }

    fn sessions_mut(&mut self) -> &mut Vec<gwt_agent::Session> {
        self.sessions();
        Arc::make_mut(&mut self.sessions)
            .get_mut()
            .expect("session ledger resolved above")
    }

    fn load_agent_options() -> Vec<crate::AgentOption> {
        crate::load_agent_options()
    }

    /// Start install detection on a background thread so `load` (and thus
    /// app startup / settings updates) never waits on `<agent> --version`
    /// probes. A thread that cannot be spawned falls back to synchronous
    /// detection so the options are never silently empty.
    fn spawn_agent_options_detection() -> Arc<Mutex<AgentOptionsSlot>> {
        let slot = match std::thread::Builder::new()
            .name("gwt-agent-detect".to_string())
            .spawn(Self::load_agent_options)
        {
            Ok(handle) => AgentOptionsSlot::Loading(Some(handle)),
            Err(error) => {
                tracing::warn!(error = %error, "agent detection thread unavailable; detecting inline");
                AgentOptionsSlot::Ready(Self::load_agent_options())
            }
        };
        Arc::new(Mutex::new(slot))
    }

    pub fn refresh_agent_options(&mut self) {
        self.agent_options = Self::spawn_agent_options_detection();
    }

    /// Refresh after a Host installer in the same active-profile environment.
    /// Called on the maintenance worker so every version probe stays off Tao.
    pub fn refresh_agent_options_for_profile(&mut self, config_path: &Path, cwd: &Path) {
        // A completion can immediately refresh an open wizard. Resolve its
        // ledger here as well, before that cache is handed back to Tao.
        self.sessions();
        let mut options: Vec<_> = self
            .agent_options()
            .into_iter()
            .filter(|agent| agent.custom_agent.is_some())
            .collect();
        match gwt_agent::LaunchEnvironment::from_active_profile(
            config_path,
            gwt_agent::LaunchRuntimeTarget::Host,
        ) {
            Ok(environment) => {
                let (env, remove_env) = environment.into_parts();
                let detected = gwt_agent::builtin_agent_descriptors()
                    .iter()
                    .filter_map(|descriptor| {
                        gwt_agent::AgentDetector::detect_by_command_with_environment(
                            descriptor.command,
                            &env,
                            &remove_env,
                            Some(cwd),
                        )
                    })
                    .collect();
                options.extend(
                    crate::build_builtin_agent_options(detected)
                        .into_iter()
                        .filter(|agent| agent.available),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "cannot refresh agents in the active Host profile")
            }
        }
        self.agent_options = Arc::new(Mutex::new(AgentOptionsSlot::Ready(options)));
    }

    pub fn agent_options(&self) -> Vec<crate::AgentOption> {
        let mut slot = self
            .agent_options
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let AgentOptionsSlot::Loading(handle) = &mut *slot {
            let options = handle
                .take()
                .and_then(|handle| handle.join().ok())
                .unwrap_or_else(Self::load_agent_options);
            *slot = AgentOptionsSlot::Ready(options);
        }
        match &*slot {
            AgentOptionsSlot::Ready(options) => options.clone(),
            AgentOptionsSlot::Loading(_) => unreachable!("slot joined above"),
        }
    }

    /// Read a completed detection snapshot without waiting or probing providers.
    /// Status projections stay unavailable while the worker or its lock is busy.
    pub fn ready_agent_options(&self) -> Option<Vec<crate::AgentOption>> {
        let mut slot = self.agent_options.try_lock().ok()?;
        if let AgentOptionsSlot::Loading(handle) = &mut *slot {
            if !handle.as_ref()?.is_finished() {
                return None;
            }
            let options = handle.take()?.join().ok()?;
            *slot = AgentOptionsSlot::Ready(options);
        }
        match &*slot {
            AgentOptionsSlot::Ready(options) => Some(options.clone()),
            AgentOptionsSlot::Loading(_) => None,
        }
    }

    /// Settings shows the entire built-in catalog while the wizard still
    /// offers only installed agents. Reuse its existing detection snapshot.
    pub fn supported_agents(&self) -> Vec<crate::protocol::SupportedAgentView> {
        let options = self.agent_options();
        gwt_agent::builtin_agent_descriptors()
            .iter()
            .map(|descriptor| {
                let detected = options.iter().find(|option| {
                    option.id == descriptor.command
                        && option.available
                        && option.custom_agent.is_none()
                });
                crate::protocol::SupportedAgentView {
                    id: descriptor.command.to_string(),
                    name: descriptor.display_name.to_string(),
                    installed: detected.is_some(),
                    available_version: None,
                    update_check_error: None,
                    update_available: false,
                    up_to_date: false,
                    install_supported: !matches!(
                        descriptor.distribution,
                        gwt_agent::DistributionRoute::None
                    ),
                    installed_version: detected
                        .and_then(|agent| agent.installed_version.as_deref())
                        .map(str::trim)
                        .filter(|version| !version.is_empty())
                        .map(str::to_string),
                }
            })
            .collect()
    }

    /// SPEC-3170 FR-001: cached `claude --version`-derived ultracode capability,
    /// resolved once at load time so wizard open never re-spawns the probe.
    pub fn claude_ultracode_supported(&self) -> bool {
        self.claude_ultracode_supported
    }

    /// SPEC-3170 FR-001: cached Claude dynamic-workflows capability, resolved
    /// once at load time so wizard open never re-reads the settings file.
    pub fn claude_workflows_enabled(&self) -> bool {
        self.claude_workflows_enabled
    }

    pub fn quick_start_entries(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Vec<crate::QuickStartEntry> {
        let sessions = self.visible_sessions();
        crate::launch_wizard::quick_start_entries_from_sessions(repo_path, branch_name, &sessions)
    }

    fn visible_sessions(&self) -> Vec<gwt_agent::Session> {
        // `sessions` is recovery-filtered at every ingest boundary
        // (`load_sessions`, `replace_sessions`, and `record_session`). Keep UI
        // reads cache-only so opening or sorting the wizard never performs one
        // filesystem probe per Session on the tao thread.
        self.sessions().clone()
    }

    pub fn latest_resumable_branch_session(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        let entry = self
            .quick_start_entries(repo_path, branch_name)
            .into_iter()
            .find(|entry| entry.resume_session_id.is_some())?;
        self.sessions()
            .iter()
            .find(|session| session.id == entry.session_id)
            .cloned()
    }

    /// Replace all cached sessions with a freshly disk-loaded set. Called from
    /// the off-thread branch load (#2995) so resume availability and resolution
    /// observe session TOMLs the hook CLI wrote out-of-process after launch,
    /// without ever blocking the main UI thread on disk I/O.
    pub fn replace_sessions(&mut self, sessions: Vec<gwt_agent::Session>) {
        let sessions: Vec<_> = sessions
            .into_iter()
            .filter(|session| !durable_launch_recovery_exists(&self.sessions_dir, &session.id))
            .collect();
        // A still-running startup load is superseded; dropping it detaches.
        self.pending_sessions = Arc::new(Mutex::new(None));
        self.sessions = Arc::new(OnceLock::from(sessions));
    }

    pub fn session_by_id(&self, session_id: &str) -> Option<&gwt_agent::Session> {
        self.sessions()
            .iter()
            .find(|session| session.id == session_id)
    }

    /// Read only an already-resolved ledger; never join or load it on a status path.
    pub fn ready_session_by_id(&self, session_id: &str) -> Option<&gwt_agent::Session> {
        self.sessions
            .get()?
            .iter()
            .find(|session| session.id == session_id)
    }

    pub fn agent_preferences(&self) -> crate::LaunchWizardPreviousProfiles {
        crate::launch_wizard::previous_launch_profiles_from_sessions(&self.visible_sessions())
    }

    pub fn previous_profiles(&self, repo_path: &Path) -> crate::LaunchWizardPreviousProfiles {
        crate::launch_wizard::previous_launch_profiles_for_repo_from_sessions(
            repo_path,
            &self.visible_sessions(),
        )
    }

    pub fn record_session(&mut self, session: gwt_agent::Session) {
        if durable_launch_recovery_exists(&self.sessions_dir, &session.id) {
            self.forget_session(&session.id);
            return;
        }
        let sessions = self.sessions_mut();
        if let Some(existing) = sessions
            .iter_mut()
            .find(|existing| existing.id == session.id)
        {
            *existing = session;
        } else {
            sessions.push(session);
        }
    }

    /// The launch worker has resolved the shared ledger and recovery-filtered
    /// this snapshot. Later cache replacements are already resolved as well.
    pub fn record_prepared_session(
        &mut self,
        session_id: &str,
        session: Option<gwt_agent::Session>,
    ) {
        let sessions = Arc::make_mut(&mut self.sessions)
            .get_mut()
            .expect("launch worker resolved the Session ledger");
        match session {
            Some(session) => {
                if let Some(existing) = sessions
                    .iter_mut()
                    .find(|existing| existing.id == session.id)
                {
                    *existing = session;
                } else {
                    sessions.push(session);
                }
            }
            None => sessions.retain(|session| session.id != session_id),
        }
    }

    pub fn forget_prepared_genesis_session(
        &mut self,
        identity: &gwt_agent::SessionExecutionIdentity,
    ) {
        let sessions = Arc::make_mut(&mut self.sessions)
            .get_mut()
            .expect("launch worker resolved the Session ledger");
        sessions.retain(|session| {
            gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(identity)
        });
    }

    pub fn mark_stopped(&mut self, session_id: &str) {
        if let Some(session) = self
            .sessions_mut()
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.update_status(gwt_agent::AgentStatus::Stopped);
        }
    }

    pub fn forget_session(&mut self, session_id: &str) {
        self.sessions_mut()
            .retain(|session| session.id != session_id);
    }
}

#[cfg(test)]
mod fr001_capability_cache_tests {
    use crate::launch_wizard::LaunchWizardMemoryCache;

    // SPEC-3170 FR-001: the Claude capability probes are resolved once at cache
    // load time and the getters return the stored booleans verbatim, so opening
    // the Launch wizard reads cached values instead of re-spawning
    // `claude --version` on the tao main event-loop thread.
    #[test]
    fn caches_claude_capabilities_for_reuse_without_reprobe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(dir.path());

        let on = LaunchWizardMemoryCache::load_with_agent_options_and_capabilities(
            dir.path(),
            Vec::new(),
            true,
            true,
        );
        assert!(on.claude_ultracode_supported());
        assert!(on.claude_workflows_enabled());

        let off = LaunchWizardMemoryCache::load_with_agent_options_and_capabilities(
            dir.path(),
            Vec::new(),
            false,
            false,
        );
        assert!(!off.claude_ultracode_supported());
        assert!(!off.claude_workflows_enabled());
    }
}

#[cfg(test)]
mod lazy_session_ledger_tests {
    use crate::launch_wizard::LaunchWizardMemoryCache;

    #[test]
    fn tier_projection_cache_reads_pending_and_ready_snapshots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cache = LaunchWizardMemoryCache::load_with_agent_options(dir.path(), Vec::new());
        let session =
            gwt_agent::Session::new(dir.path(), "work/issue-4499", gwt_agent::AgentId::Codex);
        let session_id = session.id.clone();
        let (release_session, wait_session) = std::sync::mpsc::channel();
        cache.pending_sessions =
            std::sync::Arc::new(std::sync::Mutex::new(Some(std::thread::spawn(move || {
                wait_session.recv().unwrap();
                vec![session]
            }))));
        let (release_agents, wait_agents) = std::sync::mpsc::channel();
        cache.agent_options = std::sync::Arc::new(std::sync::Mutex::new(
            super::AgentOptionsSlot::Loading(Some(std::thread::spawn(move || {
                wait_agents.recv().unwrap();
                Vec::new()
            }))),
        ));
        assert!(cache.ready_agent_options().is_none());
        assert!(cache.ready_session_by_id(&session_id).is_none());
        release_agents.send(()).unwrap();
        while !matches!(
            &*cache.agent_options.lock().unwrap(),
            super::AgentOptionsSlot::Loading(Some(worker)) if worker.is_finished()
        ) {
            std::thread::yield_now();
        }
        assert!(cache.ready_agent_options().unwrap().is_empty());
        let guard = cache.agent_options.lock().unwrap();
        assert!(
            cache.ready_agent_options().is_none(),
            "contention must not block a status update"
        );
        drop(guard);
        release_session.send(()).unwrap();
        cache.sessions();
        assert_eq!(
            cache
                .ready_session_by_id(&session_id)
                .map(|session| &session.id),
            Some(&session_id)
        );
    }

    /// Issue #4377 (AC-1 / AC-3): the Session ledger is parsed off the
    /// constructor (and so off `AppRuntime::new`); the first wizard read
    /// joins it and serves the same Sessions, stopped ones included.
    #[test]
    fn session_ledger_loads_off_the_constructor_and_serves_the_same_sessions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(dir.path());
        let sessions_dir = dir.path().join("sessions");
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");
        let mut stopped =
            gwt_agent::Session::new(&worktree, "work/stopped", gwt_agent::AgentId::Codex);
        stopped.id = "session-stopped".to_string();
        stopped.agent_session_id = Some("native-stopped".to_string());
        stopped.update_status(gwt_agent::AgentStatus::Stopped);
        stopped.save(&sessions_dir).expect("save stopped session");

        let cache = LaunchWizardMemoryCache::load_with_agent_options(&sessions_dir, Vec::new());

        assert!(
            cache.sessions.get().is_none(),
            "the constructor must not parse the Session ledger"
        );
        assert_eq!(
            cache
                .session_by_id("session-stopped")
                .map(|session| session.status),
            Some(gwt_agent::AgentStatus::Stopped)
        );
        assert!(cache.clone().session_by_id("session-stopped").is_some());
        let expected = crate::launch_wizard::load_quick_start_entries(
            &worktree,
            &sessions_dir,
            "work/stopped",
        );
        assert_eq!(
            expected.len(),
            1,
            "the stopped conversation remains resumable"
        );
        assert_eq!(
            cache.quick_start_entries(&worktree, "work/stopped"),
            expected
        );
        let resumed = cache
            .latest_resumable_branch_session(&worktree, "work/stopped")
            .expect("stopped session remains a restore candidate");
        assert_eq!(resumed.id, stopped.id);
        assert_eq!(resumed.agent_session_id, stopped.agent_session_id);
    }
}
