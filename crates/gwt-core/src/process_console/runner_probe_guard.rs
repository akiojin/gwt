//! Test-only guard that refuses real host package-runner probes (Issue #3972).
//!
//! Agent launch validates the host runner before spawning a provider. When the
//! agent's own CLI is not installed, that validation rewrites the command to
//! `npx`/`bunx` and probes the runner itself (`npx --version`) under a fixed
//! five-second budget. In a test binary that spawns whatever `npx` happens to
//! be on the developer's or CI runner's `PATH` — a real, uncontrolled external
//! process. On a loaded host (the shared build machine, or CI running the whole
//! `--bin gwt` suite) the spawn does not answer inside the budget, the launch
//! aborts with `npx package-runner probe timed out for <pkg>@latest`, and a
//! test that was asserting Resume behavior fails for a reason that has nothing
//! to do with the behavior under test.
//!
//! Test binaries opt in by calling
//! [`forbid_real_package_runner_probes_for_tests`] before tests run (the gwt
//! crate does this from the same `#[ctor]` that arms the `gh` guard). Once
//! armed, a package-runner probe is refused unless a marker proves the scope
//! installed its own fake runner ([`RUNNER_PROBE_SANDBOX_MARKER`]) or
//! explicitly opted in to the real host runner
//! ([`ALLOW_REAL_RUNNER_PROBE_MARKER`]).
//!
//! The fix a refused test usually needs is neither marker: it is to put a fake
//! provider executable on the launch `PATH` so the *direct* runner probe
//! succeeds and the package-runner fallback is never reached at all.

use std::sync::atomic::{AtomicBool, Ordering};

/// Machine-readable prefix of the refusal error. Tests and humans can grep for
/// this instead of guessing from a generic probe failure.
pub const REAL_RUNNER_PROBE_BLOCKED_ERROR_CODE: &str = "real_package_runner_probe_blocked_in_tests";

/// Marker proving the scope installed its own fake `npx`/`bunx`. Honored both
/// in the launch environment the probe runs with and in the process
/// environment, so a test can scope it to one `LaunchConfig` instead of
/// mutating the process-global `PATH` (Issue #3895).
pub const RUNNER_PROBE_SANDBOX_MARKER: &str = "GWT_TEST_RUNNER_SANDBOX";

/// Explicit opt-in for tests that mean to exercise the real host runner.
pub const ALLOW_REAL_RUNNER_PROBE_MARKER: &str = "GWT_ALLOW_REAL_PACKAGE_RUNNER_PROBE";

static FORBID_REAL_PROBES: AtomicBool = AtomicBool::new(false);

/// Arm the guard for the rest of the process lifetime. Intended to be called
/// once at test-binary start; production binaries never call this.
pub fn forbid_real_package_runner_probes_for_tests() {
    FORBID_REAL_PROBES.store(true, Ordering::Relaxed);
}

/// Whether the guard is armed.
pub fn real_package_runner_probes_forbidden() -> bool {
    FORBID_REAL_PROBES.load(Ordering::Relaxed)
}

/// The refusal detail for `label` when the guard is armed and neither marker is
/// visible through `has_marker`, or `None` when the probe may proceed.
///
/// `has_marker` is supplied by the caller because the launch environment a
/// probe runs with is not the process environment: gwt-agent looks the markers
/// up in the probe's own `env_vars` first and only then falls back to the
/// process.
pub fn real_package_runner_probe_denial(
    label: &str,
    has_marker: impl Fn(&str) -> bool,
) -> Option<String> {
    if !real_package_runner_probes_forbidden() {
        return None;
    }
    denial_for_markers(label, has_marker)
}

/// Pure decision core: refuse unless one of the markers is present.
fn denial_for_markers(label: &str, has_marker: impl Fn(&str) -> bool) -> Option<String> {
    if has_marker(RUNNER_PROBE_SANDBOX_MARKER) || has_marker(ALLOW_REAL_RUNNER_PROBE_MARKER) {
        return None;
    }
    Some(format!(
        "{REAL_RUNNER_PROBE_BLOCKED_ERROR_CODE}: '{label}' would spawn the real host package \
         runner during a test, which fails under a five-second budget whenever the host is busy. \
         Put a fake provider executable on the launch PATH so the direct runner probe succeeds, \
         set {RUNNER_PROBE_SANDBOX_MARKER}=1 in the launch environment once a fake npx/bunx is \
         installed, or opt in to the real runner with {ALLOW_REAL_RUNNER_PROBE_MARKER}=1 \
         (Issue #3972)."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers<'a>(present: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |name| present.contains(&name)
    }

    #[test]
    fn denies_package_runner_probe_when_no_marker_is_present() {
        let detail = denial_for_markers("npx --version (@xai-official/grok@latest)", markers(&[]))
            .expect("an unstubbed package-runner probe is denied");
        assert!(
            detail.contains(REAL_RUNNER_PROBE_BLOCKED_ERROR_CODE),
            "detail must lead with the machine-readable code: {detail}"
        );
        assert!(
            detail.contains("npx --version (@xai-official/grok@latest)"),
            "detail must name the refused probe: {detail}"
        );
    }

    #[test]
    fn denial_names_the_direct_path_fix_first() {
        let detail = denial_for_markers("npx --version (@openai/codex@latest)", markers(&[]))
            .expect("an unstubbed package-runner probe is denied");
        let path_fix = detail
            .find("fake provider executable on the launch PATH")
            .expect("the PATH fix must be offered");
        let marker_fix = detail
            .find(RUNNER_PROBE_SANDBOX_MARKER)
            .expect("the sandbox marker must be named");
        assert!(
            path_fix < marker_fix,
            "the actionable fix is the PATH fake, not a marker: {detail}"
        );
    }

    #[test]
    fn sandbox_marker_allows_the_probe() {
        assert_eq!(
            denial_for_markers("npx --version", markers(&[RUNNER_PROBE_SANDBOX_MARKER])),
            None,
            "an installed fake runner must allow the probe"
        );
    }

    #[test]
    fn real_runner_opt_in_allows_the_probe() {
        assert_eq!(
            denial_for_markers("npx --version", markers(&[ALLOW_REAL_RUNNER_PROBE_MARKER])),
            None
        );
    }

    #[test]
    fn disarmed_guard_allows_everything() {
        // The global flag defaults to off; production binaries never arm it.
        // Only the pure core is exercised here — arming the process-global flag
        // inside gwt-core's parallel test binary would refuse the legitimate
        // probe fixtures in sibling tests.
        assert_eq!(
            real_package_runner_probe_denial("npx --version", markers(&[])),
            None
        );
    }
}
