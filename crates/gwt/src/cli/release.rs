//! `release.status` — the interrupted-release standing check (Issue #3516).
//!
//! `/release` bumps the version on `develop` and only then opens the
//! `develop -> main` Release PR. When it stops in between, the bump sits on the
//! branch and nothing drives it to a release until a human notices. This
//! operation makes that gap readable every PM cycle, and — when the caller opts
//! in with `ensure_release_pr` — opens the missing Release PR idempotently.

use std::path::Path;

use gwt_core::update::{self, UpdateApplyOutcome};
use gwt_git::release_status::{
    self, ReleaseCheck, ReleaseCheckOptions, ReleaseCheckState, ReleasePrEnsure, RuntimeBuildStamp,
    RuntimeGeneration,
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
    /// Defer automatic apply of this exact staged version on the host.
    UpdateDefer { version: String },
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: ReleaseCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let (release_branch, base_branch, scan_commits, ensure_release_pr) = match command {
        ReleaseCommand::Status {
            release_branch,
            base_branch,
            scan_commits,
            ensure_release_pr,
        } => (release_branch, base_branch, scan_commits, ensure_release_pr),
        ReleaseCommand::UpdateDefer { version } => {
            let manifest =
                update::defer_pending_update(&version).map_err(SpecOpsError::Validation)?;
            let payload = serde_json::json!({
                "pending_update_version": manifest.version,
                "auto_apply_deferred": manifest.auto_apply_deferred,
                "update_stage": "deferred",
            });
            out.push_str(
                &serde_json::to_string_pretty(&payload).map_err(super::serde_as_api_error)?,
            );
            out.push('\n');
            return Ok(0);
        }
    };
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
    let generation = release_status::fetch_runtime_generation(
        &repo_path,
        &options.release_branch,
        build_stamp(),
    );
    render(&outcome, &generation, Some(&repo_path), out)
}

/// The build stamp `build.rs` compiled into this binary.
fn build_stamp() -> RuntimeBuildStamp {
    build_stamp_from(
        option_env!("GWT_BUILD_COMMIT"),
        option_env!("GWT_BUILD_EPOCH"),
    )
}

/// Read the stamp, treating an absent or empty value as unknown.
///
/// `build.rs` emits both variables unconditionally and leaves them empty when
/// git was unavailable, so "" means "not known", never "no commit".
fn build_stamp_from(commit: Option<&str>, epoch: Option<&str>) -> RuntimeBuildStamp {
    RuntimeBuildStamp {
        commit: non_empty(commit),
        time: non_empty(epoch)
            .and_then(|seconds| seconds.parse::<i64>().ok())
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|stamp| stamp.to_rfc3339()),
    }
}

/// `Some(trimmed)` when the value carries something.
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
fn render(
    outcome: &ReleasePrEnsure,
    generation: &RuntimeGeneration,
    repo_path: Option<&Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let payload = match repo_path {
        Some(path) => status_json_with_local(outcome, generation, path),
        None => status_json(outcome, generation),
    };
    out.push_str(&serde_json::to_string_pretty(&payload).map_err(super::serde_as_api_error)?);
    out.push('\n');
    Ok(0)
}

/// JSON projection of the check, including the action a PM should take.
///
/// The `build_*` / `*_head` / `stale_runtime` fields answer a different
/// question than the release state: not "did the release finish" but "is the
/// binary answering this call built from the code we merged" (SPEC #4249
/// FR-002). They are always present, and `null` wherever the comparison could
/// not be made — reporting an unknown runtime as current is the bug.
pub fn status_json(outcome: &ReleasePrEnsure, generation: &RuntimeGeneration) -> serde_json::Value {
    let check = &outcome.check;
    serde_json::json!({
        "state": check.state.as_str(),
        "stalled": check.is_stalled(),
        "version": check.version,
        "pending_version": check.pending_version,
        "pending_update_version": null,
        "auto_apply_deferred": false,
        "update_stage": null,
        "update_wait": null,
        "update_wait_overdue": false,
        "last_apply_result": null,
        "last_apply_failure": null,
        "version_source": "github_remote_tags",
        "release_pr": check.release_pr,
        "release_branch": check.release_branch,
        "base_branch": check.base_branch,
        "created_release_pr": outcome.created,
        "release_pr_url": outcome.pr_url,
        "default_action": default_action(check, outcome.created),
        "build_commit": generation.build_commit,
        "build_time": generation.build_time,
        "default_branch_head": generation.default_branch_head,
        "behind_commits": generation.behind_commits,
        "stale_runtime": generation.stale_runtime,
        "owner_action": generation.owner_action,
    })
}

