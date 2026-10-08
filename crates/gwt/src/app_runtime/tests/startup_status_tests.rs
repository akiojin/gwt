use super::*;

/// Issue #4584 AC-1/AC-4: the regression this Issue exists for. A turn ended
/// on a provider API error and the pane kept reporting `running`.
///
/// Unlike the quota path there is no settle window. A quota hold releases the
/// Monitor's slot, so a false positive throws away a live launch and has to be
/// paid for with a wait. This changes only what the pane reports, and the
/// detector re-runs on every output chunk — so a pane that is genuinely still
/// working clears itself on its very next write. The state can only persist
/// when output has actually stopped, which is the condition being reported.
#[test]
fn a_provider_api_error_stops_the_pane_reading_as_running() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = api_error_live_runtime(temp.path());

    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running),
        "precondition: the pane reads as working before the error"
    );

    let _ = runtime.observe_provider_api_error(&window_id, Some(CLAUDE_API_ERROR_SCREEN));

    assert_ne!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running),
        "a pane whose turn died on an API error must not read as working"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Waiting),
        "it is waiting for something from outside, like a quota-blocked pane"
    );
}

/// AC-2: the reason has to travel with the state, or the reader is back to
/// opening panes one at a time — which is what cost the twenty minutes.
#[test]
fn the_api_error_hold_carries_the_status_and_the_message() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = api_error_live_runtime(temp.path());

    let _ = runtime.observe_provider_api_error(&window_id, Some(CLAUDE_API_ERROR_SCREEN));

    let hold = runtime
        .provider_api_error_holds
        .get(&window_id)
        .expect("the hold records what stopped the pane");
    assert_eq!(hold.http_status, Some(529));
    assert!(hold.summary.contains("Overloaded"));

    let detail = runtime
        .window_details
        .get(&window_id)
        .expect("the pane must say why it is waiting");
    assert!(
        detail.contains("529"),
        "detail carries the status: {detail}"
    );
    assert!(
        detail.contains("Overloaded"),
        "detail carries the message: {detail}"
    );
}

/// The self-clearing property the design leans on. Without it a pane that
/// recovered would stay mislabelled — the same defect pointing the other way.
#[test]
fn resumed_output_releases_the_api_error_hold() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = api_error_live_runtime(temp.path());

    let _ = runtime.observe_provider_api_error(&window_id, Some(CLAUDE_API_ERROR_SCREEN));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Waiting)
    );

    let _ = runtime.observe_provider_api_error(
        &window_id,
        Some("⏺ Resuming. Re-running the failing test now.\n❯\n"),
    );

    assert!(!runtime.provider_api_error_holds.contains_key(&window_id));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running),
        "the pane must go back to reporting the work it resumed"
    );
    assert!(
        !runtime
            .window_details
            .get(&window_id)
            .is_some_and(|detail| detail.contains("529")),
        "a released hold must not leave its reason behind"
    );
}

/// AC-1/AC-2 end to end: the canvas snapshot is what the Issue Monitor joins
/// into its status rows, so the state and the reason have to survive the trip
/// out of the runtime, not just exist inside it.
#[test]
fn the_window_snapshot_carries_the_api_error_state_and_reason() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = api_error_live_runtime(temp.path());

    let observed = |runtime: &AppRuntime| {
        runtime
            .issue_monitor_window_snapshot_for_tab("tab-1", "2026-09-22T01:13:00Z")
            .expect("snapshot")
            .windows
            .into_iter()
            .find(|observed| observed.window_id == window_id)
            .expect("the window is on the canvas")
    };

    let healthy = observed(&runtime);
    assert_eq!(healthy.status, WindowProcessStatus::Running);
    assert_eq!(
        healthy.hold_reason, None,
        "a working pane must not claim to be held"
    );

    let _ = runtime.observe_provider_api_error(&window_id, Some(CLAUDE_API_ERROR_SCREEN));

    let held = observed(&runtime);
    assert_eq!(
        held.status,
        WindowProcessStatus::Waiting,
        "the state the Monitor joins must be the corrected one"
    );
    let reason = held.hold_reason.expect("the snapshot names the cause");
    assert!(reason.contains("529"), "{reason}");
    assert!(reason.contains("Overloaded"), "{reason}");
}

/// Quota exhaustion is the more specific diagnosis, is corroborated by the
/// usage poller, and carries a reset instant. It must not be displaced by the
/// weaker reading.
#[test]
fn a_quota_hold_outranks_an_api_error_on_the_same_pane() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:20:00Z"),
    );
    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:25:00Z"),
    );
    assert!(runtime.provider_quota_holds.contains_key(&window_id));

    let _ = runtime.observe_provider_api_error(&window_id, Some(CLAUDE_API_ERROR_SCREEN));

    assert!(
        !runtime.provider_api_error_holds.contains_key(&window_id),
        "the quota hold already explains this pane"
    );
    assert!(
        runtime
            .window_details
            .get(&window_id)
            .is_some_and(|detail| detail.contains("usage limit")),
        "the quota reason must survive"
    );
}

/// Issue #3616: Claude does not exit when its quota runs out — the pane stays
/// alive showing the notice and stops responding, which reads as a healthy
/// `idle` agent.
///
/// A notice that has only just appeared is not yet a block: the settle window
/// exists so a pane that merely *rendered* the sentence (an agent working on
/// this very Issue does) is not released out from under itself.
#[test]
fn a_live_quota_notice_becomes_a_hold_only_after_the_settle_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:20:00Z"),
    );
    assert!(
        !runtime.provider_quota_holds.contains_key(&window_id),
        "a freshly rendered notice must not release a live launch"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Idle),
        "the pane keeps its own state until the block is confirmed"
    );

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:25:00Z"),
    );

    assert!(
        runtime.provider_quota_holds.contains_key(&window_id),
        "a notice that persists without any agent activity is a block"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Waiting),
        "a live but quota-blocked pane must not read as a healthy idle agent"
    );
}

/// Issue #3616: a hook arrival proves the agent is working, so the candidate is
/// abandoned. This is what keeps an agent that renders the sentence mid-task
/// from ever being released.
#[test]
fn agent_activity_abandons_a_pending_quota_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:20:00Z"),
    );
    runtime.released_provider_quota_notices.insert(
        window_id.clone(),
        gwt_core::usage::detect_provider_limit_notice(
            CLAUDE_USAGE_LIMIT_SCREEN,
            &chrono::Local::now(),
        )
        .unwrap(),
    );

    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));
    assert!(!runtime
        .released_provider_quota_notices
        .contains_key(&window_id));
    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:25:00Z"),
    );

    assert!(
        !runtime.provider_quota_holds.contains_key(&window_id),
        "the agent ran a tool after the notice appeared, so it is not blocked"
    );
}

/// Issue #3616: the notice leaving the screen abandons the candidate too.
#[test]
fn a_redrawn_screen_abandons_a_pending_quota_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:20:00Z"),
    );
    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some("> running cargo test\ncompiling gwt v9.81.0"),
        instant("2026-08-17T09:21:00Z"),
    );
    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:25:00Z"),
    );

    assert!(
        !runtime.provider_quota_holds.contains_key(&window_id),
        "the settle clock restarts when the notice leaves the screen"
    );
}

