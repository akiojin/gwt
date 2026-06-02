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

use chrono::{DateTime, Utc};
use gwt_config::{BoardProviderKind, Settings, SlackConfig, TeamsConfig};
use gwt_core::coordination::{
    BoardAudienceScope, BoardEntry, BoardEntryKind, BoardHistoryPage, BoardProvider,
    CoordinationSnapshot, LocalProvider,
};
use gwt_core::{GwtError, Result};

use crate::board_remote::http::ReqwestHttpClient;
use crate::board_remote::slack::SlackProvider;
use crate::board_remote::teams::TeamsProvider;
use crate::board_remote::token_store::{self, TokenSet};

/// The currently selected provider kind, read fresh from `Settings`. Reading
/// per call (rather than caching a process global) keeps a settings change
/// effective immediately (FR-008) and avoids cross-call/test state leakage.
/// Unreadable config falls back to `local` (FR-004).
///
/// Test seam: in `#[cfg(test)]` builds the kind comes from a per-thread
/// override that defaults to `Local`, so board unit tests run against the
/// filesystem provider regardless of the developer machine's
/// `~/.gwt/config.toml`. `Settings::global_config_path` resolves via
/// `dirs::home_dir()` (which ignores `HOME`/`USERPROFILE` on Windows), so the
/// global config cannot be isolated with env vars; the override is the
/// race-free way to keep tests hermetic. Production builds always read config.
pub fn current_kind() -> BoardProviderKind {
    #[cfg(test)]
    {
        test_provider_override::current()
    }
    #[cfg(not(test))]
    {
        Settings::load()
            .map(|s| s.board.provider)
            .unwrap_or_default()
    }
}

/// Per-thread provider-kind override used only by unit tests (see
/// [`current_kind`]). Thread-local so parallel tests never race on it.
#[cfg(test)]
pub(crate) mod test_provider_override {
    use super::BoardProviderKind;
    use std::cell::Cell;

    thread_local! {
        static KIND: Cell<BoardProviderKind> = const { Cell::new(BoardProviderKind::Local) };
    }

    /// Current override for this thread (defaults to `Local`).
    pub(crate) fn current() -> BoardProviderKind {
        KIND.with(Cell::get)
    }

    /// Force `kind` for the duration of the returned guard, then restore.
    pub(crate) fn force(kind: BoardProviderKind) -> Guard {
        let previous = KIND.with(|cell| cell.replace(kind));
        Guard(previous)
    }

    /// RAII guard restoring the previous override on drop.
    pub(crate) struct Guard(BoardProviderKind);

    impl Drop for Guard {
        fn drop(&mut self) {
            KIND.with(|cell| cell.set(self.0));
        }
    }
}

/// A provider that fails every operation with a clear reason. Returned when a
/// remote provider is selected but not usable yet (not signed in,
/// misconfigured, or not implemented). Per FR-010 we surface the reason rather
/// than silently falling back to local.
struct UnconfiguredProvider {
    reason: String,
}

impl UnconfiguredProvider {
    fn boxed(reason: impl Into<String>) -> Box<dyn BoardProvider> {
        Box::new(Self {
            reason: reason.into(),
        })
    }

    fn err<T>(&self) -> Result<T> {
        Err(GwtError::Other(self.reason.clone()))
    }
}

impl BoardProvider for UnconfiguredProvider {
    fn post_entry(&self, _: &Path, _: BoardEntry) -> Result<CoordinationSnapshot> {
        self.err()
    }
    fn load_snapshot(&self, _: &Path) -> Result<CoordinationSnapshot> {
        self.err()
    }
    fn load_snapshot_for_scope(
        &self,
        _: &Path,
        _: &BoardAudienceScope,
    ) -> Result<CoordinationSnapshot> {
        self.err()
    }
    fn load_entries_since(&self, _: &Path, _: DateTime<Utc>) -> Result<Vec<BoardEntry>> {
        self.err()
    }
    fn load_entries_since_for_scope(
        &self,
        _: &Path,
        _: DateTime<Utc>,
        _: &BoardAudienceScope,
    ) -> Result<Vec<BoardEntry>> {
        self.err()
    }
    fn has_recent_post_by(
        &self,
        _: &Path,
        _: &str,
        _: &BoardEntryKind,
        _: chrono::Duration,
    ) -> Result<bool> {
        self.err()
    }
    fn board_entry_exists(&self, _: &Path, _: &str) -> Result<bool> {
        self.err()
    }
    fn load_entries_before(&self, _: &Path, _: Option<&str>, _: usize) -> Result<BoardHistoryPage> {
        self.err()
    }
    fn load_entries_before_for_scope(
        &self,
        _: &Path,
        _: Option<&str>,
        _: usize,
        _: &BoardAudienceScope,
    ) -> Result<BoardHistoryPage> {
        self.err()
    }
}

