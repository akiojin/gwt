//! Automatic build-artifact reclaim on low disk (Issue #4391).
//!
//! `worktree.gc_build_artifacts` (Issue #4009) reclaims the `target/` of
//! merged, idle worktrees, and `issue.monitor.status` warns once a volume runs
//! low — but nothing connected the two. On 2026-09-15 the host carried 749 GB
//! of build output with 327 GB of it reclaimable across 43 merged worktrees,
//! and it stayed there because only an operator ever ran the sweep. The Issue
//! Monitor scan now asks [`maybe_spawn`] on every pass: once the threshold the
//! warning uses is crossed, the sweep runs on a background thread, preferring
//! merged caches and reclaiming idle unmerged caches only while pressure
//! remains. Its result is appended to a record the status reads.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cli::worktree_gc::{self, GcFailure, GcOptions, GcRemoval, GcReport};
use crate::disk_space::{DiskSpaceStatus, DiskThresholds};

/// The first priority is the same as the manual defaults. Under continued
/// pressure the runner also considers idle unmerged caches (Issue #4704).
pub(crate) const AUTO_GC_OPTIONS: GcOptions = GcOptions {
    include_unmerged: false,
    include_protected_workspaces: false,
};

/// After a run, the next one waits this long even while the disk stays low.
/// A sweep spawns `git` twice per worktree (462 on the reporting host), and a
/// sweep that just reclaimed everything it could reclaims nothing more until
/// further worktrees merge — on the order of an hour of fleet activity.
pub const COOLDOWN_SECS: i64 = 60 * 60;

/// Append-only run history under the project data directory (AC-3).
pub const RECORD_FILE_NAME: &str = "build-artifact-gc.jsonl";
/// Held while a sweep runs, so two gwt processes on the host never sweep the
/// same worktrees at once.
const LOCK_FILE_NAME: &str = "build-artifact-gc.lock";

/// One sweep at a time per process.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// What one scan pass decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoGcDecision {
    /// Reclaim now; carries the disk warning that triggered it.
    Run {
        trigger: String,
    },
    Skip {
        reason: String,
    },
}

/// Whether a run's counts are measurements (Issue #4566 AC-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildArtifactGcOutcome {
    /// The sweep reclaimed build cache bytes.
    Swept,
    /// Disk pressure triggered a sweep, but it reclaimed no bytes.
    NoReclaim,
    /// The sweep never got a worktree list. `candidates`,
    /// `reclaimable_bytes` and `reclaimed_bytes` are placeholders below, not
    /// measurements: a reader must not take the zeros for an idle host.
    EnumerationFailed,
}

/// One automatic run, as written to [`RECORD_FILE_NAME`] (AC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildArtifactGcRecord {
    pub started_at: String,
    pub finished_at: String,
    /// The disk warning that started the run.
    pub trigger: String,
    pub base: String,
    /// Whether enumeration and reclaim succeeded. [`Self::outcome`] also
    /// interprets legacy records that lack this field or call zero bytes swept.
    #[serde(default, rename = "outcome")]
    pub recorded_outcome: Option<BuildArtifactGcOutcome>,
    pub candidates: usize,
    pub reclaimable_bytes: u64,
    pub reclaimed_bytes: u64,
    #[serde(default)]
    pub removed: Vec<GcRemoval>,
    #[serde(default)]
    pub failed: Vec<GcFailure>,
    /// Kept worktrees per reason (`active process`, `not merged into
    /// origin/develop`, …). The per-worktree detail names pids and sessions
    /// that are meaningless an hour later; the counts are what an operator
    /// reads to see why space was not reclaimed.
    #[serde(default)]
    pub kept_by_reason: BTreeMap<String, usize>,
    /// Pressure was not relieved by any reclaim; kept reasons explain why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// The sweep itself failed (for example `git worktree list`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BuildArtifactGcRecord {
    /// What this run actually did, for records old and new alike.
    pub fn outcome(&self) -> BuildArtifactGcOutcome {
        let outcome = self.recorded_outcome.unwrap_or(if self.error.is_some() {
            BuildArtifactGcOutcome::EnumerationFailed
        } else {
            BuildArtifactGcOutcome::Swept
        });
        if outcome == BuildArtifactGcOutcome::Swept && self.reclaimed_bytes == 0 {
            BuildArtifactGcOutcome::NoReclaim
        } else {
            outcome
        }
    }

    fn normalize_outcome(&mut self) {
        let outcome = self.outcome();
        self.recorded_outcome = Some(outcome);
        if outcome == BuildArtifactGcOutcome::NoReclaim {
            self.warning = Some(format!(
                "disk pressure GC reclaimed 0 bytes; kept_by_reason: {:?}",
                self.kept_by_reason
            ));
        }
    }
}

