//! Issue #4647: project-owned runtime state must follow ProjectKey + generation.
#[test]
fn remaining_project_state_has_one_project_owner() {
    let source = include_str!("../src/app_runtime/mod.rs");
    let project = source
        .split("pub(crate) struct ProjectRuntimeState {")
        .nth(1)
        .unwrap()
        .split("\n}")
        .next()
        .unwrap();
    let app = source
        .split("pub struct AppRuntime {")
        .nth(1)
        .unwrap()
        .split("\n}")
        .next()
        .unwrap();
    for field in [
        "work_merged_branches",
        "work_items_cache",
        "active_work_projection_cache",
        "active_work_projection_payload_cache",
        "pm_sessions",
        "pending_pm_launches",
        "pending_pm_closes",
        "pending_pm_wakes",
        "pending_pm_worktree_preparations",
        "pending_launch_wizard_materializations",
        "issue_monitor_scheduled_scans_in_flight",
    ] {
        assert!(
            project.contains(&format!("{field}:")),
            "{field} must belong to ProjectRuntimeState"
        );
        assert!(
            !app.contains(&format!("{field}:")),
            "{field} must not have a second AppRuntime owner"
        );
    }
}
