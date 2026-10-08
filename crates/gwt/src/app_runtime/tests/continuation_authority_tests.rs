use super::*;

#[test]
fn continue_work_operation_binding_cannot_be_poisoned_by_another_work() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().join("repo"),
            ProjectKind::NonRepo,
            &[],
        )],
        Some("tab-1"),
    );
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_outcomes
        .insert(
            "immutable-operation".to_string(),
            CachedContinueWorkOutcome {
                work_id: "work-a".to_string(),
                outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                message: None,
                error_code: None,
                retryable: false,
            },
        );

    let conflict = runtime.continue_work_events(
        &runtime.test_context(),
        "client-b",
        "immutable-operation".to_string(),
        "work-b".to_string(),
        canvas_bounds(),
    );
    assert!(conflict.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            ..
        } if work_id == "work-b" && code == "operation_conflict"
    )));

    let replay = runtime.continue_work_events(
        &runtime.test_context(),
        "client-a",
        "immutable-operation".to_string(),
        "work-a".to_string(),
        canvas_bounds(),
    );
    assert!(replay.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            error_code: None,
            ..
        } if work_id == "work-a"
    )));
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_outcomes
            .get("immutable-operation")
            .map(|cached| cached.work_id.as_str()),
        Some("work-a")
    );
}

#[test]
fn continue_work_handoff_context_carries_safe_lineage_and_redacts_private_text() {
    let context = WorkspaceResumeContext {
        title: Some("Continue safe Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: Some(
            "Authorization: Bearer private-secret at /Users/alice/private/repo".to_string(),
        ),
        next_action: Some(
            "Open https://user:password@example.com/private?token=hidden".to_string(),
        ),
    };
    let predecessor = gwt_agent::ExecutionBindingIdentity {
        generation_id: "generation-predecessor".to_string(),
        binding_id: "binding-predecessor".to_string(),
        ledger_head_hash: "head-private".to_string(),
    };

    let handoff = super::super::continuation::handoff_context(&context, "work-safe", &predecessor);

    assert!(handoff.contains("Work: work-safe"));
    assert!(handoff.contains("Predecessor generation: generation-predecessor"));
    assert!(handoff.contains("Predecessor binding: binding-predecessor"));
    for private in [
        "private-secret",
        "/Users/alice/private/repo",
        "user:password",
        "token=hidden",
        "head-private",
    ] {
        assert!(
            !handoff.contains(private),
            "handoff must redact private input and omit ledger heads: {handoff}"
        );
    }
}

#[test]
fn current_codex_session_reconstruction_enables_default_mode_questions_once() {
    let mut session = gwt_agent::Session::new(
        "/tmp/worktree",
        "work/issue-1921",
        gwt_agent::AgentId::Codex,
    );
    session.launch_args = vec!["resume".to_string(), "stored-session".to_string()];

    assert_eq!(
        session.schema_version,
        gwt_agent::Session::CURRENT_SCHEMA_VERSION
    );
    let config = super::super::launch_config_from_persisted_session(&session);

    assert_eq!(
        config
            .args
            .iter()
            .filter(|arg| {
                arg.as_str() == "--config=features.default_mode_request_user_input=true"
            })
            .count(),
        1,
        "current persisted Sessions must rebuild through the canonical Codex launch contract"
    );
}

#[test]
fn continue_work_provider_preflight_distinguishes_present_missing_and_foreign_conversations() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let claude_home = temp.path().join("claude-home");
    let project_store = claude_home.join("projects").join("encoded-project");
    fs::create_dir_all(&project_store).expect("create Claude project store");
    let _claude_home = ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &claude_home);

    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-2359", gwt_agent::AgentId::ClaudeCode);
    session.agent_session_id = Some("conversation-present".to_string());
    fs::write(
        project_store.join("conversation-present.jsonl"),
        format!(
            "{{\"sessionId\":\"conversation-present\",\"cwd\":{}}}\n",
            serde_json::to_string(&worktree.display().to_string()).expect("serialize cwd")
        ),
    )
    .expect("write present transcript");
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Present
    );
    let mut present_config = super::super::launch_config_from_persisted_session(&session);
    assert_eq!(
        super::super::continuation::configure_provider_continuation(&mut present_config, &session),
        gwt::ContinueWorkOutcomeKind::ContinuedConversation
    );
    assert_eq!(present_config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(
        present_config.resume_session_id.as_deref(),
        Some("conversation-present")
    );

    fs::write(
        project_store.join("conversation-without-cwd.jsonl"),
        "{\"sessionId\":\"conversation-without-cwd\"}\n",
    )
    .expect("write transcript without a verifiable cwd");
    session.agent_session_id = Some("conversation-without-cwd".to_string());
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Foreign,
        "a transcript without a verifiable cwd must fail closed"
    );
    let mut no_cwd_config = super::super::launch_config_from_persisted_session(&session);
    assert_eq!(
        super::super::continuation::configure_provider_continuation(&mut no_cwd_config, &session),
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    assert_eq!(no_cwd_config.session_mode, gwt_agent::SessionMode::Normal);
    assert!(no_cwd_config.resume_session_id.is_none());

    session.agent_session_id = Some("conversation-missing".to_string());
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Missing
    );
    let mut missing_config = super::super::launch_config_from_persisted_session(&session);
    assert_eq!(
        super::super::continuation::configure_provider_continuation(&mut missing_config, &session),
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    assert_eq!(missing_config.session_mode, gwt_agent::SessionMode::Normal);
    assert!(missing_config.resume_session_id.is_none());

    let foreign = temp.path().join("foreign-worktree");
    fs::create_dir_all(&foreign).expect("create foreign worktree");
    fs::write(
        project_store.join("conversation-foreign.jsonl"),
        format!(
            "{{\"sessionId\":\"conversation-foreign\",\"cwd\":{}}}\n",
            serde_json::to_string(&foreign.display().to_string()).expect("serialize foreign cwd")
        ),
    )
    .expect("write foreign transcript");
    session.agent_session_id = Some("conversation-foreign".to_string());
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Foreign
    );

    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Unknown
    );
}

/// Issue #3716 AC-2: Grok Build exact Resume is safe only when its official
/// session summary exists for the same native id and worktree cwd.
#[test]
fn continue_work_provider_preflight_recognizes_grok_session_storage() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let grok_home = temp.path().join("grok-home");
    let conversation_id = "01a0195c-fbd7-7352-8d29-da6f6f755010";
    let session_dir = grok_home
        .join("sessions")
        .join("%2Ftmp%2Fworktree")
        .join(conversation_id);
    fs::create_dir_all(&session_dir).expect("create Grok session store");
    fs::write(
        session_dir.join("summary.json"),
        serde_json::json!({
            "info": {
                "id": conversation_id,
                "cwd": worktree,
            }
        })
        .to_string(),
    )
    .expect("write Grok summary");
    let _grok_home = ScopedEnvVar::set("GROK_HOME", &grok_home);
    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-3716", gwt_agent::AgentId::GrokBuild);
    session.agent_session_id = Some(conversation_id.to_string());

    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Missing,
        "a Grok summary without its authoritative updates log cannot resume",
    );
    fs::write(
        session_dir.join("updates.jsonl"),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": conversation_id, "update": {"sessionUpdate": "agent_message_chunk"}}
        })
        .to_string()
            + "\n",
    )
    .expect("write Grok updates");
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Present,
    );

    fs::write(
        session_dir.join("summary.json"),
        serde_json::json!({
            "info": {
                "id": "different-native-session",
                "cwd": worktree,
            }
        })
        .to_string(),
    )
    .expect("replace Grok summary with foreign id");
    assert_eq!(
        super::super::continuation::provider_conversation_availability(&session),
        super::super::continuation::ProviderConversationAvailability::Foreign,
    );
}

/// Issue #3716: ordinary Continue Work must inspect the Grok store from the
/// active launch Profile, with relative GROK_HOME resolved from the child cwd.
#[test]
fn continue_work_grok_preflight_uses_the_active_profile_environment() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _wrong_process_home = ScopedEnvVar::set("GROK_HOME", temp.path().join("wrong-store"));
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab("tab-1", "Repo", worktree.clone(), ProjectKind::Git, &[]);
    let (runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let profile_path = runtime
        .profile_config_path
        .as_deref()
        .expect("fixture profile config path");
    let mut settings = Settings::default();
    settings
        .profiles
        .set_env_var("default", "GROK_HOME", "relative-grok-home")
        .expect("set relative Profile Grok home");
    write_profile_config(profile_path, &settings);

    let conversation_id = "01a0195c-fbd7-7352-8d29-da6f6f755016";
    let profile_grok_home = worktree.join("relative-grok-home");
    let session_dir = profile_grok_home
        .join("sessions/%2Ffixture%2Fworktree")
        .join(conversation_id);
    fs::create_dir_all(&session_dir).expect("create Profile Grok session store");
    fs::write(
        session_dir.join("summary.json"),
        serde_json::json!({"info":{"id":conversation_id,"cwd":worktree}}).to_string(),
    )
    .expect("write summary");
    fs::write(
        session_dir.join("updates.jsonl"),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": conversation_id, "update": {"sessionUpdate": "agent_message_chunk"}}
        })
        .to_string()
            + "\n",
    )
    .expect("write updates");
    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-3716", gwt_agent::AgentId::GrokBuild);
    session.agent_session_id = Some(conversation_id.to_string());

    let resolved = runtime
        .active_profile_grok_home_for_continuation(&session, &worktree)
        .expect("resolve active Profile Grok home");
    assert_eq!(resolved, profile_grok_home);
    let mut config = super::super::launch_config_from_persisted_session(&session);
    assert_eq!(
        super::super::continuation::configure_provider_continuation_with_grok_home(
            &mut config,
            &session,
            Some(&resolved),
        ),
        gwt::ContinueWorkOutcomeKind::ContinuedConversation,
    );
    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(config.resume_session_id.as_deref(), Some(conversation_id));
}