/// The `[build_artifact_gc]` settings, or the defaults when the settings
/// file is missing or unreadable: a typo must not switch the reclaim off.
pub fn current_config() -> gwt_config::BuildArtifactGcConfig {
    let path = gwt_core::paths::gwt_config_path();
    if !path.is_file() {
        return gwt_config::BuildArtifactGcConfig::default();
    }
    gwt_config::Settings::load_from_path(&path)
        .map(|settings| settings.build_artifact_gc)
        .unwrap_or_default()
}

/// Free space where the worktrees live and where the verification
/// coordinator writes its lease, judged against the configured thresholds.
/// `issue.monitor.status` reports exactly this, so the warning and the
/// trigger are one judgment (AC-1).
pub fn probe_disk(
    project_root: &Path,
    config: &gwt_config::BuildArtifactGcConfig,
) -> DiskSpaceStatus {
    let coordinator_root = gwt_core::index_coordinator::coordinator_root();
    crate::disk_space::probe_with(
        &[project_root, coordinator_root.as_path()],
        DiskThresholds::from(config),
    )
}

/// Whether this pass should reclaim. Pure, so every branch is a unit test
/// (AC-6).
pub fn decide(
    config: &gwt_config::BuildArtifactGcConfig,
    disk: &DiskSpaceStatus,
    last_finished_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> AutoGcDecision {
    if !config.auto {
        return AutoGcDecision::Skip {
            reason: "disabled by [build_artifact_gc] auto = false".to_string(),
        };
    }
    let Some(warning) = &disk.warning else {
        return AutoGcDecision::Skip {
            reason: "free space is above the threshold".to_string(),
        };
    };
    if within_cooldown(last_finished_at, now) {
        return AutoGcDecision::Skip {
            reason: format!("cooling down: the last run finished less than {COOLDOWN_SECS}s ago"),
        };
    }
    AutoGcDecision::Run {
        trigger: warning.clone(),
    }
}

/// Where this project's run history lives.
pub fn record_path(project_root: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(project_root).join(RECORD_FILE_NAME)
}

/// The most recent run, if any. Read-only: a missing file is `None`.
pub fn last_record(path: &Path) -> Option<BuildArtifactGcRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut record: BuildArtifactGcRecord = text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())?;
    record.normalize_outcome();
    Some(record)
}

fn append_record(path: &Path, record: &BuildArtifactGcRecord) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

fn last_finished_at(path: &Path) -> Option<DateTime<Utc>> {
    last_record(path)
        .and_then(|record| DateTime::parse_from_rfc3339(&record.finished_at).ok())
        .map(|at| at.with_timezone(&Utc))
}

fn within_cooldown(last_finished_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    last_finished_at.is_some_and(|last| (now - last).num_seconds() < COOLDOWN_SECS)
}

