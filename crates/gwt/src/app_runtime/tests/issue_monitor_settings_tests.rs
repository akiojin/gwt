use super::*;

#[test]
fn app_runtime_issue_monitor_configure_profile_shows_the_saved_head_while_it_is_held() {
    // Issue #4366 AC-6: a hold on the saved head's provider must not change
    // what Agent Settings shows. The form used to open on the launch choice,
    // which skips held providers — so it read as the fallback agent, and
    // saving it unchanged replaced the operator's head with that fallback.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    // The saved head carries a non-default reasoning level, so the profile the
    // form is filled from shows in what an unchanged save would write.
    let mut saved_head = pool_profile("codex");
    saved_head.model = Some("gpt-6-astra".to_string());
    saved_head.reasoning = Some("high".to_string());
    seeded.set_launch_profile_pool(vec![saved_head, pool_profile("claude")]);
    seeded
        .provider_quota_holds
        .insert("codex".to_string(), "2099-01-01T00:00:00Z".to_string());
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed held pool");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureProfile,
    );
    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    let pool = view
        .issue_monitor_pool
        .as_ref()
        .expect("Agent Settings lists the saved candidates as its sets");
    assert_eq!(pool.active_index, 0);
    assert_eq!(
        view.selected_agent_id, "codex",
        "the form opens on the saved head, not on the held fallback: {pool:?}"
    );
    assert_eq!(
        pool.sets
            .iter()
            .map(|set| set.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex", "claude"],
        "saving the form unchanged must not switch the saved head to the fallback: {pool:?}"
    );
    // The agent picker follows the installed agents in this environment, so
    // the profile the form was filled from shows in what the save would
    // write: an unchanged save must leave the saved pool exactly as it is.
    let saved_summary =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), seeded)
            .status_view()
            .launch_profile_summary;
    assert_eq!(
        pool.resulting_summary, saved_summary,
        "the form must open on the saved head's own model and reasoning: {pool:?}"
    );
}

#[test]
fn app_runtime_issue_monitor_configure_profile_saves_global_profile_without_launching() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let autonomous_tuning = gwt::issue_monitor::AutonomousTuning {
        max_attempts: 9,
        review_model: Some("gpt-5.5-review".to_string()),
        ..gwt::issue_monitor::AutonomousTuning::default()
    };
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            autonomous_tuning: autonomous_tuning.clone(),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed issue monitor prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureProfile,
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings opened" && issue_number.is_none()
        )
    }));
    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    assert_eq!(view.title, "Configure Issue Monitor");
    assert_eq!(view.linked_issue_number, None);
    assert_eq!(view.primary_action_label, "Continue");
    let save_context = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .issue_monitor_profile_save
        .as_ref()
        .expect("profile save context");
    assert_eq!(save_context.issue_number, None);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .initial_prompt,
        ""
    );

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetModel {
            model: "gpt-5.5".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);
    wait_for_recorded_event(
        "global issue monitor settings runtime resolution",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    let _confirm_events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    let saved_events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );

    assert!(saved_events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings saved" && issue_number.is_none()
        )
    }));
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .is_none());
    assert!(
        runtime.window_details.is_empty(),
        "saving global Issue Monitor settings must not spawn an agent window"
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    let profile = prefs.launch_profile.expect("saved launch profile");
    assert_eq!(profile.agent_id, "codex");
    assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(profile.reasoning.as_deref(), Some("high"));
    assert_eq!(
        prefs.autonomous_tuning, autonomous_tuning,
        "Agent settings save must not rewrite autonomous tuning fields"
    );
}

