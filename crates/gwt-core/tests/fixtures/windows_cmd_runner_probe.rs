//! Standalone real-consumer probe; run with run_windows_cmd_runner_probe.py.
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

use gwt_agent::{
    prepare::probe_host_runner_with_timeout, resolve_host_runner_health_checked_with_probe,
    AgentId, AgentLaunchBuilder, HostRunnerProbeKind,
};
use gwt_core::paths::normalize_windows_child_process_path;
use gwt_terminal::{pty::SpawnConfig, PtyHandle};

fn assert_fixture_output(output: &str, expected: &Path) {
    let output = gwt_core::process_console::strip_ansi(output);
    assert!(output.contains("GWT_ARG:--version"), "{output}");
    let actual = output
        .lines()
        .find_map(|line| line.strip_prefix("GWT_FILE:"))
        .expect("executed fixture path");
    assert_eq!(
        fs::canonicalize(actual.trim()).unwrap(),
        fs::canonicalize(expected).unwrap()
    );
}

fn launch(command: &str, env: HashMap<String, String>, cwd: PathBuf, expected: &Path) -> bool {
    let pty = Arc::new(
        PtyHandle::spawn(SpawnConfig {
            command: command.to_owned(),
            args: vec!["--version".into()],
            cols: 100,
            rows: 24,
            env,
            remove_env: vec![],
            cwd: Some(cwd),
        })
        .expect("spawn real ConPTY consumer"),
    );
    let mut reader = pty.reader().expect("ConPTY reader");
    let (output_tx, output_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let result = reader.read_to_end(&mut output);
        let _ = output_tx.send((result, output));
    });
    let child = Arc::clone(&pty);
    let (exit_tx, exit_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = exit_tx.send(child.wait());
    });
    // test-hygiene: allow-wall-clock-deadline Watchdog for a real ConPTY child; no timing assertion.
    let status = exit_rx.recv_timeout(Duration::from_secs(30));
    if status.is_err() {
        let _ = pty.kill();
        panic!("ConPTY child exceeded its watchdog");
    }
    let success = status.unwrap().expect("ConPTY exit status").success();
    pty.release_descriptors();
    let (result, output) = output_rx
        // test-hygiene: allow-wall-clock-deadline Bound a real pipe drain after child exit.
        .recv_timeout(Duration::from_secs(30))
        .expect("ConPTY output drain");
    result.expect("read ConPTY output");
    if success {
        assert_fixture_output(&String::from_utf8_lossy(&output), expected);
    }
    success
}