/// Called by the Issue Monitor scan on every pass. Cheap unless it decides to
/// run: one free-space query per volume and one small file read. The sweep
/// itself — which walks every worktree and may delete hundreds of gigabytes —
/// runs on its own thread so it never holds the scan.
pub fn maybe_spawn(project_root: &Path) -> AutoGcDecision {
    let config = current_config();
    let disk = probe_disk(project_root, &config);
    let record_path = record_path(project_root);
    let decision = decide(&config, &disk, last_finished_at(&record_path), Utc::now());
    let AutoGcDecision::Run { trigger } = &decision else {
        return decision;
    };
    if RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return decision;
    }
    let project_root = project_root.to_path_buf();
    let trigger = trigger.clone();
    let spawned = std::thread::Builder::new()
        .name("build-artifact-auto-gc".to_string())
        .spawn(move || {
            run_exclusive(&project_root, &record_path, &trigger);
            RUNNING.store(false, Ordering::Release);
        });
    if let Err(error) = spawned {
        RUNNING.store(false, Ordering::Release);
        tracing::warn!(%error, "build artifact auto-gc: failed to start the sweep thread");
    }
    decision
}

/// Run one sweep under the host-wide lock and record it.
fn run_exclusive(project_root: &Path, record_path: &Path, trigger: &str) {
    use fs2::FileExt as _;

    let Some(dir) = record_path.parent() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!(%error, dir = %dir.display(), "build artifact auto-gc: cannot create the record directory");
        return;
    }
    let lock = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE_NAME))
    {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!(%error, "build artifact auto-gc: cannot open the lock file");
            return;
        }
    };
    if lock.try_lock_exclusive().is_err() {
        // Another gwt process on this host is already sweeping.
        return;
    }
    // A sweep that finished while this one waited for the lock already did
    // the work; do not repeat it inside the cooldown.
    if within_cooldown(last_finished_at(record_path), Utc::now()) {
        return;
    }
    let started_at = now_rfc3339();
    let base = gwt_git::pr_status::SETTLEMENT_BASE_BRANCH;
    let config = current_config();
    let observe_disk = || probe_disk(project_root, &config);
    let result = worktree_gc::run_gc_with_pressure(
        project_root,
        base,
        AUTO_GC_OPTIONS,
        false,
        Some(&observe_disk),
    )
    .map_err(|error| error.to_string());
    let finished_at = now_rfc3339();
    let record = record_from_result(result, started_at, finished_at, trigger, base);
    if let Some(warning) = &record.warning {
        tracing::warn!(%warning, kept_by_reason = ?record.kept_by_reason, "build artifact auto-gc did not reclaim space");
    }
    tracing::info!(
        outcome = ?record.outcome(),
        candidates = record.candidates,
        reclaimed_bytes = record.reclaimed_bytes,
        removed = record.removed.len(),
        failed = record.failed.len(),
        error = record.error.as_deref().unwrap_or(""),
        "build artifact auto-gc finished"
    );
    if let Err(error) = append_record(record_path, &record) {
        tracing::warn!(%error, "build artifact auto-gc: cannot append the run record");
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The record for one sweep, whichever way it ended. A failed sweep is
/// recorded with the same three zeros a clean one would report, so it carries
/// [`BuildArtifactGcOutcome::EnumerationFailed`] to say those zeros measure
/// nothing (Issue #4566 AC-2).
fn record_from_result(
    result: Result<GcReport, String>,
    started_at: String,
    finished_at: String,
    trigger: &str,
    base: &str,
) -> BuildArtifactGcRecord {
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            return BuildArtifactGcRecord {
                started_at,
                finished_at,
                trigger: trigger.to_string(),
                base: base.to_string(),
                recorded_outcome: Some(BuildArtifactGcOutcome::EnumerationFailed),
                candidates: 0,
                reclaimable_bytes: 0,
                reclaimed_bytes: 0,
                removed: Vec::new(),
                failed: Vec::new(),
                kept_by_reason: BTreeMap::new(),
                warning: None,
                error: Some(error),
            };
        }
    };
    let mut kept_by_reason = BTreeMap::new();
    for kept in &report.kept {
        *kept_by_reason
            .entry(kept_reason_category(&kept.reason))
            .or_default() += 1;
    }
    let mut record = BuildArtifactGcRecord {
        started_at,
        finished_at,
        trigger: trigger.to_string(),
        base: report.base,
        recorded_outcome: Some(BuildArtifactGcOutcome::Swept),
        candidates: report.candidates.len(),
        reclaimable_bytes: report.reclaimable_bytes,
        reclaimed_bytes: report.reclaimed_bytes,
        removed: report.removed,
        failed: report.failed,
        kept_by_reason,
        warning: None,
        error: None,
    };
    record.normalize_outcome();
    record
}

