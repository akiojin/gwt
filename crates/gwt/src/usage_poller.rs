//! Background provider-usage poller (SPEC-2970).
//!
//! Runs as a tokio task for the lifetime of the GUI process but only does work
//! while at least one WebSocket client is connected (FR-007). Each tick it
//! builds a [`UsageSnapshot`] and broadcasts it as
//! [`BackendEvent::ProviderUsage`].
//!
//! Account-level usage is sourced here:
//! - Codex: local rollout files (always allowed when enabled).
//! - Claude: undocumented `/api/oauth/usage`, opt-in, rate-limited to
//!   `CLAUDE_MIN_FETCH_SECS`; the last successful result is cached between
//!   fetch windows so the UI keeps showing the value (with staleness applied).
//!
//! Per-session rows are gathered by [`collect_sessions`], which enumerates the
//! gwt session store and reads each session's local rollout / transcript.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use gwt_agent::{AgentId, AgentStatus, Session};
use gwt_config::{usage_config::UsageConfig, Settings};
use gwt_core::usage::{
    claude, codex, consumption,
    state::{apply_staleness, should_fetch_claude, DEFAULT_STALE_AFTER_SECS},
    ProviderConsumption, ProviderUsage, SessionUsage, UsageProvider, UsageSnapshot, UsageState,
    WindowKind,
};
use tokio::sync::Notify;
use tokio::time::{interval, MissedTickBehavior};

use crate::embedded_server::ClientHub;
use crate::OutboundEvent;
use gwt::BackendEvent;

/// Base polling cadence. Codex (local) refreshes every tick; Claude is further
/// gated by [`should_fetch_claude`].
const TICK_SECS: u64 = 30;

/// Consumption aggregation scans many local files, so it is recomputed at most
/// this often (or immediately on a forced refresh).
const CONSUMPTION_CACHE_SECS: i64 = 300;

/// Spawn the usage poller onto the shared tokio runtime.
pub fn spawn_usage_poller(
    runtime: &tokio::runtime::Runtime,
    clients: ClientHub,
    refresh: Arc<Notify>,
) {
    drop(runtime.handle().spawn(run(clients, refresh)));
}

