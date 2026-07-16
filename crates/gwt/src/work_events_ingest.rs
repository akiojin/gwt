//! SPEC-2359 W-16 (FR-387/FR-388): project-level work events ingest
//! orchestrator.
//!
//! Collects `.gwt/work/events.jsonl` content from every reachable source —
//! local worktree filesystems (the base/main checkout included) and fetched
//! `origin/*` refs (checkout-free blob reads) — and funnels each through the
//! idempotent gwt-core intake into the home works projection. A fingerprint
//! cache (`work-events-intake.json`) skips unchanged sources; correctness
//! never depends on it (dedup is event-id based, SC-260).
//!
//! Spawn budget per run: 1 `git worktree list` (inventory) + 1
//! `for-each-ref` + 1 `cat-file --batch-check` + one `cat-file blob` per
//! not-yet-ingested events blob. Callers run this off the UI thread.

use std::path::{Path, PathBuf};

#[cfg(test)]
use gwt_core::work_events_intake::ingest_work_events_content;
use gwt_core::work_events_intake::{
    content_fingerprint, ingest_work_events_contents, ingest_work_events_with_local_path,
    load_work_events_intake_state, rebuild_work_events_with_shared_loader,
    save_work_events_intake_state,
};
use gwt_core::workspace_projection::WorkspaceExecutionContainerRef;

/// Where one ingested chunk of content came from (cache key prefix).
const SOURCE_WORKTREE: &str = "worktree:";
const SOURCE_REF: &str = "ref:";
const SOURCE_LOCAL_LIFECYCLE: &str = "local-lifecycle:";

/// Bump this when projection-time source metadata changes. Older cache entries
/// used only the raw content/blob fingerprint, which would skip the repair pass.
const SOURCE_CONTEXT_FINGERPRINT_VERSION: &str = "source-context-v6-complete-project-transaction";

/// Tree path of the persistent core inside a worktree / commit.
const EVENTS_TREE_PATH: &str = ".gwt/work/events.jsonl";

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
    content: String,
    reload: Option<ReloadableWorkEventsSource>,
}

#[derive(Debug)]
struct ReloadableWorkEventsSource {
    events_path: PathBuf,
    container: Option<WorkspaceExecutionContainerRef>,
}

type SourceFingerprints = Vec<(String, String)>;
type ReloadedWorkEventsSources = (Vec<String>, SourceFingerprints);

