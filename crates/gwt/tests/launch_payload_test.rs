//! Launch payload contracts through the GUI-independent library boundary.

use std::collections::HashMap;

use gwt::project_runtime::launch_payload::ProcessLaunch;

#[test]
fn process_launch_debug_redacts_agent_capability_and_session_identity() {
    let secret = "agent-capability-secret-sentinel";
    let readiness = "continue-readiness-secret-sentinel";
    let launch = ProcessLaunch {
        initial_prompt_file: None,
        command: "docker".to_string(),
        args: vec![
            format!("{}={secret}", gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            format!("{}=session-private", gwt_agent::GWT_SESSION_ID_ENV),
            format!(
                "{}={readiness}",
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV
            ),
        ],
        env: HashMap::from([
            (
                gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                secret.to_string(),
            ),
            (
                gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                "session-private".to_string(),
            ),
            (
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
                readiness.to_string(),
            ),
        ]),
        remove_env: Vec::new(),
        cwd: None,
        resource_policy: None,
    };

    let debug = format!("{launch:?}");
    assert!(!debug.contains(secret));
    assert!(!debug.contains(readiness));
    assert!(!debug.contains("session-private"));
    assert!(debug.contains("<redacted>"));
}
