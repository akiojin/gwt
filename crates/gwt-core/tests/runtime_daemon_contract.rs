use std::path::PathBuf;

use gwt_core::daemon::{
    persist_endpoint, resolve_bootstrap_action, validate_handshake, DaemonBootstrapAction,
    DaemonEndpoint, HookEnvelope, IpcHandshakeRequest, IpcHandshakeResponse, RuntimeScope,
    RuntimeTarget, DAEMON_PROTOCOL_VERSION,
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn daemon_endpoint_path_is_scoped_by_repo_and_worktree() {
    let project_root = tempdir().unwrap();
    let scope = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();

    let gwt_home = tempdir().unwrap();
    let endpoint_path = scope.endpoint_path(gwt_home.path());

    assert!(endpoint_path.starts_with(
        gwt_home
            .path()
            .join("projects")
            .join("repo-scope-1234")
            .join("runtime")
            .join("daemon")
    ));
    assert_eq!(
        endpoint_path.file_name().unwrap(),
        "worktree-scope-5678.json"
    );
}

#[test]
fn bootstrap_reuses_live_endpoint_and_rejects_stale_or_mismatched_versions() {
    let project_root = tempdir().unwrap();
    let scope = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let gwt_home = tempdir().unwrap();

    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        4242,
        "http://127.0.0.1:7777".into(),
        "secret-token".into(),
        "0.1.0".into(),
    );
    persist_endpoint(&scope.endpoint_path(gwt_home.path()), &endpoint).unwrap();

    let reuse = resolve_bootstrap_action(gwt_home.path(), &scope, DAEMON_PROTOCOL_VERSION, |pid| {
        pid == 4242
    })
    .unwrap();
    assert_eq!(reuse, DaemonBootstrapAction::Reuse(endpoint.clone()));

    let stale =
        resolve_bootstrap_action(gwt_home.path(), &scope, DAEMON_PROTOCOL_VERSION, |_| false)
            .unwrap();
    assert!(matches!(
        stale,
        DaemonBootstrapAction::Spawn { ref endpoint_path } if endpoint_path == &scope.endpoint_path(gwt_home.path())
    ));
    assert!(!scope.endpoint_path(gwt_home.path()).exists());

    let mismatched = DaemonEndpoint {
        protocol_version: DAEMON_PROTOCOL_VERSION + 1,
        ..endpoint
    };
    persist_endpoint(&scope.endpoint_path(gwt_home.path()), &mismatched).unwrap();

    let restart =
        resolve_bootstrap_action(gwt_home.path(), &scope, DAEMON_PROTOCOL_VERSION, |pid| {
            pid == 4242
        })
        .unwrap();
    assert!(matches!(
        restart,
        DaemonBootstrapAction::Spawn { ref endpoint_path } if endpoint_path == &scope.endpoint_path(gwt_home.path())
    ));
    assert!(!scope.endpoint_path(gwt_home.path()).exists());
}

#[test]
fn authenticated_handshake_accepts_matching_contract_and_rejects_mismatch() {
    let project_root = tempdir().unwrap();
    let scope = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        4242,
        "http://127.0.0.1:7777".into(),
        "secret-token".into(),
        "0.1.0".into(),
    );

    let request = IpcHandshakeRequest {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        auth_token: "secret-token".into(),
        scope: scope.clone(),
    };
    let response = IpcHandshakeResponse {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        daemon_version: "0.1.0".into(),
        accepted: true,
        rejection_reason: None,
    };
    validate_handshake(&endpoint, &request, &response).unwrap();

    let bad_token = IpcHandshakeRequest {
        auth_token: "wrong-token".into(),
        ..request.clone()
    };
    assert!(validate_handshake(&endpoint, &bad_token, &response)
        .unwrap_err()
        .to_string()
        .contains("token"));

    let bad_request_protocol = IpcHandshakeRequest {
        protocol_version: DAEMON_PROTOCOL_VERSION + 1,
        ..request.clone()
    };
    assert!(
        validate_handshake(&endpoint, &bad_request_protocol, &response)
            .unwrap_err()
            .to_string()
            .contains("protocol")
    );

    let bad_response_protocol = IpcHandshakeResponse {
        protocol_version: DAEMON_PROTOCOL_VERSION + 1,
        ..response
    };
    assert!(
        validate_handshake(&endpoint, &request, &bad_response_protocol)
            .unwrap_err()
            .to_string()
            .contains("protocol")
    );
}

#[test]
fn hook_envelope_serializes_runtime_scope_and_payload() {
    let project_root = tempdir().unwrap();
    let scope = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let envelope = HookEnvelope {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        scope,
        hook_name: "pre-command".into(),
        session_id: Some("session-123".into()),
        cwd: PathBuf::from(project_root.path()),
        payload: json!({
            "event": "agent-started",
            "command": "codex"
        }),
    };

    let json = serde_json::to_value(&envelope).unwrap();
    assert_eq!(json["hook_name"], "pre-command");
    assert_eq!(json["payload"]["event"], "agent-started");
    assert_eq!(json["scope"]["repo_hash"], "repo-scope-1234");
    assert_eq!(json["scope"]["worktree_hash"], "worktree-scope-5678");
}