/// Build the Slack provider from its config and a stored token. Returns a clear
/// error reason when not usable yet (FR-010 — no silent local fallback).
fn build_slack(
    config: &SlackConfig,
    token: Option<TokenSet>,
) -> std::result::Result<Box<dyn BoardProvider>, String> {
    let default_channel = config
        .default_channel
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Slack default channel is not configured".to_string())?;
    let token = token.ok_or_else(|| "Slack is not signed in".to_string())?;
    Ok(Box::new(SlackProvider::new(
        token.access_token,
        default_channel,
        config.channel_map.clone(),
        Box::new(ReqwestHttpClient::new()),
        60,
    )))
}

/// Build the Teams provider from its config and a stored token (FR-010).
fn build_teams(
    config: &TeamsConfig,
    token: Option<TokenSet>,
) -> std::result::Result<Box<dyn BoardProvider>, String> {
    let default_channel = config
        .default_channel
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Teams default channel (team_id/channel_id) is not configured".to_string()
        })?;
    let token = token.ok_or_else(|| "Teams is not signed in".to_string())?;
    Ok(Box::new(TeamsProvider::new(
        token.access_token,
        default_channel,
        config.channel_map.clone(),
        Box::new(ReqwestHttpClient::new()),
        60,
    )))
}

/// Build the active remote provider from settings + stored credentials.
fn build_remote(kind: BoardProviderKind, settings: &Settings) -> Box<dyn BoardProvider> {
    match kind {
        BoardProviderKind::Local => Box::new(LocalProvider),
        BoardProviderKind::Slack => {
            let token = token_store::load("slack").ok().flatten();
            build_slack(&settings.board.slack, token).unwrap_or_else(UnconfiguredProvider::boxed)
        }
        BoardProviderKind::Teams => {
            let token = token_store::load("teams").ok().flatten();
            build_teams(&settings.board.teams, token).unwrap_or_else(UnconfiguredProvider::boxed)
        }
    }
}

/// The active provider, resolved from current settings. `local` stays on the
/// zero-cost fast path; remote providers load settings + credentials.
pub fn provider() -> Box<dyn BoardProvider> {
    // `current_kind()` honours the test override (defaulting to `Local` in
    // tests) and reads `Settings` in production, so unit tests stay hermetic.
    match current_kind() {
        BoardProviderKind::Local => Box::new(LocalProvider),
        kind => build_remote(kind, &Settings::load().unwrap_or_default()),
    }
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
    fn build_remote_local_reads_empty_board() {
        let dir = tempfile::tempdir().unwrap();
        let provider = build_remote(BoardProviderKind::Local, &Settings::default());
        assert!(provider
            .load_snapshot(dir.path())
            .unwrap()
            .board
            .entries
            .is_empty());
    }

    #[test]
    fn build_remote_slack_without_config_is_unconfigured() {
        // SPEC-2959/2963 FR-010: a selected-but-unusable remote surfaces an
        // error rather than silently serving local. Default settings have no
        // Slack channel, so the provider is Unconfigured regardless of tokens.
        let dir = tempfile::tempdir().unwrap();
        let provider = build_remote(BoardProviderKind::Slack, &Settings::default());
        assert!(provider.load_snapshot(dir.path()).is_err());
    }

    #[test]
    fn build_remote_teams_is_unconfigured_until_phase_6() {
        let dir = tempfile::tempdir().unwrap();
        let provider = build_remote(BoardProviderKind::Teams, &Settings::default());
        let err = provider.load_snapshot(dir.path()).unwrap_err();
        assert!(err.to_string().contains("Teams"));
    }

    #[test]
    fn current_kind_defaults_to_local_in_tests_and_provider_is_hermetic() {
        // Without an override the unit-test default is Local regardless of the
        // machine's config.toml, so board behaviour tests never depend on a
        // configured remote provider.
        assert_eq!(current_kind(), BoardProviderKind::Local);
        let dir = tempfile::tempdir().unwrap();
        assert!(provider().load_snapshot(dir.path()).is_ok());
    }

    #[test]
    fn test_override_controls_current_kind_and_restores() {
        // The override drives `current_kind()` (which `provider()` routes on),
        // and restores on guard drop. Kept hermetic by asserting on
        // `current_kind()` only — `provider()` for a remote kind reads the real
        // machine config/credentials, which must not leak into this unit test.
        assert_eq!(current_kind(), BoardProviderKind::Local);
        {
            let _slack = test_provider_override::force(BoardProviderKind::Slack);
            assert_eq!(current_kind(), BoardProviderKind::Slack);
        }
        assert_eq!(current_kind(), BoardProviderKind::Local);
        {
            let _teams = test_provider_override::force(BoardProviderKind::Teams);
            assert_eq!(current_kind(), BoardProviderKind::Teams);
        }
        assert_eq!(current_kind(), BoardProviderKind::Local);
    }

    #[test]
    fn build_slack_requires_channel_and_token() {
        assert!(build_slack(&SlackConfig::default(), None).is_err());

        let with_channel = SlackConfig {
            default_channel: Some("CH".to_string()),
            ..Default::default()
        };
        // channel but no token → still an error.
        assert!(build_slack(&with_channel, None).is_err());

        let token = TokenSet {
            access_token: "xoxb".to_string(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(build_slack(&with_channel, Some(token)).is_ok());
    }
}
