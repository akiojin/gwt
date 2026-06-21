//! Launch wizard handler split out of `app_runtime/mod.rs` for SPEC-2077
//! Phase F1 / F2a-1 (arch-review handoff, 2026-05-01).
//!
//! Phase F is split into multiple sub-phases to keep blast radius small:
//! - F1 (merged): wizard state broadcast / clear helpers
//! - F2a-1 (this PR): branch-level open helpers (open_launch_wizard,
//!   open_launch_wizard_for_branch, refresh_open_launch_wizard_from_cache)
//! - F2a-2 (follow-up): issue-level open + prepared dispatch handlers
//! - F2b (follow-up): handle_launch_wizard_action (~600 lines)
//! - F3a/F3b (follow-up): spawn_wizard_shell_window* (~525 lines)
//!
//! [`LaunchWizardSession`] still lives in `mod.rs` because the larger wizard
//! handlers (Phase F2b / F3 scope) construct and mutate it; once those
//! phases land the struct can move here too.

use std::{
    path::{Path, PathBuf},
    thread,
};

use chrono::Utc;
use gwt::{
    knowledge_launch_target_branch_name, KnowledgeKind, LaunchWizardCompletion,
    LaunchWizardContext, LaunchWizardHydration, LaunchWizardLaunchPath, LaunchWizardLaunchRequest,
    LaunchWizardState, LaunchWizardView, LinkedIssueKind, WindowGeometry,
};
use uuid::Uuid;

use crate::{ShellLaunchConfig, UserEvent};

/// `Pr => None` because Launch Agent is not exposed for PR bridges
/// (`KnowledgeDetailView::launch_issue_number` stays `None` for PR entries).
fn linked_issue_kind_from_knowledge(kind: KnowledgeKind) -> Option<LinkedIssueKind> {
    match kind {
        KnowledgeKind::Issue => Some(LinkedIssueKind::Issue),
        KnowledgeKind::Spec => Some(LinkedIssueKind::Spec),
        KnowledgeKind::Pr => None,
    }
}

fn launch_wizard_open_error(
    client_id: &str,
    title: &str,
    message: impl Into<String>,
) -> OutboundEvent {
    OutboundEvent::reply(
        client_id.to_string(),
        BackendEvent::LaunchWizardOpenError {
            title: title.to_string(),
            message: message.into(),
        },
    )
}

fn launch_agent_open_error(client_id: &str, message: impl Into<String>) -> Vec<OutboundEvent> {
    vec![launch_wizard_open_error(client_id, "Launch Agent", message)]
}

fn start_work_open_error(client_id: &str, message: impl Into<String>) -> Vec<OutboundEvent> {
    vec![launch_wizard_open_error(client_id, "Start Work", message)]
}

use super::{
    build_shell_process_launch, combined_window_id, detect_wizard_docker_context_and_status,
    knowledge_error_event, knowledge_kind_for_preset, linked_issue_workspace_context,
    list_branch_entries_with_active_sessions, normalize_branch_name, preferred_issue_launch_branch,
    resolve_shell_launch_worktree, save_shell_work_projection, session_exact_resume_materializable,
    synthetic_branch_entry, workspace_projection_for_current_resume,
    workspace_resume_branch_exists, workspace_resume_branch_from_journal_project_root,
    workspace_resume_context_for_work_item, workspace_resume_context_from_journal,
    workspace_resume_context_from_projection, workspace_resume_owner_issue_number,
    AgentKanbanLaunchTarget, AppEventProxy, AppRuntime, BackendEvent, IssueLaunchWizardPrepared,
    LaunchFeedbackContext, LaunchWizardMemoryCache, LaunchWizardSession, OutboundEvent,
    WindowPreset, WindowProcessStatus, WorkspaceResumeContext, WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
};
use crate::usable_worktree_path_for_branch;

