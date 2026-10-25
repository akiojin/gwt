//! Active Resume binding contracts through the shared project runtime.

use std::path::PathBuf;

use gwt::project_runtime::launch_authority::install_authenticated_active_resume_binding;

#[test]
fn authenticated_active_resume_binding_repairs_missing_owner_before_install() {
    let mut session = gwt_agent::Session::new(
        PathBuf::from("missing-owner-session-worktree"),
        "work/issue-42",
        gwt_agent::AgentId::Codex,
    );
    session.id = "predecessor-session".to_string();
    session.repo_hash = Some("trusted-repository".to_string());
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: "successor-session".to_string(),
        repo_hash: "trusted-repository".to_string(),
        owner_kind: "issue".to_string(),
        owner_number: 42,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation-42".to_string(),
            binding_id: "binding-42".to_string(),
            ledger_head_hash: "ledger-head-42".to_string(),
        },
        capability_generation: 1,
    };
    let mut launch_owner = None;

    install_authenticated_active_resume_binding(&mut session, &mut launch_owner, &binding)
        .expect("authenticated binding must repair a missing launch owner");

    assert_eq!(session.id, binding.session_id);
    assert_eq!(session.linked_issue_number, Some(42));
    assert_eq!(session.execution_binding.as_ref(), Some(&binding));
    assert_eq!(launch_owner, Some(42));
}

#[test]
fn authenticated_active_resume_binding_rejects_foreign_owner_without_mutation() {
    let mut session = gwt_agent::Session::new(
        PathBuf::from("foreign-owner-session-worktree"),
        "work/issue-41",
        gwt_agent::AgentId::Codex,
    );
    session.id = "foreign-owner-session".to_string();
    session.linked_issue_number = Some(41);
    let original_id = session.id.clone();
    let original_owner = session.linked_issue_number;
    let original_binding = session.execution_binding.clone();
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: "trusted-owner-session".to_string(),
        repo_hash: "trusted-repository".to_string(),
        owner_kind: "issue".to_string(),
        owner_number: 42,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation-42".to_string(),
            binding_id: "binding-42".to_string(),
            ledger_head_hash: "ledger-head-42".to_string(),
        },
        capability_generation: 1,
    };
    let mut launch_owner = Some(41);

    let error =
        install_authenticated_active_resume_binding(&mut session, &mut launch_owner, &binding)
            .expect_err("a trusted foreign owner must not overwrite Session authority");

    assert!(
        error.contains("does not match the linked Session owner"),
        "{error}"
    );
    assert_eq!(session.id, original_id);
    assert_eq!(session.linked_issue_number, original_owner);
    assert_eq!(session.execution_binding, original_binding);
    assert_eq!(launch_owner, Some(41));
}

use gwt::agent_capability::AgentCapabilityIssuer;
use gwt::project_runtime::launch_finalization::FinalizedAgentCapabilityLaunch;
use gwt_core::test_support::ScopedEnvVar;
use std::collections::HashMap;
use std::path::Path;

