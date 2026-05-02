use std::path::PathBuf;

use gwt_core::daemon::{
    persist_endpoint, resolve_bootstrap_action, validate_handshake, ClientFrame,
    DaemonBootstrapAction, DaemonEndpoint, DaemonFrame, DaemonStatus, HookEnvelope,
    IpcHandshakeRequest, IpcHandshakeResponse, RuntimeScope, RuntimeTarget,
    DAEMON_PROTOCOL_VERSION,
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
fn runtime_scope_rejects_empty_repo_hash() {
    let project_root = tempdir().unwrap();
    let err = RuntimeScope::new(
        "",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap_err();
    assert!(err.to_string().contains("repo_hash"));
}

#[test]
fn runtime_scope_rejects_empty_worktree_hash() {
    let project_root = tempdir().unwrap();
    let err = RuntimeScope::new(
        "repo-scope-1234",
        "  ",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap_err();
    assert!(err.to_string().contains("worktree_hash"));
}

#[test]
fn runtime_scope_rejects_relative_project_root() {
    let err = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        PathBuf::from("relative/path"),
        RuntimeTarget::Host,
    )
    .unwrap_err();
    assert!(err.to_string().contains("absolute"));
}

#[test]
fn daemon_endpoint_is_usable_returns_false_for_mismatched_protocol() {
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
    assert!(!endpoint.is_usable(&scope, DAEMON_PROTOCOL_VERSION + 1, |_| true));
}

#[test]
fn daemon_endpoint_is_usable_returns_false_for_empty_bind() {
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
        "  ".into(),
        "secret-token".into(),
        "0.1.0".into(),
    );
    assert!(!endpoint.is_usable(&scope, DAEMON_PROTOCOL_VERSION, |_| true));
}

#[test]
fn daemon_endpoint_is_usable_returns_false_for_empty_auth_token() {
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
        "".into(),
        "0.1.0".into(),
    );
    assert!(!endpoint.is_usable(&scope, DAEMON_PROTOCOL_VERSION, |_| true));
}

#[test]
fn daemon_endpoint_is_usable_returns_false_for_dead_process() {
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
    assert!(!endpoint.is_usable(&scope, DAEMON_PROTOCOL_VERSION, |_| false));
}

#[test]
fn validate_handshake_rejects_scope_mismatch() {
    let project_a = tempdir().unwrap();
    let project_b = tempdir().unwrap();
    let scope_a = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_a.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let scope_b = RuntimeScope::new(
        "repo-scope-9999",
        "worktree-scope-0000",
        project_b.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let endpoint = DaemonEndpoint::new(
        scope_a.clone(),
        4242,
        "http://127.0.0.1:7777".into(),
        "secret-token".into(),
        "0.1.0".into(),
    );
    let request = IpcHandshakeRequest {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        auth_token: "secret-token".into(),
        scope: scope_b,
    };
    let response = IpcHandshakeResponse {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        daemon_version: "0.1.0".into(),
        accepted: true,
        rejection_reason: None,
    };
    assert!(validate_handshake(&endpoint, &request, &response)
        .unwrap_err()
        .to_string()
        .contains("scope"));
}

#[test]
fn validate_handshake_rejects_with_reason() {
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
        accepted: false,
        rejection_reason: Some("version too old".into()),
    };
    let err = validate_handshake(&endpoint, &request, &response)
        .unwrap_err()
        .to_string();
    assert!(err.contains("rejected"));
    assert!(err.contains("version too old"));
}

#[test]
fn validate_handshake_rejects_without_reason() {
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
        accepted: false,
        rejection_reason: None,
    };
    let err = validate_handshake(&endpoint, &request, &response)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown rejection"));
}

#[test]
fn resolve_bootstrap_action_spawns_when_endpoint_file_missing() {
    let gwt_home = tempdir().unwrap();
    let project_root = tempdir().unwrap();
    let scope = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let result =
        resolve_bootstrap_action(gwt_home.path(), &scope, DAEMON_PROTOCOL_VERSION, |_| true)
            .unwrap();
    assert!(matches!(result, DaemonBootstrapAction::Spawn { .. }));
}