fn load_pending_sources_for_rebuild(
    pending_sources: &[PendingWorkEventsSource],
) -> gwt_core::Result<ReloadedWorkEventsSources> {
    let mut contents = Vec::with_capacity(pending_sources.len());
    let mut fingerprints = Vec::with_capacity(pending_sources.len());
    for source in pending_sources {
        let Some(reload) = &source.reload else {
            contents.push(source.content.clone());
            fingerprints.push((source.key.clone(), source.fingerprint.clone()));
            continue;
        };
        let content = std::fs::read_to_string(&reload.events_path)?;
        fingerprints.push((
            source.key.clone(),
            source_fingerprint(&content_fingerprint(&content), reload.container.as_ref()),
        ));
        contents.push(content_with_source_container(
            &content,
            reload.container.as_ref(),
        ));
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
    ingest_project_work_events_paths_inner(project_root, work_items_path, state_path, || {})
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
    ingest_project_work_events_paths_inner(project_root, work_items_path, state_path, before_intake)
}

fn ingest_project_work_events_paths_inner<F>(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
    before_intake: F,
) -> WorkEventsIngestSummary
where
    F: FnOnce(),
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
    let rebuild_required = projection_requires_rebuild
        || !state.projection_is_current(SOURCE_CONTEXT_FINGERPRINT_VERSION);
    let mut pending_sources = Vec::new();
    let mut source_discovery_failed = false;

    // 1) Local worktree filesystems (base/main checkout included): committed
    //    or not, the working copy is the freshest view of each branch's log.
    let worktree_sources = match worktree_event_sources(project_root) {
        Ok(sources) => sources,
        Err(error) => {
            tracing::warn!(%error, "work events ingest: worktree enumeration failed");
            source_discovery_failed = true;
            Vec::new()
        }
    };
    for source in worktree_sources {
        let events_path = source.events_path;
        match events_path.try_exists() {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                tracing::warn!(%error, path = %events_path.display(), "work events ingest: worktree source discovery failed");
                source_discovery_failed = true;
                continue;
            }
        }
        let content = match std::fs::read_to_string(&events_path) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(%error, path = %events_path.display(), "work events ingest: worktree source read failed");
                source_discovery_failed = true;
                continue;
            }
        };
        let key = format!("{SOURCE_WORKTREE}{}", events_path.display());
        let source_container = source.container.as_ref();
        let fingerprint = source_fingerprint(&content_fingerprint(&content), source_container);
        if !rebuild_required && state.is_current(&key, &fingerprint) {
            summary.sources_skipped += 1;
            continue;
        }
        let ingest_content = content_with_source_container(&content, source_container);
        pending_sources.push(PendingWorkEventsSource {
            key,
            fingerprint,
            content: ingest_content,
            reload: Some(ReloadableWorkEventsSource {
                events_path,
                container: source.container,
            }),
        });
    }

    // 2) Fetched origin/* refs — checkout-free blob reads. Close-kind
    //    filtering inside the intake keeps foreign close state out (FR-384)
    //    and lenient parsing guards against contaminated logs (#3023).
    match gwt_git::refs::list_origin_refs_with_commit(project_root) {
        Ok(refs) if !refs.is_empty() => {
            let commits: Vec<String> = refs.iter().map(|(_, sha)| sha.clone()).collect();
            match gwt_git::blob::events_blob_oids_batch(project_root, &commits, EVENTS_TREE_PATH) {
                Ok(oids) => {
                    for ((refname, _), oid) in refs.iter().zip(oids) {
                        let Some(oid) = oid else { continue };
                        let key = format!("{SOURCE_REF}{refname}");
                        let source_container = origin_ref_execution_container(refname);
                        let fingerprint = source_fingerprint(&oid, source_container.as_ref());
                        if !rebuild_required && state.is_current(&key, &fingerprint) {
                            summary.sources_skipped += 1;
                            continue;
                        }
                        let content = match gwt_git::blob::read_blob(project_root, &oid) {
                            Ok(content) => content,
                            Err(error) => {
                                tracing::warn!(%error, refname, "work events ingest: blob read failed");
                                source_discovery_failed = true;
                                continue;
                            }
                        };
                        let ingest_content =
                            content_with_source_container(&content, source_container.as_ref());
                        pending_sources.push(PendingWorkEventsSource {
                            key,
                            fingerprint,
                            content: ingest_content,
                            reload: None,
                        });
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "work events ingest: batch-check failed");
                    source_discovery_failed = true;
                }
            }
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "work events ingest: origin ref listing failed");
            source_discovery_failed = true;
        }
    }

    let close_path = work_items_path
        .parent()
        .map(|parent| parent.join("work-events-closed.jsonl"));
    let pending_local_lifecycle = match close_path.as_ref().map(std::fs::read_to_string) {
        Some(Ok(content)) if !content.is_empty() => {
            let key = format!(
                "{SOURCE_LOCAL_LIFECYCLE}{}",
                close_path.as_ref().unwrap().display()
            );
            let fingerprint = content_fingerprint(&content);
            if !rebuild_required && state.is_current(&key, &fingerprint) {
                summary.sources_skipped += 1;
                None
            } else {
                Some((key, fingerprint))
            }
        }
        Some(Ok(_)) | None => None,
        Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
        Some(Err(error)) => {
            tracing::warn!(%error, "work events ingest: local lifecycle log read failed");
            return summary;
        }
    };

    if rebuild_required && source_discovery_failed {
        tracing::warn!(
            "work events ingest: projection rebuild deferred because source discovery was incomplete"
        );
        return summary;
    }

    if pending_sources.is_empty() && pending_local_lifecycle.is_none() {
        if rebuild_required {
            tracing::warn!(
                "work events ingest: projection rebuild deferred because no shared or local lifecycle source was readable"
            );
        }
        return summary;
    }

    before_intake();
    let contents = pending_sources.iter().map(|source| source.content.as_str());
    let intake = if rebuild_required {
        rebuild_work_events_with_shared_loader(
            work_items_path,
            || load_pending_sources_for_rebuild(&pending_sources),
            close_path.as_deref(),
        )
    } else if pending_local_lifecycle.is_some() {
        ingest_work_events_with_local_path(work_items_path, contents, close_path.as_deref()).map(
            |(report, local_fingerprint)| {
                (
                    report,
                    pending_sources
                        .iter()
                        .map(|source| (source.key.clone(), source.fingerprint.clone()))
                        .collect(),
                    local_fingerprint,
                )
            },
        )
    } else {
        ingest_work_events_contents(work_items_path, contents).map(|report| {
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
                // permanently skip its events.
                state.sources.clear();
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
            if let Err(error) = save_work_events_intake_state(state_path, &state) {
                tracing::warn!(%error, "work events ingest: state save failed");
            }
        }
        Err(error) => {
            tracing::warn!(%error, "work events ingest: globally ordered intake failed");
        }
    }
    summary
}

