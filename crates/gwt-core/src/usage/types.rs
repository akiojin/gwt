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

use crate::timestamp::UtcTimestamp;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The provider an account's usage belongs to. A provider may have several
/// accounts in its pool (Issue #7841); [`ProviderUsage::pool_account_id`]
/// tells them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageProvider {
    Codex,
    ClaudeCode,
    /// No official usage source yet: reported as [`UsageState::Unknown`].
    GrokBuild,
    /// No official usage source yet: reported as [`UsageState::Unknown`].
    Copilot,
    /// No official usage source yet: reported as [`UsageState::Unknown`].
    Antigravity,
}

impl UsageProvider {
    /// Stable wire identifier used in the frontend protocol.
    pub fn as_str(self) -> &'static str {
        match self {
            UsageProvider::Codex => "codex",
            UsageProvider::ClaudeCode => "claude_code",
            UsageProvider::GrokBuild => "grok_build",
            UsageProvider::Copilot => "copilot",
            UsageProvider::Antigravity => "antigravity",
        }
    }

    /// The agent command id whose sign-in this provider's account is. The
    /// default (adopted) account of a pool uses this as its account id.
    pub fn agent_command(self) -> &'static str {
        match self {
            UsageProvider::Codex => "codex",
            UsageProvider::ClaudeCode => "claude",
            UsageProvider::GrokBuild => "grok",
            UsageProvider::Copilot => "gh",
            UsageProvider::Antigravity => "agy",
        }
    }
}

/// The kind of rate-limit window an account exposes.
///
/// Codex windows are classified from their reported `window_minutes`
/// (Issue #3860), never from the `primary` / `secondary` key position, so an
/// upstream reshuffle of the pools cannot mislabel them. Claude windows are
/// keyed by name (`five_hour` / `seven_day` / ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// 5-hour rolling window (Claude `five_hour`; Codex `window_minutes≈300`).
    FiveHour,
    /// 7-day window (Claude `seven_day`; Codex `window_minutes≈10080`).
    Weekly,
    /// Claude Opus-specific weekly sub-limit (`seven_day_opus`).
    OpusWeekly,
    /// Claude Sonnet-specific weekly sub-limit (`seven_day_sonnet`).
    SonnetWeekly,
    /// Claude Fable-specific weekly sub-limit.
    FableWeekly,
    /// Codex code-review weekly sub-limit.
    CodeReviewWeekly,
    /// A window whose length is missing or matches no known kind. The value is
    /// kept (with its `window_minutes`, when reported) rather than guessed.
    Unknown,
}

/// Minutes in the 5-hour window.
const FIVE_HOUR_MINUTES: u32 = 300;
/// Minutes in the 7-day window.
const WEEKLY_MINUTES: u32 = 10_080;

impl WindowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "five_hour",
            WindowKind::Weekly => "weekly",
            WindowKind::OpusWeekly => "opus_weekly",
            WindowKind::SonnetWeekly => "sonnet_weekly",
            WindowKind::FableWeekly => "fable_weekly",
            WindowKind::CodeReviewWeekly => "code_review_weekly",
            WindowKind::Unknown => "unknown",
        }
    }

    /// Classify a window by its reported length in minutes. Upstream values
    /// drift by a minute or so around the nominal length, so each known kind
    /// accepts ±10%. Anything else is `None` (caller keeps it as `Unknown`).
    pub fn from_minutes(minutes: u32) -> Option<WindowKind> {
        fn near(minutes: u32, nominal: u32) -> bool {
            let tolerance = nominal / 10;
            minutes >= nominal - tolerance && minutes <= nominal + tolerance
        }
        if near(minutes, FIVE_HOUR_MINUTES) {
            Some(WindowKind::FiveHour)
        } else if near(minutes, WEEKLY_MINUTES) {
            Some(WindowKind::Weekly)
        } else {
            None
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
    pub resets_at: Option<UtcTimestamp>,
    /// Actual window length as reported upstream, when known. Lets the UI show
    /// the real length (and label `Unknown` windows) instead of inferring it
    /// from `kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u32>,
}

impl UsageWindow {
    /// Build a window, clamping `used_percent` into `[0, 100]`.
    pub fn new(kind: WindowKind, used_percent: f32, resets_at: Option<UtcTimestamp>) -> Self {
        Self {
            kind,
            used_percent: clamp_percent(used_percent),
            resets_at,
            window_minutes: None,
        }
    }

    /// Attach the upstream-reported window length.
    pub fn with_window_minutes(mut self, window_minutes: Option<u32>) -> Self {
        self.window_minutes = window_minutes;
        self
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
    /// gwt has no official way to read this account's usage (Issue #7841).
    /// Said outright instead of being shown as zero usage.
    Unknown { reason: String },
}

/// Optional Claude usage credits, in the units reported by the provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f32>,
    pub currency: Option<String>,
    pub disabled_reason: Option<String>,
}

/// Optional Codex credits, expressed in the provider's reported units.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexCredits {
    pub has_credits: bool,
    #[serde(default)]
    pub unlimited: bool,
    pub balance: Option<f64>,
}

