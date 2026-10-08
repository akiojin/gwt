use super::*;

#[test]
fn pm_process_refresh_replaces_prior_fresh_when_git_root_inspection_fails() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("not-a-git-repository");
    fs::create_dir_all(&repo).expect("non-Git project directory");
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.worktree_freshness = Some(gwt::pm_registry::PmWorktreeFreshness {
            state: gwt::pm_registry::PmWorktreeFreshnessState::Fresh,
            base_ref: "origin/develop".to_string(),
            head_sha: Some("stale-fresh-sentinel".to_string()),
            target_sha: Some("stale-fresh-sentinel".to_string()),
            behind: Some(0),
            target_observation: gwt::pm_registry::PmWorktreeTargetObservation::Fresh,
            checked_at: "2026-08-29T00:00:00Z".to_string(),
            failure_stage: None,
            failure_reason: None,
        });
    })
    .expect("seed prior Fresh state");

    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect_err("Git-root inspection must fail for a non-Git project");

    let freshness = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("inspection failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
    assert_eq!(
        freshness.target_observation,
        gwt::pm_registry::PmWorktreeTargetObservation::Unavailable
    );
}

#[cfg(unix)]
#[test]
fn pm_ensure_fresh_spawn_rejects_symlinked_scratch_dir() {
    use std::os::unix::fs::symlink;

    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo);
    fs::create_dir_all(scratch.parent().expect("project-state parent"))
        .expect("create project-state");
    let external = temp.path().join("external-scratch");
    fs::create_dir_all(&external).expect("create external scratch target");
    symlink(&external, &scratch).expect("symlink PM scratch root");
    let original_target = fs::read_link(&scratch).expect("read scratch symlink target");
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.worktree_freshness = Some(gwt::pm_registry::PmWorktreeFreshness {
            state: gwt::pm_registry::PmWorktreeFreshnessState::Fresh,
            base_ref: "origin/develop".to_string(),
            head_sha: Some("stale-fresh-sentinel".to_string()),
            target_sha: Some("stale-fresh-sentinel".to_string()),
            behind: Some(0),
            target_observation: gwt::pm_registry::PmWorktreeTargetObservation::Fresh,
            checked_at: "2026-08-29T00:00:00Z".to_string(),
            failure_stage: None,
            failure_reason: None,
        });
    })
    .expect("seed prior Fresh state");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "a symlinked scratch root must stop PM spawn"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
    assert!(fs::symlink_metadata(&scratch)
        .expect("scratch symlink metadata")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&scratch).expect("scratch symlink remains"),
        original_target
    );
    assert!(
        fs::read_dir(&external)
            .expect("external scratch target")
            .next()
            .is_none(),
        "spawn refusal must not create notes through the external target"
    );
    let freshness = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("preflight failure must replace prior Fresh state");
    assert_ne!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Fresh,
        "a rejected scratch preflight must not leave a stale Fresh claim"
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::ScratchMigration)
    );
}

#[cfg(unix)]
#[test]
fn pm_ensure_fresh_spawn_rejects_symlinked_project_state_dir() {
    use std::os::unix::fs::symlink;

    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let project_state = gwt_core::paths::gwt_project_dir_for_repo_path(&repo).join("project-state");
    fs::create_dir_all(project_state.parent().expect("project directory"))
        .expect("create project directory");
    let external = temp.path().join("external-project-state");
    fs::create_dir_all(&external).expect("create external project-state target");
    symlink(&external, &project_state).expect("symlink project-state");
    let original_target = fs::read_link(&project_state).expect("read project-state target");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "a symlinked project-state directory must stop PM spawn"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
    assert!(fs::symlink_metadata(&project_state)
        .expect("project-state symlink metadata")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&project_state).expect("project-state symlink remains"),
        original_target
    );
    assert!(
        !external.join("pm-scratch").exists(),
        "spawn refusal must not create scratch through the external target"
    );
}

/// SPEC-3431 FR-021: "停止中・未起動でもボタンは押下可能で、押下すると起動
/// （または resume）する" — an explicit click must start the PM even when
/// auto-start is opted out. `auto_start` governs the automatic ensure on
/// project open (FR-002); it is not a lock on the user's own actions, and
/// treating it as one leaves the launcher and the Restart button silently
/// dead with no way back short of editing pm.json by hand.
#[test]
fn explicit_pm_actions_start_the_pm_even_when_auto_start_is_opted_out() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    disable_pm_auto_start(&repo);

    // The project-open path stays suppressed (FR-002).
    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "the automatic ensure must still honour the opt-out"
    );

    // The launcher click does not.
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenPmAgent { bounds: None },
    );

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .len(),
        1,
        "an explicit PM launcher click must start the PM"
    );
}

#[test]
fn pm_codex_hook_trust_uses_runtime_paths_and_relative_codex_home() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&temp.path().join("repo"));
    fs::create_dir_all(&worktree).unwrap();
    let runtime = worktree.parent().unwrap().join("runtime");
    let codex_home = runtime.join("codex-state");
    fs::create_dir_all(&codex_home).unwrap();
    gwt_skills::generate_codex_hooks(&runtime).unwrap();
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .build();
    config
        .env_vars
        .insert("CODEX_HOME".into(), "codex-state".into());
    let report = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &temp.path().join("missing-config.toml"),
        &worktree,
        &config,
        None,
        gwt_skills::CodexHookDiscoveryMode::Both,
        None,
    )
    .unwrap()
    .expect("PM hook trust resolves CODEX_HOME relative to the provider cwd");
    assert!(!report.trusted_entries.is_empty());
    let trusted: toml::Value =
        toml::from_str(&fs::read_to_string(codex_home.join("config.toml")).unwrap()).unwrap();
    assert_every_codex_hook_is_trusted(&trusted, &runtime.join(".codex/hooks.json"));
    assert!(!worktree.join("codex-state").exists());
}