/// The events.jsonl file of every local worktree (main checkout included).
fn worktree_event_sources(
    project_root: &Path,
) -> Result<Vec<WorkEventsSource>, gwt::worktree_inventory::InventoryError> {
    gwt::worktree_inventory::enumerate_worktrees(project_root, None).map(|entries| {
        entries
            .into_iter()
            .map(|entry| WorkEventsSource {
                events_path: entry.path.join(EVENTS_TREE_PATH),
                container: entry.branch.map(|branch| WorkspaceExecutionContainerRef {
                    branch: Some(branch),
                    worktree_path: Some(entry.path),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                }),
            })
            .collect()
    })
}

#[derive(Debug, Clone)]
struct WorkEventsSource {
    events_path: PathBuf,
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

fn content_with_source_container(
    content: &str,
    container: Option<&WorkspaceExecutionContainerRef>,
) -> String {
    let Some(container) = container else {
        return content.to_string();
    };
    let Ok(container_value) = serde_json::to_value(container) else {
        return content.to_string();
    };

    let mut output = String::new();
    let mut changed = false;
    for line in content.lines() {
        let replacement = match serde_json::from_str::<serde_json::Value>(line.trim()) {
            Ok(mut value) => {
                if let Some(object) = value.as_object_mut() {
                    let missing_container = object
                        .get("execution_container")
                        .map(|value| value.is_null())
                        .unwrap_or(true);
                    if missing_container {
                        object.insert("execution_container".to_string(), container_value.clone());
                        changed = true;
                    }
                }
                serde_json::to_string(&value).unwrap_or_else(|_| line.to_string())
            }
            Err(_) => line.to_string(),
        };
        output.push_str(&replacement);
        output.push('\n');
    }
    if !content.ends_with('\n') {
        output.pop();
    }

    if changed {
        output
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::work_events_intake::WorkEventsIntakeState;
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
        run(Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(path));
        run(Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path));
        run(Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path));
        run(Command::new("git")
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
        run(Command::new("git")
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
        run(Command::new("git")
            .args(["add", ".gwt/work/events.jsonl"])
            .current_dir(&repo));
        run(Command::new("git")
            .args(["commit", "-m", "remote events"])
            .current_dir(&repo));
        run(Command::new("git")
            .args(["update-ref", "refs/remotes/origin/work/remote-side", "HEAD"])
            .current_dir(&repo));
        run(Command::new("git")
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
    fn projection_parse_failure_requires_rebuild_with_current_version_and_fingerprints() {
        let temp = tempfile::tempdir().expect("tempdir");
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
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        run(Command::new("git")
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
        run(Command::new("git")
            .args(["add", ".gwt/work/events.jsonl"])
            .current_dir(&repo));
        run(Command::new("git")
            .args(["commit", "-m", "cache repair events"])
            .current_dir(&repo));
        run(Command::new("git")
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
        let repo = temp.path().join("repo");
        let owner_worktree = temp.path().join("owner-worktree");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

        run(Command::new("git")
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
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);

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
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(Command::new("git")
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
    fn version_rebuild_replaces_stale_source_state_so_restored_source_is_ingested() {
        let temp = tempfile::tempdir().expect("tempdir");
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

        run(Command::new("git")
            .args(["worktree", "add", "-b", "work/restored-source"])
            .arg(&restored_worktree)
            .current_dir(&repo));
        std::fs::create_dir_all(restored_events.parent().unwrap()).unwrap();
        std::fs::write(&restored_events, restored_content).unwrap();

        let restored = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
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
        let repo = temp.path().join("repo");
        let side_worktree = temp.path().join("side-worktree");
        std::fs::create_dir_all(&repo).expect("repo dir");
        init_repo(&repo);
        run(Command::new("git")
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
