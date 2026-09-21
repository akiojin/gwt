//! Test-binary arming of the external-process guards (Issues #3675, #3972).
//!
//! Unit tests in this crate must never reach the real GitHub API: one test
//! that does can burn shared quota, and — once the quota is exhausted — a
//! single real rate-limited `gh` call poisons the process-global
//! [`gwt_core::github_quota`] gate, which then refuses every later `gh`
//! spawn in the same test process, fake or not. The `#[ctor]` below arms the
//! guard before any test runs so an unsandboxed `gh` spawn fails that test
//! explicitly instead of silently leaving the process.
//!
//! The same `#[ctor]` arms the package-runner probe guard. Agent launch falls
//! back to `npx`/`bunx` when the agent's own CLI is missing and probes that
//! runner under a five-second budget; on a loaded host the real spawn misses
//! the budget and aborts the launch, so an unrelated `app_runtime` test fails
//! with `npx package-runner probe timed out` (Issue #3972). Armed, the probe
//! is refused up front with an actionable message instead.

// Armed for the lib test binary (cfg(test)) and, through the `test-gh-guard`
// self dev-dependency feature, for every other test target that links this
// lib — bin unit tests and integration-test binaries alike. The extra
// debug_assertions gate keeps a release build unarmed even when the feature
// leaks in through --all-features.
// SAFETY(pre-main): only stores relaxed AtomicBools; no allocation, no std
// services, no other statics touched.
#[cfg(any(test, all(feature = "test-gh-guard", debug_assertions)))]
#[ctor::ctor(unsafe)]
fn forbid_real_external_processes_in_tests() {
    gwt_core::process_console::forbid_unsandboxed_gh_spawns_for_tests();
    gwt_core::process_console::forbid_real_package_runner_probes_for_tests();
}

#[cfg(test)]
mod tests {
    use gwt_core::process_console::{
        spawn_logged_blocking, ProcessConsoleHub, ProcessKind, SpawnOptions,
        ALLOW_REAL_RUNNER_PROBE_MARKER, REAL_GH_BLOCKED_ERROR_CODE,
        REAL_RUNNER_PROBE_BLOCKED_ERROR_CODE, RUNNER_PROBE_SANDBOX_MARKER,
    };
    use gwt_core::test_support::ScopedEnvVar;
    use std::collections::HashMap;
    use std::time::Duration;

    fn clear_markers() -> Vec<ScopedEnvVar> {
        [
            "GWT_TEST_GH_SANDBOX",
            "GWT_TEST_GH",
            "GWT_FAKE_GH_MODE",
            "GWT_ALLOW_REAL_GH",
        ]
        .into_iter()
        .map(ScopedEnvVar::unset)
        .collect()
    }

    fn clear_runner_markers() -> Vec<ScopedEnvVar> {
        [RUNNER_PROBE_SANDBOX_MARKER, ALLOW_REAL_RUNNER_PROBE_MARKER]
            .into_iter()
            .map(ScopedEnvVar::unset)
            .collect()
    }

    #[test]
    fn unsandboxed_gh_spawn_fails_the_test_explicitly() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _markers = clear_markers();

        let error = spawn_logged_blocking(
            &ProcessConsoleHub::new(),
            ProcessKind::Gh,
            "gh",
            &["--version"],
            SpawnOptions::new("gh --version guard probe"),
        )
        .expect_err("an unsandboxed gh spawn must be refused in test builds");

        assert!(
            error.to_string().contains(REAL_GH_BLOCKED_ERROR_CODE),
            "refusal must carry the machine-readable code: {error}"
        );
    }

    #[test]
    fn sandbox_marker_lets_gh_kind_spawns_proceed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _sandbox = ScopedEnvVar::set("GWT_TEST_GH_SANDBOX", "1");

        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), "echo sandboxed".to_string()])
        } else {
            ("sh", vec!["-c".to_string(), "echo sandboxed".to_string()])
        };
        let output = spawn_logged_blocking(
            &ProcessConsoleHub::new(),
            ProcessKind::Gh,
            program,
            &args,
            SpawnOptions::new("sandboxed gh-kind fixture"),
        )
        .expect("a sandbox-marked gh-kind spawn must proceed");

        assert!(output.success());
        assert!(output.stdout.contains("sandboxed"));
    }

    /// Issue #3972 AC-4: a test that reaches the host package runner is refused
    /// explicitly instead of spawning a real `npx` under a five-second budget.
    #[test]
    fn real_package_runner_probe_fails_the_test_explicitly() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _markers = clear_runner_markers();

        let outcome = gwt_agent::prepare::probe_host_runner_with_timeout(
            gwt_agent::HostRunnerProbeKind::Runner,
            "npx",
            vec!["--version".to_string()],
            &HashMap::new(),
            &[],
            None,
            Duration::from_secs(5),
            Duration::from_millis(50),
        );

        assert!(
            !outcome.success,
            "an unstubbed package-runner probe must be refused"
        );
        assert!(
            outcome
                .stderr
                .contains(REAL_RUNNER_PROBE_BLOCKED_ERROR_CODE),
            "refusal must carry the machine-readable code: {outcome:?}"
        );
        assert!(
            !outcome.timed_out,
            "a refusal must not look like the flake it prevents: {outcome:?}"
        );
    }

    /// The sandbox marker is honored from the launch environment, so a test can
    /// scope its opt-in to one `LaunchConfig` instead of mutating the
    /// process-global environment (Issue #3895).
    #[test]
    fn launch_environment_sandbox_marker_lets_the_probe_proceed() {
        let program = if cfg!(windows) { "cmd" } else { "sh" };
        let args = if cfg!(windows) {
            vec!["/C".to_string(), "echo 1.2.3".to_string()]
        } else {
            vec!["-c".to_string(), "echo 1.2.3".to_string()]
        };
        let env_vars = HashMap::from([(RUNNER_PROBE_SANDBOX_MARKER.to_string(), "1".to_string())]);

        let outcome = gwt_agent::prepare::probe_host_runner_with_timeout(
            gwt_agent::HostRunnerProbeKind::Runner,
            program,
            args,
            &env_vars,
            &[],
            None,
            Duration::from_secs(30),
            Duration::from_millis(50),
        );

        assert!(
            outcome.success,
            "a sandbox-marked package-runner probe must proceed: {outcome:?}"
        );
        assert!(outcome.stdout.contains("1.2.3"), "{outcome:?}");
    }
}