/// SPEC-4486 AC-5a: provider discovery is isolated while project data and
/// canonical Session identity remain available on fresh and resumed launches.
#[test]
fn pm_process_launch_isolates_discovery_and_keeps_project_data_readable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = create_detached_pm_worktree_fixture(&repo);
    fs::write(worktree.join("AGENTS.md"), "PROJECT_POLICY_DATA").unwrap();
    fs::write(worktree.join("source.rs"), "PROJECT_SOURCE_DATA").unwrap();
    fs::create_dir_all(worktree.join(".codex")).unwrap();
    fs::write(worktree.join(".codex/config.toml"), "project_marker = true").unwrap();
    let runtime_dir = worktree.parent().unwrap().join("runtime");
    let sessions = temp.path().join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let mut previous = None;
    for resumed in [false, true] {
        let mut config = if let Some(session) = previous.as_ref() {
            super::super::launch_config_from_persisted_session(session)
        } else {
            AppRuntime::pm_launch_config(
                &worktree,
                &gwt::pm_registry::PmLaunchProfile {
                    agent_id: "codex".into(),
                    model: None,
                    reasoning: None,
                    version: None,
                },
            )
        };
        assert_eq!(config.working_dir.as_deref(), Some(worktree.as_path()));
        config.command = fake_codex.display().to_string();
        let codex_home = temp.path().join("codex-state");
        fs::create_dir_all(&codex_home).unwrap();
        config
            .env_vars
            .insert("CODEX_HOME".into(), codex_home.display().to_string());
        let (proxy, events) = AppEventProxy::stub();
        AppRuntime::spawn_agent_window_async(
            proxy,
            sessions.clone(),
            repo.display().to_string(),
            "tab-1::pm-isolation".into(),
            config,
            temp.path().join("missing-config.toml"),
            None,
        );
        let recorded = events.lock().unwrap();
        let result = recorded
            .iter()
            .find_map(|event| match event {
                UserEvent::LaunchComplete { result, .. } => Some(result.as_ref()),
                _ => None,
            })
            .expect("LaunchComplete");
        let completion = result.as_ref().expect("successful PM process preparation");
        assert_eq!(
            completion.0.cwd.as_deref(),
            Some(runtime_dir.as_path()),
            "resume={resumed}"
        );
        assert_eq!(completion.4, worktree);
        assert_eq!(
            completion.0.env.get("GWT_PROJECT_ROOT"),
            Some(&worktree.display().to_string())
        );
        let mut session =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", completion.1))).unwrap();
        assert_eq!(session.worktree_path, worktree);
        assert_eq!(session.project_state_root.as_deref(), Some(repo.as_path()));
        assert_eq!(
            session.session_mode,
            if resumed {
                gwt_agent::SessionMode::Resume
            } else {
                gwt_agent::SessionMode::Normal
            }
        );
        assert!(!runtime_dir.join("AGENTS.md").exists());
        assert!(!runtime_dir.join(".codex/config.toml").exists());
        let project = PathBuf::from(&completion.0.env["GWT_PROJECT_ROOT"]);
        assert_eq!(
            fs::read_to_string(project.join("AGENTS.md")).unwrap(),
            "PROJECT_POLICY_DATA"
        );
        assert_eq!(
            fs::read_to_string(project.join("source.rs")).unwrap(),
            "PROJECT_SOURCE_DATA"
        );
        session.agent_session_id = Some("pm-isolation-resume".into());
        session.save(&sessions).unwrap();
        previous = Some(session);
    }
}

/// SPEC-1921 AS-1921-D: legacy Monitor selectors do not reach the launch;
/// reading them does not rewrite the saved candidate pool.
#[test]
fn monitor_stored_version_selector_is_ignored_without_rewriting_the_profile() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let path = temp.path().join("issue-monitor.json");
    for version in ["latest", "0.121.0"] {
        let mut profile = codex_issue_monitor_launch_profile();
        profile.version = Some(version.into());
        let prefs = gwt::IssueMonitorPrefs {
            launch_profile: Some(profile),
            ..Default::default()
        };
        gwt::save_issue_monitor_prefs(&path, &prefs).expect("save profile");
        let before = fs::read(&path).expect("profile bytes");
        let loaded = gwt::load_issue_monitor_prefs(&path).expect("load profile");
        let previous =
            gwt::LaunchWizardPreviousProfiles::from_profile(loaded.launch_profile.map(Into::into));
        let mut wizard = sample_ready_agent_launch_wizard_session("tab-1", temp.path()).wizard;
        wizard.apply_hydration(gwt::LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/demo".into(),
            worktree_path: Some(temp.path().to_path_buf()),
            quick_start_root: temp.path().to_path_buf(),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(previous),
            open_branch_candidates: Vec::new(),
        });
        wizard.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".into(),
        });
        let config = wizard.build_launch_config().expect("launch config");
        assert_eq!(config.command, "codex");
        assert_eq!(config.tool_version, None);
        assert!(!config.args.iter().any(|arg| arg.contains("@openai/codex@")));
        assert_eq!(fs::read(&path).expect("unchanged profile"), before);
    }
}

#[test]
fn pm_launch_config_reads_a_stored_version_selector_as_installed() {
    let worktree = std::path::Path::new("/tmp/pm-worktree");
    for (agent_id, version, command) in [
        ("grok", "latest", "grok"),
        ("grok", "1.0.3", "grok"),
        ("codex", "0.121.0", "codex"),
    ] {
        let config = AppRuntime::pm_launch_config(
            worktree,
            &gwt::pm_registry::PmLaunchProfile {
                agent_id: agent_id.to_string(),
                model: None,
                reasoning: None,
                version: Some(version.to_string()),
            },
        );
        assert_eq!(config.command, command, "{:?}", config.args);
        assert!(
            config
                .args
                .iter()
                .all(|arg| !arg.ends_with(&format!("@{version}"))),
            "the stored selector must not reach the launch: {:?}",
            config.args
        );
        assert_eq!(config.tool_version, None, "{agent_id}@{version}");
    }
}