#[test]
fn resolve_bootstrap_action_spawns_when_endpoint_is_malformed() {
    let gwt_home = tempdir().unwrap();
    let project_root = tempdir().unwrap();
    let scope = RuntimeScope::new(
        "repo-scope-1234",
        "worktree-scope-5678",
        project_root.path().to_path_buf(),
        RuntimeTarget::Host,
    )
    .unwrap();
    let ep_path = scope.endpoint_path(gwt_home.path());
    std::fs::create_dir_all(ep_path.parent().unwrap()).unwrap();
    std::fs::write(&ep_path, b"not-json").unwrap();

    let result =
        resolve_bootstrap_action(gwt_home.path(), &scope, DAEMON_PROTOCOL_VERSION, |_| true)
            .unwrap();
    assert!(matches!(result, DaemonBootstrapAction::Spawn { .. }));
    assert!(!ep_path.exists());
}

#[test]
fn persist_endpoint_round_trips_through_file_system() {
    let gwt_home = tempdir().unwrap();
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
    let path = scope.endpoint_path(gwt_home.path());
    persist_endpoint(&path, &endpoint).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let loaded: DaemonEndpoint = serde_json::from_str(&content).unwrap();
    assert_eq!(loaded.pid, 4242);
    assert_eq!(loaded.bind, "http://127.0.0.1:7777");
    assert_eq!(loaded.auth_token, "secret-token");
}

#[test]
fn client_frame_hook_round_trips_through_json() {
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
        session_id: Some("sess-1".into()),
        cwd: PathBuf::from(project_root.path()),
        payload: json!({"command": "codex"}),
    };

    let frame = ClientFrame::Hook(envelope);
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "hook");
    assert_eq!(json_value["hook_name"], "pre-command");
    assert_eq!(json_value["payload"]["command"], "codex");

    let round_trip: ClientFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn client_frame_subscribe_serializes_channel_list() {
    let frame = ClientFrame::Subscribe {
        channels: vec!["board".to_string(), "runtime-status".to_string()],
    };
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "subscribe");
    assert_eq!(json_value["channels"][0], "board");
    assert_eq!(json_value["channels"][1], "runtime-status");

    let round_trip: ClientFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn daemon_frame_ack_serializes_to_canonical_shape() {
    let frame = DaemonFrame::Ack;
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "ack");
    let round_trip: DaemonFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn daemon_frame_event_carries_channel_and_payload() {
    let frame = DaemonFrame::Event {
        channel: "board".to_string(),
        payload: json!({"entries": 3}),
    };
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "event");
    assert_eq!(json_value["channel"], "board");
    assert_eq!(json_value["payload"]["entries"], 3);
    let round_trip: DaemonFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn daemon_frame_error_serializes_message() {
    let frame = DaemonFrame::Error {
        message: "unknown frame type".to_string(),
    };
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "error");
    assert_eq!(json_value["message"], "unknown frame type");
    let round_trip: DaemonFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn client_frame_status_serializes_to_canonical_shape() {
    let frame = ClientFrame::Status;
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "status");
    let round_trip: ClientFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn daemon_frame_status_carries_uptime_and_channel_count() {
    let frame = DaemonFrame::Status(DaemonStatus {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        daemon_version: "9.14.0".to_string(),
        uptime_seconds: 42,
        broadcast_channels: 3,
        connections: 2,
    });
    let json_value = serde_json::to_value(&frame).unwrap();
    assert_eq!(json_value["type"], "status");
    assert_eq!(json_value["protocol_version"], DAEMON_PROTOCOL_VERSION);
    assert_eq!(json_value["daemon_version"], "9.14.0");
    assert_eq!(json_value["uptime_seconds"], 42);
    assert_eq!(json_value["broadcast_channels"], 3);
    assert_eq!(json_value["connections"], 2);
    let round_trip: DaemonFrame = serde_json::from_value(json_value).unwrap();
    assert_eq!(round_trip, frame);
}

#[test]
fn daemon_status_connections_field_defaults_to_zero_when_missing() {
    // Older daemons may not include `connections` in their wire form;
    // serde must treat the missing field as 0 to keep forward-compat
    // working for clients reading from a daemon that predates the
    // counter.
    let json_value = json!({
        "type": "status",
        "protocol_version": DAEMON_PROTOCOL_VERSION,
        "daemon_version": "old",
        "uptime_seconds": 1,
        "broadcast_channels": 0,
    });
    let frame: DaemonFrame = serde_json::from_value(json_value).unwrap();
    match frame {
        DaemonFrame::Status(status) => {
            assert_eq!(status.connections, 0);
        }
        other => panic!("expected Status frame, got: {other:?}"),
    }
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
