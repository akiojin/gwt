//! Host-wide admission and background work for supported-agent maintenance.

use std::path::Path;

use gwt::agent_maintenance::{
    latest_version, run_maintenance, update_available, version_is_current, MaintenanceAction,
    MaintenanceResult,
};

use super::{
    AppRuntime, BackendEvent, ClientId, LaunchWizardMemoryCache, OutboundEvent, UserEvent,
};

pub(super) const AGENT_MAINTENANCE_BUSY: &str =
    "Agent maintenance is in progress. Try launching the agent again when it finishes.";

#[derive(Default)]
pub(crate) struct AgentMaintenanceState {
    pub(crate) busy: bool,
    pub(crate) app_update_committing: bool,
    pub(super) catalog_generation: u64,
    saved_auto_update: Option<bool>,
    pub(super) deferred_startup_bounds: Option<gwt::WindowGeometry>,
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
    for agent in &mut agents {
        match latest_version(&agent.id, config_path, cwd) {
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
        }
    }
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

    pub(super) fn agent_maintenance_rejection(&self) -> Option<&'static str> {
        if self.agent_maintenance.busy {
            return Some(AGENT_MAINTENANCE_BUSY);
        }
        if self.agent_maintenance.app_update_committing {
            return Some("gwt is applying an update. Start agent maintenance after it restarts.");
        }
        let live_agent = self.tabs.iter().any(|tab| {
            tab.workspace.persisted().windows.iter().any(|window| {
                crate::runtime_support::window_is_agent_pane(window)
                    && (window.status == gwt::WindowProcessStatus::Starting
                        || self
                            .runtimes
                            .contains_key(&crate::combined_window_id(&tab.id, &window.id)))
            })
        }) || self
            .active_agent_sessions
            .keys()
            .any(|id| self.runtimes.contains_key(id));
        let pending_launch = !self.inflight_launches.is_empty()
            || !self.pending_launch_completions.is_empty()
            || !self.issue_monitor_launch_preparations.is_empty()
            || !self.pending_continue_work.is_empty()
            || !self.pending_fresh_execution_launches.is_empty()
            || !self.pending_startup_auto_resume_sessions.is_empty()
            || self.project_states.values().any(|state| {
                !state.pending_pm_launches.is_empty()
                    || !state.pending_pm_worktree_preparations.is_empty()
                    || !state.pending_launch_wizard_materializations.is_empty()
            });
        (live_agent || pending_launch).then_some(
            "Close all live agent panes and wait for pending agent launches before maintenance.",
        )
    }

    pub(super) fn maintain_supported_agent_events(
        &mut self,
        client_id: ClientId,
        agent_id: String,
        action: MaintenanceAction,
    ) -> Vec<OutboundEvent> {
        if let Some(message) = self.agent_maintenance_rejection() {
            return vec![OutboundEvent::reply(
                client_id,
                result_event(failed_result(agent_id, action, message.into())),
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
        if let Err(message) = self.spawn_agent_maintenance(Some((agent_id.clone(), action))) {
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
    pub(super) fn start_agent_auto_update(&mut self) {
        let enabled = self
            .profile_config_path()
            .ok()
            .and_then(|path| gwt_config::Settings::load_from_path(&path).ok())
            .is_some_and(|settings| settings.agent.auto_update);
        if !enabled || self.agent_maintenance_rejection().is_some() {
            return;
        }
        if let Err(error) = self.spawn_agent_maintenance(None) {
            tracing::warn!(%error, "automatic agent maintenance could not start");
        }
    }

    fn spawn_agent_maintenance(
        &mut self,
        manual: Option<(String, MaintenanceAction)>,
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
                                    && gwt_agent::builtin_agent_descriptor_for_command(&agent.id)
                                        .is_some_and(|descriptor| {
                                            descriptor.distribution.npm_package().is_some()
                                        })
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
        if let Some(bounds) = self.agent_maintenance.deferred_startup_bounds.take() {
            events.extend(self.startup_auto_resume_ready_events(bounds));
        }
        events
    }
}
