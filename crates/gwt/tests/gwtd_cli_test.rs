use std::process::{Command, Stdio};

#[test]
fn gwtd_dispatches_internal_hook_cli_without_gui_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .args(["__internal", "daemon-hook", "forward"])
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd");

    assert!(
        output.status.success(),
        "gwtd internal hook should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "headless internal hook should not print GUI guidance, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn gwtd_help_describes_the_headless_cli_surface() {
    let output = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .arg("--help")
        .output()
        .expect("run gwtd --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gwtd"));
    assert!(stdout.contains("issue"));
    assert!(stdout.contains("pr"));
    assert!(stdout.contains("hook"));
    assert!(
        !stdout.contains("Launch `gwt` instead"),
        "gwtd help must not redirect agent-facing CLI users to the GUI front door"
    );
}