#[test]
fn persisted_direct_session_observed_version_does_not_pin_restore() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut session = gwt_agent::Session::new(
        temp.path(),
        "work/issue-3894",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.tool_version = Some("2.1.156".into());
    session.launch_command = "/opt/bin/claude".into();
    let config = super::super::launch_config_from_persisted_session(&session);
    assert_eq!(config.command, "claude");
    assert_eq!(config.tool_version, None);
}

/// SPEC-1921 AS-1921-D (AC-1921-L6): a Session saved while a version could
/// still be selected restores onto the resolved executable. The stored
/// selector is not read, so `latest` and pinned versions cannot bring the
/// package runner back.
#[test]
fn persisted_session_with_legacy_version_selector_restores_the_resolved_executable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let cases = [
        (
            gwt_agent::AgentId::Codex,
            gwt_agent::LaunchRuntimeTarget::Docker,
            "latest",
            "bunx",
            vec!["@openai/codex@latest"],
            "codex",
        ),
        (
            gwt_agent::AgentId::OpenCode,
            gwt_agent::LaunchRuntimeTarget::Host,
            "1.4.0",
            "npx",
            vec!["--yes", "opencode-ai@1.4.0"],
            "opencode",
        ),
        (
            gwt_agent::AgentId::ClaudeCode,
            gwt_agent::LaunchRuntimeTarget::Host,
            "2.1.156",
            "npx",
            vec!["--yes", "@anthropic-ai/claude-code@2.1.156"],
            "claude",
        ),
    ];
    for (agent_id, runtime_target, selector, launch_command, launch_args, command) in cases {
        let mut session = gwt_agent::Session::new(temp.path(), "work/issue-1921", agent_id);
        session.runtime_target = runtime_target;
        session.tool_version = Some(selector.to_string());
        session.tool_version_selector = Some(selector.to_string());
        session.launch_command = launch_command.to_string();
        session.launch_args = launch_args.into_iter().map(str::to_string).collect();

        let config = super::super::launch_config_from_persisted_session(&session);

        assert_eq!(config.command, command, "{:?}", config.args);
        assert!(
            config.args.iter().all(|arg| !arg.contains(selector)),
            "the stored selector must not reach the launch: {:?}",
            config.args
        );
        assert_eq!(config.tool_version, None, "{command}");
    }
}

#[test]
fn continue_work_without_durable_session_starts_projection_only_handoff() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-success",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: true,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let canonical_project_root = crate::runtime_support::normalize_recent_project_path(&repo);
    runtime
        .tab_mut("tab-1")
        .expect("projection-only tab")
        .project_root = repo.clone();
    assert!(
        !runtime
            .sessions_dir
            .join("historical-session.toml")
            .exists(),
        "fixture must not provide durable Session metadata"
    );

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-only",
        "projection-only-operation".to_string(),
        work_id.clone(),
        canvas_bounds(),
    );

    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            error_code: Some(code),
            ..
        } if code == "session_metadata_missing"
    )));
    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "projection-only-operation")
        .expect("projection-only fallback must prepare one pending continuation");
    assert_eq!(pending.work_id, work_id);
    assert_eq!(
        pending.project_root, canonical_project_root,
        "a direct-pick linked-worktree tab must still use the canonical workspace home for execution authority"
    );
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    assert!(matches!(
        &pending.execution,
        PendingContinueWorkExecution::Successor(request)
            if request.source == "continue-work:handoff"
                && request.initial_session_id == pending.binding.session_id
    ));
    let persisted = runtime.tab("tab-1").expect("tab").workspace.persisted();
    let windows = &persisted.windows;
    assert_eq!(
        windows.len(),
        1,
        "fallback must materialize one candidate pane"
    );
    assert_eq!(windows[0].agent_id.as_deref(), Some("codex"));
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read projection-only ledger")
        .expect("projection-only ledger");
    assert_eq!(ledger.continuation_attempts.len(), 1);
}

#[test]
fn projection_only_continue_uses_spec_generation_authority_without_issue_cache() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, _repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-spec-authority",
        ProjectionOnlyContinueFixture {
            work_owner: Some("3248"),
            owner_number: 3248,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-spec",
        "projection-spec-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "projection-spec-operation")
        .expect("SPEC authority must prepare projection-only continuation");
    assert_eq!(pending.owner, owner);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
}

#[test]
fn projection_only_continue_uses_spec_legacy_flat_authority_without_issue_cache() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, _repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-spec-legacy-flat-authority",
        ProjectionOnlyContinueFixture {
            work_owner: Some("SPEC #3248"),
            owner_number: 3248,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: true,
        },
    );

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-spec-legacy-flat",
        "projection-spec-legacy-flat-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "projection-spec-legacy-flat-operation")
        .expect("legacy flat SPEC authority must prepare continuation");
    assert_eq!(pending.owner, owner);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
}

#[test]
fn projection_only_continue_matches_canonical_branch_for_durable_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-canonical-branch",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let mut branch_alias = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Update,
        &work_id,
        chrono::Utc::now() + chrono::Duration::seconds(1),
    );
    branch_alias.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("origin/work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    gwt_core::workspace_projection::record_workspace_work_event(&project_root, branch_alias)
        .expect("record canonical branch alias");
    let mut durable = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    durable.id = "historical-session".to_string();
    durable.display_name = "Durable branch alias settings".to_string();
    durable.project_state_root = Some(project_root);
    durable.linked_issue_number = Some(owner.number);
    durable
        .save(&runtime.sessions_dir)
        .expect("save canonical-branch durable Session");

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-canonical-branch",
        "projection-canonical-branch-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    let _pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "projection-canonical-branch-operation")
        .expect("canonical branch alias must use the durable Session");
    let window = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .first()
        .expect("candidate window");
    assert_eq!(window.agent_id.as_deref(), Some("codex"));
    assert_eq!(window.title, "Durable branch alias settings");
}

#[test]
fn linked_workspace_resume_routes_through_authority_producing_continuation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "linked-resume-producing",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let mut durable = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    durable.id = "historical-session".to_string();
    durable.display_name = "Authority-producing Resume".to_string();
    durable.project_state_root = Some(project_root);
    durable.linked_issue_number = Some(owner.number);
    durable
        .save(&runtime.sessions_dir)
        .expect("save linked durable Session");

    let events = runtime.resume_workspace_agent_events(
        &runtime.test_context(),
        "resume-client",
        "linked-resume-operation".to_string(),
        durable.id.clone(),
        None,
        canvas_bounds(),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentStarted {
            operation_id,
            session_id,
            ..
        } if operation_id == "linked-resume-operation" && session_id == "historical-session"
    )));
    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "linked-resume-operation")
        .expect("linked Resume must prepare continuation authority");
    assert_eq!(pending.work_id, work_id);
    assert!(matches!(
        &pending.execution,
        PendingContinueWorkExecution::Successor(request)
            if request.initial_session_id == pending.binding.session_id
    ));
    assert_eq!(pending.binding.owner_number, owner.number);
}

#[test]
fn continue_work_prefers_exact_custom_durable_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_agent = write_fake_agent_command(temp.path(), "review-bot");
    let _path = prepend_tool_parent_to_path(&fake_agent);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-custom-durable",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("review-bot"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let mut durable = gwt_agent::Session::new(
        &repo,
        "work/issue-2359",
        gwt_agent::AgentId::Custom("review-bot".to_string()),
    );
    durable.id = "historical-session".to_string();
    durable.display_name = "Exact custom durable settings".to_string();
    durable.project_state_root = Some(project_root);
    durable.linked_issue_number = Some(owner.number);
    durable
        .save(&runtime.sessions_dir)
        .expect("save exact custom durable Session");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-custom-durable",
        "custom-durable-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            error_code: Some(code),
            ..
        } if code == "agent_identity_unsupported"
    )));
    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "custom-durable-operation")
        .expect("exact custom durable Session must be preferred");
    assert_eq!(
        pending.work_agent_session_id.as_deref(),
        Some("historical-session")
    );
    assert_eq!(
        pending.work_agent_id,
        gwt_agent::AgentId::Custom("review-bot".to_string())
    );
}

#[test]
fn continue_work_durable_session_requires_same_work_ref_agent() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-ref-agent-mismatch",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Claude Code"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let mut latest = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Update,
        &work_id,
        chrono::Utc::now() + chrono::Duration::seconds(2),
    );
    latest.agent_session_id = Some("latest-codex-session".to_string());
    latest.agent_id = Some("Codex".to_string());
    latest.display_name = Some("Codex".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&project_root, latest)
        .expect("record latest Codex Work ref");
    let mut mismatched =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    mismatched.id = "historical-session".to_string();
    mismatched.display_name = "Mismatched durable settings".to_string();
    mismatched.project_state_root = Some(project_root);
    mismatched.linked_issue_number = Some(owner.number);
    mismatched
        .save(&runtime.sessions_dir)
        .expect("save ref-mismatched durable Session");

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-ref-agent-mismatch",
        "ref-agent-mismatch-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "ref-agent-mismatch-operation")
        .expect("latest projection agent must still provide handoff fallback");
    assert_eq!(pending.work_agent_session_id, None);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
}

