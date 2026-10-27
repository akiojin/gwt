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

    use gwt::project_runtime::continuation::ContinueWorkAction;
    let conflict =
        continue_work_domain_action(&runtime, "client-b", "immutable-operation", "work-b");
    assert!(
        matches!(conflict, ContinueWorkAction::Failure { ref failure, .. }
        if failure.outcome == gwt::ContinueWorkOutcomeKind::Failed && failure.code == "operation_conflict"),
        "a conflicting Work cannot replace the cached operation: {conflict:?}"
    );
    let replay = continue_work_domain_action(&runtime, "client-a", "immutable-operation", "work-a");
    assert!(
        matches!(replay, ContinueWorkAction::Replay { ref outcome, .. }
        if outcome.work_id == "work-a" && outcome.outcome == gwt::ContinueWorkOutcomeKind::ContinuedConversation
            && outcome.error_code.is_none()),
        "the original Work must retain its replay: {replay:?}"
    );
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

    let actual = owner_continue_launch_fixture(&runtime, "projection-only-operation", &work_id);
    let pending = &actual.pending;
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
    let inventory = actual
        .executor
        .as_ref()
        .unwrap()
        .block_on(
            actual
                .owner
                .as_ref()
                .unwrap()
                .command(gwt::project_runtime::events::OwnerCommand::ListPanes),
        )
        .unwrap();
    assert_eq!(
        inventory["pending"].as_array().unwrap().len(),
        1,
        "the actual owner must materialize one candidate"
    );
    assert_eq!(
        actual.captured.session.as_ref().unwrap().agent_id.command(),
        "codex"
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read projection-only ledger")
        .expect("projection-only ledger");
    assert_eq!(ledger.continuation_attempts.len(), 1);
    actual.shutdown();
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
    let (runtime, _repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual = owner_continue_launch_fixture(&runtime, "projection-spec-operation", &work_id);
    let pending = &actual.pending;
    assert_eq!(pending.owner, owner);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    actual.shutdown();
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
    let (runtime, _repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual =
        owner_continue_launch_fixture(&runtime, "projection-spec-legacy-flat-operation", &work_id);
    let pending = &actual.pending;
    assert_eq!(pending.owner, owner);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    actual.shutdown();
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual =
        owner_continue_launch_fixture(&runtime, "projection-canonical-branch-operation", &work_id);
    assert_eq!(actual.pending.work_branch, "work/issue-2359");
    let materialized = actual
        .captured
        .session
        .as_ref()
        .expect("actual chosen provider metadata");
    assert_eq!(materialized.agent_id.command(), "codex");
    assert_eq!(materialized.display_name, "Durable branch alias settings");
    actual.shutdown();
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

    let canonical_profile = gwt_core::paths::gwt_config_path();
    fs::create_dir_all(canonical_profile.parent().unwrap()).unwrap();
    fs::copy(
        runtime.profile_config_path.as_ref().unwrap(),
        canonical_profile,
    )
    .unwrap();
    let executor = tokio::runtime::Runtime::new().unwrap();
    let physical_owner = executor.block_on(production_owner_fixture(&repo));
    let checkpoint = executor
        .block_on(physical_owner.pause_next_agent_launch_at(
            gwt::project_runtime::test_support::OwnerLaunchStage::MaterializedBeforeInstall,
        ))
        .unwrap();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let events = runtime.resume_workspace_agent_events(
        &runtime.test_context(),
        "resume-client",
        "linked-resume-operation".into(),
        durable.id.clone(),
        None,
    );
    assert!(events.iter().any(|event| matches!(
        &event.event, BackendEvent::WorkspaceResumeAgentStarted { operation_id, session_id, .. }
            if operation_id == "linked-resume-operation" && session_id == "historical-session"
    )));
    let task = tasks
        .lock()
        .unwrap()
        .pop()
        .expect("the GUI must enqueue its immutable owner request");
    let worker_home = temp.path().to_path_buf();
    let worker = thread::spawn(move || {
        let _home = ScopedGwtHome::set(worker_home);
        task();
    });
    let captured = executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual GUI-to-owner Resume materialization checkpoint
        tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap()
    });
    let pending = captured.pending_continue_work.clone();
    executor.block_on(async {
        let client = physical_owner.client();
        let id = captured.id.clone();
        let operation_id = captured.operation_id.clone();
        let cancelled = tokio::spawn(async move {
            client
                .command(gwt::project_runtime::events::OwnerCommand::PendingCancel {
                    id,
                    operation_id,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for exact cancellation admission before releasing the real Resume worker
        tokio::time::timeout(Duration::from_secs(30), async {
            while captured.current.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        checkpoint.release();
        cancelled
            .await
            .unwrap()
            .expect("actual Resume cancellation cleanup ACK");
    });
    worker
        .join()
        .expect("the actual GUI owner request worker must finish");
    executor.block_on(physical_owner.shutdown());
    let pending =
        pending.expect("linked Resume must prepare continuation authority at the actual owner");
    assert_eq!(pending.work_id, work_id);
    assert!(
        matches!(&pending.execution, PendingContinueWorkExecution::Successor(request)
        if request.initial_session_id == pending.binding.session_id)
    );
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual = owner_continue_launch_fixture(&runtime, "custom-durable-operation", &work_id);
    let pending = &actual.pending;
    assert_eq!(
        pending.work_agent_session_id.as_deref(),
        Some("historical-session")
    );
    assert_eq!(
        pending.work_agent_id,
        gwt_agent::AgentId::Custom("review-bot".to_string())
    );
    assert_eq!(
        actual.captured.session.as_ref().unwrap().display_name,
        "Exact custom durable settings"
    );
    actual.shutdown();
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual = owner_continue_launch_fixture(&runtime, "ref-agent-mismatch-operation", &work_id);
    let pending = &actual.pending;
    assert_eq!(pending.work_agent_session_id, None);
    assert_eq!(
        pending.outcome,
        gwt::ContinueWorkOutcomeKind::StartedWithHandoff
    );
    actual.shutdown();
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
        let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

        let failure = continue_work_domain_failure(
            &runtime,
            "client-missing-work-agent",
            &(format!("operation-{case_name}")),
            &(work_id),
        );

        assert_eq!(failure.outcome, gwt::ContinueWorkOutcomeKind::Failed);
        assert!(!failure.retryable);
        assert_eq!(failure.code, "agent_identity_missing");
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let failure = continue_work_domain_failure(
        &runtime,
        "client-branch-divergence",
        &("operation-branch-divergence".to_string()),
        &(work_id),
    );

    assert_eq!(
        failure.outcome,
        gwt::ContinueWorkOutcomeKind::ConflictUnknown
    );
    assert!(failure.retryable);
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let failure = continue_work_domain_failure(
        &runtime,
        "client-shell-ref",
        &("shell-ref-operation".to_string()),
        &(work_id),
    );

    assert!(runtime.pending_continue_work.is_empty());
    assert_eq!(failure.outcome, gwt::ContinueWorkOutcomeKind::Failed);
    assert!(!failure.retryable);
    assert_eq!(failure.code, "agent_identity_missing");
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual = owner_continue_launch_fixture(
        &runtime,
        "projection-noncanonical-session-operation",
        &work_id,
    );
    let materialized = actual
        .captured
        .session
        .as_ref()
        .expect("projection fallback candidate");
    assert_ne!(
        materialized.display_name,
        "Injected outside Session settings"
    );
    assert_eq!(materialized.agent_id.command(), "codex");
    actual.shutdown();
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
    let runtime = sample_runtime(&temp.path().join("runtime"), vec![tab], Some("tab-foreign"));
    let mut forged =
        gwt_agent::Session::new(&foreign, "work/issue-2359", gwt_agent::AgentId::Codex);
    forged.id = predecessor_session_id.to_string();
    forged.project_state_root = Some(project_root);
    forged.linked_issue_number = Some(owner.number);
    forged
        .save(&runtime.sessions_dir)
        .expect("save foreign Session with matching project root");

    let failure = continue_work_domain_failure(
        &runtime,
        "client-foreign-recovery",
        &(operation_id.to_string()),
        &(work_id.to_string()),
    );

    assert_eq!(failure.outcome, gwt::ContinueWorkOutcomeKind::Failed);
    assert!(!failure.retryable);
    assert_eq!(failure.code, "agent_identity_missing");
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual = owner_continue_launch_fixture(
        &runtime,
        "projection-conflicting-session-operation",
        &work_id,
    );
    let pending = &actual.pending;
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
    actual.shutdown();
}

#[test]
fn projection_only_continue_authenticates_container_before_using_durable_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let failure = continue_work_domain_failure(
        &runtime,
        "client-projection-container-first",
        &("projection-container-first-operation".to_string()),
        &(work_id),
    );

    assert_eq!(failure.outcome, gwt::ContinueWorkOutcomeKind::Failed);
    assert!(!failure.retryable);
    assert_eq!(failure.code, "execution_container_ambiguous");
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
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, authority, work_id) =
        owner_continuation_seed(temp.path(), "projection-only-spawn-failure");
    let work_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    let before_work = fs::read(&work_path).unwrap();
    let before_events = tracked_work_event_store_snapshot(&repo);
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, authority)
        .unwrap()
        .unwrap();
    let executor = TokioRuntime::new().unwrap();
    executor.block_on(async {
        use gwt::project_runtime::{
            events::{OwnerCommand, OwnerEvent},
            test_support::OwnerLaunchStage,
        };
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let mut outcomes = owner.subscribe();
        let client = owner.client();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::ContinueWork {
                    client_id: "client-projection-spawn-failure".into(),
                    operation_id: "projection-spawn-failure-operation".into(),
                    work_id,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual reconstructed candidate before PTY installation
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        let candidate = captured
            .session
            .as_ref()
            .expect("actual materialized candidate");
        let runner = temp
            .path()
            .join("fixture-runner-bin")
            .join(if cfg!(windows) { "codex.cmd" } else { "codex" });
        fs::remove_file(runner)
            .expect("remove the already-probed real fixture provider before physical spawn");
        checkpoint.release();
        assert!(launch.await.unwrap().is_err());
        // test-hygiene: allow-wall-clock-deadline hang guard for correlated actual ContinueWork physical launch failure
        let failure = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = outcomes.recv().await.unwrap();
                if matches!(&event, OwnerEvent::ContinueWorkOutcome { operation_id, .. }
                    if operation_id == "projection-spawn-failure-operation")
                {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(failure, OwnerEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::Failed, error_code: Some(ref code), ..
        } if code == "launch_failed"),
            "{failure:?}"
        );
        let inventory = owner.command(OwnerCommand::ListPanes).await.unwrap();
        assert!(inventory["panes"].as_array().unwrap().is_empty());
        assert!(inventory["pending"].as_array().unwrap().is_empty());
        assert!(!gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", candidate.id))
            .exists());
        assert_eq!(fs::read(&work_path).unwrap(), before_work);
        assert_eq!(tracked_work_event_store_snapshot(&repo), before_events);
        let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, authority)
            .unwrap()
            .unwrap();
        assert_eq!(ledger.generations, before_ledger.generations);
        assert_eq!(
            ledger.current_generation_id,
            before_ledger.current_generation_id
        );
        assert!(ledger
            .continuation_attempts
            .iter()
            .any(
                |attempt| attempt.request.operation_id == "projection-spawn-failure-operation"
                    && attempt.status
                        == gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
            ));
        owner.shutdown().await;
    });
}

#[test]
fn projection_only_continue_owner_change_before_activation_aborts_without_committing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, owner_key, work_id) = owner_continuation_seed(
        temp.path(),
        "projection_only_continue_owner_change_before_activation_aborts_without_committing",
    );
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
        .expect("predecessor ledger")
        .expect("ledger");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("actual prepared candidate");
        let transport = owner.client();
        let selected = work_id.clone();
        let launch = tokio::spawn(async move {
            transport
                .command(OwnerCommand::ContinueWork {
                    client_id: "client-1".into(),
                    operation_id: "readiness-rejection".into(),
                    work_id: selected,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual preparation reaches the checkpoint");
        let pending = captured
            .pending_continue_work
            .as_ref()
            .expect("actual owner Prepared receipt");
        let candidate = captured.session.as_ref().expect("actual candidate Session");
        assert_eq!(pending.binding.session_id, candidate.id);
        assert!(captured
            .issuer
            .grant_is_current(captured.grant.as_ref().expect("actual Prepared Host grant")));
        let mut owner_change = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            &work_id,
            chrono::Utc::now() + chrono::Duration::seconds(1),
        );
        owner_change.owner = Some("SPEC #3248".into());
        gwt_core::workspace_projection::record_workspace_work_event(&repo, owner_change)
            .expect("change selected Work after actual preparation");

        let work_before_finalize = tracked_work_event_store_snapshot(&repo);
        checkpoint.release();
        launch
            .await
            .expect("launch worker")
            .expect("physical launch ACK");
        let mut projected = owner.subscribe();
        let mut readiness = runtime_hook_state_for_event("Waiting", "SessionStart", &candidate.id);
        readiness.project_root = Some(repo.display().to_string());
        readiness.branch = Some(candidate.branch.clone());
        readiness.continuation_readiness_nonce = Some(pending.readiness_nonce.clone());
        owner.transport_event(gwt::project_transport::TransportEvent::RuntimeHook(
            readiness,
        ));
        // test-hygiene: allow-wall-clock-deadline hang guard for the exact owner outcome; accepted messages establish ordering
        let failure = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let event @ OwnerEvent::ContinueWorkOutcome { .. } =
                    projected.recv().await.expect("owner outcome")
                {
                    break event;
                }
            }
        })
        .await
        .expect("readiness rejection outcome");
        assert!(
            matches!(failure, OwnerEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::Failed, error_code: Some(ref code), ..
        } if code == "launch_failed"),
            "exact owner must reject invalid readiness: {failure:?}"
        );
        assert!(
            !gwt_core::paths::gwt_sessions_dir()
                .join(format!("{}.toml", candidate.id))
                .exists(),
            "rejection must discard only the actual exact candidate Session"
        );
        assert_eq!(
            tracked_work_event_store_snapshot(&repo),
            work_before_finalize,
            "rejected readiness must not rewrite selected Work"
        );
        let after_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
            .expect("rejected ledger")
            .expect("ledger");
        assert_eq!(after_ledger.generations, before_ledger.generations);
        assert_eq!(
            after_ledger.current_generation_id,
            before_ledger.current_generation_id
        );
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &repo,
                owner_key,
                &captured.operation_id
            )
            .expect("rejected attempt")
            .expect("actual attempt")
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
        );
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation: captured.incarnation,
            })
            .await
            .expect("physical cleanup ACK");
        assert!(captured
            .pty
            .as_ref()
            .expect("actual child")
            .try_wait()
            .expect("child wait")
            .is_some());
        owner.shutdown().await;
    });
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
    let (runtime, repo, owner, work_id) = projection_only_continue_runtime(
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

    let actual = owner_continue_launch_fixture(&runtime, "owner-kind-heal-operation", &work_id);
    let pending = &actual.pending;
    assert_eq!(pending.work_id, work_id);
    assert_eq!(
        pending.owner, owner,
        "the continuation must run under the trusted spec authority"
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read healed ledger")
        .expect("healed ledger");
    assert_eq!(ledger.continuation_attempts.len(), 1);
    actual.shutdown();
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
    assert_actual_legacy_inspection_repair(false);
    assert_actual_legacy_inspection_repair(true);
}

fn assert_actual_legacy_inspection_repair(close_during_publication: bool) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    run_git(&repo, &["branch", "-M", "work/issue-2359"]);
    let runner_dir = write_fixture_runners(temp.path(), &["codex"]);
    let runner = runner_dir.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
    #[cfg(unix)]
    fs::write(&runner, "#!/bin/sh\ncase \"$*\" in *--version*) printf '1.2.3\\n'; exit 0;; *'features list'*) exit 0;; esac\nexec /bin/cat\n").unwrap();
    #[cfg(windows)]
    fs::write(&runner, "@echo off\r\nif \"%~1\"==\"--version\" (echo 1.2.3 & exit /b 0)\r\nping -n 31 127.0.0.1 >NUL\r\n").unwrap();
    let owner_key = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    owner_fixture_block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        let owner = production_owner_fixture(&repo).await;
        let mut command =
            owner_fixture_agent_command(&owner, &repo, "legacy-live", "legacy-inspection-launch");
        let OwnerCommand::LaunchAgent {
            config, metadata, ..
        } = &mut command
        else {
            unreachable!()
        };
        **config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(&repo)
            .build();
        config.command = runner.display().to_string();
        config.args.clear();
        config.suppress_execution_control = true;
        config.linked_issue_number = Some(owner_key.number);
        metadata.workspace_resume_context = None;
        let receipt = owner
            .command(command)
            .await
            .expect("actual observation-only physical launch");
        let id = receipt["id"].as_str().unwrap().to_string();
        let incarnation = receipt["incarnation"].as_u64().unwrap();
        let physical = owner.probe(&id, incarnation).await.unwrap();
        assert!(physical.pty.try_wait().unwrap().is_none());
        let inspection = physical
            .current_capability_grant()
            .expect("actual inspection bearer");
        assert!(inspection.principal().active_execution_binding().is_none());
        let mut durable = physical.session.clone().expect("actual unbound Session");
        let session_id = durable.id.clone();
        assert!(durable.execution_binding.is_none());
        durable.project_state_root = None;
        durable.linked_issue_number = Some(owner_key.number);
        durable
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("legacy metadata on the exact owned source");
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner_key.kind,
            owner_key.number,
            &session_id,
            "gwt-execute",
            false,
        )
        .expect("legacy flat Active projection");
        let work_id = "work-legacy-live";
        let mut start = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            work_id,
            chrono::Utc::now(),
        );
        start.title = Some("Legacy live Work".into());
        start.owner = Some("Issue #2359".into());
        start.agent_session_id = Some(session_id.clone());
        start.agent_id = Some("Codex".into());
        start.display_name = Some("Codex".into());
        start.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/issue-2359".into()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, start).unwrap();
        let mut events = owner.subscribe();
        let writer = close_during_publication.then(|| {
            gwt_agent::session::runtime_writer_lease(&gwt_agent::runtime_state_path(
                &gwt_core::paths::gwt_sessions_dir(),
                &session_id,
            ))
            .expect("hold only the actual runtime proof publication lease")
        });
        let client = owner.client();
        let continuation = tokio::spawn(async move {
            client
                .command(OwnerCommand::ContinueWork {
                    client_id: "legacy-client".into(),
                    operation_id: "legacy-focus-operation".into(),
                    work_id: work_id.into(),
                })
                .await
        });
        let mut close = None;
        if close_during_publication {
            // test-hygiene: allow-wall-clock-deadline hang guard for real authenticated inspection promotion before releasing its publication lease
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if physical
                        .current_capability_grant()
                        .is_some_and(|grant| grant.principal().active_execution_binding().is_some())
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            owner.command(OwnerCommand::Health).await.unwrap();
            let client = owner.client();
            let target = id.clone();
            close = Some(tokio::spawn(async move {
                client
                    .command(OwnerCommand::Close {
                        id: target,
                        incarnation,
                    })
                    .await
            }));
            // test-hygiene: allow-wall-clock-deadline hang guard for the exact actor close fence while binding proof publication is leased
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let inventory = owner.command(OwnerCommand::ListPanes).await.unwrap();
                    if inventory["closing"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| row["id"] == id && row["incarnation"] == incarnation)
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(
                physical.current.load(Ordering::Acquire),
                "deferred close must retain only the exact publication permit until its ACK"
            );
            assert!(owner
                .command(OwnerCommand::SendInput {
                    id: id.clone(),
                    incarnation,
                    text: "must not reach the closing child".into()
                })
                .await
                .is_err());
        }
        let promoted = physical.current_capability_grant();
        drop(writer);
        let result = continuation.await.unwrap();
        if let Some(close) = close {
            assert!(
                result.is_err(),
                "closing cannot publish a successful focus receipt: {result:?}"
            );
            close
                .await
                .unwrap()
                .expect("publication ACK must precede exact physical Close ACK");
        } else {
            result.expect("actual legacy focus ACK");
        }
        // test-hygiene: allow-wall-clock-deadline hang guard for the correlated actual legacy binding-repair outcome
        let (outcome, observed) = tokio::time::timeout(Duration::from_secs(30), async {
            let mut observed = Vec::new();
            loop {
                let event = events.recv().await.unwrap();
                observed.push(event.clone());
                if matches!(&event, OwnerEvent::ContinueWorkOutcome { operation_id, .. } if operation_id == "legacy-focus-operation") {
                    break (event, observed);
                }
            }
        }).await.unwrap();
        let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
            .unwrap()
            .unwrap();
        let current = gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
            .unwrap()
            .unwrap();
        let rebound = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
        )
        .unwrap()
        .execution_binding
        .expect("actual legacy Session must be rebound");
        let grant = if close_during_publication {
            promoted.expect("captured real promoted capability")
        } else {
            let after = owner.probe(&id, incarnation).await.unwrap();
            assert!(Arc::ptr_eq(&after.pty, &physical.pty));
            let grant = after
                .current_capability_grant()
                .expect("same actual bearer promoted to Active");
            owner
                .command(OwnerCommand::Close { id, incarnation })
                .await
                .expect("actual exact legacy child cleanup");
            grant
        };
        owner.shutdown().await;
        if close_during_publication {
            assert!(
                matches!(
                    outcome,
                    OwnerEvent::ContinueWorkOutcome {
                        outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                        ..
                    }
                ),
                "closed repair outcome: {outcome:?}"
            );
            assert!(
                observed
                    .iter()
                    .all(|event| !matches!(event, OwnerEvent::FocusExisting { .. })),
                "a closing pane must not gain a successful focus event"
            );
            assert_eq!(rebound.identity.generation_id, current.generation_id);
            assert_eq!(rebound.identity.binding_id, current.binding_id);
        } else {
            assert!(
                matches!(
                    outcome,
                    OwnerEvent::ContinueWorkOutcome {
                        outcome: gwt::ContinueWorkOutcomeKind::FocusedExisting,
                        retryable: false,
                        ..
                    }
                ),
                "actual legacy continuation: {outcome:?}"
            );
            assert_eq!(rebound.identity, current);
        }
        assert_eq!(ledger.generations.len(), 1);
        assert!(ledger.continuation_attempts.is_empty());
        assert!(ledger.takeover_attempts.is_empty());
        assert_eq!(grant.principal().active_execution_binding(), Some(&rebound));
        assert!(
            !physical.issuer.grant_is_current(&grant),
            "exact Close must revoke the promoted bearer"
        );
        assert!(physical.pty.tree_exit_confirmed());
    });
}

