//! Session ledger cache — mtime/size-keyed incremental loader for
//! `~/.gwt/sessions/*.toml`.
//!
//! The Workspace projection attaches the machine-local session ledger to
//! every branch row (SPEC-2359 FR-402). Ledgers grow into the thousands of
//! TOML files, and re-parsing all of them on every projection broadcast made
//! window close (and every other projection-bearing action) stall for
//! ~1 second per event ("the × button does not close the window", user
//! report 2026-06-11). This cache re-parses only files whose (mtime, size)
//! changed and drops entries whose files disappeared, turning the steady
//! state into a readdir + stat sweep.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

struct CachedSession {
    mtime: SystemTime,
    size: u64,
    session: gwt_agent::Session,
}

#[derive(Default)]
pub(crate) struct SessionLedgerCache {
    entries: HashMap<PathBuf, CachedSession>,
    /// Number of TOML parses performed over the cache's lifetime. Tests use
    /// this to prove the steady state stops re-parsing unchanged files.
    pub(crate) parse_count: u64,
}

impl SessionLedgerCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Load every session under `sessions_dir`, reusing parsed entries whose
    /// (mtime, size) are unchanged. Files that fail to stat or parse are
    /// skipped, matching the previous eager loader's semantics.
    pub(crate) fn load(&mut self, sessions_dir: &Path) -> Vec<gwt_agent::Session> {
        let Ok(dir) = std::fs::read_dir(sessions_dir) else {
            self.entries.clear();
            return Vec::new();
        };
        let mut seen: HashMap<PathBuf, CachedSession> = HashMap::new();
        let mut sessions = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let Ok(mtime) = meta.modified() else {
                continue;
            };
            let size = meta.len();
            let cached = match self.entries.remove(&path) {
                Some(hit) if hit.mtime == mtime && hit.size == size => hit,
                _ => {
                    let Ok(session) = gwt_agent::Session::load_and_migrate(&path) else {
                        continue;
                    };
                    self.parse_count += 1;
                    CachedSession {
                        mtime,
                        size,
                        session,
                    }
                }
            };
            sessions.push(cached.session.clone());
            seen.insert(path, cached);
        }
        // Whatever was not re-seen this sweep no longer exists on disk.
        self.entries = seen;
        sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_session(dir: &Path, branch: &str) -> gwt_agent::Session {
        let session =
            gwt_agent::Session::new(Path::new("/tmp/repo"), branch, gwt_agent::AgentId::Codex);
        session.save(dir).expect("save session");
        session
    }

    #[test]
    fn second_load_with_unchanged_files_does_not_reparse() {
        let tmp = tempdir().expect("tempdir");
        write_session(tmp.path(), "work/a");
        write_session(tmp.path(), "work/b");

        let mut cache = SessionLedgerCache::new();
        let first = cache.load(tmp.path());
        assert_eq!(first.len(), 2);
        assert_eq!(cache.parse_count, 2);

        let second = cache.load(tmp.path());
        assert_eq!(second.len(), 2);
        assert_eq!(cache.parse_count, 2, "unchanged files must not re-parse");
    }

    #[test]
    fn changed_file_is_reparsed_and_reflects_new_content() {
        let tmp = tempdir().expect("tempdir");
        let mut session = write_session(tmp.path(), "work/a");

        let mut cache = SessionLedgerCache::new();
        assert_eq!(cache.load(tmp.path()).len(), 1);
        assert_eq!(cache.parse_count, 1);

        // Grow the file so (mtime, size) cannot collide even on coarse
        // filesystem timestamps.
        session.model = Some("claude-fable-5-with-a-long-model-name".to_string());
        session.save(tmp.path()).expect("resave session");

        let reloaded = cache.load(tmp.path());
        assert_eq!(reloaded.len(), 1);
        assert_eq!(cache.parse_count, 2, "changed file must re-parse");
        assert_eq!(
            reloaded[0].model.as_deref(),
            Some("claude-fable-5-with-a-long-model-name"),
        );
    }

    #[test]
    fn removed_and_added_files_update_the_result_set() {
        let tmp = tempdir().expect("tempdir");
        let first = write_session(tmp.path(), "work/a");

        let mut cache = SessionLedgerCache::new();
        assert_eq!(cache.load(tmp.path()).len(), 1);

        // Remove the first ledger file and add a different one.
        let first_path = tmp.path().join(format!("{}.toml", first.id));
        let removed = std::fs::read_dir(tmp.path())
            .expect("read dir")
            .flatten()
            .find(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("toml"))
            .map(|entry| entry.path())
            .unwrap_or(first_path);
        std::fs::remove_file(&removed).expect("remove session file");
        let added = write_session(tmp.path(), "work/b");

        let reloaded = cache.load(tmp.path());
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].id, added.id);
        assert_eq!(reloaded[0].branch, "work/b");
    }

    #[test]
    fn missing_directory_yields_empty_and_clears_cache() {
        let tmp = tempdir().expect("tempdir");
        write_session(tmp.path(), "work/a");

        let mut cache = SessionLedgerCache::new();
        assert_eq!(cache.load(tmp.path()).len(), 1);
        assert!(cache.load(&tmp.path().join("missing")).is_empty());
        // The cache must not resurrect entries from the stale directory.
        assert_eq!(cache.entries.len(), 0);
    }
}
