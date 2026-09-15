//! SPEC-2359 W-16 (FR-387/FR-388): project-level work events ingest
//! orchestrator.
//!
//! Collects legacy `.gwt/work/events.jsonl` and canonical immutable shards
//! below `.gwt/work/events/` from every reachable source —
//! local worktree filesystems (the base/main checkout included) and fetched
//! `origin/*` refs (checkout-free blob reads) — and funnels each through the
//! idempotent gwt-core intake into the home works projection. A fingerprint
//! cache (`work-events-intake.json`) skips unchanged sources; deleting it is
//! always safe (dedup is event-id based, SC-260). After first validation,
//! immutable local shards and the frozen legacy logs use size/mtime/container
//! metadata to avoid payload I/O on the 30-second unchanged poll; metadata
//! changes force revalidation.
//!
//! Git blob contents are OID-deduplicated and read in one `cat-file --batch`;
//! tree enumeration is checkout-free and unique-commit deduplicated. Callers
//! run this off the UI thread.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
use gwt_core::work_events_intake::ingest_work_events_content;
use gwt_core::work_events_intake::{
    content_fingerprint, ingest_work_event_sources_with_local_path, ingest_work_events_sources,
    load_work_events_intake_state, rebuild_work_events_with_shared_loader,
    save_work_events_intake_state_incremental, SharedWorkEventsSource,
};
use gwt_core::workspace_projection::WorkspaceExecutionContainerRef;
use sha2::{Digest, Sha256};

/// Where one ingested chunk of content came from (cache key prefix).
const SOURCE_WORKTREE: &str = "worktree:";
const SOURCE_REF: &str = "ref:";
const SOURCE_LOCAL_LIFECYCLE: &str = "local-lifecycle:";
const SOURCE_LIST: &str = "source-list:v1";

/// Issue #4397: intake state key prefixes for the group generations that let a
/// trigger prove a whole set of sources unchanged without deriving a
/// fingerprint per member.
const GROUP_WORKTREE: &str = "worktree-group:";
const GROUP_REF: &str = "ref-group:";

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
    /// Issue #4397 AC-4: source fingerprints derived during this trigger.
    ///
    /// A trigger that changed nothing derives none: every group proved itself
    /// unchanged from its own generation token. This is the number the intake
    /// used to pay unconditionally, once per known source.
    pub sources_rederived: usize,
    /// Bytes this trigger handed to the intake state store.
    pub state_bytes_written: u64,
    /// Size of the persisted intake state after this trigger.
    pub state_bytes: u64,
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

#[derive(Debug)]
struct LocalImmutableSource {
    source: WorkEventsSource,
    key: String,
    fingerprint: String,
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
        match source.events_path.try_exists() {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => return Err(error.into()),
        }
        // Metadata before content: a write racing this read leaves the older
        // fingerprint behind, so the next pass reads the source again.
        let fingerprint =
            local_immutable_source_fingerprint(&source.events_path, source.container.as_ref())?;
        let content = read_work_event_source(&source.events_path, source.kind)?;
        let key = format!("{SOURCE_WORKTREE}{}", source.events_path.display());
        fingerprints.push((key, fingerprint));
        contents.push(SharedWorkEventsSource::new(content, source.container));
    }
    Ok((contents, fingerprints))
}

/// Paths-injected ingest (#3022): all writes go to `work_items_path` /
/// `state_path`. Source discovery/read failures are logged and skipped during
/// incremental intake. An authoritative rebuild is deferred unless every
/// discovered source was readable, so a partial snapshot cannot erase history.
pub fn ingest_project_work_events_paths(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
) -> WorkEventsIngestSummary {
    ingest_project_work_events_paths_inner(project_root, work_items_path, state_path, || {}, |_| {})
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
        || {},
        before_source_read,
    )
}

