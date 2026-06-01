//! Board provider resolution and routing (SPEC-2959).
//!
//! `gwt-core` owns the [`BoardProvider`] trait and the filesystem
//! [`LocalProvider`], but it cannot read `gwt-config` (the two crates are
//! mutually independent). Provider *selection* therefore lives here, in the
//! `gwt` crate, which depends on both.
//!
//! Call sites route Board reads/writes through the free-function shims below
//! instead of calling `gwt_core::coordination` directly. With `board.provider
//! = local` (the default and only implemented backend) the shims delegate to
//! `LocalProvider`, so behavior is identical to the pre-abstraction path.
//! A future Slack/Teams adapter (Issue #2960) plugs in via [`resolve`].

use std::path::Path;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use gwt_config::{BoardProviderKind, Settings};
use gwt_core::coordination::{
    BoardAudienceScope, BoardEntry, BoardEntryKind, BoardHistoryPage, BoardProvider,
    CoordinationSnapshot, LocalProvider,
};
use gwt_core::Result;
use tracing::warn;

/// Process-wide cached provider selection. Lazily initialized from `Settings`
/// on first use and refreshed by the settings-save path (FR-008).
static PROVIDER_KIND: RwLock<Option<BoardProviderKind>> = RwLock::new(None);

/// Override the active provider selection (e.g. after the settings UI saves a
/// new value). Takes effect for subsequent [`provider`] calls in this process.
pub fn set_provider_kind(kind: BoardProviderKind) {
    if let Ok(mut guard) = PROVIDER_KIND.write() {
        *guard = Some(kind);
    }
}

/// The currently selected provider kind, loading from `Settings` once and
/// caching the result. Unreadable config falls back to `local` (FR-004).
pub fn current_kind() -> BoardProviderKind {
    if let Ok(guard) = PROVIDER_KIND.read() {
        if let Some(kind) = *guard {
            return kind;
        }
    }
    let kind = Settings::load()
        .map(|s| s.board.provider)
        .unwrap_or_default();
    if let Ok(mut guard) = PROVIDER_KIND.write() {
        *guard = Some(kind);
    }
    kind
}

/// Build a provider for `kind`. Unimplemented remote providers warn once and
/// fall back to `LocalProvider` so the Board keeps working (FR-004).
pub fn resolve(kind: BoardProviderKind) -> Box<dyn BoardProvider> {
    match kind {
        BoardProviderKind::Local => Box::new(LocalProvider),
        other => {
            warn!(
                provider = other.as_str(),
                "board provider not implemented yet; falling back to local"
            );
            Box::new(LocalProvider)
        }
    }
}

/// The active provider, resolved from the cached configuration.
pub fn provider() -> Box<dyn BoardProvider> {
    resolve(current_kind())
}

// --- Free-function shims (same signatures as `gwt_core::coordination`) -------

/// Append a Board entry through the active provider.
pub fn post_entry(worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
    provider().post_entry(worktree_root, entry)
}

/// Load the hot projection snapshot through the active provider.
pub fn load_snapshot(worktree_root: &Path) -> Result<CoordinationSnapshot> {
    provider().load_snapshot(worktree_root)
}

/// Load the snapshot filtered to an audience scope.
pub fn load_snapshot_for_scope(
    worktree_root: &Path,
    scope: &BoardAudienceScope,
) -> Result<CoordinationSnapshot> {
    provider().load_snapshot_for_scope(worktree_root, scope)
}

/// Load entries updated strictly after `since`.
pub fn load_entries_since(worktree_root: &Path, since: DateTime<Utc>) -> Result<Vec<BoardEntry>> {
    provider().load_entries_since(worktree_root, since)
}

/// Load entries updated strictly after `since`, filtered to a scope.
pub fn load_entries_since_for_scope(
    worktree_root: &Path,
    since: DateTime<Utc>,
    scope: &BoardAudienceScope,
) -> Result<Vec<BoardEntry>> {
    provider().load_entries_since_for_scope(worktree_root, since, scope)
}

/// Whether `author` posted a message of `kind` within `within`.
pub fn has_recent_post_by(
    worktree_root: &Path,
    author: &str,
    kind: &BoardEntryKind,
    within: chrono::Duration,
) -> Result<bool> {
    provider().has_recent_post_by(worktree_root, author, kind, within)
}

/// Whether an entry with `entry_id` exists.
pub fn board_entry_exists(worktree_root: &Path, entry_id: &str) -> Result<bool> {
    provider().board_entry_exists(worktree_root, entry_id)
}

/// Load a page of older entries before `before_entry_id`.
pub fn load_entries_before(
    worktree_root: &Path,
    before_entry_id: Option<&str>,
    limit: usize,
) -> Result<BoardHistoryPage> {
    provider().load_entries_before(worktree_root, before_entry_id, limit)
}

/// Load a page of older entries before `before_entry_id`, filtered to a scope.
pub fn load_entries_before_for_scope(
    worktree_root: &Path,
    before_entry_id: Option<&str>,
    limit: usize,
    scope: &BoardAudienceScope,
) -> Result<BoardHistoryPage> {
    provider().load_entries_before_for_scope(worktree_root, before_entry_id, limit, scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_local_and_unimplemented_fall_back_to_local() {
        // Local resolves; Slack/Teams fall back to local (FR-004). We can only
        // assert they produce a working provider that reads an empty board.
        let dir = tempfile::tempdir().unwrap();
        for kind in [
            BoardProviderKind::Local,
            BoardProviderKind::Slack,
            BoardProviderKind::Teams,
        ] {
            let provider = resolve(kind);
            let snapshot = provider.load_snapshot(dir.path()).unwrap();
            assert!(snapshot.board.entries.is_empty());
        }
    }

    #[test]
    fn set_provider_kind_is_reflected_by_current_kind() {
        set_provider_kind(BoardProviderKind::Local);
        assert_eq!(current_kind(), BoardProviderKind::Local);
    }
}
