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

/// Where one ingested chunk of content came from (cache key prefix).
const SOURCE_WORKTREE: &str = "worktree:";
const SOURCE_REF: &str = "ref:";

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
    for events_path in worktree_events_files(project_root) {
        let Ok(content) = std::fs::read_to_string(&events_path) else {
            continue;
        };
        let key = format!("{SOURCE_WORKTREE}{}", events_path.display());
        let fingerprint = content_fingerprint(&content);
        if state.is_current(&key, &fingerprint) {
            summary.sources_skipped += 1;
            continue;
        }
        match ingest_work_events_content(work_items_path, &content) {
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
                        if state.is_current(&key, &oid) {
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
                        match ingest_work_events_content(work_items_path, &content) {
                            Ok(report) => {
                                summary.absorb(&report);
                                state.record(key, oid);
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
fn worktree_events_files(project_root: &Path) -> Vec<PathBuf> {
    match gwt::worktree_inventory::enumerate_worktrees(project_root, None) {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| entry.path.join(EVENTS_TREE_PATH))
            .filter(|path| path.exists())
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "work events ingest: worktree enumeration failed");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

        // Second run: every source fingerprint is current — nothing re-reads.
        let second = ingest_project_work_events_paths(&repo, &work_items_path, &state_path);
        assert_eq!(second.events_applied, 0);
        assert_eq!(second.sources_ingested, 0);
        assert!(second.sources_skipped >= 2, "fingerprint skip: {second:?}");
    }
}