/// Issue #3616: when the usage poller independently reports the account out of
/// quota, the settle window is unnecessary — two independent sources agree, so
/// waiting only prolongs a held slot.
#[test]
fn a_corroborated_quota_notice_holds_immediately() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");
    runtime.set_provider_usage_accounts(vec![gwt_core::usage::ProviderUsage {
        provider: gwt_core::usage::UsageProvider::ClaudeCode,
        account_id: None,
        account_label: None,
        plan: None,
        windows: vec![gwt_core::usage::UsageWindow::new(
            gwt_core::usage::WindowKind::Weekly,
            100.0,
            Some(instant("2026-08-19T21:00:00Z")),
        )],
        limit_reached: true,
        state: gwt_core::usage::UsageState::Ok,
        fetched_at: None,
    }]);

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CLAUDE_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:20:00Z"),
    );

    assert!(
        runtime.provider_quota_holds.contains_key(&window_id),
        "the poller already confirmed the account is out; no settle window is needed"
    );
}

/// Issue #3616: the usage-poller tick is what reaches a pending candidate's
/// settle deadline.
///
/// A blocked pane emits no further output, so the output path alone would leave
/// the candidate pending forever — the launch would keep its slot for the whole
/// multi-day window, which is the harm this Issue is about.
#[test]
fn the_usage_snapshot_tick_promotes_a_settled_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");
    runtime.provider_quota_candidates.insert(
        window_id.clone(),
        super::super::ProviderQuotaCandidate {
            first_seen: instant("2026-08-17T09:20:00Z"),
        },
    );

    // No screen is readable for a test window, so a pending candidate whose
    // notice cannot be re-confirmed is abandoned rather than promoted.
    let _ = runtime.handle_provider_usage_snapshot(Vec::new(), instant("2026-08-17T09:25:00Z"));

    assert!(
        !runtime.provider_quota_candidates.contains_key(&window_id),
        "a candidate whose notice can no longer be seen must not survive the sweep"
    );
    assert!(
        !runtime.provider_quota_holds.contains_key(&window_id),
        "and it must never be promoted on an unverifiable screen"
    );
}

/// Issue #3616: an exhausted account must not stall panes running on a
/// different provider.
#[test]
fn a_different_providers_exhaustion_does_not_corroborate_this_pane() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "codex");
    runtime.set_provider_usage_accounts(vec![gwt_core::usage::ProviderUsage {
        provider: gwt_core::usage::UsageProvider::ClaudeCode,
        account_id: None,
        account_label: None,
        plan: None,
        windows: Vec::new(),
        limit_reached: true,
        state: gwt_core::usage::UsageState::Ok,
        fetched_at: None,
    }]);

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-08-17T09:20:00Z"),
    );

    assert!(
        runtime.provider_quota_candidates.contains_key(&window_id),
        "precondition: the Codex notice was recognized, so this is the scoping check"
    );
    assert!(
        !runtime.provider_quota_holds.contains_key(&window_id),
        "Claude running out says nothing about this Codex pane, so the settle \
         window must still apply"
    );
}

/// Issue #3616: the whole chain against a real PTY, from bytes on a vt100
/// screen to the pane state a client renders.
///
/// Every other test in this group injects the screen as a string, so the one
/// seam they cannot cover is the one that failed in production: reading the
/// provider's words off a live pane. The notice is written by a real child
/// process through a real pty, wrapped by the terminal exactly as a provider
/// CLI's output would be.
#[cfg(unix)]
#[test]
fn a_real_pty_rendering_the_notice_puts_its_pane_into_waiting() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "claude");

    // The exact Claude wording observed on 2026-08-17, printed by a child that
    // then stays alive and unresponsive — the shape the exit path cannot see.
    // Issue #5178: require acknowledgement between writes to force a partial
    // PTY read without relying on the child or parent being scheduled first.
    let script = "printf '\\n> read the file\\n\\n'; \
         printf \"You've hit your weekly limit \\302\\267 resets Aug 20 at 6am (Asia/Tokyo)\\n\"; \
         read -r release; \
         printf '/usage-credits to finish what you are working on.\\n'; \
         sleep 30";
    let pane = Pane::new(
        window_id.clone(),
        "/bin/sh".to_string(),
        vec!["-c".to_string(), script.to_string()],
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("quota notice pane");
    let pane = Arc::new(Mutex::new(pane));
    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            pane.clone(),
        ),
    );

    // Pump the pty into the vt100 parser the way the production output thread
    // does, until the child's notice is on screen.
    let mut reader = pane
        .lock()
        .expect("pane lock")
        .reader()
        .expect("pty reader");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let first = instant("2026-08-17T09:20:00Z");
    let mut released = false;
    let mut rendered = false;
    let mut buffer = [0_u8; 4096];
    while std::time::Instant::now() < deadline {
        let read = std::io::Read::read(&mut reader, &mut buffer).expect("pty read");
        if read == 0 {
            break;
        }
        let screen = {
            let mut locked = pane.lock().expect("pane lock");
            locked.process_bytes(&buffer[..read]);
            locked.screen().contents()
        };
        if !released && screen.contains("weekly limit") {
            let _ = runtime.observe_provider_quota_notice_from_screen(&window_id, first);
            assert!(
                !runtime.provider_quota_candidates.contains_key(&window_id)
                    && !runtime.provider_quota_holds.contains_key(&window_id),
                "a heading without its native refusal footer must not form a quota candidate or hold"
            );
            pane.lock()
                .expect("pane lock")
                .write_input(b"\n")
                .expect("release quota notice footer");
            released = true;
        }
        if screen.contains("/usage-credits to finish what you are working on.") {
            rendered = true;
            break;
        }
    }
    assert!(
        rendered,
        "precondition: the child's complete notice must reach the vt100 screen"
    );

    let _ = runtime.observe_provider_quota_notice_from_screen(&window_id, first);
    assert!(
        !runtime.provider_quota_holds.contains_key(&window_id),
        "the settle window still applies to a real pane"
    );

    let settled = first + chrono::Duration::seconds(300);
    let _ = runtime.observe_provider_quota_notice_from_screen(&window_id, settled);

    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Waiting),
        "a real pane showing a real notice must render as waiting, not idle; screen: {:?}",
        pane.lock().expect("pane lock").screen().contents()
    );
    let hold = runtime
        .provider_quota_holds
        .get(&window_id)
        .expect("the hold was recorded");
    let gwt::IssueMonitorFailure::ProviderUsageLimit {
        provider,
        resets_at,
        ..
    } = hold
    else {
        panic!("expected a provider usage limit hold, got {hold:?}");
    };
    assert_eq!(provider, "claude");
    // The provider prints a bare local wall clock, so production resolves it in
    // the machine's zone. Deriving the expectation the same way keeps this exact
    // without asserting the machine is in any particular zone — the first
    // version hard-coded the author's +09:00 and failed on a UTC CI runner.
    let expected_reset = chrono::Local
        .with_ymd_and_hms(2026, 8, 20, 6, 0, 0)
        .earliest()
        .expect("6am on Aug 20 exists in the local zone")
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    assert_eq!(
        resets_at.as_deref(),
        Some(expected_reset.as_str()),
        "the reset instant survives the pty round trip"
    );
    assert!(
        runtime
            .window_details
            .get(&window_id)
            .is_some_and(|detail| detail.contains("usage limit")),
        "the pane must say why it is waiting"
    );

    runtime.stop_window_runtime(&window_id);
}