#[test]
fn continue_work_readiness_nonce_mismatch_aborts_candidate_before_commit() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, owner_key, work_id) = owner_continuation_seed(
        temp.path(),
        "continue_work_readiness_nonce_mismatch_aborts_candidate_before_commit",
    );
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
        .expect("predecessor ledger")
        .expect("ledger");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("actual prepared candidate");
        let transport = owner.client();
        let selected = work_id.clone();
        let launch = tokio::spawn(async move {
            transport
                .command(OwnerCommand::ContinueWork {
                    client_id: "client-1".into(),
                    operation_id: "readiness-rejection".into(),
                    work_id: selected,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual preparation reaches the checkpoint");
        let pending = captured
            .pending_continue_work
            .as_ref()
            .expect("actual owner Prepared receipt");
        let candidate = captured.session.as_ref().expect("actual candidate Session");
        assert_eq!(pending.binding.session_id, candidate.id);
        assert!(captured
            .issuer
            .grant_is_current(captured.grant.as_ref().expect("actual Prepared Host grant")));

        let work_before_finalize = tracked_work_event_store_snapshot(&repo);
        checkpoint.release();
        launch
            .await
            .expect("launch worker")
            .expect("physical launch ACK");
        let mut projected = owner.subscribe();
        let mut readiness = runtime_hook_state_for_event("Waiting", "SessionStart", &candidate.id);
        readiness.project_root = Some(repo.display().to_string());
        readiness.branch = Some(candidate.branch.clone());
        readiness.continuation_readiness_nonce = Some("wrong-readiness".into());
        owner.transport_event(gwt::project_transport::TransportEvent::RuntimeHook(
            readiness,
        ));
        // test-hygiene: allow-wall-clock-deadline hang guard for the exact owner outcome; accepted messages establish ordering
        let failure = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let event @ OwnerEvent::ContinueWorkOutcome { .. } =
                    projected.recv().await.expect("owner outcome")
                {
                    break event;
                }
            }
        })
        .await
        .expect("readiness rejection outcome");
        assert!(
            matches!(failure, OwnerEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::Failed, error_code: Some(ref code), ..
        } if code == "launch_failed"),
            "exact owner must reject invalid readiness: {failure:?}"
        );
        assert!(
            !gwt_core::paths::gwt_sessions_dir()
                .join(format!("{}.toml", candidate.id))
                .exists(),
            "rejection must discard only the actual exact candidate Session"
        );
        assert_eq!(
            tracked_work_event_store_snapshot(&repo),
            work_before_finalize,
            "rejected readiness must not rewrite selected Work"
        );
        let after_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
            .expect("rejected ledger")
            .expect("ledger");
        assert_eq!(after_ledger.generations, before_ledger.generations);
        assert_eq!(
            after_ledger.current_generation_id,
            before_ledger.current_generation_id
        );
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &repo,
                owner_key,
                &captured.operation_id
            )
            .expect("rejected attempt")
            .expect("actual attempt")
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
        );
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation: captured.incarnation,
            })
            .await
            .expect("physical cleanup ACK");
        assert!(captured
            .pty
            .as_ref()
            .expect("actual child")
            .try_wait()
            .expect("child wait")
            .is_some());
        owner.shutdown().await;
    });
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
    let runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
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

    let events = continue_work_domain_failure(
        &runtime,
        "client-retry",
        &(operation_id.to_string()),
        &(work_id.to_string()),
    );

    assert_eq!(
        events.outcome,
        gwt::ContinueWorkOutcomeKind::ConflictUnknown
    );
    assert!(events.retryable);
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

    let reconciled = continue_work_domain_failure(
        &runtime,
        "client-retry-exact",
        &(operation_id.to_string()),
        &(work_id.to_string()),
    );
    assert_eq!(reconciled.outcome, gwt::ContinueWorkOutcomeKind::Failed);
    assert!(!reconciled.retryable);
    assert_eq!(reconciled.code, "continuation_aborted");
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
    let runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
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

        let retained = continue_work_domain_failure(
            &runtime,
            "client-retry-dangling",
            &(operation_id.to_string()),
            &(work_id.to_string()),
        );

        assert_eq!(
            retained.outcome,
            gwt::ContinueWorkOutcomeKind::ConflictUnknown
        );
        assert!(retained.retryable);
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

    let raced = continue_work_domain_failure(
        &runtime,
        "client-retry-materialized",
        &(operation_id.to_string()),
        &(work_id.to_string()),
    );

    assert_eq!(raced.outcome, gwt::ContinueWorkOutcomeKind::ConflictUnknown);
    assert!(raced.retryable);
    assert!(candidate_path.exists(), "replacement candidate must remain");
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);
    fs::remove_file(&candidate_path).expect("restore true Missing state for successful cleanup");

    let events = continue_work_domain_failure(
        &runtime,
        "client-retry",
        &(operation_id.to_string()),
        &(work_id.to_string()),
    );
    assert_eq!(events.outcome, gwt::ContinueWorkOutcomeKind::Failed);
    assert!(!events.retryable);
    assert_eq!(events.code, "continuation_aborted");

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
        let runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-cleanup"));
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

        let events =
            continue_work_domain_failure(&runtime, "client-cleanup", &(operation_id), &(work_id));

        assert_eq!(
            events.outcome,
            gwt::ContinueWorkOutcomeKind::ConflictUnknown
        );
        assert!(events.retryable);
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
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let codex_home = temp.path().join("codex-home");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let (_runtime, repo, owner_key, work_id) =
        owner_continuation_seed(temp.path(), "continue-owner-exactly-once");
    persist_owner_continue_conversation(&repo, &codex_home, "historical-session", &repo);
    let predecessor = gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
        .expect("predecessor")
        .expect("binding");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        let owner = production_owner_fixture(&repo).await;
        let mut projected = owner.subscribe();
        let checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("actual prepared successor");
        let transport = owner.client();
        let selected = work_id.clone();
        let launch = tokio::spawn(async move {
            transport
                .command(OwnerCommand::ContinueWork {
                    client_id: "client-1".into(),
                    operation_id: "continue-op-commit".into(),
                    work_id: selected,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait()).await.expect("actual preparation reaches the checkpoint");
        let pending = captured
            .pending_continue_work
            .as_ref()
            .expect("actual continuation receipt");
        let candidate = captured.session.as_ref().expect("actual candidate Session");
        let grant = captured.grant.as_ref().expect("actual Prepared capability");
        assert!(captured.issuer.grant_is_current(grant));
        assert_eq!(pending.binding.session_id, candidate.id);
        assert_eq!(
            pending.outcome,
            gwt::ContinueWorkOutcomeKind::ContinuedConversation
        );
        assert!(
            gwt::cli::execution_state::prepared_execution_binding_matches(
                &repo,
                owner_key,
                &candidate.id,
                &pending.binding.identity
            )
            .expect("exact Prepared authority")
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
                .expect("Prepared ledger")
                .expect("ledger")
                .current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Completed),
            "PTY preparation must leave the predecessor current"
        );
        checkpoint.release();
        launch
            .await
            .expect("actual launch")
            .expect("physical launch ACK");
        assert!(
            !std::iter::from_fn(|| projected.try_recv().ok()).any(|event| matches!(
                event,
                OwnerEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                    ..
                }
            )),
            "PTY spawn alone must not produce success"
        );
        owner
            .command(OwnerCommand::ContinueWork {
                client_id: "client-reconnected".into(),
                operation_id: pending.operation_id.clone(),
                work_id: work_id.clone(),
            })
            .await
            .expect("exact pending retry joins readiness");
        let mut readiness = runtime_hook_state_for_event("Waiting", "SessionStart", &candidate.id);
        readiness.project_root = Some(repo.display().to_string());
        readiness.branch = Some(candidate.branch.clone());
        readiness.continuation_readiness_nonce = Some(pending.readiness_nonce.clone());
        owner.transport_event(gwt::project_transport::TransportEvent::RuntimeHook(
            readiness.clone(),
        ));
        let mut recipients = HashSet::new();
        // test-hygiene: allow-wall-clock-deadline hang guard for the exact owner outcome; accepted messages establish ordering
        tokio::time::timeout(Duration::from_secs(30), async {
            while recipients.len() < 2 {
                if let OwnerEvent::ContinueWorkOutcome {
                    client_id,
                    operation_id,
                    work_id: selected,
                    outcome,
                    ..
                } = projected.recv().await.expect("correlated outcome")
                {
                    assert_eq!(operation_id, pending.operation_id);
                    assert_eq!(selected, work_id);
                    assert_eq!(outcome, gwt::ContinueWorkOutcomeKind::ContinuedConversation);
                    recipients.insert(client_id);
                }
            }
        })
        .await
        .expect("authenticated readiness fanout");
        assert_eq!(
            recipients,
            HashSet::from(["client-1".to_string(), "client-reconnected".to_string()])
        );
        let promoted = owner.probe(&captured.id, captured.incarnation).await.expect("actual promoted runtime");
        assert!(promoted.has_capability_bearer(grant), "promotion retains the same actual bearer");
        let active_grant = promoted.grant.as_ref().expect("current promoted grant");
        assert!(captured.issuer.grant_is_current(active_grant));
        assert_eq!(active_grant.principal().active_execution_binding(), Some(&pending.binding));
        assert!(!captured.issuer.grant_is_current(grant), "the frozen Prepared principal cannot remain current after promotion");
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
                .expect("committed binding"),
            Some(pending.binding.identity.clone())
        );
        assert!(
            !gwt::cli::execution_state::current_active_execution_binding_matches(
                &repo,
                owner_key,
                "historical-session",
                &predecessor
            )
            .expect("predecessor fence")
        );
        let projection = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("Work")
            .expect("projection");
        let selected = projection
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("selected Work");
        assert_eq!(
            selected.status_category,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active
        );
        assert!(selected
            .agents
            .iter()
            .any(|agent| agent.session_id == candidate.id));
        owner.transport_event(gwt::project_transport::TransportEvent::RuntimeHook(
            readiness,
        ));
        owner
            .command(OwnerCommand::Health)
            .await
            .expect("duplicate readiness barrier");
        assert!(
            !std::iter::from_fn(|| projected.try_recv().ok())
                .any(|event| matches!(event, OwnerEvent::ContinueWorkOutcome { .. })),
            "consumed readiness must not emit a second success"
        );
        owner
            .command(OwnerCommand::ContinueWork {
                client_id: "client-after-response-loss".into(),
                operation_id: pending.operation_id.clone(),
                work_id: work_id.clone(),
            })
            .await
            .expect("same exact operation replay");
        // test-hygiene: allow-wall-clock-deadline hang guard for the exact owner outcome; accepted messages establish ordering
        let replay = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = projected.recv().await.expect("replay");
                if matches!(&event, OwnerEvent::ContinueWorkOutcome { client_id, .. } if client_id == "client-after-response-loss") { break event; }
            }
        })
        .await
        .expect("durable live response-loss replay");
        assert!(matches!(
            replay,
            OwnerEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                retryable: false,
                ..
            }
        ));
        let live = owner
            .probe(&captured.id, captured.incarnation)
            .await
            .expect("same live exact pane remains installed");
        assert!(Arc::ptr_eq(
            &live.pty,
            captured.pty.as_ref().expect("actual child")
        ));
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation: captured.incarnation,
            })
            .await
            .expect("exact close ACK");
        owner.shutdown().await;
        let restarted = production_owner_fixture(&repo).await;
        let mut retry_events = restarted.subscribe();
        restarted
            .command(OwnerCommand::ContinueWork {
                client_id: "client-retry".into(),
                operation_id: pending.operation_id.clone(),
                work_id: work_id.clone(),
            })
            .await
            .expect("terminal exact operation retry");
        assert!(
            !std::iter::from_fn(|| retry_events.try_recv().ok()).any(|event| matches!(
                event,
                OwnerEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                    ..
                }
            )),
            "a non-live pane cannot be reconstructed as strong continuation success"
        );
        let readback = gwt::cli::execution_state::load_generation_ledger(&repo, owner_key)
            .expect("retry ledger")
            .expect("ledger");
        assert_eq!(readback.generations.len(), 2);
        assert_eq!(
            readback
                .continuation_attempts
                .iter()
                .filter(|attempt| attempt.status
                    == gwt::cli::execution_state::ContinuationAttemptStatus::Activated)
                .count(),
            1,
            "response-loss retry must not append a second activation"
        );
        restarted.shutdown().await;
    });
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
    assert_fresh_execution_preserves_terminal_work(
        gwt_core::workspace_projection::WorkEventKind::Discard,
    );
}

