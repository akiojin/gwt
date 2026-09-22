//! SPEC-2359 W-16 (FR-387/FR-388): project-level work events ingest
//! orchestrator.
//!
//! Collects legacy `.gwt/work/events.jsonl` and canonical immutable shards
//! below `.gwt/work/events/` from every reachable source —
//! local worktree filesystems (the base/main checkout included) and fetched
//! `origin/*` refs (checkout-free blob reads) — and funnels each through the
//! idempotent gwt-core intake into the home works projection. A fingerprint
//! cache (`work-events-intake/`) skips unchanged sources; deleting it is
//! always safe (dedup is event-id based, SC-260). After first validation,
//! immutable local shards and the frozen legacy logs use size/mtime/container
//! metadata to avoid payload I/O on the 30-second unchanged poll; metadata
//! changes force revalidation. The cache is kept per source group — one per
//! worktree, one per origin ref — so a group whose snapshot (worktree
//! metadata digest, ref commit) is unchanged is neither re-derived nor
//! rewritten (#4397).
//!
//! Git blob contents are OID-deduplicated and read in one `cat-file --batch`;
//! tree enumeration is checkout-free and unique-commit deduplicated. Callers
//! run this off the UI thread.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use gwt_core::work_events_intake::{
    content_fingerprint, ingest_work_event_sources_with_local_path, ingest_work_events_sources,
    rebuild_work_events_with_shared_loader, work_events_intake_group_of, SharedWorkEventsSource,
    WorkEventsIntakeStore,
};
#[cfg(test)]
use gwt_core::work_events_intake::{
    ingest_work_events_content, load_work_events_intake_state, save_work_events_intake_state,
};
use gwt_core::workspace_projection::WorkspaceExecutionContainerRef;
use sha2::{Digest, Sha256};

/// Where one ingested chunk of content came from (cache key prefix).
const SOURCE_WORKTREE: &str = "worktree:";
const SOURCE_REF: &str = "ref:";
const SOURCE_LOCAL_LIFECYCLE: &str = "local-lifecycle:";
const SOURCE_LIST: &str = "source-list:v1";

/// Bump this when projection-time source metadata changes. Older cache entries
/// used only the raw content/blob fingerprint, which would skip the repair pass.
const SOURCE_CONTEXT_FINGERPRINT_VERSION: &str = "source-context-v10-legacy-log-metadata-cache";

/// Tree path of the persistent core inside a worktree / commit.
const EVENTS_TREE_PATH: &str = ".gwt/work/events.jsonl";
const EVENTS_TREE_DIR: &str = ".gwt/work/events";

