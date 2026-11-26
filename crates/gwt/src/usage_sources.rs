//! Built-in usage sources (SPEC #7347 S2).
//!
//! The default account rows are produced through a
//! [`UsageSourceRegistry`]. The Codex and Claude Code sources stay stateless:
//! their identity, cache and HTTP cadence keep living in the [`Poller`]
//! (shared with consumption and session collection), and each source reads
//! through the poll context, so the values are the ones the poller produced
//! before the registry existed (FR-005).

use gwt_agent::AgentId;
use gwt_config::usage_config::UsageConfig;
use gwt_core::timestamp::UtcTimestamp;
use gwt_core::usage::{
    UnknownUsageSource, UsageAcquisition, UsagePoll, UsageProvider, UsageSource,
    UsageSourceDescriptor, UsageSourceRegistry,
};

use super::Poller;

/// What one poll gives the built-in sources: the poller's shared state and
/// this tick's inputs.
pub(super) struct PollContext<'p> {
    pub poller: &'p mut Poller,
    pub config: &'p UsageConfig,
    pub now: UtcTimestamp,
    pub force: bool,
}

/// The usage provider of a built-in agent. `None` for custom agents.
pub(super) fn usage_provider(agent: &AgentId) -> Option<UsageProvider> {
    Some(match agent {
        AgentId::ClaudeCode => UsageProvider::ClaudeCode,
        AgentId::Codex => UsageProvider::Codex,
        AgentId::GrokBuild => UsageProvider::GrokBuild,
        AgentId::Antigravity => UsageProvider::Antigravity,
        AgentId::OpenCode => UsageProvider::OpenCode,
        AgentId::OpenClaw => UsageProvider::OpenClaw,
        AgentId::Hermes => UsageProvider::Hermes,
        AgentId::Copilot => UsageProvider::Copilot,
        AgentId::Custom(_) => return None,
    })
}

/// Two-letter label of a provider, for compact displays.
fn short_label(provider: UsageProvider) -> &'static str {
    match provider {
        UsageProvider::ClaudeCode => "CC",
        UsageProvider::Codex => "CX",
        UsageProvider::GrokBuild => "GB",
        UsageProvider::Antigravity => "AG",
        UsageProvider::OpenCode => "OC",
        UsageProvider::OpenClaw => "OW",
        UsageProvider::Hermes => "HA",
        UsageProvider::Copilot => "GH",
    }
}

/// The default Codex home's rate limits, from its local rollout files.
struct CodexUsageSource(UsageSourceDescriptor);

impl<'p> UsageSource<PollContext<'p>> for CodexUsageSource {
    fn descriptor(&self) -> &UsageSourceDescriptor {
        &self.0
    }

    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'p>) -> UsagePoll<'a> {
        Box::pin(async move { ctx.poller.codex_account(ctx.config, ctx.now) })
    }
}

/// The default Claude home's usage, from the opt-in OAuth usage API.
struct ClaudeUsageSource(UsageSourceDescriptor);

impl<'p> UsageSource<PollContext<'p>> for ClaudeUsageSource {
    fn descriptor(&self) -> &UsageSourceDescriptor {
        &self.0
    }

    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'p>) -> UsagePoll<'a> {
        Box::pin(async move {
            ctx.poller
                .claude_account(ctx.config, ctx.now, ctx.force)
                .await
        })
    }
}

