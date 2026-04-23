use super::*;

#[derive(Debug, Clone)]
pub(super) struct LaunchWizardSession {
    pub(super) tab_id: String,
    pub(super) wizard_id: String,
    pub(super) wizard: LaunchWizardState,
}

#[derive(Debug, Clone)]
pub(super) struct IssueLaunchWizardPrepared {
    pub(super) client_id: ClientId,
    pub(super) id: String,
    pub(super) knowledge_kind: KnowledgeKind,
    pub(super) tab_id: String,
    pub(super) project_root: PathBuf,
    pub(super) issue_number: u64,
    pub(super) result: Result<String, String>,
}

impl AppRuntime {
    pub(super) fn open_launch_wizard(
        &mut self,
        id: &str,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            })];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            })];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            })];
        };

        if window.preset != WindowPreset::Branches {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window is not a branches list".to_string(),
            })];
        }

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        match self.open_launch_wizard_for_branch(
            &tab_id,
            &project_root,
            branch_name,
            linked_issue_number,
        ) {
            Ok(()) => vec![self.launch_wizard_state_outbound()],
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: error,
            })],
        }
    }

    fn open_launch_wizard_for_branch(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Result<(), String> {
        let normalized_branch_name = normalize_branch_name(branch_name);
        let live_sessions = self.live_sessions_for_branch(tab_id, &normalized_branch_name);
        let wizard_id = Uuid::new_v4().to_string();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id: wizard_id.clone(),
            wizard: LaunchWizardState::open_loading(
                LaunchWizardContext {
                    selected_branch: synthetic_branch_entry(branch_name),
                    normalized_branch_name,
                    worktree_path: None,
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions,
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number,
                },
                Vec::new(),
            ),
        });

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let project_root = project_root.to_path_buf();
        let branch_name = branch_name.to_string();
        let active_session_branches = self.active_session_branches_for_tab(tab_id);
        thread::spawn(move || {
            let result = resolve_launch_wizard_hydration(
                &project_root,
                &branch_name,
                &active_session_branches,
                &sessions_dir,
            );
            proxy.send(UserEvent::LaunchWizardHydrated { wizard_id, result });
        });

        Ok(())
    }

    pub(super) fn open_issue_launch_wizard_events(
        &mut self,
        client_id: &str,
        id: &str,
        issue_number: u64,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(kind) = knowledge_kind_for_preset(window.preset) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Window is not a knowledge bridge".to_string(),
                },
            )];
        };

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

    pub(super) fn handle_launch_wizard_hydrated(
        &mut self,
        wizard_id: String,
        result: Result<LaunchWizardHydration, String>,
    ) -> Vec<OutboundEvent> {
        let Some(session) = self.launch_wizard.as_mut() else {
            return Vec::new();
        };
        if session.wizard_id != wizard_id {
            return Vec::new();
        }

        match result {
            Ok(hydration) => session.wizard.apply_hydration(hydration),
            Err(error) => session.wizard.set_hydration_error(error),
        }

        vec![self.launch_wizard_state_outbound()]
    }

    pub(super) fn handle_issue_launch_wizard_prepared(
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
                BackendEvent::KnowledgeError {
                    id,
                    knowledge_kind,
                    message: "Project tab not found".to_string(),
                },
            )];
        }
        let source_window_still_open = self
            .window_lookup
            .get(&id)
            .and_then(|address| {
                self.tab(&address.tab_id)
                    .and_then(|tab| tab.workspace.window(&address.raw_id))
            })
            .is_some_and(|window| knowledge_kind_for_preset(window.preset) == Some(knowledge_kind));
        if !source_window_still_open {
            return vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::KnowledgeError {
                    id,
                    knowledge_kind,
                    message: "Issue/Knowledge window closed".to_string(),
                },
            )];
        }

        match result {
            Ok(branch_name) => match self.open_launch_wizard_for_branch(
                &tab_id,
                &project_root,
                &branch_name,
                Some(issue_number),
            ) {
                Ok(()) => vec![self.launch_wizard_state_outbound()],
                Err(error) => vec![OutboundEvent::reply(
                    &client_id,
                    BackendEvent::KnowledgeError {
                        id,
                        knowledge_kind,
                        message: error,
                    },
                )],
            },
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::KnowledgeError {
                    id,
                    knowledge_kind,
                    message: error,
                },
            )],
        }
    }

    pub(super) fn handle_launch_wizard_action(
        &mut self,
        action: gwt::LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(mut session) = self.launch_wizard.take() else {
            return Vec::new();
        };
        session.wizard.apply(action);

        match session.wizard.completion.take() {
            Some(LaunchWizardCompletion::Cancelled) => {
                vec![self.launch_wizard_state_broadcast(None)]
            }
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    session.wizard.error =
                        Some("The selected session window is no longer available".to_string());
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                let Some(tab) = self.tab_mut(&address.tab_id) else {
                    session.wizard.error =
                        Some("The selected session tab is no longer available".to_string());
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                if !tab.workspace.focus_window(&address.raw_id, None) {
                    session.wizard.error =
                        Some("The selected session window is no longer available".to_string());
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
            Some(LaunchWizardCompletion::Launch(config)) => match *config {
                LaunchWizardLaunchRequest::Agent(config) => {
                    match self.spawn_agent_window(&session.tab_id, *config, bounds) {
                        Ok(mut events) => {
                            events.push(self.launch_wizard_state_broadcast(None));
                            events
                        }
                        Err(error) => {
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
                            session.wizard.error = Some(error);
                            self.launch_wizard = Some(session);
                            vec![self.launch_wizard_state_outbound()]
                        }
                    }
                }
            },
            None => {
                self.launch_wizard = Some(session);
                vec![self.launch_wizard_state_outbound()]
            }
        }
    }

    pub(super) fn launch_wizard_state_outbound(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: self
                .launch_wizard
                .as_ref()
                .map(|wizard| Box::new(wizard.wizard.view())),
        })
    }

    pub(super) fn launch_wizard_state_broadcast(
        &self,
        wizard: Option<gwt::LaunchWizardView>,
    ) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: wizard.map(Box::new),
        })
    }

    pub(super) fn clear_launch_wizard(&mut self) -> Option<LaunchWizardSession> {
        self.launch_wizard.take()
    }
}