fn ingest_project_work_events_paths_inner<F, R>(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
    before_intake: F,
    mut before_source_read: R,
) -> WorkEventsIngestSummary
where
    F: FnOnce(),
    R: FnMut(&Path),
{
    let mut summary = WorkEventsIngestSummary::default();
    let mut state = load_work_events_intake_state(state_path);
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
        || !state.projection_is_current(SOURCE_CONTEXT_FINGERPRINT_VERSION);
    let mut pending_sources = Vec::new();
    let mut current_sources = Vec::new();
    let mut local_immutable_sources = Vec::new();
    let mut source_discovery_failed = false;
    // Group key, generation and member count to record once the pass commits.
    let mut settled_worktree_groups: Vec<(String, String, usize)> = Vec::new();
    let mut settled_ref_groups: Vec<(String, String, usize)> = Vec::new();

    // 1) Local worktree filesystems (base/main checkout included): committed
    //    or not, the working copy is the freshest view of each branch's log.
    let worktree_entries = match gwt::worktree_inventory::enumerate_worktrees(project_root, None) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, "work events ingest: worktree enumeration failed");
            source_discovery_failed = true;
            Vec::new()
        }
    };
    let worktree_groups = match worktree_event_source_groups(&worktree_entries) {
        Ok(groups) => groups,
        Err(error) => {
            tracing::warn!(%error, "work events ingest: worktree event source discovery failed");
            source_discovery_failed = true;
            Vec::new()
        }
    };

    // Issue #4397: a shard-store directory whose generation still matches is
    // proof that none of its members moved. Carrying its recorded fingerprints
    // forward untouched is what keeps an unchanged trigger from deriving one
    // fingerprint per known shard.
    let live_worktree_groups = worktree_groups
        .iter()
        .map(|group| group.key.clone())
        .collect::<HashSet<_>>();
    let vanished_worktree_groups = state
        .groups
        .keys()
        .filter(|key| key.starts_with(GROUP_WORKTREE) && !live_worktree_groups.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    for key in vanished_worktree_groups {
        // The directory is gone: so are the sources it held. Leaving their
        // fingerprints behind would keep the snapshot claiming sources that no
        // longer exist, and a later restore would look current and be skipped.
        rebuild_required = true;
        let path = key[GROUP_WORKTREE.len()..].to_string();
        state.remove_source(&format!("{SOURCE_WORKTREE}{path}"));
        for member in group_member_keys(&state, &format!("{SOURCE_WORKTREE}{path}/"), true) {
            state.remove_source(&member);
        }
        state.remove_group(&key);
    }

    for group in worktree_groups {
        let group_is_current = !group.always_derive
            && !rebuild_required
            && state.group_is_current(&group.key, &group.generation);
        if group_is_current {
            summary.sources_skipped += state
                .group(&group.key)
                .map(|recorded| recorded.members)
                .unwrap_or_default();
            continue;
        }
        let mut derived = Vec::new();
        let mut scan_failed = false;
        for source in group.sources {
            let events_path = source.events_path.clone();
            match events_path.try_exists() {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    tracing::warn!(%error, path = %events_path.display(), "work events ingest: worktree source discovery failed");
                    source_discovery_failed = true;
                    scan_failed = true;
                    continue;
                }
            }
            let key = format!("{SOURCE_WORKTREE}{}", events_path.display());
            // Issue #4371: the frozen legacy log is judged by metadata like a
            // shard. Reading all 239 of them (398.5 MB) on every trigger only
            // to hash them was the largest cost of an unchanged pass.
            match local_immutable_source_fingerprint(&events_path, source.container.as_ref()) {
                Ok(fingerprint) => {
                    summary.sources_rederived += 1;
                    derived.push(LocalImmutableSource {
                        source,
                        key,
                        fingerprint,
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, path = %events_path.display(), "work events ingest: worktree source metadata read failed");
                    source_discovery_failed = true;
                    scan_failed = true;
                }
            }
        }

        // Within a re-scanned group the previously recorded members are known
        // exactly, so a member that changed or disappeared still forces the
        // rebuild it always did.
        let derived_by_key = derived
            .iter()
            .map(|source| (source.key.as_str(), source.fingerprint.as_str()))
            .collect::<HashMap<_, _>>();
        let mut stale = Vec::new();
        for key in group_member_keys(&state, &group.source_prefix, group.direct_children_only) {
            let recorded = state.sources.get(&key).map(String::as_str);
            if derived_by_key.get(key.as_str()).copied() != recorded {
                rebuild_required = true;
                if !derived_by_key.contains_key(key.as_str()) {
                    stale.push(key);
                }
            }
        }
        for key in stale {
            state.remove_source(&key);
        }
        if !scan_failed {
            settled_worktree_groups.push((group.key, group.generation, derived.len()));
        }
        local_immutable_sources.extend(derived);
    }

    for source in local_immutable_sources {
        if !rebuild_required && state.is_current(&source.key, &source.fingerprint) {
            current_sources.push((source.key, source.fingerprint));
            summary.sources_skipped += 1;
            continue;
        }
        before_source_read(&source.source.events_path);
        let content = match read_work_event_source(&source.source.events_path, source.source.kind) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(%error, path = %source.source.events_path.display(), "work events ingest: worktree shard read failed");
                source_discovery_failed = true;
                continue;
            }
        };
        pending_sources.push(PendingWorkEventsSource {
            key: source.key,
            fingerprint: source.fingerprint,
            content,
            container: source.source.container,
            reload_from_worktree: true,
        });
    }

    // 2) Fetched origin/* refs — checkout-free blob reads. Close-kind
    //    filtering inside the intake keeps foreign close state out (FR-384)
    //    and lenient parsing guards against contaminated logs (#3023).
    let mut unread_current_sources = current_sources;
    match gwt_git::refs::list_origin_refs_with_commit(project_root) {
        Ok(all_refs) => {
            // Issue #4397: resolve every ref's Work-event store identity in one
            // batch-check first. A ref whose legacy blob and shard tree both
            // stand still cannot hold a changed event, so it never reaches the
            // tree walk and none of its sources are re-derived. This is what
            // collapses 701 refs' worth of enumeration into the handful that
            // actually moved.
            let commits: Vec<String> = all_refs.iter().map(|(_, sha)| sha.clone()).collect();
            let identities = match gwt_git::blob::work_event_tree_identity_batch(
                project_root,
                &commits,
                EVENTS_TREE_PATH,
                EVENTS_TREE_DIR,
            ) {
                Ok(identities) => identities,
                Err(error) => {
                    tracing::warn!(%error, "work events ingest: ref event store identity failed");
                    source_discovery_failed = true;
                    Vec::new()
                }
            };
            let ref_generations: HashMap<&str, String> = all_refs
                .iter()
                .zip(&identities)
                .map(|((refname, _), identity)| {
                    let container = origin_ref_execution_container(refname);
                    (
                        refname.as_str(),
                        source_fingerprint(
                            &format!("ref-tree-v1:{}", identity.token()),
                            container.as_ref(),
                        ),
                    )
                })
                .collect();

            let live_ref_groups = all_refs
                .iter()
                .map(|(refname, _)| format!("{GROUP_REF}{refname}"))
                .collect::<HashSet<_>>();
            let vanished_ref_groups = state
                .groups
                .keys()
                .filter(|key| key.starts_with(GROUP_REF) && !live_ref_groups.contains(*key))
                .cloned()
                .collect::<Vec<_>>();
            for key in vanished_ref_groups {
                rebuild_required = true;
                let refname = key[GROUP_REF.len()..].to_string();
                for member in group_member_keys(&state, &format!("{SOURCE_REF}{refname}:"), false) {
                    state.remove_source(&member);
                }
                state.remove_group(&key);
            }

            let unchanged_ref = |refname: &str| {
                ref_generations.get(refname).is_some_and(|generation| {
                    state.group_is_current(&format!("{GROUP_REF}{refname}"), generation)
                })
            };
            let refs: Vec<(String, String)> = if rebuild_required || ref_generations.is_empty() {
                all_refs.clone()
            } else {
                all_refs
                    .iter()
                    .filter(|(refname, _)| {
                        if unchanged_ref(refname) {
                            summary.sources_skipped += state
                                .group(&format!("{GROUP_REF}{refname}"))
                                .map(|group| group.members)
                                .unwrap_or_default();
                            return false;
                        }
                        true
                    })
                    .cloned()
                    .collect()
            };
            let commits: Vec<String> = refs.iter().map(|(_, sha)| sha.clone()).collect();
            let mut ref_requires_rebuild = false;
            match gwt_git::blob::work_event_blobs_batch(
                project_root,
                &commits,
                EVENTS_TREE_PATH,
                EVENTS_TREE_DIR,
                |descriptors_by_ref| {
                    let mut discovered = HashMap::new();
                    for ((refname, _), descriptors) in refs.iter().zip(descriptors_by_ref) {
                        let container = origin_ref_execution_container(refname);
                        for descriptor in descriptors {
                            if is_work_event_writer_temp_residue(Path::new(&descriptor.path)) {
                                continue;
                            }
                            let key = format!("{SOURCE_REF}{refname}:{}", descriptor.path);
                            discovered.insert(
                                key,
                                source_fingerprint(&descriptor.oid, container.as_ref()),
                            );
                        }
                    }
                    // Only the enumerated refs can be judged here: a ref that
                    // proved itself unchanged keeps the fingerprints it already
                    // has, and an absent one would otherwise read as deleted.
                    ref_requires_rebuild = refs.iter().any(|(refname, _)| {
                        let prefix = format!("{SOURCE_REF}{refname}:");
                        group_member_keys(&state, &prefix, false)
                            .into_iter()
                            .any(|key| {
                                discovered.get(&key).map(String::as_str)
                                    != state.sources.get(&key).map(String::as_str)
                            })
                    });
                    let mut selected = HashSet::new();
                    for (descriptors, (refname, _)) in descriptors_by_ref.iter().zip(&refs) {
                        let container = origin_ref_execution_container(refname);
                        for descriptor in descriptors {
                            if is_work_event_writer_temp_residue(Path::new(&descriptor.path)) {
                                continue;
                            }
                            let key = format!("{SOURCE_REF}{refname}:{}", descriptor.path);
                            let fingerprint =
                                source_fingerprint(&descriptor.oid, container.as_ref());
                            if rebuild_required
                                || ref_requires_rebuild
                                || !state.is_current(&key, &fingerprint)
                            {
                                selected.insert(descriptor.oid.clone());
                            }
                        }
                    }
                    selected
                },
            ) {
                Ok(blobs_by_ref) => {
                    rebuild_required |= ref_requires_rebuild;
                    let mut shared_content_by_oid_path =
                        HashMap::<(String, String), Result<Arc<str>, String>>::new();
                    for ((refname, _), blobs) in refs.iter().zip(blobs_by_ref) {
                        let container = origin_ref_execution_container(refname);
                        // A re-scanned ref knows its member set exactly, so a
                        // source that disappeared from it is pruned rather than
                        // left behind to look current forever.
                        let prefix = format!("{SOURCE_REF}{refname}:");
                        let live = blobs
                            .iter()
                            .map(|blob| format!("{SOURCE_REF}{refname}:{}", blob.path))
                            .collect::<HashSet<_>>();
                        for key in group_member_keys(&state, &prefix, false) {
                            if !live.contains(&key) {
                                rebuild_required = true;
                                state.remove_source(&key);
                            }
                        }
                        if let Some(generation) = ref_generations.get(refname.as_str()) {
                            settled_ref_groups.push((
                                format!("{GROUP_REF}{refname}"),
                                generation.clone(),
                                live.len(),
                            ));
                        }
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
                            let key = format!("{SOURCE_REF}{refname}:{}", blob.path);
                            let fingerprint = source_fingerprint(&blob.oid, container.as_ref());
                            summary.sources_rederived += 1;
                            let Some(bytes) = blob.content else {
                                unread_current_sources.push((key, fingerprint));
                                summary.sources_skipped += 1;
                                continue;
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
                                container: container.clone(),
                                reload_from_worktree: false,
                            });
                        }
                    }
                }
                Err(error) => {
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

    let had_source_list_fingerprint = state.sources.contains_key(SOURCE_LIST);

    if !rebuild_required {
        pending_sources.retain(|source| {
            if state.is_current(&source.key, &source.fingerprint) {
                summary.sources_skipped += 1;
                false
            } else {
                true
            }
        });
        if pending_local_lifecycle
            .as_ref()
            .is_some_and(|(key, fingerprint)| state.is_current(key, fingerprint))
        {
            summary.sources_skipped += 1;
            pending_local_lifecycle = None;
        }
    }

    // Issue #4397: the tracked source set is the state itself now that each
    // group prunes its own departed members, so the snapshot fingerprint no
    // longer needs a full discovered list to be built, cloned and sorted on
    // every trigger. Recording the discovered fingerprints first is safe
    // because nothing is persisted unless the intake below succeeds.
    for source in &pending_sources {
        state.record(source.key.clone(), source.fingerprint.clone());
    }
    for (key, fingerprint) in unread_current_sources {
        state.record(key, fingerprint);
    }
    let discovered_source_list_fingerprint = tracked_source_list_fingerprint(&state);
    let source_list_changed = !state.is_current(SOURCE_LIST, &discovered_source_list_fingerprint);

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
            // Issue #4397: there is nothing to ingest, but the generations this
            // pass just proved are exactly what lets the next one skip the
            // derivation. A settled pass that persisted nothing would re-derive
            // every source forever.
            for (key, generation, members) in settled_worktree_groups {
                state.record_group(key, generation, members);
            }
            for (key, generation, members) in settled_ref_groups {
                state.record_group(key, generation, members);
            }
            state.record(SOURCE_LIST, discovered_source_list_fingerprint);
            match save_work_events_intake_state_incremental(state_path, &mut state) {
                Ok(write) => {
                    summary.state_bytes_written = write.bytes_written;
                    summary.state_bytes = write.state_bytes;
                }
                Err(error) => {
                    tracing::warn!(%error, "work events ingest: state save failed");
                }
            }
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
            || load_pending_sources_for_rebuild(&pending_sources, &worktree_entries),
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
                // permanently skip its events. Generations are re-established
                // from this pass's own scan; a group whose scan did not settle
                // stays absent and is re-derived next trigger.
                state.clear_sources();
                state.clear_groups();
            }
            for (key, generation, members) in settled_worktree_groups {
                state.record_group(key, generation, members);
            }
            for (key, generation, members) in settled_ref_groups {
                state.record_group(key, generation, members);
            }
            for (key, fingerprint) in shared_fingerprints {
                state.record(key, fingerprint);
            }
            if let (Some(path), Some(fingerprint)) = (close_path.as_ref(), local_fingerprint) {
                state.record(
                    format!("{SOURCE_LOCAL_LIFECYCLE}{}", path.display()),
                    fingerprint,
                );
            }
            if rebuild_required {
                state.record_projection_version(SOURCE_CONTEXT_FINGERPRINT_VERSION);
            }
            let applied_source_list_fingerprint = if rebuild_required {
                tracked_source_list_fingerprint(&state)
            } else {
                discovered_source_list_fingerprint.clone()
            };
            state.record(SOURCE_LIST, applied_source_list_fingerprint);
            match save_work_events_intake_state_incremental(state_path, &mut state) {
                Ok(write) => {
                    summary.state_bytes_written = write.bytes_written;
                    summary.state_bytes = write.state_bytes;
                }
                Err(error) => {
                    tracing::warn!(%error, "work events ingest: state save failed");
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "work events ingest: globally ordered intake failed");
        }
    }
    summary
}