/// `active process: claude (pid 1)` → `active process`; `not merged into
/// origin/develop (pass …)` → `not merged into origin/develop`.
fn kept_reason_category(reason: &str) -> String {
    reason
        .split([':', '('])
        .next()
        .unwrap_or(reason)
        .trim()
        .to_string()
}

#[cfg(test)]
mod pressure_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::worktree_gc::{plan, GcKept, WorktreeProbe};
    use crate::disk_space::{evaluate, evaluate_with, DiskVolume};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn volume(free_bytes: u64, total_bytes: u64) -> DiskVolume {
        DiskVolume {
            path: "/work".to_string(),
            free_bytes,
            total_bytes,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn enabled() -> gwt_config::BuildArtifactGcConfig {
        gwt_config::BuildArtifactGcConfig::default()
    }

    /// AC-1 / AC-6: below the warning threshold the reclaim starts, carrying
    /// the warning that triggered it.
    #[test]
    fn fires_when_free_space_is_below_the_threshold() {
        let disk = evaluate(vec![volume(GIB, 3600 * GIB)]);
        let warning = disk.warning.clone().expect("low disk warns");
        assert_eq!(
            decide(&enabled(), &disk, None, now()),
            AutoGcDecision::Run { trigger: warning }
        );
    }

    /// AC-6: a healthy volume starts nothing.
    #[test]
    fn does_not_fire_above_the_threshold() {
        let disk = evaluate(vec![volume(500 * GIB, 3600 * GIB)]);
        assert!(matches!(
            decide(&enabled(), &disk, None, now()),
            AutoGcDecision::Skip { .. }
        ));
    }

    /// AC-5 / AC-6: `auto = false` starts nothing, however low the disk is.
    #[test]
    fn does_not_fire_when_disabled() {
        let config = gwt_config::BuildArtifactGcConfig {
            auto: false,
            ..enabled()
        };
        let disk = evaluate(vec![volume(GIB, 3600 * GIB)]);
        let AutoGcDecision::Skip { reason } = decide(&config, &disk, None, now()) else {
            panic!("disabled auto-gc must not run");
        };
        assert!(reason.contains("auto = false"), "{reason}");
    }

    /// A run that just finished is not repeated every scan while the disk
    /// stays low; once the cooldown passes it runs again.
    #[test]
    fn does_not_fire_again_within_the_cooldown() {
        let disk = evaluate(vec![volume(GIB, 3600 * GIB)]);
        let recent = now() - chrono::Duration::minutes(10);
        assert!(matches!(
            decide(&enabled(), &disk, Some(recent), now()),
            AutoGcDecision::Skip { .. }
        ));
        let stale = now() - chrono::Duration::seconds(COOLDOWN_SECS + 1);
        assert!(matches!(
            decide(&enabled(), &disk, Some(stale), now()),
            AutoGcDecision::Run { .. }
        ));
    }

    /// AC-4: a configured threshold moves both the warning and the trigger.
    #[test]
    fn a_configured_threshold_moves_the_trigger() {
        let config = gwt_config::BuildArtifactGcConfig {
            below_bytes: 1000 * GIB,
            ..enabled()
        };
        // 500 GiB free of 3.6 TiB: healthy by default, low under the setting.
        let disk = evaluate_with(
            vec![volume(500 * GIB, 3600 * GIB)],
            DiskThresholds::from(&config),
        );
        assert_eq!(disk.warn_below_bytes, 1000 * GIB);
        assert!(matches!(
            decide(&config, &disk, None, now()),
            AutoGcDecision::Run { .. }
        ));
    }

    fn probe(root: &str) -> WorktreeProbe {
        WorktreeProbe {
            root: PathBuf::from(root),
            branch: Some(format!("work/{root}")),
            is_main: false,
            is_base_branch_workspace: false,
            has_build_artifacts: true,
            active_processes: Vec::new(),
            tracked_sessions: Vec::new(),
            merged: Ok(true),
        }
    }

    /// The first priority keeps unmerged caches; the pressure runner separately
    /// considers them after merged caches. Process/launch/shared guards remain.
    #[test]
    fn first_priority_excludes_running_and_unmerged_worktrees() {
        let mut running = probe("/work/running");
        running.active_processes = vec!["cargo (pid 7)".to_string()];
        let mut launched = probe("/work/launched");
        launched.tracked_sessions = vec!["session-1".to_string()];
        let mut unmerged = probe("/work/unmerged");
        unmerged.merged = Ok(false);
        let mut shared = probe("/work/develop");
        shared.is_base_branch_workspace = true;
        let merged = probe("/work/merged");

        let plan = plan(
            vec![running, launched, unmerged, shared, merged],
            "develop",
            AUTO_GC_OPTIONS,
            &[],
        );

        let candidates: Vec<_> = plan.candidates.iter().map(|c| c.worktree.clone()).collect();
        assert_eq!(candidates, vec![PathBuf::from("/work/merged")]);
        assert_eq!(plan.kept.len(), 4, "{plan:?}");
    }

    /// AC-3: the record carries the counts, the reclaimed bytes, what was
    /// removed, and why the rest was kept.
    #[test]
    fn record_counts_reclaimed_bytes_and_groups_kept_reasons() {
        let kept = |root: &str, reason: &str| GcKept {
            worktree: PathBuf::from(root),
            branch: None,
            reason: reason.to_string(),
        };
        let report = GcReport {
            dry_run: false,
            base: "develop".to_string(),
            include_unmerged: false,
            include_protected_workspaces: false,
            candidates: Vec::new(),
            kept: vec![
                kept("/a", "active process: cargo (pid 1)"),
                kept("/b", "active process: claude (pid 2)"),
                kept(
                    "/c",
                    "not merged into origin/develop (pass include_unmerged:true to reclaim)",
                ),
            ],
            reclaimable_bytes: 4096,
            removed: vec![GcRemoval {
                worktree: PathBuf::from("/d"),
                target: PathBuf::from("/d/target"),
                bytes: 4096,
            }],
            failed: Vec::new(),
            reclaimed_bytes: 4096,
            disk_space: evaluate(Vec::new()),
        };

        let record = record_from_result(
            Ok(report),
            "s".into(),
            "f".into(),
            "disk space low",
            "develop",
        );

        assert_eq!(record.outcome(), BuildArtifactGcOutcome::Swept);
        assert_eq!(record.reclaimed_bytes, 4096);
        assert_eq!(record.removed.len(), 1);
        assert_eq!(record.trigger, "disk space low");
        assert_eq!(
            record.kept_by_reason,
            BTreeMap::from([
                ("active process".to_string(), 2),
                ("not merged into origin/develop".to_string(), 1),
            ])
        );
    }

    /// Issue #4566 AC-2 / AC-3: a sweep that never got a worktree list
    /// reports the same three zeros a fully-reclaimed host reports. The
    /// record names which of the two it was, so nobody reads a broken sweep
    /// as an idle one.
    #[test]
    fn a_failed_enumeration_is_not_recorded_as_a_clean_sweep() {
        let record = record_from_result(
            Err("git worktree list failed: fatal: not a git repository".to_string()),
            "s".into(),
            "f".into(),
            "disk space low",
            "develop",
        );

        assert_eq!(record.outcome(), BuildArtifactGcOutcome::EnumerationFailed);
        assert_eq!(record.candidates, 0);
        assert!(record
            .error
            .as_deref()
            .is_some_and(|error| error.contains("not a git repository")));
        let json: serde_json::Value = serde_json::to_value(&record).expect("serialize");
        assert_eq!(json["outcome"], "enumeration_failed", "{json}");
    }

    /// Issue #4704 AC-1: enumeration success is not pressure relief.
    #[test]
    fn a_pressure_sweep_that_reclaims_nothing_is_a_warning() {
        let report = GcReport {
            dry_run: false,
            base: "develop".to_string(),
            include_unmerged: false,
            include_protected_workspaces: false,
            candidates: Vec::new(),
            kept: vec![GcKept {
                worktree: PathBuf::from("/work/busy"),
                branch: None,
                reason: "active process: cargo (pid 7)".into(),
            }],
            reclaimable_bytes: 0,
            removed: Vec::new(),
            failed: Vec::new(),
            reclaimed_bytes: 0,
            disk_space: evaluate(Vec::new()),
        };

        let record = record_from_result(Ok(report), "s".into(), "f".into(), "trigger", "develop");

        let json = serde_json::to_value(&record).expect("record");
        assert_eq!(json["outcome"], "no_reclaim");
        assert!(json["warning"]
            .as_str()
            .is_some_and(|text| text.contains("0 bytes")));
        assert_eq!(json["kept_by_reason"]["active process"], 1);
    }

    /// Legacy zero-byte runs must not keep presenting success after upgrade.
    #[test]
    fn a_legacy_record_without_an_outcome_falls_back_to_its_error() {
        let legacy = |error: &str| {
            format!(
                r#"{{"started_at":"s","finished_at":"f","trigger":"t","base":"develop",
                "candidates":0,"reclaimable_bytes":0,"reclaimed_bytes":0,"error":{error}}}"#
            )
        };
        let failed: BuildArtifactGcRecord =
            serde_json::from_str(&legacy(r#""git worktree list failed""#)).expect("failed record");
        let clean: BuildArtifactGcRecord =
            serde_json::from_str(&legacy("null")).expect("clean record");

        assert_eq!(failed.outcome(), BuildArtifactGcOutcome::EnumerationFailed);
        assert_ne!(clean.outcome(), BuildArtifactGcOutcome::Swept);
        let mut old_swept = clean;
        old_swept.recorded_outcome = Some(BuildArtifactGcOutcome::Swept);
        assert_ne!(old_swept.outcome(), BuildArtifactGcOutcome::Swept);
    }

    /// AC-3: runs are appended, never overwritten, and the last one is what
    /// the status reads back.
    #[test]
    fn records_append_and_the_last_one_is_read_back() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("project").join(RECORD_FILE_NAME);
        assert_eq!(last_record(&path), None);
        let record = |finished_at: &str| BuildArtifactGcRecord {
            started_at: finished_at.to_string(),
            finished_at: finished_at.to_string(),
            trigger: "disk space low".to_string(),
            base: "develop".to_string(),
            recorded_outcome: Some(BuildArtifactGcOutcome::Swept),
            candidates: 1,
            reclaimable_bytes: 1,
            reclaimed_bytes: 1,
            removed: Vec::new(),
            failed: Vec::new(),
            kept_by_reason: BTreeMap::new(),
            warning: None,
            error: None,
        };

        append_record(&path, &record("2026-09-15T10:00:00Z")).expect("first");
        append_record(&path, &record("2026-09-15T11:00:00Z")).expect("second");

        let text = std::fs::read_to_string(&path).expect("history");
        assert_eq!(text.lines().count(), 2);
        assert_eq!(last_record(&path), Some(record("2026-09-15T11:00:00Z")));
        assert_eq!(
            last_finished_at(&path),
            Some(
                DateTime::parse_from_rfc3339("2026-09-15T11:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
    }
}
