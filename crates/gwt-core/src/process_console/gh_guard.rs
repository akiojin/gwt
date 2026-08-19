//! Test-only guard that refuses unsandboxed `gh` spawns (Issue #3675).
//!
//! Every GitHub API access in gwt funnels through
//! [`super::spawn::spawn_logged`] with [`super::ProcessKind::Gh`] — including
//! the `gh auth token` spawn that gwt-github's HTTP client needs before it can
//! issue any direct request. That single choke point is where this guard
//! plugs in.
//!
//! Test binaries opt in by calling
//! [`forbid_unsandboxed_gh_spawns_for_tests`] before tests run (the gwt crate
//! does this from a `#[ctor]` in its test builds). Once armed, a
//! `ProcessKind::Gh` spawn is refused unless the process environment proves a
//! fake `gh` is installed (`GWT_TEST_GH_SANDBOX`, `GWT_TEST_GH`,
//! `GWT_FAKE_GH_MODE`) or the caller explicitly opted in to the live API
//! (`GWT_ALLOW_REAL_GH`). The refusal message deliberately matches none of the
//! [`crate::github_quota`] rate-limit markers, so a refused spawn can never
//! poison the process-global quota gate the way a real rate-limited call does.

use std::sync::atomic::{AtomicBool, Ordering};

/// Machine-readable prefix of the refusal error. Tests and humans can grep
/// for this instead of guessing from a generic spawn failure.
pub const REAL_GH_BLOCKED_ERROR_CODE: &str = "real_gh_spawn_blocked_in_tests";

/// Environment markers that prove a fake `gh` (or an explicit redirect) is
/// installed for the current test scope.
const SANDBOX_MARKERS: &[&str] = &["GWT_TEST_GH_SANDBOX", "GWT_TEST_GH", "GWT_FAKE_GH_MODE"];

/// Explicit opt-in for live-API smoke tests run on purpose.
const LIVE_OPT_IN_MARKER: &str = "GWT_ALLOW_REAL_GH";

static FORBID_UNSANDBOXED: AtomicBool = AtomicBool::new(false);

/// Arm the guard for the rest of the process lifetime. Intended to be called
/// once at test-binary start; production binaries never call this.
pub fn forbid_unsandboxed_gh_spawns_for_tests() {
    FORBID_UNSANDBOXED.store(true, Ordering::Relaxed);
}

/// The refusal detail for `label` when the guard is armed and no sandbox or
/// live-opt-in marker is present, or `None` when the spawn may proceed.
///
/// Public because a few modules spawn `gh` directly instead of through
/// [`super::spawn::spawn_logged`] (e.g. the Issue cache, which manages the
/// quota gate itself); those call sites must apply the same guard.
pub fn unsandboxed_gh_denial(label: &str) -> Option<String> {
    if !FORBID_UNSANDBOXED.load(Ordering::Relaxed) {
        return None;
    }
    denial_for_env(label, |name| std::env::var_os(name).is_some())
}

/// Pure decision core: refuse unless one of the markers is present.
fn denial_for_env(label: &str, has_env: impl Fn(&str) -> bool) -> Option<String> {
    if SANDBOX_MARKERS.iter().any(|marker| has_env(marker)) || has_env(LIVE_OPT_IN_MARKER) {
        return None;
    }
    Some(format!(
        "{REAL_GH_BLOCKED_ERROR_CODE}: '{label}' would reach the real GitHub API from a test. \
         Stub gh first (GWT_TEST_GH_SANDBOX=1 with a PATH fake, GWT_TEST_GH=<path>, or \
         GWT_FAKE_GH_MODE), or opt in to the live API with GWT_ALLOW_REAL_GH=1 (Issue #3675)."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with<'a>(present: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |name| present.contains(&name)
    }

    #[test]
    fn denies_gh_spawn_when_no_marker_is_present() {
        let detail =
            denial_for_env("gh pr view 12", env_with(&[])).expect("unsandboxed spawn is denied");
        assert!(
            detail.contains(REAL_GH_BLOCKED_ERROR_CODE),
            "detail must lead with the machine-readable code: {detail}"
        );
        assert!(
            detail.contains("gh pr view 12"),
            "detail must name the refused call: {detail}"
        );
    }

    #[test]
    fn denial_message_never_matches_rate_limit_markers() {
        let detail =
            denial_for_env("gh api graphql", env_with(&[])).expect("unsandboxed spawn is denied");
        assert!(
            !crate::github_quota::is_rate_limit_stderr(&detail),
            "a refusal must not be classifiable as a rate limit, or it would \
             poison the process-global quota gate: {detail}"
        );
    }

    #[test]
    fn each_sandbox_marker_allows_the_spawn() {
        for marker in ["GWT_TEST_GH_SANDBOX", "GWT_TEST_GH", "GWT_FAKE_GH_MODE"] {
            assert_eq!(
                denial_for_env("gh pr view 12", env_with(&[marker])),
                None,
                "marker {marker} must allow the spawn"
            );
        }
    }

    #[test]
    fn live_opt_in_allows_the_spawn() {
        assert_eq!(
            denial_for_env("gh pr view 12", env_with(&["GWT_ALLOW_REAL_GH"])),
            None
        );
    }

    #[test]
    fn disarmed_guard_allows_everything() {
        // The global flag defaults to off; production binaries never arm it.
        // Only the pure core is exercised here — arming the process-global
        // flag inside gwt-core's parallel test binary would refuse the
        // legitimate ProcessKind::Gh fixtures in sibling spawn tests.
        assert_eq!(unsandboxed_gh_denial("gh pr view 12"), None);
    }
}
