//! Issue #4135: ci contracts share one integration harness.

#[path = "../bin_gwt_home_isolation_contract_test.rs"]
mod bin_gwt_home_isolation_contract_test;
#[path = "../ci_apt_lock_contract_test.rs"]
mod ci_apt_lock_contract_test;
#[path = "../ci_cache_warm_contract_test.rs"]
mod ci_cache_warm_contract_test;
#[path = "../ci_concurrency_contract_test.rs"]
mod ci_concurrency_contract_test;
#[path = "../ci_flake_hygiene_contract_test.rs"]
mod ci_flake_hygiene_contract_test;
#[path = "../ci_linux_dependency_install_contract_test.rs"]
mod ci_linux_dependency_install_contract_test;
#[path = "../ci_pre_pr_contract_test.rs"]
mod ci_pre_pr_contract_test;
#[path = "../ci_test_budget_contract_test.rs"]
mod ci_test_budget_contract_test;
#[path = "../frontend_e2e_workflow_contract_test.rs"]
mod frontend_e2e_workflow_contract_test;
#[path = "../native_project_picker_contract_test.rs"]
mod native_project_picker_contract_test;
#[path = "../release_workflow_contract_test.rs"]
mod release_workflow_contract_test;
#[path = "../removal_guards.rs"]
mod removal_guards;
#[path = "../tray_module_present_test.rs"]
mod tray_module_present_test;
#[path = "../windows_binary_subsystem_test.rs"]
mod windows_binary_subsystem_test;
