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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gwt_agent::{AgentId, AgentStatus};
use gwt_config::{usage_config::UsageConfig, Settings};
use gwt_core::usage::{
    cache::UsageCache,
    claude, codex,
    state::{age_secs, apply_staleness, should_fetch_claude, DEFAULT_STALE_AFTER_SECS},
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

/// Called when the account rows meaningfully change, alongside the client broadcast.
///
/// Issue #3616: the quota classifier lives in `AppRuntime`, which only the tao
/// event loop can touch, so the caller supplies the hop. Taking a callback
/// rather than an `EventLoopProxy` keeps this module free of the event enum.
pub type AccountsObserver = Arc<dyn Fn(Vec<ProviderUsage>) + Send + Sync>;

/// Spawn the usage poller onto the shared tokio runtime.
pub fn spawn_usage_poller(
    runtime: &tokio::runtime::Runtime,
    clients: ClientHub,
    refresh: Arc<Notify>,
    project_roots: Arc<RwLock<Vec<PathBuf>>>,
    observe_accounts: AccountsObserver,
) {
    drop(
        runtime
            .handle()
            .spawn(run(clients, refresh, project_roots, observe_accounts)),
    );
}

async fn run(
    clients: ClientHub,
    refresh: Arc<Notify>,
    project_roots: Arc<RwLock<Vec<PathBuf>>>,
    observe_accounts: AccountsObserver,
) {
    let mut poller = Poller::default();
    let mut observed_accounts = None;
    let mut ticker = interval(Duration::from_secs(TICK_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        // A refresh request immediately rereads local authentication and usage. Claude
        // automatic HTTP requests retain the 180s cadence floor.
        let forced = tokio::select! {
            _ = ticker.tick() => false,
            _ = refresh.notified() => true,
        };
        // FR-007: do nothing (no network, no credential access) when the GUI
        // is closed.
        if !clients.has_clients() {
            continue;
        }
        let now = Utc::now();
        let roots = project_roots
            .read()
            .map(|roots| roots.clone())
            .unwrap_or_default();
        let snapshot = match poller.poll_in_background(now, forced, roots).await {
            Ok((returned, snapshot)) => {
                poller = returned;
                snapshot
            }
            Err(error) => {
                tracing::warn!(%error, "usage collection worker failed; retrying next tick");
                poller = Poller::default();
                continue;
            }
        };
        // Issue #5597: only meaningful changes need a GUI-thread update.
        // The existing terminal-convergence clock still sweeps pending quota
        // candidates even when the account observation remains unchanged.
        if account_usage_changed(&mut observed_accounts, &snapshot.accounts) {
            observe_accounts(snapshot.accounts.clone());
        }
        clients.dispatch(vec![OutboundEvent::broadcast(
            BackendEvent::ProviderUsage {
                accounts: snapshot.accounts,
                sessions: snapshot.sessions,
                consumption: snapshot.consumption,
            },
        )]);
    }
}

/// Poll timestamps and the age of an already-stale reading do not alter quota
/// decisions. Preserve state transitions, account identity and every usage
/// window (including its reset) when deciding whether the GUI needs a notice.
fn account_usage_changed(
    previous: &mut Option<Vec<ProviderUsage>>,
    accounts: &[ProviderUsage],
) -> bool {
    let current = accounts
        .iter()
        .cloned()
        .map(|mut account| {
            account.fetched_at = None;
            if let UsageState::Stale { age_secs } = &mut account.state {
                *age_secs = 0;
            }
            account
        })
        .collect::<Vec<_>>();
    if previous.as_ref() == Some(&current) {
        return false;
    }
    *previous = Some(current);
    true
}

/// Account recovery requires repository discovery and a durable prefs
/// transaction. Keep both on the collection worker, including unchanged ticks
/// so newly opened projects and failed writes still receive the observation.
fn reconcile_project_accounts(
    project_roots: &[PathBuf],
    accounts: &[ProviderUsage],
    now: DateTime<Utc>,
) {
    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let prefs_paths = project_roots
        .iter()
        .map(|root| gwt::issue_monitor_prefs_path_for_repo_path(root))
        .collect::<HashSet<_>>();
    for prefs_path in prefs_paths {
        #[cfg(test)]
        let timeout = crate::app_runtime::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;
        #[cfg(not(test))]
        let timeout = crate::app_runtime::ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + timeout,
        );
        if let Err(error) =
            gwt::issue_monitor::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                let mut monitor = gwt::IssueMonitorState::with_prefs(
                    gwt::IssueMonitorConfig::default(),
                    prefs.clone(),
                );
                let mut changed = false;
                for account in accounts {
                    changed |= monitor.reconcile_provider_usage(account, &timestamp);
                }
                if changed {
                    *prefs = monitor.prefs();
                }
                Ok(())
            })
        {
            tracing::warn!(%error, "provider usage recovery could not be persisted; next poll retries");
        }
    }
}

