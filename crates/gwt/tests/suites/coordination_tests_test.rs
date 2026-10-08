//! Issue #4135: coordination tests share one integration harness.

#[path = "../autonomous_e2e.rs"]
mod autonomous_e2e;
#[path = "../autonomous_handoff_control_plane_test.rs"]
mod autonomous_handoff_control_plane_test;
#[path = "../autonomous_handoff_test.rs"]
mod autonomous_handoff_test;
#[path = "../autonomous_live_gh.rs"]
mod autonomous_live_gh;
#[path = "../autonomous_question_guard_test.rs"]
mod autonomous_question_guard_test;
#[path = "../autonomous_real_gh_smoke.rs"]
mod autonomous_real_gh_smoke;
#[path = "../blocked_escalation_e2e.rs"]
mod blocked_escalation_e2e;
#[path = "../board_cli_test.rs"]
mod board_cli_test;
#[path = "../board_reminder_hook_test.rs"]
mod board_reminder_hook_test;
#[path = "../close_work_protocol_test.rs"]
mod close_work_protocol_test;
#[path = "../issue_monitor_branch_prune_test.rs"]
mod issue_monitor_branch_prune_test;
#[path = "../issue_monitor_protocol_test.rs"]
mod issue_monitor_protocol_test;
#[path = "../issue_monitor_stop_ipc_test.rs"]
mod issue_monitor_stop_ipc_test;
#[path = "../issue_monitor_test.rs"]
mod issue_monitor_test;
#[path = "../managed_launch_lifecycle_e2e.rs"]
mod managed_launch_lifecycle_e2e;
#[path = "../workspace_identity_session_start_test.rs"]
mod workspace_identity_session_start_test;
