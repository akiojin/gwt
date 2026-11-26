//! Usage source registry (SPEC #7347).
//!
//! A [`UsageSource`] reads the account usage of one provider. The
//! [`UsageSourceRegistry`] enumerates every registered source in order and
//! polls each one in isolation, so a provider that has no way to read usage,
//! or whose read fails, still yields exactly one row instead of hiding the
//! provider or breaking the others (FR-001 / FR-003 / FR-007).

use crate::timestamp::UtcTimestamp;
use crate::usage::types::{ProviderUsage, UsageProvider, UsageState};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{Context, Poll};

/// Reason on the row of a source that panicked while reading usage.
const SOURCE_PANICKED: &str = "usage reader failed";
/// Reason on the row of a provider gwt cannot read usage for.
const NO_USAGE_SOURCE: &str = "no official usage source";

/// How a provider's usage is obtained (FR-006 classification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageAcquisition {
    /// An official usage API.
    Api,
    /// Rate-limit data the provider's CLI writes locally.
    CliOutput,
    /// Token counts aggregated from local transcripts; limits are unknown.
    LocalTokens,
    /// No known way to read usage.
    None,
}

/// What a registered source describes: the provider and how to present it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSourceDescriptor {
    pub provider: UsageProvider,
    pub display_name: String,
    pub short_label: String,
    pub acquisition: UsageAcquisition,
}

/// Inputs shared by every source during one poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageContext {
    pub now: UtcTimestamp,
    /// Bypass the sources' own refresh cooldowns.
    pub force: bool,
}

/// The future a source returns from [`UsageSource::poll`].
pub type UsagePoll<'a> = Pin<Box<dyn Future<Output = ProviderUsage> + Send + 'a>>;

/// Reads the default account usage of one provider. A read that fails is
/// reported as a degraded row ([`ProviderUsage::degraded`]), never as an
/// error. The registry still isolates a source that panics.
pub trait UsageSource: Send {
    fn descriptor(&self) -> &UsageSourceDescriptor;
    fn poll<'a>(&'a mut self, ctx: &'a UsageContext) -> UsagePoll<'a>;
}

/// Ordered set of usage sources, one per provider.
#[derive(Default)]
pub struct UsageSourceRegistry {
    sources: Vec<Box<dyn UsageSource>>,
}

impl UsageSourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a source. Registration order is enumeration order.
    pub fn register(&mut self, source: Box<dyn UsageSource>) -> &mut Self {
        self.sources.push(source);
        self
    }

    /// The descriptors of every registered source, in registration order.
    pub fn descriptors(&self) -> Vec<UsageSourceDescriptor> {
        self.sources
            .iter()
            .map(|source| source.descriptor().clone())
            .collect()
    }

    /// Poll every source in registration order. Always returns exactly one
    /// row per source, keyed by the source's descriptor; a source that panics
    /// yields an `Unavailable` row and the remaining sources are still polled.
    pub async fn poll_all(&mut self, ctx: &UsageContext) -> Vec<ProviderUsage> {
        let mut rows = Vec::with_capacity(self.sources.len());
        for source in &mut self.sources {
            let provider = source.descriptor().provider;
            let polled = match catch_unwind(AssertUnwindSafe(move || {
                let source = source;
                source.poll(ctx)
            })) {
                Ok(poll) => CatchUnwind(poll).await,
                Err(panic) => Err(panic),
            };
            let mut row = polled.unwrap_or_else(|_| {
                ProviderUsage::degraded(
                    provider,
                    UsageState::Unavailable {
                        reason: SOURCE_PANICKED.to_string(),
                    },
                )
            });
            row.provider = provider;
            rows.push(row);
        }
        rows
    }
}

/// Source for a provider gwt has no way to read usage for. Its row says the
/// usage is unknown instead of showing zero usage (FR-003).
pub struct UnknownUsageSource {
    descriptor: UsageSourceDescriptor,
}

impl UnknownUsageSource {
    pub fn new(provider: UsageProvider, display_name: &str, short_label: &str) -> Self {
        Self {
            descriptor: UsageSourceDescriptor {
                provider,
                display_name: display_name.to_string(),
                short_label: short_label.to_string(),
                acquisition: UsageAcquisition::None,
            },
        }
    }
}

impl UsageSource for UnknownUsageSource {
    fn descriptor(&self) -> &UsageSourceDescriptor {
        &self.descriptor
    }

    fn poll<'a>(&'a mut self, _ctx: &'a UsageContext) -> UsagePoll<'a> {
        let provider = self.descriptor.provider;
        Box::pin(async move {
            ProviderUsage::degraded(
                provider,
                UsageState::Unknown {
                    reason: NO_USAGE_SOURCE.to_string(),
                },
            )
        })
    }
}

/// Resolves a future to `Err` instead of unwinding when it panics.
struct CatchUnwind<F>(F);

