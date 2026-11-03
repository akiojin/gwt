//! Project-scoped Reports polling. Stable inputs do not decode stores or census.

use super::{project_dir, snapshot_from_inventory, status_path, PmCurrentSnapshot};
use crate::protocol::PmReportsVersion;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::SystemTime,
};

#[derive(Clone, Default)]
pub struct PmReportsData {
    pub reports: Vec<gwt_core::pm_report::PmReport>,
    pub current: Option<PmCurrentSnapshot>,
    pub error: Option<String>,
    refresh_at: Option<DateTime<Utc>>,
    processes: Vec<(u32, u64)>,
    uncertain: bool,
    payload: Option<PathBuf>,
}

impl PmReportsData {
    fn refresh_stale(&mut self, now: DateTime<Utc>) -> bool {
        let Some(current) = &mut self.current else {
            return false;
        };
        let stale = current
            .status
            .as_ref()
            .and_then(|status| status.stale_minutes(now));
        let changed = current.stale_minutes != stale;
        current.stale_minutes = stale;
        changed
    }

    fn same_content(&self, other: &Self) -> bool {
        self.reports == other.reports
            && self.error == other.error
            && match (&self.current, &other.current) {
                (Some(a), Some(b)) => {
                    a.status == b.status && a.stale_minutes == b.stale_minutes && a.facts == b.facts
                }
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Clone, PartialEq)]
pub(super) enum SourceStamp {
    File(
        PathBuf,
        Option<(u64, Option<SystemTime>, Option<SystemTime>)>,
    ),
    Monitor(Value),
    Update(Value),
    Preferences(String),
    Trees(u64),
}

/// Shared by viewers of one Project and serialized on the blocking pool.
pub struct PmReportsReader {
    paths: Option<(Vec<PathBuf>, PathBuf)>,
    trees: Option<TreeWatch>,
    preferences: Option<(SourceStamp, SourceStamp)>,
    inputs: Option<Vec<SourceStamp>>,
    data: Option<PmReportsData>,
    version: PmReportsVersion,
}

impl Default for PmReportsReader {
    fn default() -> Self {
        Self {
            paths: None,
            trees: None,
            preferences: None,
            inputs: None,
            data: None,
            version: PmReportsVersion {
                epoch: uuid::Uuid::new_v4().to_string(),
                revision: 0,
            },
        }
    }
}

impl PmReportsReader {
    pub fn poll(
        &mut self,
        repo: &Path,
        update: Option<&gwt_core::update::UpdateState>,
        version: Option<&PmReportsVersion>,
    ) -> Option<(PmReportsVersion, PmReportsData)> {
        if self.paths.is_none() {
            let home = gwt_core::paths::gwt_home();
            self.paths = Some((
                vec![
                    gwt_core::pm_report::reports_path(repo),
                    gwt_core::user_instruction::instructions_path(repo),
                    home.join("pm-peer-messages.json"),
                    status_path(repo),
                    gwt_core::coordination::coordination_escalations_path(repo),
                    project_dir(repo).join(gwt_git::PR_INVENTORY_CACHE_FILE),
                    home.join("config.toml"),
                    gwt_core::update::pending_update_manifest_path(),
                ],
                crate::issue_monitor_prefs_path_for_repo_path(repo),
            ));
            self.trees = Some(TreeWatch::new(vec![
                gwt_core::paths::gwt_sessions_dir(),
                crate::issue_cache::issue_cache_root_for_repo_path_or_detached(repo),
            ]));
        }
        let paths = self.paths.as_ref()?;
        let mut inputs = Vec::new();
        let mut readable = paths.0.iter().all(|path| stamp(path, &mut inputs).is_ok());
        match preferences_input(&paths.1, &mut self.preferences) {
            Ok(input) => inputs.push(input),
            Err(_) => readable = false,
        }
        readable &= self.trees.as_mut()?.observe(&mut inputs);
        if let Some(data) = &self.data {
            if let Some(payload) = &data.payload {
                readable &= stamp(payload, &mut inputs).is_ok();
            }
            readable &= !data.uncertain
                && data.processes.iter().all(|(pid, started)| {
                    crate::process::host_process_start_time(*pid) == Some(*started)
                });
        }
        // The daemon can publish without changing prefs. Observe only the small
        // fields this view uses; scan/heartbeat clocks do not invalidate it.
        match crate::daemon_publisher::read_issue_monitor_status(repo) {
            Ok(status) => inputs.push(SourceStamp::Monitor(status.map(|status| serde_json::json!({
                "queue": status["queue"], "needs_human": status["needs_human"],
                "activity": status["inbox"].as_array().map(|rows| rows.iter().map(|row| (&row["issue_number"], &row["last_activity_at"])).collect::<Vec<_>>()),
            })).unwrap_or(Value::Null))),
            Err(_) => readable = false,
        }
        inputs.push(SourceStamp::Update(
            serde_json::to_value(update).unwrap_or(Value::Null),
        ));
        self.poll_with(readable.then_some(inputs), version, Utc::now(), || {
            load(repo, update)
        })
    }

