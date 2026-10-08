use super::*;

/// T-093 (FR-012): a parked PM is woken by a NeedsHuman transition, the wake
/// targets exactly the registered PM's pane, and the loop budget is re-armed.
#[test]
fn pm_wake_targets_only_the_registered_pm_pane_on_new_needs_human() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);

    // Parked long ago: budget exhausted, last continuation stale.
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-08T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed parked loop state");

    // Baseline observation: never wakes, only records what is already there.
    let baseline = [pm_wake_inbox_item(41, gwt::MonitorInboxState::Queued)];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &baseline, "2026-08-08T01:00:00Z")
            .is_none(),
        "the first observation is a baseline, not a wake"
    );

    // A new NeedsHuman row after the baseline is a wake.
    let now_with_escalation = [
        pm_wake_inbox_item(41, gwt::MonitorInboxState::Queued),
        pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman),
    ];
    let decision = runtime
        .pm_wake_decision_at(&repo, &now_with_escalation, "2026-08-08T01:01:00Z")
        .expect("a new NeedsHuman row must wake the parked PM");
    assert_eq!(
        decision.window_id, pm_window_id,
        "the wake reaches the registered PM pane and nothing else"
    );
    assert!(
        decision.prompt.contains("issue.monitor.status"),
        "the prompt instructs one reconcile cycle: {}",
        decision.prompt
    );
    assert!(
        decision.prompt.ends_with('\r'),
        "the prompt must submit itself"
    );

    let rearmed = gwt::pm_registry::load_pm_loop_state(&loop_path).expect("loop state");
    assert_eq!(
        rearmed.consecutive_continuations, 0,
        "a wake re-arms the parked loop budget"
    );

    // The same snapshot again is consumed: no second wake for old news.
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &now_with_escalation, "2026-08-08T02:00:00Z")
            .is_none(),
        "an already-woken signal set must not wake twice"
    );
}

/// T-093: while the loop is (recently) active the wake is suppressed but the
/// signal is retained, so a floor-stopped loop is still revived later.
#[test]
fn pm_wake_suppression_during_active_loop_retries_on_the_next_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);

    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 3,
            last_continued_at: Some("2026-08-08T01:00:30Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed active loop state");

    assert!(
        runtime
            .pm_wake_decision_at(&repo, &[], "2026-08-08T01:00:40Z")
            .is_none(),
        "baseline"
    );
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:00:50Z")
            .is_none(),
        "an actively-looping PM handles the event itself; no interrupt"
    );
    // The next snapshot after the interval elapses still carries the (unconsumed)
    // signal and wakes.
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:02:00Z")
            .is_some(),
        "a retained signal wakes once the loop has gone quiet"
    );
}

/// Issue #4258 AC-1/AC-2/AC-3: a quiet loop clock is not enough — a PM pane
/// that is mid-turn (Running) or at a prompt (Waiting) holds the delta wake,
/// and the held signals fire together once the pane is Idle.
#[test]
fn pm_wake_holds_the_delta_while_the_pm_pane_is_busy_and_fires_once_idle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            last_continued_at: Some("2026-08-08T01:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed quiet loop state");
    assert!(runtime
        .pm_wake_decision_at(&repo, &[], "2026-08-08T01:01:30Z")
        .is_none());

    runtime
        .window_hook_states
        .insert(pm_window_id.clone(), WindowProcessStatus::Running);
    let first = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &first, "2026-08-08T01:02:00Z")
            .is_none(),
        "a Running PM pane must not be interrupted by a delta wake"
    );

    runtime
        .window_hook_states
        .insert(pm_window_id.clone(), WindowProcessStatus::Waiting);
    let second = [
        pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman),
        pm_wake_inbox_item(43, gwt::MonitorInboxState::NeedsHuman),
    ];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &second, "2026-08-08T01:02:30Z")
            .is_none(),
        "a Waiting PM pane would read the wake as its prompt answer"
    );
    assert!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("loop state")
            .last_wake_at
            .is_none(),
        "a held wake must not stamp the wake clock"
    );

    runtime.window_hook_states.remove(&pm_window_id);
    let decision = runtime
        .pm_wake_decision_at(&repo, &second, "2026-08-08T01:02:40Z")
        .expect("the held delta fires once the PM pane is Idle");
    assert_eq!(decision.window_id, pm_window_id);
    assert!(
        decision.prompt.contains("needs_human:42") && decision.prompt.contains("needs_human:43"),
        "signals held across the busy period arrive as one prompt: {}",
        decision.prompt
    );
}