/// Issue #3616: an ordinary clean exit is untouched by the quota path.
#[test]
fn an_ordinary_clean_agent_exit_still_reaches_the_stopped_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "Stop",
        "session-1",
    ));

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
        true,
    );

    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped)
    );
}

#[test]
fn app_runtime_live_hook_recovery_clears_recoverable_pty_error_marker() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));
    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );
    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );
    assert!(runtime.recoverable_agent_error_windows.contains(&window_id));

    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    assert!(
        !runtime.recoverable_agent_error_windows.contains(&window_id),
        "live hook recovery must end the stale PTY Error duplicate window"
    );
    runtime.window_hook_states.remove(&window_id);

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("process exited".to_string()),
        true,
    );

    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
}

#[test]
fn app_runtime_active_work_projection_filters_stale_saved_agents_when_no_agent_is_live() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.status_text = "Old agent is running".to_string();
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "stale-session".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Old focus".to_string()),
            title_summary: None,
            worktree_path: None,
            branch: Some("work/old".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save stale projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_agents, 0);
    assert_eq!(view.blocked_agents, 0);
    assert!(view.agents.is_empty());
    assert_eq!(view.status_category, "idle");
    assert_eq!(view.status_text, "No active work");
}

#[test]
fn app_runtime_active_work_projection_resets_stale_current_identity_when_no_agent_is_live() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "PR-2525".to_string();
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.status_text = "Old PR is active".to_string();
    projection.summary = Some("Old PR summary".to_string());
    projection.owner = Some("PR-2525".to_string());
    projection.next_action = Some("Review old PR".to_string());
    projection.board_refs = vec!["board-old".to_string()];
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
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "stale-session".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Old focus".to_string()),
            title_summary: Some("Old title".to_string()),
            worktree_path: None,
            branch: Some("work/old".to_string()),
            last_board_entry_id: Some("board-old".to_string()),
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save stale projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.title, "Repo Work");
    assert_eq!(view.status_category, "idle");
    assert_eq!(view.status_text, "No active work");
    assert_eq!(view.summary, None);
    assert_eq!(view.owner, None);
    assert_eq!(view.next_action, None);
    assert_eq!(view.branch, None);
    assert_eq!(view.worktree_path, None);
    assert_eq!(view.pr_number, None);
    assert!(view.board_refs.is_empty());
}

#[test]
fn app_runtime_active_work_projection_filters_stale_agent_when_window_id_is_reused() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let window_id = "tab-1::agent-1";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.status_text = "Old agent is running".to_string();
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "stale-session".to_string(),
            window_id: Some(window_id.to_string()),
            agent_id: "codex".to_string(),
            display_name: "Old Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Old focus".to_string()),
            title_summary: Some("Old title".to_string()),
            worktree_path: None,
            branch: Some("work/old".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save stale projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        window_id.to_string(),
        ActiveAgentSession {
            window_id: window_id.to_string(),
            session_id: "live-session".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/live".to_string(),
            display_name: "Live Codex".to_string(),
            worktree_path: repo.join("../repo-work-live"),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_agents, 1);
    assert_eq!(view.blocked_agents, 0);
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].session_id, "live-session");
    assert_eq!(view.agents[0].display_name, "Live Codex");
    assert!(!view
        .agents
        .iter()
        .any(|agent| agent.session_id == "stale-session"));
}

#[test]
fn app_runtime_active_work_projection_includes_recent_workspace_journal_entries() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_core::workspace_projection::update_workspace_projection_with_journal(
        &repo,
        gwt_core::workspace_projection::WorkspaceProjectionUpdate {
            title: Some("Work".to_string()),
            status_category: Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle),
            status_text: Some("Ready for review".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: Some("Review summary".to_string()),
            summary: Some("Overview summary is persisted.".to_string()),
            progress_summary: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        },
    )
    .expect("workspace update");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(
        view.summary.as_deref(),
        Some("Overview summary is persisted.")
    );
    assert_eq!(view.journal_entries.len(), 1);
    assert_eq!(
        view.journal_entries[0].summary.as_deref(),
        Some("Overview summary is persisted.")
    );
    assert_eq!(
        view.journal_entries[0].next_action.as_deref(),
        Some("Review summary")
    );
}

#[test]
fn app_runtime_resume_workspace_journal_reuses_existing_branch_as_execution_container() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "work/20260507-0001";
    run_git(&repo, &["branch", branch]);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-reuse",
        temp.path().join("work").join("20260507-0001"),
        "SPEC-2359",
        "Resume the suspended Work card.",
    );
    assert_eq!(
        super::super::workspace_resume_branch_from_journal_project_root(
            &temp.path().join("work").join("20260507-0001"),
            &repo
        )
        .as_deref(),
        Some(branch)
    );
    assert!(super::super::workspace_resume_branch_exists(&repo, branch));
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-reuse".to_string()),
        },
    );

    let session = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Launch Agent");
    assert_eq!(view.branch_name, branch);
    let context = session
        .workspace_resume_context
        .as_ref()
        .expect("workspace resume context");
    assert_eq!(context.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(
        context.summary.as_deref(),
        Some("Resume the suspended Work card.")
    );
}

