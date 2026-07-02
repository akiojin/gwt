//! Frontend user-action audit logging split out of `app_runtime/mod.rs`
//! for SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - [`FrontendUserActionLog`] — the sanitized, fixed-field record emitted
//!   under the `gwt_ui_action` tracing target
//! - [`frontend_user_action_log`] — the [`FrontendEvent`] -> log-record
//!   mapping (high-volume / sensitive events return `None`)
//! - [`log_frontend_user_action`] — the dispatch-side entry point
//!
//! Behavior-preserving move: field sanitization rules (control-char strip,
//! 160-char cap, URL authority redaction, 12-item value summaries) are
//! unchanged.

use gwt::LaunchWizardAction;

use super::{AppRuntime, FrontendEvent};

#[derive(Default)]
pub(super) struct FrontendUserActionLog {
    pub(super) action: &'static str,
    pub(super) surface: &'static str,
    pub(super) window_id: String,
    pub(super) ui_target: String,
    pub(super) profile_name: String,
    pub(super) env_keys: String,
    pub(super) env_var_count: usize,
    pub(super) disabled_env_count: usize,
    pub(super) agent_id: String,
    pub(super) count: usize,
    pub(super) mode: String,
    pub(super) forced: bool,
}

impl FrontendUserActionLog {
    fn new(action: &'static str, surface: &'static str) -> Self {
        Self {
            action,
            surface,
            ..Default::default()
        }
    }

    fn window(mut self, id: &str) -> Self {
        self.window_id = sanitize_ui_action_field(id);
        self
    }

    fn target(mut self, value: impl AsRef<str>) -> Self {
        self.ui_target = sanitize_ui_action_field(value.as_ref());
        self
    }

    fn profile(mut self, name: impl AsRef<str>) -> Self {
        self.profile_name = sanitize_ui_action_field(name.as_ref());
        self
    }

    fn agent(mut self, id: impl AsRef<str>) -> Self {
        self.agent_id = sanitize_ui_action_field(id.as_ref());
        self
    }

    fn mode(mut self, value: impl AsRef<str>) -> Self {
        self.mode = sanitize_ui_action_field(value.as_ref());
        self
    }

    fn count(mut self, value: usize) -> Self {
        self.count = value;
        self
    }

    fn force(mut self, value: bool) -> Self {
        self.forced = value;
        self
    }

    fn env_keys<'a>(mut self, values: impl IntoIterator<Item = &'a str>) -> Self {
        let keys: Vec<_> = values.into_iter().collect();
        self.env_var_count = keys.len();
        self.env_keys = summarize_ui_action_values(keys);
        self
    }

    fn disabled_env_count(mut self, value: usize) -> Self {
        self.disabled_env_count = value;
        self
    }
}

fn sanitize_ui_action_field(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(160)
        .collect()
}

fn sanitize_ui_action_url(value: &str) -> String {
    let sanitized = sanitize_ui_action_field(value);
    let Some((scheme, rest)) = sanitized.split_once("://") else {
        return sanitized;
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    if authority.is_empty() {
        sanitized
    } else {
        format!("{scheme}://{authority}")
    }
}

fn summarize_ui_action_values<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut items: Vec<String> = values
        .into_iter()
        .map(sanitize_ui_action_field)
        .filter(|value| !value.is_empty())
        .collect();
    items.sort();
    items.dedup();
    let truncated = items.len() > 12;
    items.truncate(12);
    let mut summary = items.join(",");
    if truncated {
        if !summary.is_empty() {
            summary.push_str(",...");
        } else {
            summary.push_str("...");
        }
    }
    summary
}