/// The default-account sources: one per built-in agent, in the agents'
/// order (FR-002). Providers gwt has no usage source for report
/// [`UsageState::Unknown`](gwt_core::usage::UsageState::Unknown) (FR-003).
pub(super) fn default_account_sources<'p>() -> UsageSourceRegistry<PollContext<'p>> {
    let mut registry = UsageSourceRegistry::new();
    for agent in gwt_agent::builtin_agent_descriptors() {
        let Some(provider) = usage_provider(&agent.id) else {
            continue;
        };
        let label = short_label(provider);
        let descriptor = |acquisition| UsageSourceDescriptor {
            provider,
            display_name: agent.display_name.to_string(),
            short_label: label.to_string(),
            acquisition,
        };
        let source: Box<dyn UsageSource<PollContext<'p>>> = match provider {
            UsageProvider::Codex => {
                Box::new(CodexUsageSource(descriptor(UsageAcquisition::CliOutput)))
            }
            UsageProvider::ClaudeCode => {
                Box::new(ClaudeUsageSource(descriptor(UsageAcquisition::Api)))
            }
            _ => Box::new(UnknownUsageSource::new(provider, agent.display_name, label)),
        };
        registry.register(source);
    }
    registry
}
#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::usage::{ProviderUsage, UsageState};

    fn now() -> UtcTimestamp {
        UtcTimestamp::from_timestamp(1_780_000_000, 0).unwrap()
    }

    async fn through_registry(poller: &mut Poller, config: &UsageConfig) -> Vec<ProviderUsage> {
        default_account_sources()
            .poll_all(&mut PollContext {
                poller,
                config,
                now: now(),
                force: false,
            })
            .await
    }

    // FR-002 / AC-1: one source per built-in agent, in the agents' order,
    // named as the agent is.
    #[test]
    fn default_sources_cover_every_builtin_agent_in_order() {
        let described: Vec<_> = default_account_sources()
            .descriptors()
            .into_iter()
            .map(|d| (d.provider, d.display_name, d.acquisition))
            .collect();
        let builtin: Vec<_> = gwt_agent::builtin_agent_descriptors()
            .iter()
            .map(|agent| agent.display_name.to_string())
            .collect();
        assert_eq!(
            described.iter().map(|d| d.1.clone()).collect::<Vec<_>>(),
            builtin
        );
        let acquisition = |provider| {
            described
                .iter()
                .find(|d| d.0 == provider)
                .map(|d| d.2)
                .unwrap()
        };
        assert_eq!(
            acquisition(UsageProvider::Codex),
            UsageAcquisition::CliOutput
        );
        assert_eq!(
            acquisition(UsageProvider::ClaudeCode),
            UsageAcquisition::Api
        );
        for provider in [
            UsageProvider::GrokBuild,
            UsageProvider::Antigravity,
            UsageProvider::OpenCode,
            UsageProvider::OpenClaw,
            UsageProvider::Hermes,
            UsageProvider::Copilot,
        ] {
            assert_eq!(
                acquisition(provider),
                UsageAcquisition::None,
                "{provider:?}"
            );
        }
    }

    // FR-003 / AC-2: a provider gwt cannot read usage for still gets a row,
    // saying the usage is unknown, with no window and no limit.
    #[tokio::test]
    async fn unreadable_builtin_providers_report_unknown_rows() {
        let config = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: false,
        };
        let rows = through_registry(&mut Poller::default(), &config).await;
        assert_eq!(rows.len(), gwt_agent::builtin_agent_descriptors().len());
        for row in rows.iter().filter(|row| {
            !matches!(
                row.provider,
                UsageProvider::Codex | UsageProvider::ClaudeCode
            )
        }) {
            assert!(
                matches!(&row.state, UsageState::Unknown { reason } if !reason.is_empty()),
                "{row:?}"
            );
            assert!(row.windows.is_empty());
            assert!(!row.limit_reached);
        }
    }

    #[tokio::test]
    async fn registry_rows_match_the_direct_account_reads_when_disabled() {
        let config = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: false,
        };
        let mut direct = Poller::default();
        let expected = vec![
            direct.claude_account(&config, now(), false).await,
            direct.codex_account(&config, now()),
        ];

        let rows = through_registry(&mut Poller::default(), &config).await;
        let read: Vec<_> = rows
            .into_iter()
            .filter(|row| {
                matches!(
                    row.provider,
                    UsageProvider::Codex | UsageProvider::ClaudeCode
                )
            })
            .collect();

        assert_eq!(read, expected);
    }

    // FR-005 / AC-4: a Codex home with a rollout yields the same row, window
    // for window, through the registry as through `codex_account`.
    #[tokio::test]
    async fn registry_codex_row_matches_the_direct_read_for_a_rollout_fixture() {
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
        let event = serde_json::json!({"timestamp":now().to_rfc3339(), "type":"event_msg", "payload":{"type":"token_count", "rate_limits":{"primary":{"used_percent":42,"window_minutes":300,"resets_at":1780003600},"secondary":{"used_percent":7,"window_minutes":10080,"resets_at":1780500000}}}});
        std::fs::write(
            home.path().join("sessions/rollout-fixture.jsonl"),
            event.to_string(),
        )
        .unwrap();
        let config = UsageConfig {
            codex_enabled: true,
            claude_account_enabled: false,
        };

        // Auth already observed before the rollout, as in a running poller.
        let observed = || Poller {
            primary: super::super::AccountState {
                codex_auth_observed: true,
                codex_account_id: gwt_core::usage::codex::read_auth_account_identity(home.path()),
                codex_authenticated_at: Some(now() - time::Duration::seconds(1)),
                ..Default::default()
            },
            ..Default::default()
        };

        let expected = observed().codex_account(&config, now());
        let rows = through_registry(&mut observed(), &config).await;

        assert_eq!(expected.windows.len(), 2, "{expected:?}");
        let codex = rows.iter().find(|row| row.provider == UsageProvider::Codex);
        assert_eq!(codex, Some(&expected));
    }
}