    pub(super) fn poll_with(
        &mut self,
        inputs: Option<Vec<SourceStamp>>,
        version: Option<&PmReportsVersion>,
        now: DateTime<Utc>,
        load: impl FnOnce() -> PmReportsData,
    ) -> Option<(PmReportsVersion, PmReportsData)> {
        let due = self
            .data
            .as_ref()
            .and_then(|data| data.refresh_at)
            .is_some_and(|at| now >= at);
        let mut changed = false;
        if self.data.is_none() || inputs.is_none() || self.inputs != inputs || due {
            let mut next = load();
            next.refresh_stale(now);
            changed = self
                .data
                .as_ref()
                .is_none_or(|old| !old.same_content(&next));
            self.inputs = if next.error.is_none() { inputs } else { None };
            self.data = Some(next);
        }
        let data = self.data.as_mut()?;
        // Advancing the stale label does not require reading any file.
        changed |= data.refresh_stale(now);
        if changed {
            // Keep every revision exactly representable by JavaScript numbers.
            const MAX_SAFE_REVISION: u64 = (1_u64 << 53) - 1;
            if self.version.revision >= MAX_SAFE_REVISION {
                self.version.epoch = uuid::Uuid::new_v4().to_string();
                self.version.revision = 0;
            }
            self.version.revision += 1;
        }
        // Enqueueing is not an acknowledgement: a dropped response must be
        // retried until the client supplies the version it actually applied.
        (version != Some(&self.version)).then(|| (self.version.clone(), data.clone()))
    }
}

fn stamp(path: &Path, result: &mut Vec<SourceStamp>) -> io::Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    result.push(SourceStamp::File(
        path.to_owned(),
        metadata
            .as_ref()
            .map(|m| (m.len(), m.modified().ok(), m.created().ok())),
    ));
    Ok(())
}

fn preferences_input(
    path: &Path,
    cached: &mut Option<(SourceStamp, SourceStamp)>,
) -> io::Result<SourceStamp> {
    let mut stamps = Vec::new();
    stamp(path, &mut stamps)?;
    let stamp = stamps.remove(0);
    if let Some((old, semantic)) = cached {
        if *old == stamp {
            return Ok(semantic.clone());
        }
    }
    let semantic = match fs::read(path) {
        Ok(bytes) => {
            let mut value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            // These clocks never affect the Current card. Keep every other
            // preference, including runtime counts/windows and queue decisions.
            if let Some(object) = value.as_object_mut() {
                object.remove("last_scan_at");
                if let Some(hosts) = object
                    .get_mut("monitor_runtime_counts")
                    .and_then(Value::as_object_mut)
                {
                    for host in hosts.values_mut().filter_map(Value::as_object_mut) {
                        host.remove("observed_at");
                    }
                }
            }
            SourceStamp::Preferences(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&value).map_err(io::Error::other)?)
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => stamp.clone(),
        Err(error) => return Err(error),
    };
    *cached = Some((stamp, semantic.clone()));
    Ok(semantic)
}