/// SPEC-3431 FR-026: a fresh project has no profile and must still start, and
/// a configured one must actually reach the launch.
#[test]
fn pm_launch_config_resolves_the_configured_agent_and_defaults_on_a_fresh_project() {
    let worktree = std::path::Path::new("/tmp/pm-worktree");

    let default_config = AppRuntime::pm_launch_config(
        worktree,
        &gwt::pm_registry::PmLaunchProfile::default_profile(),
    );
    assert_eq!(default_config.agent_id, gwt_agent::AgentId::ClaudeCode);
    assert_eq!(default_config.model, None);
    assert!(default_config.suppress_execution_control);
    assert!(default_config.args.iter().any(|arg| arg == "$gwt-pm"));

    let configured = AppRuntime::pm_launch_config(
        worktree,
        &gwt::pm_registry::PmLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.1-codex-max".to_string()),
            reasoning: Some("high".to_string()),
            version: None,
        },
    );
    assert_eq!(configured.agent_id, gwt_agent::AgentId::Codex);
    assert_eq!(configured.model.as_deref(), Some("gpt-5.1-codex-max"));
    assert_eq!(configured.reasoning_level.as_deref(), Some("high"));
    assert!(configured.suppress_execution_control);

    let grok = AppRuntime::pm_launch_config(
        worktree,
        &gwt::pm_registry::PmLaunchProfile {
            agent_id: "grok".to_string(),
            model: Some("DefaultXL".to_string()),
            reasoning: Some("xhigh".to_string()),
            version: None,
        },
    );
    assert_eq!(grok.agent_id, gwt_agent::AgentId::GrokBuild);
    assert_eq!(grok.model.as_deref(), Some("DefaultXL"));
    assert_eq!(grok.reasoning_level.as_deref(), Some("xhigh"));
    assert_eq!(
        grok.args
            .windows(2)
            .filter(|pair| pair[0] == "--model" && pair[1] == "DefaultXL")
            .count(),
        1
    );
    assert_eq!(
        grok.args
            .windows(2)
            .filter(|pair| pair[0] == "--effort" && pair[1] == "xhigh")
            .count(),
        1
    );
    assert!(grok.args.iter().any(|arg| arg == "$gwt-pm"));
}

/// SPEC-3431 FR-119/FR-120 / T-484: selecting Grok is a durable PM profile,
/// including the free-text model and reasoning effort that the next launch
/// must receive. Existing unsupported providers remain rejected by the
/// neighboring regression test.
#[test]
fn set_pm_launch_profile_accepts_grok_and_persists_model_and_reasoning() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetPmLaunchProfile {
            agent_id: "grok".to_string(),
            model: Some("  grok-4.20-beta  ".to_string()),
            reasoning: Some("  xhigh  ".to_string()),
        },
    );

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let profile = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("load PM prefs")
        .settings
        .launch_profile
        .expect("Grok PM profile must be persisted");
    assert_eq!(profile.agent_id, "grok");
    assert_eq!(profile.model.as_deref(), Some("grok-4.20-beta"));
    assert_eq!(profile.reasoning.as_deref(), Some("xhigh"));
}

/// SPEC-3431 FR-121 / T-484: configured and running launch identities are
/// deliberately independent. A changed profile is pending restart until the
/// live pane's agent, model, and reasoning all match the configured values.
#[test]
fn pm_status_projects_configured_and_running_agent_model_and_reasoning() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = "pm-session-live".to_string();
    active.agent_id = "claude".to_string();
    active.display_name = "Claude Code".to_string();
    runtime.active_agent_sessions.insert(window_id, active);

    let mut running = gwt_agent::Session::new(&repo, "pm", gwt_agent::AgentId::ClaudeCode);
    running.id = "pm-session-live".to_string();
    running.model = Some("claude-opus-4-1".to_string());
    running.reasoning_level = Some("high".to_string());
    running
        .save(&runtime.sessions_dir)
        .expect("save live PM session launch identity");

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.settings.launch_profile = Some(gwt::pm_registry::PmLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.6".to_string()),
            reasoning: Some("xhigh".to_string()),
            version: None,
        });
    })
    .expect("configure the next PM launch");
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("register the live PM");

    let status = serde_json::to_value(runtime.pm_status_event(&runtime.test_context()))
        .expect("serialize PM status");
    assert_eq!(status["configured_agent_id"], "codex");
    assert_eq!(status["configured_model"], "gpt-5.6");
    assert_eq!(status["configured_reasoning"], "xhigh");
    assert_eq!(status["running_agent_id"], "claude");
    assert_eq!(status["running_model"], "claude-opus-4-1");
    assert_eq!(status["running_reasoning"], "high");
    assert_eq!(status["is_running"], true);
}

#[test]
fn pm_launch_config_exports_project_state_scratch_dir() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempdir().expect("gwt home");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let repo = home.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo fixture");
    init_repo(&repo);
    let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    fs::create_dir_all(&worktree).expect("create canonical PM worktree fixture");
    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo);
    assert!(
        !scratch.starts_with(&worktree),
        "PM scratch must live outside the disposable PM worktree"
    );

    for agent_id in ["claude", "codex"] {
        let config = AppRuntime::pm_launch_config(
            &worktree,
            &gwt::pm_registry::PmLaunchProfile {
                agent_id: agent_id.to_string(),
                model: None,
                reasoning: None,
                version: None,
            },
        );
        assert_eq!(
            config.env_vars.get("GWT_PM_SCRATCH_DIR").map(PathBuf::from),
            Some(scratch.clone()),
            "{agent_id} PM launch must receive the canonical project-state scratch path"
        );
    }
}

/// SPEC-3431 FR-012 / FR-026 (2026-08-06 ユーザー裁定): the PM runs unattended
/// — it subscribes, reconciles, registers Issues, and instructs launches with
/// no user present. A permission prompt in that loop is a deadlock nobody is
/// watching, so the PM always launches with permissions skipped. This mirrors
/// `force_skip_permissions_for_autonomous` on the Issue Monitor's own
/// unattended launches; it is not a per-project choice, because a PM that can
/// be configured into a hang is a PM that will eventually hang.
#[test]
fn pm_launch_config_always_skips_permissions() {
    let worktree = std::path::Path::new("/tmp/pm-worktree");
    // Assert the argv, not just the flag: setting `skip_permissions` without
    // it reaching the command line would leave the PM prompting anyway.
    for (agent_id, expected_arg) in [
        ("claude", "--dangerously-skip-permissions"),
        ("codex", "--yolo"),
    ] {
        let profile = gwt::pm_registry::PmLaunchProfile {
            agent_id: agent_id.to_string(),
            model: None,
            reasoning: None,
            version: None,
        };
        let config = AppRuntime::pm_launch_config(worktree, &profile);
        assert!(
            config.skip_permissions,
            "the resident PM must never stop on a permission prompt ({agent_id})"
        );
        assert!(
            config.args.iter().any(|arg| arg == expected_arg),
            "{agent_id} must launch with {expected_arg}; got {:?}",
            config.args
        );
    }
}

