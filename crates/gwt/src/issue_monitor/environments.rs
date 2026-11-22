use super::*;

/// Issue #7741 (SPEC #7007 item 82): one entry of a project's "available
/// execution environments" list. The order of the list is the priority.
///
/// The tool is saved with the id because it never changes for an environment,
/// so choosing a launch candidate needs no other file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectExecutionEnvironment {
    pub environment_id: String,
    /// Command id of the tool this environment starts sessions with.
    pub tool: String,
}

/// Keep the first entry per tool: a project's launch candidates are unique
/// per tool, so a second environment through the same tool could never be
/// told apart from the first.
pub fn normalize_project_execution_environments(
    list: Vec<ProjectExecutionEnvironment>,
) -> Vec<ProjectExecutionEnvironment> {
    let mut seen = BTreeSet::new();
    list.into_iter()
        .filter_map(|entry| {
            let tool = normalize_issue_monitor_provider(&entry.tool)?;
            let environment_id = entry.environment_id.trim().to_string();
            (!environment_id.is_empty() && seen.insert(tool.clone())).then_some(
                ProjectExecutionEnvironment {
                    environment_id,
                    tool,
                },
            )
        })
        .collect()
}

fn environment_candidate(tool: &str) -> IssueMonitorLaunchProfile {
    IssueMonitorLaunchProfile {
        agent_member_id: None,
        agent_id: tool.to_string(),
        model: None,
        reasoning: None,
        version: None,
        session_mode: Default::default(),
        skip_permissions: false,
        fast_mode: false,
        codex_speed: Default::default(),
        runtime_inherit: false,
        runtime_target: Default::default(),
        docker_service: None,
        docker_lifecycle_intent: Default::default(),
        windows_shell: None,
        prefer_for: Vec::new(),
    }
}

impl IssueMonitorPrefs {
    /// The saved list's tools in priority order, or `None` when the list was
    /// never saved and launches choose as they always did.
    pub(super) fn environment_tools(&self) -> Option<Vec<String>> {
        let tools: Vec<_> =
            normalize_project_execution_environments(self.execution_environments.clone())
                .into_iter()
                .map(|entry| entry.tool)
                .collect();
        (!tools.is_empty()).then_some(tools)
    }

    /// The tools launches use today, in the order they are tried. This is the
    /// list a project shows before it has saved one, so saving it unchanged
    /// changes nothing.
    pub fn current_launch_tools(&self) -> Vec<String> {
        let pool = self.launch_profile_pool();
        let candidates: Vec<_> = match pool.first() {
            // A registered agent at the head fixes the tool.
            Some(head) if head.agent_member_id.is_some() => vec![head.clone()],
            _ => self
                .effective_launch_tiers()
                .into_iter()
                .flatten()
                .collect(),
        };
        let mut seen = BTreeSet::new();
        candidates
            .iter()
            .filter_map(|profile| normalize_issue_monitor_provider(&profile.agent_id))
            .filter(|tool| seen.insert(tool.clone()))
            .collect()
    }

    /// Candidates of a pool that names no registered agent, limited to the
    /// project's environments and put in their priority order. An environment
    /// the pool has no candidate for starts with the tool's defaults.
    pub(super) fn environment_candidates(
        &self,
        pool: Vec<IssueMonitorLaunchProfile>,
    ) -> Vec<IssueMonitorLaunchProfile> {
        let Some(tools) = self.environment_tools() else {
            return pool;
        };
        tools
            .iter()
            .map(|tool| {
                pool.iter()
                    .find(|profile| {
                        normalize_issue_monitor_provider(&profile.agent_id).as_ref() == Some(tool)
                    })
                    .cloned()
                    .unwrap_or_else(|| environment_candidate(tool))
            })
            .collect()
    }