/// Issue #4908 AC-5: publish this tick's account rows where
/// `issue.monitor.status` reads them. The status bar was the only place these
/// readings went, so a JSON reader could not see the usage a hold sat beside.
/// A failed write costs that reader one tick, never the poll itself.
fn publish_accounts(accounts: &[ProviderUsage], now: DateTime<Utc>) {
    let path = gwt_core::usage::snapshot_store::provider_usage_snapshot_path();
    if let Err(error) =
        gwt_core::usage::snapshot_store::write_provider_usage_snapshot(&path, accounts, now)
    {
        tracing::warn!(
            %error,
            path = %path.display(),
            "provider usage snapshot could not be published; issue.monitor.status keeps the previous reading"
        );
    }
}

/// Mutable state carried across polls (Claude cache + cadence + UA +
/// consumption cache).
#[derive(Default)]
struct Poller {
    #[cfg(test)]
    collection_thread: Option<std::thread::ThreadId>,
    transcripts: UsageCache,
    codex_account_id: Option<String>,
    codex_auth_observed: bool,
    codex_authenticated_at: Option<DateTime<Utc>>,
    claude_account_id: Option<String>,
    cached_claude: Option<ProviderUsage>,
    last_claude_fetch: Option<DateTime<Utc>>,
    cached_user_agent: Option<String>,
    cached_consumption: Vec<ProviderConsumption>,
    last_consumption_at: Option<DateTime<Utc>>,
}

impl Poller {
    /// Keep all synchronous work (settings/auth, directory scans, parsing and
    /// publication/account recovery) off async workers and the GUI thread.
    /// HTTP futures still use the shared runtime.
    async fn poll_in_background(
        mut self,
        now: DateTime<Utc>,
        force: bool,
        project_roots: Vec<PathBuf>,
    ) -> Result<(Self, UsageSnapshot), tokio::task::JoinError> {
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let snapshot = runtime.block_on(self.poll_once(now, force));
            publish_accounts(&snapshot.accounts, now);
            reconcile_project_accounts(&project_roots, &snapshot.accounts, now);
            (self, snapshot)
        })
        .await
    }

    async fn poll_once(&mut self, now: DateTime<Utc>, force: bool) -> UsageSnapshot {
        #[cfg(test)]
        {
            self.collection_thread = Some(std::thread::current().id());
        }
        self.transcripts.begin_poll();
        let config = Settings::load().unwrap_or_default().usage;
        let accounts = vec![
            self.codex_account(&config, now),
            self.claude_account(&config, now, force).await,
        ];
        let consumption = self.consumption(&config, &accounts, now, force);
        let sessions = collect_sessions(&config, &mut self.transcripts);
        self.transcripts.end_poll();
        UsageSnapshot {
            accounts,
            sessions,
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
                out.push(self.transcripts.codex_consumption(&home, week_start, now));
            }
        }
        // Claude consumption is read from local transcripts only, so it does not
        // require the account opt-in (mirrors Claude per-session usage).
        if let Some(home) = claude::claude_home() {
            let week_start = week_start_for(accounts, UsageProvider::ClaudeCode, now);
            out.push(self.transcripts.claude_consumption(&home, week_start, now));
        }
        self.cached_consumption = out.clone();
        self.last_consumption_at = Some(now);
        out
    }

    fn codex_account(&mut self, config: &UsageConfig, now: DateTime<Utc>) -> ProviderUsage {
        if !config.codex_enabled {
            return ProviderUsage::degraded(UsageProvider::Codex, UsageState::Disabled);
        }
        let Some(home) = codex::codex_home() else {
            return ProviderUsage::degraded(UsageProvider::Codex, UsageState::NoData);
        };
        let identity = codex::read_auth_account_identity(&home);
        if !self.codex_auth_observed || self.codex_account_id != identity {
            self.codex_authenticated_at = if self.codex_auth_observed {
                Some(now)
            } else {
                codex::auth_modified_at(&home)
            };
            self.codex_account_id = identity;
            self.codex_auth_observed = true;
            self.last_consumption_at = None;
        }
        let account = self
            .transcripts
            .codex_account_since(&home, now, self.codex_authenticated_at);
        if account.account_id != self.codex_account_id {
            // The auth file changed during collection. Publish identity only;
            // the next tick will establish the new observation boundary.
            return ProviderUsage {
                account_id: account.account_id,
                ..ProviderUsage::degraded(UsageProvider::Codex, UsageState::NoData)
            };
        }
        account
    }

    fn observe_claude_identity(&mut self, identity: Option<String>) {
        if self.claude_account_id != identity {
            self.claude_account_id = identity;
            self.cached_claude = None;
            self.last_consumption_at = None;
        }
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

        let creds = claude::resolve_claude_creds(&home);
        self.observe_claude_identity(creds.as_ref().and_then(|creds| creds.account_id.clone()));
        let Some(creds) = creds else {
            return ProviderUsage::degraded(
                UsageProvider::ClaudeCode,
                UsageState::Unavailable {
                    reason: "no credentials".to_string(),
                },
            );
        };
        if force || should_fetch_claude(self.last_claude_fetch, now) {
            let user_agent = match self.cached_user_agent.clone() {
                Some(user_agent) => user_agent,
                None => match claude::claude_user_agent() {
                    Ok(user_agent) => {
                        self.cached_user_agent = Some(user_agent.clone());
                        user_agent
                    }
                    Err(error) => {
                        self.last_claude_fetch = Some(now);
                        return serve_stale_cached_or(
                            &self.cached_claude,
                            now,
                            ProviderUsage::degraded(
                                UsageProvider::ClaudeCode,
                                UsageState::Unavailable {
                                    reason: format!("Claude version probe unavailable: {error}"),
                                },
                            ),
                        );
                    }
                },
            };
            let fresh = claude::fetch_claude_account(&creds, &user_agent, now).await;
            self.last_claude_fetch = Some(now);
            let current_identity = claude::read_auth_account_identity(&home);
            if current_identity != creds.account_id {
                self.observe_claude_identity(current_identity);
                return ProviderUsage {
                    account_id: self.claude_account_id.clone(),
                    ..ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData)
                };
            }
            // Only a successful fetch (windows or credits) replaces the cache. A
            // transient failure (429 / auth / network) keeps the last good
            // value shown as stale instead of wiping the display.
            if !fresh.windows.is_empty() || fresh.extra_usage.is_some() {
                self.cached_claude = Some(fresh.clone());
                return fresh;
            }
            return serve_stale_cached_or(&self.cached_claude, now, fresh);
        }

        // Within the cooldown window: reuse the cached value with staleness.
        serve_cached_or(
            &self.cached_claude,
            now,
            ProviderUsage {
                account_id: self.claude_account_id.clone(),
                ..ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData)
            },
        )
    }
}

