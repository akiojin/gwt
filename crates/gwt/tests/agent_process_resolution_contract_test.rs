use std::{fs, path::Path, path::PathBuf};

struct ProbeSite {
    relative_path: &'static str,
    function_name: &'static str,
}

const AGENT_PROBE_SITES: &[ProbeSite] = &[
    ProbeSite {
        relative_path: "crates/gwt-agent/src/claude_capabilities.rs",
        // `detect_claude_version_raw` delegates here; this is the function
        // that spawns the probe (Issue #3895).
        function_name: "detect_claude_version_raw_in_env",
    },
    ProbeSite {
        relative_path: "crates/gwt-core/src/usage/claude.rs",
        function_name: "claude_user_agent",
    },
    ProbeSite {
        relative_path: "crates/gwt-agent/src/detect.rs",
        // `fetch_version` delegates here so profile overrides and removals
        // use the same resolved adapter as the default environment.
        function_name: "fetch_version_with_environment",
    },
    ProbeSite {
        relative_path: "crates/gwt-agent/src/prepare.rs",
        function_name: "probe_host_runner_bounded_with_hub",
    },
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

#[test]
fn codex_hook_discovery_reuses_the_single_canonical_host_health_result() {
    let path = repo_root().join("crates/gwt/src/app_runtime/launch.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let policy = function_source(&source, "codex_hook_discovery_mode_for_launch_config");
    for forbidden in [
        "resolved_command(",
        "std::process::Command::new(",
        "tokio::process::Command::new(",
        "TokioCommand::new(",
    ] {
        assert!(
            !policy.contains(forbidden),
            "Codex hook discovery policy must stay process-free: {forbidden}"
        );
    }
    assert!(policy.contains("health_report"));
    assert!(policy.contains("version_output"));

    let launch = function_source(&source, "spawn_agent_window_async_with_claim");
    assert_eq!(
        launch
            .matches("resolve_host_runner_health_checked(")
            .count(),
        1,
        "Host launch must perform exactly one canonical runner-health check"
    );
    assert!(launch.contains(
        "codex_hook_discovery_mode_for_launch_config(&config, runner_health_report.as_ref())"
    ));
    let profile_env = launch
        .find(".apply_to_config(")
        .expect("profile env applied");
    let health = launch
        .find("resolve_host_runner_health_checked(")
        .expect("canonical host health");
    assert!(
        profile_env < health,
        "health must use the effective profile env"
    );
    for mutation in [
        "refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(",
        "maybe_register_codex_managed_hook_trust_for_launch(",
        "initialize_launch_session(",
    ] {
        assert!(
            health
                < launch
                    .find(mutation)
                    .unwrap_or_else(|| panic!("missing {mutation}")),
            "canonical health must precede launch mutation {mutation}"
        );
    }
    let initializer = fs::read_to_string(repo_root().join("crates/gwt/src/session_launch.rs"))
        .expect("read extracted Session initializer");
    assert!(function_source(&initializer, "initialize_launch_session").contains("Session::new("));
}

fn function_source<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("fn {name}(");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("missing function {name}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("missing body for {name}"));
    let mut depth = 0_u32;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated body for {name}");
}

#[test]
fn function_source_requires_an_exact_function_name() {
    let source = "fn probe_helper() { bypass(); }\nfn probe() { resolved_command(); }";

    assert_eq!(
        function_source(source, "probe"),
        "fn probe() { resolved_command(); }"
    );
}

fn production_web_sources(root: &Path) -> Vec<(PathBuf, String)> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        {
            let entry = entry.expect("read web source entry");
            let path = entry.path();
            if path.is_dir() {
                if !matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("__tests__" | "node_modules" | "dist" | "build")
                ) {
                    pending.push(path);
                }
                continue;
            }
            if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("js" | "mjs")
            ) && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".test."))
            {
                let source = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
                sources.push((path, source));
            }
        }
    }
    sources
}

#[test]
fn production_web_sources_excludes_test_dependency_and_build_directories() {
    let temp = tempfile::tempdir().expect("web source fixture");
    for directory in ["__tests__", "node_modules", "dist", "build"] {
        let path = temp.path().join(directory);
        fs::create_dir_all(&path).expect("create excluded web directory");
        fs::write(
            path.join("forbidden.js"),
            "sendEvent('open_active_work_launch_wizard');",
        )
        .expect("write excluded web source");
    }
    fs::write(
        temp.path().join("production.js"),
        "export const ready = true;",
    )
    .expect("write production web source");

    let sources = production_web_sources(temp.path());
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].0.file_name().and_then(|name| name.to_str()),
        Some("production.js")
    );
}

