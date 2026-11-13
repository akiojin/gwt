//! Standalone real-consumer probe for Issue #6474. Compile against the checkout's
//! gwt-agent/gwt-terminal libraries (see run_windows_cmd_runner_probe.py).
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

use gwt_agent::{
    prepare::probe_host_runner_with_timeout, resolve_host_runner_health_checked_with_probe,
    AgentId, AgentLaunchBuilder,
};
use gwt_terminal::pty::{PtyHandle, SpawnConfig};

fn main() {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("fixture root"));
    fs::create_dir_all(root.join("日本語")).unwrap();
    let shim = root.join("fake claude.cmd");
    fs::write(
        &shim,
        b"@echo off\r\nif not \"%~1\"==\"--version\" exit /b 2\r\nif not \"%~2\"==\"\" exit /b 3\r\nif not \"%PATHEXT%\"==\".CMD\" exit /b 4\r\necho 1.2.3\r\nexit /b 0\r\n",
    )
    .unwrap();
    let identity = fs::canonicalize(&shim).unwrap();
    let unicode = root.join("日本語").join("..").join("fake claude.cmd");
    assert_eq!(fs::canonicalize(&unicode).unwrap(), identity);
    let env = HashMap::from([
        ("PATH".into(), root.to_string_lossy().into_owned()),
        ("PATHEXT".into(), ".CMD".into()),
        ("ComSpec".into(), std::env::var("ComSpec").unwrap()),
    ]);
    let cases = [
        ("bare", "fake claude".into(), root.clone(), root.clone()),
        ("absolute", shim.clone(), root.clone(), root.clone()),
        ("unicode", unicode, root.clone(), root.clone()),
        (
            "verbatim-cwd",
            shim.clone(),
            identity.parent().unwrap().into(),
            root.clone(),
        ),
        (
            "verbatim-command",
            identity.clone(),
            root.clone(),
            root.clone(),
        ),
        (
            "verbatim-path",
            "fake claude".into(),
            root.clone(),
            identity.parent().unwrap().into(),
        ),
    ];
    let mut failed = Vec::new();
    for (name, command, cwd, path) in cases {
        eprintln!("case={name} start");
        let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
        config.command = command.to_string_lossy().into_owned();
        config.args = vec!["--version".into()];
        config.env_vars = env.clone();
        config
            .env_vars
            .insert("PATH".into(), path.to_string_lossy().into_owned());
        config.working_dir = Some(cwd);
        let mut health_exit = None;
        let health = resolve_host_runner_health_checked_with_probe(
            &mut config,
            |kind, executable, args, env, remove, cwd| {
                assert_eq!(fs::canonicalize(executable).unwrap(), identity);
                assert_eq!(args, ["--version"]);
                let result = probe_host_runner_with_timeout(
                    kind,
                    executable,
                    args,
                    env,
                    remove,
                    cwd,
                    Duration::from_secs(10),
                    Duration::from_millis(100),
                );
                health_exit = result.exit_code;
                assert!(!result.timed_out, "{name}: {result:?}");
                if result.success {
                    assert_eq!(result.stdout.trim(), "1.2.3");
                }
                result
            },
        );
        let health_ok = health
            .as_ref()
            .is_ok_and(|report| report.version_output.as_deref() == Some("1.2.3"));
        eprintln!("case={name} health={health_ok} exit={health_exit:?}");
        // Launch the selected runner through the actual ConPTY consumer, even
        // in RED mode, so both independent callers are measured on the same input.
        let pty = PtyHandle::spawn(SpawnConfig {
            command: config.command.clone(),
            args: config.args.clone(),
            cols: 120,
            rows: 24,
            env: config.env_vars.clone(),
            remove_env: config.remove_env.clone(),
            cwd: config.working_dir.clone(),
        })
        .expect("spawn real PTY consumer");
        // ConPTY waits for the terminal's cursor-position reply before running
        // a freshly attached console client (the production terminal supplies it).
        pty.write_input(b"\x1b[1;1R").unwrap();
        let mut reader = pty.reader().unwrap();
        let (tx, rx) = mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut output = String::new();
            let _ = reader.read_to_string(&mut output);
            let _ = tx.send(output);
        });
        // test-hygiene: allow-wall-clock-deadline Bound a real ConPTY subprocess; this is a failure watchdog, not a timing assertion.
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = pty.try_wait().unwrap() {
                break status;
            }
            assert!(
                // test-hygiene: allow-wall-clock-deadline Fail a hung real ConPTY process instead of blocking the acceptance probe indefinitely.
                Instant::now() < deadline,
                "{name}: ConPTY child did not exit"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        pty.release_descriptors();
        // test-hygiene: allow-wall-clock-deadline Bound reader teardown after the real child has exited and its console descriptors are released.
        let output = rx.recv_timeout(Duration::from_secs(15));
        if output.is_err() {
            panic!("{name}: PTY reader did not finish");
        }
        reading.join().unwrap();
        let output = output.expect("PTY output must finish within deadline");
        let launch_ok = status.success() && output.contains("1.2.3");
        println!("case={name} health={health_ok} exit={health_exit:?} launch={launch_ok} canonical={identity:?} argv={:?} env={:?}", config.args, config.env_vars);
        if !health_ok || !launch_ok {
            failed.push(name);
        }
    }
    // A truly absent target still fails through the existing actionable path.
    let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
    config.command = root.join("missing.cmd").to_string_lossy().into_owned();
    config.env_vars = env;
    let missing = resolve_host_runner_health_checked_with_probe(
        &mut config,
        |kind, executable, args, env, remove, cwd| {
            let result = probe_host_runner_with_timeout(
                kind,
                executable,
                args,
                env,
                remove,
                cwd,
                Duration::from_secs(10),
                Duration::from_millis(100),
            );
            assert!(!result.success && result.exit_code.is_none());
            assert!(result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("resolution was rejected")));
            result
        },
    )
    .unwrap_err();
    assert!(
        missing.contains("installed runner failed its health check"),
        "{missing}"
    );
    println!("missing=diagnostic failed={failed:?}");
    let expected_red = std::env::args().any(|arg| arg == "--expect-red");
    if expected_red {
        assert_eq!(failed, ["verbatim-command", "verbatim-path"]);
    } else {
        assert!(failed.is_empty(), "{failed:?}");
    }
}