#[test]
fn app_runtime_list_resumable_agents_returns_assigned_with_session_toml() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-resumable-1";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        Some("prior-codex-uuid"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            operation_id: "list-operation".to_string(),
            workspace_id: None,
        },
    );

    let event = events
        .first()
        .expect("backend should respond to ListResumableAgents");
    assert!(matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    ));
    match &event.event {
        BackendEvent::WorkspaceResumableAgents {
            operation_id,
            agents,
            ..
        } => {
            assert_eq!(operation_id, "list-operation");
            assert_eq!(agents.len(), 1, "single assigned agent must surface");
            assert_eq!(agents[0].session_id, session_id);
            assert!(matches!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::Session
            ));
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_includes_unassigned_agents_with_session_toml() {
    // SPEC-2359 US-42 follow-up: production projections often store
    // agents with `affiliation_status = unassigned` (no explicit
    // `workspace join` step). Resume Picker must still offer them as
    // candidates when a Session toml is on disk; otherwise users see
    // "No resumable agents" for every Workspace they did not
    // manually ensure / join.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-unassigned-1";
    let mut projection = projection_with_assigned_agent(&repo, session_id);
    projection.agents[0].affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        Some("agent-session-uuid"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            operation_id: "list-operation".to_string(),
            workspace_id: None,
        },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(
                agents.len(),
                1,
                "Unassigned agent with a backing Session toml must still surface",
            );
            assert_eq!(agents[0].session_id, session_id);
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_includes_live_session_as_running() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-live-1";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        Some("agent-session-uuid"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut live = sample_active_agent_session("tab-1", "window-1");
    live.session_id = session_id.to_string();
    runtime
        .active_agent_sessions
        .insert("window-1".to_string(), live);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            operation_id: "list-operation".to_string(),
            workspace_id: None,
        },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(
                agents.len(),
                1,
                "live session should appear with Running status"
            );
            assert_eq!(
                agents[0].lifecycle_status,
                Some(gwt::ResumableAgentLifecycleStatus::Running),
            );
            assert_eq!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::Session
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_marks_idless_codex_as_native_picker() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-native-picker-1";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        None,
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            operation_id: "list-operation".to_string(),
            workspace_id: None,
        },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(agents.len(), 1);
            assert_eq!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::NativePicker,
                "Codex without an exact id should open the provider-native resume picker"
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_uses_workspace_branch_ledger_candidates() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "work/ledger-resume";
    run_git(&repo, &["branch", branch]);

    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection without raw agents");

    let work_id = gwt_core::workspace_projection::canonical_work_id(&repo, Some(branch), None)
        .expect("canonical work id");
    let updated_at = Utc.with_ymd_and_hms(2026, 6, 17, 9, 0, 0).unwrap();
    let work_item = gwt_core::workspace_projection::WorkItem {
        id: work_id.clone(),
        title: branch.to_string(),
        intent: None,
        summary: None,
        progress_summary: None,
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
        owner: None,
        created_at: updated_at,
        updated_at,
        completed_at: None,
        agents: Vec::new(),
        execution_containers: vec![
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch.to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ],
        board_refs: Vec::new(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
        legacy_metadata_snapshot: None,
        legacy_metadata_authoritative: false,
        legacy_metadata_snapshot_at: None,
        duplicate_event_containers: Default::default(),
        discarded: false,
        discarded_at: None,
    };
    let work_items = gwt_core::workspace_projection::WorkItemsProjection {
        updated_at,
        work_items: vec![work_item],
    };
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("work items parent"))
        .expect("create work items parent");
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &work_items,
    )
    .expect("save work items projection");

    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        "session-ledger-codex",
        &repo,
        branch,
        gwt_agent::AgentId::Codex,
        Some("codex-thread-ledger"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            operation_id: "list-operation".to_string(),
            workspace_id: Some(work_id),
        },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(
                agents.len(),
                1,
                "Resume picker must use the same branch ledger candidates as Workspace detail",
            );
            assert_eq!(agents[0].session_id, "session-ledger-codex");
            assert_eq!(agents[0].display_name, "Codex");
            assert_eq!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::Session
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_resume_workspace_agent_replies_error_when_session_toml_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            operation_id: "resume-operation".to_string(),
            session_id: "missing-session".to_string(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    let event = events
        .first()
        .expect("ResumeWorkspaceAgent must reply on missing session");
    assert!(matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    ));
    assert!(matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError {
            operation_id,
            session_id,
            message,
        } if operation_id == "resume-operation"
            && session_id == "missing-session"
            && !message.is_empty()
    ));
}

#[test]
fn app_runtime_resume_workspace_agent_ignores_stopped_same_session_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "stopped-session".to_string();
    session.window_id = window_id.clone();
    runtime.active_agent_sessions.insert(window_id, session);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            operation_id: "resume-operation".to_string(),
            session_id: "stopped-session".to_string(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    let event = events
        .first()
        .expect("ResumeWorkspaceAgent should proceed past stopped window");
    assert!(matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError { session_id, message, .. }
            if session_id == "stopped-session" && !message.is_empty()
    ));
}

#[test]
fn app_runtime_resume_workspace_agent_metadata_only_nonpicker_never_starts_new_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session =
        gwt_agent::Session::new(&repo, "work/metadata-only", gwt_agent::AgentId::Copilot);
    session.id = "session-metadata-only".to_string();
    session.agent_session_id = None;
    session.save(&runtime.sessions_dir).expect("save session");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            operation_id: "resume-operation".to_string(),
            session_id: session.id.clone(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError {
            session_id,
            message,
            ..
        } if session_id == "session-metadata-only"
            && message.contains("saved conversation")
    )));
    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentStarted { .. }
    )));
}

// SPEC-2359 US-79: a Session whose worktree was removed on this machine can
// still be resumed when the branch is available locally/remotely. The launch
// path must be allowed to materialize the worktree again.
#[test]
fn app_runtime_resume_workspace_agent_materializes_missing_worktree_when_branch_exists() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "feature/ghost";
    run_git(&repo, &["branch", branch]);
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let ghost_worktree = temp.path().join("ghost-worktree");
    let mut session = gwt_agent::Session::new(&ghost_worktree, branch, gwt_agent::AgentId::Codex);
    session.id = "session-ghost".to_string();
    session.agent_session_id = Some("conv-ghost".to_string());
    session.save(&runtime.sessions_dir).expect("save session");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            operation_id: "resume-operation".to_string(),
            session_id: "session-ghost".to_string(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::WorkspaceResumeAgentStarted {
                session_id,
                branch: Some(started_branch),
                ..
            }
                if session_id == "session-ghost" && started_branch == branch
        )),
        "branch-materializable missing worktree should enter launch materialization and ack"
    );
    assert!(
        events.iter().all(|event| !matches!(
            &event.event,
            BackendEvent::WorkspaceResumeAgentError { message, .. }
                if message.contains("Worktree path not found")
        )),
        "missing worktree alone must not be a synchronous resume error"
    );
}

#[test]
fn app_runtime_list_resumable_agents_filters_session_when_worktree_and_branch_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let session_id = "session-branch-gone";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &temp.path().join("deleted-worktree"),
        "feature/deleted",
        gwt_agent::AgentId::Codex,
        Some("conv-deleted"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            operation_id: "list-operation".to_string(),
            workspace_id: None,
        },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert!(
                agents.is_empty(),
                "exact Session Resume candidates require an existing worktree or materializable branch"
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

// SPEC-2359 D1: requesting a *specific* past conversation while a live window
// is running a *different* conversation must surface a visible error, not
// silently focus the live window (which would drop the requested resume).
#[test]
fn app_runtime_resume_workspace_agent_errors_when_live_window_runs_other_conversation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = "work-live".to_string();
    active.window_id = window_id.clone();
    runtime.active_agent_sessions.insert(window_id, active);

    // The live window is running "conv-current"; the user clicked Resume on
    // an older conversation ("conv-old").
    let mut session = gwt_agent::Session::new(&repo, "feature/live", gwt_agent::AgentId::Codex);
    session.id = "work-live".to_string();
    session.agent_session_id = Some("conv-current".to_string());
    session.save(&runtime.sessions_dir).expect("save session");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            operation_id: "resume-operation".to_string(),
            session_id: "work-live".to_string(),
            agent_session_id: Some("conv-old".to_string()),
            bounds: canvas_bounds(),
        },
    );

    let event = events
        .first()
        .expect("resume must reply on conversation conflict");
    assert!(matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError { session_id, message, .. }
            if session_id == "work-live" && message.contains("different conversation")
    ));
}