#[test]
fn continue_work_missing_work_agent_identity_rejects_durable_codex_and_custom_sessions_without_mutation(
) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let fake_custom = write_fake_agent_command(temp.path(), "review-bot");
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let _custom_path = prepend_tool_parent_to_path(&fake_custom);

    for (case_name, agent_id) in [
        ("missing-work-agent-codex", gwt_agent::AgentId::Codex),
        (
            "missing-work-agent-custom",
            gwt_agent::AgentId::Custom("review-bot".to_string()),
        ),
    ] {
        let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
            temp.path(),
            case_name,
            ProjectionOnlyContinueFixture {
                work_owner: Some("Issue #2359"),
                owner_number: 2359,
                owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                agent_id: None,
                cross_worktree: false,
                missing_container: false,
                conflicting_owner: false,
                conflicting_container: false,
                conflicting_branch_only: false,
                conflicting_agent: false,
                legacy_flat_only: false,
            },
        );
        let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
        let mut durable = gwt_agent::Session::new(&repo, "work/issue-2359", agent_id.clone());
        durable.id = "historical-session".to_string();
        durable.project_state_root = Some(project_root.clone());
        durable.linked_issue_number = Some(owner.number);
        durable
            .save(&runtime.sessions_dir)
            .expect("save durable Session without authenticated Work Agent identity");
        let session_path = runtime.sessions_dir.join("historical-session.toml");
        let session_before = fs::read(&session_path).expect("read Session before");
        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let work_items_before = fs::read(&work_items_path).expect("read Work projection before");
        let work_events_before = tracked_work_event_store_snapshot(&project_root);
        let ledger_before = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read authority before")
            .expect("authority ledger before");
        let authority_before =
            snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));

        let events = runtime.continue_work_events(
            &runtime.test_context(),
            "client-missing-work-agent",
            format!("operation-{case_name}"),
            work_id,
            canvas_bounds(),
        );

        assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::Failed,
                    error_code: Some(code),
                    retryable: false,
                    ..
                } if code == "agent_identity_missing"
            )),
            "{case_name}: {events:#?}"
        );
        assert!(runtime.pending_continue_work.is_empty(), "{case_name}");
        assert_eq!(
            runtime
                .tab("tab-1")
                .expect("tab")
                .workspace
                .persisted()
                .windows
                .len(),
            0,
            "{case_name} must not materialize a pane",
        );
        assert_eq!(
            fs::read(&session_path).expect("read Session after"),
            session_before
        );
        assert_eq!(
            fs::read(&work_items_path).expect("read Work projection after"),
            work_items_before,
        );
        assert_eq!(
            tracked_work_event_store_snapshot(&project_root),
            work_events_before,
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(&repo, owner)
                .expect("read authority after")
                .expect("authority ledger after"),
            ledger_before,
        );
        assert_optional_files_unchanged(&authority_before);
    }
}

#[test]
fn projection_continue_rejects_actual_worktree_branch_divergence_without_mutation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-actual-branch-divergence",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    run_git(&repo, &["branch", "-M", "work/divergent-current"]);
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
    let work_items_before = fs::read(&work_items_path).expect("read Work projection before");
    let work_events_before = tracked_work_event_store_snapshot(&project_root);
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let workspace_before =
        serde_json::to_vec(runtime.tab("tab-1").expect("tab").workspace.persisted())
            .expect("serialize Workspace before");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-branch-divergence",
        "operation-branch-divergence".to_string(),
        work_id,
        canvas_bounds(),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                retryable: true,
                ..
            }
        )),
        "actual branch divergence must conflict: {events:#?}"
    );
    assert!(runtime.pending_continue_work.is_empty());
    assert_eq!(
        serde_json::to_vec(runtime.tab("tab-1").expect("tab").workspace.persisted())
            .expect("serialize Workspace after"),
        workspace_before,
    );
    assert_eq!(
        fs::read(&work_items_path).expect("read Work projection after"),
        work_items_before
    );
    assert_eq!(
        tracked_work_event_store_snapshot(&project_root),
        work_events_before
    );
    assert_optional_files_unchanged(&authority_before);
}

#[test]
fn continue_work_shell_ref_cannot_authenticate_agent_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-shell-ref",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let mut injected = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    injected.id = "historical-session".to_string();
    injected.project_state_root = Some(project_root);
    injected.linked_issue_number = Some(owner.number);
    injected
        .save(&runtime.sessions_dir)
        .expect("save Session behind Shell ref");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-shell-ref",
        "shell-ref-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    assert!(runtime.pending_continue_work.is_empty());
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            error_code: Some(code),
            retryable: false,
            ..
        } if code == "agent_identity_missing"
    )));
}

#[test]
fn projection_only_continue_ignores_noncanonical_projected_session_path() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-noncanonical-session-path",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let mut injected_agent = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Update,
        &work_id,
        chrono::Utc::now() + chrono::Duration::seconds(1),
    );
    injected_agent.agent_session_id = Some("../outside".to_string());
    injected_agent.agent_id = Some("Codex".to_string());
    injected_agent.display_name = Some("Codex".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&project_root, injected_agent)
        .expect("record noncanonical projected Session id");
    let sessions_parent = runtime.sessions_dir.parent().expect("Sessions parent");
    let mut outside = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    outside.id = "outside".to_string();
    outside.display_name = "Injected outside Session settings".to_string();
    outside.project_state_root = Some(project_root);
    outside.linked_issue_number = Some(owner.number);
    outside
        .save(sessions_parent)
        .expect("save outside Session-shaped file");

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-noncanonical-session",
        "projection-noncanonical-session-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    let window = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .first()
        .expect("projection fallback candidate");
    assert_ne!(window.title, "Injected outside Session settings");
    assert_eq!(window.agent_id.as_deref(), Some("codex"));
}

#[test]
fn projection_only_continue_reconciles_prepared_spec_candidate_without_work_state() {
    projection_only_continue_prepared_spec_recovery_case(false);
}

#[test]
fn durable_continue_recovery_rejects_correlated_session_on_divergent_branch_without_mutation() {
    projection_only_continue_prepared_spec_recovery_case(true);
}

#[test]
fn durable_continue_recovery_rejects_foreign_worktree_with_matching_project_root() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("active-project");
    let foreign = temp.path().join("foreign-project");
    fs::create_dir_all(&project_root).expect("create active project");
    fs::create_dir_all(&foreign).expect("create foreign project");
    init_repo(&project_root);
    init_repo(&foreign);
    run_git(
        &project_root,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let predecessor_session_id = "foreign-predecessor";
    let candidate_session_id = "foreign-candidate";
    let work_id = "work-foreign-recovery";
    let operation_id = "foreign-recovery-operation";
    gwt::cli::execution_state::materialize_at_launch(
        &foreign,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize foreign predecessor");
    gwt::cli::execution_state::settle(
        &foreign,
        predecessor_session_id,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    )
    .expect("settle foreign predecessor");
    gwt::cli::execution_state::ensure_generation_ledger(
        &foreign,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import foreign predecessor");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(work_id.to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "foreign-candidate-binding".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_successor(&foreign, owner, &request)
        .expect("prepare foreign continuation");
    let foreign_authority_before = current_generation_authority_artifacts(&foreign)
        .into_iter()
        .map(|path| {
            let bytes =
                fs::read(&path).expect("read foreign authority artifact before Continue work");
            (path, bytes)
        })
        .collect::<Vec<_>>();

    let now = chrono::Utc::now();
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    start.title = Some("Active project Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(project_root.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&project_root, start)
        .expect("record active Work");
    gwt_core::workspace_projection::record_workspace_work_event(
        &project_root,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            work_id,
            now + chrono::Duration::seconds(1),
        ),
    )
    .expect("settle active Work");

    let tab = sample_project_tab(
        "tab-foreign",
        "Active",
        project_root.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(&temp.path().join("runtime"), vec![tab], Some("tab-foreign"));
    let mut forged =
        gwt_agent::Session::new(&foreign, "work/issue-2359", gwt_agent::AgentId::Codex);
    forged.id = predecessor_session_id.to_string();
    forged.project_state_root = Some(project_root);
    forged.linked_issue_number = Some(owner.number);
    forged
        .save(&runtime.sessions_dir)
        .expect("save foreign Session with matching project root");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-foreign-recovery",
        operation_id.to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                error_code: Some(code),
                retryable: false,
                ..
            } if code == "agent_identity_missing"
        )),
        "foreign durable authority must not replace the active Work failure: {events:?}"
    );
    let attempt = gwt::cli::execution_state::continuation_attempt_for_operation(
        &foreign,
        owner,
        operation_id,
    )
    .expect("read foreign attempt")
    .expect("foreign attempt");
    assert_eq!(
        attempt.status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
        "foreign attempt must remain Prepared"
    );
    for (path, before) in foreign_authority_before {
        assert_eq!(
            fs::read(&path).expect("read foreign authority artifact after Continue work"),
            before,
            "foreign authority artifact must remain byte-identical: {}",
            path.display()
        );
    }
}

#[test]
fn projection_only_continue_ignores_durable_session_with_conflicting_owner() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-conflicting-session-owner",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let mut stale = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    stale.id = "historical-session".to_string();
    stale.project_state_root = Some(repo.clone());
    stale.linked_issue_number = Some(3248);
    stale
        .save(&runtime.sessions_dir)
        .expect("save conflicting durable Session");

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-conflicting-session",
        "projection-conflicting-session-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "projection-conflicting-session-operation")
        .expect("conflicting Session must not block authenticated projection fallback");
    assert_eq!(pending.owner, owner);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    assert!(matches!(
        &pending.execution,
        PendingContinueWorkExecution::Successor(request)
            if request.source == "continue-work:handoff"
    ));
}

#[test]
fn projection_only_continue_authenticates_container_before_using_durable_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-session-hidden-container-conflict",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: true,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let mut durable = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    durable.id = "historical-session".to_string();
    durable.project_state_root = Some(repo.clone());
    durable.linked_issue_number = Some(owner.number);
    durable
        .save(&runtime.sessions_dir)
        .expect("save matching durable Session");
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read ledger before")
        .expect("ledger before");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-container-first",
        "projection-container-first-operation".to_string(),
        work_id,
        canvas_bounds(),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            ..
        } if code == "execution_container_ambiguous"
    )));
    assert!(runtime.pending_continue_work.is_empty());
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read ledger after")
            .expect("ledger after"),
        before_ledger
    );
    assert!(
        runtime
            .sessions_dir
            .join("historical-session.toml")
            .exists(),
        "rejection must not mutate the pre-existing durable Session"
    );
}

