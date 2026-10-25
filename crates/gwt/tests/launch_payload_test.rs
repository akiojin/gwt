//! Launch payload contracts through the GUI-independent library boundary.

use std::collections::HashMap;

use gwt::project_runtime::launch_payload::{LaunchFeedbackContext, ProcessLaunch};

#[test]
fn review_dispatch_feedback_context_does_not_own_the_issue_launch_binding() {
    // Issue #4041: the independent review window observes the Issue but never
    // materializes its launch.
    let mut context = LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(42),
        issue_monitor_delivery_id: None,
        issue_monitor_project_root: None,
        issue_monitor_session_mode: None,
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    };
    assert_eq!(
        context.issue_monitor_launch_binding_issue_number(),
        Some(42)
    );
    context.issue_monitor_review_dispatch = true;
    assert_eq!(context.issue_monitor_launch_binding_issue_number(), None);
}

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