async fn run(clients: ClientHub, refresh: Arc<Notify>) {
    let mut poller = Poller::default();
    let mut ticker = interval(Duration::from_secs(TICK_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        // A refresh request forces an immediate Claude re-fetch (bypassing the
        // 180s cadence gate); a normal tick respects it.
        let forced = tokio::select! {
            _ = ticker.tick() => false,
            _ = refresh.notified() => true,
        };
        // FR-007: do nothing (no network, no credential access) when the GUI
        // is closed.
        if !clients.has_clients() {
            continue;
        }
        let snapshot = poller.poll_once(Utc::now(), forced).await;
        clients.dispatch(vec![OutboundEvent::broadcast(
            BackendEvent::ProviderUsage {
                accounts: snapshot.accounts,
                sessions: snapshot.sessions,
                consumption: snapshot.consumption,
            },
        )]);
    }
}

/// Mutable state carried across polls (Claude cache + cadence + UA +
/// consumption cache).
#[derive(Default)]
struct Poller {
    cached_claude: Option<ProviderUsage>,
    last_claude_fetch: Option<DateTime<Utc>>,
    cached_user_agent: Option<String>,
    cached_consumption: Vec<ProviderConsumption>,
    last_consumption_at: Option<DateTime<Utc>>,
}

impl Poller {
    async fn poll_once(&mut self, now: DateTime<Utc>, force: bool) -> UsageSnapshot {
        let config = Settings::load().unwrap_or_default().usage;
        let accounts = vec![
            self.codex_account(&config, now),
            self.claude_account(&config, now, force).await,
        ];
        let consumption = self.consumption(&config, &accounts, now, force);
        UsageSnapshot {
            accounts,
            sessions: collect_sessions(&config),
            consumption,
        }
    }

    /// Daily/weekly consumption rollups. Heavy (scans many files), so it is
    /// cached for [`CONSUMPTION_CACHE_SECS`] unless `force` is set. The weekly
    /// window start per provider comes from the account snapshot's weekly
    /// `resets_at` (FR: "this week" aligned to the rate-limit window).
    fn consumption(
        &mut self,
        config: &UsageConfig,
        accounts: &[ProviderUsage],
        now: DateTime<Utc>,
        force: bool,
    ) -> Vec<ProviderConsumption> {
        let due = force
            || match self.last_consumption_at {
                None => true,
                Some(at) => {
                    now.signed_duration_since(at)
                        >= chrono::Duration::seconds(CONSUMPTION_CACHE_SECS)
                }
            };
        if !due {
            return self.cached_consumption.clone();
        }
        let mut out = Vec::new();
        if config.codex_enabled {
            if let Some(home) = codex::codex_home() {
                let week_start = week_start_for(accounts, UsageProvider::Codex, now);
                out.push(consumption::read_codex_consumption(&home, week_start, now));
            }
        }
        // Claude consumption is read from local transcripts only, so it does not
        // require the account opt-in (mirrors Claude per-session usage).
        if let Some(home) = claude::claude_home() {
            let week_start = week_start_for(accounts, UsageProvider::ClaudeCode, now);
            out.push(consumption::read_claude_consumption(&home, week_start, now));
        }
        self.cached_consumption = out.clone();
        self.last_consumption_at = Some(now);
        out
    }

    fn codex_account(&self, config: &UsageConfig, now: DateTime<Utc>) -> ProviderUsage {
        if !config.codex_enabled {
            return ProviderUsage::degraded(UsageProvider::Codex, UsageState::Disabled);
        }
        let Some(home) = codex::codex_home() else {
            return ProviderUsage::degraded(UsageProvider::Codex, UsageState::NoData);
        };
        let mut account = codex::read_codex_account(&home, now);
        account.state = apply_staleness(
            account.state,
            account.fetched_at,
            now,
            DEFAULT_STALE_AFTER_SECS,
        );
        account
    }

    async fn claude_account(
        &mut self,
        config: &UsageConfig,
        now: DateTime<Utc>,
        force: bool,
    ) -> ProviderUsage {
        if !config.claude_account_enabled {
            return ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::Disabled);
        }
        let Some(home) = claude::claude_home() else {
            return ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData);
        };

        if force || should_fetch_claude(self.last_claude_fetch, now) {
            let Some(creds) = claude::resolve_claude_creds(&home) else {
                return serve_cached_or(
                    &self.cached_claude,
                    now,
                    ProviderUsage::degraded(
                        UsageProvider::ClaudeCode,
                        UsageState::Unavailable {
                            reason: "no credentials".to_string(),
                        },
                    ),
                );
            };
            let user_agent = self
                .cached_user_agent
                .get_or_insert_with(claude::claude_user_agent)
                .clone();
            let fresh = claude::fetch_claude_account(&creds, &user_agent, now).await;
            self.last_claude_fetch = Some(now);
            // Only a successful fetch (has windows) replaces the cache. A
            // transient failure (429 / auth / network) keeps the last good
            // value shown as stale instead of wiping the display.
            if !fresh.windows.is_empty() {
                self.cached_claude = Some(fresh.clone());
                return fresh;
            }
            return serve_cached_or(&self.cached_claude, now, fresh);
        }

        // Within the cooldown window: reuse the cached value with staleness.
        serve_cached_or(
            &self.cached_claude,
            now,
            ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData),
        )
    }
}

