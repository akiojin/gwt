//! Issue #4135: cli contracts share one integration harness.

#[path = "../cli_flag_parse_test.rs"]
mod cli_flag_parse_test;
#[path = "../cli_test.rs"]
mod cli_test;
#[path = "../discuss_cli_test.rs"]
mod discuss_cli_test;
#[path = "../discussion_cli_test.rs"]
mod discussion_cli_test;
#[path = "../gwtd_cli_test.rs"]
mod gwtd_cli_test;
#[path = "../gwtd_resolution_test.rs"]
mod gwtd_resolution_test;
#[path = "../memory_cli_test.rs"]
mod memory_cli_test;
#[path = "../operation_catalog_test.rs"]
mod operation_catalog_test;
#[path = "../resident_pm_workspace_cli_test.rs"]
mod resident_pm_workspace_cli_test;
#[path = "../skill_exit_cli_test.rs"]
mod skill_exit_cli_test;
#[path = "../spec_tasks_test.rs"]
mod spec_tasks_test;
#[path = "../verification_admission_cli_test.rs"]
mod verification_admission_cli_test;
#[path = "../verification_command_admission_test.rs"]
mod verification_command_admission_test;
#[path = "../verification_driver_cli_test.rs"]
mod verification_driver_cli_test;
#[path = "../verification_lease_cli_test.rs"]
mod verification_lease_cli_test;
#[path = "../workspace_cli_test.rs"]
mod workspace_cli_test;
