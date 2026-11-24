//! Execution environment screens (Issue #7741, SPEC #7007 items 77-82).
//!
//! The definitions are machine-wide; each project's priority list is saved
//! with that project's Issues settings. This module builds the view the
//! screens show and applies their requests.

use std::{
    io::Read,
    sync::{mpsc, Arc},
};

use super::*;
use base64::Engine;
use gwt::account_homes;
use gwt::execution_environments::{self as environments, Store};
use gwt::protocol::{
    ExecutionAccountView, ExecutionEnvironmentProjectView, ExecutionEnvironmentViaOptionView,
    ExecutionEnvironmentView, ExecutionEnvironmentsView, PmExecutionEnvironmentsView,
    PoolAccountDeletionImpact, PoolAccountDirection, PoolAccountProviderView,
    PoolAccountSessionView, PoolExecutionAccountView, ProjectExecutionEnvironmentView,
    ProjectExecutionEnvironmentsView,
};
use gwt::ProjectExecutionEnvironment;
use gwt_terminal::{pty::SpawnConfig, PtyHandle};

const TOOLS_THAT_READ_THEIR_OWN_SETTINGS: [&str; 3] = ["hermes", "openclaw", "opencode"];
const POOL_PROVIDER_CHOICES: [&str; 6] = ["claude", "codex", "grok", "gh", "agy", "qwen"];

pub(crate) struct PoolAccountLoginTerminal {
    id: String,
    active: Option<ActivePoolAccountLogin>,
}

struct ActivePoolAccountLogin {
    account_id: String,
    pty: Arc<PtyHandle>,
    input: mpsc::SyncSender<String>,
}

fn tool_name(tool: &str) -> String {
    gwt_agent::resolve_agent_id(tool).map_or_else(
        || tool.to_string(),
        |agent| agent.display_name().to_string(),
    )
}

fn project_root(context: Option<&ProjectContext>) -> Option<String> {
    context.map(|context| context.project_root.display().to_string())
}

/// One project gwt knows on this machine and its Issues settings.
pub(crate) struct KnownProject {
    pub(crate) root: PathBuf,
    pub(crate) title: String,
    pub(crate) prefs: gwt::IssueMonitorPrefs,
}

/// Build the view from the definitions, the projects that use them, and the
/// tools installed on this machine. `current` is the project whose settings
/// are open, if any.
pub(crate) fn execution_environments_view(
    store: &Store,
    projects: &[KnownProject],
    current: Option<&Path>,
    installed: &dyn Fn(&str) -> bool,
    tool_settings: &dyn Fn(&str) -> environments::ToolSettings,
) -> ExecutionEnvironmentsView {
    let lists: Vec<_> = projects
        .iter()
        .map(|project| (project, environments::project_list(store, &project.prefs)))
        .collect();
    let environments = store
        .environments
        .iter()
        .map(|environment| {
            let read_only = store.runtime_edit_refusal(&environment.id);
            ExecutionEnvironmentView {
                id: environment.id.clone(),
                name: environment.name.clone(),
                kind: store.kind(environment),
                account_id: environment.account_id.clone(),
                account_name: store
                    .account(&environment.account_id)
                    .map_or_else(String::new, |account| account.name.clone()),
                via_tool: environment.via_tool.clone(),
                via_tool_name: tool_name(&environment.via_tool),
                runtime: environment.runtime.clone(),
                tool_installed: installed(&environment.via_tool),
                delete_blocked_reason: store
                    .delete_refusal(&environment.id)
                    .map(|refusal| refusal.0),
                tool_settings: read_only
                    .is_some()
                    .then(|| tool_settings(&environment.via_tool)),
                runtime_read_only_reason: read_only.map(|refusal| refusal.0),
                projects: lists
                    .iter()
                    .filter_map(|(project, (_, list))| {
                        let position = list
                            .iter()
                            .position(|entry| entry.environment_id == environment.id)?;
                        Some(ExecutionEnvironmentProjectView {
                            project_root: project.root.display().to_string(),
                            title: project.title.clone(),
                            priority: position + 1,
                        })
                    })
                    .collect(),
                delete_impact: None,
            }
        })
        .collect();
    let via_options = store
        .accounts
        .iter()
        .flat_map(|account| {
            TOOLS_THAT_READ_THEIR_OWN_SETTINGS.iter().map(|tool| {
                let name = tool_name(tool);
                ExecutionEnvironmentViaOptionView {
                    account_id: account.id.clone(),
                    via_tool: tool.to_string(),
                    blocked_reason: store
                        .via_refusal(&account.id, tool)
                        .map(|refusal| refusal.0)
                        .or_else(|| {
                            (!installed(tool)).then(|| {
                                format!("{name} is not installed. Install it in Tool management.")
                            })
                        }),
                    via_tool_name: name,
                }
            })
        })
        .collect();
    let project = lists
        .iter()
        .find(|(project, _)| Some(project.root.as_path()) == current)
        .map(
            |(project, (saved, list))| ProjectExecutionEnvironmentsView {
                project_root: project.root.display().to_string(),
                saved: *saved,
                entries: list
                    .iter()
                    .map(|entry| ProjectExecutionEnvironmentView {
                        environment_id: entry.environment_id.clone(),
                        tool: entry.tool.clone(),
                        tool_name: tool_name(&entry.tool),
                        name: store
                            .environment(&entry.environment_id)
                            .map(|environment| environment.name.clone()),
                    })
                    .collect(),
            },
        );
    let (pm_saved, pm_list) = store.pm_list();
    let pm = PmExecutionEnvironmentsView {
        saved: pm_saved,
        entries: pm_list
            .into_iter()
            .map(|environment| ProjectExecutionEnvironmentView {
                environment_id: environment.id.clone(),
                tool: environment.via_tool.clone(),
                tool_name: tool_name(&environment.via_tool),
                name: Some(environment.name.clone()),
            })
            .collect(),
    };
    ExecutionEnvironmentsView {
        pm,
        agent_priorities: store.agent_priorities.clone(),
        pool_providers: Vec::new(),
        active_sessions: Vec::new(),
        accounts: store
            .accounts
            .iter()
            .map(|account| ExecutionAccountView {
                id: account.id.clone(),
                name: account.name.clone(),
                tool: account.tool.clone(),
                tool_name: tool_name(&account.tool),
                signed_in: false,
                sign_in_identity: None,
                sign_in_blocked_reason: None,
                delete_blocked_reason: None,
                delete_impact: None,
            })
            .collect(),
        environments,
        via_options,
        project,
    }
}

