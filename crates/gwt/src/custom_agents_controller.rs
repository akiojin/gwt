use gwt::{BackendEvent, FrontendEvent};

use super::{AppEventProxy, BlockingTaskSpawner, ClientId, OutboundEvent, UserEvent};

pub(crate) struct CustomAgentsController {
    proxy: AppEventProxy,
    blocking_tasks: BlockingTaskSpawner,
}

impl CustomAgentsController {
    pub(crate) fn new(proxy: AppEventProxy, blocking_tasks: BlockingTaskSpawner) -> Self {
        Self {
            proxy,
            blocking_tasks,
        }
    }

    pub(crate) fn handle_event(
        &self,
        client_id: ClientId,
        event: FrontendEvent,
    ) -> Vec<OutboundEvent> {
        match event {
            FrontendEvent::ListCustomAgents => {
                vec![self.reply(client_id, gwt::custom_agents_dispatch::list_event())]
            }
            FrontendEvent::ListCustomAgentPresets => {
                vec![self.reply(client_id, gwt::custom_agents_dispatch::list_presets_event())]
            }
            FrontendEvent::AddCustomAgentFromPreset { input } => vec![self.reply(
                client_id,
                gwt::custom_agents_dispatch::add_from_preset_event(input),
            )],
            FrontendEvent::UpdateCustomAgent { agent } => {
                vec![self.reply(client_id, gwt::custom_agents_dispatch::update_event(*agent))]
            }
            FrontendEvent::DeleteCustomAgent { agent_id } => vec![self.reply(
                client_id,
                gwt::custom_agents_dispatch::delete_event(agent_id),
            )],
            FrontendEvent::TestBackendConnection { base_url, api_key } => {
                self.spawn_backend_connection_probe(client_id, base_url, api_key);
                Vec::new()
            }
            other => panic!("unsupported custom agents event: {other:?}"),
        }
    }

    fn reply(&self, client_id: ClientId, event: BackendEvent) -> OutboundEvent {
        OutboundEvent::reply(client_id, event)
    }

    fn spawn_backend_connection_probe(
        &self,
        client_id: ClientId,
        base_url: String,
        api_key: String,
    ) {
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = gwt::custom_agents_dispatch::test_connection_event(&base_url, &api_key);
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }
}