/// Add read-only observations from this machine without changing the remote
/// release bump's `pending_version` meaning. An apply result describes only
/// the most recent marker; its attempt counter is never a cumulative count.
fn status_json_with_local(
    outcome: &ReleasePrEnsure,
    generation: &RuntimeGeneration,
    repo_path: &Path,
) -> serde_json::Value {
    let mut payload = status_json(outcome, generation);
    // Keep the staged version and its failure observation visible even when
    // the prepared payload has disappeared. Only the validated loader grants
    // eligibility for the GUI apply guidance below.
    let pending = std::fs::read(update::pending_update_manifest_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<update::PendingUpdateManifest>(&bytes).ok());
    let can_apply = update::load_pending_update_manifest().is_some();
    let payload_missing = pending.is_some() && !can_apply;
    let deferred = pending
        .as_ref()
        .is_some_and(|manifest| manifest.auto_apply_deferred);
    let wait = update::load_update_wait_observation(repo_path).filter(|wait| {
        pending.as_ref().is_some_and(|manifest| {
            !manifest.auto_apply_deferred
                && manifest.version == wait.version
                && match (
                    chrono::DateTime::parse_from_rfc3339(&manifest.downloaded_at),
                    chrono::DateTime::parse_from_rfc3339(&wait.observed_at),
                ) {
                    (Ok(downloaded), Ok(observed)) => observed >= downloaded,
                    _ => false,
                }
        })
    });
    let result = update::load_update_apply_result();
    let failure = result
        .as_ref()
        .filter(|result| result.outcome == UpdateApplyOutcome::Failure);
    payload["pending_update_version"] = serde_json::json!(pending.as_ref().map(|m| &m.version));
    payload["auto_apply_deferred"] = serde_json::json!(deferred);
    payload["update_stage"] = serde_json::json!(if payload_missing {
        Some("payload_missing")
    } else if deferred {
        Some("deferred")
    } else {
        wait.as_ref()
            .map(|wait| wait.stage.as_str())
            .or_else(|| pending.as_ref().map(|_| "staged"))
    });
    payload["update_wait"] = serde_json::json!(wait);
    // Issue #5062 AC-2: report an overdue waiting observation without
    // assuming why it has not been refreshed.
    let overdue = wait.as_ref().is_some_and(|wait| {
        wait.next_evaluation_at
            .as_deref()
            .and_then(|next| chrono::DateTime::parse_from_rfc3339(next).ok())
            .is_some_and(|next| {
                chrono::Utc::now() - next.with_timezone(&chrono::Utc)
                    > chrono::Duration::seconds(UPDATE_WAIT_OVERDUE_AFTER_SECS)
            })
    });
    payload["update_wait_overdue"] = serde_json::json!(overdue);
    payload["last_apply_result"] = serde_json::json!(result);
    payload["last_apply_failure"] = serde_json::json!(failure);

    let current_failure = wait
        .as_ref()
        .is_some_and(|wait| wait.stage == "pending_failed")
        || failure.is_some_and(|failure| {
            pending
                .as_ref()
                .is_none_or(|manifest| manifest.version == failure.to_version)
        });
    let defer_operation = pending.as_ref().map(|manifest| {
        format!(
            "release.update.defer {}",
            serde_json::json!({"version":manifest.version}),
        )
    });
    if deferred && can_apply {
        payload["owner_action"] = serde_json::json!(format!(
            "automatic apply of v{} is deferred; Issue Monitor launches resume; apply the staged update in the GUI when ready",
            pending.as_ref().unwrap().version,
        ));
    } else if payload_missing || current_failure {
        payload["owner_action"] = serde_json::json!(
            "download the update again or reinstall GWT.app; inspect update_wait.reason and last_apply_failure before retrying"
        );
    } else if can_apply && overdue {
        payload["owner_action"] = serde_json::json!(format!(
            "drain evaluation observation is overdue; inspect update_wait for the last recorded blocker and next evaluation; run {} while staged/waiting to defer it and resume Issue Monitor launches; an already committed helper cannot be cancelled",
            defer_operation.as_deref().unwrap_or_default(),
        ));
    } else if can_apply {
        // Issue #5062 AC-3: a drain holds new launches until it applies, so
        // name the control that defers the update and resumes launches.
        payload["owner_action"] = serde_json::json!(format!(
            "apply the pending update in the GUI after the active work has drained; inspect update_wait for the current blocker; to defer the update and resume Issue Monitor launches, run {} while staged/waiting; an already committed helper cannot be cancelled",
            defer_operation.as_deref().unwrap_or_default(),
        ));
    }
    payload
}