#[test]
fn app_runtime_agent_settings_update_preserves_the_saved_profile_and_wizard() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let profile = codex_issue_monitor_launch_profile();
    let (mut runtime, recorded, opened) =
        open_agent_settings_sets(temp.path(), &repo, vec![profile]);
    let before = agent_settings_view(&opened);
    assert_eq!(before.selected_launch_target, "agent");
    assert!(before.error.is_none());
    let model = before.selected_model.clone();
    let reasoning = before.selected_reasoning.clone();
    // The update must never run the host's real package manager in a test.
    let bin = write_fixture_runners(temp.path(), &["npm", "codex"]);
    let mut settings = Settings::default();
    pin_launch_agents(&mut settings, &bin);
    let _path = ScopedEnvVar::set(
        "PATH",
        &settings.profiles.get("default").unwrap().env_vars["PATH"],
    );
    write_profile_config(runtime.profile_config_path.as_ref().unwrap(), &settings);
    let context = runtime.test_context();
    let wizard_id = runtime
        .launch_wizard_for(&context)
        .unwrap()
        .wizard_id
        .clone();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs_before = fs::read(&prefs_path).unwrap();

    let events =
        runtime.handle_launch_wizard_action(&context, LaunchWizardAction::RunAgentSetup, None);
    let updated = agent_settings_view(&events);
    assert!(updated.error.is_none(), "{:?}", updated.error);
    assert_eq!(updated.selected_launch_target, "agent");
    assert_eq!(updated.selected_model, model);
    assert_eq!(updated.selected_reasoning, reasoning);
    assert_eq!(
        runtime.launch_wizard_for(&context).unwrap().wizard_id,
        wizard_id
    );
    assert_eq!(fs::read(&prefs_path).unwrap(), prefs_before);
    assert!(runtime.runtimes.is_empty());
    assert!(updated.agent_setup.as_ref().unwrap().pending);
    wait_for_recorded_event("CLI update result", &recorded, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardAgentUpdated { .. }
            )
        })
    });
    let (result_id, result) = recorded
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match recorded_project_payload(event) {
            UserEvent::LaunchWizardAgentUpdated { wizard_id, result } => {
                Some((wizard_id.clone(), result.clone()))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(result_id, wizard_id);
    let completed = runtime.handle_launch_wizard_agent_updated(result_id, *result);
    let view = agent_settings_view(&completed);
    assert!(!view.agent_setup.as_ref().unwrap().pending);
    assert!(view
        .agent_setup
        .as_ref()
        .unwrap()
        .status
        .as_ref()
        .unwrap()
        .contains("1.2.3"));
    assert_eq!(view.selected_model, model);
    assert_eq!(view.selected_reasoning, reasoning);
    assert_eq!(
        runtime.launch_wizard_for(&context).unwrap().wizard_id,
        wizard_id
    );
    assert_eq!(fs::read(&prefs_path).unwrap(), prefs_before);
    assert_eq!(
        runtime
            .launch_wizard_cache
            .agent_options()
            .into_iter()
            .find(|agent| agent.id == "codex")
            .unwrap()
            .installed_version
            .as_deref(),
        Some("1.2.3")
    );
}

#[test]
fn app_runtime_issue_monitor_agent_settings_preserves_an_undetected_saved_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut codex = pool_profile("codex");
    codex.version = Some("0.121.0".to_string());
    codex.prefer_for = vec!["type:fix".to_string()];
    let (mut runtime, _, _) = open_agent_settings_sets(temp.path(), &repo, vec![codex.clone()]);
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.path().join("sessions"),
        vec![agent_settings_option("claude", "Claude Code")],
    );
    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureProfile,
    );
    assert_eq!(
        agent_settings_set_agents(agent_settings_view(&events)),
        ["codex"]
    );
    assert_eq!(
        runtime
            .launch_wizard_for(&runtime.test_context())
            .unwrap()
            .wizard
            .agent_id,
        "codex"
    );
    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    assert!(agent_settings_view(&events).error.is_some());
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("read unchanged pool");
    assert_eq!(prefs.launch_profile_pool(), [codex]);
}

#[test]
fn app_runtime_issue_monitor_agent_settings_sets_are_added_reordered_and_saved_in_order() {
    // Issue #4911 AC-1/AC-3/AC-5/AC-7: `＋` adds a set edited through the same
    // form, a set can be moved to the front, an agent cannot appear twice, and
    // the save writes the sets in the order `issue.monitor.profiles` reports.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut codex = pool_profile("codex");
    codex.prefer_for = vec!["type:fix".to_string()];
    let mut claude = pool_profile("claude");
    claude.skip_permissions = true;
    let (mut runtime, recorded_events, events) =
        open_agent_settings_sets(temp.path(), &repo, vec![codex, claude.clone()]);

    let view = agent_settings_view(&events);
    assert_eq!(agent_settings_set_agents(view), vec!["codex", "claude"]);
    let pool = view.issue_monitor_pool.as_ref().expect("pool view");
    assert_eq!(pool.active_index, 0);
    assert!(pool.remove_disabled_reason.is_none());
    assert!(pool.add_disabled_reason.is_none());

    // An edit to the open set survives opening another one.
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        },
        None,
    );
    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::AddAgentSettingsSet,
        None,
    );
    let view = agent_settings_view(&events);
    assert_eq!(
        agent_settings_set_agents(view),
        vec!["codex", "claude", "grok"],
        "the new set takes the first agent no other set uses"
    );
    let pool = view.issue_monitor_pool.as_ref().expect("pool view");
    assert_eq!(pool.active_index, 2, "the new set opens for editing");
    assert_eq!(view.selected_agent_id, "grok");
    assert!(
        pool.add_disabled_reason.is_some(),
        "every offered agent now has a set: {pool:?}"
    );

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        },
        None,
    );
    let view = agent_settings_view(&events);
    assert!(
        view.error
            .as_deref()
            .is_some_and(|error| error.contains("Agent Settings 1")),
        "a second set for the same agent is refused and names the set that has it: {:?}",
        view.error
    );
    assert_eq!(view.selected_agent_id, "grok");

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::MoveAgentSettingsSet { index: 2, to: 0 },
        None,
    );
    let view = agent_settings_view(&events);
    assert_eq!(
        agent_settings_set_agents(view),
        vec!["grok", "codex", "claude"]
    );
    let pool = view.issue_monitor_pool.as_ref().expect("pool view");
    assert_eq!(pool.active_index, 0, "the open set moves with its form");
    let previewed_summary = pool.resulting_summary.clone();

    let saved_events = {
        // Verify settings and order independently of host fsync latency.
        let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
        save_agent_settings_sets(&mut runtime, &recorded_events)
    };
    assert!(
        saved_events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, .. } if message == "Issue Monitor settings saved"
        )),
        "Agent Settings save failed: {saved_events:#?}"
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load prefs");
    let saved = prefs.launch_profile_pool();
    assert_eq!(
        saved
            .iter()
            .map(|profile| profile.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["grok", "codex", "claude"],
        "the set order is the launch candidate order"
    );
    assert_eq!(
        prefs.launch_profile.as_ref().map(|p| p.agent_id.as_str()),
        Some("grok"),
        "the first set is what the next launch uses"
    );
    assert_eq!(saved[1].reasoning.as_deref(), Some("high"));
    assert_eq!(
        saved[1].prefer_for,
        vec!["type:fix".to_string()],
        "routing tags stay with the set whose agent did not change"
    );
    assert_eq!(saved[2], claude, "a set that was never opened is untouched");
    let status =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs).status_view();
    assert_eq!(
        status.launch_profile_summary, previewed_summary,
        "the form previews the summary the Monitor reports after the save"
    );
    assert_eq!(
        status
            .launch_profile_candidates
            .iter()
            .map(|candidate| candidate.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["grok", "codex", "claude"]
    );
}

