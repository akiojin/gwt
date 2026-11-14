use super::*;

fn request(action: &str) -> FrontendEvent {
    serde_json::from_value(serde_json::json!({
        "kind": "maintain_supported_agent", "agent_id": "codex", "action": action
    }))
    .expect("supported agents expose maintenance requests")
}

#[test]
fn maintenance_refuses_starting_agent_panes_without_scheduling_a_worker() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent",
        WindowPreset::Codex,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let events = runtime.handle_frontend_event("settings".into(), request("update"));
    let result = events
        .iter()
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .find(|event| event["kind"] == "supported_agent_maintenance")
        .expect("refusal response");
    assert_eq!(result["success"], false);
    assert!(result["message"].as_str().unwrap().contains("agent"));
    assert!(tasks.lock().unwrap().is_empty());
}

#[test]
fn maintenance_reserves_admission_before_the_worker_and_refuses_agent_launches() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let events = runtime.handle_frontend_event("settings".into(), request("update"));
    assert!(events
        .iter()
        .any(|event| serde_json::to_value(&event.event).unwrap()["pending"] == true));
    assert_eq!(tasks.lock().unwrap().len(), 1);
    let error = runtime
        .spawn_agent_window(
            "tab-1",
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex).build(),
            canvas_bounds(),
            None,
        )
        .unwrap_err();
    assert!(error.contains("maintenance"));
    let legacy = runtime.start_window("tab-1", "agent", WindowPreset::Codex, canvas_bounds());
    assert!(legacy.iter().any(|event| serde_json::to_value(&event.event)
        .unwrap()
        .to_string()
        .contains("maintenance")));
    assert!(runtime.runtimes.is_empty());
}

#[test]
fn automatic_update_setting_is_saved_without_starting_maintenance() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let before_save_generation = runtime.agent_maintenance.catalog_generation;
    let old_catalog = || BackendEvent::SupportedAgentList {
        agents: Vec::new(),
        auto_update: false,
        maintenance_pending: false,
    };
    let event =
        serde_json::from_value(serde_json::json!({"kind":"set_agent_auto_update", "enabled":true}))
            .expect("auto-update request");
    runtime.handle_frontend_event("settings".into(), event);
    let settings = Settings::load_from_path(&runtime.profile_config_path().unwrap()).unwrap();
    let saved = toml::to_string(&settings).unwrap();
    assert!(saved.contains("auto_update = true"));
    assert!(!settings.agent.auto_install_deps);
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "only the refreshed Settings catalog is queued"
    );
    assert!(
        runtime.runtimes.is_empty(),
        "enabling takes effect next startup"
    );
    let delayed = runtime.handle_supported_agent_catalog_ready(
        "settings".into(),
        before_save_generation,
        old_catalog(),
    );
    assert_eq!(
        serde_json::to_value(&delayed[0].event).unwrap()["auto_update"],
        true,
        "a pre-save catalog must not overwrite a successful save"
    );
    let completed = runtime.handle_supported_agent_maintenance_complete(
        super::super::agent_maintenance::AgentMaintenanceCompletion {
            results: Vec::new(),
            cache: None,
            catalog: Some(old_catalog()),
        },
    );
    assert_eq!(
        serde_json::to_value(&completed[0].event).unwrap()["auto_update"],
        true,
        "a maintenance catalog must retain the latest successful save"
    );
    runtime.profile_config_path = Some(temp.path().to_path_buf());
    let failed = runtime.handle_frontend_event(
        "settings".into(),
        FrontendEvent::SetAgentAutoUpdate { enabled: false },
    );
    assert!(
        failed.iter().any(|event| matches!(
            &event.event,
            BackendEvent::SupportedAgentMaintenance {
                agent_id, success: Some(false), message, ..
            } if agent_id.is_empty() && message.contains("automatic")
        )),
        "save failure must be visible in Supported Agents"
    );
}