#[test]
fn projection_only_continue_spawn_failure_aborts_without_committing_candidate_state() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-spawn-failure",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
    let before_work_items = fs::read(&work_items_path).expect("read Work projection before");
    let before_work_events = tracked_work_event_store_snapshot(&project_root);
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read ledger before")
        .expect("ledger before");

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-spawn-failure",
        "projection-spawn-failure-operation".to_string(),
        work_id,
        canvas_bounds(),
    );
    let window_id = runtime
        .pending_continue_work
        .iter()
        .find(|(_, pending)| pending.operation_id == "projection-spawn-failure-operation")
        .map(|(window_id, _)| window_id.clone())
        .expect("projection fallback must prepare candidate");

    let events = runtime.handle_launch_complete_and_drain(
        window_id,
        Err("simulated projection-only spawn failure".to_string()),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                ..
            } if code == "launch_failed"
        )),
        "unexpected spawn failure events: {events:#?}"
    );
    assert!(runtime.pending_continue_work.is_empty());
    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "aborted candidate pane must be removed"
    );
    assert_eq!(
        fs::read(&work_items_path).expect("read Work projection after"),
        before_work_items
    );
    assert_eq!(
        tracked_work_event_store_snapshot(&project_root),
        before_work_events
    );
    assert!(
        fs::read_dir(&runtime.sessions_dir)
            .expect("read Sessions after")
            .filter_map(Result::ok)
            .all(|entry| entry.path().extension().and_then(|value| value.to_str()) != Some("toml")),
        "aborted candidate must leave no Session TOML"
    );
    let after_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read ledger after")
        .expect("ledger after");
    assert_eq!(after_ledger.generations, before_ledger.generations);
    assert_eq!(
        after_ledger.current_generation_id,
        before_ledger.current_generation_id
    );
    assert!(after_ledger.continuation_attempts.iter().any(|attempt| {
        attempt.request.operation_id == "projection-spawn-failure-operation"
            && attempt.status == gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
    }));
}

#[test]
fn projection_only_continue_owner_change_before_activation_aborts_without_committing() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let operation_id = "projection-owner-toctou-operation";
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-owner-toctou",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-owner-toctou",
        operation_id.to_string(),
        work_id.clone(),
        canvas_bounds(),
    );
    let (window_id, pending) = runtime
        .pending_continue_work
        .iter()
        .find(|(_, pending)| pending.operation_id == operation_id)
        .map(|(window_id, pending)| (window_id.clone(), pending.clone()))
        .expect("prepare projection continuation");
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read predecessor ledger")
        .expect("predecessor ledger");
    let mut candidate = gwt_agent::Session::new(
        &pending.worktree_path,
        pending.work_branch.clone(),
        pending.work_agent_id.clone(),
    );
    candidate.id = pending.binding.session_id.clone();
    candidate.project_state_root = Some(pending.project_root.clone());
    candidate.repo_hash = Some(pending.binding.repo_hash.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(pending.binding.clone()))
        .expect("bind candidate Session");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save candidate Session");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = pending.binding.session_id.clone();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = project_root.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue_prepared(&repo, &pending.binding.session_id, pending.binding.clone())
        .expect("issue Prepared capability");
    runtime.agent_capability_issuer = Some(issuer);
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), capability.token);

    let mut owner_change = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Update,
        &work_id,
        chrono::Utc::now() + chrono::Duration::seconds(1),
    );
    owner_change.owner = Some("SPEC #3248".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&project_root, owner_change)
        .expect("change Work owner before activation");
    let work_before_finalize = tracked_work_event_store_snapshot(&project_root);

    let events = runtime
        .finalize_continue_work_session_start(&window_id, Some(pending.readiness_nonce.as_str()));

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                ..
            } if code == "launch_failed"
        )),
        "changed owner must reject activation: {events:?}"
    );
    assert!(!runtime.pending_continue_work.contains_key(&window_id));
    assert!(!runtime
        .sessions_dir
        .join(format!("{}.toml", pending.binding.session_id))
        .exists());
    assert_eq!(
        tracked_work_event_store_snapshot(&project_root),
        work_before_finalize,
        "rejected activation must not overwrite the changed Work authority",
    );
    let after_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read rejected ledger")
        .expect("rejected ledger");
    assert_eq!(after_ledger.generations, before_ledger.generations);
    assert_eq!(
        after_ledger.current_generation_id,
        before_ledger.current_generation_id
    );
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id,)
            .expect("read rejected attempt")
            .expect("rejected attempt")
            .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Aborted,
    );
}

#[test]
fn projection_only_continue_rejects_missing_owner_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-missing-owner",
        ProjectionOnlyContinueFixture {
            work_owner: None,
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "execution_owner_missing",
    );
}

#[test]
fn projection_only_continue_rejects_missing_agent_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-missing-agent",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: None,
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "agent_identity_missing",
    );
}

#[test]
fn projection_only_continue_rejects_missing_container_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-missing-container",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: true,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "execution_container_missing",
    );
}

#[test]
fn projection_only_continue_rejects_conflicting_owner_history_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-conflicting-owner",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: true,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "execution_owner_ambiguous",
    );
}

#[test]
fn projection_only_continue_rejects_multi_owner_text_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-multi-owner-text",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359 / SPEC #3248"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "execution_owner_ambiguous",
    );
}

#[test]
fn projection_only_continue_rejects_conflicting_container_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-conflicting-container",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: true,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "execution_container_ambiguous",
    );
}

#[test]
fn projection_only_continue_rejects_branch_only_container_conflict_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-branch-only-conflict",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: true,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
        "execution_container_ambiguous",
    );
}

// #3426: a same-number kind-only mismatch (Work says `Issue #N`, trusted
// ledger says `spec/N`) previously wedged Continue work behind
// execution_owner_ambiguous even though the trusted authority is unambiguous.
// The trusted owner now wins and the continuation proceeds under it.
#[test]
fn continue_work_heals_issue_kind_work_owner_against_trusted_spec_authority() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        "projection-only-owner-kind-heal",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-owner-kind-heal",
        "owner-kind-heal-operation".to_string(),
        work_id.clone(),
        canvas_bounds(),
    );

    assert!(
        events.iter().all(|event| !matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                error_code: Some(code),
                ..
            } if code == "execution_owner_ambiguous"
        )),
        "a kind-only mismatch against an unambiguous trusted authority must self-heal: {events:?}"
    );
    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == "owner-kind-heal-operation")
        .expect("healed continuation must prepare one pending attempt");
    assert_eq!(pending.work_id, work_id);
    assert_eq!(
        pending.owner, owner,
        "the continuation must run under the trusted spec authority"
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read healed ledger")
        .expect("healed ledger");
    assert_eq!(ledger.continuation_attempts.len(), 1);
}

// #3426: with no trusted generation authority and no cache evidence, a
// Work-declared kind must be retained instead of failing ambiguous or being
// silently downgraded to Issue by the detection default.
#[test]
fn canonical_continue_owner_without_authority_or_evidence_retains_declared_kind() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("plain-project");
    fs::create_dir_all(&project_root).expect("create project root");

    let projected = super::super::continuation::strict_projection_owner("SPEC #4444")
        .expect("strict projection owner");
    let resolved = super::super::continuation::canonical_continue_work_owner(
        &project_root,
        &project_root,
        projected,
    )
    .expect("declared kind must resolve without authority or evidence");
    assert_eq!(
        resolved,
        gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 4444,
        }
    );
}

#[test]
fn projection_only_continue_rejects_conflicting_agent_without_mutation() {
    assert_projection_only_continue_rejection(
        "projection-only-conflicting-agent",
        ProjectionOnlyContinueFixture {
            work_owner: Some("Issue #2359"),
            owner_number: 2359,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: true,
            legacy_flat_only: false,
        },
        "agent_identity_ambiguous",
    );
}

#[test]
fn continue_work_nonlocal_liveness_distinguishes_live_dead_and_stopped_owners() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let session_id = "owner-session";
    let live_sidecar = gwt_agent::runtime_state_path_for_pid(
        &runtime.sessions_dir,
        std::process::id(),
        session_id,
    );
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
        .save(&live_sidecar)
        .expect("save live runtime");
    assert_eq!(
        runtime.classify_nonlocal_active_owner_liveness(session_id),
        ActiveOwnerLiveness::Unknown,
        "a live foreign/current Host PID must never be taken over automatically"
    );
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Stopped)
        .save(&live_sidecar)
        .expect("mark live-Host runtime stopped");
    assert!(matches!(
        runtime.classify_nonlocal_active_owner_liveness(session_id),
        ActiveOwnerLiveness::Stale("all owning Host runtimes are stopped")
    ));

    fs::remove_file(&live_sidecar).expect("remove live runtime");
    let dead_sidecar =
        gwt_agent::runtime_state_path_for_pid(&runtime.sessions_dir, i32::MAX as u32, session_id);
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
        .save(&dead_sidecar)
        .expect("save dead runtime");
    assert!(matches!(
        runtime.classify_nonlocal_active_owner_liveness(session_id),
        ActiveOwnerLiveness::Stale("all owning Host runtimes are dead")
    ));

    fs::remove_file(&dead_sidecar).expect("remove dead runtime");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut stopped = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    stopped.id = session_id.to_string();
    stopped.update_status(gwt_agent::AgentStatus::Stopped);
    stopped
        .save(&runtime.sessions_dir)
        .expect("save stopped Session");
    assert!(matches!(
        runtime.classify_nonlocal_active_owner_liveness(session_id),
        ActiveOwnerLiveness::Stale("durable Session is stopped")
    ));
}