#[test]
fn app_runtime_latest_branch_resume_picks_newest_resumable_session() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut older = gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
    older.id = "session-older".to_string();
    older.agent_session_id = Some("native-older".to_string());
    older.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 9, 0, 0).unwrap();
    older.updated_at = older.last_activity_at;
    older.created_at = older.last_activity_at;
    older.save(&sessions_dir).expect("save older session");

    let mut newer = gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
    newer.id = "session-newer".to_string();
    newer.agent_session_id = Some("native-newer".to_string());
    newer.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 10, 0, 0).unwrap();
    newer.updated_at = newer.last_activity_at;
    newer.created_at = newer.last_activity_at;
    newer.save(&sessions_dir).expect("save newer session");

    let mut metadata_only =
        gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
    metadata_only.id = "session-metadata-only".to_string();
    metadata_only.agent_session_id = None;
    metadata_only.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 11, 0, 0).unwrap();
    metadata_only.updated_at = metadata_only.last_activity_at;
    metadata_only.created_at = metadata_only.last_activity_at;
    metadata_only
        .save(&sessions_dir)
        .expect("save metadata-only session");

    let runtime = sample_runtime(temp.path(), Vec::new(), None);

    let selected = runtime
        .latest_resumable_branch_session(&repo, "work/manual-resume")
        .expect("latest resumable session");

    assert_eq!(selected.id, "session-newer");
    assert_eq!(selected.agent_session_id.as_deref(), Some("native-newer"));
}

#[test]
fn app_runtime_latest_branch_resume_reflects_sessions_refreshed_after_cache_load() {
    // #2995 regression: the managed hook CLI persists a session's real
    // agent_session_id out-of-process *after* the GUI loaded its in-memory
    // session cache. The branch load's disk-fresh refresh
    // (apply_refreshed_launch_wizard_sessions, dispatched off-thread) must
    // make such a session resumable without a full process restart — the
    // gwt daemon/tray process otherwise keeps the stale cache alive.
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    // Runtime constructed first, so its in-memory cache starts empty.
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    assert!(
        runtime
            .latest_resumable_branch_session(&repo, "work/late-write")
            .is_none(),
        "no session exists yet"
    );

    // Session TOML appears on disk afterwards (hook CLI writing the native
    // agent_session_id post-launch). The stale cache still cannot see it.
    let mut session = gwt_agent::Session::new(&repo, "work/late-write", gwt_agent::AgentId::Codex);
    session.id = "session-late".to_string();
    session.agent_session_id = Some("native-late".to_string());
    session.save(&sessions_dir).expect("save late session");
    assert!(
        runtime
            .latest_resumable_branch_session(&repo, "work/late-write")
            .is_none(),
        "stale cache must not yet see the late on-disk session"
    );

    // The off-thread branch load refreshes the cache from disk (no main
    // thread session-dir scan); resolution now finds the session.
    runtime
        .apply_refreshed_launch_wizard_sessions(gwt::launch_wizard::load_sessions(&sessions_dir));
    let selected = runtime
        .latest_resumable_branch_session(&repo, "work/late-write")
        .expect("disk-fresh refresh makes the late session resumable");
    assert_eq!(selected.id, "session-late");
    assert_eq!(selected.agent_session_id.as_deref(), Some("native-late"));
}

#[test]
fn app_runtime_open_launch_wizard_shows_only_latest_resume_and_focus_methods() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    for (session_id, native_session_id, hour) in [
        ("session-older", "native-older", 9),
        ("session-newer", "native-newer", 10),
    ] {
        let mut session =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, hour, 0, 0).unwrap();
        session.updated_at = session.last_activity_at;
        session.created_at = session.last_activity_at;
        session.save(&sessions_dir).expect("save session");
    }
    for (session_id, hour) in [("session-live-older", 11), ("session-live-newer", 12)] {
        let mut session =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = None;
        session.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, hour, 0, 0).unwrap();
        session.updated_at = session.last_activity_at;
        session.created_at = session.last_activity_at;
        session
            .save(&sessions_dir)
            .expect("save live session metadata");
    }

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "branches-1",
        repo,
        WindowPreset::Branches,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "branches-1");
    for (window, session, display) in [
        ("agent-older", "session-live-older", "Codex Older"),
        ("agent-newer", "session-live-newer", "Codex Newer"),
    ] {
        let agent_window_id = combined_window_id("tab-1", window);
        runtime.active_agent_sessions.insert(
            agent_window_id.clone(),
            ActiveAgentSession {
                window_id: agent_window_id,
                session_id: session.to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/manual-resume".to_string(),
                display_name: display.to_string(),
                worktree_path: PathBuf::from("/tmp/repo"),
                agent_project_root: "/tmp/repo".to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
    }

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenLaunchWizard {
            id: window_id,
            branch_name: "work/manual-resume".to_string(),
            linked_issue_number: None,
        },
    );

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert_eq!(
        view.quick_start_entries
            .iter()
            .map(|entry| entry.resume_session_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("native-newer")]
    );
    let continue_method = view
        .start_methods
        .iter()
        .find(|method| method.kind == "continue_last_session")
        .expect("continue method");
    assert!(
        continue_method
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("native-newer"),
        "continue method should describe the latest resumable session"
    );
    let focus_method = view
        .start_methods
        .iter()
        .find(|method| method.kind == "focus_running_session")
        .expect("focus method");
    assert!(
        focus_method.summary.contains("Codex Newer"),
        "focus method should target the latest live session"
    );
}

#[test]
fn app_runtime_resume_branch_latest_returns_branch_error_when_no_resumable_session_exists() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "branches-1",
        repo,
        WindowPreset::Branches,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "branches-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeBranchLatestAgent {
            id: window_id.clone(),
            branch_name: "work/manual-resume".to_string(),
            bounds: canvas_bounds(),
        },
    );

    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::BranchError { id, message })
            if id == &window_id && message.contains("No resumable session")
    ));
}

#[test]
fn app_runtime_bootstrap_auto_resumes_clean_waiting_input_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("auto-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/auto-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    for (session_id, native_session_id) in [
        ("session-auto-one", "native-session-one"),
        ("session-auto-two", "native-session-two"),
    ] {
        let worktree = temp.path().join("worktrees").join(session_id);
        let branch = format!("work/{session_id}");
        run_git(
            &repo,
            &["worktree", "add", "-b", &branch, worktree.to_str().unwrap()],
        );
        let mut session = gwt_agent::Session::new(&worktree, &branch, gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert_eq!(
        runtime.tabs.len(),
        1,
        "bootstrap should reopen the worktree tab"
    );
    assert!(same_worktree_path(&runtime.tabs[0].project_root, &worktree));
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 2,
        "exact-resumable sessions in distinct worktrees should restart"
    );
}