#[test]
fn app_runtime_issue_monitor_agent_settings_keeps_a_runtime_the_form_was_not_asked_about() {
    // Issue #4911 AC-5: the form only asks where a set runs in its Runtime
    // step and reads Host before that, so leaving a set for another one must
    // not turn a saved Docker candidate into a Host one.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut codex = pool_profile("codex");
    codex.version = Some("0.121.0".to_string());
    codex.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    codex.docker_service = Some("gwt".to_string());
    codex.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
    let (mut runtime, recorded_events, _events) = open_agent_settings_sets(
        temp.path(),
        &repo,
        vec![codex.clone(), pool_profile("claude")],
    );

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SelectAgentSettingsSet { index: 1 },
        None,
    );
    let view = agent_settings_view(&events);
    assert_eq!(view.selected_agent_id, "claude");
    let pool = view.issue_monitor_pool.as_ref().expect("pool view");
    assert!(
        pool.sets[0]
            .summary
            .iter()
            .any(|row| row.label == "Runtime" && row.value == "docker:gwt"),
        "the set that was left still runs where it was saved to run: {pool:?}"
    );
    assert!(
        pool.sets[0]
            .summary
            .iter()
            .any(|row| row.label == "Version" && row.value == "0.159.2"),
        "closed sets must show the detected version, not the ignored saved pin: {pool:?}"
    );

    save_agent_settings_sets(&mut runtime, &recorded_events);

    let saved = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load prefs")
        .launch_profile_pool();
    assert_eq!(
        saved[0], codex,
        "opening a set and leaving it unedited changes nothing about it"
    );
}

#[test]
fn app_runtime_issue_monitor_agent_settings_runtime_step_starts_from_the_open_set() {
    // Issue #4911 AC-5: the Runtime step proposes where the open set runs. It
    // used to propose the repository's default — Docker wherever a Compose
    // service exists — so saving a Host set unedited rewrote it to Docker.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let (mut runtime, recorded_events, _events) =
        open_agent_settings_sets(temp.path(), &repo, vec![pool_profile("codex")]);

    save_agent_settings_sets_in(&mut runtime, &recorded_events, |hydration| {
        hydration.docker_context = Some(gwt::DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
    });

    let saved = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load prefs")
        .launch_profile_pool();
    assert_eq!(saved[0].agent_id, "codex");
    assert_eq!(
        saved[0].runtime_target,
        gwt_agent::LaunchRuntimeTarget::Host,
        "a set saved to run on the host still does after an unedited save"
    );
    assert_eq!(saved[0].docker_service, None);
}

#[test]
fn app_runtime_issue_monitor_agent_settings_keeps_at_least_one_set() {
    // Issue #4911 AC-2/AC-6: `−` removes a set, but the last one stays and the
    // form says why.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let (mut runtime, _recorded_events, _events) = open_agent_settings_sets(
        temp.path(),
        &repo,
        vec![pool_profile("codex"), pool_profile("claude")],
    );

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::RemoveAgentSettingsSet { index: 0 },
        None,
    );
    let view = agent_settings_view(&events);
    assert_eq!(agent_settings_set_agents(view), vec!["claude"]);
    assert_eq!(
        view.selected_agent_id, "claude",
        "removing the open set opens the one that took its place"
    );
    let reason = view
        .issue_monitor_pool
        .as_ref()
        .and_then(|pool| pool.remove_disabled_reason.clone())
        .expect("the last set says why it cannot be removed");

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::RemoveAgentSettingsSet { index: 0 },
        None,
    );
    let view = agent_settings_view(&events);
    assert_eq!(agent_settings_set_agents(view), vec!["claude"]);
    assert_eq!(view.error.as_deref(), Some(reason.as_str()));
}

