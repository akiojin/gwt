//! Board provider resolution and routing (SPEC-2959).
//!
//! `gwt-core` owns the [`BoardProvider`] trait and the filesystem
//! [`LocalProvider`], but it cannot read `gwt-config` (the two crates are
//! mutually independent). Provider *selection* therefore lives here, in the
//! `gwt` crate, which depends on both.
//!
//! Call sites route Board reads/writes through the free-function shims below
//! instead of calling `gwt_core::coordination` directly. Each shim resolves the
//! provider per-repo via [`provider_for`], which overlays the repo's
//! `.gwt/work/board.toml` (provider / channel / tenant) onto the global
//! `[board]` config (SPEC-2963 FR-026). With a `local` resolution the shims
//! delegate to `LocalProvider` (the zero-cost default); `slack` / `teams`
//! resolve to a remote provider scoped to that project's own channel, so Board
//! posts and reads never mix across projects (FR-027).

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use gwt_config::{BoardProviderKind, ProjectBoardConfig, Settings, SlackConfig, TeamsConfig};
use gwt_core::coordination::{
    BoardAudienceScope, BoardEntry, BoardEntryKind, BoardHistoryPage, BoardProvider,
    CoordinationSnapshot, LocalProvider,
};
use gwt_core::paths::gwt_repo_local_work_dir;
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
/// `~/.gwt/config.toml`. The override is still the race-free way to keep tests
/// hermetic without depending on process-global env var mutation. Production
/// builds always read config.
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

/// Resolved Board routing for one repo: the effective provider kind plus, for
/// remote providers, the project's channel and tenant (SPEC-2963 FR-026).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedBoard {
    kind: BoardProviderKind,
    channel: Option<String>,
    tenant: Option<String>,
}

/// Overlay the per-project `board.toml` onto the global `[board]` config.
/// Precedence: project board.toml → global `[board]` (FR-031). `global_kind` is
/// the global provider kind (the test-override-aware `current_kind()` in unit
/// tests). The channel falls back to the global default channel for the
/// resolved kind; the tenant is project-only (FR-029).
fn resolve_board(
    project: &ProjectBoardConfig,
    settings: &Settings,
    global_kind: BoardProviderKind,
) -> ResolvedBoard {
    let kind = project.provider.unwrap_or(global_kind);
    let global_channel = match kind {
        BoardProviderKind::Slack => settings.board.slack.default_channel.clone(),
        BoardProviderKind::Teams => settings.board.teams.default_channel.clone(),
        BoardProviderKind::Local => None,
    };
    let trim_nonempty = |value: String| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    };
    ResolvedBoard {
        kind,
        channel: project
            .channel
            .clone()
            .or(global_channel)
            .and_then(trim_nonempty),
        tenant: project.tenant.clone().and_then(trim_nonempty),
    }
}

/// Load the OAuth token for a remote provider, keyed by tenant (FR-029). A
/// tenant-scoped token (`board-<provider>-<tenant>.json`) is preferred so
/// projects in different tenants stay independent; the legacy provider-only key
/// is the migration fallback. `dir` is the credentials directory (injectable
/// for hermetic tests).
fn load_tenant_token_in(dir: &Path, provider: &str, tenant: Option<&str>) -> Option<TokenSet> {
    if let Some(tenant) = tenant.map(str::trim).filter(|t| !t.is_empty()) {
        if let Ok(Some(token)) = token_store::load_in(dir, &format!("{provider}-{tenant}")) {
            return Some(token);
        }
    }
    token_store::load_in(dir, provider).ok().flatten()
}

/// Tenant-keyed token from the default credentials directory.
fn load_tenant_token(provider: &str, tenant: Option<&str>) -> Option<TokenSet> {
    load_tenant_token_in(&token_store::default_dir(), provider, tenant)
}

/// Build a remote provider scoped to a single project's channel. The provider
/// is constructed with an **empty** workspace channel_map so its read history
/// touches only this project's channel — never another project's (FR-027).
fn build_remote_for(
    provider: &str,
    resolved: &ResolvedBoard,
) -> std::result::Result<Box<dyn BoardProvider>, String> {
    let channel = resolved
        .channel
        .clone()
        .ok_or_else(|| format!("{provider} channel is not configured for this project"))?;
    let token = load_tenant_token(provider, resolved.tenant.as_deref()).ok_or_else(|| {
        match resolved.tenant.as_deref() {
            Some(tenant) => {
                format!("{provider} is not signed in for tenant '{tenant}' (this project)")
            }
            None => format!("{provider} is not signed in"),
        }
    })?;
    let http = Box::new(ReqwestHttpClient::new());
    Ok(match provider {
        "teams" => Box::new(TeamsProvider::new(
            token.access_token,
            channel,
            BTreeMap::new(),
            http,
            60,
        )),
        _ => Box::new(SlackProvider::new(
            token.access_token,
            channel,
            BTreeMap::new(),
            http,
            60,
        )),
    })
}