#[test]
fn app_runtime_bootstrap_resumes_session_in_linked_worktree_of_workspace_home_tab() {
    // Issue #2942 root cause: the open tab's project_root is the gwt
    // workspace home / main repo, while a resumable agent session lives in
    // a *linked worktree*. `repo_hash` / `project_scope_hash` differ between
    // the two, so scope-hash matching failed and the session never resumed
    // on startup. It must match via the shared main worktree root instead.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("linked-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/linked-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    // Tab project_root is the workspace home / main repo, NOT the worktree.
    let tab = sample_project_tab("tab-home", "Home", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-home"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/linked-resume",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "sess-linked".to_string();
    session.agent_session_id = Some("native-linked".to_string());
    // A non-matching persisted repo_hash guarantees the scope-hash fallback
    // cannot match; only the main-worktree-root association can.
    session.repo_hash = Some("zz-nonmatching-scope-hash".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime
        .tab("tab-home")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "a session in a linked worktree must resume into the workspace-home tab"
    );
}

#[test]
fn app_runtime_bootstrap_resumes_unclosed_window_despite_stopped_status() {
    // Issue #2942: a session whose status drifted to Stopped (an idle timeout)
    // must STILL resume on startup when its agent window is still present in
    // the workspace — the user did not explicitly close it. The
    // status-candidate gate would exclude this session on the orphan path;
    // only the "unclosed placeholder" path can restore it.
    //
    // Issue #4441 supersedes the *age* half of that contract, which this test
    // used to assert at 30 hours. #2942 read a surviving placeholder as proof
    // that the user had left the window open, because closing a window removes
    // it from the workspace. Agent panes never close themselves, so in practice
    // the placeholder set became every launch the machine ever performed: 72
    // windows, 55 of them stuck in `starting`, one Issue restored eight times
    // over. "Not explicitly closed" is not the same fact as "open at the last
    // exit", so the 24-hour bound now applies to this path too, and the case is
    // asserted at both ends below.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("unclosed-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/unclosed-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );

    // Tab still holds the paused agent placeholder (not closed by the user).
    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("sess-unclosed".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-unclosed".to_string(),
        title: "Unclosed".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-unclosed"));

    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/unclosed-resume",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "sess-unclosed".to_string();
    session.agent_session_id = Some("native-unclosed".to_string());
    session.record_hook_event("Stop");
    session.record_completed_stop();
    // A launch marks the window as one to restore; this is what separates a
    // window the user left open from one whose agent settled (Issue #4441).
    session.restore_window_on_startup = true;
    // Status drifted to Stopped (would fail the candidate gate on the orphan
    // path) but the window is recent.
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.last_activity_at = chrono::Utc::now() - chrono::Duration::hours(2);
    session
        .save(&runtime.sessions_dir)
        .expect("save stopped session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime
        .tab("tab-unclosed")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "an unclosed agent window must resume despite Stopped status"
    );
    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        1,
        "the resumed unclosed window must track its source session"
    );
}

/// Issue #4441: the other end of the case above — the same unclosed window,
/// aged past the freshness bound, does not come back.
///
/// This is the half of Issue #2942 that #4441 supersedes. Keeping the two
/// assertions adjacent is deliberate: the difference between them is the entire
/// behavioural change, and reading one without the other makes it look like
/// either #2942 or #4441 was simply dropped.
#[test]
fn app_runtime_bootstrap_does_not_resume_an_unclosed_window_past_the_freshness_bound() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("stale-unclosed");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/stale-unclosed",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("sess-stale".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-stale".to_string(),
        title: "Stale".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-stale"));

    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/stale-unclosed",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "sess-stale".to_string();
    session.agent_session_id = Some("native-stale".to_string());
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session.restore_window_on_startup = true;
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.last_activity_at = chrono::Utc::now() - chrono::Duration::hours(30);
    session
        .save(&runtime.sessions_dir)
        .expect("save stale stopped session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert!(
        runtime.pending_auto_resume_sources.is_empty(),
        "a placeholder older than the freshness bound is relaunch history, not the last-open set"
    );
}

#[test]
fn app_runtime_bootstrap_queues_startup_auto_resume_until_canvas_ready() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("queued-auto-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/queued-auto-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/queued-auto-resume",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-queued-auto".to_string();
    session.agent_session_id = Some("native-queued-auto".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();

    assert!(
        runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .all(|window| window.preset != WindowPreset::Agent),
        "bootstrap should wait for the frontend canvas bounds before placing restored windows"
    );

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(agent_windows, 1);
    assert_eq!(runtime.pending_auto_resume_sources.len(), 1);
}

/// Issue #3934: a closed agent window leaves its Session durably `Idle` with
/// no runtime sidecar anywhere. The coarse liveness prefilter reads that as
/// "cannot tell" and used to drop the owner before the exact stage ever looked
/// at it, so the generation was held for good and the work could only be
/// restarted under a fresh Issue number. The prefilter may narrow the work the
/// exact stage does; it must never be what decides the outcome.
#[test]
fn startup_reaper_reclaims_a_durably_idle_holder_the_prefilter_calls_unknown() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("idle-owner");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/idle-owner",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3934,
    };
    let session_id = "startup-idle-holder";
    let worktrees = seed_defunct_active_owner(
        &runtime.sessions_dir,
        &repo,
        &worktree,
        "work/idle-owner",
        owner,
        session_id,
        gwt_agent::AgentStatus::Idle,
    );

    assert!(
        matches!(
            runtime.classify_nonlocal_active_owner_liveness(session_id),
            ActiveOwnerLiveness::Unknown
        ),
        "the coarse prefilter still cannot classify a durably Idle holder"
    );

    let summary = runtime.reap_startup_defunct_active_generations(&worktrees);

    assert_eq!(
        summary.reaped, 1,
        "the exact stage decides, not the prefilter"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&worktree, owner)
            .expect("load ledger")
            .expect("ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
    );
}

/// Issue #3964 AC-2: a launch that died before its agent ever ran leaves the
/// Session durably `Running` with no runtime sidecar anywhere (four of the
/// production rows: #3619 / #3620 / #3623 / #3807). The prefilter reads a
/// Running holder as "cannot tell" and the durable state does not permit
/// reclaim on its own, so the exact stage — which would have found no runtime
/// at all — was never consulted.
#[test]
fn startup_reaper_reclaims_a_durably_running_holder_with_no_runtime() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for status in [
        gwt_agent::AgentStatus::Running,
        gwt_agent::AgentStatus::WaitingInput,
        gwt_agent::AgentStatus::Unknown,
    ] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("running-owner");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/running-owner",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 3964,
        };
        let session_id = "startup-running-holder-without-runtime";
        let worktrees = seed_defunct_active_owner(
            &runtime.sessions_dir,
            &repo,
            &worktree,
            "work/running-owner",
            owner,
            session_id,
            status,
        );

        let summary = runtime.reap_startup_defunct_active_generations(&worktrees);

        assert_eq!(
            summary.reaped, 1,
            "a {status:?} holder with no runtime anywhere is not running: {summary:?}"
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(&worktree, owner)
                .expect("load ledger")
                .expect("ledger")
                .current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
        );
    }
}

