//! SPEC-2963: git-tracked root-thread mapping for remote Board providers.
//!
//! Slack/Teams thread every Board post for a Workspace under a single "root"
//! message (a Workspace summary card). The mapping
//! `(provider, channel, key) -> root message id` must be shared across machines
//! and agents so a Workspace root is created exactly once. It is stored as an
//! append-only JSONL in TWO stores: the repo-local `.gwt/work/` directory
//! (like `events.jsonl`, with a `merge=union` gitattribute so branch-divergent
//! appends reconcile without conflicts — crosses machines via PR merges) and
//! the machine-shared `~/.gwt/projects/<repo-hash>/` home store (FR-022..024,
//! immediate sharing across worktrees on one machine, closing the git
//! propagation lag that minted duplicate General roots). Lookup merges both;
//! the latest line per key wins.
//!
//! `key` is the Workspace id (from a Board entry's `audience`) or the literal
//! `"general"` for broadcast / non-Workspace posts (their own General thread).

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    error::{GwtError, Result},
    paths::{gwt_board_remote_roots_path, gwt_project_dir, gwt_repo_local_work_dir},
    repo_hash::detect_repo_hash,
};

/// Reserved `key` for posts with no Workspace audience (broadcast / system).
pub const GENERAL_THREAD_KEY: &str = "general";

const GITATTRIBUTES_LINE: &str = "**/.gwt/work/board-remote-roots.jsonl merge=union";

/// One root-thread mapping line: a Workspace (or General) thread root for a
/// given remote provider + channel. `card_hash` lets the provider detect when
/// the Workspace summary card changed and the root needs updating.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootMapping {
    /// Workspace id, or [`GENERAL_THREAD_KEY`] for broadcast posts.
    pub key: String,
    /// Provider tag, e.g. `"slack"` / `"teams"`.
    pub provider: String,
    /// Channel id the root lives in (Slack channel / Teams `team/channel`).
    pub channel: String,
    /// Remote root message id (Slack `thread_ts`, Teams message id).
    pub root_id: String,
    /// Hash of the rendered root summary card, for change detection.
    #[serde(default)]
    pub card_hash: String,
    pub updated_at: DateTime<Utc>,
}

impl RootMapping {
    fn normalized(&self) -> Self {
        let mut mapping = self.clone();
        mapping.channel = mapping.channel.trim().to_string();
        mapping
    }

    fn dedup_key(&self) -> (String, String, String) {
        (
            self.provider.clone(),
            self.channel.clone(),
            self.key.clone(),
        )
    }
}

/// The machine-shared home store for the repo at `repo_root`
/// (`~/.gwt/projects/<repo-hash>/board-remote-roots.jsonl`), or `None` when
/// the repo hash (normalized origin URL) cannot be resolved (FR-024). The git
/// propagation of the worktree store crosses machines but only via PR merges;
/// the home store closes the gap for worktrees of the same repo on one
/// machine so a fresh worktree never re-creates an existing thread root
/// (FR-022/FR-023, duplicate-General regression).
fn home_roots_path(repo_root: &Path) -> Option<PathBuf> {
    let repo_hash = detect_repo_hash(repo_root)?;
    Some(gwt_project_dir(&repo_hash).join("board-remote-roots.jsonl"))
}

/// Append a root mapping line to the repo-local JSONL store (and best-effort
/// to the machine-shared home store) and ensure the union-merge gitattribute
/// exists. Append-only: a later line for the same `(provider, channel, key)`
/// supersedes earlier ones on load.
pub fn append_root_mapping(repo_root: &Path, mapping: &RootMapping) -> Result<()> {
    let mapping = mapping.normalized();
    let path = gwt_board_remote_roots_path(repo_root);
    ensure_gitattributes(repo_root);
    append_mapping_line(&path, &mapping)?;
    // FR-023: the home store write is best-effort — an unresolvable repo hash
    // or a home I/O failure must never fail the post itself.
    if let Some(home_path) = home_roots_path(repo_root) {
        let _ = append_mapping_line(&home_path, &mapping);
    }
    Ok(())
}

