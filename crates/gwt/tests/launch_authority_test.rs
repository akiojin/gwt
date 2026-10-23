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