/// Serve the last good cached account (as stale) when present, else the given
/// fallback. Keeps a transient fetch failure (e.g. 429) from wiping a usable
/// display. "Good" = the cached value carries windows or credits.
fn serve_cached_or(
    cached: &Option<ProviderUsage>,
    now: DateTime<Utc>,
    fallback: ProviderUsage,
) -> ProviderUsage {
    match cached {
        Some(c) if !c.windows.is_empty() || c.extra_usage.is_some() => {
            let mut c = c.clone();
            c.state = apply_staleness(c.state, c.fetched_at, now, DEFAULT_STALE_AFTER_SECS);
            c
        }
        _ => fallback,
    }
}

/// Serve the last good account explicitly as stale after a failed refresh.
/// Unlike [`serve_cached_or`], this does not wait for the normal age threshold:
/// the attempted refresh already proved the displayed value is no longer fresh.
fn serve_stale_cached_or(
    cached: &Option<ProviderUsage>,
    now: DateTime<Utc>,
    fallback: ProviderUsage,
) -> ProviderUsage {
    match cached {
        Some(cached) if !cached.windows.is_empty() || cached.extra_usage.is_some() => {
            let mut cached = cached.clone();
            cached.state = UsageState::Stale {
                age_secs: cached
                    .fetched_at
                    .map(|fetched_at| age_secs(fetched_at, now))
                    .unwrap_or(0),
            };
            cached
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

fn collect_sessions(config: &UsageConfig, cache: &mut UsageCache) -> Vec<SessionUsage> {
    collect_sessions_with_clock(config, cache, Utc::now)
}

/// Read only conversations referenced by the gwt ledger. Historical rows keep
/// their Session/member identity for Today; only the current active conversation
/// participates in the existing per-agent table. Missing or ambiguous evidence
/// stays NoData instead of silently becoming a measured zero.
#[cfg(test)]
fn collect_sessions_at(
    config: &UsageConfig,
    cache: &mut UsageCache,
    now: DateTime<Utc>,
) -> Vec<SessionUsage> {
    collect_sessions_with_clock(config, cache, || now)
}

fn collect_sessions_with_clock(
    config: &UsageConfig,
    cache: &mut UsageCache,
    clock: impl FnOnce() -> DateTime<Utc>,
) -> Vec<SessionUsage> {
    #[derive(PartialEq, Eq)]
    enum ConversationOwner {
        Member(String),
        ProjectManager(PathBuf),
    }

    let mut out = Vec::new();
    let dir = gwt_core::paths::gwt_sessions_dir();
    let codex_home = codex::codex_home();
    let claude_home = claude::claude_home();
    let sessions = crate::session_ledger_cache::SessionLedgerCache::new().load_shared(&dir);
    let mut references = Vec::new();
    let mut owners: HashMap<(UsageProvider, String), Vec<Arc<gwt_agent::Session>>> = HashMap::new();
    for session in sessions {
        let Some(provider) = session_provider(&session.agent_id) else {
            continue;
        };
        if provider == UsageProvider::Codex && !config.codex_enabled {
            continue;
        }
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for id in session.agent_session_id.iter().map(String::as_str).chain(
            session
                .session_history
                .iter()
                .map(|entry| entry.agent_session_id.as_str()),
        ) {
            if !id.trim().is_empty() && seen.insert(id.to_owned()) {
                ids.push(id.to_owned());
            }
        }
        if ids.is_empty() && is_active_status(session.status) {
            ids.push(String::new());
        }
        for id in ids {
            owners
                .entry((provider, id.clone()))
                .or_default()
                .push(session.clone());
            references.push((session.clone(), provider, id));
        }
    }
    let confirmed_identity = |session: &gwt_agent::Session| {
        let project = dunce::canonicalize(
            session
                .project_state_root
                .as_deref()
                .unwrap_or(&session.worktree_path),
        )
        .ok()?;
        if let Some(member) = session
            .agent_member_id
            .as_ref()
            .filter(|id| !id.trim().is_empty())
        {
            return Some((ConversationOwner::Member(member.clone()), project));
        }
        // The resident PM has no roster member. Its canonical project-owned
        // container is the stable identity retained across restored Sessions.
        let worktree = dunce::canonicalize(&session.worktree_path).ok()?;
        let expected = gwt::pm_registry::pm_worktree_path_for_repo_path(&project);
        (gwt::pm_registry::is_canonical_pm_worktree(&worktree)
            && dunce::canonicalize(expected).ok().as_ref() == Some(&worktree))
        .then_some((ConversationOwner::ProjectManager(worktree), project))
    };
    let mut registered_pms = HashMap::<PathBuf, Option<String>>::new();
    for (session, provider, agent_sid) in references {
        let conversation_owners = owners
            .get(&(provider, agent_sid.clone()))
            .expect("every conversation reference has an owner");
        let identity = conversation_owners
            .first()
            .and_then(|owner| confirmed_identity(owner));
        let same_identity = identity.as_ref().is_some_and(|identity| {
            conversation_owners
                .iter()
                .all(|owner| confirmed_identity(owner).as_ref() == Some(identity))
        });
        let ambiguous = conversation_owners.len() > 1 && !same_identity;
        if !ambiguous && conversation_owners.len() > 1 {
            // A restore can retain the same provider conversation across gwt
            // Sessions. Prefer the registered PM, otherwise the newest exact
            // project/member owner, and count the conversation only once.
            // Ledger status is a display hint, never a proof of physical liveness.
            let registered_pm = identity.as_ref().and_then(|(owner, project)| {
                let ConversationOwner::ProjectManager(worktree) = owner else {
                    return None;
                };
                registered_pms
                    .entry(project.clone())
                    .or_insert_with(|| {
                        let registration = gwt::pm_registry::load_pm_prefs(
                            &gwt::pm_registry::pm_prefs_path_for_repo_path(project),
                        )
                        .ok()?
                        .registration?;
                        (dunce::canonicalize(registration.worktree_path)
                            .ok()
                            .as_ref()
                            == Some(worktree))
                        .then_some(registration.session_id)
                    })
                    .as_deref()
            });
            let selected = registered_pm
                .and_then(|id| conversation_owners.iter().find(|owner| owner.id == id))
                .or_else(|| {
                    conversation_owners.iter().max_by(|a, b| {
                        (a.created_at, a.updated_at, &a.id).cmp(&(
                            b.created_at,
                            b.updated_at,
                            &b.id,
                        ))
                    })
                })
                .expect("a shared conversation has owners");
            if selected.id != session.id {
                continue;
            }
        }
        let usage_home = if ambiguous || agent_sid.is_empty() {
            None
        } else {
            match provider {
                UsageProvider::Codex => session
                    .codex_auth_root
                    .as_ref()
                    .map(|root| &root.path)
                    .or(codex_home.as_ref())
                    .cloned(),
                UsageProvider::ClaudeCode => claude_home.clone(),
            }
        };
        let usage = usage_home.as_deref().and_then(|home| match provider {
            UsageProvider::Codex => cache.codex_session(home, &agent_sid),
            UsageProvider::ClaudeCode => cache.claude_session(home, &agent_sid),
        });
        let telemetry_home = usage.as_ref().and(usage_home);
        let mut usage = usage.unwrap_or_else(|| match provider {
            UsageProvider::Codex => codex::parse_codex_session(&agent_sid, ""),
            UsageProvider::ClaudeCode => claude::parse_claude_transcript(&agent_sid, ""),
        });
        usage.session_id = session.id.clone();
        usage.provider_session_id = (!agent_sid.is_empty()).then_some(agent_sid.clone());
        usage.agent_member_id = session.agent_member_id.clone();
        usage.project_path = dunce::canonicalize(
            session
                .project_state_root
                .as_deref()
                .unwrap_or(&session.worktree_path),
        )
        .ok();
        usage.active = is_active_status(session.status)
            && (session.agent_session_id.as_deref() == Some(agent_sid.as_str())
                || agent_sid.is_empty());
        usage.eligible = session.backend_id.is_none();
        out.push((usage, telemetry_home));
    }
    // Every transcript is frozen before one common clock, including across a
    // minute rollover. Projection must not reread a provider file after this.
    let observed = clock();
    out.into_iter()
        .map(|(mut usage, home)| {
            if let Some(home) = home {
                cache.project_session_telemetry_at(&home, &mut usage, observed);
            }
            usage
        })
        .collect()
}

#[cfg(test)]
#[path = "usage_poller_performance_tests.rs"]
mod performance_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_sessions_freezes_reads_before_one_observation_clock() {
        use std::io::Write;

        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let codex_home = home.path().join("codex");
        let claude_home = home.path().join("claude");
        let _codex = gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", &codex_home);
        let _claude = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &claude_home);
        let observed = DateTime::parse_from_rfc3339("2026-10-28T00:30:01Z")
            .unwrap()
            .to_utc();
        let event = |provider, at: DateTime<Utc>| {
            let value = match provider {
                UsageProvider::Codex => serde_json::json!({"timestamp": at, "payload": {
                "type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": at.timestamp(), "output_tokens": 2},
                    "last_token_usage": {"input_tokens": 10, "cached_input_tokens": 0, "output_tokens": 2}
                }}}),
                UsageProvider::ClaudeCode => {
                    serde_json::json!({"timestamp": at, "type": "assistant",
                    "message": {"usage": {"input_tokens": 10, "output_tokens": 2,
                        "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}})
                }
            };
            value.to_string() + "\n"
        };
        let paths = [
            (
                UsageProvider::Codex,
                AgentId::Codex,
                codex_home.join("sessions/rollout-clock-codex.jsonl"),
            ),
            (
                UsageProvider::ClaudeCode,
                AgentId::ClaudeCode,
                claude_home.join("projects/project/clock-claude.jsonl"),
            ),
        ];
        for (provider, agent, path) in &paths {
            let sid = if *provider == UsageProvider::Codex {
                "clock-codex"
            } else {
                "clock-claude"
            };
            let mut session = gwt_agent::Session::new(home.path(), "work/clock", agent.clone());
            session.agent_session_id = Some(sid.to_owned());
            session.agent_member_id = Some(sid.to_owned());
            session.status = AgentStatus::Running;
            session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                event(*provider, observed - chrono::Duration::seconds(1)),
            )
            .unwrap();
        }
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: false,
        };
        let mut cache = UsageCache::default();
        let rows = collect_sessions_with_clock(&config, &mut cache, || {
            // This append occurs after observation ends and belongs to the next poll.
            for (provider, _, path) in &paths {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .unwrap()
                    .write_all(event(*provider, observed + chrono::Duration::minutes(1)).as_bytes())
                    .unwrap();
            }
            observed
        });
        assert_eq!(rows.len(), 2);
        for row in rows {
            let telemetry = row.telemetry.unwrap();
            assert_eq!(telemetry.quality, "measured");
            assert_eq!(telemetry.today_tokens, Some(12));
            assert_eq!(
                telemetry.minute_buckets.last().unwrap().started_at,
                observed - chrono::Duration::seconds(1)
            );
            assert_eq!(telemetry.minute_buckets.last().unwrap().tokens, Some(12));
        }
        // An event beyond the real observation end remains untrusted, then becomes measured.
        let future = collect_sessions_at(&config, &mut cache, observed);
        assert!(future
            .iter()
            .all(|row| row.telemetry.as_ref().unwrap().quality == "partial"));
        let next = collect_sessions_at(
            &config,
            &mut cache,
            observed + chrono::Duration::seconds(61),
        );
        assert!(next
            .iter()
            .all(|row| row.telemetry.as_ref().unwrap().today_tokens == Some(24)));
    }

    #[test]
    fn collect_sessions_preserves_history_and_refuses_ambiguous_conversations() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let codex_home = home.path().join("codex");
        let _codex = gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", &codex_home);
        std::fs::create_dir_all(codex_home.join("sessions")).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let make_session = |sid: &str, member: &str, status| {
            let mut session = gwt_agent::Session::new(home.path(), "work/usage", AgentId::Codex);
            session.agent_session_id = Some(sid.to_owned());
            session.agent_member_id = Some(member.to_owned());
            session.project_state_root = Some(home.path().to_owned());
            session.status = status;
            session
        };
        let mut current = make_session("current", "member-a", AgentStatus::Running);
        for sid in ["current", "history", "ambiguous"] {
            current
                .session_history
                .push(gwt_agent::session::AgentSessionHistoryEntry {
                    agent_session_id: sid.to_owned(),
                    started_at: now(),
                });
        }
        current.save(&sessions_dir).unwrap();
        let mut predecessor = make_session("current", "member-a", AgentStatus::Stopped);
        predecessor.created_at = current.created_at - chrono::Duration::seconds(1);
        predecessor.save(&sessions_dir).unwrap();
        for session in [
            make_session("ambiguous", "member-b", AgentStatus::Stopped),
            make_session("stopped", "member-a", AgentStatus::Stopped),
            make_session("missing", "member-a", AgentStatus::Stopped),
        ] {
            session.save(&sessions_dir).unwrap();
        }
        for sid in ["current", "history", "ambiguous", "stopped", "unowned"] {
            let line = serde_json::json!({"timestamp": now().to_rfc3339(), "payload": {
                "type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 10, "output_tokens": 2},
                    "last_token_usage": {"input_tokens": 10, "output_tokens": 2}
                }
            }});
            std::fs::write(
                codex_home.join(format!("sessions/rollout-{sid}.jsonl")),
                line.to_string(),
            )
            .unwrap();
        }
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: false,
        };
        let rows = collect_sessions(&config, &mut UsageCache::default());
        assert_eq!(
            rows.len(),
            6,
            "one row per certain conversation reference, including missing evidence"
        );
        let current_row = rows
            .iter()
            .find(|row| row.provider_session_id.as_deref() == Some("current"))
            .unwrap();
        assert!(current_row.active);
        assert_eq!(current_row.session_id, current.id);
        assert!(current_row.telemetry.is_some());
        assert_eq!(
            rows.iter()
                .filter(|row| row.provider_session_id.as_deref() == Some("current"))
                .count(),
            1,
            "a resumed member's conversation is measured exactly once on its newest Session"
        );
        assert_eq!(
            current_row.project_path,
            Some(
                crate::runtime_support::resolve_project_target(home.path())
                    .unwrap()
                    .project_root
            ),
            "usage paths must use the same canonical spelling as the project wire"
        );
        assert_eq!(current_row.agent_member_id.as_deref(), Some("member-a"));
        for sid in ["history", "stopped"] {
            let row = rows
                .iter()
                .find(|row| row.provider_session_id.as_deref() == Some(sid))
                .unwrap();
            assert!(!row.active);
            assert!(row.telemetry.is_some());
        }
        let ambiguous = rows
            .iter()
            .filter(|row| row.provider_session_id.as_deref() == Some("ambiguous"))
            .collect::<Vec<_>>();
        assert_eq!(ambiguous.len(), 2);
        assert!(ambiguous
            .iter()
            .all(|row| row.telemetry.is_none() && row.state == UsageState::NoData));
        let missing = rows
            .iter()
            .find(|row| row.provider_session_id.as_deref() == Some("missing"))
            .unwrap();
        assert_eq!(missing.state, UsageState::NoData);
        assert!(missing.telemetry.is_none());
    }

    #[test]
    fn collect_sessions_attributes_restored_pm_usage_to_registered_session() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let claude_home = home.path().join("claude");
        let _claude = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &claude_home);
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&project);
        std::fs::create_dir_all(&worktree).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let make_session = |path: &std::path::Path, sid: &str, status, age| {
            let mut session = gwt_agent::Session::new(path, "work", AgentId::ClaudeCode);
            session.project_state_root = Some(project.clone());
            session.agent_session_id = Some(sid.to_owned());
            session.status = status;
            session.created_at = now() + chrono::Duration::seconds(age);
            session.updated_at = session.created_at;
            session
        };
        let current = make_session(&worktree, "pm-conversation", AgentStatus::Running, 0);
        for session in [
            current.clone(),
            make_session(&worktree, "pm-conversation", AgentStatus::Stopped, -1),
            make_session(&worktree, "pm-conversation", AgentStatus::Stopped, 1),
            make_session(&project, "ordinary-conversation", AgentStatus::Running, 0),
            make_session(&project, "ordinary-conversation", AgentStatus::Stopped, -1),
        ] {
            session.save(&sessions_dir).unwrap();
        }
        gwt::pm_registry::save_pm_prefs(
            &gwt::pm_registry::pm_prefs_path_for_repo_path(&project),
            &gwt::pm_registry::PmPrefs {
                registration: Some(gwt::pm_registry::PmRegistration {
                    session_id: current.id.clone(),
                    agent_id: "claude".into(),
                    worktree_path: worktree.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let transcripts = claude_home.join("projects/project");
        std::fs::create_dir_all(&transcripts).unwrap();
        let line = serde_json::json!({"type": "assistant", "timestamp": now().to_rfc3339(),
        "message": {"model": "claude-sonnet-4-5", "usage": {
            "input_tokens": 80, "output_tokens": 12,
            "cache_read_input_tokens": 40, "cache_creation_input_tokens": 5
        }}});
        for sid in ["pm-conversation", "ordinary-conversation"] {
            std::fs::write(transcripts.join(format!("{sid}.jsonl")), line.to_string()).unwrap();
        }
        let config = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: false,
        };
        let rows = collect_sessions_at(&config, &mut UsageCache::default(), now());
        let pm = rows
            .iter()
            .filter(|row| row.provider_session_id.as_deref() == Some("pm-conversation"))
            .collect::<Vec<_>>();
        assert_eq!(
            pm.len(),
            1,
            "restored PM Sessions share one canonical PM identity"
        );
        assert_eq!(
            pm[0].session_id, current.id,
            "the registered PM wins over newer stopped history"
        );
        assert!(pm[0].active);
        assert_eq!(pm[0].state, UsageState::Ok);
        assert_eq!(pm[0].model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!((pm[0].input_tokens, pm[0].output_tokens), (80, 12));
        assert_eq!(
            (pm[0].cache_read_tokens, pm[0].cache_creation_tokens),
            (Some(40), Some(5))
        );
        assert!(pm[0]
            .telemetry
            .as_ref()
            .unwrap()
            .minute_buckets
            .iter()
            .any(|bucket| bucket.tokens == Some(137)));
        let ordinary = rows
            .iter()
            .filter(|row| row.provider_session_id.as_deref() == Some("ordinary-conversation"))
            .collect::<Vec<_>>();
        assert_eq!(ordinary.len(), 2);
        assert!(
            ordinary
                .iter()
                .all(|row| row.state == UsageState::NoData && row.telemetry.is_none()),
            "memberless ordinary work must remain ambiguous"
        );

        let foreign_project = home.path().join("foreign");
        std::fs::create_dir_all(&foreign_project).unwrap();
        let foreign_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&foreign_project);
        std::fs::create_dir_all(&foreign_worktree).unwrap();
        let mut foreign = make_session(
            &foreign_worktree,
            "pm-conversation",
            AgentStatus::Stopped,
            -2,
        );
        foreign.project_state_root = Some(foreign_project);
        foreign.save(&sessions_dir).unwrap();
        let rows = collect_sessions_at(&config, &mut UsageCache::default(), now());
        assert!(
            rows.iter()
                .filter(|row| row.provider_session_id.as_deref() == Some("pm-conversation"))
                .all(|row| row.state == UsageState::NoData && row.telemetry.is_none()),
            "a different project's PM must not inherit the conversation"
        );
    }

    #[test]
    fn account_notifications_ignore_poll_time_but_preserve_usage_changes() {
        let mut previous = None;
        let mut account = account_with_weekly_reset(UsageProvider::Codex, Some(now()));
        assert!(account_usage_changed(
            &mut previous,
            std::slice::from_ref(&account)
        ));
        account.fetched_at = Some(now() + chrono::Duration::seconds(30));
        assert!(!account_usage_changed(
            &mut previous,
            std::slice::from_ref(&account)
        ));
        account.windows[0].used_percent = 100.0;
        account.limit_reached = true;
        assert!(account_usage_changed(
            &mut previous,
            std::slice::from_ref(&account)
        ));
        account.state = UsageState::Stale { age_secs: 300 };
        assert!(account_usage_changed(
            &mut previous,
            std::slice::from_ref(&account)
        ));
        account.state = UsageState::Stale { age_secs: 330 };
        assert!(!account_usage_changed(
            &mut previous,
            std::slice::from_ref(&account)
        ));
        account.account_id = Some("replacement".into());
        assert!(account_usage_changed(
            &mut previous,
            std::slice::from_ref(&account)
        ));
        account.windows[0].resets_at = Some(now() + chrono::Duration::days(7));
        assert!(account_usage_changed(&mut previous, &[account]));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // isolated env must survive the worker's reads
    async fn complete_poll_collects_on_a_blocking_thread_and_keeps_its_cache() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        // The collector crosses threads, so isolate the inherited environment.
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let _codex =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", home.path().join("codex"));
        let _claude = gwt_core::test_support::ScopedEnvVar::set(
            "CLAUDE_CONFIG_DIR",
            home.path().join("claude"),
        );
        let codex_home = home.path().join("codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        std::fs::write(
            codex_home.join("auth.json"),
            r#"{"tokens":{"account_id":"account-a"}}"#,
        )
        .unwrap();
        let project = home.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let calling_thread = std::thread::current().id();
        let (poller, first) = Poller::default()
            .poll_in_background(now(), false, vec![project.clone()])
            .await
            .unwrap();
        assert_ne!(poller.collection_thread, Some(calling_thread));
        assert_eq!(first.accounts.len(), 2);
        assert!(first.sessions.is_empty());
        assert_eq!(first.accounts[1].state, UsageState::Disabled);
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
        let prefs = gwt::issue_monitor::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert_eq!(
            prefs.provider_quota_accounts["codex"].account_id,
            first.accounts[0].account_id.as_deref().unwrap(),
            "the collection worker must persist the same account recovery observation"
        );
        let (poller, second) = poller
            .poll_in_background(now(), false, vec![project])
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(poller.last_consumption_at, Some(now()));
        assert_eq!(poller.transcripts.bytes_read(), 0);
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap()
    }

    #[test]
    fn credits_only_cache_survives_cooldown_and_failed_refresh() {
        let mut good = ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::Ok);
        good.fetched_at = Some(now());
        good.extra_usage = Some(
            serde_json::from_value(serde_json::json!({
                "is_enabled":true, "monthly_limit":100, "used_credits":25,
                "utilization":25, "currency":"USD"
            }))
            .unwrap(),
        );
        let fail = ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData);
        assert_eq!(
            serve_cached_or(&Some(good.clone()), now(), fail.clone()),
            good
        );
        let stale = serve_stale_cached_or(&Some(good.clone()), now(), fail);
        assert_eq!(stale.extra_usage, good.extra_usage);
        assert_eq!(stale.state, UsageState::Stale { age_secs: 0 });
    }

    #[test]
    fn serve_cached_keeps_last_good_on_failure() {
        let when = now();
        let good = ProviderUsage {
            provider: UsageProvider::ClaudeCode,
            account_id: None,
            account_label: Some("claude@example.com".into()),
            plan: Some("max".into()),
            extra_usage: None,
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
        let served = serve_stale_cached_or(&Some(good), when, fail.clone());
        assert!(!served.windows.is_empty());
        assert_eq!(served.account_label.as_deref(), Some("claude@example.com"));
        assert_eq!(served.plan.as_deref(), Some("max"));
        assert_eq!(served.state, UsageState::Stale { age_secs: 0 });
        // No cache → fallback is returned.
        let served2 = serve_stale_cached_or(&None, when, fail);
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
    fn repeated_collection_skips_preferences_and_caches_bad_sessions() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tracing_subscriber::prelude::*;

        struct Warnings(Arc<AtomicUsize>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if *event.metadata().level() == tracing::Level::WARN
                    && event.metadata().target().starts_with("gwt_agent::session")
                {
                    self.0.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("display.toml"), "display_mode = 'grid'").unwrap();
        std::fs::write(sessions.join("broken.toml"), "id = [").unwrap();
        gwt_agent::Session::new(home.path(), "work/valid", AgentId::Codex)
            .save(&sessions)
            .unwrap();
        let warnings = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(Warnings(warnings.clone()));
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..2 {
                collect_sessions(
                    &UsageConfig {
                        codex_enabled: false,
                        claude_account_enabled: false,
                    },
                    &mut UsageCache::default(),
                );
            }
        });
        assert_eq!(
            warnings.load(Ordering::Relaxed),
            1,
            "only the first bad Session read warns"
        );
    }

    #[test]
    fn codex_lost_credentials_discard_previous_account_usage() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _codex = gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", home.path());
        std::fs::create_dir(home.path().join("sessions")).unwrap();
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"tokens":{"account_id":"account-a"}}"#,
        )
        .unwrap();
        let event = serde_json::json!({"timestamp":now().to_rfc3339(), "type":"event_msg", "payload":{"type":"token_count", "rate_limits":{"primary":{"used_percent":100,"window_minutes":300,"resets_at":1780003600}}}});
        std::fs::write(
            home.path().join("sessions/rollout-fixture.jsonl"),
            event.to_string(),
        )
        .unwrap();
        let mut poller = Poller {
            codex_auth_observed: true,
            codex_account_id: codex::read_auth_account_identity(home.path()),
            codex_authenticated_at: Some(now() - chrono::Duration::seconds(1)),
            ..Default::default()
        };
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: false,
        };
        assert!(!poller.codex_account(&config, now()).windows.is_empty());
        std::fs::remove_file(home.path().join("auth.json")).unwrap();
        let lost = poller.codex_account(&config, now());
        assert!(lost.windows.is_empty());
        assert!(lost.account_id.is_none());
    }

    #[test]
    fn codex_disabled_yields_disabled_state() {
        let mut poller = Poller::default();
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

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn claude_switch_invalidates_cache_during_http_cooldown() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", home.path());
        std::fs::write(
            home.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"accountId":"new-account","accessToken":"fixture-token"}}"#,
        )
        .unwrap();
        let old = account_with_weekly_reset(UsageProvider::ClaudeCode, Some(now()));
        let mut poller = Poller {
            cached_claude: Some(old),
            last_claude_fetch: Some(now()),
            ..Default::default()
        };
        let config = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: true,
        };
        let account = poller.claude_account(&config, now(), false).await;
        assert!(
            account.windows.is_empty(),
            "a different account must never reuse old windows"
        );
        assert_eq!(
            poller.last_claude_fetch,
            Some(now()),
            "switch must respect the HTTP floor"
        );
        assert!(poller.cached_claude.is_none());

        let mut current = account_with_weekly_reset(UsageProvider::ClaudeCode, Some(now()));
        current.account_id = account.account_id.clone();
        poller.cached_claude = Some(current.clone());
        std::fs::write(
            home.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"accountId":"new-account","accessToken":"refreshed-token"}}"#,
        )
        .unwrap();
        assert_eq!(poller.claude_account(&config, now(), false).await, current);

        std::fs::remove_file(home.path().join(".credentials.json")).unwrap();
        let lost = poller.claude_account(&config, now(), false).await;
        assert!(lost.windows.is_empty());
        assert!(poller.cached_claude.is_none());
        assert!(lost.account_id.is_none());
    }

    #[cfg(windows)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env lock must span the awaited fixture read
    async fn usage_poller_skips_http_for_real_bun_global_placeholder_fixture_without_safe_target() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");
        fixture
            .remove_safe_targets()
            .expect("remove safe redirect targets");
        let claude_home = temp.path().join("claude-home");
        std::fs::create_dir_all(&claude_home).expect("create Claude home");
        std::fs::write(
            claude_home.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"must-not-be-sent"}}"#,
        )
        .expect("write credentials");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);
        let _claude_home =
            gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &claude_home);
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: true,
        };
        let mut poller = Poller::default();

        let account = poller.claude_account(&config, now(), false).await;

        assert!(matches!(
            account.state,
            UsageState::Unavailable { ref reason }
                if reason.contains("Claude version probe unavailable")
                    && reason.contains("native-binary placeholder")
        ));
        assert!(poller.last_claude_fetch.is_some());
        assert!(poller.cached_user_agent.is_none());
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
            account_id: None,
            account_label: None,
            plan: None,
            extra_usage: None,
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