#[test]
fn pm_ensure_resumes_stale_registration_conversation() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    // AS5 / FR-003: a dead PM with a materializable session resumes the same
    // conversation instead of spawning a fresh PM.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let mut session = gwt_agent::Session::new(repo.clone(), "work", gwt_agent::AgentId::ClaudeCode);
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.restore_window_on_startup = true;
    session.save(&runtime.sessions_dir).expect("save session");

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture(&session.id, &repo),
        |_| false,
    )
    .expect("seed stale registration");

    let events =
        runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    assert!(!events.is_empty(), "stale PM resumes");
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "resume spawns exactly one PM pane");
    assert_eq!(windows[0].preset, WindowPreset::Agent);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "resumed launch still registers the successor session at completion"
    );
}

/// Issue #4375: the stale-registration resume must still record its successor
/// for PM registration. The synchronous path marked the new pane in the ensure
/// call, immediately after `spawn_restored_agent_session` returned. Once that
/// resume waits on an off-loop worktree preparation there is no pane to mark at
/// that moment, so the marking has to travel with the continuation — otherwise
/// launch completion never rewrites `pm.json` and it keeps naming the dead
/// session.
#[test]
fn pm_ensure_resume_inside_the_pm_worktree_still_tracks_its_launch() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let mut session = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.restore_window_on_startup = true;
    session.save(&runtime.sessions_dir).expect("save session");
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture(&session.id, &pm_worktree),
        |_| false,
    )
    .expect("seed stale registration");

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "the resume waits for the off-loop worktree preparation"
    );

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "the prepared resume spawns the PM pane");
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "the resume spawns exactly one PM pane");
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "the resumed launch must still register the successor session at completion"
    );
}

#[test]
fn pm_bootstrap_ensures_pm_for_open_git_tabs() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    // FR-002: tabs already open at launch get the PM pane from bootstrap.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.bootstrap();

    // Agent panes never spawn before the canvas reports bounds: bootstrap
    // only queues the ensure (same deferral rule as startup auto-resume).
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
    assert_eq!(runtime.pending_startup_pm_tabs, vec!["tab-1".to_string()]);

    runtime.startup_auto_resume_ready_events(canvas_bounds());

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "canvas-ready drain spawns the PM pane");
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "exactly one PM pane spawned");
    assert_eq!(windows[0].preset, WindowPreset::Agent);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1
    );
    assert!(runtime.pending_startup_pm_tabs.is_empty());
}

#[test]
fn pm_bootstrap_respects_opt_out() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    // FR-002 negative: the project-level opt-out suppresses bootstrap too.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    disable_pm_auto_start(&repo);

    runtime.bootstrap();
    runtime.startup_auto_resume_ready_events(canvas_bounds());

    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
}

#[test]
fn pm_open_project_skips_migration_pending_repo() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    // A Normal-layout repo opens with migration pending; PM ensure must wait
    // until the migration decision instead of spawning into a layout gwt is
    // about to rewrite.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    runtime.open_project_path_events(project);
    commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    let tab_id = runtime.active_tab_id.clone().expect("active tab");
    assert!(runtime
        .tab(&tab_id)
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .all(|window| window.preset != WindowPreset::Agent));
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
}

#[test]
fn pm_close_window_deregisters_pm() {
    // FR-013: closing the PM pane is an intentional stop — the registration
    // is cleared and no auto-restart may follow; settings survive.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = runtime
        .runtimes
        .get(&window_id)
        .expect("PM runtime")
        .pty
        .clone();
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");

    runtime.close_window_events(&window_id);

    let before_finalizer = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load queued prefs");
    assert!(
        before_finalizer.registration.is_some(),
        "durable PM cleanup belongs to the background finalizer"
    );
    assert!(
        pty.try_wait().expect("PM child probe").is_none(),
        "PM cleanup must not run before the captured PTY is reaped"
    );
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued PM close finalizer");
    finalizer();

    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load finalized prefs");
    assert_eq!(
        prefs.registration, None,
        "closing the PM pane must deregister the PM"
    );
    assert!(
        prefs.settings.auto_start,
        "settings survive deregistration (FR-002)"
    );
    assert!(
        pty.try_wait().expect("PM child reap probe").is_some(),
        "PM worktree cleanup runs only after the old PTY is reaped"
    );
}

#[test]
fn pm_close_completion_stays_with_owner_and_is_dropped_after_owner_closes() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("repo a");
    fs::create_dir_all(&repo_b).expect("repo b");
    let tabs = vec![
        sample_project_tab("tab-a", "Repo A", repo_a.clone(), ProjectKind::Git, &[]),
        sample_project_tab("tab-b", "Repo B", repo_b.clone(), ProjectKind::Git, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-b"));
    let stale_status = BackendEvent::PmStatus {
        available: true,
        auto_start: false,
        loop_interval_secs: 10,
        loop_interval_secs_decimal: "10".to_string(),
        configured_agent_id: "codex".to_string(),
        configured_model: None,
        configured_reasoning: None,
        running_agent_id: None,
        running_model: None,
        running_reasoning: None,
        is_running: false,
        agent_options: Vec::new(),
        start_block: None,
    };

    let after_switch = runtime.handle_window_close_finalized(
        "tab-a::pm-a",
        Some(&repo_a),
        Some("pm-a"),
        true,
        true,
        Some(stale_status.clone()),
        super::super::WindowCloseMonitorResult::Noop,
    );
    assert!(
        after_switch
            .iter()
            .all(|outbound| matches!(&outbound.target, DispatchTarget::Project(key) if key == &runtime.project_context("tab-a").expect("owner context").project_key)),
        "a delayed close from repo A must target only repo A"
    );

    runtime.close_project_tab_events("tab-a");
    runtime.close_project_tab_events("tab-b");
    let after_last_tab_close = runtime.handle_window_close_finalized(
        "tab-a::pm-a-final",
        Some(&repo_a),
        Some("pm-a"),
        true,
        false,
        Some(stale_status),
        super::super::WindowCloseMonitorResult::Noop,
    );
    assert!(
        after_last_tab_close
            .iter()
            .all(|outbound| !matches!(outbound.event, BackendEvent::PmStatus { .. })),
        "a delayed close after the last tab closed must not restore stale PM settings"
    );
}

#[test]
fn concurrent_pm_close_completions_keep_counted_fence_and_successor_cache() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pending_pm_closes
        .insert(repo.clone(), 2);
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pm_sessions
        .insert(repo.clone(), "pm-successor".to_string());
    let stale_status = BackendEvent::IssueMonitorToast {
        notification_transition: None,
        level: "error".to_string(),
        message: "stale predecessor PM status".to_string(),
        issue_number: None,
    };

    let first = runtime.handle_window_close_finalized(
        "tab-1::predecessor-a",
        Some(&repo),
        Some("pm-predecessor-a"),
        true,
        true,
        Some(stale_status),
        super::super::WindowCloseMonitorResult::Noop,
    );
    assert!(first.is_empty(), "a stale PM completion must not broadcast");
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_closes
            .get(&repo),
        Some(&1)
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pm_sessions
            .get(&repo)
            .map(String::as_str),
        Some("pm-successor")
    );

    runtime.handle_window_close_finalized(
        "tab-1::predecessor-b",
        Some(&repo),
        Some("pm-predecessor-b"),
        true,
        false,
        None,
        super::super::WindowCloseMonitorResult::Noop,
    );
    assert!(!runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_closes
        .contains_key(&repo));
}