struct TreeWatch {
    roots: Vec<PathBuf>,
    stamps: Vec<SourceStamp>,
    revision: Arc<AtomicU64>,
    healthy: Arc<AtomicBool>,
    watcher: Option<notify::RecommendedWatcher>,
}

impl TreeWatch {
    fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            stamps: Vec::new(),
            revision: Arc::new(AtomicU64::new(0)),
            healthy: Arc::new(AtomicBool::new(false)),
            watcher: None,
        }
    }

    fn observe(&mut self, inputs: &mut Vec<SourceStamp>) -> bool {
        use notify::Watcher;
        let mut stamps = Vec::new();
        if self
            .roots
            .iter()
            .any(|root| stamp(root, &mut stamps).is_err())
        {
            return false;
        }
        if self.watcher.is_none() || !self.healthy.load(Ordering::SeqCst) || self.stamps != stamps {
            self.watcher = None;
            self.healthy.store(true, Ordering::SeqCst);
            let revision = Arc::clone(&self.revision);
            let healthy = Arc::clone(&self.healthy);
            let roots = self.roots.clone();
            let watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if match event {
                        Ok(event) => relevant_tree_change(&event, &roots),
                        Err(_) => {
                            healthy.store(false, Ordering::SeqCst);
                            true
                        }
                    } {
                        revision.fetch_add(1, Ordering::SeqCst);
                    }
                });
            let Ok(mut watcher) = watcher else {
                return false;
            };
            for root in &self.roots {
                // Watching the nearest existing ancestor also catches creation
                // of an initially absent cache and replacement of its root.
                let Some(parent) = root.ancestors().find(|path| path.is_dir()) else {
                    return false;
                };
                if watcher
                    .watch(parent, notify::RecursiveMode::Recursive)
                    .is_err()
                {
                    return false;
                }
            }
            self.watcher = Some(watcher);
            self.stamps = stamps;
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
        inputs.push(SourceStamp::Trees(self.revision.load(Ordering::SeqCst)));
        self.healthy.load(Ordering::SeqCst)
    }
}

fn relevant_tree_change(event: &notify::Event, roots: &[PathBuf]) -> bool {
    !event.kind.is_access()
        && (event.paths.is_empty()
            || event.paths.iter().any(|path| {
                roots
                    .iter()
                    .any(|root| path.starts_with(root) || root.starts_with(path))
            }))
}