    /// The saved pool as launches use it now: registered agents on the
    /// environment they start with, any other pool limited to the project's
    /// environments.
    pub fn preview_launch_profile_pool(&self) -> Vec<IssueMonitorLaunchProfile> {
        let pool = self.launch_profile_pool();
        if pool
            .first()
            .is_some_and(|profile| profile.agent_member_id.is_some())
        {
            return self.agent_candidates_on_environment(pool);
        }
        self.environment_candidates(pool)
    }

    /// Issue #7647 (SPEC #7007 items 82 and 243): a registered agent is not
    /// tied to a tool. Its sessions start with the first of the project's
    /// environments that is not held by a refused launch; when every one is
    /// held, the first is kept so the wait names it. `None` when the project
    /// never saved a list, where an agent keeps the tool it was registered
    /// with.
    pub(super) fn environment_tool_for_agents(&self) -> Option<String> {
        let tools = self.environment_tools()?;
        let now = gwt_core::timestamp::UtcTimestamp::now();
        tools
            .iter()
            .find(|tool| hold_reset_after(&self.provider_quota_holds, tool, now).is_none())
            .or(tools.first())
            .cloned()
    }

    /// Registered agents' saved candidates as they start now. The saved pool
    /// itself keeps each agent's registered tool.
    pub(super) fn agent_candidates_on_environment(
        &self,
        candidates: Vec<IssueMonitorLaunchProfile>,
    ) -> Vec<IssueMonitorLaunchProfile> {
        let Some(tool) = self.environment_tool_for_agents() else {
            return candidates;
        };
        candidates
            .into_iter()
            .map(|candidate| candidate.on_tool(&tool))
            .collect()
    }

    /// Whether `tool` is one of the project's saved environments, which any
    /// registered agent may start a session with.
    pub fn environment_lists_tool(&self, tool: &str) -> bool {
        normalize_issue_monitor_provider(tool).is_some_and(|tool| {
            self.environment_tools()
                .is_some_and(|tools| tools.contains(&tool))
        })
    }

    /// Order one tier's candidates by the project's environments and drop the
    /// ones the project does not use.
    pub(super) fn environment_tier(
        &self,
        tier: Vec<IssueMonitorLaunchProfile>,
    ) -> Vec<IssueMonitorLaunchProfile> {
        let Some(tools) = self.environment_tools() else {
            return tier;
        };
        let position = |profile: &IssueMonitorLaunchProfile| {
            let tool = normalize_issue_monitor_provider(&profile.agent_id)?;
            tools.iter().position(|listed| *listed == tool)
        };
        let mut tier: Vec<_> = tier
            .into_iter()
            .filter_map(|profile| Some((position(&profile)?, profile)))
            .collect();
        tier.sort_by_key(|(position, _)| *position);
        tier.into_iter().map(|(_, profile)| profile).collect()
    }
}

impl IssueMonitorLaunchProfile {
    /// This candidate started with `tool`. Tuning saved for another tool does
    /// not carry over: the agent's traits and the environment's runtime
    /// options fill it in when the session starts.
    pub fn on_tool(mut self, tool: &str) -> Self {
        if normalize_issue_monitor_provider(&self.agent_id).as_deref() != Some(tool) {
            self.agent_id = tool.to_string();
            self.model = None;
            self.reasoning = None;
            self.version = None;
            self.fast_mode = false;
            self.codex_speed = Default::default();
        }
        self
    }
}