#[test]
fn fresh_execution_authenticated_session_start_creates_successor_after_done_work() {
    assert_fresh_execution_preserves_terminal_work(
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

fn assert_fresh_execution_preserves_terminal_work(
    terminal_kind: gwt_core::workspace_projection::WorkEventKind,
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-terminal-history",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        Some(WorkspaceResumeContext {
            title: Some("Fresh legacy recovery".into()),
            owner: Some("Issue #2359".into()),
            summary: None,
            next_action: None,
        }),
    );
    let repo = fixture.repo.clone();
    let owner = fixture.authority;
    let predecessor_session_id = "predecessor-fresh-terminal-history";
    let candidate_session_id = fixture.pending.binding.session_id.as_str();
    let predecessor_session_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("{predecessor_session_id}.toml"));
    let predecessor_session = gwt_agent::Session::load(&predecessor_session_path).unwrap();
    let predecessor_session_bytes = fs::read(&predecessor_session_path).unwrap();
    let predecessor_generation = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .unwrap()
        .unwrap()
        .generations[0]
        .clone();
    let predecessor_binding = fixture.predecessor_binding.clone();
    let mut predecessor_active = sample_active_agent_session("tab-1", "tab-1::historical");
    predecessor_active.session_id = predecessor_session_id.into();
    predecessor_active.branch_name = predecessor_session.branch.clone();
    predecessor_active.worktree_path = repo.clone();
    predecessor_active.agent_id = predecessor_session.agent_id.to_string();
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
                    terminal_kind,
                    projection.id.clone(),
                    Utc::now(),
                );
                // The foreign container is historical input, not a new local
                // shard. Seed its original bytes before transaction replay.
                let shard =
                    gwt_core::paths::gwt_repo_local_work_event_shard_path(&repo, &other_host.id);
                fs::create_dir_all(shard.parent().expect("legacy shard bucket"))?;
                let mut bytes = serde_json::to_vec(&other_host).expect("legacy event");
                bytes.push(b'\n');
                fs::write(shard, bytes)?;
                Ok((projection.id.clone(), vec![other_host, event]))
            },
        )
        .expect("close predecessor Work");
    let predecessor_work_snapshot =
        gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .unwrap()
            .unwrap()
            .work_items
            .into_iter()
            .find(|work| work.id == predecessor_work)
            .expect("terminal predecessor Work");
    assert!(predecessor_work_snapshot.is_terminal());
    assert_eq!(predecessor_work_snapshot.execution_containers.len(), 2);

    let receipt = commit_owner_fresh_readiness(&fixture);
    assert!(receipt.validated);
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(probe.readiness_watch.is_none());
    assert!(!durable_launch_recovery_exists(
        &gwt_core::paths::gwt_sessions_dir(),
        candidate_session_id
    ));
    assert!(probe.has_capability_bearer(fixture.captured.grant.as_ref().unwrap()));
    assert!(probe.issuer.grant_is_current(probe.grant.as_ref().unwrap()));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner).unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .unwrap()
        .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(ledger.generations[0], predecessor_generation);
    assert_ne!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner).unwrap(),
        Some(predecessor_binding)
    );
    assert_eq!(
        fs::read(&predecessor_session_path).unwrap(),
        predecessor_session_bytes
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
        "fresh activation preserves terminal Work and exact historical membership"
    );
    let successor = works
        .work_items
        .iter()
        .find(|work| {
            work.agents
                .iter()
                .any(|agent| agent.session_id == candidate_session_id)
        })
        .unwrap();
    assert_ne!(successor.id, predecessor_work);
    assert!(!successor.is_terminal());
    assert!(successor.related_work_item_ids.contains(&predecessor_work));
    save_workspace_launch_projection(
        &repo,
        probe.active_session.as_ref().unwrap(),
        None,
        Some(owner.number),
        Some(owner),
        None,
        WorkspaceLaunchProjectionKind::Resume {
            created_by_start_work: true,
        },
        Some(&HashSet::from([candidate_session_id.to_string()])),
    )
    .expect("retry reuses successor Work");
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
            Some(&HashSet::from([predecessor_session_id.to_string()]))
        )
        .is_err(),
        "a terminal predecessor Session cannot move to its successor"
    );
    let (status, update) = fixture
        .executor
        .block_on(probe.workspace_update(&gwt::AgentWorkspaceUpdateRequest {
            schema_version: gwt::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
            claimed_session_id: candidate_session_id.to_string(),
            observation: gwt::observe_agent_runtime(&repo).unwrap(),
            intent: gwt::AgentWorkspaceUpdateIntent {
                current_focus: Some("Continue the next owner slice".into()),
                ..Default::default()
            },
        }))
        .unwrap();
    assert!(
        status.is_success(),
        "actual bound workspace.update: {status}: {update}"
    );
    assert_eq!(update["work_id"], successor.id);
    assert_eq!(
        fs::read(&predecessor_session_path).unwrap(),
        predecessor_session_bytes
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
    fixture.shutdown();
}