/// Issue #4258 AC-1/AC-3: the periodic wake is held by a busy PM pane the
/// same way, and fires on the first tick after the pane is Idle.
#[test]
fn periodic_wake_holds_while_the_pm_pane_is_busy_and_fires_once_idle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    seed_quiet_standing_supervision(&repo);
    // Quiet for the 60s interval, but well inside the busy-deferral bound.
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            last_continued_at: Some("2026-08-10T00:58:30Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed recently quiet loop");

    for busy in [WindowProcessStatus::Running, WindowProcessStatus::Waiting] {
        runtime
            .window_hook_states
            .insert(pm_window_id.clone(), busy);
        assert!(
            runtime
                .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
                .is_none(),
            "a {busy:?} PM pane must not receive the scheduled tick"
        );
    }
    assert!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("loop state")
            .last_wake_at
            .is_none(),
        "a held tick must not stamp the wake clock"
    );

    runtime.window_hook_states.remove(&pm_window_id);
    let decision = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:10Z")
        .expect("the first tick after the pane is Idle wakes the PM");
    assert_eq!(decision.window_id, pm_window_id);
}

/// Issue #4258 AC-4: a pane stuck on Running (a missed Stop hook, #3809)
/// cannot hold wakes forever — past the bound the wake fires anyway. A
/// Waiting pane never gets that override: the injected text would answer
/// its prompt.
#[test]
fn pm_wake_busy_deferral_is_bounded_for_running_but_not_for_waiting() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    // Last loop activity an hour ago: far past the deferral bound.
    seed_quiet_standing_supervision(&repo);

    runtime
        .window_hook_states
        .insert(pm_window_id.clone(), WindowProcessStatus::Waiting);
    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
            .is_none(),
        "a Waiting PM pane is never overridden"
    );

    runtime
        .window_hook_states
        .insert(pm_window_id.clone(), WindowProcessStatus::Running);
    let decision = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
        .expect("a Running pane past the deferral bound is woken anyway");
    assert_eq!(decision.window_id, pm_window_id);
}

#[test]
fn pm_wake_next_decision_reloads_updated_loop_interval() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.settings.loop_interval_secs = 60;
    })
    .expect("seed sixty-second interval");
    let registration_before = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("load seeded prefs")
        .registration;
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            last_continued_at: Some("2026-08-08T01:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed active loop state");

    assert!(
        runtime
            .pm_wake_decision_at(&repo, &[], "2026-08-08T01:00:20Z")
            .is_none(),
        "first snapshot is the baseline"
    );
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:00:30Z")
            .is_none(),
        "sixty-second interval suppresses the retained signal"
    );

    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.settings.loop_interval_secs = 10;
    })
    .expect("lower loop interval");

    let decision = runtime
        .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:00:30Z")
        .expect("next wake decision must reload the ten-second interval");
    assert_eq!(decision.window_id, pm_window_id);
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "wake must not restart PM"
    );
    assert_eq!(
        gwt::pm_registry::load_pm_prefs(&prefs_path)
            .expect("reload PM prefs")
            .registration,
        registration_before,
        "the hot interval update must preserve the live registration"
    );
}