impl IssueMonitorState {
    /// Replace the project's available execution environments. The order is
    /// the priority; an empty list returns the project to its earlier choice.
    pub fn set_execution_environments(&mut self, list: Vec<ProjectExecutionEnvironment>) -> bool {
        let list = normalize_project_execution_environments(list);
        if self.execution_environments == list {
            return false;
        }
        self.execution_environments = list;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-20T00:00:00Z";
    const LATER: &str = "2999-01-01T00:00:00Z";

    fn candidate(agent_id: &str, member: Option<&str>) -> IssueMonitorLaunchProfile {
        IssueMonitorLaunchProfile {
            agent_member_id: member.map(str::to_string),
            model: Some(format!("{agent_id}-model")),
            ..environment_candidate(agent_id)
        }
    }

    fn listed(entries: &[(&str, &str)]) -> Vec<ProjectExecutionEnvironment> {
        entries
            .iter()
            .map(|(environment_id, tool)| ProjectExecutionEnvironment {
                environment_id: environment_id.to_string(),
                tool: tool.to_string(),
            })
            .collect()
    }

    fn tools(profiles: &[IssueMonitorLaunchProfile]) -> Vec<&str> {
        profiles
            .iter()
            .map(|profile| profile.agent_id.as_str())
            .collect()
    }

    fn pick(prefs: &IssueMonitorPrefs, issue_number: u64) -> Option<String> {
        let pool = prefs.registered_launch_profiles_for_issue(issue_number);
        let selection = select_launch_profile(
            &pool,
            &prefs.launch_admission_provider_quota_holds(),
            &[],
            None,
            NOW,
        );
        selection.selected.map(|index| pool[index].agent_id.clone())
    }

    fn roster_prefs() -> IssueMonitorPrefs {
        let member = |id: &str, tool: &str| {
            serde_json::json!({
                "agent_member_id": id, "agent_id": tool, "name": id,
                "enabled": true, "created_at": NOW,
            })
        };
        let mut prefs = IssueMonitorPrefs {
            agent_roster: Some(
                serde_json::from_value(serde_json::json!({
                    "schema_version": 1, "revision": 1,
                    "members": [member("m-claude", "claude"), member("m-codex", "codex")],
                }))
                .unwrap(),
            ),
            ..IssueMonitorPrefs::default()
        };
        prefs.set_launch_profile_pool(vec![
            candidate("claude", Some("m-claude")),
            candidate("codex", Some("m-codex")),
        ]);
        prefs
    }

    #[test]
    fn a_project_that_never_saved_a_list_chooses_as_before() {
        let mut prefs = IssueMonitorPrefs::default();
        prefs.set_launch_profile_pool(vec![candidate("codex", None), candidate("claude", None)]);
        let before = prefs.registered_launch_profiles_for_issue(42);

        assert_eq!(tools(&before), ["codex", "claude"]);
        assert_eq!(prefs.current_launch_tools(), ["codex", "claude"]);
        // Saving the list a project already shows changes nothing.
        prefs.execution_environments = listed(&[("codex", "codex"), ("claude", "claude")]);
        assert_eq!(prefs.registered_launch_profiles_for_issue(42), before);
    }

    #[test]
    fn the_list_order_is_the_priority_and_an_unlisted_tool_is_not_used() {
        let mut prefs = IssueMonitorPrefs::default();
        prefs.set_launch_profile_pool(vec![candidate("codex", None), candidate("claude", None)]);

        prefs.execution_environments = listed(&[("claude", "claude"), ("codex", "codex")]);
        assert_eq!(pick(&prefs, 42).as_deref(), Some("claude"));

        prefs.execution_environments = listed(&[("codex", "codex")]);
        assert_eq!(
            tools(&prefs.registered_launch_profiles_for_issue(42)),
            ["codex"]
        );
    }

    #[test]
    fn an_environment_that_cannot_be_used_is_skipped_for_the_next_one() {
        let mut prefs = IssueMonitorPrefs::default();
        prefs.set_launch_profile_pool(vec![candidate("claude", None)]);
        // Grok Build has no saved candidate; the list alone makes it usable.
        prefs.execution_environments =
            listed(&[("claude", "claude"), ("codex", "codex"), ("grok", "grok")]);
        assert_eq!(pick(&prefs, 42).as_deref(), Some("claude"));

        prefs
            .provider_quota_holds
            .insert("claude".into(), LATER.into());
        assert_eq!(pick(&prefs, 42).as_deref(), Some("codex"));

        prefs
            .provider_quota_holds
            .insert("codex".into(), LATER.into());
        assert_eq!(pick(&prefs, 42).as_deref(), Some("grok"));
        let pool = prefs.registered_launch_profiles_for_issue(42);
        assert_eq!(pool[0].model.as_deref(), Some("claude-model"));
        assert_eq!(pool[2], environment_candidate("grok"));

        prefs
            .provider_quota_holds
            .insert("grok".into(), LATER.into());
        assert_eq!(pick(&prefs, 42), None, "every environment is held");
    }

    /// Issue #7647 AC-3: the tool comes from the project's list, not from
    /// the agent.
    #[test]
    fn registered_agents_start_with_the_environment_in_the_list_order() {
        let mut prefs = roster_prefs();
        // Never saved: each agent keeps the tool it was registered with.
        assert_eq!(prefs.current_launch_tools(), ["claude"]);
        assert_eq!(
            prefs.registered_launch_profiles_for_issue(42),
            [candidate("claude", Some("m-claude"))]
        );
        prefs
            .provider_quota_holds
            .insert("claude".into(), LATER.into());
        assert_eq!(pick(&prefs, 42), None);
        prefs.provider_quota_holds.clear();

        // Saved: every agent starts with the first environment.
        prefs.execution_environments = listed(&[("claude", "claude"), ("codex", "codex")]);
        let members = |pool: &[IssueMonitorLaunchProfile]| -> Vec<String> {
            pool.iter()
                .map(|profile| profile.agent_member_id.clone().unwrap())
                .collect()
        };
        let pool = prefs.registered_launch_profiles_for_issue(42);
        assert_eq!(tools(&pool), ["claude", "claude"]);
        assert_eq!(members(&pool), ["m-claude", "m-codex"]);
        assert_eq!(pool[0], candidate("claude", Some("m-claude")));
        // Tuning saved for another tool is not carried over.
        assert_eq!(
            pool[1],
            IssueMonitorLaunchProfile {
                agent_member_id: Some("m-codex".into()),
                ..environment_candidate("claude")
            }
        );
        assert_eq!(prefs.preview_launch_profile_pool(), pool);
        // The saved pool keeps each agent's registered tool.
        assert_eq!(tools(&prefs.launch_profile_pool()), ["claude", "codex"]);

        // A held environment is passed for the next one.
        prefs
            .provider_quota_holds
            .insert("claude".into(), LATER.into());
        let pool = prefs.registered_launch_profiles_for_issue(42);
        assert_eq!(tools(&pool), ["codex", "codex"]);
        assert_eq!(members(&pool), ["m-claude", "m-codex"]);
        assert_eq!(pick(&prefs, 42).as_deref(), Some("codex"));

        // Every environment held: the first one is kept so the wait names it.
        prefs
            .provider_quota_holds
            .insert("codex".into(), LATER.into());
        assert_eq!(
            tools(&prefs.registered_launch_profiles_for_issue(42)),
            ["claude", "claude"]
        );
        assert_eq!(pick(&prefs, 42), None);
    }

    #[test]
    fn an_environment_no_agent_was_registered_with_is_used() {
        let mut prefs = roster_prefs();
        prefs.execution_environments = listed(&[("grok", "grok"), ("codex", "codex")]);
        assert_eq!(pick(&prefs, 42).as_deref(), Some("grok"));
        assert!(prefs.environment_lists_tool("grok"));
        assert!(!prefs.environment_lists_tool("claude"));
        assert!(!IssueMonitorPrefs::default().environment_lists_tool("codex"));
    }

    #[test]
    fn an_issue_planned_for_an_agent_starts_it_with_the_environment() {
        let mut prefs = roster_prefs();
        prefs.pending_launch_deliveries.push(
            serde_json::from_value(serde_json::json!({
                "agent_member_id": "m-codex", "delivery_id": "delivery-41", "issue_number": 41,
                "branch_name": "work/issue-41",
                "linked_issue_kind": crate::LinkedIssueKind::Issue,
                "claim_id": "claim-41", "created_at": NOW,
            }))
            .unwrap(),
        );
        assert_eq!(pick(&prefs, 41).as_deref(), Some("codex"));

        prefs.execution_environments = listed(&[("claude", "claude")]);
        let pool = prefs.registered_launch_profiles_for_issue(41);
        assert_eq!(tools(&pool), ["claude"]);
        assert_eq!(pool[0].agent_member_id.as_deref(), Some("m-codex"));
    }

    #[test]
    fn registered_agents_tiers_are_those_of_the_environment() {
        let mut prefs = roster_prefs();
        prefs.launch_auto = true;
        let tier_tools = |prefs: &IssueMonitorPrefs| -> Vec<(String, Option<String>)> {
            prefs.effective_launch_tiers()[0]
                .iter()
                .map(|profile| (profile.agent_id.clone(), profile.agent_member_id.clone()))
                .collect()
        };
        assert_eq!(
            tier_tools(&prefs),
            [("claude".to_string(), Some("m-claude".to_string()))]
        );
        prefs.execution_environments = listed(&[("codex", "codex")]);
        assert_eq!(
            tier_tools(&prefs),
            [
                ("codex".to_string(), Some("m-claude".to_string())),
                ("codex".to_string(), Some("m-codex".to_string())),
            ]
        );
    }

    #[test]
    fn automatic_tiers_try_environments_in_the_list_order() {
        let mut prefs = IssueMonitorPrefs {
            launch_auto: true,
            ..IssueMonitorPrefs::default()
        };
        let choose = |prefs: &IssueMonitorPrefs| {
            let holds = prefs.launch_admission_provider_quota_holds();
            prefs
                .select_auto_launch_profile(42, false, None, |pool| {
                    select_launch_profile(pool, &holds, &[], None, NOW)
                })
                .map(|selection| selection.profile.agent_id)
        };
        assert_eq!(prefs.current_launch_tools(), ["codex", "claude"]);
        assert_eq!(choose(&prefs).as_deref(), Some("codex"));

        prefs.execution_environments = listed(&[("claude", "claude"), ("codex", "codex")]);
        assert_eq!(choose(&prefs).as_deref(), Some("claude"));
        prefs
            .provider_quota_holds
            .insert("claude".into(), LATER.into());
        assert_eq!(choose(&prefs).as_deref(), Some("codex"));

        prefs.provider_quota_holds.clear();
        prefs.execution_environments = listed(&[("codex", "codex")]);
        prefs
            .provider_quota_holds
            .insert("codex".into(), LATER.into());
        assert_eq!(choose(&prefs), None, "claude is not in the list");
    }

    #[test]
    fn the_saved_list_is_unique_per_tool_and_survives_a_round_trip() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs::default(),
        );
        assert!(monitor.set_execution_environments(listed(&[
            ("claude", "Claude Code"),
            ("claude:hermes", "hermes"),
            ("other", "claude"),
            ("", "codex"),
        ])));
        let prefs = monitor.prefs();
        assert_eq!(
            prefs.execution_environments,
            listed(&[("claude", "claude"), ("claude:hermes", "hermes")])
        );
        assert!(!monitor.set_execution_environments(prefs.execution_environments.clone()));

        let saved = serde_json::to_string(&prefs).unwrap();
        let restored: IssueMonitorPrefs = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            restored.execution_environments,
            prefs.execution_environments
        );
        // Settings saved before the list existed read as "never saved".
        let legacy = serde_json::to_string(&IssueMonitorPrefs::default()).unwrap();
        assert!(!legacy.contains("execution_environments"));
        let legacy: IssueMonitorPrefs = serde_json::from_str(&legacy).unwrap();
        assert!(legacy.execution_environments.is_empty());
    }
}
