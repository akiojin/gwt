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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

struct CachedSession {
    mtime: SystemTime,
    size: u64,
    session: Option<crate::Session>,
}

#[derive(Default)]
pub struct SessionLedgerCache {
    entries: HashMap<PathBuf, CachedSession>,
    /// Number of Session parse attempts, including malformed/missing-field
    /// failures. Valid non-Session TOML type checks are not counted.
    pub parse_count: u64,
}

static CACHES: OnceLock<Mutex<HashMap<PathBuf, SessionLedgerCache>>> = OnceLock::new();

impl SessionLedgerCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load every session under `sessions_dir`, reusing parsed entries whose
    /// (mtime, size) are unchanged. Files that fail to stat or parse are
    /// skipped, matching the previous eager loader's semantics.
    pub fn load(&mut self, sessions_dir: &Path) -> Vec<crate::Session> {
        self.try_load(sessions_dir).unwrap_or_default()
    }

    fn try_load(&mut self, sessions_dir: &Path) -> std::io::Result<Vec<crate::Session>> {
        let dir = std::fs::read_dir(sessions_dir).inspect_err(|_| self.entries.clear())?;
        let mut seen = HashSet::new();
        let mut sessions = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            let Some((mtime, size)) = candidate_metadata(&path) else {
                continue;
            };
            if let Some(session) = self.load_entry(&path, mtime, size, false) {
                sessions.push(session);
            }
            seen.insert(path);
        }
        // Whatever was not re-seen this sweep no longer exists on disk.
        self.entries.retain(|path, _| seen.contains(path));
        Ok(sessions)
    }

    fn load_entry(
        &mut self,
        path: &Path,
        mtime: SystemTime,
        size: u64,
        fresh_success: bool,
    ) -> Option<crate::Session> {
        if let Some(hit) = self.entries.get(path) {
            if hit.mtime == mtime && hit.size == size && (!fresh_success || hit.session.is_none()) {
                return hit.session.clone();
            }
        }
        self.commit_entry(
            path,
            mtime,
            size,
            Self::parse_entry(path, fresh_success),
            fresh_success,
        )
    }

    fn cached_entry(
        &self,
        path: &Path,
        mtime: SystemTime,
        size: u64,
    ) -> Option<Option<crate::Session>> {
        self.entries
            .get(path)
            .filter(|hit| hit.mtime == mtime && hit.size == size)
            .map(|hit| hit.session.clone())
    }

    fn is_cached_failure(&self, path: &Path, mtime: SystemTime, size: u64) -> bool {
        self.entries
            .get(path)
            .is_some_and(|hit| hit.mtime == mtime && hit.size == size && hit.session.is_none())
    }

    fn commit_entry(
        &mut self,
        path: &Path,
        mtime: SystemTime,
        size: u64,
        parsed: (
            Result<Option<crate::Session>, String>,
            Option<crate::Session>,
            u64,
        ),
        fresh_success: bool,
    ) -> Option<crate::Session> {
        let (result, candidate, attempts) = parsed;
        self.parse_count += attempts;
        let session = match result {
            Ok(session) => session,
            Err(error) => {
                // Concurrent candidate misses may parse twice, but only the
                // first matching failure commit emits the diagnostic.
                if !self.is_cached_failure(path, mtime, size) {
                    tracing::warn!(path = %path.display(), %error, "Cannot load session; leaving it unchanged");
                }
                None
            }
        };
        let result = if fresh_success {
            candidate
        } else {
            session.clone()
        };
        self.entries.insert(
            path.to_path_buf(),
            CachedSession {
                mtime,
                size,
                session,
            },
        );
        result
    }

    fn parse_entry(
        path: &Path,
        fresh_success: bool,
    ) -> (
        Result<Option<crate::Session>, String>,
        Option<crate::Session>,
        u64,
    ) {
        let Ok(content) = std::fs::read_to_string(path) else {
            return (Ok(None), None, 0);
        };
        let parsed = toml::from_str::<toml::Value>(&content);
        // Legacy display/launch preferences have no Session identity. Keep
        // them in place, but never deserialize or warn about them as Sessions.
        if parsed.as_ref().is_ok_and(|value| value.get("id").is_none()) {
            return (Ok(None), None, 0);
        }
        let mut result = parsed
            .and_then(crate::Session::from_toml_value)
            .map_err(|error| error.to_string())
            .and_then(|session| {
                if path.file_stem().and_then(|stem| stem.to_str()) != Some(session.id.as_str()) {
                    Err("session id does not match filename".to_string())
                } else {
                    Ok(Some(session))
                }
            });
        // Keep startup's verbatim schema for its authoritative update barrier;
        // normalize only the cache copy, before acquiring the shared guard.
        let candidate = if fresh_success {
            result.as_ref().ok().cloned().flatten()
        } else {
            None
        };
        if let Ok(Some(session)) = &mut result {
            session.migrate_legacy_launch_args();
        }
        (result, candidate, 1)
    }
}

