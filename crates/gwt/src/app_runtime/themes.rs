//! Issue #7643 (SPEC #7007 T-025): the Themes screen of a project.
//!
//! The screen asks for the themes of its own project; the host resolves the
//! repository from the project, never from the client. GitHub is read and
//! written off the event loop, and the answer goes to the asking client only.

use super::*;

impl AppRuntime {
    /// Read the themes (`delete: None`) or delete one and answer with the rest.
    pub(super) fn theme_events(
        &self,
        client_id: ClientId,
        context: &ProjectContext,
        request_id: u64,
        delete: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let project_root = context.project_root.clone();
        let client_factory = Arc::clone(&self.issue_client_factory);
        let proxy = self.proxy.for_project(context.clone());
        let reply_client = client_id.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let event = match theme_request(&project_root, &client_factory, delete) {
                Ok(view) => BackendEvent::Themes {
                    request_id,
                    view: Box::new(view),
                },
                Err(refusal) => BackendEvent::ThemeError {
                    request_id,
                    message: refusal.0,
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &reply_client,
                event,
            )]));
        });
        match spawn {
            Ok(()) => Vec::new(),
            // The request never started: say so, so the screen stops waiting.
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::ThemeError {
                    request_id,
                    message: format!("The request could not start: {error}. Try again."),
                },
            )],
        }
    }
}

fn theme_request(
    project_root: &Path,
    client_factory: &RuntimeIssueClientFactory,
    delete: Option<u64>,
) -> Result<gwt::themes::ThemesView, gwt::themes::ThemeRefusal> {
    let (owner, repo) = gwt::issue_monitor_worker::github_remote_owner_and_repo(project_root)
        .map_err(|error| {
            gwt::themes::ThemeRefusal(format!(
                "Themes are GitHub Issues, and this project has no GitHub repository to read them from: {error}"
            ))
        })?;
    let client = client_factory(&owner, &repo)?;
    let cache = gwt_github::Cache::new(
        gwt::issue_cache::issue_cache_root_for_repo_path(project_root)
            .unwrap_or_else(|| gwt::issue_cache::issue_cache_root_for_repo_slug(&owner, &repo)),
    );
    let now = gwt_core::timestamp::UtcTimestamp::now();
    match delete {
        Some(number) => gwt::themes::delete_theme(client.as_ref(), &cache, number, now),
        None => gwt::themes::load_themes_view(client.as_ref(), &cache, now),
    }
}
