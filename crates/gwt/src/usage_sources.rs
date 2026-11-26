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
    UsageAcquisition, UsagePoll, UsageProvider, UsageSource, UsageSourceDescriptor,
    UsageSourceRegistry,
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

fn descriptor(
    agent: AgentId,
    provider: UsageProvider,
    short_label: &str,
    acquisition: UsageAcquisition,
) -> UsageSourceDescriptor {
    UsageSourceDescriptor {
        provider,
        display_name: agent.display_name().to_string(),
        short_label: short_label.to_string(),
        acquisition,
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

/// The default-account sources, in row order.
pub(super) fn default_account_sources<'p>() -> UsageSourceRegistry<PollContext<'p>> {
    let mut registry = UsageSourceRegistry::new();
    registry
        .register(Box::new(CodexUsageSource(descriptor(
            AgentId::Codex,
            UsageProvider::Codex,
            "CX",
            UsageAcquisition::CliOutput,
        ))))
        .register(Box::new(ClaudeUsageSource(descriptor(
            AgentId::ClaudeCode,
            UsageProvider::ClaudeCode,
            "CC",
            UsageAcquisition::Api,
        ))));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::usage::ProviderUsage;

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

    #[test]
    fn default_sources_are_codex_then_claude_code() {
        let descriptors = default_account_sources().descriptors();
        let described: Vec<_> = descriptors
            .iter()
            .map(|d| (d.provider, d.display_name.as_str(), d.acquisition))
            .collect();
        assert_eq!(
            described,
            [
                (UsageProvider::Codex, "Codex", UsageAcquisition::CliOutput),
                (
                    UsageProvider::ClaudeCode,
                    "Claude Code",
                    UsageAcquisition::Api
                ),
            ]
        );
    }

    #[tokio::test]
    async fn registry_rows_match_the_direct_account_reads_when_disabled() {
        let config = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: false,
        };
        let mut direct = Poller::default();
        let expected = vec![
            direct.codex_account(&config, now()),
            direct.claude_account(&config, now(), false).await,
        ];

        let rows = through_registry(&mut Poller::default(), &config).await;

        assert_eq!(rows, expected);
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
        assert_eq!(rows[0], expected);
    }
}