fn main() {
    assert!(cfg!(windows), "this probe must execute on Windows");
    let mut arguments = std::env::args().skip(1);
    let root = PathBuf::from(arguments.next().expect("fixture directory"));
    let expect_red = arguments.next().as_deref() == Some("--expect-red");
    fs::create_dir_all(&root).expect("create fixture directory");
    let ordinary = normalize_windows_child_process_path(&fs::canonicalize(root).unwrap());
    let shim = ordinary.join("fake claude.cmd");
    let unicode = ordinary.join("実行.cmd");
    fs::write(
        &shim,
        "@echo off\r\n\"%SystemRoot%\\System32\\chcp.com\" 65001 >nul\r\nif not \"%~1\"==\"--version\" exit /b 9\r\necho Claude Code 2.1.154\r\necho GWT_ARG:%~1\r\necho GWT_FILE:%~f0\r\n",
    )
    .expect("write fixture");
    if unicode.exists() {
        fs::remove_file(&unicode).expect("replace previous fixture alias");
    }
    fs::hard_link(&shim, &unicode).expect("same physical file with Unicode name");
    let verbatim_shim = fs::canonicalize(&shim).unwrap();
    let verbatim_dir = fs::canonicalize(&ordinary).unwrap();
    let comspec = std::env::var("ComSpec").expect("Windows ComSpec");
    let mut failures = Vec::new();
    for (name, command, path, cwd, expected) in [
        (
            "bare",
            Path::new("fake claude"),
            &ordinary,
            &ordinary,
            &shim,
        ),
        ("absolute", shim.as_path(), &ordinary, &ordinary, &shim),
        ("unicode", unicode.as_path(), &ordinary, &ordinary, &unicode),
        (
            "verbatim-cwd",
            shim.as_path(),
            &ordinary,
            &verbatim_dir,
            &shim,
        ),
        (
            "verbatim-command",
            verbatim_shim.as_path(),
            &ordinary,
            &ordinary,
            &shim,
        ),
        (
            "verbatim-path",
            Path::new("fake claude"),
            &verbatim_dir,
            &ordinary,
            &shim,
        ),
    ] {
        let env = HashMap::from([
            ("PATH".to_owned(), path.display().to_string()),
            ("PATHEXT".to_owned(), ".CMD".to_owned()),
            ("ComSpec".to_owned(), comspec.clone()),
        ]);
        let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
        config.command = command.display().to_string();
        config.env_vars = env.clone();
        config.remove_env.clear();
        config.working_dir = Some(cwd.clone());
        let mut measured = None;
        let health = resolve_host_runner_health_checked_with_probe(
            &mut config,
            |kind, resolved, args, actual_env, remove, actual_cwd| {
                assert_eq!(kind, HostRunnerProbeKind::Direct);
                assert_eq!(args, ["--version"]);
                assert_eq!(
                    fs::canonicalize(resolved).unwrap(),
                    fs::canonicalize(expected).unwrap()
                );
                assert_eq!(actual_env, &env);
                assert!(remove.is_empty());
                assert_eq!(actual_cwd.as_deref(), Some(cwd.as_path()));
                // test-hygiene: allow-production-probe-deadline Exercise the real installed-runner consumer.
                let outcome = probe_host_runner_with_timeout(
                    kind,
                    resolved,
                    args,
                    actual_env,
                    remove,
                    actual_cwd,
                    Duration::from_secs(30),
                    Duration::from_millis(100),
                );
                measured = Some(outcome.clone());
                outcome
            },
        );
        let outcome = measured.expect("health invokes its real probe");
        assert!(!outcome.timed_out, "{name}: {outcome:?}");
        assert_eq!(health.is_ok(), outcome.success, "{name}");
        let launched = launch(
            &command.display().to_string(),
            env.clone(),
            cwd.clone(),
            expected,
        );
        assert_eq!(launched, outcome.success, "{name}: consumers disagree");
        if outcome.success {
            assert_eq!(outcome.exit_code, Some(0));
            // Health diagnostics redact inherited path values; identity is
            // asserted against the resolved command before that boundary.
            assert!(
                outcome.stdout.contains("GWT_ARG:--version"),
                "{name}: {outcome:?}"
            );
        } else {
            assert_eq!(outcome.exit_code, Some(1), "{name}: {outcome:?}");
            failures.push(name);
        }
        println!("case={name} health={} exit={:?} launch={launched} canonical={:?} argv=[\"--version\"] env={env:?}", outcome.success, outcome.exit_code, fs::canonicalize(expected).unwrap());
    }
    let mut missing = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
    missing.command = ordinary.join("missing.cmd").display().to_string();
    missing.env_vars = HashMap::from([("ComSpec".into(), comspec)]);
    let error = resolve_host_runner_health_checked_with_probe(
        &mut missing,
        |kind, cmd, args, env, remove, cwd| {
            // test-hygiene: allow-production-probe-deadline Preserve real missing-runner diagnostics.
            probe_host_runner_with_timeout(
                kind,
                cmd,
                args,
                env,
                remove,
                cwd,
                Duration::from_secs(30),
                Duration::from_millis(100),
            )
        },
    )
    .expect_err("missing target must fail");
    assert!(
        error.contains("installed runner failed its health check"),
        "{error}"
    );
    println!("missing=diagnostic failed={failures:?}");
    let expected: Vec<&str> = if expect_red {
        vec!["verbatim-command", "verbatim-path"]
    } else {
        vec![]
    };
    assert_eq!(failures, expected);
}
