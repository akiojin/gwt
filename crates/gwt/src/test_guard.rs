//! Test-binary arming of the unsandboxed-`gh` spawn guard (Issue #3675).
//!
//! Unit tests in this crate must never reach the real GitHub API: one test
//! that does can burn shared quota, and — once the quota is exhausted — a
//! single real rate-limited `gh` call poisons the process-global
//! [`gwt_core::github_quota`] gate, which then refuses every later `gh`
//! spawn in the same test process, fake or not. The `#[ctor]` below arms the
//! guard before any test runs so an unsandboxed `gh` spawn fails that test
//! explicitly instead of silently leaving the process.

// Armed for the lib test binary (cfg(test)) and, through the `test-gh-guard`
// self dev-dependency feature, for every other test target that links this
// lib — bin unit tests and integration-test binaries alike. The extra
// debug_assertions gate keeps a release build unarmed even when the feature
// leaks in through --all-features.
// SAFETY(pre-main): only stores a relaxed AtomicBool; no allocation, no std
// services, no other statics touched.
#[cfg(any(test, all(feature = "test-gh-guard", debug_assertions)))]
#[ctor::ctor(unsafe)]
fn forbid_real_gh_in_tests() {
    gwt_core::process_console::forbid_unsandboxed_gh_spawns_for_tests();
}

#[cfg(test)]
mod tests {
    use gwt_core::process_console::{
        spawn_logged_blocking, ProcessConsoleHub, ProcessKind, SpawnOptions,
        REAL_GH_BLOCKED_ERROR_CODE,
    };
    use gwt_core::test_support::ScopedEnvVar;

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
}