/// A usage limit gwt itself saw a session hit (the provider's own notice on
/// the agent's screen), kept per account so it stays visible even when the
/// provider offers no usage numbers (Issue #7841).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitObservation {
    pub detected_at: UtcTimestamp,
    /// `None` when the notice named no reset instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<UtcTimestamp>,
}

/// Account-level usage for one account of a provider's pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: UsageProvider,
    /// The gwt pool account this row belongs to (Issue #7841). Together with
    /// `provider` it is the row's key. Snapshots written before accounts were
    /// pooled have none; they describe the provider's default account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_account_id: Option<String>,
    /// Opaque, non-secret fingerprint of the locally authenticated account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    pub plan: Option<String>,
    pub windows: Vec<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_usage: Option<ExtraUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<CodexCredits>,
    pub limit_reached: bool,
    pub state: UsageState,
    pub fetched_at: Option<UtcTimestamp>,
    /// The most recent limit gwt detected for this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_limit: Option<LimitObservation>,
}

impl ProviderUsage {
    /// A non-Ok placeholder carrying only provider + state.
    pub fn degraded(provider: UsageProvider, state: UsageState) -> Self {
        Self {
            provider,
            pool_account_id: None,
            account_id: None,
            account_label: None,
            plan: None,
            windows: Vec::new(),
            extra_usage: None,
            credits: None,
            limit_reached: false,
            state,
            fetched_at: None,
            last_limit: None,
        }
    }

    /// The pool account id this row describes: its own, or the provider's
    /// default account for a row from before accounts were pooled.
    pub fn pool_account(&self) -> &str {
        self.pool_account_id
            .as_deref()
            .unwrap_or(self.provider.agent_command())
    }
}

/// One timestamped minute of a conversation's token activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinuteTokenBucket {
    pub started_at: UtcTimestamp,
    /// Unknown coverage is null; an observed minute without tokens is zero.
    pub tokens: Option<u64>,
}

/// Measurements from one provider conversation, never from its account quota.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTelemetry {
    pub source: String,
    pub quality: String,
    pub observed_at: Option<UtcTimestamp>,
    pub today_tokens: Option<u64>,
    /// Includes cached input for both providers. Total is input plus output.
    pub today_input_tokens: Option<u64>,
    pub today_cache_read_tokens: Option<u64>,
    pub today_cache_creation_tokens: Option<u64>,
    pub today_output_tokens: Option<u64>,
    /// Thirty one-minute buckets, oldest first, including the current minute.
    pub minute_buckets: Vec<MinuteTokenBucket>,
}