#[test]
fn app_runtime_issue_monitor_agent_settings_save_refuses_a_pool_changed_elsewhere() {
    // Issue #4911 AC-8: the form saves the whole pool, so a pool another
    // window or `issue.monitor.profiles.set` changed meanwhile must not be
    // overwritten silently.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let (mut runtime, recorded_events, _events) =
        open_agent_settings_sets(temp.path(), &repo, vec![pool_profile("codex")]);

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut elsewhere = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
    elsewhere.set_launch_profile_pool(vec![pool_profile("claude"), pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &elsewhere).expect("concurrent profiles.set");

    let events = save_agent_settings_sets(&mut runtime, &recorded_events);

    assert!(!events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { message, .. } if message == "Issue Monitor settings saved"
    )));
    let view = agent_settings_view(&events);
    assert!(
        view.error
            .as_deref()
            .is_some_and(|error| error.contains("changed") && error.contains("Nothing was saved")),
        "the refusal says what happened: {:?}",
        view.error
    );
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        prefs
            .launch_profile_pool()
            .iter()
            .map(|profile| profile.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["claude", "codex"],
        "the concurrent write survives"
    );
}

#[test]
fn app_runtime_issue_monitor_profile_save_reports_authority_epoch_overflow() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            effect_authority_epoch: u64::MAX,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed max epoch");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = sample_ready_agent_launch_wizard_session("tab-1", &repo);
    let request = gwt::LaunchWizardLaunchRequest::Agent(Box::new(
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("develop")
            .build(),
    ));

    let events = runtime.save_issue_monitor_profile_from_launch_request(
        session,
        IssueMonitorProfileSaveContext {
            client_id: "client-1".to_string(),
            issue_number: None,
            pool: Vec::new(),
            sets: None,
        },
        request,
    );

    assert!(!events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { message, .. }
            if message == "Issue Monitor settings saved"
    )));
    let wizard = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("overflow keeps wizard open");
    assert_eq!(
        wizard.error.as_deref(),
        Some("Failed to save Issue Monitor settings: authority epoch overflow")
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(persisted.effect_authority_epoch, u64::MAX);
    assert!(persisted.launch_profile.is_none());
}

#[test]
fn app_runtime_issue_monitor_start_without_saved_profile_opens_global_settings_before_enable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("high".to_string());
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings opened" && issue_number.is_none()
        )
    }));
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.issue_monitor_profile_save.as_ref())
        .is_some_and(|context| context.issue_number.is_none()));
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("status resets optimistic enabled UI");
    assert!(!status.enabled);
    assert!(
        runtime.window_details.is_empty(),
        "missing saved profile must not spawn an agent window"
    );
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .unwrap_or_default();
    assert!(
        !prefs.enabled,
        "Start with only last settings must not publish or persist enabled state"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_prefers_saved_profile() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        max_active_agents: 1,
        launch_profile: Some(sample_issue_monitor_launch_profile()),
        ..Default::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("save issue monitor prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let mut agent_options = sample_agent_options();
    agent_options.push(gwt::AgentOption {
        id: "claude".to_string(),
        name: "Claude Code".to_string(),
        available: true,
        installed_version: Some("latest".to_string()),
        custom_agent: None,
    });
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.path().join("sessions"),
        agent_options,
    );

    let _events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
    );

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_none(),
        "saved Issue Monitor profile must launch silently"
    );
    let agent_window = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.agent_id.as_deref(),
        Some("claude"),
        "saved profile should override last settings"
    );
    assert!(
        runtime
            .pending_launch_feedback_contexts
            .values()
            .any(|context| context.issue_monitor_issue_number == Some(3165)),
        "saved-profile auto launch errors must be wired back to Issue Monitor"
    );
}

#[test]
fn app_runtime_issue_monitor_does_not_force_a_held_head() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for automatic in [true, false] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo_with_initial_commit(&repo);
        let mut prefs = gwt::IssueMonitorPrefs {
            launch_profile: Some(codex_issue_monitor_launch_profile()),
            provider_quota_holds: std::collections::BTreeMap::from([
                ("codex".to_string(), "2999-01-01T04:00:00Z".to_string()),
                ("claude".to_string(), "2999-01-01T04:00:00Z".to_string()),
            ]),
            ..Default::default()
        };
        prefs.set_launch_profile_pool(vec![
            codex_issue_monitor_launch_profile(),
            claude_issue_monitor_launch_profile(),
        ]);
        prefs.launch_auto = automatic;
        gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
            .expect("save prefs");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let (spawner, queued) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        runtime.auto_launch_issue_monitor_delivery_events_for_project(
            &repo,
            4774,
            LinkedIssueKind::Issue,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
        drain_queued_blocking_tasks(&queued);
        runtime.handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &recorded_events,
        ));
        assert!(
            runtime.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .all(|window| window.preset != WindowPreset::Agent),
            "automatic={automatic}: every held pool must refuse"
        );
    }
}