#[test]
fn non_pm_pane_close_does_not_count_the_pm_close_fence() {
    // PR #3787 review: `pending_pm_closes` fences `ensure_pm_agent_events`
    // (Automatic / Explicit triggers) and thereby PM crash respawn. Closing
    // an ordinary agent pane in the same project must not raise that fence —
    // only a close of the registered PM session may.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "worker-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pm_sessions
        .insert(repo.clone(), "pm-session-live".to_string());

    runtime.close_window_events(&window_id);

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_closes
            .is_empty(),
        "a non-PM pane close must not fence PM ensure"
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pm_sessions
            .get(&repo)
            .map(String::as_str),
        Some("pm-session-live"),
        "the registered PM session survives a worker pane close"
    );
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued close finalizer");
    finalizer();
}

#[test]
fn pm_pane_close_counts_the_pm_close_fence() {
    // PR #3787 review companion: the fence still counts an explicit close of
    // the registered PM session itself, and the finalized completion releases
    // exactly that count.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pm_sessions
        .insert(repo.clone(), "pm-session-live".to_string());

    runtime.close_window_events(&window_id);

    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_closes
            .get(&repo),
        Some(&1),
        "closing the registered PM session raises the ensure fence"
    );
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pm_sessions
            .contains_key(&repo),
        "the closing PM session leaves the in-memory registration cache"
    );
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued PM close finalizer");
    finalizer();
    runtime.handle_window_close_finalized(
        &window_id,
        Some(&repo),
        Some("pm-session-live"),
        true,
        false,
        None,
        super::super::WindowCloseMonitorResult::Noop,
    );
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_closes
            .contains_key(&repo),
        "the finalized PM close releases the fence"
    );
}

#[test]
fn pm_crash_records_backoff_and_respawns() {
    // FR-003/AS5: an unexpected exit keeps the registration, records the
    // crash on the backoff ladder, and (first crash) respawns immediately by
    // resuming the same conversation.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();

    let mut session = gwt_agent::Session::new(repo.clone(), "work", gwt_agent::AgentId::ClaudeCode);
    session.restore_window_on_startup = true;
    session.save(&runtime.sessions_dir).expect("save session");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = session.id.clone();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture(&session.id, &repo),
        |_| false,
    )
    .expect("seed registration");

    runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("agent crashed".to_string()),
        true,
    );

    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load prefs");
    let registration = prefs
        .registration
        .expect("crash must keep the registration");
    assert_eq!(
        registration.consecutive_crashes, 1,
        "crash is recorded on the backoff ladder"
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "immediate respawn resumes the PM conversation"
    );
}

#[test]
fn pm_close_reaps_clean_and_migrated_legacy_scratch_but_keeps_unknown_files() {
    // T-016/T249a: known legacy PM notes are migrated out of the disposable
    // worktree before reaping. Unknown files remain fail-closed user work.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);

    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    fs::write(pm_worktree.join("user-file.txt"), "user work").expect("write unknown user file");
    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "unknown");
    assert!(
        pm_worktree.exists(),
        "an unknown user-authored file must keep the PM worktree"
    );
    assert_eq!(
        fs::read_to_string(pm_worktree.join("user-file.txt")).expect("read unknown user file"),
        "user work"
    );

    fs::remove_file(pm_worktree.join("user-file.txt")).expect("remove unknown user file");
    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "clean");
    assert!(
        !pm_worktree.exists(),
        "a clean PM worktree is reaped when the PM deregisters"
    );

    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let legacy_notes = b"legacy PM notes survive close";
    fs::write(pm_worktree.join("pm-notes.md"), legacy_notes).expect("write legacy PM notes");
    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "legacy");
    assert!(
        !pm_worktree.exists(),
        "known legacy PM scratch is migrated and no longer keeps the worktree"
    );
    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo);
    assert_eq!(
        fs::read(scratch.join("pm-notes.md")).expect("read migrated PM notes"),
        legacy_notes,
        "legacy PM notes must be preserved byte-for-byte in project-state scratch"
    );

    let seed = temp.path().join("seed");
    fs::create_dir_all(seed.join("tasks")).expect("seed tracked tasks");
    fs::write(seed.join("tasks/todo.md"), "tracked project task\n").expect("tracked task");
    run_git(&seed, &["add", "tasks/todo.md"]);
    run_git(&seed, &["commit", "-qm", "add tracked project task"]);
    run_git(
        &seed,
        &[
            "push",
            temp.path()
                .join("origin.git")
                .to_str()
                .expect("origin path"),
            "develop",
        ],
    );
    run_git(&repo, &["fetch", "origin", "develop"]);
    run_git(&repo, &["reset", "--hard", "origin/develop"]);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let modified_tracked_notes = b"PM-local tracked task update\n";
    fs::write(pm_worktree.join("tasks/todo.md"), modified_tracked_notes)
        .expect("modify tracked legacy notes");
    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "tracked-legacy");
    assert!(
        !pm_worktree.exists(),
        "durably externalized tracked legacy notes must not leave a synthetic deletion that blocks cleanup"
    );
    assert_eq!(
        fs::read(scratch.join("tasks/todo.md")).expect("externalized tracked PM notes"),
        modified_tracked_notes
    );
}