/// The active provider for a specific repo, resolved from the repo's
/// `.gwt/work/board.toml` overlaid on the global settings (SPEC-2963 FR-026).
/// Each repo gets its own provider scoped to its own channel, so Board posts and
/// reads never mix across projects. `local` stays on the zero-cost fast path.
pub fn provider_for(worktree_root: &Path) -> Box<dyn BoardProvider> {
    let project = ProjectBoardConfig::load_from_work_dir(&gwt_repo_local_work_dir(worktree_root));
    let global_kind = current_kind();
    // Fast path: no project override and global is local → zero-cost local,
    // identical to the pre-per-project behaviour (avoids loading Settings).
    if project.is_empty() && global_kind == BoardProviderKind::Local {
        return Box::new(LocalProvider);
    }
    let settings = Settings::load().unwrap_or_default();
    let resolved = resolve_board(&project, &settings, global_kind);
    match resolved.kind {
        BoardProviderKind::Local => Box::new(LocalProvider),
        BoardProviderKind::Slack => {
            build_remote_for("slack", &resolved).unwrap_or_else(UnconfiguredProvider::boxed)
        }
        BoardProviderKind::Teams => {
            build_remote_for("teams", &resolved).unwrap_or_else(UnconfiguredProvider::boxed)
        }
    }
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
    provider_for(worktree_root).post_entry(worktree_root, entry)
}

/// Load the hot projection snapshot through the active provider.
pub fn load_snapshot(worktree_root: &Path) -> Result<CoordinationSnapshot> {
    provider_for(worktree_root).load_snapshot(worktree_root)
}

/// Load the snapshot filtered to an audience scope.
pub fn load_snapshot_for_scope(
    worktree_root: &Path,
    scope: &BoardAudienceScope,
) -> Result<CoordinationSnapshot> {
    provider_for(worktree_root).load_snapshot_for_scope(worktree_root, scope)
}

/// Load entries updated strictly after `since`.
pub fn load_entries_since(worktree_root: &Path, since: DateTime<Utc>) -> Result<Vec<BoardEntry>> {
    provider_for(worktree_root).load_entries_since(worktree_root, since)
}

/// Load entries updated strictly after `since`, filtered to a scope.
pub fn load_entries_since_for_scope(
    worktree_root: &Path,
    since: DateTime<Utc>,
    scope: &BoardAudienceScope,
) -> Result<Vec<BoardEntry>> {
    provider_for(worktree_root).load_entries_since_for_scope(worktree_root, since, scope)
}

/// Whether `author` posted a message of `kind` within `within`.
pub fn has_recent_post_by(
    worktree_root: &Path,
    author: &str,
    kind: &BoardEntryKind,
    within: chrono::Duration,
) -> Result<bool> {
    provider_for(worktree_root).has_recent_post_by(worktree_root, author, kind, within)
}

/// Whether an entry with `entry_id` exists.
pub fn board_entry_exists(worktree_root: &Path, entry_id: &str) -> Result<bool> {
    provider_for(worktree_root).board_entry_exists(worktree_root, entry_id)
}

/// Load a page of older entries before `before_entry_id`.
pub fn load_entries_before(
    worktree_root: &Path,
    before_entry_id: Option<&str>,
    limit: usize,
) -> Result<BoardHistoryPage> {
    provider_for(worktree_root).load_entries_before(worktree_root, before_entry_id, limit)
}