fn append_mapping_line(path: &Path, mapping: &RootMapping) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    // Serialize to a single buffer (line + trailing newline) and emit it with
    // ONE `write_all`. On POSIX an O_APPEND write of <= PIPE_BUF bytes is
    // atomic across processes, so concurrent appends from multiple agents on
    // the same repo never interleave their bytes. `serde_json::to_writer` issues
    // many small writes per value, which DID interleave and corrupt lines under
    // multi-agent posting (SPEC-2963); load then silently skipped the corrupt
    // lines and re-created duplicate thread roots.
    let mut line = serde_json::to_string(mapping)
        .map_err(|error| GwtError::Other(format!("board-remote-roots json: {error}")))?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Load all root mappings from the worktree store merged with the home store,
/// keeping the most recent (by `updated_at`) line per
/// `(provider, channel, key)`. Union-merge friendly: appends from divergent
/// branches (and from other worktrees via the home store) concatenate and the
/// latest timestamp wins, preventing duplicate roots.
pub fn load_root_mappings(repo_root: &Path) -> BTreeMap<(String, String, String), RootMapping> {
    let mut latest = BTreeMap::new();
    merge_root_mappings_from_path(&gwt_board_remote_roots_path(repo_root), &mut latest);
    if let Some(home_path) = home_roots_path(repo_root) {
        merge_root_mappings_from_path(&home_path, &mut latest);
    }
    latest
}

fn merge_root_mappings_from_path(
    path: &Path,
    latest: &mut BTreeMap<(String, String, String), RootMapping>,
) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(mapping) = serde_json::from_str::<RootMapping>(line) else {
            continue;
        };
        let mapping = mapping.normalized();
        let key = mapping.dedup_key();
        let supersedes = latest
            .get(&key)
            .map(|existing| mapping.updated_at >= existing.updated_at)
            .unwrap_or(true);
        if supersedes {
            latest.insert(key, mapping);
        }
    }
}

/// Look up the current root mapping for a `(provider, channel, key)`, if any.
pub fn find_root_mapping(
    repo_root: &Path,
    provider: &str,
    channel: &str,
    key: &str,
) -> Option<RootMapping> {
    load_root_mappings(repo_root).remove(&(
        provider.to_string(),
        channel.trim().to_string(),
        key.to_string(),
    ))
}

/// Resolve the worktree root from the repo-local work dir (`.gwt/work` ->
/// `.gwt` -> worktree root) so the gitattribute lands in the checked-out tree.
fn worktree_root_from(repo_root: &Path) -> PathBuf {
    gwt_repo_local_work_dir(repo_root)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_root.to_path_buf())
}