#[test]
fn pm_close_records_scratch_migration_failure_and_preserves_files() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let legacy = pm_worktree.join("pm-notes.md");
    let legacy_bytes = b"legacy source remains";
    fs::write(&legacy, legacy_bytes).expect("write legacy PM notes");
    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo);
    fs::create_dir_all(&scratch).expect("create PM scratch root");
    let destination = scratch.join("pm-notes.md");
    let destination_bytes = b"existing destination remains";
    fs::write(&destination, destination_bytes).expect("seed destination collision");

    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "migration-failure");

    assert!(
        pm_worktree.exists(),
        "migration failure must keep the PM worktree"
    );
    assert_eq!(fs::read(&legacy).expect("read legacy source"), legacy_bytes);
    assert_eq!(
        fs::read(&destination).expect("read colliding destination"),
        destination_bytes
    );
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load PM prefs");
    let freshness = prefs
        .worktree_freshness
        .expect("scratch migration failure must be durably visible");
    assert!(matches!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Stale
            | gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    ));
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::ScratchMigration)
    );
    assert!(matches!(
        freshness.target_observation,
        gwt::pm_registry::PmWorktreeTargetObservation::Cached
            | gwt::pm_registry::PmWorktreeTargetObservation::Unavailable
    ));
    let reason = freshness
        .failure_reason
        .expect("migration failure must include a reason");
    let destination_text = destination.to_string_lossy();
    assert!(
        reason.contains(destination_text.as_ref()),
        "failure reason must identify the colliding destination: {reason}"
    );
}

#[test]
fn pm_close_reaps_worktree_with_only_generated_hook_config() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    gwt_skills::generate_codex_hooks_for_mode(
        &pm_worktree,
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal,
    )
    .expect("generate managed Codex hooks");
    let hooks = pm_worktree.join(".codex/hooks.json");
    assert!(hooks.exists(), "managed hook fixture must exist");
    assert!(
        !gwt_skills::managed_hook_config_has_user_content(&hooks),
        "freshly generated hooks must contain only disposable gwt content"
    );

    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "generated-hooks");

    assert!(
        !pm_worktree.exists(),
        "pure gwt-generated hook config must not keep the PM worktree"
    );
}

#[test]
fn pm_close_keeps_generated_hook_config_with_user_content() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    gwt_skills::generate_codex_hooks_for_mode(
        &pm_worktree,
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal,
    )
    .expect("generate managed Codex hooks");
    let hooks = pm_worktree.join(".codex/hooks.json");
    let mut rendered: serde_json::Value =
        serde_json::from_slice(&fs::read(&hooks).expect("read managed hooks"))
            .expect("parse managed hooks");
    rendered
        .as_object_mut()
        .expect("managed hooks object")
        .insert("user-setting".to_string(), serde_json::json!(true));
    fs::write(
        &hooks,
        serde_json::to_vec_pretty(&rendered).expect("render user-extended hooks"),
    )
    .expect("write user-extended hooks");
    assert!(
        gwt_skills::managed_hook_config_has_user_content(&hooks),
        "the added key must classify the merged config as user content"
    );

    close_registered_pm_worktree_fixture(temp.path(), &repo, &pm_worktree, "user-hooks");

    assert!(
        pm_worktree.exists(),
        "user content in a merged hook config must keep the PM worktree"
    );
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&hooks).expect("read retained user hooks"))
            .expect("parse retained user hooks");
    assert_eq!(
        persisted.get("user-setting"),
        Some(&serde_json::json!(true))
    );
}

