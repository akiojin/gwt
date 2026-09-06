//! `release.status` — the interrupted-release standing check (Issue #3516).
//!
//! `/release` bumps the version on `develop` and only then opens the
//! `develop -> main` Release PR. When it stops in between, the bump sits on the
//! branch and nothing drives it to a release until a human notices. This
//! operation makes that gap readable every PM cycle, and — when the caller opts
//! in with `ensure_release_pr` — opens the missing Release PR idempotently.

use gwt_git::release_status::{
    self, ReleaseCheck, ReleaseCheckOptions, ReleaseCheckState, ReleasePrEnsure,
};
use gwt_github::{ApiError, SpecOpsError};

use crate::cli::CliEnv;

/// Command model for the `release.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseCommand {
    /// Report the release pipeline state, optionally reconciling it.
    Status {
        /// Branch the version bump lands on.
        release_branch: Option<String>,
        /// Branch the Release PR targets.
        base_branch: Option<String>,
        /// How many release-branch subjects to scan for the bump commit.
        scan_commits: Option<u64>,
        /// Open the missing Release PR when the release is stalled.
        ensure_release_pr: bool,
    },
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: ReleaseCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let ReleaseCommand::Status {
        release_branch,
        base_branch,
        scan_commits,
        ensure_release_pr,
    } = command;
    let options = options_from(release_branch, base_branch, scan_commits);
    let repo_path = env.repo_path().to_path_buf();
    let outcome = if ensure_release_pr {
        release_status::ensure_release_pr(&repo_path, &options).map_err(git_as_api_error)?
    } else {
        ReleasePrEnsure {
            check: release_status::fetch_release_check(&repo_path, &options)
                .map_err(git_as_api_error)?,
            created: false,
            pr_url: None,
        }
    };
    render(&outcome, out)
}

/// Build the check options, falling back to the project's release topology.
fn options_from(
    release_branch: Option<String>,
    base_branch: Option<String>,
    scan_commits: Option<u64>,
) -> ReleaseCheckOptions {
    let defaults = ReleaseCheckOptions::default();
    ReleaseCheckOptions {
        release_branch: release_branch.unwrap_or(defaults.release_branch),
        base_branch: base_branch.unwrap_or(defaults.base_branch),
        scan_commits: scan_commits
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count > 0)
            .unwrap_or(defaults.scan_commits),
    }
}

/// Render the JSON payload for one check.
fn render(outcome: &ReleasePrEnsure, out: &mut String) -> Result<i32, SpecOpsError> {
    let payload = status_json(outcome);
    out.push_str(&serde_json::to_string_pretty(&payload).map_err(super::serde_as_api_error)?);
    out.push('\n');
    Ok(0)
}

/// JSON projection of the check, including the action a PM should take.
pub fn status_json(outcome: &ReleasePrEnsure) -> serde_json::Value {
    let check = &outcome.check;
    serde_json::json!({
        "state": check.state.as_str(),
        "stalled": check.is_stalled(),
        "version": check.version,
        "release_pr": check.release_pr,
        "release_branch": check.release_branch,
        "base_branch": check.base_branch,
        "created_release_pr": outcome.created,
        "release_pr_url": outcome.pr_url,
        "default_action": default_action(check, outcome.created),
    })
}

/// The single next step for this state, so the PM classifies nothing itself.
fn default_action(check: &ReleaseCheck, created: bool) -> &'static str {
    if created {
        return "none — the missing Release PR was opened by this call";
    }
    match check.state {
        ReleaseCheckState::NoBump => "none — no version bump is pending",
        ReleaseCheckState::Released => "none — the bumped version is already tagged",
        ReleaseCheckState::PrOpen => "none — the Release PR is already open",
        ReleaseCheckState::Stalled => {
            "rerun release.status with ensure_release_pr:true to open the missing Release PR"
        }
    }
}

/// Map a git/gh failure onto the CLI error type.
fn git_as_api_error(error: gwt_core::GwtError) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(state: ReleaseCheckState, version: Option<&str>, pr: Option<u64>) -> ReleaseCheck {
        ReleaseCheck {
            state,
            version: version.map(str::to_string),
            release_pr: pr,
            release_branch: "develop".to_string(),
            base_branch: "main".to_string(),
        }
    }

    fn ensure(check: ReleaseCheck, created: bool, url: Option<&str>) -> ReleasePrEnsure {
        ReleasePrEnsure {
            check,
            created,
            pr_url: url.map(str::to_string),
        }
    }

    #[test]
    fn options_fall_back_to_the_project_release_topology() {
        let options = options_from(None, None, None);
        assert_eq!(options.release_branch, "develop");
        assert_eq!(options.base_branch, "main");
        assert_eq!(options.scan_commits, release_status::DEFAULT_SCAN_COMMITS);
    }

    #[test]
    fn options_accept_overrides_and_reject_a_zero_scan_window() {
        let options = options_from(
            Some("release".to_string()),
            Some("trunk".to_string()),
            Some(0),
        );
        assert_eq!(options.release_branch, "release");
        assert_eq!(options.base_branch, "trunk");
        assert_eq!(options.scan_commits, release_status::DEFAULT_SCAN_COMMITS);
        assert_eq!(options_from(None, None, Some(5)).scan_commits, 5);
    }

    #[test]
    fn a_stalled_release_renders_the_recovery_action() {
        let mut out = String::new();
        let outcome = ensure(
            check(ReleaseCheckState::Stalled, Some("v9.91.0"), None),
            false,
            None,
        );
        render(&outcome, &mut out).expect("render");
        assert!(out.contains("\"state\": \"stalled\""), "{out}");
        assert!(out.contains("\"stalled\": true"), "{out}");
        assert!(out.contains("\"version\": \"v9.91.0\""), "{out}");
        assert!(out.contains("ensure_release_pr:true"), "{out}");
    }

    #[test]
    fn a_created_release_pr_reports_the_url_and_no_further_action() {
        let mut out = String::new();
        let outcome = ensure(
            check(ReleaseCheckState::PrOpen, Some("v9.91.0"), Some(3513)),
            true,
            Some("https://github.com/akiojin/gwt/pull/3513"),
        );
        render(&outcome, &mut out).expect("render");
        assert!(out.contains("\"created_release_pr\": true"), "{out}");
        assert!(out.contains("/pull/3513"), "{out}");
        assert!(out.contains("opened by this call"), "{out}");
    }

    #[test]
    fn the_quiet_states_ask_for_nothing() {
        for (state, marker) in [
            (ReleaseCheckState::NoBump, "no version bump is pending"),
            (ReleaseCheckState::Released, "already tagged"),
            (ReleaseCheckState::PrOpen, "already open"),
        ] {
            let outcome = ensure(check(state, Some("v9.91.0"), None), false, None);
            let payload = status_json(&outcome);
            assert_eq!(payload["stalled"], serde_json::json!(false));
            assert_eq!(payload["created_release_pr"], serde_json::json!(false));
            assert!(
                payload["default_action"]
                    .as_str()
                    .is_some_and(|action| action.contains(marker)),
                "unexpected action for {state:?}: {payload}"
            );
        }
    }
}