#[test]
fn app_runtime_issue_monitor_auto_launch_skips_a_held_candidate_and_reports_why() {
    // SPEC #3914 AC-4 / US-1: pool [codex, claude] with codex held launches
    // claude and surfaces the skip reason as a toast.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let mut prefs = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        max_active_agents: 1,
        provider_quota_holds: std::collections::BTreeMap::from([(
            "codex".to_string(),
            "2999-01-01T04:00:00Z".to_string(),
        )]),
        ..Default::default()
    };
    prefs.set_launch_profile_pool(vec![
        codex_issue_monitor_launch_profile(),
        claude_issue_monitor_launch_profile(),
    ]);
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("save issue monitor prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let mut agent_options = sample_agent_options();
    agent_options.push(gwt::AgentOption {
        id: "claude".to_string(),
        name: "Claude Code".to_string(),
        available: true,
        installed_version: Some("latest".to_string()),
        custom_agent: None,
    });
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.path().join("sessions"),
        agent_options,
    );

    let events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3914,
        LinkedIssueKind::Spec,
    );

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_none(),
        "pool launch must stay silent"
    );
    let agent_window = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.agent_id.as_deref(),
        Some("claude"),
        "the held head candidate yields to the next one"
    );
    let toast = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number: Some(3914),
                ..
            } if message.contains("Held codex") => Some((level.clone(), message.clone())),
            _ => None,
        })
        .expect("skip reason toast");
    assert_eq!(toast.0, "info");
    assert!(toast.1.contains("claude"), "{}", toast.1);
    assert!(toast.1.contains("04:00"), "{}", toast.1);
}

#[test]
fn app_runtime_issue_monitor_auto_tier_change_starts_fresh() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "tier-fresh-session",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load Monitor prefs");
    let mut value = serde_json::to_value(prefs).expect("serialize prefs");
    let profile = codex_issue_monitor_launch_profile();
    value["launch_auto"] = serde_json::json!(true);
    value["launch_tiers"] = serde_json::json!([[profile.clone()], [profile.clone()], [profile]]);
    value["issue_tiers"] = serde_json::json!({"3165": {
        "floor": 2, "launch_tier": 1, "landing_tier": null
    }});
    let prefs = serde_json::from_value(value).expect("tier prefs");
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save tier prefs");

    prepare_monitor_relaunch(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result = take_monitor_launch_complete("fresh tier launch", &fixture.recorded_events);
    assert_monitor_fresh_successor(result, &fixture);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload tier prefs");
    let value = serde_json::to_value(prefs).expect("serialize tier result");
    assert_eq!(value["issue_tiers"]["3165"]["launch_tier"], 2);
}

#[test]
fn app_runtime_issue_monitor_auto_same_tier_override_starts_fresh() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "same-tier-override",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
    prefs.launch_auto = true;
    let mut profile = codex_issue_monitor_launch_profile();
    profile.model = Some("gpt-6-astra".to_string());
    profile.reasoning = Some("high".to_string());
    prefs.launch_tiers = vec![vec![profile.clone()], vec![profile]];
    prefs.record_tier_launch(3165, 1);
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save updated tier");
    prepare_monitor_relaunch(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result = take_monitor_launch_complete("same tier override", &fixture.recorded_events);
    let result = result.expect("launch result");
    assert_eq!(result.9, gwt_agent::SessionMode::Normal);
    let session =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{}.toml", result.1)))
            .expect("fresh session");
    assert_eq!(session.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(session.reasoning_level.as_deref(), Some("high"));
    assert!(session.agent_session_id.is_none());
}

#[test]
fn app_runtime_issue_monitor_auto_answered_handoff_defers_tier_selection() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "auto-answer-exact-session",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    seed_resumed_autonomous_handoff(&fixture, "Keep the original conversation");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
    prefs.launch_auto = true;
    // Switching auto to a different provider must not redirect the answer.
    prefs.launch_tiers = vec![vec![claude_issue_monitor_launch_profile()]; 3];
    prefs.record_tier_launch(3165, 0);
    let history = prefs.issue_tiers.clone();
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save changed auto tiers");
    let events = prepare_monitor_relaunch(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    assert!(
        !events.iter().any(|event| matches!(&event.event,
            BackendEvent::IssueMonitorToast { level, .. } if level == "error"
        )),
        "answer delivery must bypass changed auto candidates"
    );
    let result = take_monitor_launch_complete("auto answer delivery", &fixture.recorded_events);
    assert_monitor_exact_resume(result, &fixture);
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        persisted.issue_tiers, history,
        "answer delivery does not select a new tier"
    );
}

