use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

use gwt_core::workspace_projection::load_or_default_workspace_projection;

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
    assert!(stdout.contains("memory"));
    assert!(
        !stdout.contains("Launch `gwt` instead"),
        "gwtd help must not redirect agent-facing CLI users to the GUI front door"
    );
}

#[test]
fn gwtd_no_args_dispatches_stdin_json_envelope() {
    let project = tempfile::tempdir().expect("project tempdir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(project.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            br#"{
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "agent_session": "session-bin-json",
                    "purpose": "Binary JSON envelope",
                    "current_focus": "integration test"
                }
            }"#,
        )
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait gwtd");

    assert!(
        output.status.success(),
        "gwtd JSON envelope should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(r#""ok":true"#),
        "stdout should be success JSON, got: {stdout}"
    );
    let projection =
        load_or_default_workspace_projection(project.path()).expect("load workspace projection");
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-bin-json")
        .expect("agent upserted by gwtd JSON envelope");
    assert_eq!(agent.title_summary.as_deref(), Some("Binary JSON envelope"));
}

#[test]
fn gwtd_rejects_legacy_family_argv_invocations() {
    for args in [
        ["board", "show"].as_slice(),
        ["issue", "view", "1"].as_slice(),
        ["hook", "register-codex-managed-hook-trust"].as_slice(),
        ["index", "--help"].as_slice(),
        ["workspace", "update", "--title-summary", "legacy"].as_slice(),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_gwtd"))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run gwtd legacy argv");

        assert_eq!(
            output.status.code(),
            Some(2),
            "legacy argv must exit 2 for args {args:?}; stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stdin JSON envelope"),
            "stderr must point agents to JSON envelope for args {args:?}, got: {stderr}"
        );
    }
}

#[test]
fn gwtd_index_help_lists_every_rebuild_scope() {
    let output = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .args(["--help", "index"])
        .output()
        .expect("run gwtd --help index");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("all|issues|specs|memory|discussions|board|files|files-docs"),
        "index help must list every accepted rebuild scope, got: {stdout}"
    );
}

#[test]
fn gwtd_hook_register_codex_managed_hook_trust_writes_requested_config() {
    let project = tempfile::tempdir().expect("project tempdir");
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    let previous_hook_bin = std::env::var_os("GWT_HOOK_BIN");
    std::env::set_var("GWT_HOOK_BIN", env!("CARGO_BIN_EXE_gwtd"));
    gwt_skills::generate_codex_hooks(project.path()).expect("generate hooks");
    match previous_hook_bin {
        Some(value) => std::env::set_var("GWT_HOOK_BIN", value),
        None => std::env::remove_var("GWT_HOOK_BIN"),
    }

    let output = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd hook register");
    let mut child = output;
    write!(
        child.stdin.take().expect("stdin"),
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "operation": "hook.register_codex_managed_hook_trust",
            "params": {
                "project_root": project.path().to_str().expect("project path utf8"),
                "codex_config": config_path.to_str().expect("config path utf8"),
            }
        })
    )
    .expect("write JSON envelope");
    let output = child.wait_with_output().expect("wait gwtd hook register");

    assert!(
        output.status.success(),
        "registration should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("trusted 5"),
        "stdout should report trusted hook count, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let config = fs::read_to_string(&config_path).expect("read config");
    assert!(
        config.contains("trusted_hash"),
        "Codex config must receive trusted hashes, got: {config}"
    );
    assert_eq!(
        config.matches("enabled = true").count(),
        5,
        "Codex config must enable every trusted managed hook, got: {config}"
    );
}

#[test]
fn gwtd_managed_hook_event_remains_argv_transport_exception() {
    let output = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .args(["hook", "event", "SessionStart"])
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd hook event");

    assert!(
        output.status.success(),
        "managed hook transport should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("hookSpecificOutput"),
        "SessionStart should keep the managed hook stdout contract, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
