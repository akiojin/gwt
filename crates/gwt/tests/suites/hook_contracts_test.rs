//! Issue #4135: hook contracts share one integration harness.

#[path = "../hook_block_bash_policy_test.rs"]
mod hook_block_bash_policy_test;
#[path = "../hook_block_cd_test.rs"]
mod hook_block_cd_test;
#[path = "../hook_block_file_ops_test.rs"]
mod hook_block_file_ops_test;
#[path = "../hook_block_git_branch_ops_test.rs"]
mod hook_block_git_branch_ops_test;
#[path = "../hook_block_git_dir_override_test.rs"]
mod hook_block_git_dir_override_test;
#[path = "../hook_diagnostics_test.rs"]
mod hook_diagnostics_test;
#[path = "../hook_exit_codes_test.rs"]
mod hook_exit_codes_test;
#[path = "../hook_health_test.rs"]
mod hook_health_test;
#[path = "../hook_runtime_state_test.rs"]
mod hook_runtime_state_test;
#[path = "../hook_types_test.rs"]
mod hook_types_test;
#[path = "../hook_workflow_policy_test.rs"]
mod hook_workflow_policy_test;
