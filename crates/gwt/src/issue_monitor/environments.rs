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

    /// The saved pool as a preview shows it: limited to the project's
    /// environments when no registered agent heads it.
    pub fn preview_launch_profile_pool(&self) -> Vec<IssueMonitorLaunchProfile> {
        let pool = self.launch_profile_pool();
        if pool
            .first()
            .is_some_and(|profile| profile.agent_member_id.is_some())
        {
            return pool;
        }
        self.environment_candidates(pool)
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

    /// The tool an Issue no registered agent is planned for starts with: the
    /// first environment, in priority order, that some registered agent can
    /// run and that is not held by a refused launch. When every such
    /// environment is held, the first one is returned so the wait names it.
    pub(super) fn environment_tool_for_unplanned_issue(
        &self,
        tools: &[String],
        mut has_agent: impl FnMut(&str) -> bool,
    ) -> Option<String> {
        let now = gwt_core::timestamp::UtcTimestamp::now();
        let mut held = None;
        for tool in tools {
            if !has_agent(tool) {
                continue;
            }
            if hold_reset_after(&self.provider_quota_holds, tool, now).is_none() {
                return Some(tool.clone());
            }
            held.get_or_insert_with(|| tool.clone());
        }
        held
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

    #[test]
    fn registered_agents_follow_the_list_when_their_tool_is_held() {
        let mut prefs = roster_prefs();
        // Never saved: the agent at the head fixes the tool, as before.
        assert_eq!(prefs.current_launch_tools(), ["claude"]);
        prefs
            .provider_quota_holds
            .insert("claude".into(), LATER.into());
        assert_eq!(pick(&prefs, 42), None);
        prefs.provider_quota_holds.clear();

        prefs.execution_environments = listed(&[("claude", "claude"), ("codex", "codex")]);
        assert_eq!(pick(&prefs, 42).as_deref(), Some("claude"));
        prefs
            .provider_quota_holds
            .insert("claude".into(), LATER.into());
        let pool = prefs.registered_launch_profiles_for_issue(42);
        assert_eq!(tools(&pool), ["codex"]);
        assert_eq!(pool[0].agent_member_id.as_deref(), Some("m-codex"));

        // Every environment held: the first one is kept so the wait names it.
        prefs
            .provider_quota_holds
            .insert("codex".into(), LATER.into());
        assert_eq!(
            tools(&prefs.registered_launch_profiles_for_issue(42)),
            ["claude"]
        );
        assert_eq!(pick(&prefs, 42), None);
    }

    #[test]
    fn a_listed_environment_no_registered_agent_can_run_is_passed_over() {
        let mut prefs = roster_prefs();
        prefs.execution_environments = listed(&[("grok", "grok"), ("codex", "codex")]);
        assert_eq!(pick(&prefs, 42).as_deref(), Some("codex"));

        prefs.execution_environments = listed(&[("grok", "grok")]);
        assert!(prefs.registered_launch_profiles_for_issue(42).is_empty());
    }

    #[test]
    fn an_issue_planned_for_an_agent_waits_when_its_tool_is_not_listed() {
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

        prefs.execution_environments = listed(&[("claude", "claude"), ("codex", "codex")]);
        assert_eq!(
            pick(&prefs, 41).as_deref(),
            Some("codex"),
            "the planned agent keeps its own tool"
        );
        prefs.execution_environments = listed(&[("claude", "claude")]);
        assert!(prefs.registered_launch_profiles_for_issue(41).is_empty());
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