#[test]
fn maintenance_refuses_live_idle_waiting_pm_agents_and_pending_launches() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "pm",
        WindowPreset::Codex,
        WindowProcessStatus::Idle,
    );
    let mut persisted = tab.workspace.persisted().clone();
    persisted.windows[0].is_pm = true;
    tab.workspace = WindowCanvasState::from_persisted(persisted);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let id = combined_window_id("tab-1", "pm");
    insert_test_pane_runtime(&mut runtime, &id);
    for status in [WindowProcessStatus::Idle, WindowProcessStatus::Waiting] {
        runtime.set_window_status("tab-1", "pm", status);
        assert!(
            runtime.agent_maintenance_rejection().is_some(),
            "live {status:?} PM agent"
        );
    }
    runtime.stop_window_runtime(&id);
    runtime
        .inflight_launches
        .insert("pending".into(), ("next-agent".into(), Instant::now()));
    assert!(runtime.agent_maintenance_rejection().is_some());
    runtime.inflight_launches.clear();
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pending_launch_wizard_materializations
        .insert(
            "confirmed-wizard".into(),
            sample_launch_wizard_session("tab-1", temp.path()),
        );
    assert!(
        runtime.agent_maintenance_rejection().is_some(),
        "confirmed wizard preparation owns launch admission"
    );
}

#[test]
fn agent_maintenance_interlock_holds_self_update_quiescence() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.agent_maintenance.busy = true;
    let (panes, _) = runtime.capture_update_quiescence_inputs();
    let blockers =
        gwt::update_drain::update_quiescence(&gwt::update_drain::UpdateQuiescenceSnapshot {
            panes,
            ..Default::default()
        });
    assert!(
        blockers.is_err(),
        "the installer process must hold self-update quiescence"
    );
}

#[test]
fn maintenance_worker_admission_failure_releases_the_launch_guard() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.blocking_tasks = BlockingTaskSpawner::failing("worker unavailable");
    let events = runtime.handle_frontend_event("settings".into(), request("install"));
    assert!(!runtime.agent_maintenance.busy);
    assert!(events
        .iter()
        .any(|event| serde_json::to_value(&event.event).unwrap()["success"] == false));
}