/// SPEC #3914 FR-007 (PR #3968 review): a non-head selection is reported on
/// the exact-Resume path too, not only when a fresh session is spawned.
#[test]
fn app_runtime_issue_monitor_resume_reports_skipped_candidates() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "resume-skip-reason",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    // Pool [claude, codex] with claude held: codex is the non-head selection
    // and owns the stored resumable conversation.
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load Monitor prefs");
    prefs.set_launch_profile_pool(vec![
        claude_issue_monitor_launch_profile(),
        codex_issue_monitor_launch_profile(),
    ]);
    prefs
        .provider_quota_holds
        .insert("claude".to_string(), "2999-01-01T04:00:00Z".to_string());
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save pooled Monitor prefs");

    let events = fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result =
        take_monitor_launch_complete("resume with a skipped head", &fixture.recorded_events);
    assert_monitor_exact_resume(result, &fixture);
    let toast = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number: Some(3165),
                ..
            } if message.contains("Held claude") => Some((level.clone(), message.clone())),
            _ => None,
        })
        .expect("skip reason toast on the resume path");
    assert_eq!(toast.0, "info");
    assert!(toast.1.contains("codex"), "{}", toast.1);
    assert!(toast.1.contains("04:00"), "{}", toast.1);
}

#[test]
fn app_runtime_issue_monitor_profile_save_switches_the_head_to_a_second_provider() {
    // SPEC #3914 FR-003 / US-7, amended by Issue #4079 AC-1: saving another
    // provider from Agent settings is a switch, so it takes candidate 1 and the
    // `launch_profile` mirror. Appending a candidate is a `profiles.set`
    // operation, not something the settings form does behind the operator.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![claude_issue_monitor_launch_profile()]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = sample_ready_agent_launch_wizard_session("tab-1", &repo);
    let request = gwt::LaunchWizardLaunchRequest::Agent(Box::new(
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("develop")
            .build(),
    ));

    let events = runtime.save_issue_monitor_profile_from_launch_request(
        session,
        IssueMonitorProfileSaveContext {
            client_id: "client-1".to_string(),
            issue_number: None,
            pool: Vec::new(),
            sets: None,
        },
        request,
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { message, .. }
            if message == "Issue Monitor settings saved"
    )));
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let pool = persisted.launch_profile_pool();
    assert_eq!(
        pool.iter()
            .map(|profile| profile.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex"],
        "the chosen provider replaces candidate 1"
    );
    assert_eq!(
        persisted
            .launch_profile
            .as_ref()
            .map(|profile| profile.agent_id.as_str()),
        Some("codex"),
        "the compatibility mirror follows the pool head"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_without_previous_settings_opens_wizard() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
    );

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_some(),
        "auto launch without saved or last settings must open one settings window"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .issue_monitor_profile_save
        .is_some());
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "wizard fallback must broadcast LaunchWizardState so the user can configure settings"
    );
    assert!(
        runtime.window_details.is_empty(),
        "wizard fallback must not silently spawn an agent window"
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context()).expect("test project state").launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .initial_prompt,
        "$gwt-execute #3165\n\nThis prompt was generated by Issue Monitor. It is not a statement, approval, or visual confirmation by a human user."
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_keeps_existing_settings_wizard() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let _first_events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
    );
    let first_wizard_id = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("first settings wizard")
        .wizard_id
        .clone();

    let second_events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3166,
        LinkedIssueKind::Spec,
    );

    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .as_ref()
            .expect("existing settings wizard")
            .wizard_id,
        first_wizard_id,
        "additional auto launch requests must not replace the open settings wizard"
    );
    assert!(
        !second_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "additional auto launch requests must not open another settings wizard"
    );
}

#[test]
fn app_runtime_issue_monitor_launch_now_ignores_auto_max_active_setting() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        enabled: true,
        max_active_agents: 1,
        priority_order: Vec::new(),
        launch_profile: None,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("save issue monitor prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorLaunchNow {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor launch prepared" && *issue_number == Some(3165)
        )
    }));
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_some(),
        "manual Issue Monitor launch should not be capped by max_active_agents"
    );
}