/// Issue #3964 AC-1: a refused relaunch materializes the worktree again but
/// publishes nothing into it, so the owner ledger is Active while the
/// worktree's trusted pointer/projection pair is missing (production rows
/// #3465 / #3481 / #3567 / #3624 ...). The holder identity used to be read
/// from that missing publication, which silently counted the owner as
/// unchanged on every startup and every scan.
#[test]
fn startup_reaper_heals_a_lost_worktree_publication_before_judging_the_holder() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("unpublished-owner");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/unpublished-owner",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3465,
    };
    let session_id = "startup-unpublished-holder";
    let worktrees = seed_defunct_active_owner(
        &runtime.sessions_dir,
        &repo,
        &worktree,
        "work/unpublished-owner",
        owner,
        session_id,
        gwt_agent::AgentStatus::Stopped,
    );
    let trusted_dir =
        gwt::cli::trusted_store::trusted_dir_for_worktree(&worktree).expect("trusted worktree dir");
    fs::remove_file(trusted_dir.join("execution-generation-pointer.json")).expect("drop pointer");
    fs::remove_file(trusted_dir.join("execution-control.json")).expect("drop projection");

    let summary = runtime.reap_startup_defunct_active_generations(&worktrees);

    assert_eq!(
        summary.reaped, 1,
        "the owner ledger is the authority; a lost publication is republished, not skipped: {summary:?}"
    );
    assert!(
        trusted_dir
            .join("execution-generation-pointer.json")
            .is_file(),
        "the reaper republishes the worktree pointer before judging the holder"
    );
    assert!(
        trusted_dir.join("execution-control.json").is_file(),
        "the reaper republishes the worktree projection before judging the holder"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&worktree, owner)
            .expect("load ledger")
            .expect("ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
    );
}

/// SPEC-2359 W-37 / Issue #3735: restore selection completes before the
/// generation reaper. The selected exact holder remains Active while another
/// stale owner in the same repository is audited Blocked in the same batch.
#[test]
fn startup_reaper_reaps_stale_owner_but_preserves_selected_restore_holder() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let protected_worktree = temp.path().join("worktrees").join("protected-restore");
    let protected_unknown_worktree = temp
        .path()
        .join("worktrees")
        .join("protected-unknown-restore");
    let stale_worktree = temp.path().join("worktrees").join("stale-owner");
    let second_stale_worktree = temp.path().join("worktrees").join("second-stale-owner");
    for (branch, worktree) in [
        ("work/protected-restore", &protected_worktree),
        (
            "work/protected-unknown-restore",
            &protected_unknown_worktree,
        ),
        ("work/stale-owner", &stale_worktree),
        ("work/second-stale-owner", &second_stale_worktree),
    ] {
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                worktree.to_str().expect("worktree path"),
            ],
        );
    }
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let protected_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3735,
    };
    let stale_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3736,
    };
    let protected_unknown_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3737,
    };
    let second_stale_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3738,
    };

    for (owner, session_id, worktree, branch, selected_for_restore, exact_binding) in [
        (
            protected_owner,
            "startup-protected-holder",
            protected_worktree.as_path(),
            "work/protected-restore",
            true,
            true,
        ),
        (
            protected_unknown_owner,
            "startup-protected-unknown-holder",
            protected_unknown_worktree.as_path(),
            "work/protected-unknown-restore",
            true,
            false,
        ),
        (
            stale_owner,
            "startup-stale-holder",
            stale_worktree.as_path(),
            "work/stale-owner",
            false,
            true,
        ),
        (
            second_stale_owner,
            "startup-second-stale-holder",
            second_stale_worktree.as_path(),
            "work/second-stale-owner",
            false,
            true,
        ),
    ] {
        gwt::cli::execution_state::materialize_at_launch(
            worktree,
            owner.kind,
            owner.number,
            session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize Active execution");
        gwt::cli::execution_state::ensure_generation_ledger(
            worktree,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("ensure generation ledger");
        let binding = gwt::cli::execution_state::current_execution_binding(worktree, owner)
            .expect("load current binding")
            .expect("current binding");
        let mut session = gwt_agent::Session::new(worktree, branch, gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(format!("native-{session_id}"));
        session.linked_issue_number = Some(owner.number);
        session.restore_window_on_startup = selected_for_restore;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.update_status(if selected_for_restore {
            gwt_agent::AgentStatus::Interrupted
        } else {
            gwt_agent::AgentStatus::Stopped
        });
        if exact_binding {
            session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash: session.repo_hash.clone().expect("repo hash"),
                owner_kind: "issue".to_string(),
                owner_number: owner.number,
                identity: binding,
                capability_generation: 1,
            });
        }
        session
            .save(&runtime.sessions_dir)
            .expect("save startup holder Session");
    }

    runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    assert_eq!(runtime.pending_startup_auto_resume_sessions.len(), 2);
    let startup_worktrees = gwt::worktree_inventory::enumerate_worktrees(&repo, None)
        .expect("startup worktree inventory")
        .into_iter()
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    let trusted_worktree_dir =
        gwt::cli::trusted_store::trusted_dir_for_worktree(&protected_worktree)
            .expect("trusted worktree directory");
    let corrupt_owner_dir = trusted_worktree_dir
        .parent()
        .expect("repository trusted root")
        .join("execution-owners")
        .join("owner-999999");
    fs::create_dir_all(&corrupt_owner_dir).expect("create corrupt owner directory");
    fs::write(
        corrupt_owner_dir.join("generation-ledger.json"),
        b"{malformed owner ledger",
    )
    .expect("write corrupt owner ledger");

    let mut summary = None;
    let logs = capture_tracing_events(|| {
        summary = Some(runtime.reap_startup_defunct_active_generations(&startup_worktrees));
    });
    let summary = summary.expect("startup reaper summary");

    assert_eq!(summary.reaped, 2);
    assert_eq!(summary.protected, 2);
    assert_eq!(summary.failures, 1);
    assert!(logs.iter().any(|event| {
        event.level == Level::INFO
            && event.fields.get("message").map(String::as_str)
                == Some("startup Active generation reaper completed")
            && event.fields.contains_key("duration_ms")
    }));
    assert!(logs.iter().any(|event| {
        event.level == Level::WARN
            && event.fields.get("message").map(String::as_str)
                == Some("startup Active generation owner inspection failed closed")
    }));
    assert_eq!(
        gwt::cli::execution_state::load(&protected_worktree)
            .unwrap()
            .unwrap()
            .status,
        gwt::cli::execution_state::ExecutionControlStatus::Active
    );
    assert_eq!(
        gwt::cli::execution_state::load(&stale_worktree)
            .unwrap()
            .unwrap()
            .status,
        gwt::cli::execution_state::ExecutionControlStatus::Blocked
    );
    assert_eq!(
        gwt::cli::execution_state::load(&protected_unknown_worktree)
            .unwrap()
            .unwrap()
            .status,
        gwt::cli::execution_state::ExecutionControlStatus::Active
    );
    assert_eq!(
        gwt::cli::execution_state::load(&second_stale_worktree)
            .unwrap()
            .unwrap()
            .status,
        gwt::cli::execution_state::ExecutionControlStatus::Blocked
    );
}