#[test]
fn workspace_view_marks_only_the_registered_pm_window() {
    // SPEC-3431 FR-020: the frontend needs to tell the PM window apart from
    // ordinary agent windows, and the marker must follow the durable
    // registration — never a stale persisted flag.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let mut persisted = empty_workspace_state();
    let mut pm_window = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    pm_window.session_id = Some("pm-session".to_string());
    let mut other_window =
        sample_window("agent-2", WindowPreset::Agent, WindowProcessStatus::Running);
    other_window.session_id = Some("worker-session".to_string());
    persisted.windows = vec![pm_window, other_window];

    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let foreign_repo = temp.path().join("foreign-repo");
    fs::create_dir_all(&foreign_repo).expect("foreign repo");
    init_repo(&foreign_repo);
    let foreign_tab = sample_project_tab(
        "tab-foreign",
        "Foreign",
        foreign_repo,
        ProjectKind::Git,
        &[],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab, foreign_tab], Some("tab-1"));

    let before = runtime.workspace_view_for_tab(runtime.tab("tab-1").expect("tab"));
    assert!(
        before.windows.iter().all(|window| !window.is_pm),
        "no registration means no PM window"
    );

    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pm_sessions
        .insert(repo.clone(), "pm-session".to_string());

    let view = runtime.workspace_view_for_tab(runtime.tab("tab-1").expect("tab"));
    let marked: Vec<&str> = view
        .windows
        .iter()
        .filter(|window| window.is_pm)
        .map(|window| window.id.as_str())
        .collect();
    assert_eq!(
        marked,
        vec!["tab-1::agent-1"],
        "exactly the registered PM session's window is marked"
    );

    // Chat projection must use the same registration as the role marker.
    // A worker's transcript is never exposed through the PM endpoint.
    for (id, availability) in [
        ("tab-1::agent-1", "waiting"),
        ("tab-1::agent-2", "unsupported"),
    ] {
        let request = serde_json::from_value(serde_json::json!({
            "kind": "load_pm_conversation", "id": id
        }))
        .expect("PM conversation request");
        let replies = runtime.handle_frontend_event("chat-client".to_string(), request);
        assert_eq!(replies.len(), 1);
        let reply = serde_json::to_value(&replies[0].event).unwrap();
        assert_eq!(reply["kind"], "pm_conversation");
        assert_eq!(reply["snapshot"]["availability"], availability);
        assert_eq!(reply["snapshot"]["messages"], serde_json::json!([]));
    }

    let foreign_context = runtime
        .project_context("tab-foreign")
        .expect("foreign context");
    assert!(
        runtime
            .handle_frontend_event_for_project(
                &foreign_context,
                "foreign-client".to_owned(),
                FrontendEvent::LoadPmConversation {
                    id: "tab-1::agent-1".to_owned()
                },
            )
            .is_empty(),
        "a project-scoped client cannot request another project's PM"
    );

    let native_home = temp.path().join("claude-home");
    let _claude_home = ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &native_home);
    let native_dir = native_home.join("projects/project");
    fs::create_dir_all(&native_dir).expect("native transcript directory");
    fs::write(
        native_dir.join("native-pm.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({
                "type":"assistant", "uuid":"answer", "sessionId":"native-pm", "cwd":repo,
                "message":{"role":"assistant","content":[{"type":"text","text":"PM answer"}]}
            })
        ),
    )
    .expect("native transcript");
    let mut session = gwt_agent::Session::new(&repo, "pm", gwt_agent::AgentId::ClaudeCode);
    session.id = "pm-session".to_owned();
    session.agent_session_id = Some("native-pm".to_owned());
    session.save(&runtime.sessions_dir).expect("PM session");
    assert!(
        runtime
            .handle_frontend_event(
                "chat-client".to_owned(),
                FrontendEvent::LoadPmConversation {
                    id: "tab-1::agent-1".to_owned()
                },
            )
            .is_empty(),
        "native reads return asynchronously"
    );
    wait_for_recorded_event("PM conversation read", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::PmConversationLoaded { .. }
            )
        })
    });
    let completion = recorded_events
        .lock()
        .unwrap()
        .iter()
        .find(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::PmConversationLoaded { .. }
            )
        })
        .unwrap()
        .clone();
    let UserEvent::PmConversationLoaded {
        client_id,
        window_id,
        session_id,
        snapshot,
    } = runtime
        .accept_project_completion(completion)
        .expect("current project completion")
    else {
        panic!("expected PM conversation completion");
    };
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].text, "PM answer");
    let replies = runtime.pm_conversation_loaded_events(
        client_id.clone(),
        &window_id,
        &session_id,
        snapshot.clone(),
    );
    assert!(
        matches!(replies.as_slice(), [OutboundEvent { target: DispatchTarget::Client(id), event: BackendEvent::PmConversation { .. }, .. }] if id == "chat-client")
    );

    // A native /clear may rotate the conversation without replacing the gwt
    // Session. A queued old read must not overwrite that new conversation.
    session.agent_session_id = Some("native-successor".to_owned());
    session
        .save(&runtime.sessions_dir)
        .expect("rotated native conversation");
    assert!(runtime
        .pm_conversation_loaded_events(client_id.clone(), &window_id, &session_id, snapshot.clone())
        .is_empty());
    session.agent_session_id = Some("native-pm".to_owned());
    session
        .save(&runtime.sessions_dir)
        .expect("original native identity");
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pm_sessions
        .insert(repo, "restarted-pm-session".to_owned());
    assert!(runtime
        .pm_conversation_loaded_events(client_id, &window_id, &session_id, snapshot)
        .is_empty());
}

/// SPEC #3885 T-020 (Issue #4082 AC-1): the workspace view carries the moment
/// the agent's PTY runtime started, so the Issue row's elapsed time survives a
/// frontend reload instead of restarting from the last observed state change.
/// The field is wire-only: the persisted image never learns it.
#[test]
fn workspace_view_carries_the_runtime_start_time_for_live_agent_windows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let mut persisted = empty_workspace_state();
    persisted.windows = vec![
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running),
        sample_window("agent-2", WindowPreset::Agent, WindowProcessStatus::Running),
    ];
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let before_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    insert_test_pane_runtime(&mut runtime, "tab-1::agent-1");
    let after_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;

    let view = runtime.workspace_view_for_tab(runtime.tab("tab-1").expect("tab"));
    let live = view
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .expect("live agent window");
    let started = live
        .runtime_started_at_ms
        .expect("a window with a live PTY runtime reports when it started");
    assert!(
        (before_ms..=after_ms).contains(&started),
        "start time {started} must fall inside the install window {before_ms}..={after_ms}"
    );
    let idle = view
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-2")
        .expect("agent window without a runtime");
    assert_eq!(
        idle.runtime_started_at_ms, None,
        "no PTY runtime means no start time"
    );
    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .iter()
            .all(|window| window.runtime_started_at_ms.is_none()),
        "the start time is wire-only and never enters the persisted image"
    );
    // The wire field must not be readable back from disk either.
    let json = serde_json::json!({
        "id": "agent-9",
        "title": "Agent",
        "preset": "agent",
        "geometry": { "x": 0, "y": 0, "width": 100, "height": 100 },
        "z_index": 1,
        "status": "running",
        "runtime_started_at_ms": 1_700_000_000_000u64,
    });
    let restored: gwt::PersistedWindowState =
        serde_json::from_value(json).expect("window state deserializes");
    assert_eq!(restored.runtime_started_at_ms, None);

    runtime.stop_window_runtime("tab-1::agent-1");
}

#[test]
fn open_pm_agent_event_routes_to_the_active_tab_ensure() {
    // SPEC-3431 FR-018/FR-019: the launcher event must reach the ensure gate
    // for the ACTIVE tab and carry the caller's canvas bounds so an existing
    // PM gets framed rather than merely raised.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenPmAgent {
            bounds: Some(canvas_bounds()),
        },
    );

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "the launcher must produce events");
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "the launcher started the PM pane");
    assert_eq!(windows[0].preset, WindowPreset::Agent);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1
    );
}

/// SPEC-3431 FR-026: the auto-start opt-out governs the NEXT project open, not
/// the session that is running right now. Stopping the live PM as a side
/// effect of unticking a checkbox would destroy a conversation the user never
/// asked to end.
#[test]
fn set_pm_auto_start_persists_and_does_not_stop_a_live_pm() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetPmAutoStart { enabled: false },
    );

    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load prefs");
    assert!(!prefs.settings.auto_start, "the opt-out must persist");
    assert!(
        prefs.registration.is_some(),
        "the live PM keeps its registration"
    );
    assert!(
        runtime.live_pm_window_id("pm-session-live").is_some(),
        "the live PM pane must keep running"
    );
    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .iter()
            .any(|window| window.id == "agent-1"),
        "the PM window must not be closed by a settings write"
    );

    // The panel is driven by pm_status; a write that does not broadcast leaves
    // the UI showing the old value until some unrelated event arrives.
    let status = events
        .iter()
        .find_map(|outbound| match &outbound.event {
            BackendEvent::PmStatus {
                auto_start,
                is_running,
                ..
            } => Some((*auto_start, *is_running)),
            _ => None,
        })
        .expect("the settings write must broadcast pm_status");
    assert_eq!(status, (false, true), "status mirrors prefs + live pane");
}

