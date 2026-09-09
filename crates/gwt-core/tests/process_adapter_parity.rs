use std::{ffi::OsString, path::Path, process::Command};

use gwt_core::process::{resolved_command, resolved_tokio_command, ProcessPlanRequest};

fn assert_command_contract(command: &Command, program: &Path) {
    assert_eq!(command.get_program(), program);
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![OsString::from("--version").as_os_str()]
    );
    assert_eq!(
        command.get_current_dir(),
        Some(std::path::Path::new("workspace"))
    );

    let env = command
        .get_envs()
        .map(|(key, value)| (key.to_os_string(), value.map(OsString::from)))
        .collect::<Vec<_>>();
    assert!(env.contains(&(
        OsString::from("GWT_PROCESS_RESOLVER_TEST"),
        Some(OsString::from("preserved"))
    )));
    assert!(env.contains(&(OsString::from("GWT_PROCESS_REMOVE_TEST"), None)));
}

fn request(program: &Path) -> ProcessPlanRequest {
    ProcessPlanRequest::new(program)
        .arg("--version")
        .current_dir("workspace")
        .env("GWT_PROCESS_RESOLVER_TEST", "preserved")
        .env_remove("GWT_PROCESS_REMOVE_TEST")
}

#[test]
fn std_and_tokio_adapters_preserve_the_same_resolved_plan_contract() {
    let program = std::env::current_exe().expect("resolve current test executable");
    let std_command = resolved_command(request(&program)).expect("build std command");
    let tokio_command = resolved_tokio_command(request(&program)).expect("build tokio command");

    assert_command_contract(&std_command, &program);
    assert_command_contract(tokio_command.as_std(), &program);
}

#[cfg(windows)]
#[test]
fn std_adapter_executes_cmd_shim_metacharacters_as_literal_arguments() {
    use std::fs;

    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("Program Files").join("npm bin");
    fs::create_dir_all(&bin).expect("create cmd shim directory");
    fs::write(
        bin.join("runner.cmd"),
        "@echo off\r\necho GWT_ARG1:\"%~1\"\r\necho GWT_ARG2:\"%~2\"\r\necho GWT_ARG3:\"%~3\"\r\n",
    )
    .expect("write cmd shim");
    let comspec = std::env::var_os("ComSpec").expect("Windows ComSpec");
    let mut command = resolved_command(
        ProcessPlanRequest::new("runner")
            .arg("a&b")
            .arg("%PATH%")
            .arg("a!GWT_UNDEFINED!b")
            .inherit_env(false)
            .env("PATH", bin.as_os_str())
            .env("PATHEXT", ".CMD")
            .env("ComSpec", &comspec),
    )
    .expect("resolve cmd shim through ComSpec");

    let output = command.output().expect("spawn cmd shim");

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GWT_ARG1:\"a&b\""), "{output:?}");
    assert!(stdout.contains("GWT_ARG2:\"%PATH%\""), "{output:?}");
    assert!(
        stdout.contains("GWT_ARG3:\"a!GWT_UNDEFINED!b\""),
        "{output:?}"
    );
}