/// T-093 negative space: no registration, a dead PM pane, or a disabled
/// Monitor never wake anything — and never target another session's pane.
#[test]
fn pm_wake_never_fires_without_a_live_registered_pm_or_with_monitor_off() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);

    assert!(
        runtime
            .pm_wake_decision_at(&repo, &[], "2026-08-08T01:00:00Z")
            .is_none(),
        "baseline"
    );
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];

    // Monitor off: the user parked the project; stay silent.
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor_prefs = gwt::load_issue_monitor_prefs(&monitor_prefs_path).expect("prefs");
    monitor_prefs.enabled = false;
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor_prefs).expect("save prefs");
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:01:00Z")
            .is_none(),
        "a disabled Monitor must not wake the PM"
    );
    monitor_prefs.enabled = true;
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor_prefs).expect("save prefs");

    // Dead PM pane: the crash-resume path owns recovery, not the wake path.
    runtime.active_agent_sessions.remove(&pm_window_id);
    let escalated_more = [
        pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman),
        pm_wake_inbox_item(43, gwt::MonitorInboxState::NeedsHuman),
    ];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated_more, "2026-08-08T01:02:00Z")
            .is_none(),
        "a dead PM pane is never woken (and no other pane is targeted)"
    );

    // No registration at all: nothing to wake.
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::deregister_pm(&prefs_path, "pm-session-live").expect("deregister");
    let escalated_again = [
        pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman),
        pm_wake_inbox_item(43, gwt::MonitorInboxState::NeedsHuman),
        pm_wake_inbox_item(44, gwt::MonitorInboxState::NeedsHuman),
    ];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated_again, "2026-08-08T01:03:00Z")
            .is_none(),
        "no registration, no wake"
    );
}

/// SPEC-3431 FR-021 (review fix): an explicit "Open PM" click starts the PM
/// even while the crash-backoff floor is in the future — the floor damps only
/// the automatic respawn ladder, never the user's own button.
#[test]
fn explicit_pm_open_bypasses_the_crash_backoff_floor() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
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
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    // Issue #4375: the spawn this test is about now begins with a Git refresh
    // handed to the blocking worker, so the queue is where "did the launcher
    // start the PM?" is answered. Holding the queue also keeps the fixture's
    // remote-less repository from running `git fetch` at all.
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    // A crashed PM registration whose backoff floor is far in the future, with
    // no live pane and no materializable session (forces the fresh-spawn arm).
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let mut registration = pm_registration_fixture("pm-session-crashed", &repo);
    registration.consecutive_crashes = 3;
    registration.next_not_before = Some("2999-01-01T00:00:00Z".to_string());
    gwt::pm_registry::try_register_pm(&prefs_path, registration, |_| false)
        .expect("seed crashed registration");

    let automatic =
        runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "the automatic ladder must keep honouring the backoff floor"
    );
    assert!(
        queued.lock().expect("queued tasks").is_empty(),
        "the backoff floor must stop the automatic ladder before it even          prepares the worktree"
    );
    drop(automatic);

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Explicit);

    assert_eq!(
        queued.lock().expect("queued tasks").len(),
        1,
        "FR-021: the explicit launcher must start the PM despite the backoff          floor, which now begins by preparing the worktree off the event loop"
    );
}

/// SPEC-3431 T-093 (review fix): a PM that a human just prompted is busy with
/// that conversation — monitor activity must not be injected into it; the
/// signal is retained and delivered once the conversation has gone quiet.
#[test]
fn pm_wake_defers_to_an_active_human_conversation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);

    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            last_user_prompt_at: Some("2026-08-08T01:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed conversation state");

    assert!(
        runtime
            .pm_wake_decision_at(&repo, &[], "2026-08-08T01:00:05Z")
            .is_none(),
        "baseline"
    );
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:00:10Z")
            .is_none(),
        "a PM in an active human conversation must not receive injected input"
    );
    assert!(
        runtime
            .pm_wake_decision_at(&repo, &escalated, "2026-08-08T01:02:00Z")
            .is_some(),
        "the retained signal wakes once the conversation has gone quiet"
    );
}

#[test]
fn malformed_or_old_quiet_timestamps_can_select_a_prompt_but_never_refresh_head() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);
    let origin = temp.path().join("wake-origin.git");
    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        &repo,
        &[
            "remote",
            "set-url",
            "origin",
            origin.to_str().expect("origin"),
        ],
    );
    run_git(&repo, &["config", "user.name", "Wake Test"]);
    run_git(&repo, &["config", "user.email", "wake@example.com"]);
    run_git(&repo, &["checkout", "-b", "develop"]);
    fs::write(repo.join("README.md"), "A\n").expect("write A");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "A"]);
    run_git(&repo, &["push", "-u", "origin", "develop"]);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    fs::write(repo.join("README.md"), "B\n").expect("write B");
    run_git(&repo, &["commit", "-am", "B"]);
    run_git(&repo, &["push", "origin", "develop"]);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("monitor prefs for stable remote identity");
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::deregister_pm(&prefs_path, "pm-session-live").expect("deregister old PM");
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &pm_worktree),
        |_| false,
    )
    .expect("register canonical PM");
    gwt::pm_registry::save_pm_loop_state(
        &gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo),
        &gwt::pm_registry::PmLoopState {
            last_continued_at: Some("malformed".to_string()),
            last_user_prompt_at: Some("2020-01-01T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("quiet state");

    assert!(runtime
        .pm_wake_decision_at(&repo, &[], "2026-08-29T00:00:00Z")
        .is_none());
    assert!(runtime
        .pm_wake_decision_at(
            &repo,
            &[pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)],
            "2026-08-29T00:01:00Z",
        )
        .is_some());
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        commit_a,
        "quiet-time prompt selection has no fetch or re-point authority"
    );
}