/// The legacy log and every canonical event shard in each local worktree.
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

fn worktree_event_sources(
    entries: &[gwt::worktree_inventory::WorktreeEntry],
) -> gwt_core::Result<Vec<WorkEventsSource>> {
    Ok(worktree_event_source_groups(entries)?
        .into_iter()
        .flat_map(|group| group.sources)
        .collect())
}

/// One set of worktree sources that a single directory generation covers.
///
/// Issue #4397: the intake used to derive a fingerprint for every known shard
/// on every trigger. A shard store directory changes its own mtime and entry
/// count whenever a member is published or removed, and the gwt writer never
/// rewrites a published shard — `write_workspace_work_event_shard_bytes` links
/// the shard into place and reports divergent content as an error rather than
/// replacing it. Folding the worktree's git index identity into the generation
/// covers the one writer that can legitimately replace a shard's bytes in
/// place: git itself, on checkout or merge.
#[derive(Debug)]
struct WorkEventsSourceGroup {
    /// Key recorded in the intake state.
    key: String,
    /// Token that changes whenever a member could have changed.
    generation: String,
    /// Intake-state key prefix shared by every member.
    source_prefix: String,
    /// Members are the direct children of `source_prefix` only.
    direct_children_only: bool,
    /// The group is re-derived on every trigger because its member can be
    /// rewritten in place — only the mutable legacy log is.
    always_derive: bool,
    sources: Vec<WorkEventsSource>,
}