#[cfg(unix)]
fn startup_fixture(
    root: &Path,
    installed: Option<&str>,
    latest: &str,
    fail_update: bool,
) -> (
    AppRuntime,
    Arc<Mutex<Vec<UserEvent>>>,
    BlockingTestTaskQueue,
) {
    use std::os::unix::fs::PermissionsExt;
    let (mut runtime, events) = sample_runtime_with_events(root, Vec::new(), None);
    let bin = root.join("agent-bin");
    fs::create_dir(&bin).unwrap();
    let codex = bin.join("codex");
    if let Some(version) = installed {
        fs::write(
            &codex,
            format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\n"),
        )
        .unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let npm = bin.join("npm");
    let install = if fail_update {
        "exit 7\n".to_string()
    } else {
        format!("printf '#!/bin/sh\\nprintf \"codex-cli {latest}\\\\n\"\\n' > \"$base/codex\"\n/bin/chmod +x \"$base/codex\"\n")
    };
    fs::write(&npm, format!("#!/bin/sh\nbase=${{0%/*}}\nprintf '%s\\n' \"$*\" >> \"$base/calls\"\nif [ \"$1\" = view ]; then\n printf '\"{latest}\"\\n'\nelse\n {install}fi\n")).unwrap();
    fs::set_permissions(&npm, fs::Permissions::from_mode(0o755)).unwrap();
    let mut settings = Settings::default();
    settings.agent.auto_update = true;
    settings.profiles.profiles[0]
        .env_vars
        .insert("PATH".into(), bin.to_string_lossy().into_owned());
    settings
        .save(&runtime.profile_config_path().unwrap())
        .unwrap();
    runtime.launch_wizard_cache =
        LaunchWizardMemoryCache::load_with_agent_options(&runtime.sessions_dir, Vec::new());
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    (runtime, events, tasks)
}

#[cfg(unix)]
fn finish_startup_worker(
    runtime: &mut AppRuntime,
    events: &Arc<Mutex<Vec<UserEvent>>>,
    tasks: &BlockingTestTaskQueue,
) -> Vec<OutboundEvent> {
    drain_queued_blocking_tasks(tasks);
    let completion = {
        let mut events = events.lock().unwrap();
        let index = events
            .iter()
            .position(|event| matches!(event, UserEvent::SupportedAgentMaintenanceComplete(_)))
            .expect("maintenance completion");
        let UserEvent::SupportedAgentMaintenanceComplete(completion) = events.remove(index) else {
            unreachable!()
        };
        completion
    };
    runtime.handle_supported_agent_maintenance_complete(*completion)
}

#[cfg(unix)]
#[test]
fn startup_updates_only_installed_npm_agents_and_refreshes_catalog_after_completion() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, tasks) = startup_fixture(temp.path(), Some("1.2.3"), "1.2.4", false);
    runtime.start_agent_auto_update();
    assert!(
        runtime.agent_maintenance.busy,
        "startup reserves admission before its worker"
    );
    let outbound = finish_startup_worker(&mut runtime, &events, &tasks);
    assert!(!runtime.agent_maintenance.busy);
    let calls = fs::read_to_string(temp.path().join("agent-bin/calls")).unwrap();
    let installs: Vec<_> = calls
        .lines()
        .filter(|line| line.starts_with("install"))
        .collect();
    assert_eq!(installs, ["install -g @openai/codex@1.2.4"]);
    let payload = outbound
        .iter()
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .find(|event| event["kind"] == "supported_agent_list")
        .unwrap();
    let codex = payload["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == "codex")
        .unwrap();
    assert_eq!(codex["installed_version"], "codex-cli 1.2.4");
    assert_eq!(codex["up_to_date"], true);
    assert_eq!(codex["update_available"], false);
    assert_eq!(payload["auto_update"], true);
}

#[cfg(unix)]
#[test]
fn startup_does_not_install_missing_current_or_unknown_version_agents_and_failure_releases_guard() {
    for (installed, fail_update) in [
        (None, false),
        (Some("1.2.4"), false),
        (Some("unknown"), false),
        (Some("1.2.3"), true),
    ] {
        let temp = tempdir().unwrap();
        let _gwt_home = ScopedGwtHome::set(temp.path());
        let (mut runtime, events, tasks) =
            startup_fixture(temp.path(), installed, "1.2.4", fail_update);
        runtime.start_agent_auto_update();
        let outbound = finish_startup_worker(&mut runtime, &events, &tasks);
        assert!(!runtime.agent_maintenance.busy);
        assert!(runtime.agent_maintenance_rejection().is_none());
        let calls = fs::read_to_string(temp.path().join("agent-bin/calls")).unwrap_or_default();
        assert_eq!(
            calls.lines().any(|line| line.starts_with("install")),
            fail_update
        );
        if fail_update {
            assert!(outbound
                .iter()
                .any(|event| serde_json::to_value(&event.event).unwrap()["success"] == false));
        }
    }
}

#[test]
fn startup_disabled_or_pending_launch_does_not_schedule_updates() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.start_agent_auto_update();
    assert!(tasks.lock().unwrap().is_empty());
    let path = runtime.profile_config_path().unwrap();
    let mut settings = Settings::load_from_path(&path).unwrap();
    settings.agent.auto_update = true;
    settings.save(&path).unwrap();
    runtime
        .inflight_launches
        .insert("pending".into(), ("next-agent".into(), Instant::now()));
    runtime.start_agent_auto_update();
    assert!(tasks.lock().unwrap().is_empty());
    assert!(!runtime.agent_maintenance.busy);
}