#[test]
fn startup_nonlocal_liveness_indexes_runtime_namespaces_once_for_128_sessions() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let runtime_root = runtime.sessions_dir.join("runtime");
    for pid in 1_000_000..1_000_064_u32 {
        fs::create_dir_all(runtime_root.join(pid.to_string())).expect("create runtime namespace");
    }
    let session_ids = (0..128)
        .map(|index| format!("startup-index-session-{index}"))
        .collect::<Vec<_>>();

    reset_nonlocal_runtime_index_scan_metrics();
    let liveness = runtime
        .classify_nonlocal_active_owner_liveness_batch(session_ids.iter().map(String::as_str));
    let metrics = nonlocal_runtime_index_scan_metrics();

    assert_eq!(liveness.len(), 128);
    assert!(liveness.values().all(|value| matches!(
        value,
        ActiveOwnerLiveness::Stale("durable Session is missing")
    )));
    assert_eq!(metrics.runtime_root_enumerations, 1);
    assert_eq!(metrics.runtime_namespace_enumerations, 64);
    assert_eq!(metrics.host_process_probes, 0);

    for session_id in session_ids.iter().take(2) {
        gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
            .save(&gwt_agent::runtime_state_path_for_pid(
                &runtime.sessions_dir,
                1_000_000,
                session_id,
            ))
            .expect("save relevant dead-Host sidecar");
    }
    reset_nonlocal_runtime_index_scan_metrics();
    let indexed = runtime
        .classify_nonlocal_active_owner_liveness_batch(session_ids.iter().map(String::as_str));
    let metrics = nonlocal_runtime_index_scan_metrics();
    assert!(session_ids.iter().take(2).all(|session_id| matches!(
        indexed.get(session_id),
        Some(ActiveOwnerLiveness::Stale(
            "all owning Host runtimes are dead"
        ))
    )));
    assert_eq!(metrics.runtime_root_enumerations, 1);
    assert_eq!(metrics.runtime_namespace_enumerations, 64);
    assert_eq!(metrics.host_process_probes, 1);
}

#[test]
fn continue_work_rebinds_live_local_legacy_session_without_new_generation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(
        &repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    let session_id = "legacy-live-session";
    let work_id = "work-legacy-live";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize legacy Active projection");

    let now = chrono::Utc::now();
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    start.title = Some("Legacy live Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
        .expect("record legacy live Work");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let runtime_root = temp.path().join(".gwt");
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut durable = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    durable.id = session_id.to_string();
    durable.project_state_root = None;
    durable.repo_hash = detect_repo_hash(&repo).map(|value| value.to_string());
    durable.linked_issue_number = Some(owner.number);
    durable.execution_binding = None;
    durable
        .save(&runtime.sessions_dir)
        .expect("save legacy durable Session");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let inspection = issuer
        .issue(&repo, session_id)
        .expect("issue legacy inspection capability");
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), inspection.token.clone());

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-1",
        "legacy-focus-operation".to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::FocusedExisting,
            retryable: false,
            ..
        }
    )));
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("load imported generation")
        .expect("imported generation");
    assert_eq!(ledger.generations.len(), 1);
    assert!(ledger.continuation_attempts.is_empty());
    assert!(ledger.takeover_attempts.is_empty());
    let current = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read current binding")
        .expect("current binding");
    let rebound = gwt_agent::Session::load_and_migrate(
        &runtime.sessions_dir.join(format!("{session_id}.toml")),
    )
    .expect("read rebound Session")
    .execution_binding
    .expect("legacy Session must be rebound");
    assert_eq!(rebound.identity, current);
    assert!(issuer.active_token_is_current(&inspection.token, &rebound));
}

#[test]
fn continue_work_readiness_nonce_mismatch_aborts_candidate_before_commit() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
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
    let window_id = combined_window_id("tab-1", "agent-1");
    let session_id = "nonce-candidate";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        "predecessor-session",
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "predecessor-session",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import predecessor");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "nonce-operation".to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some("nonce-work".to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "binding-candidate".to_string(),
        initial_session_id: session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare successor");
    let identity =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive successor binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let mut candidate =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate.id = session_id.to_string();
    candidate.project_state_root = Some(repo.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save candidate");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = session_id.to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    runtime.pending_continue_work.insert(
        window_id.clone(),
        PendingContinueWork {
            client_id: "client-1".to_string(),
            operation_id: "nonce-operation".to_string(),
            work_id: "nonce-work".to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: None,
            execution: PendingContinueWorkExecution::Successor(request),
            binding,
            readiness_nonce: "expected-readiness".to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: None,
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: "predecessor-session".to_string(),
            predecessor_binding,
        },
    );

    let events = runtime.finalize_continue_work_session_start(&window_id, Some("wrong-readiness"));

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            ..
        } if code == "launch_failed"
    )));
    assert!(!runtime.pending_continue_work.contains_key(&window_id));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(
        !runtime
            .sessions_dir
            .join(format!("{session_id}.toml"))
            .exists(),
        "nonce mismatch must discard only the exact candidate Session"
    );
}

#[test]
fn continue_work_retry_after_host_crash_aborts_stale_prepared_candidate() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(
        &repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    let predecessor_session_id = "crash-predecessor";
    let candidate_session_id = "crash-candidate";
    let work_id = "work-crash-recovery";
    let operation_id = "continue-crash-operation";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            predecessor_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import predecessor");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(work_id.to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "crash-candidate-binding".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare crash candidate");
    let planned =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive crash candidate binding");

    let now = chrono::Utc::now();
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    start.title = Some("Crash recovery Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start).expect("record Work");
    gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            work_id,
            now + chrono::Duration::seconds(1),
        ),
    )
    .expect("settle Work");

    let runtime_root = temp.path().join(".gwt");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let mut predecessor =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    predecessor.id = predecessor_session_id.to_string();
    predecessor.project_state_root = Some(repo.clone());
    predecessor.repo_hash = detect_repo_hash(&repo).map(|value| value.to_string());
    predecessor.linked_issue_number = Some(owner.number);
    predecessor.agent_session_id = Some("provider-conversation".to_string());
    predecessor
        .save(&runtime.sessions_dir)
        .expect("save predecessor Session");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned,
        capability_generation: 1,
    };
    let mut candidate =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate.id = candidate_session_id.to_string();
    candidate.project_state_root = Some(repo.clone());
    candidate.repo_hash = Some(binding.repo_hash.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate.status = gwt_agent::AgentStatus::Stopped;
    candidate
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate");
    let exact_candidate = candidate.clone();
    let replacement = candidate
        .execution_binding
        .as_mut()
        .expect("candidate execution binding");
    replacement.identity.ledger_head_hash =
        "replacement-ledger-head-with-same-public-ids".to_string();
    replacement.capability_generation += 1;
    candidate
        .save(&runtime.sessions_dir)
        .expect("save stopped replacement candidate");
    let candidate_path = runtime
        .sessions_dir
        .join(format!("{candidate_session_id}.toml"));
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-retry",
        operation_id.to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                retryable: true,
                ..
            }
        )),
        "replacement binding must be retained for explicit reconciliation: {events:?}"
    );
    assert!(candidate_path.exists());
    assert_optional_files_unchanged(&authority_before);
    let attempt =
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id)
            .expect("read crash attempt")
            .expect("crash attempt");
    assert_eq!(
        attempt.status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read crash recovery ledger")
        .expect("crash recovery ledger");
    assert_eq!(ledger.generations.len(), 1);
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Completed)
    );
    fs::remove_file(&candidate_path).expect("remove replacement candidate fixture");
    exact_candidate
        .save(&runtime.sessions_dir)
        .expect("restore exact candidate fixture");

    let reconciled = runtime.continue_work_events(
        &runtime.test_context(),
        "client-retry-exact",
        operation_id.to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );
    assert!(
        reconciled.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                retryable: false,
                ..
            } if code == "continuation_aborted"
        )),
        "exact candidate must finish aborted cleanup: {reconciled:?}"
    );
    assert!(!candidate_path.exists());
    let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work")
        .expect("Work")
        .work_items
        .into_iter()
        .find(|item| item.id == work_id)
        .expect("crash Work");
    assert_eq!(
        work.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done
    );
}