#[derive(Debug, Clone, Copy)]
enum WorkEventsSourceKind {
    Legacy,
    Shard,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkEventsIngestSummary {
    /// Sources whose content was read and offered to the intake.
    pub sources_ingested: usize,
    /// Sources skipped because their fingerprint was already current.
    pub sources_skipped: usize,
    /// Events applied across all ingested sources.
    pub events_applied: usize,
    /// The projection was rebuilt with the current fold semantics.
    pub projection_rebuilt: bool,
    /// Source fingerprints derived this pass: only groups whose snapshot
    /// changed, every group on a rebuild or a migration (#4397).
    pub sources_rederived: usize,
    /// Fingerprints the intake state holds after this pass.
    pub state_sources: usize,
    /// Bytes the intake state occupies on disk after this pass.
    pub state_bytes: u64,
    /// Bytes this pass wrote to the intake state.
    pub state_bytes_written: u64,
}

impl WorkEventsIngestSummary {
    pub fn changed(&self) -> bool {
        self.events_applied > 0 || self.projection_rebuilt
    }
}

#[derive(Debug)]
struct PendingWorkEventsSource {
    key: String,
    fingerprint: String,
    content: Arc<str>,
    container: Option<WorkspaceExecutionContainerRef>,
    reload_from_worktree: bool,
}

type SourceFingerprints = Vec<(String, String)>;
type ReloadedWorkEventsSources = (Vec<SharedWorkEventsSource>, SourceFingerprints);

fn read_work_event_source(path: &Path, kind: WorkEventsSourceKind) -> gwt_core::Result<Arc<str>> {
    let content = std::fs::read(path)?;
    work_event_source_content(path, kind, &content)
}

fn work_event_source_content(
    path: &Path,
    kind: WorkEventsSourceKind,
    content: &[u8],
) -> gwt_core::Result<Arc<str>> {
    if matches!(kind, WorkEventsSourceKind::Shard) {
        validate_work_event_shard(path, content)?;
    }
    std::str::from_utf8(content)
        .map(Arc::<str>::from)
        .map_err(|error| {
            gwt_core::GwtError::Other(format!(
                "work event source {} is not UTF-8: {error}",
                path.display()
            ))
        })
}

fn shared_ref_source_content<F>(
    cache: &mut HashMap<(String, String), Result<Arc<str>, String>>,
    oid: &str,
    path: &str,
    kind: WorkEventsSourceKind,
    bytes: &[u8],
    validate_shard: F,
) -> gwt_core::Result<Arc<str>>
where
    F: FnOnce(&Path, &[u8]) -> gwt_core::Result<()>,
{
    let key = (oid.to_string(), path.to_string());
    if let Some(result) = cache.get(&key) {
        return result
            .as_ref()
            .map(Arc::clone)
            .map_err(|error| gwt_core::GwtError::Other(error.clone()));
    }
    let result = (|| {
        if matches!(kind, WorkEventsSourceKind::Shard) {
            validate_shard(Path::new(path), bytes).map_err(|error| error.to_string())?;
        }
        std::str::from_utf8(bytes)
            .map(Arc::<str>::from)
            .map_err(|error| format!("work event source {path} is not UTF-8: {error}"))
    })();
    cache.insert(key, result.clone());
    result.map_err(gwt_core::GwtError::Other)
}

fn validate_work_event_shard(path: &Path, content: &[u8]) -> gwt_core::Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            gwt_core::GwtError::Other(format!(
                "work event shard has an invalid filename: {}",
                path.display()
            ))
        })?;
    let Some(hash) = name.strip_suffix(".jsonl") else {
        return Err(gwt_core::GwtError::Other(format!(
            "work event shard has an invalid filename: {}",
            path.display()
        )));
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(gwt_core::GwtError::Other(format!(
            "work event shard has an invalid filename: {}",
            path.display()
        )));
    }
    if content.last() != Some(&b'\n') || content.iter().filter(|byte| **byte == b'\n').count() != 1
    {
        return Err(gwt_core::GwtError::Other(format!(
            "work event shard must contain exactly one newline-terminated event: {}",
            path.display()
        )));
    }
    let payload = &content[..content.len() - 1];
    gwt_core::workspace_projection::decode_workspace_work_event_line(payload).map_err(|error| {
        gwt_core::GwtError::Other(format!(
            "work event shard has an incompatible event schema {}: {error}",
            path.display()
        ))
    })?;
    let event: serde_json::Value = serde_json::from_slice(payload).map_err(|error| {
        gwt_core::GwtError::Other(format!(
            "work event shard contains invalid JSON {}: {error}",
            path.display()
        ))
    })?;
    let id = event
        .as_object()
        .and_then(|object| object.get("id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            gwt_core::GwtError::Other(format!(
                "work event shard payload has no string id: {}",
                path.display()
            ))
        })?;
    let expected = format!("{:x}", Sha256::digest(id.as_bytes()));
    if hash != expected {
        return Err(gwt_core::GwtError::Other(format!(
            "work event shard filename does not match payload id: {}",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let grandparent = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    match (parent, grandparent) {
        (Some("events"), _) => {}
        (Some(bucket), Some("events")) if bucket == &hash[..2] => {}
        (Some(bucket), Some("events")) => {
            return Err(gwt_core::GwtError::Other(format!(
                "work event shard bucket {bucket} does not match digest {}: {}",
                &hash[..2],
                path.display()
            )))
        }
        _ => {
            return Err(gwt_core::GwtError::Other(format!(
                "work event shard is outside the canonical or legacy event store layout: {}",
                path.display()
            )))
        }
    }
    Ok(())
}

fn work_event_source_kind_for_ref_path(path: &str) -> gwt_core::Result<WorkEventsSourceKind> {
    if path == EVENTS_TREE_PATH {
        return Ok(WorkEventsSourceKind::Legacy);
    }
    let relative = path
        .strip_prefix(&format!("{EVENTS_TREE_DIR}/"))
        .ok_or_else(|| {
            gwt_core::GwtError::Other(format!(
                "work event source is outside {EVENTS_TREE_DIR}: {path}"
            ))
        })?;
    let parts = relative.split('/').collect::<Vec<_>>();
    let valid_flat = parts.len() == 1;
    let valid_bucketed = parts.len() == 2
        && parts[0].len() == 2
        && parts[0]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid_flat && !valid_bucketed {
        return Err(gwt_core::GwtError::Other(format!(
            "work event shard has an unsupported nesting layout: {path}"
        )));
    }
    Ok(WorkEventsSourceKind::Shard)
}

fn load_pending_sources_for_rebuild(
    pending_sources: &[PendingWorkEventsSource],
    worktree_entries: &[gwt::worktree_inventory::WorktreeEntry],
) -> gwt_core::Result<ReloadedWorkEventsSources> {
    let mut contents = Vec::with_capacity(pending_sources.len());
    let mut fingerprints = Vec::with_capacity(pending_sources.len());
    for source in pending_sources {
        if source.reload_from_worktree {
            continue;
        }
        contents.push(SharedWorkEventsSource::new(
            Arc::clone(&source.content),
            source.container.clone(),
        ));
        fingerprints.push((source.key.clone(), source.fingerprint.clone()));
    }

    // Re-scan the already enumerated worktree roots after the projection lock
    // is taken. This catches an immutable shard atomically published between
    // the initial source scan and intake without paying for a second
    // `git worktree list` process.
    for source in worktree_event_sources(worktree_entries)? {
        // The scan took metadata before this content read: a write racing
        // the read leaves the older fingerprint behind, so the next pass
        // reads the source again.
        let fingerprint = source_fingerprint(&source.metadata, source.container.as_ref());
        let content = read_work_event_source(&source.events_path, source.kind)?;
        fingerprints.push((source.key(), fingerprint));
        contents.push(SharedWorkEventsSource::new(content, source.container));
    }
    Ok((contents, fingerprints))
}

/// Paths-injected ingest (#3022): all writes go to `work_items_path` /
/// `state_path`. Source discovery/read failures are logged and skipped during
/// incremental intake. An authoritative rebuild is deferred unless every
/// discovered source was readable, so a partial snapshot cannot erase history.
#[cfg(test)]
pub fn ingest_project_work_events_paths(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
) -> WorkEventsIngestSummary {
    ingest_project_work_events_paths_with_inventory(project_root, work_items_path, state_path, None)
}

/// Issue #4378 AC-1: `inventory` is a worktree listing the caller already
/// holds (startup lists once and shares it); `None` lists the worktrees here.
pub fn ingest_project_work_events_paths_with_inventory(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
    inventory: Option<&[gwt::worktree_inventory::WorktreeEntry]>,
) -> WorkEventsIngestSummary {
    let started = std::time::Instant::now();
    let summary = ingest_project_work_events_paths_inner(
        project_root,
        work_items_path,
        state_path,
        inventory,
        || {},
        |_| {},
    );
    record_ingest_perf(&summary, started.elapsed());
    summary
}

/// Issue #4397 AC-4: the route total plus what the pass cost the intake state.
fn record_ingest_perf(summary: &WorkEventsIngestSummary, elapsed: std::time::Duration) {
    use gwt::perf::{global, PerfRoute, PerfUnit};

    global::record_route(PerfRoute::WorkEventsIngest, elapsed);
    for (metric, value, unit) in [
        (
            "state_sources",
            summary.state_sources as f64,
            PerfUnit::Count,
        ),
        ("state_bytes", summary.state_bytes as f64, PerfUnit::Bytes),
        (
            "sources_rederived",
            summary.sources_rederived as f64,
            PerfUnit::Count,
        ),
        (
            "state_bytes_written",
            summary.state_bytes_written as f64,
            PerfUnit::Bytes,
        ),
    ] {
        global::record_route_metric(PerfRoute::WorkEventsIngest, metric, value, unit);
    }
}

#[cfg(test)]
fn ingest_project_work_events_paths_with_before_intake<F>(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
    before_intake: F,
) -> WorkEventsIngestSummary
where
    F: FnOnce(),
{
    ingest_project_work_events_paths_inner(
        project_root,
        work_items_path,
        state_path,
        None,
        before_intake,
        |_| {},
    )
}

#[cfg(test)]
fn ingest_project_work_events_paths_with_source_read_hook<R>(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
    before_source_read: R,
) -> WorkEventsIngestSummary
where
    R: FnMut(&Path),
{
    ingest_project_work_events_paths_inner(
        project_root,
        work_items_path,
        state_path,
        None,
        || {},
        before_source_read,
    )
}

fn ingest_project_work_events_paths_inner<F, R>(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
    inventory: Option<&[gwt::worktree_inventory::WorktreeEntry]>,
    before_intake: F,
    mut before_source_read: R,
) -> WorkEventsIngestSummary
where
    F: FnOnce(),
    R: FnMut(&Path),
{
    let mut phases =
        gwt::perf::global::RoutePhaseClock::start(gwt::perf::PerfRoute::WorkEventsIngest);
    let mut summary = WorkEventsIngestSummary::default();
    let mut store = WorkEventsIntakeStore::open(state_path);
    summary.state_sources = store.source_count();
    summary.state_bytes = store.stored_bytes();
    phases.mark("state_load");
    let projection_requires_rebuild =
        match gwt_core::workspace_projection::load_workspace_work_items_from_path(work_items_path) {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(gwt_core::GwtError::JsonDecode {
                kind: gwt_core::JsonDecodeKind::Malformed,
                message: error,
                ..
            }) => {
                tracing::warn!(
                    %error,
                    path = %work_items_path.display(),
                    "work events ingest: corrupt projection requires rebuild"
                );
                true
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    path = %work_items_path.display(),
                    "work events ingest: projection read failed"
                );
                return summary;
            }
        };
    let mut rebuild_required = projection_requires_rebuild
        || !store.projection_is_current(SOURCE_CONTEXT_FINGERPRINT_VERSION);
    let mut pending_sources = Vec::new();
    let mut source_discovery_failed = false;
    // Issue #4397: the state is kept per source group (one per worktree, one
    // per origin ref). A group whose snapshot matches the stored one is
    // skipped whole; only the others are derived source by source.
    let mut scanned_groups = BTreeMap::<String, ScannedGroup>::new();
    // Every discovered group: its snapshot, and whether it holds a source.
    let mut discovered_groups = BTreeMap::<String, (String, bool)>::new();
    let mut vanished_groups = Vec::new();

    // 1) Local worktree filesystems (base/main checkout included): committed
    //    or not, the working copy is the freshest view of each branch's log.
    let listed;
    let worktree_entries = match inventory {
        Some(entries) => entries,
        None => {
            listed = match gwt::worktree_inventory::enumerate_worktrees(project_root, None) {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(%error, "work events ingest: worktree enumeration failed");
                    source_discovery_failed = true;
                    Vec::new()
                }
            };
            &listed
        }
    };
    phases.mark("worktree_list");
    let local_groups = match worktree_event_sources(worktree_entries) {
        Ok(sources) => local_group_scans(sources),
        Err(error) => {
            tracing::warn!(%error, "work events ingest: worktree event source discovery failed");
            source_discovery_failed = true;
            Vec::new()
        }
    };
    phases.mark("worktree_scan");
    for group in &local_groups {
        discovered_groups.insert(group.name.clone(), (group.snapshot.clone(), true));
    }
    let vanished_local = store
        .group_names()
        .filter(|name| name.starts_with(SOURCE_WORKTREE) && !discovered_groups.contains_key(*name))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for name in vanished_local {
        rebuild_required |= store.group(&name).is_some_and(|group| group.sources > 0);
        vanished_groups.push(name);
    }

    let mut new_local_keys = HashSet::new();
    for group in &local_groups {
        let verified = store
            .group(&group.name)
            .and_then(|stored| stored.snapshot.as_deref())
            == Some(group.snapshot.as_str());
        if verified && !rebuild_required {
            summary.sources_skipped += group.sources.len();
            continue;
        }
        let discovered = group
            .sources
            .iter()
            .map(|(source, key)| {
                (
                    key.clone(),
                    source_fingerprint_in_context(&source.metadata, &group.context),
                )
            })
            .collect::<BTreeMap<_, _>>();
        summary.sources_rederived += discovered.len();
        if !rebuild_required {
            match group_additions(&mut store, &group.name, &discovered) {
                Some(keys) => {
                    summary.sources_skipped += discovered.len() - keys.len();
                    new_local_keys.extend(keys.into_iter().map(str::to_owned));
                }
                None => rebuild_required = true,
            }
        }
        scanned_groups.insert(
            group.name.clone(),
            ScannedGroup {
                snapshot: group.snapshot.clone(),
                discovered,
            },
        );
    }

    for group in &local_groups {
        let scanned = scanned_groups.get(&group.name);
        for (source, key) in &group.sources {
            if !rebuild_required && !new_local_keys.contains(key) {
                continue;
            }
            let fingerprint = scanned
                .and_then(|scanned| scanned.discovered.get(key).cloned())
                .unwrap_or_else(|| source_fingerprint_in_context(&source.metadata, &group.context));
            before_source_read(&source.events_path);
            let content = match read_work_event_source(&source.events_path, source.kind) {
                Ok(content) => content,
                Err(error) => {
                    tracing::warn!(%error, path = %source.events_path.display(), "work events ingest: worktree shard read failed");
                    source_discovery_failed = true;
                    continue;
                }
            };
            pending_sources.push(PendingWorkEventsSource {
                key: key.clone(),
                fingerprint,
                content,
                container: source.container.clone(),
                reload_from_worktree: true,
            });
        }
    }
    phases.mark("worktree_read");

    // 2) Fetched origin/* refs — checkout-free blob reads. Close-kind
    //    filtering inside the intake keeps foreign close state out (FR-384)
    //    and lenient parsing guards against contaminated logs (#3023). A ref
    //    whose commit is unchanged is not read at all.
    match gwt_git::refs::list_origin_refs_with_commit(project_root) {
        Ok(refs) => {
            let ref_scans = refs
                .iter()
                .map(|(refname, commit)| RefScan::new(refname, commit))
                .collect::<Vec<_>>();
            for scan in &ref_scans {
                let holds_sources = store
                    .group(&scan.group)
                    .is_some_and(|group| group.sources > 0);
                discovered_groups
                    .insert(scan.group.clone(), (scan.snapshot.clone(), holds_sources));
            }
            let vanished_refs = store
                .group_names()
                .filter(|name| {
                    name.starts_with(SOURCE_REF) && !discovered_groups.contains_key(*name)
                })
                .map(str::to_owned)
                .collect::<Vec<_>>();
            for name in vanished_refs {
                rebuild_required |= store.group(&name).is_some_and(|group| group.sources > 0);
                vanished_groups.push(name);
            }

            let mut batch = ref_scans
                .iter()
                .filter(|scan| {
                    rebuild_required
                        || store
                            .group(&scan.group)
                            .and_then(|group| group.snapshot.as_deref())
                            != Some(scan.snapshot.as_str())
                })
                .collect::<Vec<_>>();
            let mut result = (!batch.is_empty())
                .then(|| read_ref_batch(project_root, &batch, &mut store, rebuild_required));
            if let Some(Ok(read)) = &result {
                if read.requires_rebuild {
                    rebuild_required = true;
                    if batch.len() < ref_scans.len() {
                        // The unchanged refs' payloads are needed as well.
                        batch = ref_scans.iter().collect();
                        result = Some(read_ref_batch(project_root, &batch, &mut store, true));
                    }
                }
            }
            let batched = batch
                .iter()
                .map(|scan| scan.group.as_str())
                .collect::<HashSet<_>>();
            for scan in &ref_scans {
                if !batched.contains(scan.group.as_str()) {
                    summary.sources_skipped +=
                        store.group(&scan.group).map_or(0, |group| group.sources);
                }
            }

            match result {
                None => {}
                Some(Ok(RefBatch {
                    blobs_by_ref,
                    discovered: discovered_by_ref,
                    new_keys,
                    ..
                })) => {
                    let mut shared_content_by_oid_path =
                        HashMap::<(String, String), Result<Arc<str>, String>>::new();
                    for ((scan, blobs), discovered) in
                        batch.iter().zip(blobs_by_ref).zip(discovered_by_ref)
                    {
                        summary.sources_rederived += discovered.len();
                        discovered_groups.insert(
                            scan.group.clone(),
                            (scan.snapshot.clone(), !discovered.is_empty()),
                        );
                        for blob in blobs {
                            if is_work_event_writer_temp_residue(Path::new(&blob.path)) {
                                continue;
                            }
                            let kind = match work_event_source_kind_for_ref_path(&blob.path) {
                                Ok(kind) => kind,
                                Err(error) => {
                                    tracing::warn!(%error, path = %blob.path, "work events ingest: invalid ref event source path");
                                    source_discovery_failed = true;
                                    continue;
                                }
                            };
                            let key = format!("{SOURCE_REF}{}:{}", scan.refname, blob.path);
                            let Some(fingerprint) = discovered.get(&key).cloned() else {
                                continue;
                            };
                            let bytes = match blob.content {
                                Some(bytes) if rebuild_required || new_keys.contains(&key) => bytes,
                                _ => {
                                    summary.sources_skipped += 1;
                                    continue;
                                }
                            };
                            let content = match shared_ref_source_content(
                                &mut shared_content_by_oid_path,
                                &blob.oid,
                                &blob.path,
                                kind,
                                &bytes,
                                validate_work_event_shard,
                            ) {
                                Ok(content) => content,
                                Err(error) => {
                                    tracing::warn!(%error, source = %key, "work events ingest: ref source validation failed");
                                    source_discovery_failed = true;
                                    continue;
                                }
                            };
                            pending_sources.push(PendingWorkEventsSource {
                                key,
                                fingerprint,
                                content,
                                container: scan.container.clone(),
                                reload_from_worktree: false,
                            });
                        }
                        scanned_groups.insert(
                            scan.group.clone(),
                            ScannedGroup {
                                snapshot: scan.snapshot.clone(),
                                discovered,
                            },
                        );
                    }
                }
                Some(Err(error)) => {
                    tracing::warn!(%error, "work events ingest: ref event batch discovery failed");
                    source_discovery_failed = true;
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "work events ingest: origin ref listing failed");
            source_discovery_failed = true;
        }
    }
    phases.mark("ref_scan");

    let close_path = work_items_path
        .parent()
        .map(|parent| parent.join("work-events-closed.jsonl"));
    let mut pending_local_lifecycle = match close_path.as_ref().map(std::fs::read_to_string) {
        Some(Ok(content)) if !content.is_empty() => {
            let key = format!(
                "{SOURCE_LOCAL_LIFECYCLE}{}",
                close_path.as_ref().unwrap().display()
            );
            let fingerprint = content_fingerprint(&content);
            Some((key, fingerprint))
        }
        Some(Ok(_)) | None => None,
        Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
        Some(Err(error)) => {
            tracing::warn!(%error, "work events ingest: local lifecycle log read failed");
            return summary;
        }
    };

    let discovered_source_list_fingerprint = group_list_fingerprint(&discovered_groups);
    let had_source_list_fingerprint = store.contains(SOURCE_LIST);
    let source_list_changed = !store.is_current(SOURCE_LIST, &discovered_source_list_fingerprint);

    if !rebuild_required
        && pending_local_lifecycle
            .as_ref()
            .is_some_and(|(key, fingerprint)| store.is_current(key, fingerprint))
    {
        summary.sources_skipped += 1;
        pending_local_lifecycle = None;
    }

    if rebuild_required && source_discovery_failed {
        tracing::warn!(
            "work events ingest: projection rebuild deferred because source discovery was incomplete"
        );
        return summary;
    }

    if pending_sources.is_empty() && pending_local_lifecycle.is_none() {
        let authoritative_empty_source_deletion =
            rebuild_required && source_list_changed && had_source_list_fingerprint;
        if rebuild_required && !authoritative_empty_source_deletion {
            tracing::warn!(
                "work events ingest: projection rebuild deferred because no shared or local lifecycle source was readable"
            );
            return summary;
        }
        if !authoritative_empty_source_deletion {
            // Nothing to ingest, but the snapshots this pass verified (and a
            // migrated legacy state) still spare the next pass the work.
            record_scanned_groups(&mut store, &scanned_groups, &[]);
            for name in &vanished_groups {
                store.remove_group(name);
            }
            save_intake_store(&mut store, &mut summary);
            phases.mark("state_save");
            return summary;
        }
    }

    before_intake();
    let shared_sources = pending_sources
        .iter()
        .map(|source| {
            SharedWorkEventsSource::new(Arc::clone(&source.content), source.container.clone())
        })
        .collect::<Vec<_>>();
    let intake = if rebuild_required {
        rebuild_work_events_with_shared_loader(
            work_items_path,
            || load_pending_sources_for_rebuild(&pending_sources, worktree_entries),
            close_path.as_deref(),
        )
    } else if pending_local_lifecycle.is_some() {
        ingest_work_event_sources_with_local_path(
            work_items_path,
            shared_sources,
            close_path.as_deref(),
        )
        .map(|(report, local_fingerprint)| {
            (
                report,
                pending_sources
                    .iter()
                    .map(|source| (source.key.clone(), source.fingerprint.clone()))
                    .collect(),
                local_fingerprint,
            )
        })
    } else {
        ingest_work_events_sources(work_items_path, shared_sources).map(|report| {
            (
                report,
                pending_sources
                    .iter()
                    .map(|source| (source.key.clone(), source.fingerprint.clone()))
                    .collect(),
                None,
            )
        })
    };
    phases.mark("intake");
    match intake {
        Ok((report, shared_fingerprints, local_fingerprint)) => {
            summary.sources_ingested =
                shared_fingerprints.len() + usize::from(local_fingerprint.is_some());
            summary.events_applied = report.applied;
            summary.projection_rebuilt = rebuild_required;
            if rebuild_required {
                // A semantics rebuild establishes a new source snapshot. A
                // fingerprint retained for a source that was not actually
                // folded would make a later-restored source look current and
                // permanently skip its events.
                record_rebuilt_groups(
                    &mut store,
                    &discovered_groups,
                    &scanned_groups,
                    shared_fingerprints,
                );
                store.record_projection_version(SOURCE_CONTEXT_FINGERPRINT_VERSION);
            } else {
                record_scanned_groups(&mut store, &scanned_groups, &shared_fingerprints);
                for name in &vanished_groups {
                    store.remove_group(name);
                }
            }
            if let (Some(path), Some(fingerprint)) = (close_path.as_ref(), local_fingerprint) {
                store.record(
                    format!("{SOURCE_LOCAL_LIFECYCLE}{}", path.display()),
                    fingerprint,
                );
            }
            store.record(SOURCE_LIST, discovered_source_list_fingerprint);
            save_intake_store(&mut store, &mut summary);
            phases.mark("state_save");
        }
        Err(error) => {
            tracing::warn!(%error, "work events ingest: globally ordered intake failed");
        }
    }
    summary
}

/// One group this pass derived source by source.
struct ScannedGroup {
    snapshot: String,
    /// Source key → fingerprint, exactly as this scan discovered them.
    discovered: BTreeMap<String, String>,
}

/// The fingerprints `discovered` adds to `group`, or `None` when a source
/// the store holds changed or disappeared, or the group's history is
/// unreadable. Either way a rebuild is required.
fn group_additions<'a>(
    store: &mut WorkEventsIntakeStore,
    group: &str,
    discovered: &'a BTreeMap<String, String>,
) -> Option<Vec<&'a str>> {
    let previous = store.group_sources(group)?;
    if previous
        .iter()
        .any(|(key, fingerprint)| discovered.get(key) != Some(fingerprint))
    {
        return None;
    }
    Some(
        discovered
            .keys()
            .filter(|key| !previous.contains_key(*key))
            .map(String::as_str)
            .collect(),
    )
}