fn init_execution_repo(repo: &Path, branch: &str) {
    std::fs::create_dir_all(repo).expect("create execution repository");
    for args in [
        vec!["init", "-q", "-b", branch],
        vec![
            "remote",
            "add",
            "origin",
            "https://github.com/example/launch-binding.git",
        ],
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(&args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct PersistedExecutionLaunch {
    project: PathBuf,
    sessions_dir: PathBuf,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session: gwt_agent::Session,
}

fn persisted_execution_launch(home: &Path) -> PersistedExecutionLaunch {
    let project = home.join("project");
    init_execution_repo(&project, "work/issue-2359");
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        number: 2359,
    };
    let mut session =
        gwt_agent::Session::new(&project, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.project_state_root = Some(project.clone());
    session.linked_issue_number = Some(owner.number);
    session.update_status(gwt_agent::AgentStatus::Running);
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
    gwt::session_launch::persist_finalized_launch_session(
        &sessions_dir,
        Some(&runtime_path),
        &mut session,
        None,
    )
    .expect("persist finalized Session before authority");
    PersistedExecutionLaunch {
        project,
        sessions_dir,
        owner,
        session,
    }
}

#[test]
fn producing_launch_persists_exact_binding_before_issuing_bound_capability() {
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home tempdir");
    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    let mut launch = persisted_execution_launch(home.path());

    let issuer = AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45123/internal/hook-live",
        "ws://127.0.0.1:46234/ws",
        "ws://127.0.0.1:45123/internal/pane-ws",
    );
    let mut env = HashMap::new();
    FinalizedAgentCapabilityLaunch {
        issuer: Some(&issuer),
        sessions_dir: &launch.sessions_dir,
        session: &mut launch.session,
        project_root: &launch.project,
        worktree: &launch.project,
        producing_owner: Some(launch.owner),
        prepared_continuation: None,
        rebound_continuation: None,
        execution_entrypoint: "$gwt-execute #2359",
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        container_runtime: None,
        permission_decision: None,
    }
    .install(&mut env)
    .expect("install exact bound launch authority");

    let persisted = gwt_agent::Session::load(
        &launch
            .sessions_dir
            .join(format!("{}.toml", launch.session.id)),
    )
    .expect("reload bound Session");
    let binding = persisted
        .execution_binding
        .expect("producing Session binding");
    assert_eq!(binding.capability_generation, 1);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
            .expect("read current generation")
            .expect("current generation binding"),
        binding.identity
    );
    let token = env
        .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .expect("bound capability token");
    let grant = issuer
        .grant_for_test(token)
        .expect("authenticate issued capability");
    assert!(grant.principal().authorizes_producing_mutation());
    assert_eq!(grant.principal().execution_binding(), Some(&binding));
}

#[test]
fn prepared_continuation_issues_observation_only_capability_without_activating_successor() {
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home tempdir");
    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    let mut predecessor = persisted_execution_launch(home.path());
    let issuer = AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45123/internal/hook-live",
        "ws://127.0.0.1:46234/ws",
        "ws://127.0.0.1:45123/internal/pane-ws",
    );
    let mut predecessor_env = HashMap::new();
    FinalizedAgentCapabilityLaunch {
        issuer: Some(&issuer),
        sessions_dir: &predecessor.sessions_dir,
        session: &mut predecessor.session,
        project_root: &predecessor.project,
        worktree: &predecessor.project,
        producing_owner: Some(predecessor.owner),
        prepared_continuation: None,
        rebound_continuation: None,
        execution_entrypoint: "$gwt-execute #2359",
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        container_runtime: None,
        permission_decision: None,
    }
    .install(&mut predecessor_env)
    .expect("materialize predecessor generation");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &predecessor.project,
            &predecessor.session.id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    let settled_predecessor_identity = gwt::cli::execution_state::current_execution_binding(
        &predecessor.project,
        predecessor.owner,
    )
    .expect("read settled predecessor binding")
    .expect("settled predecessor generation");

    let continuation_session_id = uuid::Uuid::new_v4().to_string();
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "prepared-launch-operation".to_string(),
        principal_id: "host-test-principal".to_string(),
        work_id: Some("work-prepared-launch".to_string()),
        source: "continue-work".to_string(),
        session_binding_id: uuid::Uuid::new_v4().to_string(),
        initial_session_id: continuation_session_id.clone(),
        entrypoint: "resume".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_successor(&predecessor.project, predecessor.owner, &request)
        .expect("prepare successor");
    let planned_identity = gwt::cli::execution_state::prepared_successor_execution_binding(
        &predecessor.project,
        predecessor.owner,
        &request,
    )
    .expect("derive exact future binding");
    let mut continuation = gwt_agent::Session::new(
        &predecessor.project,
        "work/issue-2359",
        gwt_agent::AgentId::Codex,
    );
    continuation.id = continuation_session_id;
    continuation.project_state_root = Some(predecessor.project.clone());
    continuation.linked_issue_number = Some(predecessor.owner.number);
    continuation.update_status(gwt_agent::AgentStatus::Running);
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: continuation.id.clone(),
        repo_hash: continuation
            .repo_hash
            .clone()
            .expect("continuation repository hash"),
        owner_kind: predecessor.owner.kind.as_str().to_string(),
        owner_number: predecessor.owner.number,
        identity: planned_identity.clone(),
        capability_generation: 1,
    };
    continuation
        .set_execution_binding(Some(binding.clone()))
        .expect("bind Prepared continuation Session");

    let mut env = HashMap::new();
    FinalizedAgentCapabilityLaunch {
        issuer: Some(&issuer),
        sessions_dir: &predecessor.sessions_dir,
        session: &mut continuation,
        project_root: &predecessor.project,
        worktree: &predecessor.project,
        producing_owner: None,
        prepared_continuation: Some(&binding),
        rebound_continuation: None,
        execution_entrypoint: "resume",
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        container_runtime: None,
        permission_decision: None,
    }
    .install(&mut env)
    .expect("issue exact Prepared capability");

    let token = env
        .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .expect("Prepared capability token");
    let grant = issuer
        .grant_for_test(token)
        .expect("authenticate Prepared capability");
    assert_eq!(
        grant.principal().execution_authority_kind(),
        gwt::agent_capability::AgentExecutionAuthorityKind::Prepared
    );
    assert!(!grant.principal().authorizes_producing_mutation());
    assert_eq!(grant.principal().execution_binding(), Some(&binding));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &predecessor.project,
            predecessor.owner,
        )
        .expect("read current predecessor binding"),
        Some(settled_predecessor_identity),
        "issuing Prepared authority must not activate the successor"
    );
    assert!(
        gwt::cli::execution_state::prepared_execution_binding_matches(
            &predecessor.project,
            predecessor.owner,
            &continuation.id,
            &planned_identity,
        )
        .expect("read Prepared authority")
    );
}