impl AppRuntime {
    pub(crate) fn launch_wizard_state_outbound(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: self
                .launch_wizard
                .as_ref()
                .map(|wizard| Box::new(wizard.wizard.view())),
        })
    }

    pub(crate) fn launch_wizard_state_broadcast(
        &self,
        wizard: Option<LaunchWizardView>,
    ) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: wizard.map(Box::new),
        })
    }

    pub(crate) fn clear_launch_wizard(&mut self) -> Option<LaunchWizardSession> {
        self.launch_wizard.take()
    }

    pub(crate) fn open_launch_wizard(
        &mut self,
        client_id: &str,
        id: &str,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return launch_agent_open_error(client_id, "Window not found");
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return launch_agent_open_error(client_id, "Project tab not found");
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return launch_agent_open_error(client_id, "Window not found");
        };

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            tracing::warn!(
                preset = ?window.preset,
                window_id = id,
                "open_launch_wizard rejected: wrong preset"
            );
            return launch_agent_open_error(
                client_id,
                format!("Window preset {:?} is not a Work surface", window.preset),
            );
        }
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return launch_agent_open_error(
                client_id,
                "Complete the project migration before launching an agent",
            );
        }

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        match self.open_launch_wizard_for_branch(
            &tab_id,
            &project_root,
            branch_name,
            linked_issue_number,
            None,
        ) {
            Ok(()) => vec![self.launch_wizard_state_outbound()],
            Err(error) => launch_agent_open_error(client_id, error),
        }
    }

    pub(crate) fn open_launch_wizard_for_branch(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        branch_name: &str,
        linked_issue_number: Option<u64>,
        linked_issue_kind: Option<LinkedIssueKind>,
    ) -> Result<(), String> {
        self.open_launch_wizard_for_branch_with_context(
            tab_id,
            project_root,
            branch_name,
            linked_issue_number,
            linked_issue_kind,
            None,
        )
    }

    pub(crate) fn open_launch_wizard_for_branch_with_context(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        branch_name: &str,
        linked_issue_number: Option<u64>,
        linked_issue_kind: Option<LinkedIssueKind>,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<(), String> {
        let normalized_branch_name = normalize_branch_name(branch_name);
        let live_sessions = self.live_sessions_for_branch(tab_id, &normalized_branch_name);
        let worktree_path = None;
        let quick_start_root = project_root.to_path_buf();
        // SPEC-2014 US-27: Branches > Launch Agent must expose all
        // resumable sessions in Quick Start so users can choose a specific
        // prior conversation. The cache is already in memory, so this stays
        // off the GUI hot path's filesystem scan.
        let mut quick_start_entries = self
            .launch_wizard_cache
            .quick_start_entries(&quick_start_root, &normalized_branch_name);
        if workspace_resume_context.is_none() {
            quick_start_entries.retain(|entry| entry.resume_session_id.is_some());
        }
        let previous_profiles = self.launch_wizard_cache.agent_preferences();
        let agent_options = self.launch_wizard_cache.agent_options();
        let docker_context = None;
        let docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        let wizard_id = Uuid::new_v4().to_string();
        let mut wizard = LaunchWizardState::open_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(branch_name),
                normalized_branch_name,
                worktree_path,
                quick_start_root,
                live_sessions,
                docker_context,
                docker_service_status,
                linked_issue_number,
                linked_issue_kind,
                ultracode_supported: gwt_agent::claude_ultracode_supported(),
                claude_workflows_enabled: gwt_agent::claude_workflows_enabled(),
            },
            agent_options,
            quick_start_entries,
            previous_profiles,
        );
        wizard.mark_runtime_context_unresolved();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard,
            workspace_resume_context,
            agent_kanban_target: None,
        });

        Ok(())
    }

    pub(crate) fn open_knowledge_launch_wizard_for_base_branch(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        base_branch_name: &str,
        issue_number: u64,
        linked_issue_kind: LinkedIssueKind,
    ) -> Result<(), String> {
        let base_branch_name = normalize_branch_name(base_branch_name);
        let target_branch_name =
            knowledge_launch_target_branch_name(linked_issue_kind, issue_number);
        let live_sessions = self.live_sessions_for_branch(tab_id, &target_branch_name);
        let quick_start_root = project_root.to_path_buf();
        let quick_start_entries = Vec::new();
        let previous_profiles = self.launch_wizard_cache.agent_preferences();
        let agent_options = self.launch_wizard_cache.agent_options();
        let docker_context = None;
        let docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        let wizard_id = Uuid::new_v4().to_string();
        let owner_label = match linked_issue_kind {
            LinkedIssueKind::Issue => format!("Issue #{issue_number}"),
            LinkedIssueKind::Spec => format!("SPEC #{issue_number}"),
        };
        let workspace_resume_context = Some(linked_issue_workspace_context(
            project_root,
            issue_number,
            owner_label,
        ));
        let mut wizard = LaunchWizardState::open_knowledge_launch_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(&base_branch_name),
                normalized_branch_name: target_branch_name,
                worktree_path: None,
                quick_start_root,
                live_sessions,
                docker_context,
                docker_service_status,
                linked_issue_number: Some(issue_number),
                linked_issue_kind: Some(linked_issue_kind),
                ultracode_supported: gwt_agent::claude_ultracode_supported(),
                claude_workflows_enabled: gwt_agent::claude_workflows_enabled(),
            },
            base_branch_name,
            agent_options,
            quick_start_entries,
            previous_profiles,
        );
        wizard.mark_runtime_context_unresolved();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard,
            workspace_resume_context,
            agent_kanban_target: None,
        });

        Ok(())
    }

    pub(crate) fn open_active_work_launch_wizard(
        &mut self,
        client_id: &str,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return launch_agent_open_error(client_id, "Open a project before adding an agent");
        };
        let Some(tab) = self.tab(&tab_id) else {
            return launch_agent_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return launch_agent_open_error(client_id, "Add Agent requires a Git project");
        }
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return launch_agent_open_error(
                client_id,
                "Complete the project migration before adding an agent",
            );
        }

        let project_root = tab.project_root.clone();
        if let Some(window_id) = self.live_agent_window_for_work(&tab_id, Some(branch_name), None) {
            return self.focus_existing_live_work_agent_events(&window_id, None);
        }
        match self.open_launch_wizard_for_branch(
            &tab_id,
            &project_root,
            branch_name,
            linked_issue_number,
            None,
        ) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard.as_mut() {
                    session.wizard.launch_path = LaunchWizardLaunchPath::ManualSetup;
                }
                vec![self.launch_wizard_state_outbound()]
            }
            Err(error) => launch_agent_open_error(client_id, error),
        }
    }

    pub(crate) fn open_start_work(&mut self, client_id: &str) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return start_work_open_error(client_id, "Open a project before starting work");
        };
        let Some(tab) = self.tab(&tab_id) else {
            return start_work_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return start_work_open_error(client_id, "Start Work requires a Git project");
        }
        // SPEC-1934 US-7 / FR-034: refuse Start Work on a project whose
        // Nested Bare+Worktree migration has not completed. Without this
        // gate, `git fetch origin --prune` on a single-branch refspec leaves
        // the new `work/*` branch unsynchronized and the launch path dies
        // with `fatal: invalid reference: origin/work/<branch>`.
        if tab.migration_pending {
            return start_work_open_error(
                client_id,
                "Complete the project migration before starting work",
            );
        }

        let project_root = tab.project_root.clone();
        match self.open_start_work_for_project(&tab_id, &project_root) {
            Ok(()) => vec![self.launch_wizard_state_outbound()],
            Err(error) => start_work_open_error(client_id, error),
        }
    }

    pub(crate) fn open_start_work_in_agent_kanban(
        &mut self,
        client_id: &str,
        board_id: &str,
        lane_id: gwt::AgentKanbanLane,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(board_id).cloned() else {
            return start_work_open_error(client_id, "Window not found");
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return start_work_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return start_work_open_error(client_id, "Start Work requires a Git project");
        }
        if tab.migration_pending {
            return start_work_open_error(
                client_id,
                "Complete the project migration before starting work",
            );
        }
        let Some(board_window) = tab.workspace.window(&address.raw_id) else {
            return start_work_open_error(client_id, "Window not found");
        };
        if board_window.preset != gwt::WindowPreset::AgentKanban {
            return start_work_open_error(
                client_id,
                format!(
                    "Window preset {:?} is not an Agent Kanban surface",
                    board_window.preset
                ),
            );
        }

        let tab_id = address.tab_id.clone();
        let project_root = tab.project_root.clone();
        match self.open_start_work_for_project(&tab_id, &project_root) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard.as_mut() {
                    session.agent_kanban_target = Some(AgentKanbanLaunchTarget {
                        board_id: address.raw_id,
                        lane_id,
                    });
                }
                self.active_tab_id = Some(tab_id);
                vec![self.launch_wizard_state_outbound()]
            }
            Err(error) => start_work_open_error(client_id, error),
        }
    }

    pub(crate) fn open_agent_kanban_launch_wizard(
        &mut self,
        client_id: &str,
        board_id: &str,
        lane_id: gwt::AgentKanbanLane,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(board_id).cloned() else {
            return launch_agent_open_error(client_id, "Window not found");
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return launch_agent_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return launch_agent_open_error(client_id, "Launch Agent requires a Git project");
        }
        if tab.migration_pending {
            return launch_agent_open_error(
                client_id,
                "Complete the project migration before launching an agent",
            );
        }
        let Some(board_window) = tab.workspace.window(&address.raw_id) else {
            return launch_agent_open_error(client_id, "Window not found");
        };
        if board_window.preset != gwt::WindowPreset::AgentKanban {
            return launch_agent_open_error(
                client_id,
                format!(
                    "Window preset {:?} is not an Agent Kanban surface",
                    board_window.preset
                ),
            );
        }

        let tab_id = address.tab_id.clone();
        let project_root = tab.project_root.clone();
        let branch_name = match current_branch_name_for_launch_agent(&project_root) {
            Ok(branch_name) => branch_name,
            Err(error) => return launch_agent_open_error(client_id, error),
        };
        match self.open_launch_wizard_for_branch(&tab_id, &project_root, &branch_name, None, None) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard.as_mut() {
                    session.agent_kanban_target = Some(AgentKanbanLaunchTarget {
                        board_id: address.raw_id,
                        lane_id,
                    });
                    session.wizard.launch_path = LaunchWizardLaunchPath::ManualSetup;
                }
                self.active_tab_id = Some(tab_id);
                vec![self.launch_wizard_state_outbound()]
            }
            Err(error) => launch_agent_open_error(client_id, error),
        }
    }

    pub(crate) fn resume_workspace_events(
        &mut self,
        client_id: &str,
        source: gwt::WorkspaceResumeSource,
        journal_id: Option<String>,
    ) -> Vec<OutboundEvent> {
        // SPEC-2359 / Issue #2757: Resume click failures must surface through
        // `LaunchWizardOpenError` (a client-scoped reply) instead of the
        // legacy `ProjectOpenError` broadcast, which the frontend renders only
        // on the project picker overlay and is therefore invisible while a
        // project tab is already open.
        let error_event =
            |message: &str| vec![launch_wizard_open_error(client_id, "Resume Work", message)];

        let Some(tab_id) = self.active_tab_id.clone() else {
            return error_event("Open a project before resuming work");
        };
        let Some(tab) = self.tab(&tab_id) else {
            return error_event("Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return error_event("Resume Work requires a Git project");
        }
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return error_event("Complete the project migration before resuming work");
        }
        let project_root = tab.project_root.clone();
        let tab_title = tab.title.clone();
        let current_sessions = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .collect::<Vec<_>>();

        let (branch_candidate, context) = match source {
            gwt::WorkspaceResumeSource::Current => {
                let projection =
                    gwt_core::workspace_projection::load_workspace_projection(&project_root)
                        .ok()
                        .flatten()
                        .map(|projection| {
                            workspace_projection_for_current_resume(
                                projection,
                                &current_sessions,
                                &tab_title,
                                Utc::now(),
                            )
                        });
                let branch = projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.branch.clone());
                let context = projection
                    .as_ref()
                    .map(workspace_resume_context_from_projection)
                    .unwrap_or_else(|| WorkspaceResumeContext {
                        title: Some(format!("{tab_title} Work")),
                        owner: None,
                        summary: None,
                        next_action: None,
                    });
                (branch, context)
            }
            gwt::WorkspaceResumeSource::Journal => {
                let Some(journal_id) = journal_id else {
                    return error_event("Work journal id is required");
                };
                let Ok(entries) =
                    gwt_core::workspace_projection::load_recent_workspace_journal_entries(
                        &project_root,
                        WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
                    )
                else {
                    return error_event("Work journal could not be loaded");
                };
                let Some(entry) = entries.into_iter().find(|entry| entry.id == journal_id) else {
                    return error_event("Work journal entry not found");
                };
                (
                    workspace_resume_branch_from_journal_project_root(
                        &entry.project_root,
                        &project_root,
                    ),
                    workspace_resume_context_from_journal(&entry),
                )
            }
        };

        if let Some(branch_name) = branch_candidate
            .as_deref()
            .map(normalize_branch_name)
            .filter(|branch| !branch.trim().is_empty())
        {
            if workspace_resume_branch_exists(&project_root, &branch_name) {
                if let Some(window_id) =
                    self.live_agent_window_for_work(&tab_id, Some(&branch_name), None)
                {
                    return self.focus_existing_live_work_agent_events(&window_id, None);
                }
                let linked_issue_number =
                    workspace_resume_owner_issue_number(context.owner.as_deref());
                return match self.open_launch_wizard_for_branch_with_context(
                    &tab_id,
                    &project_root,
                    &branch_name,
                    linked_issue_number,
                    None,
                    Some(context),
                ) {
                    Ok(()) => vec![self.launch_wizard_state_outbound()],
                    Err(error) => error_event(&error),
                };
            }
        }

        match self.open_start_work_for_project_with_context(&tab_id, &project_root, Some(context)) {
            Ok(()) => vec![self.launch_wizard_state_outbound()],
            Err(error) => error_event(&error),
        }
    }

    // SPEC-2359 US-42: list / resume entries for the Workspace Resume
    // picker. These bypass the Launch Wizard entirely so the Resume
    // button can restart a previously-assigned agent in-place.

    pub(crate) fn list_resumable_agents_events(
        &mut self,
        client_id: &str,
        workspace_id: Option<String>,
    ) -> Vec<OutboundEvent> {
        let agents = self.collect_resumable_agents(workspace_id.as_deref());
        vec![OutboundEvent::reply(
            client_id.to_string(),
            BackendEvent::WorkspaceResumableAgents {
                agents,
                workspace_id,
            },
        )]
    }

    pub(crate) fn resume_workspace_agent_events(
        &mut self,
        client_id: &str,
        session_id: String,
        agent_session_id: Option<String>,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let reply_error = |message: String| {
            vec![OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::WorkspaceResumeAgentError {
                    session_id: session_id.clone(),
                    message,
                },
            )]
        };
        // SPEC-2359 W-17 (FR-398): client-scoped ack so the requesting
        // frontend can settle its pending Resume UI deterministically.
        let started_ack = |session_id: &str, branch: Option<String>| {
            OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::WorkspaceResumeAgentStarted {
                    session_id: session_id.to_string(),
                    branch,
                },
            )
        };

        let Some(tab_id) = self.active_tab_id.clone() else {
            return reply_error("Open a project before resuming an agent".to_string());
        };
        let Some(tab) = self.tab(&tab_id) else {
            return reply_error("Project tab not found".to_string());
        };
        if tab.kind != gwt::ProjectKind::Git {
            return reply_error("Resume requires a Git project".to_string());
        }
        if tab.migration_pending {
            return reply_error(
                "Complete the project migration before resuming an agent".to_string(),
            );
        }

        if let Some((window_id, live_gwt_session)) = self
            .active_agent_sessions
            .iter()
            .find(|(window_id, session)| {
                session.session_id == session_id
                    && session.tab_id == tab_id
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self
                        .window_status(window_id.as_str())
                        .is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
            })
            .map(|(window_id, session)| (window_id.clone(), session.session_id.clone()))
        {
            // SPEC-2359 D1: focusing a live window resumes the conversation it is
            // already running. If the user clicked Resume on a *different* (older)
            // Session, focusing would silently drop that intent — surface a
            // visible error instead of pretending the request succeeded.
            if self.resume_conversation_conflicts(agent_session_id.as_deref(), &live_gwt_session) {
                return reply_error(
                    "This Work is currently running a different conversation; stop it before resuming an older Session.".to_string(),
                );
            }
            let mut events = self.focus_existing_live_work_agent_events(&window_id, Some(bounds));
            events.push(started_ack(&session_id, None));
            return events;
        }

        let project_root = tab.project_root.clone();
        let sessions_dir = self.sessions_dir.clone();
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        let session = match gwt_agent::Session::load_and_migrate(&session_path) {
            Ok(session) => session,
            Err(_) => {
                return reply_error(
                    "Session metadata is missing; restart via Start Work or Launch Agent."
                        .to_string(),
                );
            }
        };
        if let Some(window_id) = self.live_agent_window_for_work(
            &tab_id,
            (!session.branch.trim().is_empty()).then_some(session.branch.as_str()),
            Some(session.worktree_path.as_path()),
        ) {
            // D1: the matched live window may belong to a *different* Work on the
            // same branch/worktree. Resolve its gwt session id and apply the same
            // conversation-conflict guard before focusing.
            let live_gwt_session = self
                .active_agent_sessions
                .iter()
                .find(|(candidate, _)| candidate.as_str() == window_id.as_str())
                .map(|(_, live)| live.session_id.clone());
            if let Some(live_gwt_session) = live_gwt_session {
                if self
                    .resume_conversation_conflicts(agent_session_id.as_deref(), &live_gwt_session)
                {
                    return reply_error(
                    "This Work is currently running a different conversation; stop it before resuming an older Session.".to_string(),
                );
                }
            }
            let mut events = self.focus_existing_live_work_agent_events(&window_id, Some(bounds));
            events.push(started_ack(
                &session_id,
                (!session.branch.trim().is_empty()).then(|| session.branch.clone()),
            ));
            return events;
        }

        let session_worktree_exists = session.worktree_path.as_path().exists();
        if !session_exact_resume_materializable(&project_root, &session) {
            return reply_error(
                "This Session cannot be resumed on this machine because its branch is no longer available; use Workspace Continue or Launch Agent to start a new Work.".to_string(),
            );
        }

        // Build a fresh LaunchConfig from the persisted Session and add the
        // resume_session_id only when the agent CLI captured a previous
        // conversation handle (Claude / Codex / opt-in custom agents).
        let agent_id = session.agent_id.clone();
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id.clone());
        if session_worktree_exists {
            builder = builder.working_dir(session.worktree_path.clone());
        }
        if !session.branch.is_empty() {
            builder = builder.branch(session.branch.clone());
        }
        if let Some(model) = session.model.clone() {
            builder = builder.model(model);
        }
        if let Some(version) = session.tool_version.clone() {
            builder = builder.version(version);
        }
        if let Some(level) = session.reasoning_level.clone() {
            builder = builder.reasoning_level(level);
        }
        if session.skip_permissions {
            builder = builder.skip_permissions(true);
        }
        if session.fast_mode_enabled() {
            builder = builder.fast_mode(true);
        }
        builder = builder.runtime_target(session.runtime_target);
        if let Some(service) = session.docker_service.clone() {
            builder = builder.docker_service(service);
        }
        builder = builder.docker_lifecycle_intent(session.docker_lifecycle_intent);
        if let Some(shell) = session.windows_shell {
            builder = builder.windows_shell(shell);
        }
        if let Some(linked) = session.linked_issue_number {
            builder = builder.linked_issue_number(linked);
        }

        // Resume the specific Session (conversation UUID) the user clicked when
        // one was requested; otherwise resume the Work's latest conversation.
        if let Some(resume_id) = session.resume_session_id_for(agent_session_id.as_deref()) {
            builder = builder
                .session_mode(gwt_agent::SessionMode::Resume)
                .resume_session_id(resume_id);
        } else if agent_session_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        {
            return reply_error(
                "The requested Session id is not resumable; use the Work Resume picker or start a new Work.".to_string(),
            );
        } else if session.agent_id.supports_resume_picker() {
            builder = builder.session_mode(gwt_agent::SessionMode::Resume);
        } else {
            // Legacy metadata-only rows for agents without a native resume
            // picker remain a fresh start fallback. Claude Code / Codex use
            // their provider-native picker instead of silently losing Resume.
            builder = builder.session_mode(gwt_agent::SessionMode::Normal);
        }

        let mut config = builder.build();
        // Preserve persisted tool version + display name so the launcher
        // does not re-derive them from version cache (mirrors Quick Start
        // Resume behavior).
        if let Some(version) = session.tool_version.clone() {
            config.tool_version = Some(version);
        }
        if !session.display_name.is_empty() {
            config.display_name = session.display_name.clone();
        }

        // Build a Workspace Resume context so the spawned window's title
        // and the Workspace projection summary keep the prior identity
        // instead of falling back to the agent's default display name.
        // #3065: the context comes from the resumed branch's own Work item,
        // never from the repo-shared current projection.
        let resume_context_root = if session_worktree_exists {
            session.worktree_path.as_path()
        } else {
            project_root.as_path()
        };
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            resume_context_root,
            Some(session.branch.as_str()),
            resume_context_root,
        ));

        match self.spawn_agent_window(&tab_id, config, bounds, workspace_resume_context) {
            Ok(mut events) => {
                events.push(started_ack(
                    &session_id,
                    (!session.branch.trim().is_empty()).then(|| session.branch.clone()),
                ));
                events
            }
            Err(error) => reply_error(error),
        }
    }

    /// SPEC-2359 D1: true when a specific conversation was requested for resume
    /// but the live window a focus would land on is running a *different*
    /// conversation. `live_gwt_session_id` is the gwt session id (Work) owning
    /// that live window; its Session TOML's latest `agent_session_id` is the
    /// conversation it is currently running. Returns false when no specific
    /// conversation was requested (a plain Work resume is satisfied by focus).
    fn resume_conversation_conflicts(
        &self,
        requested: Option<&str>,
        live_gwt_session_id: &str,
    ) -> bool {
        let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
            return false;
        };
        let live_conversation = {
            let path = self
                .sessions_dir
                .join(format!("{live_gwt_session_id}.toml"));
            gwt_agent::Session::load_and_migrate(&path)
                .ok()
                .and_then(|session| session.agent_session_id)
        };
        live_conversation.as_deref().map(str::trim) != Some(requested)
    }

    pub(crate) fn resume_branch_latest_agent_events(
        &mut self,
        client_id: &str,
        id: &str,
        branch_name: &str,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let branch_error = |message: String| {
            vec![OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message,
                },
            )]
        };

        let Some(address) = self.window_lookup.get(id).cloned() else {
            return branch_error("Window not found".to_string());
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return branch_error("Project tab not found".to_string());
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return branch_error("Window not found".to_string());
        };
        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            tracing::warn!(
                preset = ?window.preset,
                window_id = id,
                "resume_branch_latest_agent rejected: wrong preset"
            );
            return branch_error(format!(
                "Window preset {:?} is not a Work surface",
                window.preset
            ));
        }
        if tab.kind != gwt::ProjectKind::Git {
            return branch_error("Resume requires a Git project".to_string());
        }
        if tab.migration_pending {
            return branch_error(
                "Complete the project migration before resuming an agent".to_string(),
            );
        }

        let tab_id = address.tab_id.clone();
        let project_root = tab.project_root.clone();
        let normalized_branch_name = normalize_branch_name(branch_name);
        // SPEC-2359 W-17 (FR-398): client-scoped ack so the requesting
        // frontend can settle its pending Resume UI deterministically.
        let started_ack = |session_id: String, branch: String| {
            OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::WorkspaceResumeAgentStarted {
                    session_id,
                    branch: Some(branch),
                },
            )
        };
        if let Some(window_id) =
            self.live_agent_window_for_work(&tab_id, Some(&normalized_branch_name), None)
        {
            let live_session_id = self
                .active_agent_sessions
                .get(&window_id)
                .map(|session| session.session_id.clone())
                .unwrap_or_default();
            let mut events =
                self.focus_existing_live_work_agent_events(&window_id, Some(bounds.clone()));
            events.push(started_ack(live_session_id, normalized_branch_name.clone()));
            return events;
        }
        let Some(session) =
            self.latest_resumable_branch_session(&project_root, &normalized_branch_name)
        else {
            return branch_error(format!(
                "No resumable session found for {normalized_branch_name}"
            ));
        };

        if let Some(window_id) = self
            .active_agent_sessions
            .iter()
            .find(|(_, active)| active.session_id == session.id)
            .map(|(window_id, _)| window_id.clone())
        {
            if !self.window_lookup.contains_key(&window_id) {
                return branch_error(format!("Agent window not found for session {}", session.id));
            }
            let mut events = self.focus_window_events(&window_id, Some(bounds));
            if events.is_empty() {
                events.push(self.workspace_state_broadcast());
            }
            events.push(started_ack(
                session.id.clone(),
                normalized_branch_name.clone(),
            ));
            return events;
        }

        if !session_exact_resume_materializable(&project_root, &session) {
            return branch_error(format!(
                "No resumable session found for {normalized_branch_name}"
            ));
        }
        let mut config = super::launch_config_from_persisted_session(&session);
        if !session.worktree_path.as_path().exists() {
            config.working_dir = None;
        }
        if config.session_mode != gwt_agent::SessionMode::Resume {
            return branch_error(format!(
                "No resumable session found for {normalized_branch_name}"
            ));
        }
        // #3065: the context comes from the resumed branch's own Work item,
        // never from the repo-shared current projection.
        let resume_context_root = if session.worktree_path.as_path().exists() {
            session.worktree_path.as_path()
        } else {
            project_root.as_path()
        };
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            resume_context_root,
            Some(session.branch.as_str()),
            resume_context_root,
        ));

        match self.spawn_agent_window(&tab_id, config, bounds, workspace_resume_context) {
            Ok(mut events) => {
                events.push(started_ack(
                    session.id.clone(),
                    normalized_branch_name.clone(),
                ));
                events
            }
            Err(error) => branch_error(error),
        }
    }

    /// Build a list of agents that the Workspace Resume picker can offer
    /// for the currently-active Git project tab. Includes live agents with
    /// `lifecycle_status = Running` so the picker can show them and focus
    /// their window on click. Non-live entries require a backing Session
    /// toml on disk.
    fn collect_resumable_agents(&self, workspace_id: Option<&str>) -> Vec<gwt::ResumableAgentView> {
        let Some(tab_id) = self.active_tab_id.as_deref() else {
            return Vec::new();
        };
        let Some(tab) = self.tab(tab_id) else {
            return Vec::new();
        };
        if tab.kind != gwt::ProjectKind::Git {
            return Vec::new();
        }
        let live_session_ids: std::collections::HashSet<&str> = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .map(|session| session.session_id.as_str())
            .collect();

        let project_root = tab.project_root.clone();
        let Ok(Some(projection)) =
            gwt_core::workspace_projection::load_workspace_projection(&project_root)
        else {
            return Vec::new();
        };

        let sessions_dir = self.sessions_dir.clone();

        let workspace_work_item = workspace_id.and_then(|wid| {
            gwt_core::workspace_projection::load_workspace_work_items(&project_root)
                .ok()
                .flatten()
                .and_then(|items| items.work_items.into_iter().find(|item| item.id == wid))
        });
        let work_item_session_ids: Option<std::collections::HashSet<String>> =
            workspace_work_item.as_ref().map(|item| {
                item.agents
                    .iter()
                    .map(|agent| agent.session_id.clone())
                    .collect()
            });
        let work_item_branch = workspace_work_item.as_ref().and_then(|item| {
            item.execution_containers
                .iter()
                .filter_map(|container| container.branch.as_deref())
                .map(str::trim)
                .find(|branch| !branch.is_empty())
                .map(str::to_string)
        });

        let resume_kind_for_session = |session: &gwt_agent::Session| {
            if session.exact_resume_session_id().is_some() {
                gwt::ResumableAgentResumeKind::Session
            } else if session.agent_id.supports_resume_picker() {
                gwt::ResumableAgentResumeKind::NativePicker
            } else {
                gwt::ResumableAgentResumeKind::MetadataOnly
            }
        };
        let lifecycle_status_for_session = |session: &gwt_agent::Session| {
            if session.should_mark_interrupted_from_lifecycle()
                || session.status == gwt_agent::AgentStatus::Interrupted
            {
                Some(gwt::ResumableAgentLifecycleStatus::Interrupted)
            } else if session.exact_auto_resume_candidate() {
                Some(gwt::ResumableAgentLifecycleStatus::Active)
            } else {
                None
            }
        };

        let mut entries: Vec<gwt::ResumableAgentView> = projection
            .agents
            .iter()
            .filter(|agent| !agent.session_id.trim().is_empty())
            .filter(|agent| match &work_item_session_ids {
                Some(ids) => ids.contains(&agent.session_id),
                None => true,
            })
            .filter_map(|agent| {
                let is_live = live_session_ids.contains(agent.session_id.as_str());
                let (resume_kind, lifecycle_status) = if is_live {
                    (
                        gwt::ResumableAgentResumeKind::Session,
                        Some(gwt::ResumableAgentLifecycleStatus::Running),
                    )
                } else {
                    let session_path = sessions_dir.join(format!("{}.toml", agent.session_id));
                    match gwt_agent::Session::load_and_migrate(&session_path) {
                        Ok(session) => {
                            if !session_exact_resume_materializable(&project_root, &session) {
                                return None;
                            }
                            (
                                resume_kind_for_session(&session),
                                lifecycle_status_for_session(&session),
                            )
                        }
                        Err(_) => return None,
                    }
                };
                Some(gwt::ResumableAgentView {
                    session_id: agent.session_id.clone(),
                    agent_id: agent.agent_id.clone(),
                    display_name: agent.display_name.clone(),
                    branch: agent.branch.clone(),
                    worktree_path: agent
                        .worktree_path
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    last_activity_at: Some(agent.updated_at.to_rfc3339()),
                    resume_kind,
                    lifecycle_status,
                })
            })
            .collect();

        if let Some(branch) = work_item_branch.as_deref() {
            let agent_sessions = self
                .session_ledger_cache
                .borrow_mut()
                .load(&self.sessions_dir);
            let project_repo_hash = gwt_core::repo_hash::detect_repo_hash(&project_root);
            let registry = crate::workspace_session_registry::branch_session_registry(
                &agent_sessions,
                project_repo_hash.as_ref().map(|hash| hash.as_str()),
            );
            let existing_session_ids: Vec<&str> = entries
                .iter()
                .map(|entry| entry.session_id.as_str())
                .collect();
            let (branch_sessions, _) =
                crate::workspace_session_registry::registry_sessions_for_branch(
                    &registry,
                    Some(branch),
                    &existing_session_ids,
                    crate::workspace_session_registry::REGISTRY_SESSION_CAP,
                );
            for session in branch_sessions {
                if !session_exact_resume_materializable(&project_root, session) {
                    continue;
                }
                if entries.iter().any(|entry| entry.session_id == session.id) {
                    continue;
                }
                entries.push(gwt::ResumableAgentView {
                    session_id: session.id.clone(),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.display_name.clone(),
                    branch: (!session.branch.trim().is_empty()).then(|| session.branch.clone()),
                    worktree_path: Some(session.worktree_path.display().to_string()),
                    last_activity_at: Some(
                        session
                            .last_activity_at
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    ),
                    resume_kind: resume_kind_for_session(session),
                    lifecycle_status: lifecycle_status_for_session(session),
                });
            }
        }

        entries.sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
        entries
    }

    pub(crate) fn open_start_work_for_project(
        &mut self,
        tab_id: &str,
        project_root: &Path,
    ) -> Result<(), String> {
        self.open_start_work_for_project_with_context(tab_id, project_root, None)
    }

    pub(crate) fn open_start_work_for_project_with_context(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<(), String> {
        let base_branch = gwt::start_work::START_WORK_BASE_BRANCH_CANDIDATES[0].to_string();
        let work_branch =
            gwt::start_work::reserve_start_work_branch_name_for_project(project_root, Utc::now())
                .map_err(|error| error.to_string())?;
        let quick_start_root = project_root.to_path_buf();
        let quick_start_entries = Vec::new();
        let previous_profiles = self.launch_wizard_cache.agent_preferences();
        let agent_options = self.launch_wizard_cache.agent_options();
        let docker_context = None;
        let docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        let wizard_id = Uuid::new_v4().to_string();
        let mut wizard = LaunchWizardState::open_start_work_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(&base_branch),
                normalized_branch_name: work_branch,
                worktree_path: None,
                quick_start_root,
                live_sessions: Vec::new(),
                docker_context,
                docker_service_status,
                linked_issue_number: workspace_resume_context.as_ref().and_then(|context| {
                    workspace_resume_owner_issue_number(context.owner.as_deref())
                }),
                linked_issue_kind: None,
                ultracode_supported: gwt_agent::claude_ultracode_supported(),
                claude_workflows_enabled: gwt_agent::claude_workflows_enabled(),
            },
            base_branch,
            agent_options,
            quick_start_entries,
            previous_profiles,
        );
        wizard.mark_runtime_context_unresolved();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard,
            workspace_resume_context,
            agent_kanban_target: None,
        });

        Ok(())
    }

    pub(crate) fn open_issue_launch_wizard_events(
        &mut self,
        client_id: &str,
        id: &str,
        issue_number: u64,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, KnowledgeKind::Issue, "Window not found", None, None),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Project tab not found",
                    None,
                    None,
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, KnowledgeKind::Issue, "Window not found", None, None),
            )];
        };
        let Some(kind) = knowledge_kind_for_preset(window.preset) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Window is not a knowledge bridge",
                    None,
                    None,
                ),
            )];
        };
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Complete the project migration before launching from an Issue",
                    None,
                    None,
                ),
            )];
        }

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id = id.to_string();
        let active_session_branches = self.active_session_branches_for_tab(&address.tab_id);
        thread::spawn(move || {
            let result =
                list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
                    .map_err(|error| error.to_string())
                    .and_then(|entries| {
                        preferred_issue_launch_branch(&entries)
                            .ok_or_else(|| "No local branch is available for launch".to_string())
                    });
            proxy.send(UserEvent::IssueLaunchWizardPrepared(
                IssueLaunchWizardPrepared {
                    client_id,
                    id,
                    knowledge_kind: kind,
                    tab_id,
                    project_root,
                    issue_number,
                    result,
                },
            ));
        });
        Vec::new()
    }

    pub(crate) fn handle_issue_launch_wizard_prepared(
        &mut self,
        prepared: IssueLaunchWizardPrepared,
    ) -> Vec<OutboundEvent> {
        let IssueLaunchWizardPrepared {
            client_id,
            id,
            knowledge_kind,
            tab_id,
            project_root,
            issue_number,
            result,
        } = prepared;
        if self.tab(&tab_id).is_none() {
            return vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(id, knowledge_kind, "Project tab not found", None, None),
            )];
        }

        match result {
            Ok(base_branch_name) => {
                let Some(linked_issue_kind) = linked_issue_kind_from_knowledge(knowledge_kind)
                else {
                    return vec![OutboundEvent::reply(
                        &client_id,
                        knowledge_error_event(
                            id,
                            knowledge_kind,
                            "Launch Agent is not available for this knowledge bridge",
                            None,
                            None,
                        ),
                    )];
                };
                match self.open_knowledge_launch_wizard_for_base_branch(
                    &tab_id,
                    &project_root,
                    &base_branch_name,
                    issue_number,
                    linked_issue_kind,
                ) {
                    Ok(()) => vec![self.launch_wizard_state_outbound()],
                    Err(error) => vec![OutboundEvent::reply(
                        &client_id,
                        knowledge_error_event(id, knowledge_kind, error, None, None),
                    )],
                }
            }
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(id, knowledge_kind, error, None, None),
            )],
        }
    }

    #[cfg(test)]
    pub(crate) fn handle_launch_wizard_action(
        &mut self,
        action: gwt::LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        self.handle_launch_wizard_action_for_client(None, action, bounds)
    }

    pub(crate) fn handle_launch_wizard_action_for_client(
        &mut self,
        client_id: Option<&str>,
        action: gwt::LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(mut session) = self.launch_wizard.take() else {
            return Vec::new();
        };
        let action_stage = Self::launch_wizard_action_error_stage(&action);
        let action_label = Self::launch_wizard_action_label(&action);
        let requested_agent_id = match &action {
            gwt::LaunchWizardAction::SetAgent { agent_id } => Some(agent_id.clone()),
            _ => None,
        };
        session.wizard.apply(action);
        if let Some(error) = session.wizard.error.as_deref() {
            Self::log_launch_wizard_error(
                &session,
                action_stage,
                action_label,
                requested_agent_id.as_deref(),
                error,
            );
        }

        match session.wizard.completion.take() {
            Some(LaunchWizardCompletion::Cancelled) => {
                vec![self.launch_wizard_state_broadcast(None)]
            }
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    let error = "The selected session window is no longer available".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                let Some(tab) = self.tab_mut(&address.tab_id) else {
                    let error = "Project tab not found".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                if !tab.workspace.focus_window(&address.raw_id, None) {
                    let error = "The selected session window is no longer available".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                }
                self.active_tab_id = Some(address.tab_id);
                let _ = self.persist();
                vec![
                    self.workspace_state_broadcast(),
                    self.launch_wizard_state_broadcast(None),
                ]
            }
            Some(LaunchWizardCompletion::ResolveRuntime(config)) => {
                let Some(project_root) = self
                    .tab(&session.tab_id)
                    .map(|tab| tab.project_root.clone())
                else {
                    let error = "Project tab not found".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "resolve_runtime",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                let wizard_id = session.wizard_id.clone();
                let branch_name = session.wizard.branch_name.clone();
                let cache = self.launch_wizard_cache.clone();
                let proxy = self.proxy.clone();
                session
                    .wizard
                    .mark_runtime_resolution_pending("Preparing runtime context...");
                thread::spawn(move || {
                    let result = resolve_launch_wizard_runtime_context_hydration(
                        &project_root,
                        *config,
                        branch_name,
                        cache,
                    );
                    proxy.send(UserEvent::LaunchWizardRuntimeResolved {
                        wizard_id,
                        result: Box::new(result),
                    });
                });
                self.launch_wizard = Some(session);
                vec![self.launch_wizard_state_outbound()]
            }
            Some(LaunchWizardCompletion::Launch(config)) => {
                let Some(bounds) = bounds else {
                    let error = "Viewport bounds are required to launch a window".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "launch_bounds",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                match *config {
                    LaunchWizardLaunchRequest::Agent(config) => {
                        let workspace_resume_context = session.workspace_resume_context.clone();
                        let launch_feedback_context =
                            client_id.map(|client_id| LaunchFeedbackContext {
                                client_id: client_id.to_string(),
                                title: if session.wizard.wizard_mode
                                    == gwt::LaunchWizardMode::StartWork
                                {
                                    "Start Work".to_string()
                                } else {
                                    "Launch Agent".to_string()
                                },
                            });
                        let spawn_result = if let Some(target) = session.agent_kanban_target.clone()
                        {
                            self.spawn_agent_window_in_agent_kanban(
                                &session.tab_id,
                                *config,
                                bounds,
                                workspace_resume_context,
                                launch_feedback_context,
                                target,
                            )
                        } else if let Some(launch_feedback_context) = launch_feedback_context {
                            self.spawn_agent_window_with_feedback(
                                &session.tab_id,
                                *config,
                                bounds,
                                workspace_resume_context,
                                launch_feedback_context,
                            )
                        } else {
                            self.spawn_agent_window(
                                &session.tab_id,
                                *config,
                                bounds,
                                workspace_resume_context,
                            )
                        };
                        match spawn_result {
                            Ok(mut events) => {
                                events.insert(0, self.launch_wizard_state_broadcast(None));
                                events
                            }
                            Err(error) => {
                                Self::log_launch_wizard_error(
                                    &session,
                                    "spawn_agent_window",
                                    action_label,
                                    requested_agent_id.as_deref(),
                                    &error,
                                );
                                session.wizard.error = Some(error);
                                self.launch_wizard = Some(session);
                                vec![self.launch_wizard_state_outbound()]
                            }
                        }
                    }
                    LaunchWizardLaunchRequest::Shell(config) => {
                        match self.spawn_wizard_shell_window(&session.tab_id, *config, bounds) {
                            Ok(mut events) => {
                                events.insert(0, self.launch_wizard_state_broadcast(None));
                                events
                            }
                            Err(error) => {
                                Self::log_launch_wizard_error(
                                    &session,
                                    "spawn_shell_window",
                                    action_label,
                                    requested_agent_id.as_deref(),
                                    &error,
                                );
                                session.wizard.error = Some(error);
                                self.launch_wizard = Some(session);
                                vec![self.launch_wizard_state_outbound()]
                            }
                        }
                    }
                }
            }
            None => {
                self.launch_wizard = Some(session);
                vec![self.launch_wizard_state_outbound()]
            }
        }
    }

    pub(crate) fn handle_launch_wizard_runtime_resolved(
        &mut self,
        wizard_id: String,
        result: Result<LaunchWizardHydration, String>,
    ) -> Vec<OutboundEvent> {
        let Some(mut session) = self.launch_wizard.take() else {
            return Vec::new();
        };
        if session.wizard_id != wizard_id {
            self.launch_wizard = Some(session);
            return Vec::new();
        }
        match result {
            Ok(hydration) => {
                session.wizard.apply_runtime_context(hydration);
            }
            Err(error) => {
                Self::log_launch_wizard_error(
                    &session,
                    "resolve_runtime",
                    "runtime_resolved",
                    None,
                    &error,
                );
                session.wizard.set_hydration_error(error);
            }
        }
        self.launch_wizard = Some(session);
        vec![self.launch_wizard_state_outbound()]
    }
}

fn resolve_launch_wizard_runtime_context_hydration(
    project_root: &Path,
    _config: LaunchWizardLaunchRequest,
    branch_name: String,
    cache: LaunchWizardMemoryCache,
) -> Result<LaunchWizardHydration, String> {
    let (context_path, resolved_worktree_path) =
        launch_runtime_context_paths(project_root, &branch_name);
    let quick_start_entries = cache.quick_start_entries(&context_path, &branch_name);
    let previous_profiles = cache.previous_profiles(&context_path);
    let agent_options = cache.agent_options();
    let (docker_context, docker_service_status) =
        detect_wizard_docker_context_and_status(&context_path);
    Ok(LaunchWizardHydration {
        selected_branch: None,
        normalized_branch_name: branch_name,
        worktree_path: resolved_worktree_path,
        quick_start_root: context_path,
        docker_context,
        docker_service_status,
        agent_options,
        quick_start_entries,
        previous_profiles: Some(previous_profiles),
    })
}

fn launch_runtime_context_paths(
    project_root: &Path,
    branch_name: &str,
) -> (PathBuf, Option<PathBuf>) {
    let worktrees = launch_runtime_worktrees(project_root);
    if let Some(worktree_path) = worktrees
        .as_deref()
        .and_then(|worktrees| usable_worktree_path_for_branch(worktrees, branch_name))
    {
        return (worktree_path.clone(), Some(worktree_path));
    }
    if project_root_is_git_worktree(project_root) {
        return (project_root.to_path_buf(), None);
    }
    if let Some(default_worktree_path) = worktrees
        .as_deref()
        .and_then(default_runtime_detection_worktree_path)
    {
        return (default_worktree_path, None);
    }
    (project_root.to_path_buf(), None)
}

fn current_branch_name_for_launch_agent(project_root: &Path) -> Result<String, String> {
    let repository = gwt_git::Repository::open(project_root)
        .map_err(|error| format!("Current branch is unavailable: {error}"))?;
    let branch = repository
        .current_branch()
        .map_err(|error| format!("Current branch is unavailable: {error}"))?
        .ok_or_else(|| "Launch Agent requires a checked-out branch".to_string())?;
    let branch = normalize_branch_name(&branch);
    if branch.is_empty() {
        Err("Launch Agent requires a checked-out branch".to_string())
    } else {
        Ok(branch)
    }
}

fn launch_runtime_worktrees(project_root: &Path) -> Option<Vec<gwt_git::WorktreeInfo>> {
    let main_repo_path = gwt_git::worktree::main_worktree_root(project_root).ok()?;
    gwt_git::WorktreeManager::new(&main_repo_path).list().ok()
}

fn project_root_is_git_worktree(project_root: &Path) -> bool {
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(project_root)
        .output();
    output.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
    })
}