#[test]
fn set_pm_loop_interval_rejects_below_floor_without_mutation_or_status() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime.active_agent_sessions.insert(window_id, session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");
    let before = fs::read(&prefs_path).expect("read seeded prefs");

    for rejected in [0, 9] {
        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SetPmLoopInterval {
                loop_interval_secs: rejected,
            },
        );

        assert_eq!(
            fs::read(&prefs_path).expect("read rejected prefs"),
            before,
            "a rejected backend value must leave pm.json byte-identical"
        );
        assert!(
            events
                .iter()
                .all(|outbound| !matches!(outbound.event, BackendEvent::PmStatus { .. })),
            "a rejected write must not broadcast a misleading committed status"
        );
    }
    assert!(
        runtime.live_pm_window_id("pm-session-live").is_some(),
        "rejection must not touch the live PM pane"
    );
}

#[test]
fn set_pm_loop_interval_accepts_minimum_and_preserves_live_pm() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.settings.auto_start = false;
        prefs.settings.launch_profile = Some(gwt::pm_registry::PmLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.1-codex-max".to_string()),
            reasoning: Some("high".to_string()),
            version: None,
        });
    })
    .expect("seed non-target PM prefs");
    let prefs_before = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load seeded prefs");
    let windows_before = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect::<Vec<_>>();

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetPmLoopInterval {
            loop_interval_secs: 10,
        },
    );

    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("reload committed prefs");
    assert_eq!(prefs.settings.loop_interval_secs, 10);
    let mut expected_prefs = prefs_before;
    expected_prefs.settings.loop_interval_secs = 10;
    assert_eq!(
        prefs, expected_prefs,
        "the interval mutation must preserve auto-start, profile, and registration"
    );
    let status = events
        .iter()
        .find_map(|outbound| match (&outbound.target, &outbound.event) {
            (
                DispatchTarget::Project(key),
                BackendEvent::PmStatus {
                    loop_interval_secs,
                    is_running,
                    ..
                },
            ) if Some(key) == runtime.project_key_for_tab("tab-1") => {
                Some((*loop_interval_secs, *is_running))
            }
            _ => None,
        })
        .expect("a committed write must broadcast pm_status");
    assert_eq!(status, (10, true));
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "write must not restart PM"
    );
    assert_eq!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .iter()
            .map(|window| window.id.clone())
            .collect::<Vec<_>>(),
        windows_before,
        "write must keep the live pane unchanged"
    );
    assert_eq!(
        runtime
            .active_agent_sessions
            .get(&window_id)
            .map(|session| session.session_id.as_str()),
        Some("pm-session-live")
    );
    let state_files = fs::read_dir(prefs_path.parent().expect("project-state dir"))
        .expect("read project-state")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert!(
        state_files
            .iter()
            .all(|name| name == "pm.json" || name == "pm.lock"),
        "the shared durable prefs writer must clean up scratch files: {state_files:?}"
    );
}

/// SPEC-3431 FR-026: only agents with a `gwt-pm` skills mirror can resolve the
/// `$gwt-pm` bootstrap prompt. Persisting an unsupported one would hand the PM
/// a prompt that resolves to nothing, so the write is refused outright rather
/// than silently falling back at launch time.
#[test]
fn set_pm_launch_profile_rejects_an_unsupported_agent() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetPmLaunchProfile {
            agent_id: "gemini".to_string(),
            model: Some("gemini-3-pro".to_string()),
            reasoning: None,
        },
    );

    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load prefs");
    assert_eq!(
        prefs.settings.launch_profile, None,
        "an agent without a gwt-pm mirror must never be persisted"
    );

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetPmLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.1-codex-max".to_string()),
            reasoning: Some("high".to_string()),
        },
    );

    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load prefs");
    let profile = prefs
        .settings
        .launch_profile
        .expect("a supported agent is persisted");
    assert_eq!(profile.agent_id, "codex");
    assert_eq!(profile.model.as_deref(), Some("gpt-5.1-codex-max"));
    assert_eq!(profile.reasoning.as_deref(), Some("high"));
    // A profile change is not a stop: the running conversation continues until
    // the user explicitly restarts.
    assert!(
        runtime.live_pm_window_id("pm-session-live").is_some(),
        "changing the profile must not touch the running pane"
    );
}

/// SPEC-3431 FR-026: a restart swaps the agent, so it must end the old pane and
/// bring a new one up — but the PM worktree holds the PM's own notes, and an
/// intentional close reaps a clean one. The restart path must keep it.
#[test]
fn restart_pm_agent_keeps_the_worktree_and_respawns() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);

    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    fs::create_dir_all(pm_worktree.parent().expect("parent")).expect("pm dir");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            pm_worktree.to_str().expect("pm worktree path"),
        ],
    );

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &pm_worktree),
        |_| false,
    )
    .expect("seed registration");

    runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::RestartPmAgent);

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "restart must produce events");
    assert!(
        pm_worktree.exists(),
        "the PM worktree (and its notes) must survive a restart"
    );
    assert!(
        runtime.live_pm_window_id("pm-session-live").is_none(),
        "the old PM pane is gone"
    );
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "exactly one PM pane after the restart");
    assert_eq!(windows[0].preset, WindowPreset::Agent);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "the respawn registers the successor session at launch completion"
    );
    // The surviving pane is the freshly spawned one, not the closed pane left
    // behind: it is the window the pending PM launch is tracking.
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .contains_key(&crate::runtime_support::combined_window_id(
                "tab-1",
                &windows[0].id
            )),
        "the pane on the canvas must be the restart's new spawn"
    );
    // FR-026: the panel is told the PM came back.
    assert!(
        events
            .iter()
            .any(|outbound| matches!(outbound.event, BackendEvent::PmStatus { .. })),
        "the restart must broadcast pm_status"
    );
}