fn load(repo: &Path, update: Option<&gwt_core::update::UpdateState>) -> PmReportsData {
    let now = Utc::now();
    let mut data = PmReportsData::default();
    match gwt_core::pm_report::load_reports_with_deadline(repo) {
        Ok((reports, deadline)) => {
            data.reports = reports;
            data.refresh_at = deadline;
        }
        Err(error) => data.error = Some(format!("PM reports could not be loaded: {error}")),
    }
    let inventory =
        crate::session_inventory::observe_sessions(repo, &gwt_core::paths::gwt_sessions_dir());
    data.processes = inventory
        .sessions
        .iter()
        .map(|session| (session.child_pid, session.child_started_at))
        .collect();
    data.uncertain = !inventory.uncertainties.is_empty();
    match snapshot_from_inventory(repo, update, now, &inventory) {
        Ok(current) => data.current = Some(current),
        Err(error) => data.error = Some(data.error.unwrap_or(error)),
    }
    let settings =
        gwt_config::Settings::load_from_path(&gwt_core::paths::gwt_home().join("config.toml"))
            .unwrap_or_default();
    let cache = gwt_git::pr_status::PrInventoryCache::load(
        &project_dir(repo).join(gwt_git::PR_INVENTORY_CACHE_FILE),
    );
    let refresh = cache.fetched_at.unwrap_or(now)
        + chrono::Duration::seconds(
            settings.pr_inventory.cache_ttl_secs.min(i64::MAX as u64) as i64
        );
    // Retry a failed/budget-denied refresh on the existing five-second cadence.
    let refresh = refresh.max(now + chrono::Duration::seconds(5));
    data.refresh_at = Some(
        data.refresh_at
            .map_or(refresh, |deadline| deadline.min(refresh)),
    );
    // Track the referenced path even while absent: its later creation must
    // invalidate the "no pending update" projection without a manifest rewrite.
    data.payload = fs::read(gwt_core::update::pending_update_manifest_path())
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<gwt_core::update::PendingUpdateManifest>(&bytes).ok()
        })
        .map(|manifest| match manifest.payload {
            gwt_core::update::PreparedPayload::PortableBinary { path }
            | gwt_core::update::PreparedPayload::Installer { path, .. } => path,
        });
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_resent_until_the_client_acknowledges_them() {
        let mut reader = PmReportsReader::default();
        let now = Utc::now();
        let (version, _) = reader
            .poll_with(Some(vec![]), None, now, PmReportsData::default)
            .unwrap();
        let (retry_version, _) = reader
            .poll_with(Some(vec![]), None, now, || {
                panic!("an unacknowledged retry reuses the cached snapshot")
            })
            .expect("sending a snapshot does not acknowledge its application");
        assert_eq!(retry_version, version);
        assert!(reader
            .poll_with(Some(vec![]), Some(&version), now, || panic!(
                "an acknowledged poll reuses the cached snapshot"
            ))
            .is_none());
    }

    #[test]
    fn a_new_reader_or_revision_limit_starts_a_new_epoch() {
        let now = Utc::now();
        let (old, _) = PmReportsReader::default()
            .poll_with(Some(vec![]), None, now, PmReportsData::default)
            .unwrap();
        let mut reader = PmReportsReader::default();
        let (first, _) = reader
            .poll_with(Some(vec![]), Some(&old), now, PmReportsData::default)
            .unwrap();
        assert_ne!(first.epoch, old.epoch);
        assert_eq!(first.revision, 1);
        reader.version.revision = (1_u64 << 53) - 1;
        let (rotated, _) = reader
            .poll_with(None, Some(&first), now, || PmReportsData {
                error: Some("unavailable".to_owned()),
                ..Default::default()
            })
            .unwrap();
        assert_ne!(rotated.epoch, first.epoch);
        assert_eq!(rotated.revision, 1);
    }

    #[test]
    fn preferences_ignore_observation_clocks_but_track_counts_and_decisions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("prefs.json");
        let mut value = serde_json::json!({"last_scan_at":"first", "priority_order":[1,2],
            "monitor_runtime_counts":{"42":{"observed_at":"first", "counts":{"1":1}, "windows":{"pane":1}}}});
        fs::write(&path, value.to_string()).unwrap();
        let first = preferences_input(&path, &mut None).unwrap();
        value["last_scan_at"] = serde_json::json!("second clock value");
        value["monitor_runtime_counts"]["42"]["observed_at"] = serde_json::json!("later");
        fs::write(&path, value.to_string()).unwrap();
        assert!(first == preferences_input(&path, &mut None).unwrap());
        value["monitor_runtime_counts"]["42"]["counts"]["1"] = serde_json::json!(2);
        fs::write(&path, value.to_string()).unwrap();
        assert!(first != preferences_input(&path, &mut None).unwrap());
        value["priority_order"] = serde_json::json!([2, 1]);
        fs::write(&path, value.to_string()).unwrap();
        assert!(first != preferences_input(&path, &mut None).unwrap());
    }

    #[test]
    fn tree_notifications_ignore_our_reads_and_notice_changes_below_roots() {
        let root = PathBuf::from("sessions");
        let roots = vec![root.clone()];
        let changed =
            notify::Event::new(notify::EventKind::Any).add_path(root.join("runtime/42/pane.json"));
        assert!(relevant_tree_change(&changed, &roots));
        let read = notify::Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
            .add_path(root.join("pane.toml"));
        assert!(!relevant_tree_change(&read, &roots));
        let outside =
            notify::Event::new(notify::EventKind::Any).add_path(PathBuf::from("unrelated"));
        assert!(!relevant_tree_change(&outside, &roots));
    }

    #[cfg(unix)]
    #[test]
    fn metadata_tracks_the_store_target_through_a_symlink() {
        let stores = tempfile::tempdir().unwrap();
        let targets = tempfile::tempdir().unwrap();
        let target = targets.path().join("reports.jsonl");
        fs::write(&target, "first").unwrap();
        std::os::unix::fs::symlink(&target, stores.path().join("reports.jsonl")).unwrap();
        let fingerprint = || {
            let mut inputs = Vec::new();
            stamp(&stores.path().join("reports.jsonl"), &mut inputs).unwrap();
            inputs
        };
        let first = fingerprint();
        fs::write(&target, "second report").unwrap();
        assert!(first != fingerprint());
    }

    #[test]
    fn clock_changes_refresh_deadlines_and_advance_stale_without_store_reads() {
        let now = Utc::now();
        let status = serde_json::from_value(serde_json::json!({
            "items": [{"kind":"waiting", "text":"CI", "started_at":now}],
            "next_trigger":"CI", "updated_at":now - chrono::Duration::minutes(15),
            "stale_after_minutes":15,
        }))
        .unwrap();
        let data = PmReportsData {
            current: Some(PmCurrentSnapshot {
                observed_at: now,
                status: Some(status),
                stale_minutes: Some(15),
                facts: Default::default(),
            }),
            refresh_at: Some(now + chrono::Duration::minutes(2)),
            ..Default::default()
        };
        let mut reader = PmReportsReader::default();
        let (version, _) = reader
            .poll_with(Some(vec![]), None, now, || data.clone())
            .unwrap();
        let later = reader
            .poll_with(
                Some(vec![]),
                Some(&version),
                now + chrono::Duration::minutes(1),
                || panic!("stale clock uses cached status"),
            )
            .unwrap();
        assert_eq!(later.0.epoch, version.epoch);
        assert_eq!(later.0.revision, version.revision + 1);
        assert_eq!(later.1.current.unwrap().stale_minutes, Some(16));
        let mut refreshed = false;
        reader.poll_with(
            Some(vec![]),
            Some(&later.0),
            now + chrono::Duration::minutes(2),
            || {
                refreshed = true;
                data.clone()
            },
        );
        assert!(
            refreshed,
            "peer/PR expiry must reload even with stable file stamps"
        );
    }

    #[test]
    fn metadata_changes_reload_but_observation_time_alone_is_not_resent() {
        let now = Utc::now();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime/host/session.json");
        let fingerprint = || {
            let mut inputs = vec![];
            stamp(&path, &mut inputs).unwrap();
            Some(inputs)
        };
        let mut reader = PmReportsReader::default();
        let data = PmReportsData {
            current: Some(PmCurrentSnapshot {
                observed_at: now,
                status: None,
                stale_minutes: None,
                facts: Default::default(),
            }),
            ..Default::default()
        };
        let (version, _) = reader
            .poll_with(fingerprint(), None, now, || data.clone())
            .unwrap();
        assert!(reader
            .poll_with(fingerprint(), Some(&version), now, || panic!(
                "stable metadata must not decode files"
            ))
            .is_none());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not parsed by the fingerprint").unwrap();
        let mut loaded = false;
        assert!(reader
            .poll_with(fingerprint(), Some(&version), now, || {
                loaded = true;
                let mut next = data.clone();
                next.current.as_mut().unwrap().observed_at = now + chrono::Duration::seconds(5);
                next
            })
            .is_none());
        assert!(loaded);
        fs::remove_file(&path).unwrap();
        let mut removed = false;
        reader.poll_with(fingerprint(), Some(&version), now, || {
            removed = true;
            data
        });
        assert!(removed);
    }
}