/// Issue #3628 AC-3/AC-6: the GUI recovery for a row whose launch is gone.
///
/// Launch Now only opens the wizard, so an operator who wanted the row back in
/// the queue *without* starting an agent had no control at all and fell back to
/// editing `issue-monitor.json` by hand. Seeded in the 2026-08-17 shape: a
/// persisted failure hold with no launch left, beside a live launch that the
/// recovery must not disturb.
#[test]
fn app_runtime_issue_monitor_requeue_releases_a_dead_hold_without_launching() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            max_active_agents: 2,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 3629,
                window_id: "tab-1::agent-live".to_string(),
            }],
            failed_issues: vec![gwt::IssueMonitorFailedIssue {
                issue_number: 3628,
                message: "an execution generation already exists for issue #3628".to_string(),
                window_id: Some("tab-1::agent-dead".to_string()),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorRequeue { issue_number: 3628 },
    );

    assert!(
        !events.is_empty(),
        "missing daemon forces the atomic GUI fallback writer"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_none(),
        "the recovery returns the row to the queue and must not start an agent"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(
        persisted.failed_issues.is_empty(),
        "the persisted hold must be gone: {:?}",
        persisted.failed_issues
    );
    assert_eq!(
        persisted
            .released_failures
            .iter()
            .map(|release| release.issue_number)
            .collect::<Vec<_>>(),
        vec![3628],
        "the release must be published so other processes converge on it"
    );
    assert_eq!(
        persisted.launched_issues.len(),
        1,
        "the unrelated live launch must survive the recovery"
    );

    // Aiming the recovery at the live row is refused, and the refusal changes
    // nothing — killing a running agent is the one mistake no later
    // compensation can undo.
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorRequeue { issue_number: 3629 },
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        persisted.launched_issues.len(),
        1,
        "a refused recovery must leave the live launch bound"
    );
    assert!(
        persisted
            .released_failures
            .iter()
            .all(|release| release.issue_number != 3629),
        "a refused recovery must not publish a release"
    );
}

#[test]
fn app_runtime_issue_monitor_launch_now_wires_launch_feedback_to_issue_row() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            max_active_agents: 1,
            ..Default::default()
        },
    )
    .expect("seed positive Monitor pane capacity");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.path().join("sessions"),
        sample_agent_options(),
    );

    let _events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorLaunchNow {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::UseStartMethod {
                method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
            },
            bounds: None,
        },
    );
    resolve_launch_wizard_runtime_confirmation(
        &mut runtime,
        &recorded_events,
        "issue monitor launch now runtime resolution",
    );
    let confirm_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
            bounds: None,
        },
    );
    assert!(
        confirm_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "runtime submit should move to the launch confirmation step"
    );

    let launch_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
            bounds: Some(canvas_bounds()),
        },
    );

    assert!(
        launch_events.iter().any(|event| {
            matches!(
                &event.event,
                BackendEvent::LaunchWizardState {
                    wizard: Some(wizard)
                } if wizard.launch_materialization_pending
            )
        }),
        "confirm submit should report materialization progress before creating an agent window"
    );
    let launch_events = dispatch_launch_materialization_request(
        &mut runtime,
        &recorded_events,
        "issue monitor launch now materialization",
    );
    assert!(
        launch_events.iter().any(|event| {
            matches!(
                event.event,
                BackendEvent::LaunchWizardState { wizard: None }
            )
        }),
        "materialization dispatch should close the wizard and create an agent window"
    );
    assert!(
        runtime
            .pending_launch_feedback_contexts
            .values()
            .any(|context| context.issue_monitor_issue_number == Some(3165)),
        "manual Issue Monitor launches must report launch completion/failure back to the row"
    );
}

#[test]
fn app_runtime_agent_window_initial_state_broadcast_includes_agent_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();

    let events = runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("initial WindowCanvasState broadcast");
    let tab = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .expect("tab in WindowCanvasState");
    let agent_window = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window in WindowCanvasState");

    assert_eq!(agent_window.title, "Claude Code");
    assert_eq!(agent_window.agent_id.as_deref(), Some("claude"));
}

