//! SPEC-2963: git-tracked root-thread mapping for remote Board providers.
//!
//! Slack/Teams thread every Board post for a Workspace under a single "root"
//! message (a Workspace summary card). The mapping
//! `(provider, channel, key) -> root message id` must be shared across machines
//! and agents so a Workspace root is created exactly once. It is stored as an
//! append-only JSONL under the repo-local `.gwt/work/` directory (like
//! `events.jsonl`) with a `merge=union` gitattribute, so branch-divergent
//! appends reconcile without conflicts. The latest line per key wins.
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
    paths::{gwt_board_remote_roots_path, gwt_repo_local_work_dir},
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
    fn dedup_key(&self) -> (String, String, String) {
        (
            self.provider.clone(),
            self.channel.clone(),
            self.key.clone(),
        )
    }
}

/// Append a root mapping line to the repo-local JSONL store and ensure the
/// union-merge gitattribute exists. Append-only: a later line for the same
/// `(provider, channel, key)` supersedes earlier ones on load.
pub fn append_root_mapping(repo_root: &Path, mapping: &RootMapping) -> Result<()> {
    let path = gwt_board_remote_roots_path(repo_root);
    ensure_gitattributes(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    serde_json::to_writer(&mut file, mapping)
        .map_err(|error| GwtError::Other(format!("board-remote-roots json: {error}")))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Load all root mappings, keeping the most recent (by `updated_at`) line per
/// `(provider, channel, key)`. Union-merge friendly: appends from divergent
/// branches concatenate and the latest timestamp wins, preventing duplicate
/// roots.
pub fn load_root_mappings(repo_root: &Path) -> BTreeMap<(String, String, String), RootMapping> {
    let path = gwt_board_remote_roots_path(repo_root);
    load_root_mappings_from_path(&path)
}

fn load_root_mappings_from_path(path: &Path) -> BTreeMap<(String, String, String), RootMapping> {
    let Ok(content) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let mut latest: BTreeMap<(String, String, String), RootMapping> = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(mapping) = serde_json::from_str::<RootMapping>(line) else {
            continue;
        };
        let key = mapping.dedup_key();
        let supersedes = latest
            .get(&key)
            .map(|existing| mapping.updated_at >= existing.updated_at)
            .unwrap_or(true);
        if supersedes {
            latest.insert(key, mapping);
        }
    }
    latest
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
        channel.to_string(),
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