/// Issue #3497: the PM must come up on a bare-layout project — a project
/// root that is not itself a git repository but contains the bare `<name>.git`
/// the worktrees hang off. The launch paths resolve this layout through
/// `main_worktree_root`; the PM worktree preparation used the raw project
/// root and died with "not a git repository", leaving the PM silently absent.
#[test]
fn pm_spawn_prepares_the_worktree_for_a_bare_layout_project() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let canonical_home = fs::canonicalize(temp.path()).expect("canonical test home");
    let _home = ScopedEnvVar::set("HOME", &canonical_home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &canonical_home);
    let _gwt_home = ScopedGwtHome::set(canonical_home.join(".gwt"));

    // A seed repository with one commit, cloned bare into the layout the
    // user actually opens: parent/ (not a repo) containing parent/repo.git.
    let seed = temp.path().join("seed");
    fs::create_dir_all(&seed).expect("seed dir");
    init_repo_with_initial_commit(&seed);
    run_git(&seed, &["branch", "-M", "develop"]);
    let parent = temp.path().join("parent");
    fs::create_dir_all(&parent).expect("parent dir");
    let clone = gwt_core::process::hidden_command("git")
        .args([
            "clone",
            "--bare",
            seed.to_str().expect("seed utf8"),
            parent.join("repo.git").to_str().expect("bare utf8"),
        ])
        .output()
        .expect("run git clone");
    assert!(
        clone.status.success(),
        "bare clone failed: {}",
        String::from_utf8_lossy(&clone.stderr)
    );

    let tab = sample_project_tab("tab-1", "Repo", parent.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Explicit);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "the PM spawn must survive the bare layout instead of dying on \
         worktree preparation"
    );
}

/// Issue #3505 / FR-108(b): the periodic wake re-arms a quiet PM when
/// standing supervision work exists — no new signal delta required — and the
/// wake clock keeps it from stacking prompts inside one quiet window.
#[test]
fn periodic_wake_rearms_a_quiet_pm_with_standing_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);

    // Standing supervision work: one launched issue in the durable prefs.
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..gwt::IssueMonitorConfig::default()
        },
        gwt::load_issue_monitor_prefs(&monitor_prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    monitor.complete_active_launch(42, "tab-1::other-window");
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor.prefs()).expect("save prefs");

    // Quiet loop: parked long ago.
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed loop state");

    let decision = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
        .expect("standing work must periodically wake a quiet PM");
    assert_eq!(decision.window_id, pm_window_id);
    assert!(decision.prompt.contains("issue.monitor.status"));
    assert!(decision.prompt.ends_with('\r'));

    let rearmed = gwt::pm_registry::load_pm_loop_state(&loop_path).expect("state");
    assert_eq!(rearmed.consecutive_continuations, 0, "budget re-armed");
    assert!(rearmed.last_wake_at.is_some(), "wake clock stamped");

    // The wake clock suppresses an immediate repeat.
    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:10Z")
            .is_none(),
        "one quiet window gets at most one injected prompt"
    );
}

