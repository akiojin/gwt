//! Execution environment screens (Issue #7741, SPEC #7007 items 77-82).
//!
//! The definitions are machine-wide; each project's priority list is saved
//! with that project's Issues settings. This module builds the view the
//! screens show and applies their requests.

use super::*;
use gwt::execution_environments::{self as environments, Store};
use gwt::protocol::{
    ExecutionAccountView, ExecutionEnvironmentProjectView, ExecutionEnvironmentViaOptionView,
    ExecutionEnvironmentView, ExecutionEnvironmentsView, ProjectExecutionEnvironmentView,
    ProjectExecutionEnvironmentsView,
};
use gwt::ProjectExecutionEnvironment;

const TOOLS_THAT_READ_THEIR_OWN_SETTINGS: [&str; 3] = ["hermes", "openclaw", "opencode"];

fn tool_name(tool: &str) -> String {
    gwt_agent::resolve_agent_id(tool).map_or_else(
        || tool.to_string(),
        |agent| agent.display_name().to_string(),
    )
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
    ExecutionEnvironmentsView {
        accounts: store
            .accounts
            .iter()
            .map(|account| ExecutionAccountView {
                id: account.id.clone(),
                name: account.name.clone(),
                tool: account.tool.clone(),
                tool_name: tool_name(&account.tool),
            })
            .collect(),
        environments,
        via_options,
        project,
    }
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
    fn known_projects(&self) -> Vec<KnownProject> {
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

    fn installed_tools(&self) -> Vec<String> {
        self.launch_wizard_cache
            .agent_options()
            .into_iter()
            .filter(|option| option.available)
            .map(|option| option.id)
            .collect()
    }

    /// Carry an existing setup over: every tool that signs in to its own
    /// account and is installed here, or that a known project already
    /// launches, has its account and environment afterwards.
    fn ensure_execution_environments(
        &self,
        projects: &[KnownProject],
        installed: &[String],
    ) -> std::io::Result<Store> {
        let mut tools = installed.to_vec();
        for project in projects {
            tools.extend(project.prefs.current_launch_tools());
        }
        environments::ensure_accounts(&tools)
    }

    fn execution_environments_reply(
        &self,
        client_id: &str,
        context: Option<&ProjectContext>,
    ) -> Vec<OutboundEvent> {
        let projects = self.known_projects();
        let installed = self.installed_tools();
        let event = match self.ensure_execution_environments(&projects, &installed) {
            Ok(store) => {
                let is_installed = |tool: &str| {
                    gwt_agent::resolve_agent_id(tool).is_some_and(|agent| {
                        installed
                            .iter()
                            .any(|id| gwt_agent::resolve_agent_id(id).as_ref() == Some(&agent))
                    })
                };
                BackendEvent::ExecutionEnvironments {
                    view: Box::new(execution_environments_view(
                        &store,
                        &projects,
                        context.map(|context| context.project_root.as_path()),
                        &is_installed,
                        &environments::tool_settings,
                    )),
                }
            }
            Err(error) => BackendEvent::ExecutionEnvironmentError {
                message: format!("Could not read the execution environments: {error}"),
            },
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    fn execution_environment_error(client_id: &str, message: String) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::ExecutionEnvironmentError { message },
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
        let outcome: Result<(), String> = match event {
            FrontendEvent::GetExecutionEnvironments => Ok(()),
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
            _ => return Vec::new(),
        };
        match outcome {
            Ok(()) => self.execution_environments_reply(client_id, context),
            Err(message) => Self::execution_environment_error(client_id, message),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