#[test]
fn continue_work_retry_rejects_lingering_work_transaction_for_aborted_attempt() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(
        &repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );

    let predecessor_session_id = "aborted-predecessor";
    let candidate_session_id = "aborted-candidate";
    let work_id = "work-aborted-recovery";
    let operation_id = "continue-aborted-operation";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            predecessor_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import predecessor");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(work_id.to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "aborted-candidate-binding".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare candidate");
    gwt::cli::execution_state::abort_successor(&repo, owner, &request, "simulated Host failure")
        .expect("abort candidate before Work cleanup");

    let now = chrono::Utc::now();
    let mut current =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo.clone());
    current.id = work_id.to_string();
    current.title = "Aborted recovery Work".to_string();
    gwt_core::workspace_projection::save_workspace_projection(&repo, &current)
        .expect("save current Work projection");
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    start.title = Some("Aborted recovery Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start).expect("record Work");
    gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            work_id,
            now + chrono::Duration::seconds(1),
        ),
    )
    .expect("settle Work");

    let staged = gwt_core::workspace_projection::transact_workspace_state_with_commit(
        &repo,
        operation_id,
        |projection, _, _| {
            projection.id = work_id.to_string();
            projection.title = "staged aborted continuation".to_string();
            let mut event = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Update,
                work_id,
                now + chrono::Duration::seconds(2),
            );
            event.summary = Some("staged aborted continuation".to_string());
            Ok(((), vec![event]))
        },
        || {
            Err(gwt_core::error::GwtError::Other(
                "simulated response loss after durable abort".to_string(),
            ))
        },
    );
    assert!(
        staged.is_err(),
        "the Work transaction must remain Prepared: {staged:?}"
    );
    let blocked_before_retry = gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            work_id,
            now + chrono::Duration::milliseconds(2500),
        ),
    );
    assert!(
        blocked_before_retry.is_err(),
        "the unresolved external transaction must block ordinary Work writers before reconciliation"
    );

    let runtime_root = temp.path().join(".gwt");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let mut predecessor =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    predecessor.id = predecessor_session_id.to_string();
    predecessor.project_state_root = Some(repo.clone());
    predecessor.repo_hash = detect_repo_hash(&repo).map(|value| value.to_string());
    predecessor.linked_issue_number = Some(owner.number);
    predecessor.agent_session_id = Some("provider-conversation".to_string());
    predecessor
        .save(&runtime.sessions_dir)
        .expect("save predecessor Session");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let candidate_path = runtime
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        let missing_target = runtime.sessions_dir.join("missing-aborted-candidate");
        symlink(&missing_target, &candidate_path)
            .expect("create dangling Aborted candidate Session");
        let authority_before =
            snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
        let work_before = tracked_workspace_work_store_snapshot(&repo);

        let retained = runtime.continue_work_events(
            &runtime.test_context(),
            "client-retry-dangling",
            operation_id.to_string(),
            work_id.to_string(),
            canvas_bounds(),
        );

        assert!(
            retained.iter().any(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                    retryable: true,
                    ..
                }
            )),
            "dangling Aborted candidate must remain an ambiguous present Session: {retained:?}",
        );
        assert!(fs::symlink_metadata(&candidate_path)
            .expect("dangling candidate entry must remain")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&candidate_path).unwrap(), missing_target);
        assert_optional_files_unchanged(&authority_before);
        assert_tracked_workspace_work_store_unchanged(&repo, &work_before);
        fs::remove_file(&candidate_path).expect("restore genuinely missing candidate state");
    }

    let candidate_path = runtime
        .sessions_dir
        .join(format!("{candidate_session_id}.toml"));
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let work_before = tracked_workspace_work_store_snapshot(&repo);
    let sessions_dir = runtime.sessions_dir.clone();
    let replacement_repo = repo.clone();
    set_missing_session_cleanup_hook_for_test(Box::new(move |observed_session_id| {
        assert_eq!(observed_session_id, candidate_session_id);
        let mut replacement = gwt_agent::Session::new(
            &replacement_repo,
            "work/replacement",
            gwt_agent::AgentId::Codex,
        );
        replacement.id = observed_session_id.to_string();
        replacement
            .save(&sessions_dir)
            .expect("materialize same-id Aborted candidate after Missing observation");
    }));

    let raced = runtime.continue_work_events(
        &runtime.test_context(),
        "client-retry-materialized",
        operation_id.to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );

    assert!(
        raced.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                retryable: true,
                ..
            }
        )),
        "same-id materialization must retain reconciliation evidence: {raced:?}",
    );
    assert!(candidate_path.exists(), "replacement candidate must remain");
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);
    fs::remove_file(&candidate_path).expect("restore true Missing state for successful cleanup");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-retry",
        operation_id.to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                retryable: false,
                ..
            } if code == "continuation_aborted"
        )),
        "aborted retry outcome: {events:?}"
    );

    let follow_up = gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Blocked,
            work_id,
            now + chrono::Duration::seconds(3),
        ),
    );
    assert!(
        follow_up.is_ok(),
        "Aborted reconciliation must reject the lingering Prepared Work transaction: {follow_up:?}"
    );
}

#[test]
fn continue_work_retry_repairs_activated_generation_before_work_publish() {
    continue_work_activated_successor_recovery_case(1, false, false, false, false, false, false);
}

#[test]
fn continue_work_activated_successor_rejects_non_initial_capability_without_mutation() {
    continue_work_activated_successor_recovery_case(2, false, false, false, false, false, false);
}

#[test]
fn continue_work_activated_successor_revalidates_candidate_after_repair_before_work_publish() {
    continue_work_activated_successor_recovery_case(1, true, false, false, false, false, false);
}

#[test]
fn continue_work_activated_successor_leases_candidate_through_work_publish() {
    continue_work_activated_successor_recovery_case(1, false, true, false, false, false, false);
}

#[test]
fn continue_work_activated_takeover_rejects_non_initial_capability_without_mutation() {
    continue_work_activated_successor_recovery_case(2, false, false, true, false, false, false);
}

#[test]
fn continue_work_activated_successor_rejects_candidate_agent_substitution_without_mutation() {
    continue_work_activated_successor_recovery_case(1, false, false, false, true, false, false);
}

#[test]
fn continue_work_activated_takeover_rejects_candidate_agent_substitution_without_mutation() {
    continue_work_activated_successor_recovery_case(1, false, false, true, true, false, false);
}

#[test]
fn continue_work_activated_successor_rejects_live_agent_substitution_without_mutation() {
    continue_work_activated_successor_recovery_case(1, false, false, false, false, true, false);
}

#[test]
fn continue_work_prepared_and_aborted_cleanup_reject_agent_substitution_without_mutation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());

    for (same_generation_takeover, already_aborted) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let case_name = format!(
            "{}-{}",
            if same_generation_takeover {
                "takeover"
            } else {
                "successor"
            },
            if already_aborted {
                "aborted"
            } else {
                "prepared"
            }
        );
        let repo = temp.path().join(format!("cleanup-agent-{case_name}"));
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        run_git(
            &repo,
            &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
        );
        let predecessor_session_id = format!("cleanup-predecessor-{case_name}");
        let candidate_session_id = format!("cleanup-candidate-{case_name}");
        let operation_id = format!("cleanup-operation-{case_name}");
        let work_id = format!("cleanup-work-{case_name}");
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            &predecessor_session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize cleanup predecessor");
        if !same_generation_takeover {
            gwt::cli::execution_state::settle(
                &repo,
                &predecessor_session_id,
                gwt::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle cleanup predecessor");
        }
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            if same_generation_takeover {
                gwt::cli::execution_state::LegacyActiveDisposition::Live
            } else {
                gwt::cli::execution_state::LegacyActiveDisposition::Unknown
            },
        )
        .expect("import cleanup predecessor");
        let now = chrono::Utc::now();
        let planned = if same_generation_takeover {
            let request = gwt::cli::execution_state::GenerationTakeoverRequest {
                operation_id: operation_id.clone(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: Some(work_id.clone()),
                source: Some("continue-work:resume".to_string()),
                from_session_id: predecessor_session_id.clone(),
                to_session_id: candidate_session_id.clone(),
                reason: "continue-work-stale-takeover: cleanup test".to_string(),
                requested_at: now,
            };
            gwt::cli::execution_state::prepare_generation_takeover(&repo, owner, &request)
                .expect("prepare cleanup takeover");
            let planned =
                gwt::cli::execution_state::prepared_generation_takeover_execution_binding(
                    &repo, owner, &request,
                )
                .expect("derive cleanup takeover binding");
            if already_aborted {
                gwt::cli::execution_state::abort_generation_takeover(
                    &repo,
                    owner,
                    &request,
                    "fixture abort",
                )
                .expect("abort cleanup takeover");
            }
            planned
        } else {
            let request = gwt::cli::execution_state::SuccessorRequest {
                operation_id: operation_id.clone(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: Some(work_id.clone()),
                source: "continue-work:resume".to_string(),
                session_binding_id: format!("cleanup-binding-{case_name}"),
                initial_session_id: candidate_session_id.clone(),
                entrypoint: "gwt-execute".to_string(),
                requested_at: now,
            };
            gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
                .expect("prepare cleanup successor");
            let planned = gwt::cli::execution_state::prepared_successor_execution_binding(
                &repo, owner, &request,
            )
            .expect("derive cleanup successor binding");
            if already_aborted {
                gwt::cli::execution_state::abort_successor(&repo, owner, &request, "fixture abort")
                    .expect("abort cleanup successor");
            }
            planned
        };

        let mut start = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &work_id,
            now,
        );
        start.title = Some("Cleanup authority Work".to_string());
        start.owner = Some("Issue #2359".to_string());
        start.agent_session_id = Some(predecessor_session_id.clone());
        start.agent_id = Some("Codex".to_string());
        start.display_name = Some("Codex".to_string());
        start.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/issue-2359".to_string()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
            .expect("record cleanup Work");
        if !same_generation_takeover {
            gwt_core::workspace_projection::record_workspace_work_event(
                &repo,
                gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Done,
                    &work_id,
                    now + chrono::Duration::seconds(1),
                ),
            )
            .expect("settle cleanup Work");
        }

        let runtime_root = temp.path().join(format!("cleanup-runtime-{case_name}"));
        let tab = sample_project_tab_with_window_at(
            "tab-cleanup",
            "shell-cleanup",
            repo.clone(),
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-cleanup"));
        let mut predecessor =
            gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
        predecessor.id = predecessor_session_id.clone();
        predecessor.project_state_root = Some(repo.clone());
        predecessor.repo_hash = detect_repo_hash(&repo).map(|value| value.to_string());
        predecessor.linked_issue_number = Some(owner.number);
        predecessor
            .save(&runtime.sessions_dir)
            .expect("save cleanup predecessor Session");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: candidate_session_id.clone(),
            repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: planned,
            capability_generation: 1,
        };
        let mut replacement = gwt_agent::Session::new(
            &repo,
            "work/issue-2359",
            gwt_agent::AgentId::Custom("review-bot".to_string()),
        );
        replacement.id = candidate_session_id.clone();
        replacement.project_state_root = Some(repo.clone());
        replacement.repo_hash = Some(binding.repo_hash.clone());
        replacement.linked_issue_number = Some(owner.number);
        replacement.status = gwt_agent::AgentStatus::Stopped;
        replacement
            .set_execution_binding(Some(binding))
            .expect("bind cleanup replacement Session");
        replacement
            .save(&runtime.sessions_dir)
            .expect("save cleanup replacement Session");

        let candidate_path = runtime
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        let candidate_before = fs::read(&candidate_path).expect("read replacement before retry");
        let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
        let work_items_before = fs::read(&work_items_path).expect("read Work projection before");
        let work_events_before = tracked_work_event_store_snapshot(&repo);
        let authority_before =
            snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
        let workspace_before = serde_json::to_vec(
            runtime
                .tab("tab-cleanup")
                .expect("cleanup tab")
                .workspace
                .persisted(),
        )
        .expect("serialize cleanup Workspace before");

        let events = runtime.continue_work_events(
            &runtime.test_context(),
            "client-cleanup",
            operation_id,
            work_id,
            canvas_bounds(),
        );

        assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                    retryable: true,
                    ..
                }
            )),
            "{case_name} replacement must conflict: {events:#?}"
        );
        assert_eq!(
            fs::read(&candidate_path).expect("read retained cleanup replacement"),
            candidate_before,
            "{case_name} replacement must be retained byte-identically",
        );
        assert_eq!(
            fs::read(&work_items_path).expect("read Work projection after"),
            work_items_before,
            "{case_name} must not mutate Work projection",
        );
        assert_eq!(
            tracked_work_event_store_snapshot(&repo),
            work_events_before,
            "{case_name} must not mutate Work events",
        );
        assert_optional_files_unchanged(&authority_before);
        assert_eq!(
            serde_json::to_vec(
                runtime
                    .tab("tab-cleanup")
                    .expect("cleanup tab after")
                    .workspace
                    .persisted(),
            )
            .expect("serialize cleanup Workspace after"),
            workspace_before,
            "{case_name} must not materialize a pane",
        );
        assert!(runtime.pending_continue_work.is_empty(), "{case_name}");
    }
}