/// How far past `next_evaluation_at` a waiting observation may lag before
/// `release.status` reports the drain evaluation as overdue.
const UPDATE_WAIT_OVERDUE_AFTER_SECS: i64 = 120;

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
            pending_version: matches!(
                state,
                ReleaseCheckState::Stalled | ReleaseCheckState::PrOpen
            )
            .then(|| version.map(str::to_string))
            .flatten(),
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
    fn local_update_fields_are_explicit_when_no_observation_exists() {
        let outcome = ensure(check(ReleaseCheckState::NoBump, None, None), false, None);
        let payload = status_json(&outcome, &RuntimeGeneration::unknown("develop"));
        for field in [
            "pending_update_version",
            "update_wait",
            "last_apply_result",
            "last_apply_failure",
        ] {
            assert_eq!(
                payload.get(field),
                Some(&serde_json::Value::Null),
                "{field}"
            );
        }
    }

    #[test]
    fn local_update_status_reports_deferred_pending_without_live_wait() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let payload_path = home.path().join("prepared-binary");
        std::fs::write(&payload_path, "prepared payload").unwrap();
        let manifest = serde_json::json!({
            "version": "9.200.0",
            "asset_url": "https://example.invalid/update",
            "payload": {"PortableBinary": {"path": payload_path}},
            "downloaded_at": "2026-10-01T00:00:00Z",
            "auto_apply_deferred": true,
        });
        std::fs::create_dir_all(update::pending_update_dir()).unwrap();
        std::fs::write(
            update::pending_update_manifest_path(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        update::log_update_event(
            "pending_waiting",
            &[
                ("project_root", home.path().to_str().unwrap()),
                ("version", "9.200.0"),
                ("reason", "active_work"),
                ("observed_at", "2026-10-01T00:10:00Z"),
                ("next_evaluation_at", "2026-10-01T00:10:05Z"),
            ],
        );
        let outcome = ensure(check(ReleaseCheckState::NoBump, None, None), false, None);
        let payload = status_json_with_local(
            &outcome,
            &RuntimeGeneration::unknown("develop"),
            home.path(),
        );
        assert_eq!(payload["pending_update_version"], "9.200.0");
        assert_eq!(payload["auto_apply_deferred"], true);
        assert_eq!(payload["update_stage"], "deferred");
        assert!(payload["update_wait"].is_null());
        assert_eq!(payload["update_wait_overdue"], false);
        assert!(payload["owner_action"]
            .as_str()
            .unwrap()
            .contains("deferred"));
        assert!(payload["owner_action"]
            .as_str()
            .unwrap()
            .contains("9.200.0"));
        assert!(payload["owner_action"].as_str().unwrap().contains("GUI"));
        assert!(payload["owner_action"]
            .as_str()
            .unwrap()
            .contains("launches"));
    }

    #[test]
    fn local_update_status_reads_valid_payload_and_the_last_attempt_without_applying() {
        use gwt_core::update::{
            self, PendingUpdateManifest, PreparedPayload, UpdateApplyOutcome, UpdateApplyResult,
        };
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let payload_path = home.path().join("prepared-binary");
        std::fs::write(&payload_path, "not executable").unwrap();
        let manifest = PendingUpdateManifest {
            version: "9.200.0".into(),
            asset_url: "https://example.invalid/update".into(),
            payload: PreparedPayload::PortableBinary {
                path: payload_path.clone(),
            },
            downloaded_at: "2026-10-01T00:00:00Z".into(),
            auto_apply_deferred: false,
        };
        update::persist_pending_update_manifest(&manifest).unwrap();
        let outcome = ensure(
            check(ReleaseCheckState::Stalled, Some("v9.201.0"), None),
            false,
            None,
        );
        let generation = RuntimeGeneration::unknown("develop");
        let payload = status_json_with_local(&outcome, &generation, home.path());
        assert_eq!(payload["pending_version"], "v9.201.0");
        assert_eq!(payload["pending_update_version"], "9.200.0");
        assert_eq!(payload["update_stage"], "staged");
        assert!(payload["owner_action"].as_str().unwrap().contains("apply"));
        assert_eq!(
            std::fs::read_to_string(&payload_path).unwrap(),
            "not executable"
        );

        update::log_update_event(
            "pending_waiting",
            &[
                ("project_root", home.path().to_str().unwrap()),
                ("version", "9.200.0"),
                ("reason", "active_work"),
                ("observed_at", "2026-10-01T00:10:00Z"),
                ("next_evaluation_at", "2026-10-01T00:10:05Z"),
            ],
        );
        let waiting = status_json_with_local(&outcome, &generation, home.path());
        assert_eq!(waiting["update_stage"], "pending_waiting");
        assert_eq!(waiting["update_wait"]["reason"], "active_work");
        assert_eq!(
            waiting["update_wait"]["next_evaluation_at"],
            "2026-10-01T00:10:05Z"
        );
        // Issue #5062 AC-2: mark an overdue observation without claiming
        // that its age proves a stopped scheduler.
        assert_eq!(waiting["update_wait_overdue"], true);
        assert!(waiting["owner_action"]
            .as_str()
            .unwrap()
            .contains("overdue"));
        // Issue #5062 AC-3: the owner can resume launches without applying.
        assert!(
            waiting["owner_action"]
                .as_str()
                .unwrap()
                .contains("release.update.defer {\"version\":\"9.200.0\"}"),
            "{}",
            waiting["owner_action"]
        );
        update::log_update_event(
            "pending_failed",
            &[
                ("project_root", home.path().to_str().unwrap()),
                ("version", "9.200.0"),
                ("reason", "apply_failed"),
                ("observed_at", "2026-10-01T00:11:00Z"),
            ],
        );
        let failed_wait = status_json_with_local(&outcome, &generation, home.path());
        assert!(failed_wait["owner_action"]
            .as_str()
            .unwrap()
            .contains("download"));

        update::persist_update_apply_result(&UpdateApplyResult {
            outcome: UpdateApplyOutcome::Failure,
            from_version: "9.199.0".into(),
            to_version: "9.200.0".into(),
            observed_version: "9.199.0".into(),
            attempt: 1,
            recorded_at: "2026-10-01T01:00:00Z".into(),
            message: Some("payload copy failed".into()),
        })
        .unwrap();
        let failed = status_json_with_local(&outcome, &generation, home.path());
        assert_eq!(
            failed["last_apply_failure"]["message"],
            "payload copy failed"
        );
        assert_eq!(failed["last_apply_result"]["attempt"], 1);
        assert!(failed["owner_action"]
            .as_str()
            .unwrap()
            .contains("download"));
        assert!(!failed["owner_action"]
            .as_str()
            .unwrap()
            .contains("apply the pending"));

        std::fs::remove_file(payload_path).unwrap();
        std::fs::remove_file(update::update_apply_result_path()).unwrap();
        update::log_update_event(
            "pending_failed",
            &[
                ("project_root", home.path().to_str().unwrap()),
                ("version", "9.200.0"),
                ("reason", "payload_missing"),
                ("observed_at", "2026-10-01T01:10:00Z"),
            ],
        );
        let missing = status_json_with_local(&outcome, &generation, home.path());
        assert_eq!(missing["pending_update_version"], "9.200.0");
        assert_eq!(missing["update_stage"], "payload_missing");
        assert_eq!(missing["update_wait"]["stage"], "pending_failed");
        assert_eq!(missing["update_wait"]["reason"], "payload_missing");
        assert!(missing["last_apply_result"].is_null());
        assert!(missing["owner_action"]
            .as_str()
            .unwrap()
            .contains("reinstall"));
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
        render(
            &outcome,
            &RuntimeGeneration::unknown("develop"),
            None,
            &mut out,
        )
        .expect("render");
        assert!(out.contains("\"state\": \"stalled\""), "{out}");
        assert!(out.contains("\"stalled\": true"), "{out}");
        assert!(out.contains("\"version\": \"v9.91.0\""), "{out}");
        assert!(out.contains("ensure_release_pr:true"), "{out}");
    }

    #[test]
    fn released_payload_identifies_the_remote_version_source() {
        let outcome = ensure(
            check(ReleaseCheckState::Released, Some("v9.102.0"), None),
            false,
            None,
        );
        let payload = status_json(&outcome, &RuntimeGeneration::unknown("develop"));
        assert_eq!(payload["version_source"], "github_remote_tags");
        assert_eq!(
            payload.get("pending_version"),
            Some(&serde_json::Value::Null)
        );
    }

    #[test]
    fn a_created_release_pr_reports_the_url_and_no_further_action() {
        let mut out = String::new();
        let outcome = ensure(
            check(ReleaseCheckState::PrOpen, Some("v9.91.0"), Some(3513)),
            true,
            Some("https://github.com/akiojin/gwt/pull/3513"),
        );
        render(
            &outcome,
            &RuntimeGeneration::unknown("develop"),
            None,
            &mut out,
        )
        .expect("render");
        assert!(out.contains("\"created_release_pr\": true"), "{out}");
        assert!(out.contains("/pull/3513"), "{out}");
        assert!(out.contains("opened by this call"), "{out}");
    }

    /// SPEC #4249 FR-002 / AC-3: the PM reads "is the running binary the code
    /// we merged?" from this payload, so the six runtime-generation fields are
    /// always present — including when they are unknown.
    #[test]
    fn the_payload_carries_the_runtime_generation() {
        let outcome = ensure(check(ReleaseCheckState::NoBump, None, None), false, None);
        let generation = RuntimeGeneration {
            branch: "develop".to_string(),
            build_commit: Some("5b10bca75c4f2a6d9e8b1c3f0a7d4e2b6c8f1a39".to_string()),
            build_time: Some("2026-09-10T01:29:00+00:00".to_string()),
            default_branch_head: Some("85a216ee6b1d4c7f9a2e8b5c0d3f6a1e4b7c9d02".to_string()),
            behind_commits: Some(37),
            stale_runtime: Some(true),
            owner_action: Some("restart GWT.app".to_string()),
        };
        let payload = status_json(&outcome, &generation);
        assert_eq!(
            payload["build_commit"],
            serde_json::json!("5b10bca75c4f2a6d9e8b1c3f0a7d4e2b6c8f1a39")
        );
        assert_eq!(
            payload["build_time"],
            serde_json::json!("2026-09-10T01:29:00+00:00")
        );
        assert_eq!(
            payload["default_branch_head"],
            serde_json::json!("85a216ee6b1d4c7f9a2e8b5c0d3f6a1e4b7c9d02")
        );
        assert_eq!(payload["behind_commits"], serde_json::json!(37));
        assert_eq!(payload["stale_runtime"], serde_json::json!(true));
        assert_eq!(
            payload["owner_action"],
            serde_json::json!("restart GWT.app")
        );
    }

    #[test]
    fn an_unknown_runtime_generation_renders_json_null_not_a_missing_key() {
        let outcome = ensure(check(ReleaseCheckState::NoBump, None, None), false, None);
        let payload = status_json(&outcome, &RuntimeGeneration::unknown("develop"));
        for field in [
            "build_commit",
            "build_time",
            "default_branch_head",
            "behind_commits",
            "stale_runtime",
            "owner_action",
        ] {
            assert_eq!(
                payload.get(field),
                Some(&serde_json::Value::Null),
                "{field} must be present and null"
            );
        }
    }

    /// The build stamp is compiled in by `build.rs`; an empty value (no git at
    /// build time) must read as unknown rather than as an empty commit id.
    #[test]
    fn the_embedded_build_stamp_treats_an_empty_value_as_unknown() {
        assert_eq!(
            build_stamp_from(Some(""), Some("")),
            RuntimeBuildStamp::default()
        );
        assert_eq!(
            build_stamp_from(Some("5b10bca7"), Some("1789003740")),
            RuntimeBuildStamp {
                commit: Some("5b10bca7".to_string()),
                time: Some("2026-09-10T01:29:00+00:00".to_string()),
            }
        );
        assert_eq!(build_stamp_from(None, None), RuntimeBuildStamp::default());
    }

    #[test]
    fn the_quiet_states_ask_for_nothing() {
        for (state, marker) in [
            (ReleaseCheckState::NoBump, "no version bump is pending"),
            (ReleaseCheckState::Released, "already tagged"),
            (ReleaseCheckState::PrOpen, "already open"),
        ] {
            let outcome = ensure(check(state, Some("v9.91.0"), None), false, None);
            let payload = status_json(&outcome, &RuntimeGeneration::unknown("develop"));
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