pub(super) fn frontend_user_action_log(event: &FrontendEvent) -> Option<FrontendUserActionLog> {
    let log = match event {
        FrontendEvent::FrontendReady => FrontendUserActionLog::new("frontend_ready", "app"),
        FrontendEvent::SetClaudeAccountUsageEnabled { enabled } => {
            FrontendUserActionLog::new("set_claude_account_usage_enabled", "usage")
                .mode(if *enabled { "on" } else { "off" })
        }
        FrontendEvent::RefreshUsage => FrontendUserActionLog::new("refresh_usage", "usage"),
        FrontendEvent::OpenProjectDialog => {
            FrontendUserActionLog::new("open_project_dialog", "project")
        }
        FrontendEvent::SelectCloneProjectParent => {
            FrontendUserActionLog::new("select_clone_project_parent", "project")
        }
        FrontendEvent::GithubRepositorySearch { query } => {
            FrontendUserActionLog::new("github_repository_search", "project").count(query.len())
        }
        FrontendEvent::CloneProjectStart { url, parent_path } => {
            FrontendUserActionLog::new("clone_project_start", "project")
                .count(url.len())
                .mode(parent_path)
        }
        FrontendEvent::ReopenRecentProject { path } => {
            FrontendUserActionLog::new("reopen_recent_project", "project").target(path)
        }
        FrontendEvent::SelectProjectTab { tab_id } => {
            FrontendUserActionLog::new("select_project_tab", "project").target(tab_id)
        }
        FrontendEvent::CloseProjectTab { tab_id } => {
            FrontendUserActionLog::new("close_project_tab", "project").target(tab_id)
        }
        FrontendEvent::CreateWindow { preset, .. } => {
            FrontendUserActionLog::new("create_window", "window").target(format!("{preset:?}"))
        }
        FrontendEvent::LoadProcessConsole { id } => {
            FrontendUserActionLog::new("load_process_console", "console").window(id)
        }
        FrontendEvent::FocusWindow { id, .. } => {
            FrontendUserActionLog::new("focus_window", "window").window(id)
        }
        FrontendEvent::CycleFocus { direction, .. } => {
            FrontendUserActionLog::new("cycle_focus", "window").mode(format!("{direction:?}"))
        }
        FrontendEvent::ArrangeWindows { mode, .. } => {
            FrontendUserActionLog::new("arrange_windows", "window").mode(format!("{mode:?}"))
        }
        FrontendEvent::DockWindowTab { id, target_id } => {
            FrontendUserActionLog::new("dock_window_tab", "window")
                .window(id)
                .target(target_id)
        }
        FrontendEvent::ActivateWindowTab { id } => {
            FrontendUserActionLog::new("activate_window_tab", "window").window(id)
        }
        FrontendEvent::DetachWindowTab { id, .. } => {
            FrontendUserActionLog::new("detach_window_tab", "window").window(id)
        }
        FrontendEvent::PlaceAgentWindowInKanban {
            id,
            board_id,
            lane_id,
            ..
        } => FrontendUserActionLog::new("place_agent_window_in_kanban", "window")
            .window(id)
            .target(format!("{board_id}:{lane_id:?}")),
        FrontendEvent::MoveAgentKanbanCard {
            id,
            board_id,
            lane_id,
            ..
        } => FrontendUserActionLog::new("move_agent_kanban_card", "window")
            .window(id)
            .target(format!("{board_id}:{lane_id:?}")),
        FrontendEvent::UndockAgentWindow { id, .. } => {
            FrontendUserActionLog::new("undock_agent_window", "window").window(id)
        }
        FrontendEvent::SetAgentKanbanCardCollapsed { id, collapsed } => {
            FrontendUserActionLog::new("set_agent_kanban_card_collapsed", "window")
                .window(id)
                .mode(collapsed.to_string())
        }
        FrontendEvent::UpdateTerminalGrid { id, .. } => {
            FrontendUserActionLog::new("update_terminal_grid", "terminal").window(id)
        }
        FrontendEvent::ListWindows => FrontendUserActionLog::new("list_windows", "window"),
        FrontendEvent::CloseWindow { id } => {
            FrontendUserActionLog::new("close_window", "window").window(id)
        }
        FrontendEvent::StopWindow { id } => {
            FrontendUserActionLog::new("stop_window", "window").window(id)
        }
        FrontendEvent::StopAllWindows {} => {
            FrontendUserActionLog::new("stop_all_windows", "window")
        }
        FrontendEvent::RestartWindow { id } => {
            FrontendUserActionLog::new("restart_window", "window").window(id)
        }
        FrontendEvent::LoadFileTree { id, path } => {
            FrontendUserActionLog::new("load_file_tree", "file")
                .window(id)
                .target(path.as_deref().unwrap_or_default())
        }
        FrontendEvent::ListFileTreeWorktrees { id } => {
            FrontendUserActionLog::new("list_file_tree_worktrees", "file").window(id)
        }
        FrontendEvent::SelectFileTreeWorktree { id, worktree_id } => {
            FrontendUserActionLog::new("select_file_tree_worktree", "file")
                .window(id)
                .target(worktree_id)
        }
        FrontendEvent::LoadFileContent { id, path, mode, .. } => {
            FrontendUserActionLog::new("load_file_content", "file")
                .window(id)
                .target(path)
                .mode(format!("{mode:?}"))
        }
        FrontendEvent::SaveFileContent { id, path, mode, .. } => {
            FrontendUserActionLog::new("save_file_content", "file")
                .window(id)
                .target(path)
                .mode(format!("{mode:?}"))
        }
        FrontendEvent::LoadBranches { id } => {
            FrontendUserActionLog::new("load_branches", "branches").window(id)
        }
        FrontendEvent::RequestRemoteStartWorkBranches { id } => {
            FrontendUserActionLog::new("request_remote_start_work_branches", "branches").window(id)
        }
        FrontendEvent::RunBranchCleanup {
            id,
            branches,
            delete_remote,
            force_filesystem_delete,
        } => FrontendUserActionLog::new("run_branch_cleanup", "branches")
            .window(id)
            .target(summarize_ui_action_values(
                branches.iter().map(String::as_str),
            ))
            .count(branches.len())
            .mode(if *delete_remote {
                "delete_remote"
            } else {
                "local_only"
            })
            .force(*force_filesystem_delete),
        FrontendEvent::RunWorkspaceCleanup {
            branch,
            delete_remote,
            force_filesystem_delete,
        } => FrontendUserActionLog::new("run_workspace_cleanup", "workspace")
            .target(branch)
            .count(1)
            .mode(if *delete_remote {
                "delete_remote"
            } else {
                "local_only"
            })
            .force(*force_filesystem_delete),
        FrontendEvent::LoadBoard { id, all } => FrontendUserActionLog::new("load_board", "board")
            .window(id)
            .mode(if *all { "all" } else { "workspace" }),
        FrontendEvent::LoadBoardHistory { id, all, limit, .. } => {
            FrontendUserActionLog::new("load_board_history", "board")
                .window(id)
                .count(*limit)
                .mode(if *all { "all" } else { "workspace" })
        }
        FrontendEvent::PostBoardEntry {
            id,
            entry_kind,
            body,
            ..
        } => FrontendUserActionLog::new("post_board_entry", "board")
            .window(id)
            .mode(format!("{entry_kind:?}"))
            .count(body.len()),
        FrontendEvent::OpenBoardOriginAgent {
            id,
            origin_session_id,
            ..
        } => FrontendUserActionLog::new("open_board_origin_agent", "board")
            .window(id)
            .target(origin_session_id),
        FrontendEvent::LoadProfile { id } => {
            FrontendUserActionLog::new("load_profile", "profile").window(id)
        }
        FrontendEvent::SelectProfile { id, profile_name } => {
            FrontendUserActionLog::new("select_profile", "profile")
                .window(id)
                .profile(profile_name)
        }
        FrontendEvent::CreateProfile { id, name } => {
            FrontendUserActionLog::new("create_profile", "profile")
                .window(id)
                .profile(name)
        }
        FrontendEvent::SetActiveProfile { id, profile_name } => {
            FrontendUserActionLog::new("set_active_profile", "profile")
                .window(id)
                .profile(profile_name)
        }
        FrontendEvent::SaveProfile {
            id,
            name,
            env_vars,
            disabled_env,
            ..
        } => FrontendUserActionLog::new("save_profile", "profile")
            .window(id)
            .profile(name)
            .env_keys(env_vars.iter().map(|entry| entry.key.as_str()))
            .disabled_env_count(disabled_env.len()),
        FrontendEvent::DeleteProfile { id, profile_name } => {
            FrontendUserActionLog::new("delete_profile", "profile")
                .window(id)
                .profile(profile_name)
        }
        FrontendEvent::LoadLogs { id } => {
            FrontendUserActionLog::new("load_logs", "logs").window(id)
        }
        FrontendEvent::LoadKnowledgeBridge {
            id,
            knowledge_kind,
            refresh,
            ..
        } => FrontendUserActionLog::new("load_knowledge_bridge", "knowledge")
            .window(id)
            .mode(format!("{knowledge_kind:?}"))
            .force(*refresh),
        FrontendEvent::SearchKnowledgeBridge {
            id,
            knowledge_kind,
            query,
            ..
        } => FrontendUserActionLog::new("search_knowledge_bridge", "knowledge")
            .window(id)
            .mode(format!("{knowledge_kind:?}"))
            .count(query.len()),
        FrontendEvent::SearchProjectIndex {
            id,
            query,
            scopes,
            worktree_hash,
            ..
        } => FrontendUserActionLog::new("search_project_index", "index")
            .window(id)
            .mode(summarize_ui_action_values(
                scopes.iter().map(|scope| scope.as_str()),
            ))
            .agent(worktree_hash.as_deref().unwrap_or_default())
            .count(query.len()),
        FrontendEvent::RequestWorkAdvisory { id, query, .. } => {
            FrontendUserActionLog::new("request_work_advisory", "launch")
                .window(id)
                .count(query.len())
        }
        FrontendEvent::SelectKnowledgeBridgeEntry {
            id,
            knowledge_kind,
            number,
            ..
        } => FrontendUserActionLog::new("select_knowledge_bridge_entry", "knowledge")
            .window(id)
            .mode(format!("{knowledge_kind:?}"))
            .target(number.to_string()),
        FrontendEvent::UpdateKnowledgeBridgePhase {
            id,
            issue_number,
            target_phase,
            ..
        } => FrontendUserActionLog::new("update_knowledge_bridge_phase", "knowledge")
            .window(id)
            .target(issue_number.to_string())
            .mode(target_phase.as_deref().unwrap_or("backlog")),
        FrontendEvent::RebuildIndexCell {
            project_root,
            scope,
            worktree_hash,
        } => FrontendUserActionLog::new("rebuild_index_cell", "index")
            .target(project_root)
            .mode(format!("{scope:?}"))
            .agent(worktree_hash.as_deref().unwrap_or_default()),
        FrontendEvent::RefreshIndexStatus { project_root } => {
            FrontendUserActionLog::new("refresh_index_status", "index").target(project_root)
        }
        FrontendEvent::OpenIssueLaunchWizard { id, issue_number } => {
            FrontendUserActionLog::new("open_issue_launch_wizard", "launch")
                .window(id)
                .target(issue_number.to_string())
        }
        FrontendEvent::QuickRegisterIssue { launch, .. } => {
            FrontendUserActionLog::new("quick_register_issue", "issue_monitor").mode(if *launch {
                "register_and_launch"
            } else {
                "register"
            })
        }
        FrontendEvent::OpenStartWork => FrontendUserActionLog::new("open_start_work", "launch"),
        FrontendEvent::OpenStartWorkInAgentKanban { board_id, lane_id } => {
            FrontendUserActionLog::new("open_start_work_in_agent_kanban", "launch")
                .window(board_id)
                .mode(format!("{lane_id:?}"))
        }
        FrontendEvent::OpenAgentKanbanLaunchWizard { board_id, lane_id } => {
            FrontendUserActionLog::new("open_agent_kanban_launch_wizard", "launch")
                .window(board_id)
                .mode(format!("{lane_id:?}"))
        }
        FrontendEvent::ResumeWorkspace { source, .. } => {
            FrontendUserActionLog::new("resume_workspace", "workspace").mode(format!("{source:?}"))
        }
        FrontendEvent::ListResumableAgents { workspace_id } => {
            FrontendUserActionLog::new("list_resumable_agents", "workspace")
                .target(workspace_id.as_deref().unwrap_or_default())
        }
        FrontendEvent::ResumeWorkspaceAgent { session_id, .. } => {
            FrontendUserActionLog::new("resume_workspace_agent", "workspace").target(session_id)
        }
        FrontendEvent::ResumeBranchLatestAgent {
            id, branch_name, ..
        } => FrontendUserActionLog::new("resume_branch_latest_agent", "launch")
            .window(id)
            .target(branch_name),
        FrontendEvent::OpenLaunchWizard {
            id,
            branch_name,
            linked_issue_number,
        } => FrontendUserActionLog::new("open_launch_wizard", "launch")
            .window(id)
            .target(branch_name)
            .count(linked_issue_number.unwrap_or_default() as usize),
        FrontendEvent::OpenActiveWorkLaunchWizard {
            branch_name,
            linked_issue_number,
        } => FrontendUserActionLog::new("open_active_work_launch_wizard", "launch")
            .target(branch_name)
            .count(linked_issue_number.unwrap_or_default() as usize),
        FrontendEvent::LaunchWizardAction { action, .. } => {
            let mut log = FrontendUserActionLog::new("launch_wizard_action", "launch")
                .mode(AppRuntime::launch_wizard_action_label(action));
            match action {
                LaunchWizardAction::SetAgent { agent_id } => {
                    log = log.agent(agent_id);
                }
                LaunchWizardAction::SetBranchName { value }
                | LaunchWizardAction::SetBranchType { prefix: value }
                | LaunchWizardAction::SetModel { model: value }
                | LaunchWizardAction::SetReasoning { reasoning: value }
                | LaunchWizardAction::SetVersion { version: value }
                | LaunchWizardAction::SetExecutionMode { mode: value }
                | LaunchWizardAction::SetDockerService { service: value } => {
                    log = log.target(value);
                }
                LaunchWizardAction::SubmitText { value }
                | LaunchWizardAction::SetInitialPrompt { value } => {
                    log = log.count(value.len());
                }
                LaunchWizardAction::SetSkipPermissions { enabled }
                | LaunchWizardAction::SetFastMode { enabled }
                | LaunchWizardAction::SetCodexFastMode { enabled } => {
                    log = log.force(*enabled);
                }
                LaunchWizardAction::SetLinkedIssue { issue_number } => {
                    log = log.target(issue_number.to_string());
                }
                _ => {}
            }
            log
        }
        FrontendEvent::ApplyUpdate => FrontendUserActionLog::new("apply_update", "update"),
        FrontendEvent::ApplyUpdateStart => {
            FrontendUserActionLog::new("apply_update_start", "update")
        }
        FrontendEvent::CancelUpdateDownload => {
            FrontendUserActionLog::new("cancel_update_download", "update")
        }
        FrontendEvent::ApplyUpdateLater => {
            FrontendUserActionLog::new("apply_update_later", "update")
        }
        FrontendEvent::ApplyUpdateRestartNow => {
            FrontendUserActionLog::new("apply_update_restart_now", "update")
        }
        FrontendEvent::OpenUpdateLog { log_path } => {
            FrontendUserActionLog::new("open_update_log", "update")
                .target(log_path.as_deref().unwrap_or_default())
        }
        FrontendEvent::OpenServerUrl { .. } => {
            FrontendUserActionLog::new("open_server_url", "status")
        }
        FrontendEvent::ListCustomAgents => {
            FrontendUserActionLog::new("list_custom_agents", "custom_agents")
        }
        FrontendEvent::ListCustomAgentPresets => {
            FrontendUserActionLog::new("list_custom_agent_presets", "custom_agents")
        }
        FrontendEvent::AddCustomAgentFromPreset { input } => {
            FrontendUserActionLog::new("add_custom_agent_from_preset", "custom_agents")
                .agent(&input.id)
                .profile(&input.display_name)
        }
        FrontendEvent::UpdateCustomAgent { agent } => {
            FrontendUserActionLog::new("update_custom_agent", "custom_agents")
                .agent(&agent.id)
                .profile(&agent.display_name)
                .env_keys(agent.env.keys().map(String::as_str))
        }
        FrontendEvent::DeleteCustomAgent { agent_id } => {
            FrontendUserActionLog::new("delete_custom_agent", "custom_agents").agent(agent_id)
        }
        FrontendEvent::TestBackendConnection { base_url, .. } => {
            FrontendUserActionLog::new("test_backend_connection", "custom_agents")
                .target(sanitize_ui_action_url(base_url))
        }
        FrontendEvent::ListAgentBackends { agent } => {
            FrontendUserActionLog::new("list_agent_backends", "agent_backends")
                .agent(agent.as_str())
        }
        FrontendEvent::AddAgentBackend { agent, profile } => {
            FrontendUserActionLog::new("add_agent_backend", "agent_backends")
                .agent(agent.as_str())
                .profile(&profile.id)
        }
        FrontendEvent::UpdateAgentBackend { agent, id, .. } => {
            FrontendUserActionLog::new("update_agent_backend", "agent_backends")
                .agent(agent.as_str())
                .profile(id)
        }
        FrontendEvent::DeleteAgentBackend { agent, id } => {
            FrontendUserActionLog::new("delete_agent_backend", "agent_backends")
                .agent(agent.as_str())
                .profile(id)
        }
        FrontendEvent::TestAgentBackendConnection {
            agent, base_url, ..
        } => FrontendUserActionLog::new("test_agent_backend_connection", "agent_backends")
            .agent(agent.as_str())
            .target(sanitize_ui_action_url(base_url)),
        FrontendEvent::StartMigration { tab_id } => {
            FrontendUserActionLog::new("start_migration", "migration").target(tab_id)
        }
        FrontendEvent::SkipMigration { tab_id } => {
            FrontendUserActionLog::new("skip_migration", "migration").target(tab_id)
        }
        FrontendEvent::QuitMigration { tab_id } => {
            FrontendUserActionLog::new("quit_migration", "migration").target(tab_id)
        }
        FrontendEvent::GetSystemSettings => {
            FrontendUserActionLog::new("get_system_settings", "settings")
        }
        FrontendEvent::GetBoardAuthStatus => {
            FrontendUserActionLog::new("get_board_auth_status", "settings")
        }
        FrontendEvent::BoardProviderSignIn { provider } => {
            FrontendUserActionLog::new("board_provider_sign_in", "settings").target(provider)
        }
        FrontendEvent::BoardProviderSignOut { provider } => {
            FrontendUserActionLog::new("board_provider_sign_out", "settings").target(provider)
        }
        FrontendEvent::UpdateBoardProviderConfig { provider, .. } => {
            FrontendUserActionLog::new("update_board_provider_config", "settings").target(provider)
        }
        FrontendEvent::UpdateBoardOauthPort { port } => {
            FrontendUserActionLog::new("update_board_oauth_port", "settings")
                .target(port.to_string())
        }
        FrontendEvent::GetProjectBoardConfig { project_root } => {
            FrontendUserActionLog::new("get_project_board_config", "settings").target(project_root)
        }
        FrontendEvent::UpdateProjectBoardConfig { project_root, .. } => {
            FrontendUserActionLog::new("update_project_board_config", "settings")
                .target(project_root)
        }
        FrontendEvent::UpdateSystemSettings {
            language,
            codex_trust_managed_hooks,
            ..
        } => FrontendUserActionLog::new("update_system_settings", "settings")
            .target(language)
            .force(codex_trust_managed_hooks.unwrap_or(false)),
        FrontendEvent::GetAutostartStatus => {
            FrontendUserActionLog::new("get_autostart_status", "settings")
        }
        FrontendEvent::UpdateAutostart { enabled } => {
            FrontendUserActionLog::new("update_autostart", "settings").force(*enabled)
        }
        FrontendEvent::WorkspaceProjectionPrune { dry_run, ids } => {
            FrontendUserActionLog::new("workspace_projection_prune", "workspace")
                .mode(if *dry_run { "dry_run" } else { "apply" })
                .count(ids.len())
        }
        FrontendEvent::SaveUiTrace { trace } => {
            let entries = trace.entries().map(|entries| entries.len()).unwrap_or(0);
            FrontendUserActionLog::new("save_ui_trace", "diagnostics")
                .target(trace.session_id().unwrap_or_default())
                .count(entries)
        }
        FrontendEvent::OpenReleaseNotes { focus_version, .. } => {
            FrontendUserActionLog::new("open_release_notes", "release_notes")
                .target(focus_version.as_deref().unwrap_or_default())
        }
        FrontendEvent::ApplyUpdateToVersion { version } => {
            FrontendUserActionLog::new("apply_update_to_version", "update").target(version)
        }
        FrontendEvent::CloseWork {
            work_id,
            close_kind,
        } => FrontendUserActionLog::new("close_work", "workspace")
            .target(format!("{work_id} ({close_kind})")),
        FrontendEvent::ImprovementPromoteIssue { id } => {
            FrontendUserActionLog::new("improvement_promote_issue", "improvement").target(id)
        }
        FrontendEvent::ImprovementDismiss { id, .. } => {
            FrontendUserActionLog::new("improvement_dismiss", "improvement").target(id)
        }
        // SPEC-3050: log the injection request without its text payload —
        // the injected line lands in the PTY transcript anyway.
        FrontendEvent::PaneSendInput { session_id, .. } => {
            FrontendUserActionLog::new("pane_send_input", "terminal").target(session_id)
        }
        FrontendEvent::SetIssueMonitorEnabled { enabled } => {
            FrontendUserActionLog::new("set_issue_monitor_enabled", "issue_monitor")
                .mode(if *enabled { "on" } else { "off" })
        }
        FrontendEvent::SetIssueMonitorAutonomousMode { enabled } => {
            FrontendUserActionLog::new("set_issue_monitor_autonomous_mode", "issue_monitor")
                .mode(if *enabled { "on" } else { "off" })
        }
        FrontendEvent::SetIssueMonitorMaxActiveAgents { max_active_agents } => {
            FrontendUserActionLog::new("set_issue_monitor_max_active_agents", "issue_monitor")
                .target(max_active_agents.to_string())
        }
        FrontendEvent::ReorderIssueMonitorIssues { issue_numbers } => {
            FrontendUserActionLog::new("reorder_issue_monitor_issues", "issue_monitor")
                .target(issue_numbers.len().to_string())
        }
        FrontendEvent::ListIssueMonitor => {
            FrontendUserActionLog::new("list_issue_monitor", "issue_monitor")
        }
        FrontendEvent::IssueMonitorLaunchNow { issue_number, .. } => {
            FrontendUserActionLog::new("issue_monitor_launch_now", "issue_monitor")
                .target(issue_number.to_string())
        }
        FrontendEvent::IssueMonitorConfigureIssue { issue_number, .. } => {
            FrontendUserActionLog::new("issue_monitor_configure_issue", "issue_monitor")
                .target(issue_number.to_string())
        }
        // These events can contain high-volume, high-frequency, or sensitive
        // payloads. They are handled by more specific logs or diagnostics.
        FrontendEvent::StartupAutoResumeReady { .. }
        | FrontendEvent::UpdateViewport { .. }
        | FrontendEvent::UpdateWindowGeometry { .. }
        | FrontendEvent::TerminalInput { .. }
        | FrontendEvent::PasteImage { .. }
        | FrontendEvent::PasteImageUploaded { .. }
        | FrontendEvent::AttachFiles { .. } => return None,
    };
    Some(log)
}

pub(super) fn log_frontend_user_action(client_id: &str, event: &FrontendEvent) {
    let Some(log) = frontend_user_action_log(event) else {
        return;
    };
    tracing::info!(
        target: "gwt_ui_action",
        client_id = %client_id,
        action = %log.action,
        surface = %log.surface,
        window_id = %log.window_id,
        ui_target = %log.ui_target,
        profile_name = %log.profile_name,
        env_keys = %log.env_keys,
        env_var_count = log.env_var_count as u64,
        disabled_env_count = log.disabled_env_count as u64,
        agent_id = %log.agent_id,
        count = log.count as u64,
        mode = %log.mode,
        forced = log.forced,
        "frontend user action"
    );
}