/// Serve the last good cached account (as stale) when present, else the given
/// fallback. Keeps a transient fetch failure (e.g. 429) from wiping a usable
/// display. "Good" = the cached value carries windows.
fn serve_cached_or(
    cached: &Option<ProviderUsage>,
    now: DateTime<Utc>,
    fallback: ProviderUsage,
) -> ProviderUsage {
    match cached {
        Some(c) if !c.windows.is_empty() => {
            let mut c = c.clone();
            c.state = apply_staleness(c.state, c.fetched_at, now, DEFAULT_STALE_AFTER_SECS);
            c
        }
        _ => fallback,
    }
}

/// Start instant of the weekly consumption window: the provider's weekly
/// rate-limit `resets_at` minus 7 days, or a rolling 7-day fallback.
fn week_start_for(
    accounts: &[ProviderUsage],
    provider: UsageProvider,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    accounts
        .iter()
        .find(|a| a.provider == provider)
        .and_then(|a| a.windows.iter().find(|w| w.kind == WindowKind::Weekly))
        .and_then(|w| w.resets_at)
        .map(|reset| reset - chrono::Duration::days(7))
        .unwrap_or_else(|| now - chrono::Duration::days(7))
}

/// Map a gwt agent id to a usage provider (only Codex / Claude Code are in
/// scope; FR-014).
fn session_provider(agent_id: &AgentId) -> Option<UsageProvider> {
    match agent_id {
        AgentId::Codex => Some(UsageProvider::Codex),
        AgentId::ClaudeCode => Some(UsageProvider::ClaudeCode),
        _ => None,
    }
}

/// Whether a session is currently active enough to show per-session usage.
fn is_active_status(status: AgentStatus) -> bool {
    matches!(
        status,
        AgentStatus::Running
            | AgentStatus::Idle
            | AgentStatus::WaitingInput
            | AgentStatus::Interrupted
    )
}

