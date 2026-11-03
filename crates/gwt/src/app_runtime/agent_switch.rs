//! Switch requests and read projections; the project owner controls its PTYs.
use super::*;
use gwt::issue_monitor::{AgentSwitchAction, AgentSwitchStage};

impl AppRuntime {
    pub(crate) fn agent_switch_events(
        &mut self,
        context: &ProjectContext,
        client_id: &ClientId,
        action: AgentSwitchAction,
    ) -> Vec<OutboundEvent> {
        self.queue_owner_switch(context, Some(client_id.clone()), Some(action))
    }

    pub(crate) fn agent_switch_tick_events(&mut self) -> Vec<OutboundEvent> {
        let mut seen = HashSet::new();
        let mut events = Vec::new();
        for context in self
            .project_contexts()
            .into_iter()
            .filter(|context| seen.insert(context.project_root.clone()))
        {
            let pending = gwt::load_issue_monitor_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(&context.project_root),
            )
            .is_ok_and(|prefs| {
                prefs.agent_switch_intents.values().any(|intent| {
                    matches!(
                        intent.stage,
                        AgentSwitchStage::Requested
                            | AgentSwitchStage::Checkpointing
                            | AgentSwitchStage::CheckpointVerified
                            | AgentSwitchStage::StoppingCurrent
                            | AgentSwitchStage::LaunchingNext
                    )
                })
            });
            if pending {
                events.extend(self.queue_owner_switch(&context, None, None));
            }
        }
        events
    }

    fn queue_owner_switch(
        &self,
        context: &ProjectContext,
        client_id: Option<ClientId>,
        action: Option<AgentSwitchAction>,
    ) -> Vec<OutboundEvent> {
        let requested = action.is_some();
        let operation_id = action.as_ref().and_then(|action| match action {
            AgentSwitchAction::Read { .. } => None,
            AgentSwitchAction::Request { operation_id, .. }
            | AgentSwitchAction::Cancel { operation_id } => Some(operation_id.clone()),
        });
        let context = context.clone();
        let supervisor = Arc::clone(&self.daemon_supervisor);
        let proxy = self.proxy.clone().for_project(context.clone());
        let failed_client = client_id.clone();
        let failed_project = context.project_key.clone();
        if let Err(message) = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string()).and_then(|executor| executor.block_on(async {
                let client = gwt::project_runtime::client::ProjectRuntimeClient::connect_for_gui(context.project_root.clone(), supervisor).await?;
                let command = match action {
                    Some(action) => gwt::project_runtime::events::OwnerCommand::AgentSwitch { action: Box::new(action) },
                    None => gwt::project_runtime::events::OwnerCommand::DriveAgentSwitch,
                };
                let value = client.command(command).await?;
                if value.get("busy").and_then(serde_json::Value::as_bool) == Some(true) { return Ok(None); }
                serde_json::from_value(value["snapshot"].clone()).map(Some).map_err(|error| error.to_string())
            }));
            let event = match result {
                Ok(Some(snapshot)) => Some(BackendEvent::AgentSwitch { snapshot }),
                Ok(None) => None,
                Err(message) if client_id.is_some() => Some(BackendEvent::AgentSwitchError { project_key: context.project_key.to_string(), message, operation_id }),
                Err(error) => { tracing::warn!(%error, "project owner Switch observation failed; keeping current work"); None }
            };
            if let Some(event) = event {
                let outbound = match client_id { Some(client_id) => OutboundEvent::reply(client_id, event), None => OutboundEvent::project(context.project_key, event) };
                proxy.send(UserEvent::Dispatch(vec![outbound]));
            }
            if requested { proxy.send(UserEvent::IssueMonitorScheduledTick); }
        }) {
            if let Some(client_id) = failed_client {
                return vec![OutboundEvent::reply(client_id, BackendEvent::AgentSwitchError { project_key: failed_project.to_string(), message, operation_id: None })];
            }
        }
        Vec::new()
    }
}