#[test]
fn builtin_package_runner_resolution_stays_absent() {
    let source = fs::read_to_string(repo_root().join("crates/gwt-agent/src/launch.rs"))
        .expect("read launch implementation");
    assert!(!source.contains("fn package_runner_candidates_for_agent("));
    assert!(!source.contains("fn resolve_host_npx_fallback_executable("));
}

#[test]
fn dormant_and_legacy_launch_surfaces_cannot_send_targeted_launch_events() {
    let web_root = repo_root().join("crates/gwt/web");
    let sources = production_web_sources(&web_root);
    let forbidden = [
        "\"open_active_work_launch_wizard\"",
        "'open_active_work_launch_wizard'",
        "\"resume_workspace\"",
        "'resume_workspace'",
        "\"open_start_work_in_agent_kanban\"",
        "'open_start_work_in_agent_kanban'",
        "\"apply_quick_start\"",
        "'apply_quick_start'",
        "\"select_quick_start\"",
        "'select_quick_start'",
    ];

    for (path, source) in sources {
        for event in forbidden {
            assert!(
                !source.contains(event),
                "dormant/legacy launch event {event} must not gain a production sender in {}",
                path.display()
            );
        }
    }

    let state_path = repo_root().join("crates/gwt/src/launch_wizard/state.rs");
    let state = fs::read_to_string(&state_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", state_path.display()));
    let production_state = state.split("#[cfg(test)]").next().unwrap_or(&state);
    assert_eq!(
        production_state
            .matches("open_existing_branch_with_previous_profiles")
            .count(),
        1,
        "the dormant existing-branch opener must have no production caller"
    );
}

#[test]
fn unmerged_codex_app_server_bridge_stays_absent() {
    let src_root = repo_root().join("crates/gwt/src");
    for relative in [
        "codex_app_server.rs",
        "app_runtime/codex_app_server.rs",
        "app_runtime/recovery_bridge.rs",
    ] {
        assert!(
            !src_root.join(relative).exists(),
            "Phase 75 must not import the unmerged app-server bridge: {relative}"
        );
    }

    for path in [
        src_root.join("lib.rs"),
        src_root.join("main.rs"),
        src_root.join("app_runtime/mod.rs"),
    ] {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for symbol in [
            "mod codex_app_server",
            "CodexAppServer",
            "spawn_codex_app_server",
            "start_codex_app_server",
        ] {
            assert!(
                !source.contains(symbol),
                "Phase 75 must not wire the unmerged app-server bridge via {symbol} in {}",
                path.display()
            );
        }
    }
}

#[test]
fn every_reachable_app_route_enters_the_shared_agent_launch_transaction() {
    let root = repo_root();
    for relative in [
        "crates/gwt/src/app_runtime/board.rs",
        "crates/gwt/src/app_runtime/continuation.rs",
        "crates/gwt/src/app_runtime/startup.rs",
        "crates/gwt/src/app_runtime/wizard.rs",
    ] {
        let path = root.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            source.contains("spawn_agent_window") || source.contains("spawn_continue_work_window"),
            "reachable route owner must enter an AppRuntime launch wrapper: {relative}"
        );
        for bypass in [
            "prepare_agent_launch(",
            "resolve_host_runner_health_checked(",
            "PtyHandle::spawn(",
            "spawn_agent_window_async(",
        ] {
            assert!(
                !source.contains(bypass),
                "reachable route owner must not bypass the shared transaction via {bypass}: {relative}"
            );
        }
    }

    let launch_path = root.join("crates/gwt/src/app_runtime/launch.rs");
    let launch = fs::read_to_string(&launch_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", launch_path.display()));
    for wrapper in [
        "spawn_agent_window",
        "spawn_agent_window_with_feedback",
        "spawn_agent_window_with_feedback_at_geometry",
        "spawn_agent_window_in_agent_kanban",
        "spawn_agent_window_at_geometry",
        "spawn_continue_work_window",
    ] {
        assert!(
            function_source(&launch, wrapper).contains("spawn_agent_window_with_placement("),
            "{wrapper} must converge on the shared placement transaction"
        );
    }
    let placement = function_source(&launch, "spawn_agent_window_with_placement");
    let claim = placement
        .find("claim_prepared_manual_successor_launch(")
        .expect("prepared manual successor is claimed before pane creation");
    let pane = placement
        .find(".add_window_with_title(")
        .expect("shared placement transaction creates the pane");
    assert!(
        claim < pane,
        "durable successor claim must precede pane creation"
    );
    assert!(placement.contains("Self::spawn_agent_window_async_with_claim("));
    let asynchronous = function_source(&launch, "spawn_agent_window_async_with_claim");
    for boundary in [
        "resolve_host_runner_health_checked(&mut config)",
        "apply_windows_host_shell_wrapper(&mut config)",
    ] {
        assert!(
            asynchronous.contains(boundary),
            "shared asynchronous launch transaction must own {boundary}"
        );
    }
}

#[test]
fn agent_probes_use_the_shared_resolved_process_adapter() {
    let root = repo_root();
    let forbidden = [
        "hidden_command(",
        "std::process::Command::new(",
        "tokio::process::Command::new(",
        "TokioCommand::new(",
    ];

    for site in AGENT_PROBE_SITES {
        let path = root.join(site.relative_path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let function = function_source(&source, site.function_name);
        for pattern in forbidden {
            assert!(
                !function.contains(pattern),
                "{} must not bypass the shared resolver with {pattern}",
                site.function_name
            );
        }
        assert!(
            function.contains("resolved_command("),
            "{} must consume the shared resolved process adapter",
            site.function_name
        );
    }
}

#[test]
fn agent_detector_reuses_the_version_probe_resolution() {
    let path = repo_root().join("crates/gwt-agent/src/detect.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

    for function_name in ["detect_by_command", "detect_one"] {
        let function = function_source(&source, function_name);
        assert!(
            !function.contains("resolve_process_plan("),
            "{function_name} must reuse the program resolved by fetch_version"
        );
    }
}

#[test]
fn windows_ci_runs_the_real_resolver_pty_and_caller_regression_targets() {
    let workflow_path = repo_root().join(".github/workflows/test.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    assert!(!workflow.contains("cargo test -p gwt-core terminal::pty"));
    for command in [
        "node scripts/ci-windows-tests.mjs run gwt-core test windows_process_resolver",
        "node scripts/ci-windows-tests.mjs run gwt-core test process_adapter_parity",
        "node scripts/ci-windows-tests.mjs run gwt-core test windows_claude_user_agent",
        "node scripts/ci-windows-tests.mjs run gwt-agent lib gwt_agent real_bun_global_placeholder_fixture",
        "node scripts/ci-windows-tests.mjs run gwt-agent lib gwt_agent direct_runner_resolution_failure_still_emits_an_end_summary",
        "node scripts/ci-windows-tests.mjs run gwt bin gwt real_bun_global_placeholder_fixture",
        "node scripts/ci-windows-tests.mjs run gwt bin gwt command_prompt_agent_wrapper",
        "node scripts/ci-windows-tests.mjs run gwt-terminal lib gwt_terminal pty::windows_spawn::tests",
        "node scripts/ci-windows-tests.mjs run gwt test runtime_tests agent_process_resolution_contract_test::",
    ] {
        assert!(
            workflow.contains(command),
            "Windows CI must run `{command}`"
        );
    }

    assert!(workflow.contains("test-windows-agent-launch-e2e:"));
    let shard_job = workflow
        .split("  test-windows-agent-launch-e2e:")
        .nth(1)
        .and_then(|tail| tail.split("\n  test-frontend:").next())
        .expect("Windows agent launch E2E job body");
    assert!(shard_job.contains("GWT_WINDOWS_AGENT_E2E_CREDENTIAL_FREE: \"1\""));
    assert!(shard_job.contains(
        "cargo test -p gwt --test windows_agent_launch_e2e -- --ignored --test-threads=1"
    ));
    for secret in ["secrets.", "CODEX_API_KEY", "ANTHROPIC_API_KEY"] {
        assert!(
            !shard_job.contains(secret),
            "ordinary deterministic Windows CI must stay credential-free: {secret}"
        );
    }
    assert!(
        shard_job.contains("cargo test -p gwt --test windows_agent_launch_e2e --no-run"),
        "installed provider cases must share one build step"
    );
    assert!(shard_job.contains("for provider in codex claude"));
    assert!(
        shard_job.contains("GWT_WINDOWS_AGENT_PROVIDER=\"$provider\""),
        "each iteration must select its installed provider"
    );
    assert!(!shard_job.contains("GWT_WINDOWS_AGENT_SELECTOR"));
    assert!(
        shard_job.contains("::error::") && shard_job.contains("status=1"),
        "one job must keep the matrix's per-shard attribution and its \
         fail-fast: false behaviour"
    );
}

#[test]
fn windows_ci_runs_issue_monitor_launch_now_control_path_contracts() {
    let workflow_path = repo_root().join(".github/workflows/test.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    let windows_job = workflow
        .split("\n  test-windows-rust:")
        .nth(1)
        .and_then(|tail| tail.split("\n  test-windows-agent-launch-e2e:").next())
        .expect("test-windows-rust job body");
    assert!(
        windows_job
            .lines()
            .any(|line| line.trim() == "runs-on: windows-latest"),
        "launch_now platform contracts must execute on a native Windows runner"
    );
    let marker = "      - name: Run Issue Monitor launch_now control-path contracts";
    let (_, tail) = windows_job
        .split_once(marker)
        .unwrap_or_else(|| panic!("missing Windows CI step `{marker}`"));
    let step = tail
        .split_once("\n      - name:")
        .map_or(tail, |(step, _)| step);
    assert!(
        step.contains("\n        shell: bash\n"),
        "the multi-command Windows control-path step must fail fast"
    );
    assert!(
        !step.contains("--ignored"),
        "the Windows launch_now contracts must run in ordinary CI"
    );
    let command_lines = step.lines().map(str::trim).collect::<Vec<_>>();
    for command in [
        "node scripts/ci-windows-tests.mjs run gwt lib gwt cli::issue::tests::windows_launch_now_ -- --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt lib gwt cli::issue::tests::immediate_scan_delivery_never_claims_an_unacknowledged_schedule -- --exact --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt lib gwt cli::pane::tests::pane_websocket_request_carries_the_agent_capability_in_authorization -- --exact --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt lib gwt cli::pane::tests::issue_monitor_scan_client_ -- --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt bin gwt embedded_server::tests::authenticated_monitor_scan_routes_scope_guard_and_result_only_to_origin_socket -- --exact --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt bin gwt authenticated_pm_scan_now_ -- --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt bin gwt authenticated_scan_now_ -- --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt test coordination_tests issue_monitor_protocol_test::frontend_issue_monitor_events_use_snake_case_wire_shape -- --exact --test-threads=1",
        "node scripts/ci-windows-tests.mjs run gwt test coordination_tests issue_monitor_protocol_test::agent_issue_monitor_scan_ -- --test-threads=1",
    ] {
        assert!(
            command_lines.contains(&command),
            "Windows CI must exercise launch_now control-path command `{command}`"
        );
    }
}

#[test]
fn windows_multi_command_test_steps_use_a_fail_fast_shell() {
    let workflow_path = repo_root().join(".github/workflows/test.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    for step_name in [
        "Run real Claude probe caller regressions",
        "Run Issue Monitor scheduled driver contracts",
    ] {
        let marker = format!("      - name: {step_name}");
        let (_, tail) = workflow
            .split_once(&marker)
            .unwrap_or_else(|| panic!("missing Windows CI step `{step_name}`"));
        let step = tail
            .split_once("\n      - name:")
            .map_or(tail, |(step, _)| step);
        assert!(
            step.contains("\n        shell: bash\n"),
            "Windows multi-command step `{step_name}` must stop at the first failed test command"
        );
        if step_name == "Run Issue Monitor scheduled driver contracts" {
            assert!(
                step.lines().any(|line| line.trim() == "node scripts/ci-windows-tests.mjs run gwt bin gwt app_runtime::tests::issue_monitor_claim_tests::app_runtime_full_issue_monitor_scan_migrates_legacy_git_failure_and_persists_marker -- --exact --test-threads=1"),
                "Windows CI must exercise the non-Unix initial Auto-capacity admission regression"
            );
        }
    }
}