fn default_runtime_detection_worktree_path(worktrees: &[gwt_git::WorktreeInfo]) -> Option<PathBuf> {
    ["develop", "main"]
        .iter()
        .find_map(|branch| usable_worktree_path_for_branch(worktrees, branch))
}

impl AppRuntime {
    pub(crate) fn spawn_wizard_shell_window(
        &mut self,
        tab_id: &str,
        config: ShellLaunchConfig,
        bounds: WindowGeometry,
    ) -> Result<Vec<OutboundEvent>, String> {
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root = tab.project_root.display().to_string();
        let project_root_path = tab.project_root.clone();
        let title = format!(
            "{} · {}",
            config.display_name,
            config.branch.as_ref().unwrap_or(&"work".to_string())
        );
        let window = tab
            .workspace
            .add_window_with_title(WindowPreset::Shell, title, false, bounds);
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);

        // SPEC-2359 US-80 (FR-427): register the Start-Work Shell as a
        // first-class Work so it appears in the Active Work / Workspace
        // projection like an agent. `config.branch` is set even for new
        // branches, so the branch-derived Work id is stable before the worktree
        // exists; `config.working_dir` is `None` until the async launch creates
        // a new-branch worktree.
        let live_session_ids: std::collections::HashSet<String> = self
            .active_agent_sessions
            .values()
            .map(|session| session.session_id.clone())
            .collect();
        let shell_work_registered = match save_shell_work_projection(
            &project_root_path,
            &window_id,
            config.working_dir.clone(),
            config.branch.clone(),
            &live_session_ids,
        ) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root_path.display(),
                    window_id = %window_id,
                    error = %error,
                    "shell Work projection registration skipped"
                );
                false
            }
        };

        let mut events = vec![self.workspace_state_broadcast()];
        if shell_work_registered && self.active_tab_id.as_deref() == Some(tab_id) {
            if let Some(tab) = self.tab(tab_id) {
                if let Some(projection) = self.active_work_projection_for_tab(tab_id, tab) {
                    events.push(OutboundEvent::broadcast(
                        BackendEvent::ActiveWorkProjection {
                            projection: Box::new(projection),
                        },
                    ));
                }
            }
        }
        events.extend(Self::status_events(
            window_id.clone(),
            WindowProcessStatus::Running,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.clone();
        let profile_config_path = self.profile_config_path()?;
        thread::spawn(move || {
            Self::spawn_wizard_shell_window_async(
                proxy,
                project_root,
                window_id,
                config,
                profile_config_path,
            );
        });

        Ok(events)
    }

    pub(crate) fn spawn_wizard_shell_window_async(
        proxy: AppEventProxy,
        project_root: String,
        window_id: String,
        mut config: ShellLaunchConfig,
        profile_config_path: PathBuf,
    ) {
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_shell_launch_worktree(Path::new(&project_root), &mut config)?;
            let worktree_path = config
                .working_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(&project_root));
            gwt_agent::LaunchEnvironment::from_active_profile(
                &profile_config_path,
                config.runtime_target,
            )?
            .with_project_root(&worktree_path)
            .apply_to_parts(&mut config.env_vars, &mut config.remove_env);

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                proxy.send(UserEvent::LaunchProgress {
                    window_id: window_id.clone(),
                    message: "Starting Docker service...".to_string(),
                });
            }

            build_shell_process_launch(Path::new(&project_root), &mut config)
        })();

        proxy.send(UserEvent::ShellLaunchComplete { window_id, result });
    }

    pub(super) fn refresh_open_launch_wizard_from_cache(&mut self) {
        let Some(session) = self.launch_wizard.as_mut() else {
            return;
        };
        let context = session.wizard.context.clone();
        let agent_options = self.launch_wizard_cache.agent_options();
        let quick_start_entries = self
            .launch_wizard_cache
            .quick_start_entries(&context.quick_start_root, &context.normalized_branch_name);
        session.wizard.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(context.selected_branch),
            normalized_branch_name: context.normalized_branch_name,
            worktree_path: context.worktree_path,
            quick_start_root: context.quick_start_root,
            docker_context: context.docker_context,
            docker_service_status: context.docker_service_status,
            agent_options,
            quick_start_entries,
            previous_profiles: None,
        });
    }
}
