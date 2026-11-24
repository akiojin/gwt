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
        let installed = self.installed_tools();
        let has_projects = !self.tabs.is_empty() || !self.recent_projects.is_empty();
        if save.is_some() {
            let event = Self::first_run_setup_event(&installed, has_projects, save);
            return vec![OutboundEvent::reply(client_id, event)];
        }
        let proxy = self.proxy.clone();
        let reply_to = client_id.clone();
        match self.blocking_tasks.try_spawn(move || {
            let event = Self::first_run_setup_event(&installed, has_projects, None);
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                reply_to, event,
            )]));
        }) {
            Ok(_) => Vec::new(),
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::FirstRunSetupError {
                    message: error.to_string(),
                },
            )],
        }
    }

    fn first_run_setup_event(
        installed: &[String],
        has_projects: bool,
        save: Option<(Option<String>, Option<String>)>,
    ) -> BackendEvent {
        match Self::first_run_setup_view(installed, has_projects, save) {
            Ok(view) => BackendEvent::FirstRunSetup {
                view: Box::new(view),
            },
            Err(message) => BackendEvent::FirstRunSetupError { message },
        }
    }

    fn first_run_setup_view(
        installed: &[String],
        has_projects: bool,
        save: Option<(Option<String>, Option<String>)>,
    ) -> Result<setup::FirstRunSetupView, String> {
        let store = if save.is_some() {
            gwt::execution_environments::load()
        } else {
            gwt::execution_environments::adopt_signed_in_accounts()
        }
        .map_err(|error| format!("Could not read the registered tools: {error}"))?;
        let mut tools: Vec<SetupToolView> = store
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
        // An installed CLI can finish setup and open its native sign-in on
        // launch. It does not acquire an account row until it signs in.
        for agent in installed
            .iter()
            .filter_map(|id| gwt_agent::resolve_agent_id(id))
        {
            let tool = agent.command();
            if gwt::execution_environments::carries_own_account(tool)
                && !tools.iter().any(|entry| entry.tool == tool)
            {
                tools.push(SetupToolView {
                    name: agent.display_name().to_owned(),
                    tool: tool.to_owned(),
                    installed: true,
                });
            }
        }
        let members: Vec<SetupMemberView> = gwt::agent_registry::SessionMemberSnapshot::load()
            .map_err(|error| format!("Could not read the agent ledger: {error}"))?
            .registered_members()
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
        Ok(setup::view(tools, members, pm, has_projects))
    }
}