/// The mutable legacy log, plus one group per shard-store directory, for each
/// local worktree.
fn worktree_event_source_groups(
    entries: &[gwt::worktree_inventory::WorktreeEntry],
) -> gwt_core::Result<Vec<WorkEventsSourceGroup>> {
    let mut groups = Vec::new();
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
        // The legacy log is the one mutable local source, so it carries no
        // generation: it is stat'ed on every trigger, one file per worktree.
        let legacy_path = entry.path.join(EVENTS_TREE_PATH);
        groups.push(WorkEventsSourceGroup {
            key: format!("{GROUP_WORKTREE}{}", legacy_path.display()),
            generation: "legacy-log-v1".to_string(),
            source_prefix: format!("{SOURCE_WORKTREE}{}", legacy_path.display()),
            direct_children_only: false,
            always_derive: true,
            sources: vec![WorkEventsSource {
                events_path: legacy_path,
                kind: WorkEventsSourceKind::Legacy,
                container: container.clone(),
            }],
        });

        let events_dir = entry.path.join(EVENTS_TREE_DIR);
        if !validate_work_event_store_path(&events_dir)? {
            continue;
        }
        let index_identity = worktree_git_index_identity(&entry.path);
        let mut store_entries = std::fs::read_dir(&events_dir)?.collect::<Result<Vec<_>, _>>()?;
        store_entries.sort_by_key(std::fs::DirEntry::file_name);
        let mut flat = Vec::new();
        let mut buckets = Vec::new();
        for store_entry in store_entries {
            let file_type = store_entry.file_type()?;
            if file_type.is_file() {
                if is_work_event_writer_temp_residue(&store_entry.path()) {
                    continue;
                }
                flat.push(WorkEventsSource {
                    events_path: store_entry.path(),
                    kind: WorkEventsSourceKind::Shard,
                    container: container.clone(),
                });
                continue;
            }
            if !file_type.is_dir() || !is_work_event_bucket_name(&store_entry.file_name()) {
                return Err(gwt_core::GwtError::Other(format!(
                    "work event shard store entry is neither a legacy flat shard nor a digest bucket: {}",
                    store_entry.path().display()
                )));
            }
            buckets.push(store_entry.path());
        }

        groups.push(WorkEventsSourceGroup {
            key: format!("{GROUP_WORKTREE}{}", events_dir.display()),
            generation: shard_directory_generation(
                &events_dir,
                flat.len(),
                &index_identity,
                container.as_ref(),
            )?,
            source_prefix: format!("{SOURCE_WORKTREE}{}/", events_dir.display()),
            direct_children_only: true,
            always_derive: false,
            sources: flat,
        });

        for bucket in buckets {
            let mut bucket_entries = std::fs::read_dir(&bucket)?.collect::<Result<Vec<_>, _>>()?;
            bucket_entries.sort_by_key(std::fs::DirEntry::file_name);
            let mut sources = Vec::new();
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
                sources.push(WorkEventsSource {
                    events_path: shard.path(),
                    kind: WorkEventsSourceKind::Shard,
                    container: container.clone(),
                });
            }
            groups.push(WorkEventsSourceGroup {
                key: format!("{GROUP_WORKTREE}{}", bucket.display()),
                generation: shard_directory_generation(
                    &bucket,
                    sources.len(),
                    &index_identity,
                    container.as_ref(),
                )?,
                source_prefix: format!("{SOURCE_WORKTREE}{}/", bucket.display()),
                direct_children_only: true,
                always_derive: false,
                sources,
            });
        }
    }
    Ok(groups)
}