fn deletion_impact(
    store: &Store,
    sessions: &[PoolAccountSessionView],
    account_id: Option<&str>,
    deleted_environments: &[String],
) -> PoolAccountDeletionImpact {
    let sessions = sessions
        .iter()
        .filter(|session| account_id == Some(session.execution_account.account_id.as_str()))
        .map(|session| session.title.clone())
        .collect();
    let agents = store
        .agent_priorities
        .iter()
        .filter(|(_, priority)| {
            priority
                .environment_ids
                .iter()
                .any(|id| deleted_environments.contains(id))
        })
        .map(|(member_id, _)| member_id.clone())
        .collect();
    PoolAccountDeletionImpact {
        sessions,
        agents,
        unknown_sessions: Vec::new(),
    }
}

fn account_deletion_impact(
    store: &Store,
    sessions: &[PoolAccountSessionView],
    account_id: &str,
) -> (Vec<String>, PoolAccountDeletionImpact) {
    let removed = store.clone().delete_account(account_id);
    let impact = deletion_impact(store, sessions, Some(account_id), &removed);
    (removed, impact)
}

fn account_session_view(
    sessions_dir: &Path,
    active: ActiveAgentSession,
    project_root: Option<&Path>,
) -> Option<PoolAccountSessionView> {
    let session =
        gwt_agent::session_ledger::load_session_candidate(sessions_dir, &active.session_id)?;
    let is_pm = gwt::pm_registry::session_is_registered_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(
            project_root.unwrap_or_else(|| Path::new(&active.agent_project_root)),
        ),
        &session.id,
    );
    let account = session.execution_account?;
    Some(PoolAccountSessionView {
        id: active.window_id,
        title: active.display_name,
        is_pm,
        execution_account: PoolExecutionAccountView {
            account_id: account.account_id,
        },
        account_switches: session.account_switches,
    })
}

/// Issue #7652 (SPEC #7007 items 243 and 251): the execution environment a
/// manual launch with `tool` runs on, its place on the project's list, and
/// why each entry ahead of it is not the one used.
pub(crate) fn launch_wizard_environment_view(
    store: &Store,
    prefs: &gwt::IssueMonitorPrefs,
    tool: &str,
    installed: &dyn Fn(&str) -> bool,
) -> Option<gwt::LaunchWizardExecutionEnvironmentView> {
    let agent = gwt_agent::resolve_agent_id(tool)?;
    let (_, list) = environments::project_list(store, prefs);
    let position = list
        .iter()
        .position(|entry| gwt_agent::resolve_agent_id(&entry.tool).as_ref() == Some(&agent));
    let environment = match position {
        Some(index) => store.environment(&list[index].environment_id),
        None => store.environment_for_tool(tool),
    };
    if position.is_none() && environment.is_none() {
        return None;
    }
    let chosen = tool_name(tool);
    // A tool the project launches without a registered environment keeps its
    // place on the list under its own name.
    let entry_name = |entry: &ProjectExecutionEnvironment| {
        store
            .environment(&entry.environment_id)
            .map_or_else(|| tool_name(&entry.tool), |found| found.name.clone())
    };
    let skipped = list[..position.unwrap_or(0)]
        .iter()
        .enumerate()
        .map(|(index, entry)| gwt::LaunchWizardSkippedEnvironmentView {
            priority: index + 1,
            name: entry_name(entry),
            reason: if installed(&entry.tool) {
                format!("{chosen} was chosen for this launch.")
            } else {
                format!(
                    "{} is not installed on this machine.",
                    tool_name(&entry.tool)
                )
            },
        })
        .collect();
    Some(gwt::LaunchWizardExecutionEnvironmentView {
        name: environment.map_or_else(|| chosen.clone(), |found| found.name.clone()),
        via_tool_name: chosen,
        priority: position.map(|index| index + 1),
        list_len: list.len(),
        note: match position {
            Some(0) => "First on this project's list.".to_string(),
            Some(index) => format!(
                "Number {} of {} on this project's list.",
                index + 1,
                list.len()
            ),
            None => "Not on this project's list. It is used for this launch only.".to_string(),
        },
        skipped,
    })
}