fn candidate_metadata(path: &Path) -> Option<(SystemTime, u64)> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("toml")
        || path.file_name()?.to_string_lossy().starts_with('.')
    {
        return None;
    }
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some((metadata.modified().ok()?, metadata.len()))
}

/// Read only a metadata-selected startup candidate. Unchanged non-Sessions
/// and failures reuse the shared cache; successful Sessions are read fresh,
/// without migration, before the caller's authoritative writer barrier.
pub fn load_session_candidate(sessions_dir: &Path, session_id: &str) -> Option<crate::Session> {
    crate::validate_session_id_path_component(session_id).ok()?;
    let path = sessions_dir.join(format!("{session_id}.toml"));
    let (mtime, size) = candidate_metadata(&path)?;
    if with_shared_cache(sessions_dir, |cache| {
        cache.is_cached_failure(&path, mtime, size)
    })
    .0
    {
        return None;
    }
    // Preserve startup's bounded parallel parsing: the fresh read is outside
    // the shared guard; only the negative check and result commit are locked.
    let parsed = SessionLedgerCache::parse_entry(&path, true);
    with_shared_cache(sessions_dir, |cache| {
        cache.commit_entry(&path, mtime, size, parsed, true)
    })
    .0
}

// File reads, parsing, and normalization happen outside the shared guard.
// Guard durations include only cache lookup/commit/pruning.
fn with_shared_cache<T>(
    sessions_dir: &Path,
    load: impl FnOnce(&mut SessionLedgerCache) -> T,
) -> (T, Duration, Duration, u64) {
    let started = Instant::now();
    let mut caches = CACHES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let locked = Instant::now();
    let cache = caches.entry(sessions_dir.to_path_buf()).or_default();
    let previous_parse_count = cache.parse_count;
    let result = load(cache);
    let parse_attempts = cache.parse_count - previous_parse_count;
    (
        result,
        locked.elapsed(),
        locked.duration_since(started),
        parse_attempts,
    )
}