/// SPEC #4320 AC-3/AC-7: an unresolved Concern is standing supervision work
/// even after the Monitor owner queue and active-launch inventory are empty.
#[test]
fn periodic_wake_rearms_a_quiet_pm_with_an_unresolved_concern() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);

    gwt_core::concern::ConcernStore::for_repo(&repo)
        .create(
            serde_json::from_value(serde_json::json!({
                "summary": "restored windows remain visible",
                "symptom_measurement": {
                    "kind": "shell_command",
                    "command": "printf '{\"count\":1}'"
                },
                "baseline": {"count": 1},
                "verification_predicate": {
                    "pointer": "/count",
                    "op": "eq",
                    "expected": 0
                },
                "owner_issues": [42]
            }))
            .expect("valid NewConcern fixture"),
        )
        .expect("create unresolved Concern");
    assert!(
        gwt_core::concern::has_unresolved_concerns(&repo).expect("read valid Concern store"),
        "fixture must exercise the readable unresolved-Concern path"
    );

    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed quiet loop");

    let decision = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
        .expect("an unresolved Concern must periodically wake a quiet PM");
    assert_eq!(decision.window_id, pm_window_id);
}

/// A transient Concern-store read failure must retain supervision eligibility;
/// treating it as an empty store could park the only process that can repair or
/// report the unobservable Concern state.
#[test]
fn periodic_wake_remains_eligible_when_the_concern_store_is_unreadable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);

    let repo_hash = detect_repo_hash(&repo).expect("repo hash");
    let concerns_path = gwt_core::paths::gwt_project_dir(&repo_hash)
        .join("project-state")
        .join("concerns.json");
    fs::create_dir_all(concerns_path.parent().expect("project-state directory"))
        .expect("create project-state directory");
    fs::write(&concerns_path, b"{ malformed concern store")
        .expect("write malformed Concern fixture");

    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed quiet loop");

    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
            .is_some(),
        "an unreadable Concern store must keep periodic supervision eligible"
    );
}

#[test]
fn periodic_wake_uses_the_scheduled_snapshot_for_queue_only_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&monitor_prefs_path).expect("prefs"),
    );
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor.prefs())
        .expect("persist scheduled queue membership");
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed quiet loop");

    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:00Z")
            .is_none(),
        "prefs reconstruction has no ephemeral queue"
    );
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(monitor_prefs_path.clone());
    let events = runtime.complete_scheduled_scan_for_test(
        &repo,
        &monitor_prefs_path,
        "2026-08-10T01:00:00Z",
        Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(monitor))),
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorStatus { status } if status.queue_len == 1
    )));
    assert_eq!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("completion wake state")
            .last_wake_at
            .as_deref(),
        Some("2026-08-10T01:00:00Z"),
        "completion injects one periodic wake for queue-only standing work"
    );
    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:00:10Z")
            .is_none(),
        "the shared wake clock suppresses an immediate duplicate"
    );
    assert!(runtime.active_agent_sessions.contains_key(&pm_window_id));
}

#[test]
fn scheduled_completion_rearms_periodic_wake_for_needs_human_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    monitor.escalate_to_needs_human(
        42,
        gwt::NeedsHumanKind::UserChoiceRequired,
        "operator decision required",
    );
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("needs-human prefs");
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("quiet loop");
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());

    let events = runtime.complete_scheduled_scan_for_test(
        &repo,
        &prefs_path,
        "2026-08-10T01:00:00Z",
        Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(monitor))),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorStatus { status }
            if status.autonomous_issues.iter().any(|issue| {
                issue.issue_number == 42 && issue.needs_human
            })
    )));
    assert_eq!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("completion wake state")
            .last_wake_at
            .as_deref(),
        Some("2026-08-10T01:00:00Z")
    );
}

#[test]
fn both_wake_prompts_carry_the_open_escalation_bodies() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);
    let entry_id = seed_open_escalation(
        &repo,
        "2338",
        "事象: execution.reopen が immutable で拒否\n\
         原因: Completed ECR\n\
         依頼: fresh launch を手配してほしい\n\
         再開条件: #2338 に紐づく新しい pane",
    );

    // The delta wake: baseline first, then one genuinely new signal.
    assert!(runtime
        .pm_wake_decision_at(&repo, &[], "2026-08-18T01:00:00Z")
        .is_none());
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    let delta = runtime
        .pm_wake_decision_at(&repo, &escalated, "2026-08-18T01:01:00Z")
        .expect("a fresh signal must wake the quiet PM");

    // The periodic wake, standing on the escalation alone: no queue, no active
    // launch, no autonomous needs_human row.
    let periodic = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-18T01:05:00Z")
        .expect("an open escalation is standing supervision work on its own");

    for (prompt, label) in [
        (delta.delivery_prompt(), "delta"),
        (periodic.delivery_prompt(), "periodic"),
    ] {
        assert!(
            prompt.contains("UNRESOLVED BLOCKED ESCALATIONS (1)"),
            "{label} wake must name the standing blockers; got: {prompt}"
        );
        assert!(
            prompt.contains("fresh launch を手配してほしい"),
            "{label} wake must carry the body, not just a count; got: {prompt}"
        );
        assert!(
            prompt.contains(&format!("params.resolves:[\"{entry_id}\"]")),
            "{label} wake must hand back the handle that closes it; got: {prompt}"
        );
        assert!(
            !prompt.trim_end_matches('\r').contains('\n'),
            "{label} wake is typed into a pane and must stay one line; got: {prompt}"
        );
    }
}