#[test]
fn app_state_view_projects_agent_window_worktree_form_without_guessing_restored_windows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let mut tab_workspace = empty_workspace_state();
    let mut ephemeral = sample_window(
        "agent-ephemeral",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    ephemeral.title = "Codex".to_string();
    ephemeral.agent_id = Some("codex".to_string());
    let mut branch_backed = sample_window(
        "agent-branch-backed",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    branch_backed.title = "Codex".to_string();
    branch_backed.agent_id = Some("codex".to_string());
    let mut restored = sample_window(
        "agent-restored",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    restored.title = "Codex".to_string();
    restored.agent_id = Some("codex".to_string());
    tab_workspace
        .windows
        .extend([ephemeral, branch_backed, restored]);
    tab_workspace.next_z_index = 4;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let ephemeral_id = combined_window_id("tab-1", "agent-ephemeral");
    let branch_backed_id = combined_window_id("tab-1", "agent-branch-backed");
    let manager = gwt_git::WorktreeManager::new(&repo);
    let ephemeral_worktree = temp.path().join(".intake-ephemeral");
    manager
        .create_detached("HEAD", &ephemeral_worktree)
        .expect("create detached ephemeral worktree");
    let branch_backed_worktree = temp.path().join(".intake-branch-backed");
    manager
        .create_from_base(
            "HEAD",
            "feature/worktree-form-real",
            &branch_backed_worktree,
        )
        .expect("create branch worktree with an ephemeral-like basename");

    let mut ephemeral_session = sample_active_agent_session("tab-1", &ephemeral_id);
    ephemeral_session.branch_name = "work".to_string();
    ephemeral_session.worktree_path = ephemeral_worktree;
    runtime
        .active_agent_sessions
        .insert(ephemeral_id.clone(), ephemeral_session);

    let mut branch_backed_session = sample_active_agent_session("tab-1", &branch_backed_id);
    branch_backed_session.branch_name = "feature/worktree-form-real".to_string();
    branch_backed_session.worktree_path = branch_backed_worktree;
    runtime
        .active_agent_sessions
        .insert(branch_backed_id.clone(), branch_backed_session);

    let view = runtime.app_state_view();
    let windows = &view
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .expect("tab")
        .workspace
        .windows;
    let form = |raw_id: &str| {
        windows
            .iter()
            .find(|window| window.id == combined_window_id("tab-1", raw_id))
            .map(|window| window.worktree_form)
            .expect("projected window")
    };

    assert_eq!(form("agent-ephemeral"), gwt::WindowWorktreeForm::Ephemeral);
    assert_eq!(
        form("agent-branch-backed"),
        gwt::WindowWorktreeForm::BranchBacked,
        "a named branch worktree with an .intake-* basename must not be classified as ephemeral",
    );
    assert_eq!(
        form("agent-restored"),
        gwt::WindowWorktreeForm::Unknown,
        "restored agent windows without an active session signal must remain unknown",
    );
}

#[test]
fn app_runtime_agent_window_initial_title_uses_branch_issue_link_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            2468,
            "SPEC: Branch linked purpose",
            &["gwt-spec"],
            "Spec body",
            "2026-05-06T00:00:00Z",
        ))
        .expect("write issue cache");
    write_issue_link_store(
        &repo,
        HashMap::from([("work/20260506-1257".to_string(), 2468)]),
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260506-1257")
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.purpose_title.as_deref(),
        Some("SPEC: Branch linked purpose")
    );
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_agent_window_initial_title_falls_back_to_projection_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.owner = Some("SPEC-2008".to_string());
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260506-1257".to_string()),
        worktree_path: Some(repo.join("work/20260506-1257")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260506-1257")
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(agent_window.purpose_title.as_deref(), Some("SPEC-2008"));
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_agent_window_initial_title_ignores_projection_owner_for_other_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.owner = Some("PR-2525".to_string());
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/old".to_string()),
        worktree_path: Some(repo.join("work-old")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260507-0714")
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(agent_window.purpose_title, None);
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_board_milestone_updates_same_session_agent_window_detail_only() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent_one = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent_one.title = "Codex".to_string();
    agent_one.purpose_title = Some("Initial purpose".to_string());
    let mut agent_two = sample_window("agent-2", WindowPreset::Agent, WindowProcessStatus::Running);
    agent_two.title = "Claude".to_string();
    agent_two.purpose_title = Some("Other purpose".to_string());
    tab_workspace.windows.push(agent_one);
    tab_workspace.windows.push(agent_two);
    tab_workspace.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let first_window_id = combined_window_id("tab-1", "agent-1");
    let second_window_id = combined_window_id("tab-1", "agent-2");
    runtime.active_agent_sessions.insert(
        first_window_id.clone(),
        ActiveAgentSession {
            window_id: first_window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260506-0736".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    runtime.active_agent_sessions.insert(
        second_window_id,
        ActiveAgentSession {
            window_id: combined_window_id("tab-1", "agent-2"),
            session_id: "session-2".to_string(),
            agent_id: "claude".to_string(),
            branch_name: "work/20260506-0737".to_string(),
            display_name: "Claude".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_assigned_workspace_projection_for_test(
        &repo,
        runtime
            .active_agent_sessions
            .get(&first_window_id)
            .expect("first session"),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Implement dynamic title sync with detailed workspace context",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_title_summary("Implement dynamic title sync");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent 1")
            .dynamic_title
            .as_deref(),
        None
    );
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent 1")
            .dynamic_title_detail
            .as_deref(),
        Some("Implement dynamic title sync with detailed workspace context")
    );
    assert_eq!(
        tab.workspace
            .window("agent-2")
            .expect("agent 2")
            .dynamic_title
            .as_deref(),
        None
    );
}

/// Phase U-5 (SPEC-2359 US-38, FR-125, FR-126): a Board post that updates
/// an agent's current focus must broadcast both `WindowCanvasState` (so the
/// pane detail rehydrates on WS reconnect / GUI reload), then schedule
/// `ActiveWorkProjection` (Active Work card) on the background path. Board
/// `title_summary` is legacy history metadata; the live pane title comes
/// from Workspace purpose updates.
#[test]
fn app_runtime_board_milestone_broadcasts_workspace_state_for_focus_sync() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260513-0343".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_assigned_workspace_projection_for_test(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Implementing Phase U-5 Board path title sync hardening",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_title_summary("Implementing Phase U-5");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
            "expected WindowCanvasState broadcast from Board path so pane heading refreshes on reconnect: {events:?}"
        );
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let projection = wait_for_active_work_projection(&mut runtime);
    assert_eq!(projection.active_agents, 1);
}