/// SPEC-2970 FR-015..FR-020: enumerate active gwt sessions and build their
/// per-session usage from local rollout / transcript files. Keyed by the gwt
/// session id so the frontend can match it to agent cards. Per-session reads
/// are local-only; Claude does not require the account opt-in here.
fn collect_sessions(config: &UsageConfig) -> Vec<SessionUsage> {
    let mut out = Vec::new();
    let dir = gwt_core::paths::gwt_sessions_dir();
    let codex_home = codex::codex_home();
    let claude_home = claude::claude_home();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(session) = Session::load_and_migrate(&path) else {
            continue;
        };
        if !is_active_status(session.status) {
            continue;
        }
        let Some(provider) = session_provider(&session.agent_id) else {
            continue;
        };
        let Some(agent_sid) = session.agent_session_id.clone() else {
            continue;
        };
        let usage = match provider {
            UsageProvider::Codex => {
                if !config.codex_enabled {
                    continue;
                }
                codex_home
                    .as_ref()
                    .and_then(|home| codex::read_codex_session(home, &agent_sid))
            }
            UsageProvider::ClaudeCode => claude_home
                .as_ref()
                .and_then(|home| claude::read_claude_session(home, &agent_sid)),
        };
        if let Some(mut usage) = usage {
            // Re-key to the gwt session id (frontend agent cards use it) and
            // mark API-key backend sessions ineligible for subscription frames.
            usage.session_id = session.id.clone();
            usage.eligible = session.backend_id.is_none();
            out.push(usage);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap()
    }

    #[test]
    fn serve_cached_keeps_last_good_on_failure() {
        let when = now();
        let good = ProviderUsage {
            provider: UsageProvider::ClaudeCode,
            plan: Some("max".into()),
            windows: vec![gwt_core::usage::UsageWindow::new(
                WindowKind::FiveHour,
                19.0,
                None,
            )],
            limit_reached: false,
            state: UsageState::Ok,
            fetched_at: Some(when),
        };
        let fail = ProviderUsage::degraded(
            UsageProvider::ClaudeCode,
            UsageState::Unavailable {
                reason: "rate limited (429)".into(),
            },
        );
        // A transient failure must not wipe a usable cached value.
        let served = serve_cached_or(&Some(good), when, fail.clone());
        assert!(!served.windows.is_empty());
        assert_eq!(served.plan.as_deref(), Some("max"));
        // No cache → fallback is returned.
        let served2 = serve_cached_or(&None, when, fail);
        assert!(served2.windows.is_empty());
        assert!(matches!(served2.state, UsageState::Unavailable { .. }));
    }

    #[test]
    fn provider_mapping_scopes_to_codex_and_claude() {
        assert_eq!(
            session_provider(&AgentId::Codex),
            Some(UsageProvider::Codex)
        );
        assert_eq!(
            session_provider(&AgentId::ClaudeCode),
            Some(UsageProvider::ClaudeCode)
        );
        assert_eq!(session_provider(&AgentId::Custom("foo".into())), None);
    }

    #[test]
    fn active_status_excludes_stopped_and_unknown() {
        assert!(is_active_status(AgentStatus::Running));
        assert!(is_active_status(AgentStatus::WaitingInput));
        assert!(!is_active_status(AgentStatus::Stopped));
        assert!(!is_active_status(AgentStatus::Unknown));
    }

    #[test]
    fn codex_disabled_yields_disabled_state() {
        let poller = Poller::default();
        let config = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: false,
        };
        let account = poller.codex_account(&config, now());
        assert_eq!(account.provider, UsageProvider::Codex);
        assert_eq!(account.state, UsageState::Disabled);
    }

    #[tokio::test]
    async fn claude_disabled_yields_disabled_state() {
        let mut poller = Poller::default();
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: false,
        };
        let account = poller.claude_account(&config, now(), false).await;
        assert_eq!(account.provider, UsageProvider::ClaudeCode);
        assert_eq!(account.state, UsageState::Disabled);
        // No fetch should have happened.
        assert!(poller.last_claude_fetch.is_none());
    }

    fn account_with_weekly_reset(
        provider: UsageProvider,
        reset: Option<DateTime<Utc>>,
    ) -> ProviderUsage {
        let kind = if reset.is_some() {
            WindowKind::Weekly
        } else {
            WindowKind::FiveHour
        };
        ProviderUsage {
            provider,
            plan: None,
            windows: vec![gwt_core::usage::UsageWindow::new(kind, 10.0, reset)],
            limit_reached: false,
            state: UsageState::Ok,
            fetched_at: Some(now()),
        }
    }

    #[test]
    fn week_start_uses_weekly_reset_minus_seven_days() {
        let reset = now() + chrono::Duration::days(2);
        let accounts = vec![account_with_weekly_reset(UsageProvider::Codex, Some(reset))];
        let ws = week_start_for(&accounts, UsageProvider::Codex, now());
        assert_eq!(ws, reset - chrono::Duration::days(7));
    }

    #[test]
    fn week_start_falls_back_to_rolling_window() {
        // Provider absent from the snapshot → rolling 7-day fallback.
        let accounts = vec![account_with_weekly_reset(UsageProvider::Codex, Some(now()))];
        let fb = week_start_for(&accounts, UsageProvider::ClaudeCode, now());
        assert_eq!(fb, now() - chrono::Duration::days(7));
        // Provider present but with no Weekly window → same fallback.
        let no_weekly = vec![account_with_weekly_reset(UsageProvider::Codex, None)];
        let fb2 = week_start_for(&no_weekly, UsageProvider::Codex, now());
        assert_eq!(fb2, now() - chrono::Duration::days(7));
    }

    #[test]
    fn consumption_serves_cache_when_not_due() {
        let mut poller = Poller::default();
        let cached = vec![ProviderConsumption::empty(UsageProvider::Codex, now())];
        poller.cached_consumption = cached.clone();
        poller.last_consumption_at = Some(now());
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: false,
        };
        // Not forced and within the cache window → returns the cached value
        // untouched without scanning the filesystem.
        let out = poller.consumption(&config, &[], now(), false);
        assert_eq!(out, cached);
    }
}