#[test]
fn a_resolved_escalation_leaves_the_wake_prompts_unchanged() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);
    let entry_id = seed_open_escalation(
        &repo,
        "2338",
        "事象: 拒否\n原因: immutable\n依頼: fresh launch\n再開条件: 新 pane",
    );
    let mut resolution = gwt_core::coordination::BoardEntry::new(
        gwt_core::coordination::AuthorKind::User,
        "You",
        gwt_core::coordination::BoardEntryKind::Decision,
        "fresh launch を手配しました",
        None,
        None,
        vec![],
        vec!["2338".to_string()],
    );
    resolution.resolves_entry_ids = vec![entry_id];
    gwt_core::coordination::post_entry(&repo, resolution).expect("post resolution");

    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-18T01:05:00Z")
            .is_none(),
        "a resolved escalation must stop counting as supervision work"
    );
}

#[test]
fn wake_prompts_ask_for_a_report_only_when_the_cycle_changed_something() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);

    // Standing supervision work so the periodic wake is eligible on its own.
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..gwt::IssueMonitorConfig::default()
        },
        gwt::load_issue_monitor_prefs(&monitor_prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-17T00:00:00Z",
    );
    monitor.complete_active_launch(42, "tab-1::other-window");
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor.prefs()).expect("save prefs");

    // The delta wake: baseline, then one genuinely new signal.
    assert!(runtime
        .pm_wake_decision_at(&repo, &[], "2026-08-17T01:00:00Z")
        .is_none());
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    let delta = runtime
        .pm_wake_decision_at(&repo, &escalated, "2026-08-17T01:01:00Z")
        .expect("a fresh signal must wake the quiet PM");
    assert_wake_prompt_reports_only_on_change(&delta.prompt, "the delta wake prompt");

    // The periodic wake, one quiet window later.
    let periodic = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-17T01:05:00Z")
        .expect("standing work must periodically wake a quiet PM");
    assert_wake_prompt_reports_only_on_change(&periodic.prompt, "the periodic wake prompt");

    // Issue #3632 FR-4/AC-5: the cycle a silent PM just ran is still visible
    // outside the conversation, so "quiet" stays distinguishable from "dead"
    // without a keepalive line in the chat.
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    assert_eq!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("loop state")
            .last_wake_at
            .as_deref(),
        Some("2026-08-17T01:05:00Z"),
        "pm-loop.json must record the wake even when the cycle reports nothing"
    );
}

/// Issue #3505 / FR-108(b): a delta wake and the periodic wake share the wake
/// clock, so one tick never stacks two prompts into the PM pane.
#[test]
fn delta_and_periodic_wakes_do_not_double_fire_in_one_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);

    // Standing work so the periodic wake would be eligible on its own.
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..gwt::IssueMonitorConfig::default()
        },
        gwt::load_issue_monitor_prefs(&monitor_prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    monitor.complete_active_launch(42, "tab-1::other-window");
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor.prefs()).expect("save prefs");

    // Baseline then a delta wake.
    assert!(runtime
        .pm_wake_decision_at(&repo, &[], "2026-08-10T01:00:00Z")
        .is_none());
    let escalated = [pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)];
    assert!(runtime
        .pm_wake_decision_at(&repo, &escalated, "2026-08-10T01:01:00Z")
        .is_some());

    // The periodic wake in the same window must stand down.
    assert!(
        runtime
            .pm_periodic_wake_decision_at(&repo, "2026-08-10T01:01:00Z")
            .is_none(),
        "the delta wake's stamp must suppress the periodic wake"
    );
}