#[test]
fn continue_work_activated_candidate_only_fallback_rejects_agent_substitution_without_mutation() {
    continue_work_activated_successor_recovery_case(1, false, false, false, true, false, true);
}

#[test]
fn continue_work_authenticated_session_start_commits_successor_and_work_exactly_once() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(
        &repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );

    let predecessor_session_id = "predecessor-session";
    let candidate_session_id = "candidate-session";
    let selected_work_id = "work-selected";
    let operation_id = "continue-op-commit";
    let readiness_nonce = "continue-ready-commit";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor execution");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            predecessor_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    let ledger = gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import completed predecessor");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Completed)
    );
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");

    let now = chrono::Utc::now();
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(selected_work_id.to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "binding-candidate".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: now,
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare successor");
    let planned_identity =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive Prepared binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned_identity.clone(),
        capability_generation: 1,
    };

    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        selected_work_id,
        now,
    );
    start.title = Some("Continue selected Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
        .expect("record selected Work");
    gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            selected_work_id,
            now + chrono::Duration::seconds(1),
        ),
    )
    .expect("complete selected Work");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let runtime_root = temp.path().join(".gwt");
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = candidate_session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);

    let mut candidate_session =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate_session.id = candidate_session_id.to_string();
    candidate_session.project_state_root = Some(repo.clone());
    candidate_session.linked_issue_number = Some(owner.number);
    candidate_session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate Session");
    candidate_session
        .save(&runtime.sessions_dir)
        .expect("save candidate Session");

    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue_prepared(&repo, candidate_session_id, binding.clone())
        .expect("issue Prepared capability");
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), capability.token.clone());
    runtime.pending_continue_work.insert(
        window_id.clone(),
        PendingContinueWork {
            client_id: "client-1".to_string(),
            operation_id: operation_id.to_string(),
            work_id: selected_work_id.to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: Some(predecessor_session_id.to_string()),
            execution: PendingContinueWorkExecution::Successor(request),
            binding: binding.clone(),
            readiness_nonce: readiness_nonce.to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: Some("Continue selected Work".to_string()),
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: predecessor_session_id.to_string(),
            predecessor_binding: predecessor_binding.clone(),
        },
    );
    let committed_pending = runtime
        .pending_continue_work
        .get(&window_id)
        .expect("pending continuation")
        .clone();
    let _retry_focus = runtime.continue_work_events(
        &runtime.test_context(),
        "client-reconnected",
        operation_id.to_string(),
        selected_work_id.to_string(),
        canvas_bounds(),
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_waiters
            .get(operation_id)
            .is_some_and(|waiters| waiters.contains("client-reconnected")),
        "a reconnect retry must subscribe to the exact pending operation"
    );
    assert!(
        gwt::cli::execution_state::prepared_execution_binding_matches(
            &repo,
            owner,
            candidate_session_id,
            &binding.identity,
        )
        .expect("read Prepared authority"),
        "the candidate binding must match the durable Prepared attempt"
    );
    let prepared_probe = gwt::probe_authenticated_prepared_execution_binding(
        &repo,
        candidate_session_id,
        &binding,
        "continue-work-test-host",
        gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: operation_id.to_string(),
            nonce: "prepared-probe-nonce".to_string(),
        },
    );
    assert!(
        prepared_probe.is_ok(),
        "the Prepared Host probe must succeed before activation: {prepared_probe:?}"
    );

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_outcomes
            .is_empty(),
        "PTY spawn alone must not produce success"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read Prepared ledger")
            .expect("Prepared ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Completed),
        "Prepared launch must leave the predecessor current"
    );

    let events = runtime.finalize_continue_work_session_start(&window_id, Some(readiness_nonce));

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                operation_id: emitted_operation_id,
                work_id,
                outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                ..
            } if emitted_operation_id == operation_id && work_id == selected_work_id
        )),
        "authenticated readiness must emit one correlated success; events={events:?}, attempt={:?}",
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id)
            .expect("read continuation attempt")
    );
    assert!(events.iter().any(|event| {
        matches!(
            (&event.target, &event.event),
            (
                DispatchTarget::Client(client_id),
                BackendEvent::ContinueWorkOutcome {
                    operation_id: emitted_operation_id,
                    outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                    ..
                }
            ) if client_id == "client-reconnected" && emitted_operation_id == operation_id
        )
    }));
    assert!(!runtime.pending_continue_work.contains_key(&window_id));
    assert!(issuer.active_token_is_current(&capability.token, &binding));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read committed binding"),
        Some(planned_identity)
    );
    assert!(
        !gwt::cli::execution_state::current_active_execution_binding_matches(
            &repo,
            owner,
            predecessor_session_id,
            &predecessor_binding,
        )
        .expect("verify predecessor fence"),
        "the predecessor binding must be rejected after activation"
    );
    let projection = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work projection")
        .expect("Work projection");
    let selected = projection
        .work_items
        .iter()
        .find(|item| item.id == selected_work_id)
        .expect("selected Work");
    assert_eq!(
        selected.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Active
    );
    assert!(selected
        .agents
        .iter()
        .any(|agent| agent.session_id == candidate_session_id));
    assert!(
        runtime
            .finalize_continue_work_session_start(&window_id, Some(readiness_nonce))
            .is_empty(),
        "the consumed readiness receipt must not emit a second success"
    );
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_outcomes
        .remove(operation_id);
    runtime
        .pending_continue_work
        .insert(window_id.clone(), committed_pending.clone());
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .insert(
            operation_id.to_string(),
            HashSet::from(["client-after-response-loss".to_string()]),
        );
    let late_error_events = runtime.launch_error_events_with_continue_work(
        window_id.clone(),
        "late PTY error after committed response loss".to_string(),
        None,
    );
    assert!(
        late_error_events.iter().any(|event| matches!(
            (&event.target, &event.event),
            (
                DispatchTarget::Client(client_id),
                BackendEvent::ContinueWorkOutcome {
                    operation_id: emitted_operation_id,
                    outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                    retryable: false,
                    ..
                }
            ) if client_id == "client-after-response-loss" && emitted_operation_id == operation_id
        )),
        "an Activated late error must reconcile and fan out durable success: {late_error_events:?}"
    );
    assert!(
        !runtime.pending_continue_work.contains_key(&window_id),
        "post-commit launch errors must discard only the stale process-local receipt",
    );
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_waiters
            .contains_key(operation_id),
        "the reconciled outcome must consume reconnect waiters exactly once",
    );
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert!(
        !matches!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Error | WindowProcessStatus::Stopped)
        ),
        "durably committed response-loss recovery must not tear down the exact live pane"
    );
    let same_host_retry = runtime.continue_work_events(
        &runtime.test_context(),
        "client-response-loss",
        operation_id.to_string(),
        selected_work_id.to_string(),
        canvas_bounds(),
    );
    assert!(same_host_retry.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            retryable: false,
            ..
        }
    )));
    assert!(!runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .contains_key(operation_id));
    assert!(
        !runtime.pending_continue_work.contains_key(&window_id),
        "durable response-loss reconciliation must consume the stale pending receipt"
    );

    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_outcomes
        .remove(operation_id);
    runtime
        .pending_continue_work
        .insert(window_id.clone(), committed_pending);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .insert(
            operation_id.to_string(),
            HashSet::from(["client-error-pane".to_string()]),
        );
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Error);
    assert!(
        runtime
            .continue_work_launch_failed_events(&window_id, "already-dead pane")
            .is_empty(),
        "a non-live pane must never be reconstructed as strong continuation success"
    );
    assert!(runtime.pending_continue_work.contains_key(&window_id));
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .contains_key(operation_id));

    let retry_tab = sample_project_tab_with_window_at(
        "tab-retry",
        "shell-retry",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut restarted_runtime = sample_runtime(&runtime_root, vec![retry_tab], Some("tab-retry"));
    let retry_events = restarted_runtime.continue_work_events(
        &restarted_runtime.test_context(),
        "client-retry",
        operation_id.to_string(),
        selected_work_id.to_string(),
        canvas_bounds(),
    );
    assert!(
        retry_events.iter().all(|event| !matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                ..
            }
        )),
        "ledger + Work state without a live exact pane must not replay strong success: {retry_events:?}"
    );
    assert!(retry_events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            operation_id: emitted_operation_id,
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            retryable: true,
            ..
        } if emitted_operation_id == operation_id
            && work_id == selected_work_id
            && code == "continuation_reconciliation_required"
    )));
    let readback = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("load response-loss retry ledger")
        .expect("response-loss retry ledger");
    assert_eq!(readback.generations.len(), 2);
    assert_eq!(
        readback
            .continuation_attempts
            .iter()
            .filter(|attempt| {
                attempt.status == gwt::cli::execution_state::ContinuationAttemptStatus::Activated
            })
            .count(),
        1,
        "response-loss retry must not append another generation or activation"
    );
}

