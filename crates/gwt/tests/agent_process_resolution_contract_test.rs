use std::{fs, path::PathBuf};

struct ProbeSite {
    relative_path: &'static str,
    function_name: &'static str,
}

const AGENT_PROBE_SITES: &[ProbeSite] = &[
    ProbeSite {
        relative_path: "crates/gwt-agent/src/claude_capabilities.rs",
        function_name: "detect_claude_version_raw",
    },
    ProbeSite {
        relative_path: "crates/gwt-core/src/usage/claude.rs",
        function_name: "claude_user_agent",
    },
    ProbeSite {
        relative_path: "crates/gwt-agent/src/detect.rs",
        function_name: "fetch_version",
    },
    ProbeSite {
        relative_path: "crates/gwt/src/app_runtime/launch.rs",
        function_name: "detect_installed_codex_hook_discovery_mode",
    },
    ProbeSite {
        relative_path: "crates/gwt-agent/src/prepare.rs",
        function_name: "probe_host_package_runner",
    },
    ProbeSite {
        relative_path: "crates/gwt/src/launch_runtime.rs",
        function_name: "probe_host_package_runner_with_timeout_and_hub",
    },
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn function_source<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("fn {name}");
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
fn agent_and_package_runner_probes_use_the_shared_resolved_process_adapter() {
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
fn windows_ci_runs_the_real_resolver_pty_and_caller_regression_targets() {
    let workflow_path = repo_root().join(".github/workflows/test.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    assert!(!workflow.contains("cargo test -p gwt-core terminal::pty"));
    for command in [
        "cargo test -p gwt-core --test windows_process_resolver --test process_adapter_parity",
        "cargo test -p gwt-core --lib real_bun_global_placeholder_fixture",
        "cargo test -p gwt-agent --lib real_bun_global_placeholder_fixture",
        "cargo test -p gwt-agent --lib package_runner_resolution_failure_still_emits_an_end_summary",
        "cargo test -p gwt --bin gwt real_bun_global_placeholder_fixture",
        "cargo test -p gwt-terminal --lib pty::windows_spawn::tests",
        "cargo test -p gwt --test agent_process_resolution_contract_test",
    ] {
        assert!(
            workflow.contains(command),
            "Windows CI must run `{command}`"
        );
    }
}