/// After an incremental pass: add what was ingested to each derived group.
/// A snapshot is recorded only when the group now holds exactly what that
/// scan discovered, so a source that failed to read is retried next pass.
fn record_scanned_groups(
    store: &mut WorkEventsIntakeStore,
    scanned_groups: &BTreeMap<String, ScannedGroup>,
    ingested: &[(String, String)],
) {
    let mut ingested_by_group = HashMap::<&str, Vec<&(String, String)>>::new();
    for source in ingested {
        ingested_by_group
            .entry(source_group(&source.0))
            .or_default()
            .push(source);
    }
    for (group, scanned) in scanned_groups {
        let Some(previous) = store.group_sources(group).cloned() else {
            continue;
        };
        let mut sources = previous.clone();
        for (key, fingerprint) in ingested_by_group.remove(group.as_str()).unwrap_or_default() {
            sources.insert(key.clone(), fingerprint.clone());
        }
        let snapshot = (sources == scanned.discovered).then(|| scanned.snapshot.clone());
        if store.group(group).is_none() && sources.is_empty() && snapshot.is_none() {
            // Nothing held and nothing verified: there is no information to
            // record, and a pass that failed to read must not mutate state.
            continue;
        }
        if store.group(group).is_none() || sources != previous {
            store.set_group(group, snapshot, sources);
        } else {
            store.set_group_snapshot(group, snapshot);
        }
    }
}

/// After a rebuild: the folded sources become the whole state.
fn record_rebuilt_groups(
    store: &mut WorkEventsIntakeStore,
    discovered_groups: &BTreeMap<String, (String, bool)>,
    scanned_groups: &BTreeMap<String, ScannedGroup>,
    ingested: Vec<(String, String)>,
) {
    store.clear();
    let mut groups = BTreeMap::<String, BTreeMap<String, String>>::new();
    for (key, fingerprint) in ingested {
        let group = source_group(&key).to_owned();
        groups.entry(group).or_default().insert(key, fingerprint);
    }
    for name in discovered_groups.keys() {
        groups.entry(name.clone()).or_default();
    }
    for (name, sources) in groups {
        let snapshot = scanned_groups
            .get(&name)
            .filter(|scanned| scanned.discovered == sources)
            .map(|scanned| scanned.snapshot.clone());
        store.set_group(&name, snapshot, sources);
    }
}

fn save_intake_store(store: &mut WorkEventsIntakeStore, summary: &mut WorkEventsIngestSummary) {
    match store.save() {
        Ok(report) => summary.state_bytes_written = report.bytes_written,
        Err(error) => tracing::warn!(%error, "work events ingest: state save failed"),
    }
    summary.state_sources = store.source_count();
    summary.state_bytes = store.stored_bytes();
}

/// One fetched origin ref and the snapshot of its event trees.
struct RefScan {
    refname: String,
    commit: String,
    group: String,
    container: Option<WorkspaceExecutionContainerRef>,
    context: String,
    snapshot: String,
}

impl RefScan {
    fn new(refname: &str, commit: &str) -> Self {
        let container = origin_ref_execution_container(refname);
        let context = container_context(container.as_ref());
        // A commit pins its whole tree, so an unchanged commit is an
        // unchanged set of event sources.
        let snapshot = group_snapshot("ref-commit-v1", &context, [("commit", commit)]);
        Self {
            refname: refname.to_string(),
            commit: commit.to_string(),
            group: format!("{SOURCE_REF}{refname}"),
            container,
            context,
            snapshot,
        }
    }
}

struct RefBatch {
    blobs_by_ref: Vec<Vec<gwt_git::blob::WorkEventBlob>>,
    /// Per batched ref: source key → fingerprint.
    discovered: Vec<BTreeMap<String, String>>,
    /// Sources the store did not hold yet.
    new_keys: HashSet<String>,
    requires_rebuild: bool,
}

/// Read the event trees of `batch` in one `cat-file --batch` pass and select
/// payloads: all of them when `select_all`, otherwise only the sources the
/// store does not hold yet. A held source that changed or disappeared marks
/// the batch as requiring a rebuild and selects everything.
fn read_ref_batch(
    project_root: &Path,
    batch: &[&RefScan],
    store: &mut WorkEventsIntakeStore,
    select_all: bool,
) -> gwt_core::Result<RefBatch> {
    let commits = batch
        .iter()
        .map(|scan| scan.commit.clone())
        .collect::<Vec<_>>();
    let mut discovered = Vec::with_capacity(batch.len());
    let mut new_keys = HashSet::new();
    let mut requires_rebuild = false;
    let blobs_by_ref = gwt_git::blob::work_event_blobs_batch(
        project_root,
        &commits,
        EVENTS_TREE_PATH,
        EVENTS_TREE_DIR,
        |descriptors_by_ref| {
            let mut all_oids = HashSet::new();
            let mut new_oids = HashSet::new();
            for (scan, descriptors) in batch.iter().zip(descriptors_by_ref) {
                let mut sources = BTreeMap::new();
                let mut oid_by_key = HashMap::new();
                for descriptor in descriptors {
                    if is_work_event_writer_temp_residue(Path::new(&descriptor.path)) {
                        continue;
                    }
                    let key = format!("{SOURCE_REF}{}:{}", scan.refname, descriptor.path);
                    sources.insert(
                        key.clone(),
                        source_fingerprint_in_context(&descriptor.oid, &scan.context),
                    );
                    oid_by_key.insert(key, descriptor.oid.as_str());
                    all_oids.insert(descriptor.oid.clone());
                }
                if !select_all && !requires_rebuild {
                    match group_additions(store, &scan.group, &sources) {
                        Some(keys) => {
                            for key in keys {
                                new_oids.insert(oid_by_key[key].to_string());
                                new_keys.insert(key.to_string());
                            }
                        }
                        None => requires_rebuild = true,
                    }
                }
                discovered.push(sources);
            }
            if select_all || requires_rebuild {
                all_oids
            } else {
                new_oids
            }
        },
    )?;
    Ok(RefBatch {
        blobs_by_ref,
        discovered,
        new_keys,
        requires_rebuild,
    })
}

/// One local worktree's event sources and the snapshot of their metadata.
struct LocalGroupScan {
    name: String,
    context: String,
    snapshot: String,
    /// Each source with its intake key, in scan order.
    sources: Vec<(WorkEventsSource, String)>,
}

fn local_group_scans(sources: Vec<WorkEventsSource>) -> Vec<LocalGroupScan> {
    let mut groups = BTreeMap::<String, LocalGroupScan>::new();
    for source in sources {
        let key = source.key();
        let name = source_group(&key).to_owned();
        let group = groups
            .entry(name.clone())
            .or_insert_with(|| LocalGroupScan {
                name,
                context: container_context(source.container.as_ref()),
                snapshot: String::new(),
                sources: Vec::new(),
            });
        group.sources.push((source, key));
    }
    groups
        .into_values()
        .map(|mut group| {
            group.snapshot = group_snapshot(
                "local-metadata-v1",
                &group.context,
                group
                    .sources
                    .iter()
                    .map(|(source, key)| (key.as_str(), source.metadata.as_str())),
            );
            group
        })
        .collect()
}