/// Best-effort: ensure `.gitattributes` carries the union-merge line for the
/// mapping file (idempotent). Failures are swallowed so posting never fails on
/// a read-only / non-repository root (mirrors the events.jsonl handling).
fn ensure_gitattributes(repo_root: &Path) {
    let root = worktree_root_from(repo_root);
    let attributes_path = root.join(".gitattributes");
    let existing = fs::read_to_string(&attributes_path).unwrap_or_default();
    if existing
        .lines()
        .any(|line| line.trim() == GITATTRIBUTES_LINE)
    {
        return;
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(GITATTRIBUTES_LINE);
    next.push('\n');
    let _ = fs::write(&attributes_path, next);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn mapping(key: &str, root_id: &str, at: i64, hash: &str) -> RootMapping {
        RootMapping {
            key: key.to_string(),
            provider: "slack".to_string(),
            channel: "CH".to_string(),
            root_id: root_id.to_string(),
            card_hash: hash.to_string(),
            updated_at: Utc.timestamp_opt(at, 0).unwrap(),
        }
    }

    #[test]
    fn append_then_find_returns_latest_per_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        append_root_mapping(root, &mapping("ws-a", "ts-1", 100, "h1")).unwrap();
        append_root_mapping(root, &mapping("ws-b", "ts-2", 100, "h1")).unwrap();
        // A later append for ws-a (e.g. card update) supersedes the earlier one.
        append_root_mapping(root, &mapping("ws-a", "ts-1", 200, "h2")).unwrap();

        let found = find_root_mapping(root, "slack", "CH", "ws-a").unwrap();
        assert_eq!(found.root_id, "ts-1");
        assert_eq!(found.card_hash, "h2", "latest timestamp wins");

        let other = find_root_mapping(root, "slack", "CH", "ws-b").unwrap();
        assert_eq!(other.root_id, "ts-2");

        assert!(find_root_mapping(root, "slack", "CH", "missing").is_none());
        // A different provider/channel does not collide.
        assert!(find_root_mapping(root, "teams", "CH", "ws-a").is_none());
    }

    #[test]
    fn load_is_union_merge_friendly_latest_timestamp_wins() {
        // Simulate two branches each appending their own line for the same key
        // (concatenated by git union-merge). The newest updated_at wins, so no
        // duplicate root is used.
        let dir = tempfile::tempdir().unwrap();
        let path = gwt_board_remote_roots_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let older = serde_json::to_string(&mapping("ws-a", "ts-old", 100, "h1")).unwrap();
        let newer = serde_json::to_string(&mapping("ws-a", "ts-new", 300, "h3")).unwrap();
        // Newer line appears BEFORE the older one to prove ordering is by
        // timestamp, not file position.
        fs::write(&path, format!("{newer}\n{older}\n")).unwrap();

        let found = find_root_mapping(dir.path(), "slack", "CH", "ws-a").unwrap();
        assert_eq!(found.root_id, "ts-new");
        assert_eq!(found.card_hash, "h3");
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_root_mappings(dir.path()).is_empty());
        assert!(find_root_mapping(dir.path(), "slack", "CH", "ws-a").is_none());
    }

    #[test]
    fn concurrent_appends_do_not_interleave() {
        // SPEC-2963: multiple agents on the same repo append concurrently. Each
        // line must stay intact (no byte interleaving) so load never has to skip
        // a corrupt line and re-create a duplicate thread root.
        use std::sync::{Arc, Barrier};
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(dir.path().to_path_buf());
        let threads = 8usize;
        let per_thread = 40usize;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::new();
        for t in 0..threads {
            let root = Arc::clone(&root);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..per_thread {
                    let m = mapping(&format!("ws-{t}-{i}"), &format!("ts-{t}-{i}"), 100, "h");
                    append_root_mapping(&root, &m).unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        // Every non-empty line must parse: a corrupt (interleaved) line proves a
        // non-atomic write.
        let path = gwt_board_remote_roots_path(&root);
        let content = fs::read_to_string(&path).unwrap();
        let mut count = 0usize;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            serde_json::from_str::<RootMapping>(line)
                .unwrap_or_else(|error| panic!("interleaved/corrupt line: {error}\n{line}"));
            count += 1;
        }
        assert_eq!(
            count,
            threads * per_thread,
            "every append present, none lost or merged"
        );
        assert_eq!(load_root_mappings(&root).len(), threads * per_thread);
    }

    /// `git init` + origin remote so `detect_repo_hash` resolves; no commits
    /// are needed for the home-store path derivation.
    fn init_repo_with_origin(path: &Path, url: &str) {
        fs::create_dir_all(path).unwrap();
        for args in [vec!["init"], vec!["remote", "add", "origin", url]] {
            let output = crate::process::hidden_command("git")
                .args(&args)
                .current_dir(path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    /// Redirect `$HOME` to an isolated temp dir (FR-022..024 tests must never
    /// touch the real `~/.gwt`; #3022 isolation-leak prevention).
    fn scoped_home(
        home: &Path,
    ) -> (
        std::sync::MutexGuard<'static, ()>,
        crate::test_support::ScopedEnvVar,
    ) {
        fs::create_dir_all(home).unwrap();
        let guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let scoped = crate::test_support::ScopedEnvVar::set("HOME", home);
        (guard, scoped)
    }

    const ORIGIN: &str = "git@github.com:example/board-roots.git";

    #[test]
    fn append_writes_home_store_for_repo_with_origin() {
        // FR-023: append lands in the worktree store AND the machine-shared
        // home store (`~/.gwt/projects/<repo-hash>/board-remote-roots.jsonl`).
        let dir = tempfile::tempdir().unwrap();
        let (_lock, _home) = scoped_home(&dir.path().join("home"));
        let repo = dir.path().join("wt-a");
        init_repo_with_origin(&repo, ORIGIN);

        append_root_mapping(&repo, &mapping("ws-a", "ts-1", 100, "h1")).unwrap();

        let repo_hash = crate::repo_hash::detect_repo_hash(&repo).expect("origin resolves");
        let home_path = crate::paths::gwt_project_dir(&repo_hash).join("board-remote-roots.jsonl");
        assert!(
            home_path.starts_with(dir.path().join("home")),
            "home store must live under the redirected $HOME: {home_path:?}"
        );
        let content = fs::read_to_string(&home_path).expect("home store written");
        let line: RootMapping = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(line.key, "ws-a");
        assert_eq!(line.root_id, "ts-1");
        // The worktree store keeps its existing behavior.
        let worktree_content =
            fs::read_to_string(gwt_board_remote_roots_path(&repo)).expect("worktree store");
        assert!(worktree_content.contains("ts-1"));
    }

    #[test]
    fn home_store_shares_roots_across_worktrees_of_same_repo() {
        // FR-022/FR-024 (SC-018): worktree B has no local mapping (fresh
        // worktree before any git propagation) but must find the root that
        // worktree A created, via the repo-hash-scoped home store. This is the
        // duplicate-General-root regression.
        let dir = tempfile::tempdir().unwrap();
        let (_lock, _home) = scoped_home(&dir.path().join("home"));
        let wt_a = dir.path().join("wt-a");
        let wt_b = dir.path().join("wt-b");
        init_repo_with_origin(&wt_a, ORIGIN);
        init_repo_with_origin(&wt_b, ORIGIN);

        append_root_mapping(&wt_a, &mapping("general", "ts-root", 100, "h1")).unwrap();

        let found = find_root_mapping(&wt_b, "slack", "CH", "general")
            .expect("worktree B sees the root via the home store");
        assert_eq!(found.root_id, "ts-root");
    }

    #[test]
    fn latest_updated_at_wins_across_worktree_and_home_stores() {
        // FR-022: lookup merges both stores and the newest line per
        // (provider, channel, key) wins, regardless of which store holds it.
        let dir = tempfile::tempdir().unwrap();
        let (_lock, _home) = scoped_home(&dir.path().join("home"));
        let wt_a = dir.path().join("wt-a");
        let wt_b = dir.path().join("wt-b");
        init_repo_with_origin(&wt_a, ORIGIN);
        init_repo_with_origin(&wt_b, ORIGIN);

        append_root_mapping(&wt_a, &mapping("ws-a", "ts-old", 100, "h1")).unwrap();
        append_root_mapping(&wt_b, &mapping("ws-a", "ts-new", 300, "h3")).unwrap();

        // A's own worktree store still says ts-old, but the newer home line
        // from B supersedes it.
        let via_a = find_root_mapping(&wt_a, "slack", "CH", "ws-a").unwrap();
        assert_eq!(via_a.root_id, "ts-new");
        assert_eq!(via_a.card_hash, "h3");
    }

    #[test]
    fn append_and_find_degrade_to_worktree_store_without_origin() {
        // FR-023: when the repo hash cannot be resolved (no origin remote),
        // the home store is skipped — posting still works and nothing is
        // written under $HOME.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (_lock, _home) = scoped_home(&home);
        let root = dir.path().join("plain");
        fs::create_dir_all(&root).unwrap();

        append_root_mapping(&root, &mapping("ws-a", "ts-1", 100, "h1")).unwrap();
        let found = find_root_mapping(&root, "slack", "CH", "ws-a").unwrap();
        assert_eq!(found.root_id, "ts-1");
        assert!(
            !home.join(".gwt").exists(),
            "no origin -> no home store write"
        );
    }

    #[test]
    fn append_writes_union_merge_gitattributes() {
        let dir = tempfile::tempdir().unwrap();
        append_root_mapping(dir.path(), &mapping("ws-a", "ts-1", 100, "h1")).unwrap();
        let attrs = fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(attrs.contains(GITATTRIBUTES_LINE));
        // Idempotent: a second append does not duplicate the line.
        append_root_mapping(dir.path(), &mapping("ws-b", "ts-2", 100, "h1")).unwrap();
        let attrs2 = fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(attrs2.matches(GITATTRIBUTES_LINE).count(), 1);
    }
}