#[test]
fn live_pm_pane_fixture_drains_input_and_reaps_its_child_at_scope_exit() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_repo, mut runtime, window_id) = pm_wake_fixture(&temp);
    let pty = {
        let _fixture = attach_live_pm_pane(&mut runtime, &window_id);
        let pty = runtime.runtimes[&window_id].pty.clone();
        // Cross the macOS canonical queue limit without submitting a line.
        pty.write_input(&[b'x'; 4096]).expect("drain unsent input");
        pty
    };
    let reaped = pty.try_wait().expect("probe fixture child").is_some();
    // Keep a failing regression run from leaving the old fixture's child alive.
    pty.kill().expect("cleanup fixture child");
    assert!(
        reaped,
        "the fixture must reap its child before returning to the suite"
    );
}

/// Issue #3702 AC-1/AC-3: typing into the PM composer must not let a
/// scheduled supervision tick splice `[gwt] ...` into the unsent line.
#[test]
fn periodic_wake_does_not_inject_while_pm_pane_has_unsent_input() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    seed_quiet_standing_supervision(&repo);
    let _pm_pane = attach_live_pm_pane(&mut runtime, &pm_window_id);

    let compose = runtime.terminal_input_events(&pm_window_id, "ちゃんとbunx/npxで実行されてい");
    assert!(compose.is_empty());
    assert!(
        runtime
            .runtimes
            .get(&pm_window_id)
            .expect("live PM pane")
            .pane
            .lock()
            .expect("pane lock")
            .has_unsent_user_input(),
        "the typed prefix must remain unsent"
    );

    let _ = runtime.pm_periodic_wake_events_at(&repo, "2026-08-10T01:00:00Z");
    assert_pm_pane_is_not_in_protected_inject(&runtime, &pm_window_id);
    assert!(
        runtime
            .runtimes
            .get(&pm_window_id)
            .expect("live PM pane")
            .pane
            .lock()
            .expect("pane lock")
            .has_unsent_user_input(),
        "holding the tick must not consume the user's unsent composer"
    );
}

/// Issue #3702 AC-1: the Issue Monitor activity wake uses the same inject
/// path and must also wait while the user is mid-prompt.
#[test]
fn issue_monitor_activity_wake_does_not_inject_while_pm_pane_has_unsent_input() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    seed_quiet_standing_supervision(&repo);
    let _pm_pane = attach_live_pm_pane(&mut runtime, &pm_window_id);

    let baseline = [pm_wake_inbox_item(41, gwt::MonitorInboxState::Queued)];
    assert!(runtime.pm_wake_events(&repo, &baseline).is_empty());

    let _ = runtime.terminal_input_events(&pm_window_id, "PMはあなた自身が全てを");
    let escalated = [
        pm_wake_inbox_item(41, gwt::MonitorInboxState::Queued),
        pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman),
    ];
    let _ = runtime.pm_wake_events(&repo, &escalated);
    assert_pm_pane_is_not_in_protected_inject(&runtime, &pm_window_id);
}

/// Issue #3702 AC-4: an idle composer still receives the tick immediately.
#[test]
fn periodic_wake_injects_immediately_when_the_pm_composer_is_empty() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    seed_quiet_standing_supervision(&repo);
    let _pm_pane = attach_live_pm_pane(&mut runtime, &pm_window_id);

    let _ = runtime.pm_periodic_wake_events_at(&repo, "2026-08-10T01:00:00Z");
    drain_pm_wake_delivery_tasks(&mut runtime);
    let pty = runtime
        .runtimes
        .get(&pm_window_id)
        .expect("live PM pane")
        .pane
        .lock()
        .expect("pane lock")
        .shared_pty();
    match pty.reserve_input_transaction() {
        Ok(_) => panic!("idle delivery must start the protected inject"),
        Err(error) => assert!(
            error
                .to_string()
                .contains("another protected PTY input transaction is active"),
            "{error}"
        ),
    }
}