/// Generation of one shard-store directory: what it holds, when it last
/// changed, and which git state the worktree is on.
fn shard_directory_generation(
    dir: &Path,
    members: usize,
    index_identity: &str,
    container: Option<&WorkspaceExecutionContainerRef>,
) -> gwt_core::Result<String> {
    let metadata = std::fs::symlink_metadata(dir)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    Ok(source_fingerprint(
        &format!("shard-dir-v1:{members}:{modified}:{index_identity}"),
        container,
    ))
}

/// Size and mtime of the worktree's git index, or a marker when it cannot be
/// resolved.
///
/// Any git operation that can rewrite a published shard's bytes — checkout,
/// merge, reset — also rewrites the index, so folding this into the shard-store
/// generations makes those changes visible without stat'ing every shard.
fn worktree_git_index_identity(worktree_root: &Path) -> String {
    let git_path = worktree_root.join(".git");
    let git_dir = match std::fs::symlink_metadata(&git_path) {
        Ok(metadata) if metadata.file_type().is_dir() => git_path,
        Ok(_) => {
            let Ok(body) = std::fs::read_to_string(&git_path) else {
                return "index-unresolved".to_string();
            };
            let Some(dir) = body
                .lines()
                .find_map(|line| line.trim().strip_prefix("gitdir:"))
            else {
                return "index-unresolved".to_string();
            };
            PathBuf::from(dir.trim())
        }
        Err(_) => return "index-unresolved".to_string(),
    };
    let Ok(metadata) = std::fs::symlink_metadata(git_dir.join("index")) else {
        return "index-absent".to_string();
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    format!("index-v1:{}:{modified}", metadata.len())
}

/// Every intake-state source key recorded under one group.
fn group_member_keys(
    state: &gwt_core::work_events_intake::WorkEventsIntakeState,
    prefix: &str,
    direct_children_only: bool,
) -> Vec<String> {
    state
        .sources
        .range(prefix.to_string()..)
        .take_while(|(key, _)| key.starts_with(prefix))
        .filter(|(key, _)| !direct_children_only || !key[prefix.len()..].contains('/'))
        .map(|(key, _)| key.clone())
        .collect()
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

fn source_fingerprint(
    raw_fingerprint: &str,
    container: Option<&WorkspaceExecutionContainerRef>,
) -> String {
    let container_fingerprint = container
        .map(|container| {
            serde_json::to_string(container)
                .unwrap_or_else(|_| "container-serialization-error".into())
        })
        .unwrap_or_else(|| "no-container".to_string());
    content_fingerprint(&format!(
        "{SOURCE_CONTEXT_FINGERPRINT_VERSION}\n{raw_fingerprint}\n{container_fingerprint}"
    ))
}

fn local_immutable_source_fingerprint(
    path: &Path,
    container: Option<&WorkspaceExecutionContainerRef>,
) -> gwt_core::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
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
    Ok(source_fingerprint(
        &format!(
            "immutable-metadata-v1:{}:{}",
            metadata.len(),
            modified.as_nanos()
        ),
        container,
    ))
}

/// Snapshot fingerprint of every shared source the state tracks.
///
/// Issue #4397: this used to clone the full discovered list, sort it and join
/// it into one string — at 429,699 sources that alone allocated ~190 MB per
/// trigger. Folding each entry's digest with XOR is order-independent, so the
/// sort and the intermediate string both disappear.
fn tracked_source_list_fingerprint(
    state: &gwt_core::work_events_intake::WorkEventsIntakeState,
) -> String {
    let mut accumulator = [0u8; 32];
    for (key, fingerprint) in &state.sources {
        if !key.starts_with(SOURCE_WORKTREE) && !key.starts_with(SOURCE_REF) {
            continue;
        }
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        hasher.update([0u8]);
        hasher.update(fingerprint.as_bytes());
        for (slot, byte) in accumulator.iter_mut().zip(hasher.finalize()) {
            *slot ^= byte;
        }
    }
    accumulator
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    use gwt_core::work_events_intake::{save_work_events_intake_state, WorkEventsIntakeState};
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

    /// Publish one canonical shard at its bucketed, content-addressed path.
    fn write_shard(root: &Path, event_id: &str, work_id: &str, updated_at: &str) -> PathBuf {
        let digest = format!("{:x}", sha2::Sha256::digest(event_id.as_bytes()));
        let shard = root
            .join(EVENTS_TREE_DIR)
            .join(&digest[..2])
            .join(format!("{digest}.jsonl"));
        std::fs::create_dir_all(shard.parent().expect("bucket")).expect("event bucket");
        std::fs::write(
            &shard,
            format!(
                "{}\n",
                event_line(event_id, work_id, "Scale work", updated_at)
            ),
        )
        .expect("event shard");
        shard
    }

    /// Shards whose digests land in distinct buckets, so a test can change one
    /// bucket without touching another.
    fn write_shards_across_buckets(root: &Path, count: usize) -> Vec<PathBuf> {
        let mut shards = Vec::new();
        let mut buckets = HashSet::new();
        let mut index = 0;
        while shards.len() < count {
            let event_id = format!("evt-scale-{index}");
            index += 1;
            let digest = format!("{:x}", sha2::Sha256::digest(event_id.as_bytes()));
            if !buckets.insert(digest[..2].to_string()) {
                continue;
            }
            shards.push(write_shard(
                root,
                &event_id,
                &format!("work-scale-{}", shards.len()),
                "2026-09-01T00:00:00Z",
            ));
        }
        shards
    }

    fn projection_work_ids(work_items_path: &Path) -> Vec<String> {
        let mut ids =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(work_items_path)
                .expect("load")
                .expect("projection")
                .work_items
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// Re-ingest the same repo into fresh paths, i.e. what a full re-derivation
    /// of every fingerprint produces.
    fn full_rederivation_work_ids(repo: &Path, temp: &Path, tag: &str) -> Vec<String> {
        let work_items_path = temp.join(format!("state/full-{tag}-works.json"));
        let state_path = temp.join(format!("state/full-{tag}-intake.json"));
        ingest_project_work_events_paths(repo, &work_items_path, &state_path);
        projection_work_ids(&work_items_path)
    }

    /// Issue #4397 AC-1: a trigger that changed nothing must not derive a
    /// single source fingerprint. The intake used to derive one per known
    /// source, which is what made an unchanged pass cost O(all sources).
    #[test]
    fn an_unchanged_trigger_derives_no_source_fingerprint() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let shards = write_shards_across_buckets(&repo, 12);
        run(gwt_core::process::hidden_command("git")
            .args(["add", "."])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "shards"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["update-ref", "refs/remotes/origin/work/scale", "HEAD"])
            .current_dir(&repo));

        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.projection_rebuilt, "{first:?}");
        assert!(
            first.sources_rederived >= shards.len(),
            "the cold pass derives every source: {first:?}"
        );

        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert_eq!(
            second.sources_rederived, 0,
            "an unchanged trigger re-derived fingerprints: {second:?}"
        );
        assert!(
            second.sources_skipped >= shards.len(),
            "every known source is still accounted for: {second:?}"
        );
        assert_eq!(second.events_applied, 0);
        assert!(!second.projection_rebuilt);
    }

    /// Issue #4397 AC-1: publishing one shard costs its own group, not the
    /// whole store.
    #[test]
    fn one_new_shard_only_rederives_its_own_group() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        write_shards_across_buckets(&repo, 16);
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        write_shard(
            &repo,
            "evt-scale-late",
            "work-scale-late",
            "2026-09-02T00:00:00Z",
        );
        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            second.sources_rederived <= 2,
            "one new shard should only re-derive its own bucket: {second:?}"
        );
        assert_eq!(second.events_applied, 1, "{second:?}");
        assert!(projection_work_ids(&work_items_path).contains(&"work-scale-late".to_string()));
    }

    /// Issue #4397 AC-2: one trigger must not re-serialize the whole state.
    #[test]
    fn one_new_shard_writes_a_delta_far_below_the_state_size() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        write_shards_across_buckets(&repo, 32);
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        let first = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert!(first.state_bytes > 0, "{first:?}");

        write_shard(
            &repo,
            "evt-delta-late",
            "work-delta-late",
            "2026-09-02T00:00:00Z",
        );
        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(
            second.state_bytes_written * 4 < second.state_bytes,
            "an incremental trigger rewrote the whole state: {second:?}"
        );
    }

    /// Issue #4397 AC-3: incremental bookkeeping has to land on the same
    /// projection a full re-derivation produces — for an addition, for a
    /// deletion, and for a source whose content changed.
    #[test]
    fn partial_updates_converge_with_a_full_rederivation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        let shards = write_shards_across_buckets(&repo, 8);
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(
            projection_work_ids(&work_items_path),
            full_rederivation_work_ids(&repo, temp.path(), "initial"),
            "cold pass"
        );

        // Addition.
        write_shard(&repo, "evt-added", "work-added", "2026-09-03T00:00:00Z");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(
            projection_work_ids(&work_items_path),
            full_rederivation_work_ids(&repo, temp.path(), "added"),
            "after an addition"
        );

        // Deletion.
        std::fs::remove_file(&shards[0]).expect("remove shard");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(
            projection_work_ids(&work_items_path),
            full_rederivation_work_ids(&repo, temp.path(), "deleted"),
            "after a deletion"
        );

        // Change: the mutable legacy log is the local source that can be
        // rewritten in place.
        write_legacy_log(&repo, &[("evt-legacy-change", "work-legacy-change")]);
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        write_legacy_log(
            &repo,
            &[
                ("evt-legacy-change", "work-legacy-change"),
                ("evt-legacy-change-2", "work-legacy-change-2"),
            ],
        );
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(
            projection_work_ids(&work_items_path),
            full_rederivation_work_ids(&repo, temp.path(), "changed"),
            "after a change"
        );
    }

    /// Issue #4397 AC-5: a state written before group generations existed
    /// keeps every fingerprint it recorded, so the first pass after the upgrade
    /// neither re-reads a source nor rebuilds the projection.
    #[test]
    fn a_v1_intake_state_migrates_without_re_reading_any_source() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        write_shards_across_buckets(&repo, 8);
        write_legacy_log(&repo, &[("evt-v1-legacy", "work-v1-legacy")]);
        run(gwt_core::process::hidden_command("git")
            .args(["add", "."])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "v1 sources"])
            .current_dir(&repo));
        run(gwt_core::process::hidden_command("git")
            .args(["update-ref", "refs/remotes/origin/work/v1", "HEAD"])
            .current_dir(&repo));
        let work_items_path = temp.path().join("state/works.json");
        let state_path = temp.path().join("state/work-events-intake.json");
        ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        // Strip the group layer, leaving exactly what a pre-#4397 gwt wrote.
        let mut v1 = load_work_events_intake_state(&state_path);
        let v1_sources = v1.sources.clone();
        assert!(!v1_sources.is_empty(), "the cold pass recorded sources");
        v1.groups.clear();
        let body = serde_json::json!({
            "sources": v1_sources,
            "projection_version": v1.projection_version,
        });
        std::fs::write(
            &state_path,
            serde_json::to_vec_pretty(&body).expect("v1 body"),
        )
        .expect("write v1 state");
        let _ = std::fs::remove_file(state_path.with_file_name("work-events-intake.delta.jsonl"));

        let reads = std::sync::atomic::AtomicUsize::new(0);
        let migrated = ingest_project_work_events_paths_with_source_read_hook(
            &repo,
            &work_items_path,
            &state_path,
            |_| {
                reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        );

        assert!(
            !migrated.projection_rebuilt,
            "migration must not rebuild: {migrated:?}"
        );
        assert_eq!(
            reads.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "migration must not re-read a source whose v1 fingerprint is current"
        );
        let after = load_work_events_intake_state(&state_path);
        for (key, fingerprint) in &v1_sources {
            assert!(
                after.is_current(key, fingerprint),
                "migration lost the v1 fingerprint for {key}"
            );
        }
        assert!(!after.groups.is_empty(), "migration recorded generations");

        // And the pass after the migration derives nothing but the mutable
        // legacy log, which carries no generation by design.
        let settled = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(settled.sources_rederived, 1, "{settled:?}");
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
        let state_before = std::fs::read(&state_path).expect("state before");

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
        assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
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
        let state_before = std::fs::read(&state_path).expect("state before");

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
        assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
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
        let state_before = std::fs::read(&state_path).expect("state before");

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
        assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
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
        let state_before = std::fs::read(&state_path).expect("state before");

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
        assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
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
        let state_before = std::fs::read(&state_path).expect("state before");

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
        assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
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
        let state_before = std::fs::read(&state_path).expect("state before");

        let summary = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);

        assert!(!summary.projection_rebuilt, "must defer: {summary:?}");
        assert_eq!(std::fs::read(&work_items_path).unwrap(), projection_before);
        assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
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
        let original_state = std::fs::read(&state_path).expect("read intake state");
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
        assert_eq!(
            std::fs::read(&state_path).expect("read preserved intake state"),
            original_state
        );
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
