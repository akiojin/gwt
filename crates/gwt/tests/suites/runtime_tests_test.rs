//! Issue #4135: runtime tests share one integration harness.

#[path = "../agent_process_resolution_contract_test.rs"]
mod agent_process_resolution_contract_test;
#[path = "../autostart_protocol_test.rs"]
mod autostart_protocol_test;
#[path = "../branch_cleanup_protocol_test.rs"]
mod branch_cleanup_protocol_test;
#[path = "../branch_cleanup_reconnect_test.rs"]
mod branch_cleanup_reconnect_test;
#[path = "../branch_cleanup_test.rs"]
mod branch_cleanup_test;
#[path = "../branch_list_test.rs"]
mod branch_list_test;
#[path = "../daemon_project_root_test.rs"]
mod daemon_project_root_test;
#[path = "../daemon_runtime_hook_test.rs"]
mod daemon_runtime_hook_test;
#[path = "../daemon_socket_path_limit_test.rs"]
mod daemon_socket_path_limit_test;
#[path = "../daemon_supervisor_test.rs"]
mod daemon_supervisor_test;
#[path = "../file_content_test.rs"]
mod file_content_test;
#[path = "../file_tree_test.rs"]
mod file_tree_test;
#[path = "../gui_single_instance_test.rs"]
mod gui_single_instance_test;
#[path = "../index_bootstrap_test.rs"]
mod index_bootstrap_test;
#[path = "../index_rebuild_cell_protocol_test.rs"]
mod index_rebuild_cell_protocol_test;
#[path = "../index_resource_harness_test.rs"]
mod index_resource_harness_test;
#[path = "../index_resources_test.rs"]
mod index_resources_test;
#[path = "../index_search_batch_test.rs"]
mod index_search_batch_test;
#[path = "../index_search_public_api_test.rs"]
mod index_search_public_api_test;
#[path = "../index_status_batch_test.rs"]
mod index_status_batch_test;
#[path = "../knowledge_bridge_test.rs"]
mod knowledge_bridge_test;
#[path = "../managed_assets_test.rs"]
mod managed_assets_test;
#[path = "../migration_e2e_test.rs"]
mod migration_e2e_test;
#[path = "../migration_single_branch_test.rs"]
mod migration_single_branch_test;
#[path = "../perf_regression_test.rs"]
mod perf_regression_test;
#[path = "../project_open_e2e_contract_test.rs"]
mod project_open_e2e_contract_test;
#[path = "../project_runtime_ownership_test.rs"]
mod project_runtime_ownership_test;
#[path = "../pty_start_gate_console_test.rs"]
mod pty_start_gate_console_test;
#[path = "../release_notes_protocol_test.rs"]
mod release_notes_protocol_test;
#[path = "../start_work_test.rs"]
mod start_work_test;
#[path = "../startup_failure_observability.rs"]
mod startup_failure_observability;
#[path = "../window_controls_protocol_test.rs"]
mod window_controls_protocol_test;
#[path = "../windows_agent_launch_smoke_contract.rs"]
mod windows_agent_launch_smoke_contract;
#[path = "../worktree_inventory_test.rs"]
mod worktree_inventory_test;