fn active_by_default() -> bool {
    true
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
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
    /// Direct provider report only; never derived from prompt size/model limits.
    #[serde(default)]
    pub reported_context_left_pct: Option<f32>,
    #[serde(default)]
    pub provider_session_id: Option<String>,
    #[serde(default)]
    pub project_path: Option<PathBuf>,
    #[serde(default)]
    pub agent_member_id: Option<String>,
    #[serde(default = "active_by_default")]
    pub active: bool,
    #[serde(default)]
    pub telemetry: Option<SessionTelemetry>,
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
    fn window_kind_from_minutes_tolerates_small_drift() {
        assert_eq!(WindowKind::from_minutes(300), Some(WindowKind::FiveHour));
        assert_eq!(WindowKind::from_minutes(299), Some(WindowKind::FiveHour));
        assert_eq!(WindowKind::from_minutes(10080), Some(WindowKind::Weekly));
        assert_eq!(WindowKind::from_minutes(10079), Some(WindowKind::Weekly));
        assert_eq!(WindowKind::from_minutes(1440), None);
        assert_eq!(WindowKind::from_minutes(0), None);
        assert_eq!(WindowKind::Unknown.as_str(), "unknown");
    }

    #[test]
    fn window_minutes_is_optional_on_the_wire() {
        // Issue #3860 AC-4: the window length rides along when known and is
        // omitted (not `null`) otherwise so older frontends keep parsing.
        let without = UsageWindow::new(WindowKind::Weekly, 5.0, None);
        let json = serde_json::to_string(&without).unwrap();
        assert!(!json.contains("window_minutes"));
        let round: UsageWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(round.window_minutes, None);

        let with =
            UsageWindow::new(WindowKind::Unknown, 40.0, None).with_window_minutes(Some(1440));
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.contains("\"window_minutes\":1440"));
        assert!(json.contains("\"kind\":\"unknown\""));
        let round: UsageWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(round, with);
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
        // Issue #7841: every pooled tool has a provider of its own.
        for (provider, wire, command) in [
            (UsageProvider::GrokBuild, "grok_build", "grok"),
            (UsageProvider::Copilot, "copilot", "gh"),
            (UsageProvider::Antigravity, "antigravity", "agy"),
        ] {
            assert_eq!(provider.as_str(), wire);
            assert_eq!(provider.agent_command(), command);
            let json = serde_json::to_string(&provider).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<UsageProvider>(&json).unwrap(),
                provider
            );
        }
    }

    // Issue #7841 T-300: a provider without an official usage source says so.
    #[test]
    fn unknown_state_carries_its_reason() {
        let json = serde_json::to_value(UsageState::Unknown {
            reason: "no official usage source".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind":"unknown","reason":"no official usage source"})
        );
        assert_eq!(
            serde_json::from_value::<UsageState>(json).unwrap(),
            UsageState::Unknown {
                reason: "no official usage source".into()
            }
        );
    }

    // Issue #7841 AC-4: a row is keyed by provider + pool account, and a row
    // written before accounts were pooled is read as the default account.
    #[test]
    fn rows_are_keyed_by_pool_account_and_old_rows_read_as_the_default_account() {
        let legacy: UsageSnapshot = serde_json::from_value(serde_json::json!({
            "accounts": [
                {"provider":"codex","plan":"pro","windows":[{"kind":"weekly","used_percent":40.0,"resets_at":null}],
                 "limit_reached":false,"state":{"kind":"ok"},"fetched_at":null},
                {"provider":"claude_code","plan":null,"windows":[],
                 "limit_reached":false,"state":{"kind":"disabled"},"fetched_at":null}
            ],
            "sessions": []
        }))
        .unwrap();
        let keys: Vec<_> = legacy
            .accounts
            .iter()
            .map(|account| (account.provider, account.pool_account()))
            .collect();
        assert_eq!(
            keys,
            [
                (UsageProvider::Codex, "codex"),
                (UsageProvider::ClaudeCode, "claude")
            ]
        );
        assert!(legacy
            .accounts
            .iter()
            .all(|account| account.last_limit.is_none()));

        let mut pooled = ProviderUsage::degraded(
            UsageProvider::GrokBuild,
            UsageState::Unknown {
                reason: "no official usage source".into(),
            },
        );
        pooled.pool_account_id = Some("grok-2".into());
        pooled.last_limit = Some(LimitObservation {
            detected_at: "2026-11-24T01:00:00Z".parse().unwrap(),
            resets_at: Some("2026-11-24T06:00:00Z".parse().unwrap()),
        });
        let json = serde_json::to_value(&pooled).unwrap();
        assert_eq!(json["pool_account_id"], "grok-2");
        assert_eq!(json["last_limit"]["resets_at"], "2026-11-24T06:00:00Z");
        assert_eq!(
            serde_json::from_value::<ProviderUsage>(json).unwrap(),
            pooled
        );
        assert_eq!(pooled.pool_account(), "grok-2");
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
    fn extra_usage_survives_serde_and_old_snapshots_still_load() {
        let mut account = serde_json::json!({
            "provider":"claude_code", "plan":null, "windows":[],
            "limit_reached":false, "state":{"kind":"ok"}, "fetched_at":null
        });
        let legacy: ProviderUsage = serde_json::from_value(account.clone()).unwrap();
        assert!(serde_json::to_value(legacy)
            .unwrap()
            .get("extra_usage")
            .is_none());
        let credits = serde_json::json!({"is_enabled":true,"monthly_limit":100.0,
            "used_credits":25.0,"utilization":25.0,"currency":"USD","disabled_reason":null});
        account["extra_usage"] = credits.clone();
        let parsed: ProviderUsage = serde_json::from_value(account).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["extra_usage"],
            credits
        );
    }

    #[test]
    fn codex_credits_survive_serde_and_old_snapshots_still_load() {
        let mut account = serde_json::json!({
            "provider":"codex", "plan":null, "windows":[],
            "limit_reached":false, "state":{"kind":"ok"}, "fetched_at":null
        });
        let legacy: ProviderUsage = serde_json::from_value(account.clone()).unwrap();
        assert!(serde_json::to_value(legacy)
            .unwrap()
            .get("credits")
            .is_none());
        let credits = serde_json::json!({
            "has_credits":true, "unlimited":false, "balance":37669.9
        });
        account["credits"] = credits.clone();
        let parsed: ProviderUsage = serde_json::from_value(account).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap()["credits"], credits);
    }

    #[test]
    fn snapshot_roundtrips() {
        let snap = UsageSnapshot {
            accounts: vec![ProviderUsage {
                provider: UsageProvider::Codex,
                pool_account_id: None,
                last_limit: None,
                account_id: None,
                account_label: Some("codex@example.com".into()),
                plan: Some("pro".into()),
                extra_usage: None,
                credits: None,
                windows: vec![UsageWindow::new(
                    WindowKind::FiveHour,
                    12.0,
                    Some("2026-11-18T02:00:00.012Z".parse().unwrap()),
                )],
                limit_reached: false,
                state: UsageState::Ok,
                fetched_at: Some("2026-11-17T22:30:00.123456789Z".parse().unwrap()),
            }],
            sessions: vec![SessionUsage {
                session_id: "s1".into(),
                provider: UsageProvider::ClaudeCode,
                model: Some("claude-opus-4-7".into()),
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reported_context_left_pct: None,
                provider_session_id: None,
                project_path: None,
                agent_member_id: None,
                active: true,
                telemetry: None,
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
        let mut legacy = serde_json::to_value(&snap).unwrap();
        assert_eq!(
            legacy["accounts"][0]["fetched_at"],
            "2026-11-17T22:30:00.123456789Z"
        );
        assert_eq!(
            legacy["accounts"][0]["windows"][0]["resets_at"],
            "2026-11-18T02:00:00.012Z"
        );
        let session = legacy["sessions"][0].as_object_mut().unwrap();
        for key in [
            "cache_read_tokens",
            "cache_creation_tokens",
            "reported_context_left_pct",
            "provider_session_id",
            "project_path",
            "agent_member_id",
            "active",
            "telemetry",
        ] {
            session.remove(key);
        }
        assert_eq!(
            serde_json::from_value::<UsageSnapshot>(legacy).unwrap(),
            snap
        );
    }
}
