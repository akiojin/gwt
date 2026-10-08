//! The UI's ledger views share the same incremental loader as launch and recovery.

use std::path::Path;
#[cfg(not(test))]
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct SessionLedgerCache;

impl SessionLedgerCache {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn load(&mut self, sessions_dir: &Path) -> Vec<gwt_agent::Session> {
        #[cfg(not(test))]
        schedule_retention(sessions_dir);
        gwt_agent::session_ledger::load_sessions(sessions_dir).unwrap_or_default()
    }
}

#[cfg(not(test))]
fn schedule_retention(sessions_dir: &Path) {
    static LAST_SWEEPS: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
    let mut sweeps = LAST_SWEEPS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    if sweeps
        .get(sessions_dir)
        .is_some_and(|last| now.duration_since(*last) < Duration::from_secs(24 * 60 * 60))
    {
        return;
    }
    let directory = sessions_dir.to_path_buf();
    match std::thread::Builder::new()
        .name("gwt-session-retention".into())
        .spawn(move || {
            match crate::session_retention::prune_session_ledger(&directory, chrono::Utc::now()) {
                Ok(stats) => tracing::info!(
                    sessions_pruned = stats.sessions_pruned,
                    temporary_files_pruned = stats.temporary_files_pruned,
                    "Session retention sweep completed"
                ),
                Err(error) => tracing::warn!(%error, "Session retention sweep deferred"),
            }
        }) {
        Ok(_) => {
            sweeps.insert(sessions_dir.to_path_buf(), now);
        }
        Err(error) => tracing::warn!(%error, "Session retention worker unavailable"),
    }
}
