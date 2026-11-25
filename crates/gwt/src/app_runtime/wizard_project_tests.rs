#[test]
fn project_wizards_keep_independent_open_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root_a = temp.path().join("a");
    let root_b = temp.path().join("b");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    let mut runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab("tab-a", "A", root_a.clone(), ProjectKind::Git, &[]),
            sample_project_tab("tab-b", "B", root_b.clone(), ProjectKind::Git, &[]),
        ],
        Some("tab-a"),
    );
    runtime
        .open_launch_wizard_for_branch("tab-a", &root_a, "feature/a", None, None)
        .unwrap();
    let context_a = runtime.project_context("tab-a").unwrap();
    let first = runtime.launch_wizard_state_outbound(&context_a);
    let wizard_a_id = runtime
        .launch_wizard_for(&context_a)
        .unwrap()
        .wizard_id
        .clone();
    runtime
        .open_launch_wizard_for_branch("tab-b", &root_b, "feature/b", None, None)
        .unwrap();
    let context_b = runtime.project_context("tab-b").unwrap();
    let wizard_b_id = runtime
        .launch_wizard_for(&context_b)
        .unwrap()
        .wizard_id
        .clone();
    let restored = runtime.launch_wizard_state_outbound(&context_a);
    match (first.event, restored.event) {
        (
            BackendEvent::LaunchWizardState {
                wizard: Some(first),
            },
            BackendEvent::LaunchWizardState {
                wizard: Some(restored),
            },
        ) => {
            assert_eq!(first.branch_name, restored.branch_name);
        }
        _ => panic!("project A must retain its own wizard while B is opened"),
    }
    runtime.handle_launch_wizard_action_for_client(
        &context_a,
        None,
        gwt::LaunchWizardAction::Cancel,
    );
    assert!(runtime.launch_wizard_for(&context_a).is_none());
    assert_eq!(
        runtime.launch_wizard_for(&context_b).unwrap().wizard_id,
        wizard_b_id
    );
    assert!(runtime
        .handle_launch_wizard_runtime_resolved(wizard_a_id, Err("late A response".into()))
        .is_empty());
    assert!(runtime
        .launch_wizard_for(&context_b)
        .unwrap()
        .wizard
        .hydration_error
        .is_none());
}

/// Issue #7771: a new launch on an Issue offers the project's registered
/// agents with their traits, and reserves the one chosen in its first stage.
#[test]
fn an_issue_launch_offers_the_roster_with_traits_and_starts_as_the_chosen_agent() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[])],
        Some("tab-1"),
    );
    let sessions = runtime.sessions_dir.clone();
    // Reading the roster registers the project's first agent.
    gwt::agent_names::read_project_roster(&repo, &sessions).unwrap();
    let members = gwt::agent_names::mutate_project_roster(
        &repo,
        &sessions,
        gwt::agent_names::AgentRosterAction::Add {
            model: None,
            reasoning: None,
            pm_eligible: false,
            name: None,
            agent_id: Some("claude".into()),
        },
        None,
    )
    .unwrap()
    .members;
    assert_eq!(members.len(), 2);
    // The fixture machine has Codex only.
    configure_owner_fixture_tools(&repo, &["claude", "codex"]);
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        |prefs| {
            let roster = prefs.agent_roster.as_mut().expect("fixture roster");
            roster.members[1].traits = gwt::agent_names::AgentTraits {
                reasoning_level: Some(gwt::agent_names::TraitReasoningLevel::High),
                speed: Some(gwt::agent_names::TraitSpeed::Fast),
                personality: Some("Implementation first.".into()),
            };
            Ok(())
        },
    )
    .unwrap();
    let context = runtime.project_context("tab-1").unwrap();
    let view = |runtime: &AppRuntime| match runtime.launch_wizard_state_outbound(&context).event {
        BackendEvent::LaunchWizardState { wizard: Some(view) } => view,
        _ => panic!("the wizard is open"),
    };

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", None, None)
        .unwrap();
    assert!(
        view(&runtime).issue_plan.is_none(),
        "a launch without an Issue keeps the settings form"
    );

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", Some(42), None)
        .unwrap();
    let plan = view(&runtime).issue_plan.expect("an Issue launch is planned");
    assert_eq!(plan.issue_number, 42);
    assert_eq!(plan.stage, None, "the start method is chosen first");
    assert_eq!(
        plan.assignees
            .iter()
            .map(|agent| (agent.name.as_str(), agent.available, agent.selected))
            .collect::<Vec<_>>(),
        vec![
            (members[0].name.as_str(), false, false),
            (members[1].name.as_str(), true, true),
        ]
    );
    assert_eq!(
        plan.assignees[0].status,
        "Claude Code is not installed on this machine."
    );
    assert_eq!(plan.assignees[1].reasoning, "High");
    assert_eq!(plan.assignees[1].speed, "Fast");
    assert_eq!(
        plan.assignees[1].personality.as_deref(),
        Some("Implementation first.")
    );

    runtime.handle_launch_wizard_action(
        &context,
        gwt::LaunchWizardAction::UseStartMethod {
            method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
        },
    );
    let planned = view(&runtime);
    assert_eq!(
        planned.issue_plan.and_then(|plan| plan.stage).as_deref(),
        Some("assignee")
    );
    // The agent's traits decide the tool's tuning (Issue #7647).
    assert_eq!(planned.selected_agent_id, "codex");
    assert_eq!(planned.selected_reasoning, "high");
    assert_eq!(planned.codex_speed, gwt_agent::CodexSpeed::Fast);
    let assignee = |runtime: &AppRuntime| {
        runtime
            .launch_wizard_for(&context)
            .unwrap()
            .wizard
            .issue_plan_assignee_id()
            .map(str::to_string)
    };
    assert_eq!(assignee(&runtime), Some(members[1].agent_member_id.clone()));

    // An agent that cannot take the launch is refused with its reason.
    runtime.handle_launch_wizard_action(
        &context,
        gwt::LaunchWizardAction::SetAgentMember {
            agent_member_id: members[0].agent_member_id.clone(),
        },
    );
    assert_eq!(
        view(&runtime).error.as_deref(),
        Some("Claude Code is not installed on this machine.")
    );
    assert_eq!(assignee(&runtime), Some(members[1].agent_member_id.clone()));

    // Stage 1 → 2 finds where the session can run, then 2 → 3 confirms.
    let AppEventProxy::Stub(recorded) = &runtime.proxy else {
        panic!("the fixture records its events");
    };
    let recorded = recorded.clone();
    resolve_launch_wizard_runtime_confirmation(&mut runtime, &recorded, "issue plan runtime");
    let planned = view(&runtime);
    assert_eq!(
        planned.issue_plan.and_then(|plan| plan.stage).as_deref(),
        Some("plan")
    );
    assert_eq!(planned.primary_action_label, "Continue to confirm");
    runtime.handle_launch_wizard_action(&context, gwt::LaunchWizardAction::Submit);
    let confirm = view(&runtime);
    assert!(confirm.show_confirm);
    assert_eq!(confirm.primary_action_label, "Start");
    assert_eq!(
        confirm.launch_summary[..2]
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["Issue", "Assignee"]
    );
    assert_eq!(assignee(&runtime), Some(members[1].agent_member_id.clone()));
}