/// Share both successful and failed directory reads across read-only ledger
/// consumers. Exact authority reads still use `Session::load` under its lease.
pub fn load_sessions(sessions_dir: &Path) -> std::io::Result<Vec<crate::Session>> {
    let started = Instant::now();
    let mut seen = HashSet::new();
    let mut hold = Duration::ZERO;
    let mut wait = Duration::ZERO;
    let mut parse_attempts = 0;
    let result = std::fs::read_dir(sessions_dir).map(|dir| {
        let mut sessions = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            let Some((mtime, size)) = candidate_metadata(&path) else {
                continue;
            };
            let (cached, held, waited, _) =
                with_shared_cache(sessions_dir, |cache| cache.cached_entry(&path, mtime, size));
            hold += held;
            wait += waited;
            let session = if let Some(session) = cached {
                session
            } else {
                let parsed = SessionLedgerCache::parse_entry(&path, false);
                let (session, held, waited, attempts) = with_shared_cache(sessions_dir, |cache| {
                    cache.commit_entry(&path, mtime, size, parsed, false)
                });
                hold += held;
                wait += waited;
                parse_attempts += attempts;
                session
            };
            if let Some(session) = session {
                sessions.push(session);
            }
            seen.insert(path);
        }
        sessions
    });
    let (_, held, waited, _) = with_shared_cache(sessions_dir, |cache| {
        cache.entries.retain(|path, _| seen.contains(path));
    });
    hold += held;
    wait += waited;
    tracing::debug!(
        session_sweep_us = started.elapsed().as_micros() as u64,
        session_scan_us = hold.as_micros() as u64,
        cache_lock_wait_us = wait.as_micros() as u64,
        parse_attempts,
        "Session ledger sweep measured"
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_session(dir: &Path, branch: &str) -> crate::Session {
        let session = crate::Session::new(Path::new("/tmp/repo"), branch, crate::AgentId::Codex);
        session.save(dir).expect("save session");
        session
    }

    #[test]
    fn second_load_with_unchanged_files_does_not_reparse() {
        let tmp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
        write_session(tmp.path(), "work/a");

        let mut cache = SessionLedgerCache::new();
        assert_eq!(cache.load(tmp.path()).len(), 1);
        assert!(cache.load(&tmp.path().join("missing")).is_empty());
        // The cache must not resurrect entries from the stale directory.
        assert_eq!(cache.entries.len(), 0);
    }

    #[test]
    fn unreadable_file_does_not_hide_valid_sessions_or_reparse_until_changed() {
        let tmp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
        let valid = write_session(tmp.path(), "work/valid");
        let broken = tmp.path().join("broken.toml");
        std::fs::write(&broken, "id = [").unwrap();
        let legacy = tmp.path().join("missing-agent.toml");
        let mut legacy_value = toml::Value::try_from(&valid).unwrap();
        legacy_value.as_table_mut().unwrap().remove("agent_id");
        legacy_value["id"] = toml::Value::String("missing-agent".into());
        let legacy_content = toml::to_string(&legacy_value).unwrap();
        std::fs::write(&legacy, &legacy_content).unwrap();
        assert!(crate::Session::load(&legacy)
            .unwrap_err()
            .to_string()
            .contains("agent_id"));
        let mut cache = SessionLedgerCache::new();

        assert_eq!(cache.load(tmp.path())[0].id, valid.id);
        assert_eq!(cache.parse_count, 3, "count failed parse attempts too");
        assert_eq!(cache.load(tmp.path()).len(), 1);
        assert_eq!(cache.parse_count, 3, "unchanged failures must be cached");
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), legacy_content);

        for _ in 0..2 {
            assert!(load_session_candidate(tmp.path(), "broken").is_none());
            assert!(load_session_candidate(tmp.path(), "missing-agent").is_none());
        }
        assert_eq!(
            with_shared_cache(tmp.path(), |cache| cache.parse_count).0,
            2,
            "candidate reads must cache syntax and missing-agent failures"
        );

        let mut repaired = valid.clone();
        repaired.id = "broken".to_string();
        repaired.schema_version = 2;
        std::fs::write(&broken, toml::to_string(&repaired).unwrap()).unwrap();
        assert_eq!(cache.load(tmp.path()).len(), 2);
        assert_eq!(cache.parse_count, 4, "changed failures must be retried");
        for expected_attempts in [3, 4] {
            let fresh = load_session_candidate(tmp.path(), "broken").expect("repaired candidate");
            assert_eq!(fresh.schema_version, 2, "candidate returns verbatim schema");
            assert_eq!(
                with_shared_cache(tmp.path(), |cache| cache.parse_count).0,
                expected_attempts,
                "successful candidates must be read fresh even with a warm cache"
            );
        }
    }

    #[test]
    fn window_preferences_and_temporary_files_are_not_session_parse_attempts() {
        let tmp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
        write_session(tmp.path(), "work/valid");
        std::fs::write(
            tmp.path().join("window-state.toml"),
            "display_mode = 'grid'",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("launch-preferences.toml"),
            "last_branch = 'develop'",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".tmp123.toml"), "id = [").unwrap();
        let mut cache = SessionLedgerCache::new();
        assert_eq!(cache.load(tmp.path()).len(), 1);
        assert_eq!(cache.parse_count, 1, "only session files should be parsed");
        assert_eq!(cache.load(tmp.path()).len(), 1);
        assert_eq!(
            cache.parse_count, 1,
            "preferences are cached as non-Sessions"
        );
        for _ in 0..2 {
            for id in ["window-state", "launch-preferences", ".tmp123"] {
                assert!(load_session_candidate(tmp.path(), id).is_none());
            }
        }
        assert_eq!(
            with_shared_cache(tmp.path(), |cache| cache.parse_count).0,
            0,
            "preferences and hidden temporary candidates never reach Session parsing"
        );
    }
}