fn validate_work_event_store_path(events_dir: &Path) -> gwt_core::Result<bool> {
    let mut managed_paths = events_dir.ancestors().take(3).collect::<Vec<_>>();
    managed_paths.reverse();
    for managed_path in managed_paths {
        match std::fs::symlink_metadata(managed_path) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(gwt_core::GwtError::Other(format!(
                    "work event shard store path is not a real directory: {}",
                    managed_path.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

/// The legacy log and every canonical event shard in each local worktree,
/// with the size/mtime identity the directory listing already returned. On
/// Windows `DirEntry::metadata` costs no extra system call; stat-ing every
/// file again was most of a scan over a few hundred worktrees (#4397).
fn worktree_event_sources(
    entries: &[gwt::worktree_inventory::WorktreeEntry],
) -> gwt_core::Result<Vec<WorkEventsSource>> {
    let mut sources = Vec::new();
    for entry in entries {
        let container = entry
            .branch
            .clone()
            .map(|branch| WorkspaceExecutionContainerRef {
                branch: Some(branch),
                worktree_path: Some(entry.path.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            });
        let legacy_path = entry.path.join(EVENTS_TREE_PATH);
        match std::fs::symlink_metadata(&legacy_path) {
            Ok(metadata) => sources.push(WorkEventsSource {
                metadata: immutable_metadata_fingerprint(&legacy_path, &metadata)?,
                events_path: legacy_path,
                kind: WorkEventsSourceKind::Legacy,
                container: container.clone(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let events_dir = entry.path.join(EVENTS_TREE_DIR);
        if !validate_work_event_store_path(&events_dir)? {
            continue;
        }
        let mut store_entries = std::fs::read_dir(&events_dir)?.collect::<Result<Vec<_>, _>>()?;
        store_entries.sort_by_key(std::fs::DirEntry::file_name);
        for store_entry in store_entries {
            let file_type = store_entry.file_type()?;
            if file_type.is_file() {
                if is_work_event_writer_temp_residue(&store_entry.path()) {
                    continue;
                }
                push_scanned_shard(&mut sources, &store_entry, container.clone())?;
                continue;
            }
            if !file_type.is_dir() || !is_work_event_bucket_name(&store_entry.file_name()) {
                return Err(gwt_core::GwtError::Other(format!(
                    "work event shard store entry is neither a legacy flat shard nor a digest bucket: {}",
                    store_entry.path().display()
                )));
            }
            let mut bucket_entries =
                std::fs::read_dir(store_entry.path())?.collect::<Result<Vec<_>, _>>()?;
            bucket_entries.sort_by_key(std::fs::DirEntry::file_name);
            for shard in bucket_entries {
                if !shard.file_type()?.is_file() {
                    return Err(gwt_core::GwtError::Other(format!(
                        "work event bucket entry is not a regular file: {}",
                        shard.path().display()
                    )));
                }
                if is_work_event_writer_temp_residue(&shard.path()) {
                    continue;
                }
                push_scanned_shard(&mut sources, &shard, container.clone())?;
            }
        }
    }
    Ok(sources)
}

fn push_scanned_shard(
    sources: &mut Vec<WorkEventsSource>,
    entry: &std::fs::DirEntry,
    container: Option<WorkspaceExecutionContainerRef>,
) -> gwt_core::Result<()> {
    let events_path = entry.path();
    let metadata = match entry.metadata() {
        Ok(metadata) => metadata,
        // Unix stats here; a shard removed since the listing is simply gone.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    sources.push(WorkEventsSource {
        metadata: immutable_metadata_fingerprint(&events_path, &metadata)?,
        events_path,
        kind: WorkEventsSourceKind::Shard,
        container,
    });
    Ok(())
}

fn is_work_event_bucket_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.len() == 2
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn is_work_event_writer_temp_residue(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((hash, suffix)) = rest.split_once(".jsonl.create-") else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !suffix.is_empty()
}

#[derive(Debug, Clone)]
struct WorkEventsSource {
    events_path: PathBuf,
    kind: WorkEventsSourceKind,
    container: Option<WorkspaceExecutionContainerRef>,
    /// Size/mtime identity from the scan (`immutable-metadata-v1:…`).
    metadata: String,
}

impl WorkEventsSource {
    fn key(&self) -> String {
        format!("{SOURCE_WORKTREE}{}", self.events_path.display())
    }
}

fn origin_ref_execution_container(refname: &str) -> Option<WorkspaceExecutionContainerRef> {
    let branch = refname.strip_prefix("refs/remotes/origin/")?.trim();
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }
    Some(WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
    })
}

/// The intake group of a source key (see `work_events_intake_group_of`).
fn source_group(key: &str) -> &str {
    work_events_intake_group_of(key).unwrap_or(SOURCE_WORKTREE)
}

fn container_context(container: Option<&WorkspaceExecutionContainerRef>) -> String {
    container
        .map(|container| {
            serde_json::to_string(container)
                .unwrap_or_else(|_| "container-serialization-error".into())
        })
        .unwrap_or_else(|| "no-container".to_string())
}

fn source_fingerprint(
    raw_fingerprint: &str,
    container: Option<&WorkspaceExecutionContainerRef>,
) -> String {
    source_fingerprint_in_context(raw_fingerprint, &container_context(container))
}

fn source_fingerprint_in_context(raw_fingerprint: &str, context: &str) -> String {
    content_fingerprint(&format!(
        "{SOURCE_CONTEXT_FINGERPRINT_VERSION}\n{raw_fingerprint}\n{context}"
    ))
}

/// Identity of one group's scan: equal snapshots mean the group's sources
/// and their fingerprints are unchanged, so none of them is derived again.
fn group_snapshot<'a>(
    kind: &str,
    context: &str,
    parts: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut hasher = Sha256::new();
    for field in [SOURCE_CONTEXT_FINGERPRINT_VERSION, kind, context] {
        hasher.update(field.as_bytes());
        hasher.update(b"\n");
    }
    for (key, raw_fingerprint) in parts {
        hasher.update(key.as_bytes());
        hasher.update(b"\0");
        hasher.update(raw_fingerprint.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// Identity of the set of groups holding sources. Only "was anything held
/// before" is read from it, when a rebuild finds no source at all.
fn group_list_fingerprint(groups: &BTreeMap<String, (String, bool)>) -> String {
    content_fingerprint(
        &groups
            .iter()
            .filter(|(_, (_, holds_sources))| *holds_sources)
            .map(|(name, (snapshot, _))| format!("{name}\0{snapshot}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn immutable_metadata_fingerprint(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> gwt_core::Result<String> {
    if !metadata.file_type().is_file() {
        return Err(gwt_core::GwtError::Other(format!(
            "immutable Work event shard is not a regular file: {}",
            path.display()
        )));
    }
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            gwt_core::GwtError::Other(format!(
                "immutable Work event shard has an invalid modified time {}: {error}",
                path.display()
            ))
        })?;
    Ok(format!(
        "immutable-metadata-v1:{}:{}",
        metadata.len(),
        modified.as_nanos()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_event_store_still_rejects_a_non_directory_managed_parent() {
        let root = tempfile::tempdir().expect("managed root");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(root.path());
        std::fs::create_dir_all(root.path().join(".gwt")).expect("create .gwt");
        std::fs::write(root.path().join(".gwt/work"), b"not a directory")
            .expect("replace managed parent with a file");

        let error = validate_work_event_store_path(&root.path().join(".gwt/work/events"))
            .expect_err("a missing leaf must not bypass managed-parent validation");

        assert!(
            error.to_string().contains("not a real directory"),
            "{error}"
        );
    }

    #[test]
    fn shared_ref_blob_validates_and_decodes_once_per_oid_and_path() {
        let event_id = "evt-shared-ref-decode";
        let event = event_line(
            event_id,
            "work-shared-ref-decode",
            "Shared decode work",
            "2026-08-13T01:00:00Z",
        );
        let bytes = format!("{event}\n").into_bytes();
        let digest = format!("{:x}", sha2::Sha256::digest(event_id.as_bytes()));
        let path = format!(".gwt/work/events/{}/{}.jsonl", &digest[..2], digest);
        let oid = "a".repeat(40);
        let mut cache = HashMap::new();
        let validations = std::sync::atomic::AtomicUsize::new(0);

        let first = shared_ref_source_content(
            &mut cache,
            &oid,
            &path,
            WorkEventsSourceKind::Shard,
            &bytes,
            |path, bytes| {
                validations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                validate_work_event_shard(path, bytes)
            },
        )
        .expect("validate first ref descriptor");
        let second = shared_ref_source_content(
            &mut cache,
            &oid,
            &path,
            WorkEventsSourceKind::Shard,
            &bytes,
            |path, bytes| {
                validations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                validate_work_event_shard(path, bytes)
            },
        )
        .expect("reuse validation for second ref descriptor");

        assert_eq!(validations.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(Arc::ptr_eq(&first, &second));

        let invalid_oid = "b".repeat(40);
        let invalid_path = ".gwt/work/events/00/invalid.jsonl";
        for _ in 0..2 {
            let error = shared_ref_source_content(
                &mut cache,
                &invalid_oid,
                invalid_path,
                WorkEventsSourceKind::Shard,
                b"invalid\n",
                |path, bytes| {
                    validations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    validate_work_event_shard(path, bytes)
                },
            )
            .expect_err("invalid shared descriptor must stay fail-closed");
            assert!(error.to_string().contains("invalid filename"), "{error}");
        }
        assert_eq!(
            validations.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "the shared invalid descriptor must also validate only once"
        );
    }
    use gwt_core::work_events_intake::WorkEventsIntakeState;
    use sha2::Digest;
    use std::process::Command;

    fn run(cmd: &mut Command) {
        let output = cmd.output().expect("git command should run");
        assert!(
            output.status.success(),
            "git command failed: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(path: &Path) {
        run(gwt_core::process::hidden_command("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(path));
        run(gwt_core::process::hidden_command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path));
        run(gwt_core::process::hidden_command("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path));
    }

    fn event_line(id: &str, work_id: &str, title: &str, updated_at: &str) -> String {
        format!(
            "{{\"id\":\"{id}\",\"work_item_id\":\"{work_id}\",\"kind\":\"start\",\"updated_at\":\"{updated_at}\",\"title\":\"{title}\",\"status_category\":\"active\"}}"
        )
    }

    struct SessionEventFixture<'a> {
        id: &'a str,
        work_id: &'a str,
        kind: &'a str,
        title: &'a str,
        session_id: &'a str,
        branch: &'a str,
        worktree_path: &'a Path,
        updated_at: &'a str,
    }

    fn session_event_line(event: SessionEventFixture<'_>) -> String {
        serde_json::json!({
            "id": event.id,
            "work_item_id": event.work_id,
            "kind": event.kind,
            "updated_at": event.updated_at,
            "title": event.title,
            "status_category": "active",
            "agent_session_id": event.session_id,
            "execution_container": {
                "branch": event.branch,
                "worktree_path": event.worktree_path,
            },
        })
        .to_string()
    }

    /// Issue #4378 AC-1: startup already listed the worktrees, so the ingest
    /// reads its worktree sources from that inventory instead of running
    /// `git worktree list` again. The worktree below is a plain directory that
    /// only the inventory names, so a listing of its own would never find it.
    #[test]
    fn ingest_reads_worktree_sources_from_the_startup_inventory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let worktree = temp.path().join("inventory-only");
        std::fs::create_dir_all(worktree.join(".gwt/work")).expect("mk .gwt/work");
        std::fs::write(
            worktree.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n",
                event_line(
                    "evt-inventory-1",
                    "work-inventory-cccc3333",
                    "inventory work",
                    "2026-06-03T10:00:00Z"
                )
            ),
        )
        .expect("write inventory worktree events");
        let inventory = vec![gwt::worktree_inventory::WorktreeEntry {
            id: "inventory-only".to_string(),
            kind: gwt::worktree_inventory::WorktreeEntryKind::Workspace,
            path: worktree.clone(),
            label: "work/inventory-only".to_string(),
            branch: Some("work/inventory-only".to_string()),
            is_active: false,
        }];
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");

        let summary = ingest_project_work_events_paths_with_inventory(
            &repo,
            &work_items_path,
            &state_path,
            Some(&inventory),
        );

        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        assert!(
            projection
                .work_items
                .iter()
                .any(|item| item.id == "work-inventory-cccc3333"),
            "the inventory worktree's events must be ingested: {summary:?}"
        );
    }

    /// SC-258: events committed on another branch (visible only as a fetched
    /// origin ref) restore the Work skeleton without any checkout; the local
    /// working copy of the repo is also swept. Second run is fingerprint-
    /// skipped end to end.
    #[test]
    fn ingest_restores_skeleton_from_worktree_fs_and_origin_ref() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        // Worktree fs source: uncommitted events.jsonl in the main checkout.
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("mk .gwt/work");
        std::fs::write(
            repo.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n",
                event_line(
                    "evt-fs-1",
                    "work-fs-aaaa1111",
                    "fs work",
                    "2026-06-01T10:00:00Z"
                )
            ),
        )
        .expect("write fs events");

        // Origin ref source: events.jsonl committed on a side branch that is
        // NOT checked out anywhere, forged as a remote tracking ref.
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/remote-side"])
            .current_dir(&repo));
        std::fs::write(
            repo.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n",
                event_line(
                    "evt-ref-1",
                    "work-ref-bbbb2222",
                    "remote work",
                    "2026-06-02T10:00:00Z"
                )
            ),
        )
        .expect("write ref events");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events.jsonl"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "remote events"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["update-ref", "refs/remotes/origin/work/remote-side", "HEAD"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(&repo));
        // Restore the fs source clobbered by the branch dance.
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("mk .gwt/work");
        std::fs::write(
            repo.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n",
                event_line(
                    "evt-fs-1",
                    "work-fs-aaaa1111",
                    "fs work",
                    "2026-06-01T10:00:00Z"
                )
            ),
        )
        .expect("rewrite fs events");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");

        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(
            first.events_applied >= 2,
            "fs + ref events applied: {first:?}"
        );

        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        let ids: Vec<&str> = projection
            .work_items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert!(ids.contains(&"work-fs-aaaa1111"), "fs skeleton restored");
        assert!(
            ids.contains(&"work-ref-bbbb2222"),
            "origin ref skeleton restored"
        );
        let remote_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-ref-bbbb2222")
            .expect("remote item");
        assert!(
            remote_item
                .execution_containers
                .iter()
                .any(|container| container.branch.as_deref() == Some("work/remote-side")),
            "legacy branch-less events imported from a source ref keep that ref's branch"
        );

        // Second run: every source fingerprint is current — nothing re-reads.
        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(second.events_applied, 0);
        assert_eq!(second.sources_ingested, 0);
        assert!(second.sources_skipped >= 2, "fingerprint skip: {second:?}");
    }

    #[test]
    fn ingest_dual_reads_local_legacy_and_canonical_shards_exactly_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        let legacy_event = event_line(
            "evt-dual-legacy",
            "work-dual-legacy",
            "Legacy work",
            "2026-08-12T01:00:00Z",
        );
        let shard_event = event_line(
            "evt-dual-shard",
            "work-dual-shard",
            "Shard work",
            "2026-08-12T02:00:00Z",
        );
        let work_dir = repo.join(".gwt/work");
        std::fs::create_dir_all(work_dir.join("events")).expect("event store");
        std::fs::write(work_dir.join("events.jsonl"), format!("{legacy_event}\n"))
            .expect("legacy source");
        let shard_id = format!("{:x}", sha2::Sha256::digest(b"evt-dual-shard"));
        std::fs::write(
            work_dir.join("events").join(format!("{shard_id}.jsonl")),
            format!("{shard_event}\n"),
        )
        .expect("shard source");
        let duplicate_id = format!("{:x}", sha2::Sha256::digest(b"evt-dual-legacy"));
        std::fs::write(
            work_dir
                .join("events")
                .join(format!("{duplicate_id}.jsonl")),
            format!("{legacy_event}\n"),
        )
        .expect("duplicate shard source");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "initial dual-read rebuild: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-dual-legacy"));
        let shard = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-dual-shard")
            .expect("shard Work restored");
        assert_eq!(shard.events.len(), 1, "one shard event is folded once");
        let legacy = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-dual-legacy")
            .expect("legacy Work restored");
        assert_eq!(
            legacy.events.len(),
            1,
            "legacy/shard duplicate is exact-once"
        );
    }

    #[test]
    fn unchanged_pass_does_not_read_immutable_local_shard_bytes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let event_id = "evt-local-immutable-cache";
        let event = event_line(
            event_id,
            "work-local-immutable-cache",
            "Immutable cache work",
            "2026-08-12T02:30:00Z",
        );
        let digest = format!("{:x}", sha2::Sha256::digest(event_id.as_bytes()));
        let shard = repo
            .join(EVENTS_TREE_DIR)
            .join(&digest[..2])
            .join(format!("{digest}.jsonl"));
        std::fs::create_dir_all(shard.parent().expect("bucket")).expect("event bucket");
        std::fs::write(&shard, format!("{event}\n")).expect("event shard");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt, "{first:?}");

        let reads = std::sync::atomic::AtomicUsize::new(0);
        let second = ingest_project_work_events_paths_with_source_read_hook(
            &repo,
            &work_items_path,
            &state_path,
            |_| {
                reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        );

        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(second.events_applied, 0);
        assert_eq!(second.sources_ingested, 0);
        assert!(second.sources_skipped >= 1, "{second:?}");
    }

    fn write_legacy_log(repo: &Path, events: &[(&str, &str)]) {
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().expect("work dir")).expect("work dir");
        let content = events
            .iter()
            .map(|(event_id, work_id)| {
                event_line(event_id, work_id, "Legacy work", "2026-08-12T02:30:00Z") + "\n"
            })
            .collect::<String>();
        std::fs::write(legacy, content).expect("legacy log");
    }

    /// Issue #4371: every trigger read all 239 frozen legacy logs (398.5 MB)
    /// in full only to hash them. Like a shard, an unchanged legacy log is
    /// judged by its metadata, and the rebuild records that same fingerprint.
    #[test]
    fn unchanged_pass_does_not_read_legacy_log_bytes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        write_legacy_log(&repo, &[("evt-legacy-cache", "work-legacy-cache")]);
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt, "{first:?}");

        let reads = std::sync::atomic::AtomicUsize::new(0);
        let second = ingest_project_work_events_paths_with_source_read_hook(
            &repo,
            &work_items_path,
            &state_path,
            |_| {
                reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        );

        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(!second.projection_rebuilt, "{second:?}");
        assert_eq!(second.sources_ingested, 0);
    }

    /// Issue #4371: judging legacy logs by metadata must not hide a change.
    #[test]
    fn changed_legacy_log_is_read_on_the_next_pass() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        write_legacy_log(&repo, &[("evt-legacy-first", "work-legacy-first")]);
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        write_legacy_log(
            &repo,
            &[
                ("evt-legacy-first", "work-legacy-first"),
                ("evt-legacy-second", "work-legacy-second"),
            ],
        );
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-legacy-second"));
    }

    #[test]
    fn ingest_restores_shard_committed_only_on_fetched_origin_ref() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/remote-shard"])
            .current_dir(&repo));
        let event = event_line(
            "evt-remote-shard",
            "work-remote-shard",
            "Remote shard work",
            "2026-08-12T03:00:00Z",
        );
        let hash = format!("{:x}", sha2::Sha256::digest(b"evt-remote-shard"));
        let shard = repo.join(EVENTS_TREE_DIR).join(format!("{hash}.jsonl"));
        std::fs::create_dir_all(shard.parent().unwrap()).expect("event store");
        std::fs::write(&shard, format!("{event}\n")).expect("remote shard");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "remote shard"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "refs/remotes/origin/work/remote-shard",
                "HEAD",
            ])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(&repo));
        assert!(!repo.join(EVENTS_TREE_DIR).exists(), "shard is ref-only");
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("complete local source discovery");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(summary.projection_rebuilt, "ref shard rebuild: {summary:?}");
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-remote-shard")
            .expect("ref shard Work restored");
        assert!(item.execution_containers.iter().any(|container| {
            container.branch.as_deref() == Some("work/remote-shard")
                && container.worktree_path.is_none()
        }));
    }

    #[test]
    fn invalid_shard_defers_rebuild_and_preserves_existing_projection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-preserved",
                    "work-preserved",
                    "Preserved work",
                    "2026-08-12T04:00:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let before = std::fs::read(&work_items_path).expect("projection before invalid shard");

        let invalid = repo.join(EVENTS_TREE_DIR).join("not-a-sha256.jsonl");
        std::fs::create_dir_all(invalid.parent().unwrap()).expect("event store");
        std::fs::write(&invalid, b"{}\n").expect("invalid shard");
        let mut stale = load_work_events_intake_state(&state_path);
        stale.record_projection_version("source-context-v6-complete-project-transaction");
        save_work_events_intake_state(&state_path, &stale).expect("force rebuild");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            !summary.projection_rebuilt,
            "invalid shard must defer: {summary:?}"
        );
        assert_eq!(std::fs::read(&work_items_path).unwrap(), before);
        assert!(!load_work_events_intake_state(&state_path)
            .projection_is_current(SOURCE_CONTEXT_FINGERPRINT_VERSION));
    }

    #[cfg(unix)]
    fn assert_symlinked_local_managed_event_parent_defers(managed_parent: &str) {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-symlink-parent-base",
                    "work-symlink-parent-base",
                    "Symlink parent base",
                    "2026-08-12T04:05:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let projection_before = std::fs::read(&work_items_path).expect("projection before");
        let state_before = intake_state_files(&state_path);

        let external_parent = temp
            .path()
            .join(format!("external-{}", managed_parent.replace('/', "-")));
        std::fs::create_dir_all(&external_parent).expect("external managed parent");
        let link = repo.join(managed_parent);
        std::fs::remove_dir_all(&link).expect("replace managed parent");
        std::os::unix::fs::symlink(&external_parent, &link).expect("symlink managed parent");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(intake_state_files(&state_path), state_before);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_local_gwt_parent_defers_rebuild_without_mutation() {
        assert_symlinked_local_managed_event_parent_defers(".gwt");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_local_work_parent_defers_rebuild_without_mutation() {
        assert_symlinked_local_managed_event_parent_defers(".gwt/work");
    }

    fn assert_missing_local_managed_event_parent_is_authoritative_deletion(managed_parent: &str) {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-missing-parent-base",
                    "work-missing-parent-base",
                    "Missing parent base",
                    "2026-08-12T04:07:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let missing = repo.join(managed_parent);
        std::fs::remove_dir_all(&missing).expect("remove managed parent");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "a previously tracked source disappearing is an authoritative deletion: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("empty projection");
        assert!(projection.work_items.is_empty());
    }

    #[test]
    fn missing_local_gwt_parent_removes_previously_tracked_source() {
        assert_missing_local_managed_event_parent_is_authoritative_deletion(".gwt");
    }

    #[test]
    fn missing_local_work_parent_removes_previously_tracked_source() {
        assert_missing_local_managed_event_parent_is_authoritative_deletion(".gwt/work");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_local_event_store_defers_rebuild_without_mutating_projection_or_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-symlink-root-base",
                    "work-symlink-root-base",
                    "Symlink root base",
                    "2026-08-12T04:10:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let projection_before = std::fs::read(&work_items_path).expect("projection before");
        let state_before = intake_state_files(&state_path);

        let external = temp.path().join("external-events");
        std::fs::create_dir_all(&external).expect("external event store");
        let id = "evt-outside-symlink-root";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        std::fs::write(
            external.join(format!("{hash}.jsonl")),
            format!(
                "{}\n",
                event_line(
                    id,
                    "work-outside-symlink-root",
                    "Must stay outside",
                    "2026-08-12T04:11:00Z",
                )
            ),
        )
        .expect("external shard");
        std::os::unix::fs::symlink(&external, repo.join(EVENTS_TREE_DIR))
            .expect("symlink event store");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(intake_state_files(&state_path), state_before);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_local_event_shard_defers_rebuild_without_mutating_projection_or_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-symlink-entry-base",
                    "work-symlink-entry-base",
                    "Symlink entry base",
                    "2026-08-12T04:20:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let projection_before = std::fs::read(&work_items_path).expect("projection before");
        let state_before = intake_state_files(&state_path);

        let id = "evt-outside-symlink-entry";
        let target = temp.path().join("outside-event.jsonl");
        std::fs::write(
            &target,
            format!(
                "{}\n",
                event_line(
                    id,
                    "work-outside-symlink-entry",
                    "Must stay outside",
                    "2026-08-12T04:21:00Z",
                )
            ),
        )
        .expect("external shard");
        let events_dir = repo.join(EVENTS_TREE_DIR);
        std::fs::create_dir_all(&events_dir).expect("real event store");
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        std::os::unix::fs::symlink(&target, events_dir.join(format!("{hash}.jsonl")))
            .expect("symlink shard");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(intake_state_files(&state_path), state_before);
    }

    #[test]
    fn nested_ref_shard_defers_rebuild_without_mutating_projection_or_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-nested-ref-base",
                    "work-nested-ref-base",
                    "Nested ref base",
                    "2026-08-12T04:30:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let projection_before = std::fs::read(&work_items_path).expect("projection before");
        let state_before = intake_state_files(&state_path);

        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/nested-ref-shard"])
            .current_dir(&repo));
        let id = "evt-nested-ref-shard";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let nested = repo
            .join(EVENTS_TREE_DIR)
            .join("nested")
            .join(format!("{hash}.jsonl"));
        std::fs::create_dir_all(nested.parent().unwrap()).expect("nested event store");
        std::fs::write(
            &nested,
            format!(
                "{}\n",
                event_line(
                    id,
                    "work-nested-ref-shard",
                    "Nested ref shard",
                    "2026-08-12T04:31:00Z",
                )
            ),
        )
        .expect("nested ref shard");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "nested ref shard"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "refs/remotes/origin/work/nested-ref-shard",
                "HEAD",
            ])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(&repo));

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(intake_state_files(&state_path), state_before);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_mode_ref_shard_defers_rebuild_without_mutating_projection_or_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-symlink-ref-base",
                    "work-symlink-ref-base",
                    "Symlink ref base",
                    "2026-08-12T04:40:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let projection_before = std::fs::read(&work_items_path).expect("projection before");
        let state_before = intake_state_files(&state_path);

        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/symlink-ref-shard"])
            .current_dir(&repo));
        let id = "evt-symlink-ref-shard";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let shard = repo.join(EVENTS_TREE_DIR).join(format!("{hash}.jsonl"));
        std::fs::create_dir_all(shard.parent().unwrap()).expect("event store");
        let event = event_line(
            id,
            "work-symlink-ref-shard",
            "Symlink ref shard",
            "2026-08-12T04:41:00Z",
        );
        std::os::unix::fs::symlink(format!("{event}\n"), &shard)
            .expect("event-shaped symlink target");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "symlink ref shard"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "refs/remotes/origin/work/symlink-ref-shard",
                "HEAD",
            ])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(&repo));

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(intake_state_files(&state_path), state_before);
    }

    #[test]
    fn incomplete_event_schema_shard_defers_rebuild_and_preserves_projection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-schema-preserved",
                    "work-schema-preserved",
                    "Schema preserved work",
                    "2026-08-12T04:30:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );
        let before = std::fs::read(&work_items_path).expect("projection before invalid schema");

        let id = "evt-id-only";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let invalid = repo.join(EVENTS_TREE_DIR).join(format!("{hash}.jsonl"));
        std::fs::create_dir_all(invalid.parent().unwrap()).expect("event store");
        std::fs::write(&invalid, format!("{{\"id\":\"{id}\"}}\n")).expect("incomplete event shard");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            !summary.projection_rebuilt,
            "incomplete schema shard must defer: {summary:?}"
        );
        assert_eq!(std::fs::read(&work_items_path).unwrap(), before);
    }

    #[test]
    fn future_opaque_event_shard_is_source_valid_and_skipped() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-known-before-future",
                    "work-known-before-future",
                    "Known work",
                    "2026-08-12T04:45:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );

        let id = "evt-future-opaque";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let shard = repo.join(EVENTS_TREE_DIR).join(format!("{hash}.jsonl"));
        std::fs::create_dir_all(shard.parent().unwrap()).expect("event store");
        let future = serde_json::json!({
            "id": id,
            "work_item_id": "work-future-opaque",
            "kind": "future_release_kind",
            "updated_at": "2026-08-12T05:00:00Z",
            "future_top_level": { "preserve": [1, 2, 3] },
        });
        let future_bytes = format!("{future}\n").into_bytes();
        std::fs::write(&shard, &future_bytes).expect("future event shard");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            !summary.projection_rebuilt,
            "immutable addition is incremental"
        );
        assert_eq!(summary.sources_ingested, 1);
        assert_eq!(summary.events_applied, 0, "future event remains opaque");
        assert_eq!(std::fs::read(&shard).unwrap(), future_bytes);
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-known-before-future"));
        assert!(!projection
            .work_items
            .iter()
            .any(|item| item.id == "work-future-opaque"));
    }

    #[test]
    fn shard_source_list_fingerprint_rebuilds_after_deletion() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let events_dir = repo.join(EVENTS_TREE_DIR);
        std::fs::create_dir_all(&events_dir).expect("event store");
        let mut shards = Vec::new();
        for (id, work_id, hour) in [
            ("evt-kept-shard", "work-kept-shard", 5),
            ("evt-deleted-shard", "work-deleted-shard", 6),
        ] {
            let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
            let path = events_dir.join(format!("{hash}.jsonl"));
            std::fs::write(
                &path,
                format!(
                    "{}\n",
                    event_line(
                        id,
                        work_id,
                        work_id,
                        &format!("2026-08-12T{hour:02}:00:00Z"),
                    )
                ),
            )
            .expect("shard");
            shards.push(path);
        }
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );

        std::fs::remove_file(&shards[1]).expect("delete shard");
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "deletion changes source snapshot: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-kept-shard"));
        assert!(!projection
            .work_items
            .iter()
            .any(|item| item.id == "work-deleted-shard"));
    }

    #[test]
    fn tracked_source_list_rebuilds_after_local_legacy_source_deletion() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-local-legacy-delete",
                    "work-local-legacy-delete",
                    "Deleted local legacy",
                    "2026-08-12T06:30:00Z",
                )
            ),
        )
        .expect("legacy source");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );

        std::fs::remove_file(&legacy).expect("delete local legacy source");
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "complete discovery of an empty source list is authoritative: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(!projection
            .work_items
            .iter()
            .any(|item| item.id == "work-local-legacy-delete"));
    }

    #[test]
    fn tracked_source_list_rebuilds_after_origin_legacy_ref_deletion() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/deleted-origin-legacy"])
            .current_dir(&repo));
        let legacy = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                event_line(
                    "evt-origin-legacy-delete",
                    "work-origin-legacy-delete",
                    "Deleted origin legacy",
                    "2026-08-12T06:45:00Z",
                )
            ),
        )
        .expect("legacy source");
        run(gwt_core::process::hidden_command("git")
            .args(["add", EVENTS_TREE_PATH])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "origin legacy"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "refs/remotes/origin/work/deleted-origin-legacy",
                "HEAD",
            ])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(&repo));
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("complete local source discovery");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        assert!(
            ingest_project_work_events_paths(&repo, &work_items_path, &state_path)
                .projection_rebuilt
        );

        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "-d",
                "refs/remotes/origin/work/deleted-origin-legacy",
            ])
            .current_dir(&repo));
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "ref deletion changes source list: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(!projection
            .work_items
            .iter()
            .any(|item| item.id == "work-origin-legacy-delete"));
    }

    #[test]
    fn ingest_reads_bucketed_local_shard() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        let event = event_line(
            "evt-bucketed-local",
            "work-bucketed-local",
            "Bucketed local work",
            "2026-08-13T01:00:00Z",
        );
        let hash = format!("{:x}", sha2::Sha256::digest(b"evt-bucketed-local"));
        let shard = repo
            .join(EVENTS_TREE_DIR)
            .join(&hash[..2])
            .join(format!("{hash}.jsonl"));
        std::fs::create_dir_all(shard.parent().unwrap()).expect("bucket dir");
        std::fs::write(&shard, format!("{event}\n")).expect("bucketed shard");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(summary.projection_rebuilt, "initial rebuild: {summary:?}");
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-bucketed-local"));
    }

    /// Every byte the intake state holds on disk: the legacy file and the
    /// grouped store beside it (#4397).
    fn intake_state_files(state_path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        if let Ok(bytes) = std::fs::read(state_path) {
            files.insert(state_path.to_path_buf(), bytes);
        }
        let mut pending = vec![state_path.with_extension("")];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let bytes = std::fs::read(&path).expect("read intake state file");
                    files.insert(path, bytes);
                }
            }
        }
        assert!(
            !files.is_empty(),
            "intake state exists: {}",
            state_path.display()
        );
        files
    }

    fn write_shard(worktree: &Path, id: &str, work_id: &str) -> PathBuf {
        let event = event_line(id, work_id, "Grouped intake work", "2026-09-15T01:00:00Z");
        let digest = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let shard = worktree
            .join(EVENTS_TREE_DIR)
            .join(&digest[..2])
            .join(format!("{digest}.jsonl"));
        std::fs::create_dir_all(shard.parent().expect("bucket")).expect("event bucket");
        std::fs::write(&shard, format!("{event}\n")).expect("event shard");
        shard
    }

    /// Two worktrees with one shard each: two intake groups (#4397).
    fn two_worktree_fixture(temp: &Path) -> (PathBuf, PathBuf) {
        let repo = temp.join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let linked = temp.join("linked");
        run(gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-b", "linked"])
            .arg(&linked)
            .current_dir(&repo));
        write_shard(&repo, "evt-group-main", "work-group-main");
        write_shard(&linked, "evt-group-linked", "work-group-linked");
        (repo, linked)
    }

    /// Issue #4397 AC-1: a pass re-derives and rewrites only the groups whose
    /// sources changed.
    #[test]
    fn unchanged_groups_are_neither_rederived_nor_rewritten() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let (repo, linked) = two_worktree_fixture(temp.path());
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");

        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt, "{first:?}");
        assert!(first.state_sources >= 2, "{first:?}");

        let unchanged = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(unchanged.sources_rederived, 0, "{unchanged:?}");
        assert_eq!(unchanged.state_bytes_written, 0, "{unchanged:?}");
        assert_eq!(unchanged.sources_skipped, 2, "{unchanged:?}");

        write_shard(&linked, "evt-group-linked-2", "work-group-linked-2");
        let added = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(!added.projection_rebuilt, "{added:?}");
        assert_eq!(added.sources_ingested, 1, "{added:?}");
        assert_eq!(
            added.sources_rederived, 2,
            "only the linked worktree group is re-derived: {added:?}"
        );
        assert_eq!(added.state_sources, first.state_sources + 1, "{added:?}");
        assert!(
            added.state_bytes_written > 0 && added.state_bytes_written < added.state_bytes,
            "the unchanged group is not rewritten: {added:?}"
        );
    }

    /// Issue #4397 AC-3: after an addition, a deletion and a change, the
    /// incrementally maintained state equals a from-scratch derivation.
    #[test]
    fn incremental_state_matches_a_full_rederivation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let (repo, linked) = two_worktree_fixture(temp.path());
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        let assert_matches_fresh = |label: &str| {
            let fresh_dir = temp.path().join(format!("fresh-{label}"));
            let fresh_state = fresh_dir.join("work-events-intake.json");
            let fresh = ingest_project_work_events_paths(
                &repo,
                &fresh_dir.join("works.json"),
                &fresh_state,
            );
            assert!(fresh.projection_rebuilt, "{label}: {fresh:?}");
            assert_eq!(
                load_work_events_intake_state(&state_path),
                load_work_events_intake_state(&fresh_state),
                "{label}"
            );
        };

        let added = write_shard(&linked, "evt-group-added", "work-group-added");
        let addition = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(!addition.projection_rebuilt, "{addition:?}");
        assert_matches_fresh("addition");

        std::fs::remove_file(&added).expect("delete shard");
        let deletion = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(deletion.projection_rebuilt, "{deletion:?}");
        assert_matches_fresh("deletion");

        let main_shard = write_shard(&repo, "evt-group-main", "work-group-main");
        std::fs::File::options()
            .write(true)
            .open(&main_shard)
            .expect("open shard")
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .expect("touch shard");
        let change = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(change.projection_rebuilt, "{change:?}");
        assert_matches_fresh("change");
    }

    /// Issue #4397 AC-5: a legacy flat `work-events-intake.json` migrates in
    /// place — no rebuild, no fingerprint lost — and the next pass is gated.
    #[test]
    fn legacy_flat_intake_state_migrates_without_rebuild() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let (repo, _linked) = two_worktree_fixture(temp.path());
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        let migrated = load_work_events_intake_state(&state_path);
        std::fs::remove_dir_all(temp.path().join("state/work-events-intake"))
            .expect("drop grouped store");
        std::fs::write(&state_path, serde_json::to_vec_pretty(&migrated).unwrap())
            .expect("legacy state");

        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(!first.projection_rebuilt, "{first:?}");
        assert_eq!(first.sources_ingested, 0, "{first:?}");
        assert_eq!(first.sources_rederived, 2, "{first:?}");
        assert!(!state_path.exists(), "legacy file retired after migration");
        assert_eq!(load_work_events_intake_state(&state_path), migrated);

        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(second.sources_rederived, 0, "{second:?}");
        assert_eq!(second.state_bytes_written, 0, "{second:?}");
    }

    #[test]
    fn shard_validation_rejects_digest_in_wrong_bucket() {
        let event = event_line(
            "evt-wrong-bucket",
            "work-wrong-bucket",
            "Wrong bucket",
            "2026-08-13T01:30:00Z",
        );
        let hash = format!("{:x}", sha2::Sha256::digest(b"evt-wrong-bucket"));
        let wrong_bucket = if &hash[..2] == "00" { "01" } else { "00" };
        let path = Path::new(EVENTS_TREE_DIR)
            .join(wrong_bucket)
            .join(format!("{hash}.jsonl"));

        let error = validate_work_event_shard(&path, format!("{event}\n").as_bytes())
            .expect_err("wrong digest bucket must fail closed");
        assert!(error.to_string().contains("bucket"), "{error}");
    }

    #[test]
    fn first_run_empty_source_set_preserves_existing_projection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let now = chrono::Utc::now();
        let mut projection = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
        projection.apply_event(gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            "work-first-run-preserved",
            now,
        ));
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &work_items_path,
            &projection,
        )
        .expect("seed projection");
        let projection_before = std::fs::read(&work_items_path).expect("projection before");
        let state = WorkEventsIntakeState::default();
        save_work_events_intake_state(&state_path, &state).expect("seed state without source list");
        let state_before = intake_state_files(&state_path);

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(intake_state_files(&state_path), state_before);
    }

    #[test]
    fn origin_ref_discovery_ignores_recognized_writer_temp_residue() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/ref-temp-residue"])
            .current_dir(&repo));
        let id = "evt-ref-temp-residue";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let events_dir = repo.join(EVENTS_TREE_DIR);
        std::fs::create_dir_all(&events_dir).expect("event store");
        std::fs::write(
            events_dir.join(format!("{hash}.jsonl")),
            format!(
                "{}\n",
                event_line(
                    id,
                    "work-ref-temp-residue",
                    "Ref writer temp residue",
                    "2026-08-12T07:05:00Z",
                )
            ),
        )
        .expect("canonical flat compatibility shard");
        let residue = events_dir.join(format!(".{hash}.jsonl.create-123-concurrent"));
        std::fs::write(&residue, b"incomplete writer temp bytes").expect("writer temp residue");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["add", "-f"])
            .arg(&residue)
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "ref shard with temp residue"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "refs/remotes/origin/work/ref-temp-residue",
                "HEAD",
            ])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(&repo));
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "recognized temp is ignored: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-ref-temp-residue"));
    }

    #[test]
    fn local_source_discovery_ignores_only_recognized_writer_temp_residue() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let id = "evt-temp-residue";
        let hash = format!("{:x}", sha2::Sha256::digest(id.as_bytes()));
        let events_dir = repo.join(EVENTS_TREE_DIR);
        std::fs::create_dir_all(&events_dir).expect("event store");
        std::fs::write(
            events_dir.join(format!("{hash}.jsonl")),
            format!(
                "{}\n",
                event_line(
                    id,
                    "work-temp-residue",
                    "Writer temp residue",
                    "2026-08-12T07:00:00Z",
                )
            ),
        )
        .expect("canonical shard");
        std::fs::write(
            events_dir.join(format!(".{hash}.jsonl.create-123-concurrent")),
            b"incomplete writer temp bytes",
        )
        .expect("writer temp residue");
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            summary.projection_rebuilt,
            "recognized temp is ignored: {summary:?}"
        );
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-temp-residue"));
    }

    #[test]
    fn projection_parse_failure_requires_rebuild_with_current_version_and_fingerprints() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let events_path = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(events_path.parent().unwrap()).expect("work event dir");
        let content = format!(
            "{}\n",
            event_line(
                "evt-parse-recovery",
                "work-parse-recovery",
                "Projection parse recovery",
                "2026-07-16T07:00:00Z"
            )
        );
        std::fs::write(&events_path, &content).expect("shared event");

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let initial = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(initial.projection_rebuilt);

        let state = load_work_events_intake_state(&state_path);
        assert!(state.projection_is_current(SOURCE_CONTEXT_FINGERPRINT_VERSION));
        let current = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(!current.projection_rebuilt);
        assert_eq!(current.sources_ingested, 0);
        assert!(
            current.sources_skipped >= 1,
            "fingerprint skip: {current:?}"
        );

        std::fs::write(&work_items_path, b"{\"work_items\":")
            .expect("syntactically corrupt projection");

        let recovered = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            recovered.projection_rebuilt,
            "projection parse failure must override current cache state: {recovered:?}"
        );
        assert_eq!(recovered.sources_ingested, 1);
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load recovered projection")
                .expect("recovered projection");
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-parse-recovery"));
    }

    #[test]
    fn valid_incompatible_projection_does_not_rebuild_or_advance_intake_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let events_path = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(events_path.parent().unwrap()).expect("work event dir");
        std::fs::write(
            &events_path,
            format!(
                "{}\n",
                event_line(
                    "evt-incompatible-source",
                    "work-incompatible-source",
                    "Incompatible source",
                    "2026-07-16T08:00:00Z"
                )
            ),
        )
        .expect("shared event");

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let initial = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(initial.projection_rebuilt);

        let loaded =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load initial projection")
                .expect("initial projection");
        let mut incompatible = serde_json::to_value(&loaded).expect("projection json");
        incompatible["work_items"][0]["events"][0]
            .as_object_mut()
            .expect("Work event object")
            .insert(
                "future_schema_field".to_string(),
                serde_json::json!({ "preserve": true }),
            );
        let original_projection =
            serde_json::to_vec_pretty(&incompatible).expect("incompatible json");
        std::fs::write(&work_items_path, &original_projection)
            .expect("write incompatible projection");
        let original_state = intake_state_files(&state_path);
        let initial_source = std::fs::read_to_string(&events_path).expect("read initial source");
        std::fs::write(
            &events_path,
            format!(
                "{}{}\n",
                initial_source,
                event_line(
                    "evt-after-incompatible",
                    "work-after-incompatible",
                    "Must not advance",
                    "2026-07-16T09:00:00Z"
                )
            ),
        )
        .expect("advance shared event source");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must fail closed: {summary:?}");
        assert_eq!(summary.sources_ingested, 0, "must fail closed: {summary:?}");
        assert_eq!(summary.events_applied, 0, "must fail closed: {summary:?}");
        assert_eq!(
            std::fs::read(&work_items_path).expect("read preserved projection"),
            original_projection
        );
        assert_eq!(intake_state_files(&state_path), original_state);
    }

    #[test]
    fn ingest_reprocesses_old_raw_fingerprint_state_to_repair_source_container() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/cache-repair"])
            .current_dir(&repo));
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("mk .gwt/work");
        std::fs::write(
            repo.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n",
                event_line(
                    "evt-cache-repair",
                    "work-cache-repair-dddd4444",
                    "cache repair",
                    "2026-06-03T10:00:00Z"
                )
            ),
        )
        .expect("write ref events");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events.jsonl"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "cache repair events"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-ref",
                "refs/remotes/origin/work/cache-repair",
                "HEAD",
            ])
            .current_dir(&repo));

        let refs = gwt_git::refs::list_origin_refs_with_commit(&repo).expect("origin refs");
        let (refname, commit) = refs
            .iter()
            .find(|(refname, _)| refname == "refs/remotes/origin/work/cache-repair")
            .expect("cache repair ref");
        let oid = gwt_git::blob::events_blob_oids_batch(
            &repo,
            std::slice::from_ref(commit),
            EVENTS_TREE_PATH,
        )
        .expect("blob oid")
        .pop()
        .flatten()
        .expect("events blob oid");
        let legacy_content = gwt_git::blob::read_blob(&repo, &oid).expect("blob content");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");

        ingest_work_events_content(&work_items_path, &legacy_content)
            .expect("legacy branch-less ingest");
        let legacy_projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load legacy")
                .expect("legacy projection");
        assert!(
            legacy_projection.work_items[0]
                .execution_containers
                .is_empty(),
            "pre-fix projection starts without branch context"
        );

        let mut old_state = WorkEventsIntakeState::default();
        old_state.record(format!("{SOURCE_REF}{refname}"), oid);
        save_work_events_intake_state(&state_path, &old_state).expect("old state");

        let repaired = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(
            repaired.events_applied, 1,
            "old raw fingerprint cache must not skip source-context repair"
        );

        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load repaired")
                .expect("repaired projection");
        assert!(projection.work_items[0]
            .execution_containers
            .iter()
            .any(|container| container.branch.as_deref() == Some("work/cache-repair")));

        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(second.events_applied, 0);
        assert_eq!(second.sources_ingested, 0);
    }

    #[test]
    fn source_fingerprint_invalidates_pre_deterministic_duplicate_cache_entries() {
        let container = WorkspaceExecutionContainerRef {
            branch: Some("feature/spec-3273".to_string()),
            worktree_path: Some("/repo/feature/spec-3273".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let raw_fingerprint = content_fingerprint("event content");
        let container_fingerprint = serde_json::to_string(&container).unwrap();
        let pre_deterministic_duplicate = content_fingerprint(&format!(
            "source-context-v2-global-order\n{raw_fingerprint}\n{container_fingerprint}"
        ));
        let pre_durable_rebuild = content_fingerprint(&format!(
            "source-context-v4-projection-rebuild\n{raw_fingerprint}\n{container_fingerprint}"
        ));
        let pre_complete_transaction = content_fingerprint(&format!(
            "source-context-v5-durable-chronological-rebuild\n{raw_fingerprint}\n{container_fingerprint}"
        ));

        assert_ne!(
            source_fingerprint(&raw_fingerprint, Some(&container)),
            pre_deterministic_duplicate,
            "the deterministic duplicate-fold upgrade must force one full-source re-ingest"
        );
        assert_ne!(
            source_fingerprint(&raw_fingerprint, Some(&container)),
            pre_durable_rebuild,
            "the durable chronological fold must invalidate the v4 projection once"
        );
        assert_ne!(
            source_fingerprint(&raw_fingerprint, Some(&container)),
            pre_complete_transaction,
            "the complete transaction boundary must invalidate the v5 projection once"
        );
        assert_ne!(
            source_fingerprint(&raw_fingerprint, None),
            raw_fingerprint,
            "container-less sources must also carry the fold semantics version"
        );
    }

    #[test]
    fn version_mismatch_rebuilds_polluted_projection_with_local_close_state() {
        use chrono::{TimeZone, Utc};
        use gwt_core::workspace_projection::{
            load_workspace_work_items_from_path, save_workspace_work_items_projection_to_path,
            WorkEvent, WorkEventKind, WorkItemsProjection, WorkspaceStatusCategory,
        };

        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("work event dir");

        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 1, 0).unwrap();
        let done_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let polluted_at = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let repo_container = WorkspaceExecutionContainerRef {
            branch: Some("main".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.id = "evt-owner".to_string();
        owner.title = Some("Owner work".to_string());
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut target = WorkEvent::new(WorkEventKind::Start, "work-target", t1);
        target.id = "evt-target".to_string();
        target.title = Some("Canonical target".to_string());
        target.agent_session_id = Some("session-target".to_string());
        target.execution_container = Some(repo_container.clone());
        std::fs::write(
            repo.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&owner).unwrap(),
                serde_json::to_string(&target).unwrap()
            ),
        )
        .expect("shared event log");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let close_path = temp.path().join("state/work-events-closed.jsonl");
        std::fs::create_dir_all(close_path.parent().unwrap()).expect("state dir");
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-target", done_at);
        done.id = "evt-done".to_string();
        done.status_category = Some(WorkspaceStatusCategory::Done);
        std::fs::write(
            &close_path,
            format!("{}\n", serde_json::to_string(&done).unwrap()),
        )
        .expect("close log");

        let mut polluted = WorkItemsProjection::empty(t0);
        polluted.apply_event(owner);
        polluted.apply_event(target);
        polluted.apply_event(done);
        let mut legacy = WorkEvent::new(WorkEventKind::Backfill, "work-eventless", t0);
        legacy.title = Some("Eventless legacy work".to_string());
        polluted.apply_event(legacy);
        polluted
            .work_items
            .iter_mut()
            .find(|item| item.id == "work-eventless")
            .unwrap()
            .events
            .clear();

        let owner_agent = polluted
            .work_items
            .iter()
            .find(|item| item.id == "work-owner")
            .unwrap()
            .agents[0]
            .clone();
        let target_item = polluted
            .work_items
            .iter_mut()
            .find(|item| item.id == "work-target")
            .unwrap();
        let mut stray = WorkEvent::new(WorkEventKind::Update, "work-target", polluted_at);
        stray.id = "evt-stray-old-fold".to_string();
        stray.title = Some("Foreign target".to_string());
        stray.status_category = Some(WorkspaceStatusCategory::Active);
        stray.agent_session_id = Some("session-owner".to_string());
        stray.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        target_item.title = "Foreign target".to_string();
        target_item.status_category = WorkspaceStatusCategory::Active;
        target_item.completed_at = None;
        target_item.updated_at = polluted_at;
        target_item.agents.push(owner_agent);
        target_item
            .execution_containers
            .push(stray.execution_container.clone().unwrap());
        target_item.events.push(stray);
        save_workspace_work_items_projection_to_path(&work_items_path, &polluted).unwrap();

        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(first.sources_ingested, 2);
        let rebuilt = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let target = rebuilt
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert_eq!(target.title, "Canonical target");
        assert_eq!(target.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(target.completed_at, Some(done_at));
        assert!(target
            .agents
            .iter()
            .all(|agent| agent.session_id != "session-owner"));
        assert!(target
            .execution_containers
            .iter()
            .all(|container| container.branch.as_deref() != Some("feature/foreign")));
        assert!(rebuilt
            .work_items
            .iter()
            .any(|item| item.id == "work-eventless" && item.events.is_empty()));

        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(second.events_applied, 0);
        assert_eq!(second.sources_ingested, 0);
    }

    #[test]
    fn ingest_folds_all_sources_globally_before_resolving_session_owner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        let owner_worktree = temp.path().join("owner-worktree");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        run(gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-b", "work/owner"])
            .arg(&owner_worktree)
            .current_dir(&repo));

        std::fs::create_dir_all(repo.join(".gwt/work")).expect("main work dir");
        std::fs::write(
            repo.join(".gwt/work/events.jsonl"),
            format!(
                "{}\n",
                session_event_line(SessionEventFixture {
                    id: "evt-stray",
                    work_id: "work-target",
                    kind: "update",
                    title: "Foreign title",
                    session_id: "session-owner",
                    branch: "feature/foreign",
                    worktree_path: &repo,
                    updated_at: "2026-07-15T09:00:00Z",
                })
            ),
        )
        .expect("write stray source");

        std::fs::create_dir_all(owner_worktree.join(".gwt/work")).expect("owner work dir");
        std::fs::write(
            owner_worktree.join(".gwt/work/events.jsonl"),
            [
                session_event_line(SessionEventFixture {
                    id: "evt-owner",
                    work_id: "work-owner",
                    kind: "start",
                    title: "Owner work",
                    session_id: "session-owner",
                    branch: "work/owner",
                    worktree_path: &owner_worktree,
                    updated_at: "2026-07-15T07:00:00Z",
                }),
                session_event_line(SessionEventFixture {
                    id: "evt-target",
                    work_id: "work-target",
                    kind: "start",
                    title: "Target work",
                    session_id: "session-target",
                    branch: "feature/spec-3273",
                    worktree_path: &repo,
                    updated_at: "2026-07-15T07:00:01Z",
                }),
            ]
            .join("\n"),
        )
        .expect("write canonical source");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(summary.sources_ingested, 2);
        assert_eq!(summary.events_applied, 2, "stray event must be rejected");

        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        let owner = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-owner")
            .expect("canonical owner must survive source ordering");
        assert!(owner
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-owner"));

        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .expect("target work");
        assert_eq!(target.title, "Target work");
        assert!(target
            .agents
            .iter()
            .all(|agent| agent.session_id != "session-owner"));
        assert!(target
            .execution_containers
            .iter()
            .all(|container| container.branch.as_deref() != Some("feature/foreign")));
    }

    #[test]
    fn missing_projection_rebuilds_from_machine_local_log_without_shared_sources() {
        use chrono::{TimeZone, Utc};
        use gwt_core::workspace_projection::{
            load_workspace_work_items_from_path, WorkEvent, WorkEventKind, WorkspaceStatusCategory,
        };

        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("complete local source discovery");

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let close_path = state_dir.join("work-events-closed.jsonl");
        std::fs::create_dir_all(&state_dir).expect("state dir");
        let done_at = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-close-only", done_at);
        done.status_category = Some(WorkspaceStatusCategory::Done);
        std::fs::write(
            &close_path,
            format!("{}\n", serde_json::to_string(&done).unwrap()),
        )
        .expect("close log");

        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt);
        assert_eq!(first.sources_ingested, 1);
        let projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .expect("close-only projection");
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-close-only")
            .expect("close-only Work");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(done_at));

        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(!second.projection_rebuilt);
        assert_eq!(second.events_applied, 0);
    }

    #[test]
    fn rebuild_records_local_lifecycle_created_after_source_discovery() {
        use chrono::{TimeZone, Utc};
        use gwt_core::work_events_intake::{content_fingerprint, load_work_events_intake_state};
        use gwt_core::workspace_projection::{WorkEvent, WorkEventKind, WorkspaceStatusCategory};

        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let events_path = repo.join(".gwt/work/events.jsonl");
        std::fs::create_dir_all(events_path.parent().unwrap()).unwrap();
        std::fs::write(
            &events_path,
            format!(
                "{}\n",
                event_line(
                    "evt-start",
                    "work-racing-close",
                    "Racing close",
                    "2026-07-15T07:00:00Z"
                )
            ),
        )
        .unwrap();

        let state_dir = temp.path().join("state");
        let works = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let close_path = state_dir.join("work-events-closed.jsonl");
        let close_for_callback = close_path.clone();
        let mut done = WorkEvent::new(
            WorkEventKind::Done,
            "work-racing-close",
            Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap(),
        );
        done.status_category = Some(WorkspaceStatusCategory::Done);
        let close_content = format!("{}\n", serde_json::to_string(&done).unwrap());
        let callback_content = close_content.clone();

        let first = ingest_project_work_events_paths_with_before_intake(
            &repo,
            &works,
            &state_path,
            move || {
                std::fs::create_dir_all(close_for_callback.parent().unwrap()).unwrap();
                std::fs::write(close_for_callback, callback_content).unwrap();
            },
        );
        assert!(first.projection_rebuilt);
        assert_eq!(first.sources_ingested, 2);

        let key = format!("{SOURCE_LOCAL_LIFECYCLE}{}", close_path.display());
        let state = load_work_events_intake_state(&state_path);
        assert!(state.is_current(&key, &content_fingerprint(&close_content)));

        let second = ingest_project_work_events_paths(&repo, &works, &state_path);
        assert_eq!(second.sources_ingested, 0);
        assert_eq!(second.events_applied, 0);
    }

    #[test]
    fn detached_containerless_source_rebuilds_once_then_is_fingerprint_current() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "--detach", "HEAD"])
            .current_dir(&repo));
        let events_path = repo.join(".gwt/work/events.jsonl");
        std::fs::create_dir_all(events_path.parent().unwrap()).expect("work event dir");
        let content = format!(
            "{}\n",
            event_line(
                "evt-detached",
                "work-detached",
                "Detached source",
                "2026-07-15T07:00:00Z"
            )
        );
        std::fs::write(&events_path, &content).expect("detached events");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let key = format!("{SOURCE_WORKTREE}{}", events_path.display());
        let mut stale_state = WorkEventsIntakeState::default();
        stale_state.record(
            key,
            source_fingerprint(&content_fingerprint(&content), None),
        );
        save_work_events_intake_state(&state_path, &stale_state).expect("stale state");

        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt);
        assert_eq!(first.sources_ingested, 1);
        assert_eq!(first.events_applied, 1);

        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(!second.projection_rebuilt);
        assert_eq!(second.sources_ingested, 0);
        assert_eq!(second.events_applied, 0);
        assert_eq!(second.sources_skipped, 1);
    }

    #[test]
    fn appended_local_lifecycle_event_recovers_when_projection_save_was_missed() {
        use chrono::{TimeZone, Utc};
        use gwt_core::workspace_projection::{
            load_workspace_work_items_from_path, WorkEvent, WorkEventKind, WorkspaceStatusCategory,
        };

        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let events_path = repo.join(".gwt/work/events.jsonl");
        std::fs::create_dir_all(events_path.parent().unwrap()).expect("work event dir");
        std::fs::write(
            &events_path,
            format!(
                "{}\n",
                event_line(
                    "evt-start-durable",
                    "work-durable-recovery",
                    "Durable recovery",
                    "2026-07-15T07:00:00Z"
                )
            ),
        )
        .expect("shared event");

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let close_path = state_dir.join("work-events-closed.jsonl");
        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt);

        let done_at = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-durable-recovery", done_at);
        done.id = "evt-done-durable".to_string();
        done.status_category = Some(WorkspaceStatusCategory::Done);
        std::fs::write(
            &close_path,
            format!("{}\n", serde_json::to_string(&done).unwrap()),
        )
        .expect("durable event appended without projection save");

        let recovered = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(recovered.events_applied, 1);
        let projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-durable-recovery")
            .unwrap();
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(done_at));

        let current = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(current.events_applied, 0);
        assert_eq!(current.sources_ingested, 0);
    }

    #[test]
    fn rebuild_reloads_local_shared_source_after_discovery_before_taking_lock() {
        use chrono::{TimeZone, Utc};
        use gwt_core::workspace_projection::{
            load_workspace_work_items_from_path, record_workspace_work_event_paths, WorkEvent,
            WorkEventKind, WorkspaceStatusCategory,
        };

        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let events_path = repo.join(".gwt/work/events.jsonl");
        std::fs::create_dir_all(events_path.parent().unwrap()).expect("work event dir");
        std::fs::write(
            &events_path,
            format!(
                "{}\n",
                event_line(
                    "evt-before-discovery",
                    "work-rebuild-race",
                    "Before discovery",
                    "2026-07-15T07:00:00Z"
                )
            ),
        )
        .unwrap();

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let writer_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let summary = ingest_project_work_events_paths_with_before_intake(
            &repo,
            &work_items_path,
            &state_path,
            || {
                let mut writer =
                    WorkEvent::new(WorkEventKind::Update, "work-rebuild-race", writer_at);
                writer.id = "evt-writer-before-lock".to_string();
                writer.title = Some("Writer survived rebuild".to_string());
                writer.status_category = Some(WorkspaceStatusCategory::Active);
                record_workspace_work_event_paths(&work_items_path, &events_path, writer).unwrap();
            },
        );

        assert!(summary.projection_rebuilt);
        let projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-rebuild-race")
            .unwrap();
        assert_eq!(item.title, "Writer survived rebuild");
        assert!(item
            .events
            .iter()
            .any(|event| event.id == "evt-writer-before-lock"));

        let current = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(current.sources_ingested, 0);
        assert_eq!(current.events_applied, 0);
        assert!(current.sources_skipped >= 1);
    }

    #[test]
    fn rebuild_discovers_bucketed_shard_created_after_initial_scan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        let legacy_path = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(legacy_path.parent().unwrap()).expect("work dir");
        std::fs::write(
            &legacy_path,
            format!(
                "{}\n",
                event_line(
                    "evt-before-new-shard",
                    "work-shard-race-before",
                    "Before shard race",
                    "2026-08-13T02:00:00Z"
                )
            ),
        )
        .expect("legacy event");

        let raced_event = event_line(
            "evt-created-before-lock",
            "work-shard-race-after",
            "Shard created before lock",
            "2026-08-13T02:01:00Z",
        );
        let hash = format!("{:x}", sha2::Sha256::digest(b"evt-created-before-lock"));
        let raced_path = repo
            .join(EVENTS_TREE_DIR)
            .join(&hash[..2])
            .join(format!("{hash}.jsonl"));
        let raced_path_for_callback = raced_path.clone();
        let raced_content = format!("{raced_event}\n");

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let summary = ingest_project_work_events_paths_with_before_intake(
            &repo,
            &work_items_path,
            &state_path,
            move || {
                std::fs::create_dir_all(raced_path_for_callback.parent().unwrap())
                    .expect("bucket dir");
                std::fs::write(raced_path_for_callback, raced_content).expect("raced shard");
            },
        );

        assert!(summary.projection_rebuilt, "initial rebuild: {summary:?}");
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load")
                .expect("projection");
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-shard-race-after"));
        let state = load_work_events_intake_state(&state_path);
        let raced_filename = raced_path.file_name().unwrap().to_string_lossy();
        assert!(
            state.sources.keys().any(|key| {
                key.starts_with(SOURCE_WORKTREE) && key.ends_with(raced_filename.as_ref())
            }),
            "raced shard fingerprint must be durable"
        );
    }

    #[test]
    fn version_rebuild_replaces_stale_source_state_so_restored_source_is_ingested() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        let restored_worktree = temp.path().join("restored-worktree");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        let main_events = repo.join(".gwt/work/events.jsonl");
        std::fs::create_dir_all(main_events.parent().unwrap()).expect("main event dir");
        std::fs::write(
            &main_events,
            format!(
                "{}\n",
                event_line(
                    "evt-main-rebuild",
                    "work-main-rebuild",
                    "Main rebuild source",
                    "2026-07-15T07:00:00Z"
                )
            ),
        )
        .unwrap();

        let restored_content = format!(
            "{}\n",
            event_line(
                "evt-restored-source",
                "work-restored-source",
                "Restored source",
                "2026-07-15T08:00:00Z"
            )
        );
        let restored_events = restored_worktree.join(EVENTS_TREE_PATH);
        let restored_key = format!("{SOURCE_WORKTREE}{}", restored_events.display());
        let restored_container = WorkspaceExecutionContainerRef {
            branch: Some("work/restored-source".to_string()),
            worktree_path: Some(restored_worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let restored_fingerprint = source_fingerprint(
            &content_fingerprint(&restored_content),
            Some(&restored_container),
        );

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let mut stale = WorkEventsIntakeState::default();
        stale.record(restored_key.clone(), restored_fingerprint);
        stale.record_projection_version("source-context-v5-durable-chronological-rebuild");
        save_work_events_intake_state(&state_path, &stale).unwrap();

        let rebuilt = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(rebuilt.projection_rebuilt);
        let rebuilt_state = load_work_events_intake_state(&state_path);
        assert!(
            !rebuilt_state.sources.contains_key(&restored_key),
            "a source not folded by rebuild must not retain its old current fingerprint"
        );

        run(gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-b", "work/restored-source"])
            .arg(&restored_worktree)
            .current_dir(&repo));
        std::fs::create_dir_all(restored_events.parent().unwrap()).unwrap();
        std::fs::write(&restored_events, restored_content).unwrap();

        let restored = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(
            !restored.projection_rebuilt,
            "immutable source addition is incremental: {restored:?}"
        );
        assert_eq!(restored.sources_ingested, 1);
        assert_eq!(restored.events_applied, 1);
        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .unwrap()
                .unwrap();
        assert!(projection
            .work_items
            .iter()
            .any(|item| item.id == "work-restored-source"));
    }

    #[test]
    fn version_rebuild_defers_when_one_discovered_source_is_unreadable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        let side_worktree = temp.path().join("side-worktree");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-b", "work/unreadable-source"])
            .arg(&side_worktree)
            .current_dir(&repo));

        let main_events = repo.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(main_events.parent().unwrap()).expect("main event dir");
        std::fs::write(
            &main_events,
            format!(
                "{}\n",
                event_line(
                    "evt-readable-source",
                    "work-readable-source",
                    "Readable source",
                    "2026-07-15T07:00:00Z"
                )
            ),
        )
        .expect("main event");

        let unreadable_events = side_worktree.join(EVENTS_TREE_PATH);
        std::fs::create_dir_all(unreadable_events.parent().unwrap()).expect("side event dir");
        std::fs::write(
            &unreadable_events,
            format!(
                "{}\n",
                event_line(
                    "evt-unreadable-source",
                    "work-unreadable-source",
                    "Unreadable source",
                    "2026-07-15T08:00:00Z"
                )
            ),
        )
        .expect("side event");

        let state_dir = temp.path().join("state");
        let work_items_path = state_dir.join("works.json");
        let state_path = state_dir.join("work-events-intake.json");
        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt);

        let mut stale = load_work_events_intake_state(&state_path);
        stale.record_projection_version("source-context-v5-durable-chronological-rebuild");
        save_work_events_intake_state(&state_path, &stale).expect("stale state");

        std::fs::remove_file(&unreadable_events).expect("remove side event");
        std::fs::create_dir(&unreadable_events).expect("make side source unreadable");

        let deferred = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(
            !deferred.projection_rebuilt,
            "a partial source set must not replace the existing projection"
        );

        let projection =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load projection")
                .expect("projection");
        let unreadable = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-unreadable-source")
            .expect("unreadable source Work must survive deferred rebuild");
        assert!(unreadable
            .events
            .iter()
            .any(|event| event.id == "evt-unreadable-source"));
        assert!(
            !load_work_events_intake_state(&state_path)
                .projection_is_current(SOURCE_CONTEXT_FINGERPRINT_VERSION),
            "the incomplete rebuild must remain pending for a later retry"
        );
    }
}