/// Issue #3702 AC-2: a held tick is delivered once (coalesced) after submit.
#[test]
fn held_supervision_tick_is_delivered_after_the_composer_submits() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    seed_quiet_standing_supervision(&repo);
    let _pm_pane = attach_live_pm_pane(&mut runtime, &pm_window_id);

    let _ = runtime.terminal_input_events(&pm_window_id, "実行されてい");
    let _ = runtime.pm_periodic_wake_events_at(&repo, "2026-08-10T01:00:00Z");
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .len(),
        1
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .get(&pm_window_id)
            .is_some_and(|decision| decision.prompt.contains("Scheduled supervision tick")),
        "the held prompt must be the scheduled tick"
    );

    // A second tick while still composing stays one pending entry.
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("re-quiet the loop");
    let _ = runtime.pm_periodic_wake_events_at(&repo, "2026-08-10T01:05:00Z");
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .len(),
        1,
        "ticks must coalesce"
    );

    let _ = runtime.terminal_input_events(&pm_window_id, "ますか？\r");
    drain_pm_wake_delivery_tasks(&mut runtime);
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .is_empty(),
        "submit must deliver and clear the held tick"
    );
    let pty = runtime
        .runtimes
        .get(&pm_window_id)
        .expect("live PM pane")
        .pane
        .lock()
        .expect("pane lock")
        .shared_pty();
    match pty.reserve_input_transaction() {
        Ok(_) => panic!("the held tick must inject after submit"),
        Err(error) => assert!(
            error
                .to_string()
                .contains("another protected PTY input transaction is active"),
            "{error}"
        ),
    }
}

/// Issue #3702 AC-2: clearing the composer (Ctrl+C) also releases the tick.
#[test]
fn held_supervision_tick_is_delivered_after_the_composer_is_cleared() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    seed_quiet_standing_supervision(&repo);
    let _pm_pane = attach_live_pm_pane(&mut runtime, &pm_window_id);

    let _ = runtime.terminal_input_events(&pm_window_id, "途中の入力");
    let _ = runtime.pm_periodic_wake_events_at(&repo, "2026-08-10T01:00:00Z");
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .len(),
        1
    );

    let _ = runtime.terminal_input_events(&pm_window_id, "\u{0003}");
    drain_pm_wake_delivery_tasks(&mut runtime);
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .is_empty(),
        "Ctrl+C must deliver the held tick"
    );
}

/// Issue #3505: the scheduled tick drives the fence-aware local monitor for
/// enabled projects and stays silent for disabled ones — this is the scan
/// cadence production otherwise does not have.
#[test]
fn scheduled_tick_scans_enabled_projects_and_skips_disabled_ones() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    disable_pm_auto_start(&repo);

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);

    // Disabled: Tao still enqueues because checking prefs there would itself
    // be blocking I/O. The worker must observe disabled state and commit no
    // scan effects.
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: false,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed disabled prefs");
    let (spawner, scan_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let disabled_prefs = fs::read(&prefs_path).expect("disabled prefs before scan");
    super::super::reset_local_issue_monitor_fallback_commit_count();
    let events = runtime.issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z");
    assert!(
        events.is_empty(),
        "the Tao tick only schedules background work"
    );
    let (completed_root, completed_prefs, completed_at, outcome) =
        run_scheduled_scan_to_completion(&scan_tasks, &recorded);
    assert!(matches!(
        &outcome,
        Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon)
    ));
    assert!(
        runtime
            .complete_scheduled_scan_for_test(
                &completed_root,
                &completed_prefs,
                &completed_at,
                outcome,
            )
            .is_empty(),
        "a disabled monitor produces no projection events"
    );
    assert_eq!(super::super::local_issue_monitor_fallback_commit_count(), 0);
    assert_eq!(
        fs::read(&prefs_path).expect("disabled prefs after scan"),
        disabled_prefs,
        "the disabled worker commits no scan effects"
    );

    // Enabled: the tick drives the local monitor commit path.
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let events = runtime.issue_monitor_scheduled_tick_events_at("2026-08-10T01:05:00Z");
    assert!(
        events.is_empty(),
        "the tao tick only schedules background work"
    );
    let (completed_root, completed_prefs, completed_at, outcome) =
        run_scheduled_scan_to_completion(&scan_tasks, &recorded);
    let events = runtime.complete_scheduled_scan_for_test(
        &completed_root,
        &completed_prefs,
        &completed_at,
        outcome,
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::IssueMonitorStatus { .. })),
        "the tick must produce a monitor status snapshot for the enabled project"
    );
}