// #3426 (A): the Work projection says `Issue #2359` while the trusted ledger
// says spec/2359. The heal must survive the activation transaction and rewrite
// the Work owner. Before the fix the pre-transition precondition re-applied the
// exact-kind check that canonical_continue_work_owner had deliberately relaxed,
// so the continuation spawned a PTY and then died at authenticated SessionStart.
#[test]
fn continue_work_authenticated_session_start_commits_healed_spec_owner() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());

    let (_runtime, repo, events, work_id) =
        continue_work_heal_session_start_events(temp.path(), "heal-commit", "Issue #2359");

    assert!(
        events.iter().all(|event| !matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                ..
            }
        )),
        "a healed owner kind must not fail at activation: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                ..
            }
        )),
        "authenticated readiness must emit one correlated success: {events:?}"
    );
    let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work projection")
        .expect("Work projection")
        .work_items
        .into_iter()
        .find(|item| item.id == work_id)
        .expect("healed Work");
    assert_eq!(
        work.owner.as_deref(),
        Some("SPEC-2359"),
        "the activation transaction must persist the corrected canonical owner"
    );
}

// The relaxation is kind-only: a genuine owner NUMBER disagreement must stay
// fail-closed at the same precondition.
#[test]
fn continue_work_session_start_still_refuses_owner_number_mismatch() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());

    let (_runtime, repo, events, work_id) =
        continue_work_heal_session_start_events(temp.path(), "heal-number", "Issue #2360");

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                ..
            }
        )),
        "an owner number mismatch must still be rejected: {events:?}"
    );
    let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work projection")
        .expect("Work projection")
        .work_items
        .into_iter()
        .find(|item| item.id == work_id)
        .expect("unhealed Work");
    assert_eq!(
        work.owner.as_deref(),
        Some("Issue #2360"),
        "a refused activation must not rewrite the Work owner"
    );
}

#[test]
fn fresh_execution_authenticated_session_start_activates_new_lifetime_and_preserves_blocked_history(
) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let predecessor_session_id = "legacy-blocked-session";
    let candidate_session_id = "fresh-candidate-session";
    let operation_id = "fresh-launch-ready-operation";
    let readiness_nonce = "fresh-launch-private-readiness";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize legacy execution");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            predecessor_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "legacy terminal blocker".to_string(),
                missing_verification: Some("legacy evidence gap".to_string()),
            },
        )
        .expect("settle Blocked predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import Blocked predecessor");
    let predecessor_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("load predecessor ledger")
        .expect("predecessor ledger");
    let predecessor_generation = predecessor_ledger.generations[0].clone();
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");

    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "fresh-candidate-binding".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "$gwt-execute #2359".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(&repo, owner, &request)
        .expect("prepare fresh successor");
    let planned =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive fresh binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned.clone(),
        capability_generation: 1,
    };

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let runtime_root = temp.path().join(".gwt");
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut predecessor_session =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    predecessor_session.id = predecessor_session_id.to_string();
    predecessor_session.project_state_root = Some(repo.clone());
    predecessor_session.linked_issue_number = Some(owner.number);
    predecessor_session
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: predecessor_session_id.to_string(),
            repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: predecessor_binding.clone(),
            capability_generation: 1,
        }))
        .expect("bind predecessor Session");
    predecessor_session
        .save(&runtime.sessions_dir)
        .expect("save predecessor Session");
    let predecessor_session_path = runtime
        .sessions_dir
        .join(format!("{predecessor_session_id}.toml"));
    let predecessor_session_bytes = fs::read(&predecessor_session_path).expect("old Session bytes");

    let mut predecessor_active = sample_active_agent_session("tab-1", &window_id);
    predecessor_active.session_id = predecessor_session_id.to_string();
    predecessor_active.branch_name = "work/issue-2359".to_string();
    predecessor_active.worktree_path = repo.clone();
    save_workspace_launch_projection(
        &repo,
        &predecessor_active,
        Some("origin/develop"),
        Some(owner.number),
        Some(owner),
        None,
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([predecessor_session_id.to_string()])),
    )
    .expect("publish predecessor Work");
    let predecessor_work =
        gwt_core::workspace_projection::transact_workspace_state_for_work_event_root(
            &repo,
            &repo,
            |projection, _, _| {
                let mut other_host = gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Update,
                    projection.id.clone(),
                    Utc::now(),
                );
                other_host.execution_container = Some(
                    gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                        branch: Some(predecessor_active.branch_name.clone()),
                        worktree_path: Some(PathBuf::from("E:\\gwt\\work\\issue-2359")),
                        pr_number: None,
                        pr_url: None,
                        pr_state: None,
                    },
                );
                let event = gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Discard,
                    projection.id.clone(),
                    Utc::now(),
                );
                Ok((projection.id.clone(), vec![other_host, event]))
            },
        )
        .expect("discard predecessor Work");
    let predecessor_work_snapshot =
        gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .unwrap()
            .unwrap()
            .work_items
            .into_iter()
            .find(|work| work.id == predecessor_work)
            .expect("discarded predecessor Work");
    assert_eq!(predecessor_work_snapshot.execution_containers.len(), 2);

    let mut candidate =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate.id = candidate_session_id.to_string();
    candidate.project_state_root = Some(repo.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(binding.clone()))
        .expect("bind fresh Session");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save fresh Session");
    let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&candidate)
        .unwrap()
        .unwrap();
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: operation_id.to_string(),
        },
        candidate_session_id,
        &repo,
        &repo,
        owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist fresh launch receipt");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = candidate_session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45131/internal/hook-live",
        "ws://127.0.0.1:46241/ws",
        "ws://127.0.0.1:45131/internal/pane-ws",
    );
    let capability = issuer
        .issue_prepared(&repo, candidate_session_id, binding.clone())
        .expect("issue Prepared capability");
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), capability.token.clone());
    runtime.pending_fresh_execution_launches.insert(
        window_id.clone(),
        PendingFreshExecutionLaunch {
            operation_id: operation_id.to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            request,
            binding: binding.clone(),
            session_identity,
            readiness_nonce: readiness_nonce.to_string(),
            predecessor_binding: predecessor_binding.clone(),
            base_branch: Some("origin/develop".to_string()),
            linked_issue_number: Some(owner.number),
            resume_context: Some(WorkspaceResumeContext {
                title: Some("Fresh legacy recovery".to_string()),
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            }),
            launch_feedback_context: None,
        },
    );

    let mut session_start =
        runtime_hook_state_for_event("Working", "SessionStart", candidate_session_id);
    session_start.continuation_readiness_nonce = Some(readiness_nonce.to_string());
    session_start.project_root = Some(repo.display().to_string());
    session_start.branch = Some("work/issue-2359".to_string());
    let (spawner, _) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut events = runtime.handle_runtime_hook_event(session_start);
    events.extend(commit_pending_fresh_execution(&mut runtime));

    assert!(
        !events.is_empty(),
        "activation must publish the committed state"
    );
    assert!(!runtime
        .pending_fresh_execution_launches
        .contains_key(&window_id));
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        candidate_session_id,
    ));
    assert!(
        issuer.active_token_is_current(&capability.token, &binding),
        "fresh capability was not promoted; events={events:?}; attempt={:?}; current={:?}",
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id,),
        gwt::cli::execution_state::current_execution_binding(&repo, owner),
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read fresh current binding"),
        Some(planned),
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read activated ledger")
        .expect("activated ledger");
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(ledger.generations[0], predecessor_generation);
    assert_ne!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner).unwrap(),
        Some(predecessor_binding),
        "old binding must be stale after fresh activation",
    );
    assert_eq!(
        fs::read(predecessor_session_path).expect("old Session readback"),
        predecessor_session_bytes,
        "fresh activation must not rewrite the predecessor Session",
    );
    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .unwrap()
        .unwrap();
    assert_eq!(
        works
            .work_items
            .iter()
            .find(|work| work.id == predecessor_work),
        Some(&predecessor_work_snapshot),
        "fresh activation must preserve the discarded Work and its Session membership",
    );
    let successor = works
        .work_items
        .iter()
        .find(|work| {
            work.agents
                .iter()
                .any(|agent| agent.session_id == candidate_session_id)
        })
        .expect("fresh Session belongs to a successor Work");
    assert_ne!(successor.id, predecessor_work);
    assert!(!successor.discarded);
    assert!(successor.related_work_item_ids.contains(&predecessor_work));
    save_workspace_launch_projection(
        &repo,
        runtime.active_agent_sessions.get(&window_id).unwrap(),
        None,
        Some(owner.number),
        Some(owner),
        None,
        WorkspaceLaunchProjectionKind::Resume {
            created_by_start_work: true,
        },
        Some(&HashSet::from([candidate_session_id.to_string()])),
    )
    .expect("retry must reuse the same successor Work");
    assert!(
        save_workspace_launch_projection(
            &repo,
            &predecessor_active,
            None,
            Some(owner.number),
            Some(owner),
            None,
            WorkspaceLaunchProjectionKind::Resume {
                created_by_start_work: true
            },
            Some(&HashSet::from([predecessor_session_id.to_string()])),
        )
        .is_err(),
        "a discarded predecessor Session cannot be moved to its successor"
    );
    let retry_works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .unwrap()
        .unwrap();
    assert_eq!(retry_works.work_items.len(), 2);
    assert_eq!(
        retry_works
            .work_items
            .iter()
            .find(|work| work.id == predecessor_work),
        Some(&predecessor_work_snapshot)
    );
}