/// Issue #4378 AC-1: bootstrap lists each project's worktrees once and hands
/// that inventory to the startup ingest. Issue #3777 AC-3 then consumes it on
/// the ingest worker, so what reaches the tao callback is the already
/// reconciled branch set — no second listing, and no listing on the GUI thread.
#[test]
fn bootstrap_hands_its_worktree_inventory_to_the_startup_ingest() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, _tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.bootstrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let local_branches = loop {
        let carried = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find_map(|event| match recorded_project_payload(event) {
                UserEvent::WorkEventsIngested { local_branches, .. } => {
                    Some(local_branches.clone())
                }
                _ => None,
            });
        if let Some(carried) = carried {
            break carried;
        }
        assert!(
            Instant::now() < deadline,
            "the startup ingest never completed"
        );
        // test-hygiene: allow-short-duration polling completed ingest event under an independent deadline; event arrival establishes ordering
        thread::sleep(Duration::from_millis(20));
    };
    let local_branches =
        local_branches.expect("the ingest worker must carry back the reconciled branch set");
    assert!(
        !local_branches.is_empty(),
        "the reconcile ran on the worker from the bootstrap listing: {local_branches:?}"
    );
}

/// Issue #4825: an old HOME layout must stop startup before canonical writers.
#[test]
fn bootstrap_refuses_legacy_workspace_layout_without_mutation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let legacy_path =
        gwt_core::paths::gwt_project_dir_for_repo_path(&repo).join("workspace/current.json");
    let legacy = serde_json::to_vec(
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .unwrap();
    fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
    fs::write(&legacy_path, &legacy).unwrap();
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, _tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.bootstrap();

    let recorded = events.lock().unwrap();
    assert!(
        recorded.iter().any(|event| matches!(
            recorded_project_payload(event),
            UserEvent::WorkspaceStateLoadFailed { error, .. }
                if error.path == legacy_path && error.message.contains("v9.106.0")
        )),
        "startup must report the upgrade requirement"
    );
    assert!(
        !recorded.iter().any(|event| matches!(
            recorded_project_payload(event),
            UserEvent::WorkEventsIngested { .. }
        )),
        "legacy layouts must not enter startup ingest"
    );
    assert_eq!(fs::read(&legacy_path).unwrap(), legacy);
    let canonical = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo);
    assert!(!canonical.exists());
    assert!(!canonical.with_file_name("works.json").exists());
}

/// Issue #5116 AC-2: an ordinary ingest tick shares one worktree listing
/// between intake and reconcile, just as the startup path already does.
#[test]
fn work_events_ingest_tick_lists_worktrees_once() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let mut seed = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-session-ingest-tick",
        chrono::Utc::now(),
    );
    seed.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
    seed.title = Some("existing tick work".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&repo, seed)
        .expect("seed existing work");
    let project_key = gwt_core::paths::resolve_project_scope(&repo).hash;
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path(&project_key);
    let state_path = gwt_core::paths::gwt_workspace_work_events_intake_state_path(&project_key);
    let projection_path = gwt_core::paths::gwt_workspace_projection_path(&project_key);
    let before = worktree_listings_on_this_thread();

    let event = AppRuntime::prepare_work_events_ingest(
        repo,
        &work_items_path,
        &state_path,
        &projection_path,
        None,
    );

    assert!(matches!(
        event,
        Some(UserEvent::WorkEventsIngested {
            local_branches: Some(_),
            ..
        })
    ));
    assert_eq!(
        worktree_listings_on_this_thread() - before,
        1,
        "intake and reconcile must share the same tick's worktree listing"
    );

    // The grouped store writes index.json; state_path names its legacy file.
    // A failed next listing preserves existing bytes or an absent cursor.
    let intake_index_path = state_path.with_extension("").join("index.json");
    let work_items = fs::read(&work_items_path).ok();
    assert!(
        work_items.is_some(),
        "fixture must contain durable Work history"
    );
    let intake_state = fs::read(&intake_index_path).ok();
    let unavailable_repo = temp.path().join("not-a-repo");
    fs::create_dir_all(&unavailable_repo).expect("unavailable repository dir");
    let before = worktree_listings_on_this_thread();
    assert!(AppRuntime::prepare_work_events_ingest(
        unavailable_repo,
        &work_items_path,
        &state_path,
        &projection_path,
        None,
    )
    .is_none());
    assert_eq!(worktree_listings_on_this_thread() - before, 1);
    assert_eq!(fs::read(&work_items_path).ok(), work_items);
    assert_eq!(fs::read(&intake_index_path).ok(), intake_state);
}

/// Issue #4378 AC-1: bootstrap lists each project's worktrees once on the
/// startup path. The orphan intake prune plan used to list them a second time
/// on the GUI thread; the ingest and reconcile reuse is pinned above.
#[test]
fn bootstrap_lists_the_worktrees_once_on_the_startup_path() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, _tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let before = worktree_listings_on_this_thread();

    runtime.bootstrap();

    assert_eq!(
        worktree_listings_on_this_thread() - before,
        1,
        "bootstrap must list the worktrees exactly once"
    );
}

/// Issue #4378 AC-2: bootstrap no longer runs the generation reaper on the
/// startup path. It runs on the blocking worker and reports back with an
/// event. Issue Monitor launch deliveries that arrive first wait for that
/// event, so a launch never races a generation the reaper is about to reap.
#[test]
fn bootstrap_runs_the_generation_reaper_off_the_startup_path() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("defunct-owner");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/defunct-owner",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 4378,
    };
    seed_defunct_active_owner(
        &runtime.sessions_dir,
        &repo,
        &worktree,
        "work/defunct-owner",
        owner,
        "startup-off-loop-holder",
        gwt_agent::AgentStatus::Stopped,
    );
    let effective_status = || {
        gwt::cli::execution_state::load_generation_ledger(&worktree, owner)
            .expect("load ledger")
            .expect("ledger")
            .current_effective_status()
    };

    runtime.bootstrap();

    assert_eq!(
        effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active),
        "the reaper must not run synchronously inside bootstrap"
    );
    let delivery_id = "launch:startup-reaper-gate";
    runtime.issue_monitor_launch_deliveries.insert(
        delivery_id.to_string(),
        super::super::IssueMonitorLaunchDeliveryState::LaunchFailed {
            message: "seeded so the replay settles without a launch".to_string(),
            session_mode: gwt_agent::SessionMode::Normal,
        },
    );
    let early = runtime.auto_launch_issue_monitor_delivery_events_for_project(
        &repo,
        owner.number,
        gwt::LinkedIssueKind::Issue,
        Some(delivery_id.to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    assert!(
        early.is_empty(),
        "a launch delivery must wait for the startup reaper"
    );
    assert_eq!(
        runtime
            .deferred_issue_monitor_launches
            .as_ref()
            .map(Vec::len),
        Some(1)
    );

    let queued = std::mem::take(
        &mut *tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for task in queued {
        task();
    }

    assert_eq!(
        effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "the deferred reaper must still reap the defunct holder"
    );
    assert!(events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|event| matches!(
            recorded_project_payload(event),
            UserEvent::StartupGenerationReaperCompleted
        )));
    runtime.handle_startup_generation_reaper_completed();
    assert!(
        runtime.deferred_issue_monitor_launches.is_none(),
        "the reaper completion releases the held deliveries"
    );
}
