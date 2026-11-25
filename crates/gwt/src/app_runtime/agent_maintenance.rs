//! Per-tool admission and background work for supported-agent maintenance.

use std::{collections::HashSet, path::Path};

use gwt::agent_maintenance::{
    latest_version, run_maintenance, supports_updates, update_available, version_is_current,
    MaintenanceAction, MaintenanceResult,
};

use super::{
    AppRuntime, BackendEvent, ClientId, LaunchWizardMemoryCache, OutboundEvent, UserEvent,
};

const AGENT_MAINTENANCE_BUSY: &str =
    "Agent maintenance is in progress. Try launching the agent again when it finishes.";

#[derive(Default)]
pub(crate) struct AgentMaintenanceState {
    pub(crate) busy: bool,
    pub(crate) app_update_committing: bool,
    pub(super) catalog_generation: u64,
    saved_auto_update: Option<bool>,
    pub(super) deferred_startup_ready: bool,
    /// Tools the running worker may replace. `None` while `busy` holds every
    /// agent launch.
    scope: Option<MaintenanceScope>,
}

enum MaintenanceScope {
    Only(String),
    /// Startup auto-update: every tool but the ones skipped as in use.
    AllExcept(Vec<String>),
}

/// Catalog command of the tool an agent identifier runs. Custom and
/// unidentified agents yield `None`, which callers treat as "any tool".
fn catalog_tool(agent: Option<&str>) -> Option<&'static str> {
    match gwt_agent::resolve_agent_id(agent?)? {
        gwt_agent::AgentId::Custom(_) => None,
        id => gwt_agent::builtin_agent_descriptor_for_command(id.command())
            .map(|descriptor| descriptor.command),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentMaintenanceCompletion {
    pub(super) results: Vec<MaintenanceResult>,
    pub(super) cache: Option<LaunchWizardMemoryCache>,
    pub(super) catalog: Option<BackendEvent>,
}

pub(super) fn supported_agent_catalog(
    cache: &LaunchWizardMemoryCache,
    config_path: &Path,
    cwd: &Path,
) -> BackendEvent {
    let mut agents = cache.supported_agents();
    // Only the fixed catalog's metadata probes run concurrently. Each worker
    // owns one row, retaining catalog order and the existing per-probe deadline.
    std::thread::scope(|scope| {
        for agent in &mut agents {
            scope.spawn(move || match latest_version(&agent.id, config_path, cwd) {
                Ok(Some(version)) => {
                    agent.update_available = agent.installed
                        && agent
                            .installed_version
                            .as_deref()
                            .is_some_and(|installed| update_available(installed, &version));
                    agent.up_to_date = agent.installed
                        && agent
                            .installed_version
                            .as_deref()
                            .is_some_and(|installed| version_is_current(installed, &version));
                    agent.available_version = Some(version);
                }
                Ok(None) => {
                    agent.update_check_error =
                        Some("Updater metadata unavailable for this agent.".into());
                }
                Err(error) => agent.update_check_error = Some(error),
            });
        }
    });
    let auto_update = gwt_config::Settings::load_from_path(config_path)
        .map(|settings| settings.agent.auto_update)
        .unwrap_or(false);
    BackendEvent::SupportedAgentList {
        agents,
        auto_update,
        maintenance_pending: false,
    }
}

fn result_event(result: MaintenanceResult) -> BackendEvent {
    BackendEvent::SupportedAgentMaintenance {
        agent_id: result.agent_id,
        pending: false,
        success: Some(result.success),
        message: result.message,
        before_version: result.before_version,
        after_version: result.after_version,
    }
}

fn failed_result(
    agent_id: String,
    action: MaintenanceAction,
    message: String,
) -> MaintenanceResult {
    MaintenanceResult {
        agent_id,
        action,
        before_version: None,
        after_version: None,
        success: false,
        message,
    }
}

impl AppRuntime {
    pub(crate) fn handle_supported_agent_catalog_ready(
        &mut self,
        client_id: ClientId,
        generation: u64,
        mut event: BackendEvent,
    ) -> Vec<OutboundEvent> {
        if generation != self.agent_maintenance.catalog_generation {
            return Vec::new();
        }
        if let BackendEvent::SupportedAgentList {
            auto_update,
            maintenance_pending,
            ..
        } = &mut event
        {
            if let Some(enabled) = self.agent_maintenance.saved_auto_update {
                *auto_update = enabled;
            }
            *maintenance_pending = self.agent_maintenance.busy;
        }
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// Why `tool` cannot be installed or updated now. Only agents running
    /// that tool, and launches already bound to it, hold its admission.
    pub(super) fn agent_maintenance_rejection(&self, tool: &str) -> Option<String> {
        if self.agent_maintenance.busy {
            return Some(AGENT_MAINTENANCE_BUSY.into());
        }
        if self.agent_maintenance.app_update_committing {
            return Some(
                "gwt is applying an update. Start agent maintenance after it restarts.".into(),
            );
        }
        let name = gwt_agent::builtin_agent_descriptor_for_command(tool)
            .map_or(tool, |descriptor| descriptor.display_name);
        let uses = |agent: Option<&str>| catalog_tool(agent).is_none_or(|used| used == tool);
        let mut live = HashSet::new();
        let mut users = Vec::new();
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                let id = crate::combined_window_id(&tab.id, &window.id);
                if !crate::runtime_support::window_is_agent_pane(window)
                    || !(window.status == gwt::WindowProcessStatus::Starting
                        || self.runtimes.contains_key(&id))
                {
                    continue;
                }
                live.insert(id);
                if uses(window.agent_id.as_deref().or(window.preset.command_name())) {
                    let agent = window
                        .agent_name
                        .as_deref()
                        .or(window.purpose_title.as_deref())
                        .unwrap_or(&window.title);
                    users.push(match window.linked_issue_number {
                        Some(issue) => format!("{agent} (Issue #{issue})"),
                        None => agent.to_string(),
                    });
                }
            }
        }
        let mut detached: Vec<_> = self
            .active_agent_sessions
            .iter()
            .filter(|(id, session)| {
                self.runtimes.contains_key(*id)
                    && !live.contains(*id)
                    && uses(Some(&session.agent_id))
            })
            .map(|(_, session)| session.display_name.clone())
            .collect();
        detached.sort();
        users.extend(detached);
        if !users.is_empty() {
            return Some(format!(
                "{name} is in use by {}. Close those agent panes before maintenance.",
                users.join(", ")
            ));
        }
        // Launches that have not chosen a tool yet are not counted here:
        // `agent_maintenance_launch_block` refuses them if they pick this one.
        let pending_launch = self
            .pending_continue_work
            .values()
            .any(|pending| uses(Some(pending.work_agent_id.command())))
            || self
                .pending_startup_auto_resume_sessions
                .iter()
                .any(|pending| uses(Some(pending.session.agent_id.command())))
            || self.project_states.values().any(|state| {
                state
                    .pending_launch_wizard_materializations
                    .values()
                    .any(|session| uses(Some(&session.wizard.agent_id)))
            });
        pending_launch.then(|| {
            format!("{name} has a pending agent launch. Wait for it to start before maintenance.")
        })
    }

    /// Refuses a launch that would run a tool the maintenance worker may be
    /// replacing. Other tools keep launching.
    pub(super) fn agent_maintenance_launch_block(
        &self,
        agent: Option<&str>,
    ) -> Option<&'static str> {
        if !self.agent_maintenance.busy {
            return None;
        }
        let untouched = match (&self.agent_maintenance.scope, catalog_tool(agent)) {
            (Some(MaintenanceScope::Only(target)), Some(tool)) => tool != target,
            (Some(MaintenanceScope::AllExcept(skipped)), Some(tool)) => {
                skipped.iter().any(|skipped| skipped == tool)
            }
            _ => false,
        };
        (!untouched).then_some(AGENT_MAINTENANCE_BUSY)
    }

    pub(super) fn maintain_supported_agent_events(
        &mut self,
        client_id: ClientId,
        agent_id: String,
        action: MaintenanceAction,
    ) -> Vec<OutboundEvent> {
        if let Some(message) = self.agent_maintenance_rejection(&agent_id) {
            return vec![OutboundEvent::reply(
                client_id,
                result_event(failed_result(agent_id, action, message)),
            )];
        }
        if gwt_agent::builtin_agent_descriptor_for_command(&agent_id).is_none() {
            return vec![OutboundEvent::reply(
                client_id,
                result_event(failed_result(
                    agent_id,
                    action,
                    "Unknown supported agent.".into(),
                )),
            )];
        }
        if let Err(message) =
            self.spawn_agent_maintenance(Some((agent_id.clone(), action)), Vec::new())
        {
            return vec![OutboundEvent::reply(
                client_id,
                result_event(failed_result(agent_id, action, message)),
            )];
        }
        vec![OutboundEvent::broadcast(
            BackendEvent::SupportedAgentMaintenance {
                agent_id,
                pending: true,
                success: None,
                message: "Agent maintenance is in progress.".into(),
                before_version: None,
                after_version: None,
            },
        )]
    }

    pub(super) fn set_agent_auto_update_events(
        &mut self,
        client_id: ClientId,
        enabled: bool,
    ) -> Vec<OutboundEvent> {
        let result = self.profile_config_path().and_then(|path| {
            if self.profile_config_path.is_none() {
                return gwt_config::Settings::update_global(|settings| {
                    settings.agent.auto_update = enabled;
                    Ok(())
                })
                .map_err(|error| error.to_string());
            }
            let mut settings = if path.exists() {
                gwt_config::Settings::load_from_path(&path).map_err(|error| error.to_string())?
            } else {
                gwt_config::Settings::default()
            };
            settings.agent.auto_update = enabled;
            settings.save(&path).map_err(|error| error.to_string())
        });
        if let Err(message) = result {
            return vec![OutboundEvent::reply(
                client_id,
                result_event(failed_result(
                    String::new(),
                    MaintenanceAction::Update,
                    format!("Cannot save automatic agent updates: {message}. Check configuration permissions and retry."),
                )),
            )];
        }
        self.agent_maintenance.saved_auto_update = Some(enabled);
        self.spawn_supported_agent_list(client_id);
        Vec::new()
    }

    /// Called once before startup can create agent panes. This never installs
    /// missing agents, and changing the setting does not invoke this method.
    /// Tools already in use are skipped; the rest still update.
    pub(super) fn start_agent_auto_update(&mut self) {
        let enabled = self
            .profile_config_path()
            .ok()
            .and_then(|path| gwt_config::Settings::load_from_path(&path).ok())
            .is_some_and(|settings| settings.agent.auto_update);
        if !enabled {
            return;
        }
        let updatable: Vec<_> = gwt_agent::builtin_agent_descriptors()
            .iter()
            .filter(|descriptor| supports_updates(descriptor))
            .map(|descriptor| descriptor.command)
            .collect();
        let skipped: Vec<_> = updatable
            .iter()
            .filter(|tool| self.agent_maintenance_rejection(tool).is_some())
            .map(|tool| tool.to_string())
            .collect();
        if skipped.len() == updatable.len() {
            return;
        }
        if let Err(error) = self.spawn_agent_maintenance(None, skipped) {
            tracing::warn!(%error, "automatic agent maintenance could not start");
        }
    }

    fn spawn_agent_maintenance(
        &mut self,
        manual: Option<(String, MaintenanceAction)>,
        skipped: Vec<String>,
    ) -> Result<(), String> {
        let config_path = self.profile_config_path()?;
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        let mut cache = self.launch_wizard_cache.clone();
        let proxy = self.proxy.clone();
        self.agent_maintenance.catalog_generation = self
            .agent_maintenance
            .catalog_generation
            .checked_add(1)
            .expect("supported agent catalog generation exhausted");
        self.agent_maintenance.busy = true;
        self.agent_maintenance.scope = Some(match &manual {
            Some((tool, _)) => MaintenanceScope::Only(tool.clone()),
            None => MaintenanceScope::AllExcept(skipped.clone()),
        });
        let spawn = self.blocking_tasks.try_spawn(move || {
            let fallback = manual
                .clone()
                .unwrap_or_else(|| ("auto-update".into(), MaintenanceAction::Update));
            let completion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let targets = match manual {
                    Some(target) => vec![target],
                    None => {
                        cache.refresh_agent_options_for_profile(&config_path, &cwd);
                        cache
                            .supported_agents()
                            .into_iter()
                            .filter(|agent| {
                                agent.installed
                                    && !skipped.contains(&agent.id)
                                    && gwt_agent::builtin_agent_descriptor_for_command(&agent.id)
                                        .is_some_and(supports_updates)
                            })
                            .map(|agent| (agent.id, MaintenanceAction::Update))
                            .collect()
                    }
                };
                let results = targets
                    .into_iter()
                    .map(|(id, action)| run_maintenance(&id, action, &config_path, &cwd))
                    .collect();
                cache.refresh_agent_options_for_profile(&config_path, &cwd);
                let catalog = supported_agent_catalog(&cache, &config_path, &cwd);
                AgentMaintenanceCompletion {
                    results,
                    cache: Some(cache),
                    catalog: Some(catalog),
                }
            }))
            .unwrap_or_else(|_| AgentMaintenanceCompletion {
                results: vec![failed_result(
                    fallback.0,
                    fallback.1,
                    "Agent maintenance worker failed. Try again.".into(),
                )],
                cache: None,
                catalog: None,
            });
            proxy.send(UserEvent::SupportedAgentMaintenanceComplete(Box::new(
                completion,
            )));
        });
        if let Err(error) = spawn {
            self.agent_maintenance.busy = false;
            self.agent_maintenance.scope = None;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn handle_supported_agent_maintenance_complete(
        &mut self,
        completion: AgentMaintenanceCompletion,
    ) -> Vec<OutboundEvent> {
        self.agent_maintenance.catalog_generation = self
            .agent_maintenance
            .catalog_generation
            .checked_add(1)
            .expect("supported agent catalog generation exhausted");
        self.agent_maintenance.busy = false;
        self.agent_maintenance.scope = None;
        let mut events: Vec<_> = completion
            .results
            .into_iter()
            .map(|result| OutboundEvent::broadcast(result_event(result)))
            .collect();
        if let Some(cache) = completion.cache {
            self.launch_wizard_cache = cache;
            let contexts: Vec<_> = self
                .project_states
                .values()
                .filter(|state| state.launch_wizard.is_some())
                .map(|state| state.context.clone())
                .collect();
            for context in contexts {
                self.refresh_open_launch_wizard_from_cache(&context);
                events.push(self.launch_wizard_state_outbound(&context));
            }
        }
        if let Some(mut catalog) = completion.catalog {
            if let BackendEvent::SupportedAgentList {
                auto_update,
                maintenance_pending,
                ..
            } = &mut catalog
            {
                if let Some(enabled) = self.agent_maintenance.saved_auto_update {
                    *auto_update = enabled;
                }
                *maintenance_pending = false;
            }
            events.push(OutboundEvent::broadcast(catalog));
        }
        if std::mem::take(&mut self.agent_maintenance.deferred_startup_ready) {
            events.extend(self.startup_auto_resume_ready_events());
        }
        events
    }
}