fn resolve_launch_wizard_hydration(
    project_root: &Path,
    branch_name: &str,
    active_session_branches: &std::collections::HashSet<String>,
    sessions_dir: &Path,
) -> Result<LaunchWizardHydration, String> {
    let agent_options = load_agent_options(&gwt_agent::VersionCache::load(
        &default_wizard_version_cache_path(),
    ));
    let entries = list_branch_entries_with_active_sessions(project_root, active_session_branches)
        .map_err(|error| error.to_string())?;
    let selected_branch = entries
        .into_iter()
        .find(|entry| entry.name == branch_name)
        .ok_or_else(|| format!("Branch not found: {branch_name}"))?;
    let normalized_branch_name = normalize_branch_name(&selected_branch.name);
    let worktree_path = branch_worktree_path(project_root, &normalized_branch_name);
    let quick_start_root = worktree_path
        .clone()
        .unwrap_or_else(|| project_root.to_path_buf());
    let quick_start_entries = gwt::launch_wizard::load_quick_start_entries(
        &quick_start_root,
        sessions_dir,
        &normalized_branch_name,
    );
    let (docker_context, docker_service_status) =
        detect_wizard_docker_context_and_status(&quick_start_root);

    Ok(LaunchWizardHydration {
        selected_branch: Some(selected_branch),
        normalized_branch_name,
        worktree_path,
        quick_start_root,
        docker_context,
        docker_service_status,
        agent_options,
        quick_start_entries,
    })
}