impl<F: Future + Unpin> Future for CatchUnwind<F> {
    type Output = std::thread::Result<F::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = &mut self.0;
        match catch_unwind(AssertUnwindSafe(|| Pin::new(&mut *inner).poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::types::{UsageWindow, WindowKind};

    fn ctx() -> UsageContext {
        UsageContext {
            now: UtcTimestamp::UNIX_EPOCH,
            force: false,
        }
    }

    fn descriptor(provider: UsageProvider, name: &str, label: &str) -> UsageSourceDescriptor {
        UsageSourceDescriptor {
            provider,
            display_name: name.to_string(),
            short_label: label.to_string(),
            acquisition: UsageAcquisition::CliOutput,
        }
    }

    /// Reports a fixed healthy row. Deliberately stamps the wrong provider
    /// so the registry has to key the row by the descriptor.
    struct FixedSource {
        descriptor: UsageSourceDescriptor,
    }

    impl UsageSource for FixedSource {
        fn descriptor(&self) -> &UsageSourceDescriptor {
            &self.descriptor
        }

        fn poll<'a>(&'a mut self, ctx: &'a UsageContext) -> UsagePoll<'a> {
            Box::pin(async move {
                let mut row = ProviderUsage::degraded(UsageProvider::Copilot, UsageState::Ok);
                row.windows = vec![UsageWindow::new(WindowKind::FiveHour, 40.0, None)];
                row.fetched_at = Some(ctx.now);
                row
            })
        }
    }

    struct PanickingSource {
        descriptor: UsageSourceDescriptor,
    }

    impl UsageSource for PanickingSource {
        fn descriptor(&self) -> &UsageSourceDescriptor {
            &self.descriptor
        }

        fn poll<'a>(&'a mut self, _ctx: &'a UsageContext) -> UsagePoll<'a> {
            Box::pin(async move { panic!("usage reader exploded") })
        }
    }

    fn fixed(provider: UsageProvider, name: &str, label: &str) -> Box<dyn UsageSource> {
        Box::new(FixedSource {
            descriptor: descriptor(provider, name, label),
        })
    }

    #[tokio::test]
    async fn registry_enumerates_and_polls_sources_in_registration_order() {
        let mut registry = UsageSourceRegistry::new();
        registry
            .register(fixed(UsageProvider::ClaudeCode, "Claude Code", "CC"))
            .register(fixed(UsageProvider::Codex, "Codex", "CX"));

        let names: Vec<_> = registry
            .descriptors()
            .into_iter()
            .map(|d| (d.provider, d.display_name, d.short_label))
            .collect();
        assert_eq!(
            names,
            vec![
                (UsageProvider::ClaudeCode, "Claude Code".into(), "CC".into()),
                (UsageProvider::Codex, "Codex".into(), "CX".into()),
            ]
        );

        let rows = registry.poll_all(&ctx()).await;
        let providers: Vec<_> = rows.iter().map(|row| row.provider).collect();
        assert_eq!(
            providers,
            vec![UsageProvider::ClaudeCode, UsageProvider::Codex]
        );
        assert!(rows.iter().all(|row| row.state == UsageState::Ok));
        assert_eq!(rows[0].fetched_at, Some(UtcTimestamp::UNIX_EPOCH));
    }

    #[tokio::test]
    async fn a_panicking_source_degrades_only_its_own_row() {
        let mut registry = UsageSourceRegistry::new();
        registry
            .register(fixed(UsageProvider::Codex, "Codex", "CX"))
            .register(Box::new(PanickingSource {
                descriptor: descriptor(UsageProvider::GrokBuild, "Grok Build", "GB"),
            }))
            .register(fixed(UsageProvider::ClaudeCode, "Claude Code", "CC"));

        let rows = registry.poll_all(&ctx()).await;

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].state, UsageState::Ok);
        assert_eq!(rows[2].state, UsageState::Ok);
        assert_eq!(rows[2].provider, UsageProvider::ClaudeCode);
        let failed = &rows[1];
        assert_eq!(failed.provider, UsageProvider::GrokBuild);
        assert!(
            matches!(&failed.state, UsageState::Unavailable { reason } if !reason.is_empty()),
            "{:?}",
            failed.state
        );
        assert!(failed.windows.is_empty());
        assert!(!failed.limit_reached);
    }

    #[tokio::test]
    async fn unknown_source_reports_unknown_usage_without_a_window() {
        let mut registry = UsageSourceRegistry::new();
        registry.register(Box::new(UnknownUsageSource::new(
            UsageProvider::Antigravity,
            "Antigravity CLI",
            "AG",
        )));

        assert_eq!(
            registry.descriptors()[0].acquisition,
            UsageAcquisition::None
        );
        let rows = registry.poll_all(&ctx()).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, UsageProvider::Antigravity);
        assert!(
            matches!(&rows[0].state, UsageState::Unknown { reason } if !reason.is_empty()),
            "{:?}",
            rows[0].state
        );
        assert!(rows[0].windows.is_empty());
        assert!(!rows[0].limit_reached);
    }

    #[test]
    fn descriptor_serializes_with_snake_case_acquisition() {
        let json =
            serde_json::to_value(descriptor(UsageProvider::GrokBuild, "Grok Build", "GB")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "provider": "grok_build",
                "display_name": "Grok Build",
                "short_label": "GB",
                "acquisition": "cli_output",
            })
        );
    }
}