/// Load a page of older entries before `before_entry_id`, filtered to a scope.
pub fn load_entries_before_for_scope(
    worktree_root: &Path,
    before_entry_id: Option<&str>,
    limit: usize,
    scope: &BoardAudienceScope,
) -> Result<BoardHistoryPage> {
    provider_for(worktree_root).load_entries_before_for_scope(
        worktree_root,
        before_entry_id,
        limit,
        scope,
    )
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

    // --- Per-project isolation (SPEC-2963 FR-025..FR-032) -------------------

    #[test]
    fn resolve_board_uses_project_channel_per_project() {
        // FR-027 separation: two projects resolve to two different channels.
        let settings = Settings::default();
        let proj_a = ProjectBoardConfig {
            provider: Some(BoardProviderKind::Slack),
            channel: Some("C-A".into()),
            tenant: Some("acme".into()),
        };
        let proj_b = ProjectBoardConfig {
            provider: Some(BoardProviderKind::Slack),
            channel: Some("C-B".into()),
            tenant: Some("beta".into()),
        };
        let a = resolve_board(&proj_a, &settings, BoardProviderKind::Local);
        let b = resolve_board(&proj_b, &settings, BoardProviderKind::Local);
        assert_eq!(a.kind, BoardProviderKind::Slack);
        assert_eq!(a.channel.as_deref(), Some("C-A"));
        assert_eq!(a.tenant.as_deref(), Some("acme"));
        assert_eq!(b.channel.as_deref(), Some("C-B"));
        assert_ne!(a.channel, b.channel, "projects must not share a channel");
    }

    #[test]
    fn resolve_board_falls_back_to_global_default_channel() {
        // FR-031: an empty project inherits the global default channel + kind.
        let mut settings = Settings::default();
        settings.board.slack.default_channel = Some("  C-GLOBAL  ".into());
        let resolved = resolve_board(
            &ProjectBoardConfig::default(),
            &settings,
            BoardProviderKind::Slack,
        );
        assert_eq!(resolved.kind, BoardProviderKind::Slack);
        assert_eq!(resolved.channel.as_deref(), Some("C-GLOBAL"));
        assert!(resolved.tenant.is_none());
    }

    #[test]
    fn resolve_board_project_provider_overrides_global() {
        // FR-028: a project can pick local while global is slack, and vice versa.
        let settings = Settings::default();
        let local_proj = ProjectBoardConfig {
            provider: Some(BoardProviderKind::Local),
            ..Default::default()
        };
        assert_eq!(
            resolve_board(&local_proj, &settings, BoardProviderKind::Slack).kind,
            BoardProviderKind::Local
        );
        let teams_proj = ProjectBoardConfig {
            provider: Some(BoardProviderKind::Teams),
            channel: Some("team/chan".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_board(&teams_proj, &settings, BoardProviderKind::Local).kind,
            BoardProviderKind::Teams
        );
    }

    #[test]
    fn provider_for_honors_project_local_override_while_global_remote() {
        // End-to-end (hermetic): a repo pinned to local works even when the
        // global provider is Slack — proving per-project provider selection
        // without touching any remote credentials.
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join(".gwt").join("work");
        ProjectBoardConfig {
            provider: Some(BoardProviderKind::Local),
            ..Default::default()
        }
        .save_to_work_dir(&work)
        .unwrap();
        let _slack = test_provider_override::force(BoardProviderKind::Slack);
        assert!(provider_for(dir.path()).load_snapshot(dir.path()).is_ok());
    }

    #[test]
    fn build_remote_for_without_channel_is_unconfigured() {
        // FR-010: a remote with no resolved channel surfaces an error (no silent local).
        let resolved = ResolvedBoard {
            kind: BoardProviderKind::Slack,
            channel: None,
            tenant: None,
        };
        assert!(build_remote_for("slack", &resolved).is_err());
    }

    #[test]
    fn tenant_token_keys_are_isolated_per_tenant() {
        // FR-029: each tenant's token is independent; another tenant cannot read it.
        let dir = tempfile::tempdir().unwrap();
        let tok = TokenSet {
            access_token: "xoxb-acme".into(),
            refresh_token: None,
            expires_at: None,
        };
        token_store::save_in(dir.path(), "slack-acme", &tok).unwrap();
        assert_eq!(
            load_tenant_token_in(dir.path(), "slack", Some("acme")),
            Some(tok)
        );
        assert_eq!(
            load_tenant_token_in(dir.path(), "slack", Some("beta")),
            None
        );
    }

    #[test]
    fn tenant_token_falls_back_to_legacy_provider_key() {
        // Migration: a legacy provider-only token is still found.
        let dir = tempfile::tempdir().unwrap();
        let tok = TokenSet {
            access_token: "legacy".into(),
            refresh_token: None,
            expires_at: None,
        };
        token_store::save_in(dir.path(), "slack", &tok).unwrap();
        assert_eq!(
            load_tenant_token_in(dir.path(), "slack", None),
            Some(tok.clone())
        );
        assert_eq!(
            load_tenant_token_in(dir.path(), "slack", Some("acme")),
            Some(tok)
        );
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