/// The list a project saves for `environment_ids`, in that order. An id is an
/// environment, or a tool the project already launches without one.
pub(crate) fn project_list_for_ids(
    store: &Store,
    current: &[ProjectExecutionEnvironment],
    environment_ids: &[String],
) -> Result<Vec<ProjectExecutionEnvironment>, String> {
    if environment_ids.is_empty() {
        return Err(
            "Keep at least one execution environment. A project with none cannot start a session."
                .into(),
        );
    }
    let list = environment_ids
        .iter()
        .map(|id| {
            store
                .environment(id)
                .map(|environment| ProjectExecutionEnvironment {
                    environment_id: environment.id.clone(),
                    tool: environment.via_tool.clone(),
                })
                .or_else(|| {
                    current
                        .iter()
                        .find(|entry| entry.environment_id == *id)
                        .cloned()
                })
                .ok_or_else(|| {
                    "An execution environment on the list no longer exists. Reload and try again."
                        .to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let normalized = gwt::normalize_project_execution_environments(list.clone());
    if normalized.len() != list.len() {
        return Err(
            "Two environments on the list start sessions with the same tool. Keep one of them."
                .into(),
        );
    }
    Ok(normalized)
}

/// Each project's list once `environment_ids` are deleted, for the projects
/// whose list has to change. A list that would become empty is left alone:
/// that project goes back to choosing from its launch candidates.
pub(crate) fn lists_without_environments(
    store: &Store,
    projects: &[KnownProject],
    environment_ids: &[String],
) -> Vec<(PathBuf, Vec<ProjectExecutionEnvironment>)> {
    projects
        .iter()
        .filter_map(|project| {
            let (_, list) = environments::project_list(store, &project.prefs);
            let kept: Vec<_> = list
                .iter()
                .filter(|entry| !environment_ids.contains(&entry.environment_id))
                .cloned()
                .collect();
            (kept.len() != list.len() && !kept.is_empty()).then(|| (project.root.clone(), kept))
        })
        .collect()
}

impl AppRuntime {
    /// Projects open in a tab, then recently opened ones.
    pub(super) fn known_projects(&self) -> Vec<KnownProject> {
        let mut seen = std::collections::BTreeSet::new();
        self.tabs
            .iter()
            .map(|tab| (tab.project_root.clone(), tab.title.clone()))
            .chain(
                self.recent_projects
                    .iter()
                    .map(|project| (project.path.clone(), project.title.clone())),
            )
            .filter(|(root, _)| seen.insert(root.clone()))
            .filter_map(|(root, title)| {
                let prefs = gwt::load_issue_monitor_prefs(
                    &gwt::issue_monitor_prefs_path_for_repo_path(&root),
                )
                .ok()?;
                Some(KnownProject { root, title, prefs })
            })
            .collect()
    }

    pub(super) fn installed_tools(&self) -> Vec<String> {
        self.launch_wizard_cache
            .agent_options()
            .into_iter()
            .filter(|option| option.available)
            .map(|option| option.id)
            .collect()
    }

    fn execution_environments_event(
        store: std::io::Result<Store>,
        projects: &[KnownProject],
        installed: &[String],
        project_root: Option<&Path>,
        active: Vec<(ActiveAgentSession, Option<PathBuf>)>,
        sessions_dir: &Path,
    ) -> BackendEvent {
        match store {
            Ok(store) => {
                let is_installed = |tool: &str| {
                    gwt_agent::resolve_agent_id(tool).is_some_and(|agent| {
                        installed
                            .iter()
                            .any(|id| gwt_agent::resolve_agent_id(id).as_ref() == Some(&agent))
                    })
                };
                let mut view = execution_environments_view(
                    &store,
                    projects,
                    project_root,
                    &is_installed,
                    &environments::tool_settings,
                );
                let unknown_via_sessions = |ids: &[String]| {
                    let mut titles: Vec<_> = active
                        .iter()
                        .filter(|(session, _)| {
                            ids.iter().any(|id| {
                                store.environment(id).is_some_and(|env| {
                                    store.kind(env) == environments::EnvironmentKind::Via
                                        && env.via_tool == session.agent_id
                                })
                            })
                        })
                        .map(|(session, _)| session.display_name.clone())
                        .collect();
                    titles.sort();
                    titles
                };
                let mut sessions: Vec<_> = active
                    .iter()
                    .filter_map(|(active, root)| {
                        let visible =
                            project_root.is_none_or(|scope| root.as_deref() == Some(scope));
                        Some((
                            visible,
                            account_session_view(sessions_dir, active.clone(), root.as_deref())?,
                        ))
                    })
                    .collect();
                sessions.sort_by(|(_, a), (_, b)| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
                let all_sessions: Vec<_> = sessions.iter().map(|(_, row)| row.clone()).collect();
                view.active_sessions = sessions
                    .into_iter()
                    .filter_map(|(visible, row)| visible.then_some(row))
                    .collect();
                view.pool_providers = POOL_PROVIDER_CHOICES
                    .iter()
                    .map(|&tool| PoolAccountProviderView {
                        tool: tool.to_owned(),
                        tool_name: tool_name(tool),
                        blocked_reason: Store::pool_add_refusal(tool)
                            .map(|reason| reason.0)
                            .or_else(|| {
                                (!is_installed(tool))
                                    .then(|| "Install this tool before adding an account.".into())
                            }),
                    })
                    .collect();
                let names = gwt::agent_registry::SessionMemberSnapshot::load()
                    .map(|snapshot| {
                        snapshot
                            .registered_members()
                            .into_iter()
                            .collect::<HashMap<_, _>>()
                    })
                    .unwrap_or_default();
                for (row, account) in view.accounts.iter_mut().zip(&store.accounts) {
                    let sign_in = account_homes::pool_sign_in(account);
                    row.signed_in = sign_in.is_some();
                    row.sign_in_identity = sign_in.and_then(|state| state.identity);
                    row.sign_in_blocked_reason =
                        account_homes::pool_login_refusal(account).map(str::to_owned);
                    row.delete_blocked_reason = store
                        .pool_delete_refusal(&account.id)
                        .map(|reason| reason.0);
                    if row.delete_blocked_reason.is_none() {
                        let (removed, mut impact) =
                            account_deletion_impact(&store, &all_sessions, &account.id);
                        impact.unknown_sessions = unknown_via_sessions(&removed);
                        name_affected_agents(&mut impact, &names);
                        row.delete_impact = Some(impact);
                    }
                }
                for row in &mut view.environments {
                    if row.delete_blocked_reason.is_none() {
                        let mut impact = deletion_impact(
                            &store,
                            &all_sessions,
                            None,
                            std::slice::from_ref(&row.id),
                        );
                        impact.unknown_sessions =
                            unknown_via_sessions(std::slice::from_ref(&row.id));
                        name_affected_agents(&mut impact, &names);
                        row.delete_impact = Some(impact);
                    }
                }
                BackendEvent::ExecutionEnvironments {
                    view: Box::new(view),
                }
            }
            Err(error) => BackendEvent::ExecutionEnvironmentError {
                message: format!("Could not read the execution environments: {error}"),
                project_root: project_root.map(|path| path.display().to_string()),
            },
        }
    }

    fn refresh_execution_environments(
        &self,
        client_id: &str,
        context: Option<&ProjectContext>,
    ) -> Vec<OutboundEvent> {
        self.refresh_execution_environments_after(client_id, context, || Ok(()))
    }

    fn refresh_execution_environments_after(
        &self,
        client_id: &str,
        context: Option<&ProjectContext>,
        before_refresh: impl FnOnce() -> std::io::Result<()> + Send + 'static,
    ) -> Vec<OutboundEvent> {
        let projects = self.known_projects();
        let installed = self.installed_tools();
        let root = context.map(|context| context.project_root.clone());
        let active = self
            .active_agent_sessions
            .values()
            .map(|active| {
                (
                    active.clone(),
                    self.tab(&active.tab_id).map(|tab| tab.project_root.clone()),
                )
            })
            .collect();
        let sessions_dir = self.sessions_dir.clone();
        let proxy = self.proxy.clone();
        let reply_to = client_id.to_owned();
        match self.blocking_tasks.try_spawn(move || {
            let event = match before_refresh() {
                Ok(()) => Self::execution_environments_event(
                    environments::adopt_signed_in_accounts(),
                    &projects,
                    &installed,
                    root.as_deref(),
                    active,
                    &sessions_dir,
                ),
                Err(error) => BackendEvent::ExecutionEnvironmentError {
                    message: error.to_string(),
                    project_root: root.map(|path| path.display().to_string()),
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                reply_to, event,
            )]));
        }) {
            Ok(_) => Vec::new(),
            Err(error) => Self::execution_environment_error(client_id, context, error.to_string()),
        }
    }

    fn execution_environment_error(
        client_id: &str,
        context: Option<&ProjectContext>,
        message: String,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::ExecutionEnvironmentError {
                message,
                project_root: project_root(context),
            },
        )]
    }

    /// Save one project's list through the daemon that owns its settings,
    /// or directly when no daemon is running for it.
    fn save_project_execution_environments(
        &self,
        project_root: &Path,
        list: Vec<ProjectExecutionEnvironment>,
    ) -> Result<(), String> {
        let publication = self.publish_issue_monitor_control(
            project_root,
            serde_json::json!({ "execution_environments": list }),
        );
        match publication {
            Ok(()) => Ok(()),
            Err(error) if error.allows_local_fallback() => self
                .commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    monitor.set_execution_environments(list);
                })
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn execution_environment_events(
        &mut self,
        client_id: &str,
        context: Option<&ProjectContext>,
        event: FrontendEvent,
    ) -> Vec<OutboundEvent> {
        if matches!(
            event,
            FrontendEvent::AddPoolAccount { .. } | FrontendEvent::SignInPoolAccount { .. }
        ) && self.pool_account_logins.contains_key(client_id)
        {
            return Self::execution_environment_error(
                client_id,
                context,
                "Close the current sign-in terminal first.".into(),
            );
        }
        let outcome: Result<(), String> = match event {
            FrontendEvent::GetExecutionEnvironments => {
                return self.refresh_execution_environments(client_id, context);
            }
            FrontendEvent::AddPoolAccount { tool } => {
                return self.prepare_pool_account_login(client_id, context, move || {
                    environments::enroll_pool_account(&tool).map_err(|error| error.to_string())
                });
            }
            FrontendEvent::SignInPoolAccount { account_id } => {
                return self.prepare_pool_account_login(client_id, context, move || {
                    environments::load()
                        .map_err(|error| error.to_string())?
                        .account(&account_id)
                        .cloned()
                        .ok_or_else(|| "Account not found.".to_owned())
                });
            }
            FrontendEvent::PoolAccountLoginInput { login_id, data } => {
                if let Some(login) = self
                    .pool_account_logins
                    .get(client_id)
                    .filter(|login| login.id == login_id)
                    .and_then(|login| login.active.as_ref())
                {
                    if login.input.try_send(data).is_err() {
                        return Self::execution_environment_error(
                            client_id,
                            context,
                            "The sign-in terminal is busy or closed.".into(),
                        );
                    }
                }
                return Vec::new();
            }
            FrontendEvent::ClosePoolAccountLogin { login_id } => {
                if self
                    .pool_account_logins
                    .get(client_id)
                    .is_some_and(|login| login.id == login_id)
                {
                    self.close_pool_account_login(client_id);
                }
                return Vec::new();
            }
            FrontendEvent::ResumeNextPoolAccount { id } => {
                let mut events = self.resume_with_next_account_events(&id);
                events.extend(self.refresh_execution_environments(client_id, context));
                return events;
            }
            FrontendEvent::MovePoolAccount {
                account_id,
                direction,
            } => {
                return self.refresh_execution_environments_after(client_id, context, move || {
                    environments::mutate(|store| {
                        Ok(store.move_pool_account(
                            &account_id,
                            direction == PoolAccountDirection::Up,
                        )?)
                    })
                    .map(|_| ())
                });
            }
            FrontendEvent::UsePoolAccount { account_id } => {
                return self.refresh_execution_environments_after(client_id, context, move || {
                    environments::mutate(|store| Ok(store.use_pool_account(&account_id)?))
                        .map(|_| ())
                });
            }
            FrontendEvent::DeletePoolAccount { account_id } => {
                return match self.delete_pool_account(&account_id) {
                    Ok(()) => {
                        let mut events = self.resume_after_account_deletion(&account_id);
                        events.extend(self.refresh_execution_environments(client_id, context));
                        events
                    }
                    Err(message) => Self::execution_environment_error(client_id, context, message),
                };
            }
            FrontendEvent::AddExecutionEnvironment {
                account_id,
                via_tool,
                name,
            } => environments::mutate(|store| {
                store.register_via(&account_id, &via_tool, name.as_deref())?;
                Ok(())
            })
            .map(|_| ())
            .map_err(|error| error.to_string()),
            FrontendEvent::UpdateExecutionEnvironment {
                environment_id,
                name,
                runtime,
            } => environments::mutate(|store| {
                store.update_environment(&environment_id, name.as_deref(), runtime)?;
                Ok(())
            })
            .map(|_| ())
            .map_err(|error| error.to_string()),
            FrontendEvent::DeleteExecutionEnvironment { environment_id } => {
                self.delete_execution_environment(&environment_id)
            }
            FrontendEvent::SetProjectExecutionEnvironments { environment_ids } => match context {
                Some(context) => {
                    self.set_project_execution_environments(&context.project_root, &environment_ids)
                }
                None => Err("Open a project to change its execution environments.".into()),
            },
            FrontendEvent::SetPmExecutionEnvironments { environment_ids } => {
                environments::mutate(|store| Ok(store.set_pm_list(&environment_ids)?))
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            FrontendEvent::SetAgentExecutionEnvironments {
                member_id,
                environment_ids,
            } => self.set_agent_execution_environments(&member_id, &environment_ids),
            _ => return Vec::new(),
        };
        match outcome {
            Ok(()) => self.refresh_execution_environments(client_id, context),
            Err(message) => Self::execution_environment_error(client_id, context, message),
        }
    }

    fn prepare_pool_account_login(
        &mut self,
        client_id: &str,
        context: Option<&ProjectContext>,
        prepare: impl FnOnce() -> Result<environments::Account, String> + Send + 'static,
    ) -> Vec<OutboundEvent> {
        let login_id = uuid::Uuid::new_v4().to_string();
        let pending_id = login_id.clone();
        let reply_to = client_id.to_owned();
        let project_root = context.map(|context| context.project_root.clone());
        let proxy = self.proxy.clone();
        match self.blocking_tasks.try_spawn(move || {
            proxy.send(UserEvent::PoolAccountLoginPrepared {
                client_id: reply_to,
                login_id,
                project_root,
                result: prepare(),
            });
        }) {
            Ok(_) => {
                self.pool_account_logins.insert(
                    client_id.to_owned(),
                    PoolAccountLoginTerminal {
                        id: pending_id,
                        active: None,
                    },
                );
                Vec::new()
            }
            Err(error) => Self::execution_environment_error(client_id, context, error.to_string()),
        }
    }

    pub(crate) fn pool_account_login_prepared(
        &mut self,
        client_id: String,
        login_id: String,
        root: Option<PathBuf>,
        result: Result<environments::Account, String>,
    ) -> Vec<OutboundEvent> {
        if self
            .pool_account_logins
            .get(&client_id)
            .is_none_or(|login| login.id != login_id)
        {
            return Vec::new();
        }
        let outcome = result
            .and_then(|account| self.start_pool_account_login(&client_id, &login_id, &account));
        let context = root.as_ref().and_then(|root| {
            self.project_contexts()
                .into_iter()
                .find(|context| context.project_root == *root)
        });
        match outcome {
            Ok(event) => {
                let mut events = vec![OutboundEvent::reply(&client_id, event)];
                events.extend(self.refresh_execution_environments(&client_id, context.as_ref()));
                events
            }
            Err(message) => {
                self.close_pool_account_login(&client_id);
                Self::execution_environment_error(&client_id, context.as_ref(), message)
            }
        }
    }

    fn start_pool_account_login(
        &mut self,
        client_id: &str,
        id: &str,
        account: &environments::Account,
    ) -> Result<BackendEvent, String> {
        if environments::load()
            .map_err(|error| error.to_string())?
            .account(&account.id)
            != Some(account)
        {
            return Err("This account changed. Reload and try again.".into());
        }
        if self
            .pool_account_logins
            .values()
            .filter_map(|login| login.active.as_ref())
            .any(|login| login.account_id == account.id)
        {
            return Err("This account already has an open sign-in terminal.".into());
        }
        let command = account_homes::login_command(account).map_err(|error| error.to_string())?;
        let pty = Arc::new(
            PtyHandle::spawn(SpawnConfig {
                command: command.program,
                args: command.args,
                env: command.env,
                remove_env: command.remove_env,
                cols: 80,
                rows: 24,
                cwd: gwt_config::home::user_home(),
            })
            .map_err(|error| format!("Could not open the sign-in terminal: {error}"))?,
        );
        let mut reader = pty.reader().map_err(|error| error.to_string())?;
        let (input, submitted) = mpsc::sync_channel::<String>(64);
        let writer = Arc::clone(&pty);
        std::thread::Builder::new()
            .name("pool-account-login-input".into())
            .spawn(move || {
                for text in submitted {
                    if writer.write_input(text.as_bytes()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("Could not open input for the sign-in terminal: {error}"))?;
        self.pool_account_logins
            .get_mut(client_id)
            .ok_or("The sign-in request was closed.")?
            .active = Some(ActivePoolAccountLogin {
            account_id: account.id.clone(),
            pty,
            input,
        });
        let reply_to = client_id.to_owned();
        let login_id = id.to_owned();
        let proxy = self.proxy.clone();
        let worker = std::thread::Builder::new()
            .name("pool-account-login".into())
            .spawn(move || {
                let mut bytes = [0; 8192];
                // This terminal is client-scoped and has no persistent transcript.
                while let Ok(count) = reader.read(&mut bytes) {
                    if count == 0 {
                        break;
                    }
                    proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                        reply_to.clone(),
                        BackendEvent::PoolAccountLoginOutput {
                            login_id: login_id.clone(),
                            data_base64: base64::engine::general_purpose::STANDARD
                                .encode(&bytes[..count]),
                        },
                    )]));
                }
                drop(reader);
                proxy.send(UserEvent::PoolAccountLoginEnded {
                    client_id: reply_to,
                    login_id,
                });
            });
        if let Err(error) = worker {
            self.close_pool_account_login(client_id);
            return Err(format!("Could not stream the sign-in terminal: {error}"));
        }
        Ok(BackendEvent::PoolAccountLoginStarted {
            login_id: id.into(),
            account_id: account.id.clone(),
        })
    }

    pub(crate) fn close_pool_account_login(&mut self, client_id: &str) {
        if let Some(login) = self
            .pool_account_logins
            .remove(client_id)
            .and_then(|login| login.active)
        {
            let _ = login.pty.kill();
        }
    }

    pub(crate) fn pool_account_login_ended(
        &mut self,
        client_id: String,
        login_id: String,
    ) -> Vec<OutboundEvent> {
        if self
            .pool_account_logins
            .get(&client_id)
            .is_none_or(|login| login.id != login_id)
        {
            return Vec::new();
        }
        self.close_pool_account_login(&client_id);
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::PoolAccountLoginFinished { login_id },
        )]
    }

    fn delete_pool_account(&self, account_id: &str) -> Result<(), String> {
        if self
            .pool_account_logins
            .values()
            .filter_map(|login| login.active.as_ref())
            .any(|login| login.account_id == account_id)
        {
            return Err("Close this account's sign-in terminal before deleting it.".into());
        }
        let projects = self.known_projects();
        environments::mutate(|store| {
            if let Some(refusal) = store.pool_delete_refusal(account_id) {
                return Err(refusal.into());
            }
            let before = store.clone();
            let deleted = store.delete_account(account_id);
            for (root, list) in lists_without_environments(&before, &projects, &deleted) {
                self.save_project_execution_environments(&root, list)
                    .map_err(std::io::Error::other)?;
            }
            // Running CLI processes may still be finishing their current turn.
            // Removing the account from the store prevents new launches with it.
            Ok(())
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    fn set_project_execution_environments(
        &self,
        project_root: &Path,
        environment_ids: &[String],
    ) -> Result<(), String> {
        let store = environments::load().map_err(|error| error.to_string())?;
        let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
            project_root,
        ))
        .map_err(|error| error.to_string())?;
        let (_, current) = environments::project_list(&store, &prefs);
        let list = project_list_for_ids(&store, &current, environment_ids)?;
        self.save_project_execution_environments(project_root, list)
    }

    fn set_agent_execution_environments(
        &self,
        member_id: &str,
        environment_ids: &[String],
    ) -> Result<(), String> {
        let registry = gwt::agent_registry::SessionMemberSnapshot::load()
            .map_err(|error| error.to_string())?;
        if !registry
            .registered_members()
            .iter()
            .any(|(id, _)| id == member_id)
        {
            return Err("This agent is no longer registered. Reload and try again.".into());
        }
        environments::mutate(|store| Ok(store.set_agent_list(member_id, environment_ids)?))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Take the environment off every project's list first, then delete it,
    /// so a failure part-way never leaves a list naming a deleted environment.
    fn delete_execution_environment(&self, environment_id: &str) -> Result<(), String> {
        let store = environments::load().map_err(|error| error.to_string())?;
        if let Some(refusal) = store.delete_refusal(environment_id) {
            return Err(refusal.0);
        }
        let projects = self.known_projects();
        for (root, list) in
            lists_without_environments(&store, &projects, &[environment_id.to_string()])
        {
            self.save_project_execution_environments(&root, list)
                .map_err(|error| {
                    format!(
                        "Could not take the environment off {}: {error}",
                        root.display()
                    )
                })?;
        }
        environments::mutate(|store| Ok(store.delete_environment(environment_id)?))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn name_affected_agents(impact: &mut PoolAccountDeletionImpact, names: &HashMap<String, String>) {
    for member_id in &mut impact.agents {
        if let Some(name) = names.get(member_id) {
            member_id.clone_from(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_the_last_pool_account_lists_affected_sessions_and_agents() {
        let mut store = store();
        store
            .set_agent_list("reviewer", &["codex".into(), "claude".into()])
            .unwrap();
        store.set_agent_list("writer", &["claude".into()]).unwrap();
        let sessions = [PoolAccountSessionView {
            id: "window".into(),
            title: "Review".into(),
            is_pm: false,
            execution_account: PoolExecutionAccountView {
                account_id: "codex".into(),
            },
            account_switches: Vec::new(),
        }];
        let (removed, impact) = account_deletion_impact(&store, &sessions, "codex");
        assert_eq!(removed, ["codex"]);
        assert_eq!(impact.sessions, ["Review"]);
        assert_eq!(impact.agents, ["reviewer"]);
    }

    fn store() -> Store {
        let mut store = Store::default();
        store.register_account("claude");
        store.register_account("codex");
        store.register_via("claude", "hermes", None).unwrap();
        store
    }

    fn project(title: &str, tools: &[&str], saved: &[(&str, &str)]) -> KnownProject {
        let mut prefs = gwt::IssueMonitorPrefs::default();
        prefs.set_launch_profile_pool(
            tools
                .iter()
                .map(|tool| {
                    serde_json::from_value(serde_json::json!({ "agent_id": tool })).unwrap()
                })
                .collect(),
        );
        prefs.execution_environments = saved
            .iter()
            .map(|(environment_id, tool)| ProjectExecutionEnvironment {
                environment_id: environment_id.to_string(),
                tool: tool.to_string(),
            })
            .collect();
        KnownProject {
            root: PathBuf::from(format!("/projects/{title}")),
            title: title.to_string(),
            prefs,
        }
    }

    fn view(projects: &[KnownProject], current: Option<&str>) -> ExecutionEnvironmentsView {
        execution_environments_view(
            &store(),
            projects,
            current.map(Path::new),
            &|tool| tool != "opencode",
            &|_| environments::ToolSettings {
                path: Some("/home/.hermes/config.yaml".into()),
                values: vec![environments::ToolSetting {
                    label: "Model".into(),
                    value: "glm-4.6".into(),
                }],
            },
        )
    }

    #[test]
    fn a_launch_names_its_environment_its_place_and_why_earlier_ones_are_not_used() {
        let wizard = |saved: &[(&str, &str)], tool: &str| {
            let project = project("gwt", &["claude", "codex"], saved);
            launch_wizard_environment_view(&store(), &project.prefs, tool, &|tool| tool != "hermes")
        };

        // The head of the list has nothing ahead of it.
        let head = wizard(&[], "claude").unwrap();
        assert_eq!(head.name, "Claude Code 1");
        assert_eq!(head.via_tool_name, "Claude Code");
        assert_eq!((head.priority, head.list_len), (Some(1), 2));
        assert!(head.skipped.is_empty());
        assert!(head.note.contains("First"), "{}", head.note);

        // A later entry names every entry ahead of it with its reason.
        let saved = [
            ("claude:hermes", "hermes"),
            ("claude", "claude"),
            ("codex", "codex"),
        ];
        let third = wizard(&saved, "codex").unwrap();
        assert_eq!((third.priority, third.list_len), (Some(3), 3));
        assert!(third.note.contains("3 of 3"), "{}", third.note);
        let skipped: Vec<_> = third
            .skipped
            .iter()
            .map(|entry| (entry.priority, entry.name.as_str()))
            .collect();
        assert_eq!(
            skipped,
            [
                (1, "Claude Code 1 (via Hermes Agent)"),
                (2, "Claude Code 1")
            ]
        );
        assert!(third.skipped[0].reason.contains("not installed"));
        assert!(third.skipped[1].reason.contains("Codex was chosen"));

        // A tool off the list still launches, and says so.
        let off = wizard(&[("claude", "claude")], "codex").unwrap();
        assert_eq!((off.priority, off.list_len), (None, 1));
        assert_eq!(off.name, "Codex 1");
        assert!(off.note.contains("Not on this project's list"));
        assert!(off.skipped.is_empty());

        // A tool with no environment anywhere shows nothing.
        assert!(wizard(&[], "my-custom-agent").is_none());
        assert!(wizard(&[("claude", "claude")], "openclaw").is_none());
    }

    #[test]
    fn each_environment_says_what_can_be_done_to_it_and_why_not() {
        let view = view(&[], None);
        let by_id = |id: &str| view.environments.iter().find(|e| e.id == id).unwrap();

        let own = by_id("claude");
        assert_eq!(own.kind, environments::EnvironmentKind::Set);
        assert_eq!(own.account_name, "Claude Code 1");
        assert_eq!(own.via_tool_name, "Claude Code");
        assert!(own
            .delete_blocked_reason
            .as_deref()
            .unwrap()
            .contains("account is deleted"));
        assert!(own.runtime_read_only_reason.is_none() && own.tool_settings.is_none());

        let through = by_id("claude:hermes");
        assert_eq!(through.kind, environments::EnvironmentKind::Via);
        assert!(through.delete_blocked_reason.is_none());
        assert!(through
            .runtime_read_only_reason
            .as_deref()
            .unwrap()
            .contains("its own settings file"));
        assert_eq!(
            through.tool_settings.as_ref().unwrap().values[0].value,
            "glm-4.6"
        );
        assert!(view.project.is_none());
    }

    #[test]
    fn the_project_manager_list_is_shown_with_or_without_an_open_project() {
        let names = |view: &ExecutionEnvironmentsView| -> Vec<String> {
            view.pm
                .entries
                .iter()
                .map(|entry| entry.name.clone().unwrap())
                .collect()
        };
        let unsaved = view(&[], None);
        assert!(!unsaved.pm.saved);
        assert_eq!(names(&unsaved), ["Claude Code 1", "Codex 1"]);

        let mut store = store();
        store
            .set_pm_list(&["claude:hermes".to_string(), "codex".to_string()])
            .unwrap();
        let saved = execution_environments_view(
            &store,
            &[project("alpha", &["codex"], &[])],
            Some(Path::new("/projects/alpha")),
            &|_| true,
            &|_| environments::ToolSettings::default(),
        );
        assert!(saved.pm.saved);
        assert_eq!(
            names(&saved),
            ["Claude Code 1 (via Hermes Agent)", "Codex 1"]
        );
        assert_eq!(saved.pm.entries[0].tool_name, "Hermes Agent");
    }

    #[test]
    fn a_pair_that_cannot_be_registered_is_offered_disabled_with_its_reason() {
        let view = view(&[], None);
        let reason = |account: &str, tool: &str| {
            view.via_options
                .iter()
                .find(|option| option.account_id == account && option.via_tool == tool)
                .unwrap()
                .blocked_reason
                .clone()
        };
        assert_eq!(
            view.via_options.len(),
            6,
            "every account and tool pair is shown"
        );
        assert_eq!(reason("codex", "openclaw"), None);
        assert!(reason("codex", "hermes")
            .unwrap()
            .contains("already registered"));
        assert!(reason("codex", "opencode")
            .unwrap()
            .contains("not installed"));
    }

    #[test]
    fn an_environment_lists_the_projects_that_use_it_with_their_priority() {
        let projects = [
            project("alpha", &["codex", "claude"], &[]),
            project(
                "beta",
                &["codex"],
                &[("claude:hermes", "hermes"), ("codex", "codex")],
            ),
        ];
        let view = view(&projects, Some("/projects/alpha"));
        let users = |id: &str| -> Vec<(String, usize)> {
            view.environments
                .iter()
                .find(|environment| environment.id == id)
                .unwrap()
                .projects
                .iter()
                .map(|project| (project.title.clone(), project.priority))
                .collect()
        };
        assert_eq!(
            users("codex"),
            [("alpha".to_string(), 1), ("beta".to_string(), 2)]
        );
        assert_eq!(users("claude"), [("alpha".to_string(), 2)]);
        assert_eq!(users("claude:hermes"), [("beta".to_string(), 1)]);

        // The open project's list is the one it launches with today.
        let list = view.project.unwrap();
        assert!(!list.saved);
        let names: Vec<_> = list
            .entries
            .iter()
            .map(|entry| entry.name.as_deref())
            .collect();
        assert_eq!(names, [Some("Codex 1"), Some("Claude Code 1")]);
    }

    #[test]
    fn a_project_list_is_saved_in_the_order_given_and_refused_with_a_reason() {
        let store = store();
        let current = vec![ProjectExecutionEnvironment {
            environment_id: "my-agent".into(),
            tool: "my-agent".into(),
        }];
        let ids = |ids: &[&str]| -> Vec<String> { ids.iter().map(|id| id.to_string()).collect() };
        let save = |list: &[&str]| project_list_for_ids(&store, &current, &ids(list));

        let saved = save(&["claude:hermes", "my-agent", "codex"]).unwrap();
        let tools: Vec<_> = saved.iter().map(|entry| entry.tool.as_str()).collect();
        assert_eq!(tools, ["hermes", "my-agent", "codex"]);

        assert!(save(&[]).unwrap_err().contains("at least one"));
        assert!(save(&["gone"]).unwrap_err().contains("no longer exists"));
        assert!(save(&["codex", "codex"]).unwrap_err().contains("same tool"));
    }

    #[test]
    fn deleting_an_environment_takes_it_off_every_project_that_lists_it() {
        let projects = [
            project(
                "saved",
                &[],
                &[("claude:hermes", "hermes"), ("codex", "codex")],
            ),
            project("unsaved", &["hermes", "claude"], &[]),
            project("only", &[], &[("claude:hermes", "hermes")]),
            project("unrelated", &["codex"], &[]),
        ];
        let changed = lists_without_environments(&store(), &projects, &["claude:hermes".into()]);
        let changed: Vec<_> = changed
            .iter()
            .map(|(root, list)| {
                let ids: Vec<_> = list
                    .iter()
                    .map(|entry| entry.environment_id.as_str())
                    .collect();
                (root.file_name().unwrap().to_str().unwrap(), ids)
            })
            .collect();
        assert_eq!(
            changed,
            [("saved", vec!["codex"]), ("unsaved", vec!["claude"])]
        );
    }
}
