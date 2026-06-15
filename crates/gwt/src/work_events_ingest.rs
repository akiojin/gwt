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

use gwt_core::work_events_intake::{
    content_fingerprint, ingest_work_events_content, load_work_events_intake_state,
    save_work_events_intake_state, WorkEventsIntakeReport,
};
use gwt_core::workspace_projection::WorkspaceExecutionContainerRef;

/// Where one ingested chunk of content came from (cache key prefix).
const SOURCE_WORKTREE: &str = "worktree:";
const SOURCE_REF: &str = "ref:";

/// Bump this when projection-time source metadata changes. Older cache entries
/// used only the raw content/blob fingerprint, which would skip the repair pass.
const SOURCE_CONTEXT_FINGERPRINT_VERSION: &str = "source-context-v1";

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
}

impl WorkEventsIngestSummary {
    pub fn changed(&self) -> bool {
        self.events_applied > 0
    }

    fn absorb(&mut self, report: &WorkEventsIntakeReport) {
        self.sources_ingested += 1;
        self.events_applied += report.applied;
    }
}

/// Paths-injected ingest (#3022): all writes go to `work_items_path` /
/// `state_path`. Source-level failures are logged and skipped — a broken
/// worktree or unreadable ref never aborts the sweep.
pub fn ingest_project_work_events_paths(
    project_root: &Path,
    work_items_path: &Path,
    state_path: &Path,
) -> WorkEventsIngestSummary {
    let mut summary = WorkEventsIngestSummary::default();
    let mut state = load_work_events_intake_state(state_path);
    let mut state_changed = false;

    // 1) Local worktree filesystems (base/main checkout included): committed
    //    or not, the working copy is the freshest view of each branch's log.
    for source in worktree_event_sources(project_root) {
        let events_path = source.events_path;
        let Ok(content) = std::fs::read_to_string(&events_path) else {
            continue;
        };
        let key = format!("{SOURCE_WORKTREE}{}", events_path.display());
        let source_container = source.container.as_ref();
        let fingerprint = source_fingerprint(&content_fingerprint(&content), source_container);
        if state.is_current(&key, &fingerprint) {
            summary.sources_skipped += 1;
            continue;
        }
        let ingest_content = content_with_source_container(&content, source_container);
        match ingest_work_events_content(work_items_path, &ingest_content) {
            Ok(report) => {
                summary.absorb(&report);
                state.record(key, fingerprint);
                state_changed = true;
            }
            Err(error) => {
                tracing::warn!(%error, path = %events_path.display(), "work events ingest: worktree source failed");
            }
        }
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
                        if state.is_current(&key, &fingerprint) {
                            summary.sources_skipped += 1;
                            continue;
                        }
                        let content = match gwt_git::blob::read_blob(project_root, &oid) {
                            Ok(content) => content,
                            Err(error) => {
                                tracing::warn!(%error, refname, "work events ingest: blob read failed");
                                continue;
                            }
                        };
                        let ingest_content =
                            content_with_source_container(&content, source_container.as_ref());
                        match ingest_work_events_content(work_items_path, &ingest_content) {
                            Ok(report) => {
                                summary.absorb(&report);
                                state.record(key, fingerprint);
                                state_changed = true;
                            }
                            Err(error) => {
                                tracing::warn!(%error, refname, "work events ingest: ref source failed");
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "work events ingest: batch-check failed");
                }
            }
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "work events ingest: origin ref listing failed");
        }
    }

    if state_changed {
        if let Err(error) = save_work_events_intake_state(state_path, &state) {
            tracing::warn!(%error, "work events ingest: state save failed");
        }
    }
    summary
}

/// The events.jsonl file of every local worktree (main checkout included).
fn worktree_event_sources(project_root: &Path) -> Vec<WorkEventsSource> {
    match gwt::worktree_inventory::enumerate_worktrees(project_root, None) {
        Ok(entries) => entries
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
            .filter(|source| source.events_path.exists())
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "work events ingest: worktree enumeration failed");
            Vec::new()
        }
    }
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
    let Some(container) = container else {
        return raw_fingerprint.to_string();
    };
    let container_fingerprint =
        serde_json::to_string(container).unwrap_or_else(|_| "container-serialization-error".into());
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
}