#[test]
fn startup_canvas_ready_preserves_pm_queue_until_maintenance_finishes() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "paused",
        temp.path().to_path_buf(),
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    tab.kind = ProjectKind::NonRepo;
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.agent_maintenance.busy = true;
    runtime.pending_startup_pm_tabs.push("tab-1".into());
    runtime.pending_startup_auto_resume_sessions.push(
        super::super::PendingStartupAutoResumeSession {
            tab_id: "tab-1".into(),
            session: gwt_agent::Session::new(
                temp.path().join("missing-worktree"),
                "feature/restore",
                gwt_agent::AgentId::Codex,
            ),
            workspace_resume_context: None,
        },
    );
    let immediate = runtime.handle_frontend_event(
        "canvas".into(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );
    assert!(immediate.is_empty());
    assert_eq!(runtime.pending_startup_pm_tabs, ["tab-1"]);
    assert_eq!(runtime.pending_startup_auto_resume_sessions.len(), 1);
    assert!(
        runtime
            .tab("tab-1")
            .unwrap()
            .workspace
            .window("paused")
            .is_some(),
        "the paused restore placeholder must survive maintenance"
    );
    assert!(runtime.agent_maintenance.deferred_startup_bounds.is_some());
    runtime.handle_supported_agent_maintenance_complete(
        super::super::agent_maintenance::AgentMaintenanceCompletion {
            results: Vec::new(),
            cache: None,
            catalog: None,
        },
    );
    assert!(runtime.pending_startup_pm_tabs.is_empty());
    assert!(
        runtime.pending_startup_auto_resume_sessions.is_empty(),
        "completion replays the deferred restore drain"
    );
    assert!(runtime.agent_maintenance.deferred_startup_bounds.is_none());
}

#[test]
fn supported_agent_catalog_drops_out_of_order_snapshots_and_reconciles_pending() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let catalog = || BackendEvent::SupportedAgentList {
        agents: Vec::new(),
        auto_update: false,
        maintenance_pending: false,
    };
    runtime.spawn_supported_agent_list("settings-a".into());
    let first_client_generation = runtime.agent_maintenance.catalog_generation;
    runtime.spawn_supported_agent_list("settings-b".into());
    let current = runtime.agent_maintenance.catalog_generation;
    assert_eq!(
        runtime
            .handle_supported_agent_catalog_ready("settings-b".into(), current, catalog())
            .len(),
        1
    );
    assert_eq!(
        runtime
            .handle_supported_agent_catalog_ready(
                "settings-a".into(),
                first_client_generation,
                catalog()
            )
            .len(),
        1,
        "another client request must not invalidate the first client's reply"
    );
    runtime.handle_frontend_event("settings".into(), request("install"));
    assert!(
        runtime.agent_maintenance.catalog_generation > current,
        "reservation invalidates pre-install snapshots"
    );
    assert!(
        runtime
            .handle_supported_agent_catalog_ready("settings-b".into(), current, catalog())
            .is_empty(),
        "a pre-install catalog must not replace the maintenance snapshot"
    );
    runtime.spawn_supported_agent_list("reconnected-settings".into());
    let during_maintenance = runtime.agent_maintenance.catalog_generation;
    let pending = runtime.handle_supported_agent_catalog_ready(
        "reconnected-settings".into(),
        during_maintenance,
        catalog(),
    );
    assert_eq!(
        serde_json::to_value(&pending[0].event).unwrap()["maintenance_pending"],
        true
    );
    let complete = runtime.handle_supported_agent_maintenance_complete(
        super::super::agent_maintenance::AgentMaintenanceCompletion {
            results: Vec::new(),
            cache: None,
            catalog: Some(catalog()),
        },
    );
    assert_eq!(
        serde_json::to_value(&complete[0].event).unwrap()["maintenance_pending"],
        false
    );
    assert!(
        runtime
            .handle_supported_agent_catalog_ready(
                "reconnected-settings".into(),
                during_maintenance,
                catalog()
            )
            .is_empty(),
        "completion invalidates snapshots captured while the installer was active"
    );
}
