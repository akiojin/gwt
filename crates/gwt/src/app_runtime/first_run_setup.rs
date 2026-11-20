//! Issue #7731 (SPEC #7007 slice N-4): the first-run setup, served on the Hub
//! connection. The rules live in `gwt::first_run_setup`; this reads the
//! machine's tools and ledger for them.

use super::*;
use gwt::first_run_setup::{self as setup, PmChoice, SetupMemberView, SetupToolView};

impl AppRuntime {
    /// Answer with the setup view, after saving `save` when the screen sent a
    /// designation: `(member id, new name)`.
    pub(super) fn first_run_setup_events(
        &self,
        client_id: ClientId,
        save: Option<(Option<String>, Option<String>)>,
    ) -> Vec<OutboundEvent> {
        let event = match self.first_run_setup_view(save) {
            Ok(view) => BackendEvent::FirstRunSetup {
                view: Box::new(view),
            },
            Err(message) => BackendEvent::FirstRunSetupError { message },
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    fn first_run_setup_view(
        &self,
        save: Option<(Option<String>, Option<String>)>,
    ) -> Result<setup::FirstRunSetupView, String> {
        let installed = self.installed_tools();
        let store = self
            .ensure_execution_environments(&self.known_projects(), &installed)
            .map_err(|error| format!("Could not read the registered tools: {error}"))?;
        let tools: Vec<SetupToolView> = store
            .environments
            .iter()
            .map(|environment| SetupToolView {
                name: environment.name.clone(),
                tool: environment.via_tool.clone(),
                installed: gwt_agent::resolve_agent_id(&environment.via_tool).is_some_and(
                    |agent| {
                        installed
                            .iter()
                            .any(|id| gwt_agent::resolve_agent_id(id).as_ref() == Some(&agent))
                    },
                ),
            })
            .collect();
        let members: Vec<SetupMemberView> = gwt::agent_registry::SessionMemberSnapshot::load()
            .map_err(|error| format!("Could not read the agent ledger: {error}"))?
            .enabled_members()
            .into_iter()
            .map(|(member_id, name)| SetupMemberView { member_id, name })
            .collect();
        let pm = match save {
            Some((member_id, name)) => {
                let choice = match (member_id, name) {
                    (Some(member_id), None) => PmChoice::Member(member_id),
                    (None, Some(name)) => PmChoice::New(name),
                    _ => return Err("Choose one Project Manager.".into()),
                };
                let pm = setup::designate(&tools, &members, choice)?;
                setup::save_designation(&pm)
                    .map_err(|error| format!("Could not save the Project Manager: {error}"))?;
                Some(pm)
            }
            None => setup::load_designation()
                .map_err(|error| format!("Could not read the first-run setup: {error}"))?,
        };
        let has_projects = !self.tabs.is_empty() || !self.recent_projects.is_empty();
        Ok(setup::view(tools, members, pm, has_projects))
    }
}
