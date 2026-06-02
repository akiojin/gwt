//! Core domain types for provider usage and rate-limit display (SPEC-2970).
//!
//! Two axes are modeled here:
//! - account-level usage ([`ProviderUsage`]): a shared pool per provider
//!   account (Codex / Claude Code), holding rolling/weekly/sub windows.
//! - per-session usage ([`SessionUsage`]): tokens and context occupancy for a
//!   single agent session.
//!
//! All percentages are clamped to `[0, 100]`. Reset instants are optional
//! because upstream payloads do not always include them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A usage pool owner. Each variant maps to one CLI account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageProvider {
    Codex,
    ClaudeCode,
}

impl UsageProvider {
    /// Stable wire identifier used in the frontend protocol.
    pub fn as_str(self) -> &'static str {
        match self {
            UsageProvider::Codex => "codex",
            UsageProvider::ClaudeCode => "claude_code",
        }
    }
}

/// The kind of rate-limit window an account exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// 5-hour rolling window (Codex `primary` / Claude `five_hour`).
    FiveHour,
    /// 7-day window (Codex `secondary` / Claude `seven_day`).
    Weekly,
    /// Claude Opus-specific weekly sub-limit (`seven_day_opus`).
    OpusWeekly,
    /// Claude Sonnet-specific weekly sub-limit (`seven_day_sonnet`).
    SonnetWeekly,
    /// Codex code-review weekly sub-limit.
    CodeReviewWeekly,
}

impl WindowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "five_hour",
            WindowKind::Weekly => "weekly",
            WindowKind::OpusWeekly => "opus_weekly",
            WindowKind::SonnetWeekly => "sonnet_weekly",
            WindowKind::CodeReviewWeekly => "code_review_weekly",
        }
    }
}

/// Clamp a raw utilization percentage into the valid `[0, 100]` range.
pub fn clamp_percent(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 100.0)
    }
}

/// One rate-limit window with current utilization and optional reset instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub kind: WindowKind,
    pub used_percent: f32,
    pub resets_at: Option<DateTime<Utc>>,
}

impl UsageWindow {
    /// Build a window, clamping `used_percent` into `[0, 100]`.
    pub fn new(kind: WindowKind, used_percent: f32, resets_at: Option<DateTime<Utc>>) -> Self {
        Self {
            kind,
            used_percent: clamp_percent(used_percent),
            resets_at,
        }
    }
}

/// Display state for a usage row. This is the single source of truth for
/// graceful-degradation rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageState {
    /// Fresh data is available.
    Ok,
    /// Collection is intentionally disabled (e.g. Claude account not opted in).
    Disabled,
    /// Enabled but no source data yet (e.g. no Codex session created).
    NoData,
    /// A fetch attempt failed; carries a short human reason.
    Unavailable { reason: String },
    /// Data exists but is older than the freshness threshold.
    Stale { age_secs: u64 },
}

/// Account-level usage for one provider (shared pool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: UsageProvider,
    pub plan: Option<String>,
    pub windows: Vec<UsageWindow>,
    pub limit_reached: bool,
    pub state: UsageState,
    pub fetched_at: Option<DateTime<Utc>>,
}

impl ProviderUsage {
    /// A non-Ok placeholder carrying only provider + state.
    pub fn degraded(provider: UsageProvider, state: UsageState) -> Self {
        Self {
            provider,
            plan: None,
            windows: Vec::new(),
            limit_reached: false,
            state,
            fetched_at: None,
        }
    }
}

/// Per-session usage for a single agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    pub session_id: String,
    pub provider: UsageProvider,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    /// Tokens currently occupying the context window, when derivable.
    pub context_used_tokens: Option<u64>,
    /// Model context window size, when known.
    pub context_limit_tokens: Option<u64>,
    /// Remaining context as a percentage `[0, 100]`, when both above are known.
    pub context_left_pct: Option<f32>,
    pub limit_reached: bool,
    /// Whether this session participates in subscription usage display.
    /// API-key backends and non-target agents are `eligible == false`.
    pub eligible: bool,
    pub state: UsageState,
}

impl SessionUsage {
    /// Compute `context_left_pct` from used/limit, returning `None` when the
    /// limit is unknown or zero. Result is clamped to `[0, 100]`.
    pub fn context_left_from(used: Option<u64>, limit: Option<u64>) -> Option<f32> {
        match (used, limit) {
            (Some(used), Some(limit)) if limit > 0 => {
                let remaining = limit.saturating_sub(used) as f32 / limit as f32 * 100.0;
                Some(clamp_percent(remaining))
            }
            _ => None,
        }
    }
}

/// A complete usage poll result: all account rows + all session rows + the
/// daily/weekly consumption rollups.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub accounts: Vec<ProviderUsage>,
    pub sessions: Vec<SessionUsage>,
    #[serde(default)]
    pub consumption: Vec<super::consumption::ProviderConsumption>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_clamped() {
        assert_eq!(clamp_percent(-5.0), 0.0);
        assert_eq!(clamp_percent(150.0), 100.0);
        assert_eq!(clamp_percent(42.5), 42.5);
        assert_eq!(clamp_percent(f32::NAN), 0.0);
    }

    #[test]
    fn window_new_clamps_percent() {
        let w = UsageWindow::new(WindowKind::Weekly, 250.0, None);
        assert_eq!(w.used_percent, 100.0);
        assert_eq!(w.kind, WindowKind::Weekly);
    }

    #[test]
    fn context_left_handles_unknown_and_zero_limit() {
        assert_eq!(SessionUsage::context_left_from(Some(10), None), None);
        assert_eq!(SessionUsage::context_left_from(None, Some(100)), None);
        assert_eq!(SessionUsage::context_left_from(Some(10), Some(0)), None);
        assert_eq!(
            SessionUsage::context_left_from(Some(25), Some(100)),
            Some(75.0)
        );
        // Over-budget context never goes negative.
        assert_eq!(
            SessionUsage::context_left_from(Some(200), Some(100)),
            Some(0.0)
        );
    }

    #[test]
    fn provider_wire_ids_are_stable() {
        assert_eq!(UsageProvider::Codex.as_str(), "codex");
        assert_eq!(UsageProvider::ClaudeCode.as_str(), "claude_code");
    }

    #[test]
    fn state_serializes_with_tag() {
        let json = serde_json::to_string(&UsageState::Unavailable {
            reason: "http 429".into(),
        })
        .unwrap();
        assert!(json.contains("\"kind\":\"unavailable\""));
        assert!(json.contains("\"reason\":\"http 429\""));
        let round: UsageState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            round,
            UsageState::Unavailable {
                reason: "http 429".into()
            }
        );
    }

    #[test]
    fn snapshot_roundtrips() {
        let snap = UsageSnapshot {
            accounts: vec![ProviderUsage {
                provider: UsageProvider::Codex,
                plan: Some("pro".into()),
                windows: vec![UsageWindow::new(WindowKind::FiveHour, 12.0, None)],
                limit_reached: false,
                state: UsageState::Ok,
                fetched_at: None,
            }],
            sessions: vec![SessionUsage {
                session_id: "s1".into(),
                provider: UsageProvider::ClaudeCode,
                model: Some("claude-opus-4-7".into()),
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                context_used_tokens: Some(50),
                context_limit_tokens: Some(200),
                context_left_pct: Some(75.0),
                limit_reached: false,
                eligible: true,
                state: UsageState::Ok,
            }],
            consumption: Vec::new(),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let round: UsageSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, round);
    }
}
