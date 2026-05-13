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
    KnowledgeKind, LaunchWizardCompletion, LaunchWizardContext, LaunchWizardHydration,
    LaunchWizardLaunchRequest, LaunchWizardState, LaunchWizardView, LinkedIssueKind,
    WindowGeometry,
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
    branch_worktree_path, build_shell_process_launch, combined_window_id,
    detect_wizard_docker_context_and_status, knowledge_error_event, knowledge_kind_for_preset,
    list_branch_entries_with_active_sessions, normalize_branch_name, preferred_issue_launch_branch,
    resolve_shell_launch_worktree, synthetic_branch_entry, workspace_projection_for_current_resume,
    workspace_resume_branch_exists, workspace_resume_branch_from_journal_project_root,
    workspace_resume_context_from_journal, workspace_resume_context_from_projection,
    workspace_resume_owner_issue_number, AppEventProxy, AppRuntime, BackendEvent,
    IssueLaunchWizardPrepared, LaunchWizardSession, OutboundEvent, WindowPreset,
    WindowProcessStatus, WorkspaceResumeContext, WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
};

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

        if window.preset != WindowPreset::Branches {
            return launch_agent_open_error(client_id, "Window is not a branches list");
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
        let worktree_path = branch_worktree_path(project_root, &normalized_branch_name);
        let quick_start_root = worktree_path
            .clone()
            .unwrap_or_else(|| project_root.to_path_buf());
        let quick_start_entries = self
            .launch_wizard_cache
            .quick_start_entries(&quick_start_root, &normalized_branch_name);
        let previous_profiles = self
            .launch_wizard_cache
            .previous_profiles(&quick_start_root);
        let agent_options = self.launch_wizard_cache.agent_options();
        let (docker_context, docker_service_status) =
            detect_wizard_docker_context_and_status(&quick_start_root);
        let wizard_id = Uuid::new_v4().to_string();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard: LaunchWizardState::open_with_previous_profiles(
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
                },
                agent_options,
                quick_start_entries,
                previous_profiles,
            ),
            workspace_resume_context,
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

    pub(crate) fn resume_workspace_events(
        &mut self,
        source: gwt::WorkspaceResumeSource,
        journal_id: Option<String>,
    ) -> Vec<OutboundEvent> {
        let error_event = |message: &str| {
            vec![OutboundEvent::broadcast(BackendEvent::ProjectOpenError {
                message: message.to_string(),
            })]
        };

        let Some(tab_id) = self.active_tab_id.clone() else {
            return error_event("Open a project before resuming work");
        };
        let Some(tab) = self.tab(&tab_id) else {
            return error_event("Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return error_event("Resume Workspace requires a Git project");
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
                        title: Some(format!("{tab_title} workspace")),
                        owner: None,
                        summary: None,
                        next_action: None,
                    });
                (branch, context)
            }
            gwt::WorkspaceResumeSource::Journal => {
                let Some(journal_id) = journal_id else {
                    return error_event("Workspace journal id is required");
                };
                let Ok(entries) =
                    gwt_core::workspace_projection::load_recent_workspace_journal_entries(
                        &project_root,
                        WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
                    )
                else {
                    return error_event("Workspace journal could not be loaded");
                };
                let Some(entry) = entries.into_iter().find(|entry| entry.id == journal_id) else {
                    return error_event("Workspace journal entry not found");
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
        let base_branch = gwt::start_work::resolve_start_work_base_branch(project_root)
            .map_err(|error| error.to_string())?;
        let work_branch =
            gwt::start_work::reserve_start_work_branch_name_for_project(project_root, Utc::now())
                .map_err(|error| error.to_string())?;
        let quick_start_root = project_root.to_path_buf();
        let quick_start_entries = self
            .launch_wizard_cache
            .quick_start_entries(&quick_start_root, &work_branch);
        let previous_profiles = self
            .launch_wizard_cache
            .previous_profiles(&quick_start_root);
        let agent_options = self.launch_wizard_cache.agent_options();
        let (docker_context, docker_service_status) =
            detect_wizard_docker_context_and_status(&quick_start_root);
        let wizard_id = Uuid::new_v4().to_string();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard: LaunchWizardState::open_start_work_with_previous_profiles(
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
                },
                base_branch,
                agent_options,
                quick_start_entries,
                previous_profiles,
            ),
            workspace_resume_context,
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
            Ok(branch_name) => match self.open_launch_wizard_for_branch(
                &tab_id,
                &project_root,
                &branch_name,
                Some(issue_number),
                linked_issue_kind_from_knowledge(knowledge_kind),
            ) {
                Ok(()) => vec![self.launch_wizard_state_outbound()],
                Err(error) => vec![OutboundEvent::reply(
                    &client_id,
                    knowledge_error_event(id, knowledge_kind, error, None, None),
                )],
            },
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(id, knowledge_kind, error, None, None),
            )],
        }
    }

    pub(crate) fn handle_launch_wizard_action(
        &mut self,
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
                        match self.spawn_agent_window(
                            &session.tab_id,
                            *config,
                            bounds,
                            workspace_resume_context,
                        ) {
                            Ok(mut events) => {
                                events.push(self.launch_wizard_state_broadcast(None));
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
                                events.push(self.launch_wizard_state_broadcast(None));
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
        let title = format!(
            "{} · {}",
            config.display_name,
            config.branch.as_ref().unwrap_or(&"workspace".to_string())
        );
        let window = tab
            .workspace
            .add_window_with_title(WindowPreset::Shell, title, false, bounds);
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);

        let mut events = vec![self.workspace_state_broadcast()];
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
