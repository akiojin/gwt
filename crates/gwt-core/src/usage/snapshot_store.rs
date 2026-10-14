//! Host-local copy of the usage poller's latest account readings
//! (Issue #4908).
//!
//! The poller lives in the GUI process and runs only while a window is open,
//! so its readings reached the status bar and nothing else: a JSON reader saw
//! a provider's holds but never the usage they sat beside, and could not tell
//! "no hold because the account is fine" from "no hold because nobody looked".
//! The poller publishes each tick here, and [`read_provider_usage_readings`]
//! turns the file into one row per provider that always says either what was
//! read and when, or why there is nothing to read.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::state::{age_secs, is_stale, DEFAULT_STALE_AFTER_SECS};
use super::types::{ProviderUsage, UsageProvider, UsageState};

/// The providers the poller has telemetry for, in the order they are reported.
const PROVIDERS: [UsageProvider; 2] = [UsageProvider::Codex, UsageProvider::ClaudeCode];

/// One poller tick as published to disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredProviderUsage {
    /// When the poller produced these rows.
    pub observed_at: DateTime<Utc>,
    pub accounts: Vec<ProviderUsage>,
}

/// One rate-limit window of a [`ProviderUsageReading`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsageWindowReading {
    /// `five_hour`, `weekly`, ... — only the windows the provider reported.
    pub kind: String,
    pub used_percent: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

/// One provider's usage as a JSON reader sees it.
///
/// A row with no numbers never carries empty or zero ones: `windows` and
/// `limit_reached` are absent and `state` / `detail` say why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsageReading {
    /// The agent command the account backs (`codex` / `claude`), matching how
    /// provider quota holds are keyed.
    pub provider: String,
    /// `ok`, `stale` (numbers present but old), `disabled`, `no_data`,
    /// `unavailable`, or `not_observed` (the poller has published nothing).
    pub state: String,
    /// Why there is no fresh reading. Absent for `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When the provider reported these numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
    /// When the poller last produced a row for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    /// Seconds since `fetched_at`, or since `observed_at` without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_reached: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<ProviderUsageWindowReading>,
}

/// Where the poller publishes its readings: one file per host, because the
/// accounts are the host's and not any project's.
pub fn provider_usage_snapshot_path() -> PathBuf {
    crate::paths::gwt_home()
        .join("usage")
        .join("provider-usage.json")
}

/// Publish one poller tick. Atomic, so a reader never sees half a file.
pub fn write_provider_usage_snapshot(
    path: &Path,
    accounts: &[ProviderUsage],
    observed_at: DateTime<Utc>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stored = StoredProviderUsage {
        observed_at,
        // The label is the account's e-mail or name, shown in the GUI only.
        // No reader of this file needs it, so it is not written to disk.
        accounts: accounts
            .iter()
            .map(|account| ProviderUsage {
                account_label: None,
                ..account.clone()
            })
            .collect(),
    };
    crate::atomic_file::write_atomic(path, &serde_json::to_vec(&stored)?)
}

/// The readings in the snapshot at `path`, one row per provider, as of `now`.
pub fn read_provider_usage_readings(path: &Path, now: DateTime<Utc>) -> Vec<ProviderUsageReading> {
    let stored = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<StoredProviderUsage>(&bytes)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    };
    match stored {
        Ok(stored) => provider_usage_readings(stored.as_ref(), now),
        Err(error) => PROVIDERS
            .iter()
            .map(|provider| {
                ProviderUsageReading::without_numbers(
                    *provider,
                    "unavailable",
                    format!("the usage snapshot could not be read: {error}"),
                )
            })
            .collect(),
    }
}

/// Project a published tick (or its absence) into one row per provider.
pub fn provider_usage_readings(
    stored: Option<&StoredProviderUsage>,
    now: DateTime<Utc>,
) -> Vec<ProviderUsageReading> {
    PROVIDERS
        .iter()
        .map(|provider| {
            let account = stored.and_then(|stored| {
                stored
                    .accounts
                    .iter()
                    .find(|account| account.provider == *provider)
                    .map(|account| (account, stored.observed_at))
            });
            match account {
                Some((account, observed_at)) => {
                    ProviderUsageReading::from_account(account, observed_at, now)
                }
                None => ProviderUsageReading::without_numbers(
                    *provider,
                    "not_observed",
                    "the usage poller has published no reading for this provider; \
                     it runs only while a gwt window is open"
                        .to_string(),
                ),
            }
        })
        .collect()
}

fn provider_name(provider: UsageProvider) -> &'static str {
    match provider {
        UsageProvider::Codex => "codex",
        UsageProvider::ClaudeCode => "claude",
    }
}

fn rfc3339(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl ProviderUsageReading {
    fn without_numbers(provider: UsageProvider, state: &str, detail: String) -> Self {
        Self {
            provider: provider_name(provider).to_string(),
            state: state.to_string(),
            detail: Some(detail),
            fetched_at: None,
            observed_at: None,
            age_secs: None,
            limit_reached: None,
            windows: Vec::new(),
        }
    }

    fn from_account(
        account: &ProviderUsage,
        observed_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Self {
        let poller_age_secs = age_secs(observed_at, now);
        // Numbers are reported only when the provider actually returned some.
        let has_numbers = !account.windows.is_empty();
        let (state, detail) = match &account.state {
            UsageState::Ok if !has_numbers => (
                "no_data",
                Some("the provider reported no usage windows".to_string()),
            ),
            UsageState::Ok => ("ok", None),
            UsageState::Stale { .. } => (
                "stale",
                Some("the provider's last reading is older than the freshness window".to_string()),
            ),
            UsageState::Disabled => (
                "disabled",
                Some("usage collection for this provider is turned off in settings".to_string()),
            ),
            UsageState::NoData => (
                "no_data",
                Some("the provider has produced no usage data yet".to_string()),
            ),
            UsageState::Unavailable { reason } => ("unavailable", Some(reason.clone())),
        };
        // A row the poller stopped refreshing is old whatever it says: a
        // reading is no longer fresh, and a cause may no longer hold.
        let (state, detail) = if is_stale(observed_at, now, DEFAULT_STALE_AFTER_SECS) {
            let stopped = format!(
                "the usage poller last ran {poller_age_secs}s ago; \
                 it runs only while a gwt window is open"
            );
            (
                if state == "ok" { "stale" } else { state },
                Some(detail.map_or(stopped.clone(), |detail| format!("{detail}; {stopped}"))),
            )
        } else {
            (state, detail)
        };
        Self {
            provider: provider_name(account.provider).to_string(),
            state: state.to_string(),
            detail,
            fetched_at: account.fetched_at.map(rfc3339),
            observed_at: Some(rfc3339(observed_at)),
            age_secs: Some(
                account
                    .fetched_at
                    .map_or(poller_age_secs, |fetched_at| age_secs(fetched_at, now)),
            ),
            limit_reached: (has_numbers || account.limit_reached).then_some(account.limit_reached),
            windows: account
                .windows
                .iter()
                .map(|window| ProviderUsageWindowReading {
                    kind: window.kind.as_str().to_string(),
                    used_percent: window.used_percent,
                    resets_at: window.resets_at.map(rfc3339),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{UsageWindow, WindowKind};
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + secs, 0).unwrap()
    }

    fn codex_at_limit(fetched_at: DateTime<Utc>) -> ProviderUsage {
        ProviderUsage {
            provider: UsageProvider::Codex,
            account_id: Some("acct".to_string()),
            account_label: Some("someone@example.com".to_string()),
            plan: Some("pro".to_string()),
            windows: vec![UsageWindow::new(
                WindowKind::Weekly,
                100.0,
                Some(fetched_at + chrono::Duration::days(5)),
            )],
            limit_reached: true,
            state: UsageState::Ok,
            fetched_at: Some(fetched_at),
        }
    }

    fn reading<'a>(
        readings: &'a [ProviderUsageReading],
        provider: &str,
    ) -> &'a ProviderUsageReading {
        readings
            .iter()
            .find(|reading| reading.provider == provider)
            .unwrap_or_else(|| panic!("no {provider} row in {readings:?}"))
    }

    /// AC-5: a published reading comes back with its numbers and both clocks.
    #[test]
    fn a_published_reading_is_read_back_with_its_numbers_and_times() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("usage").join("provider-usage.json");
        let accounts = [
            codex_at_limit(t(-30)),
            ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::Disabled),
        ];

        write_provider_usage_snapshot(&path, &accounts, t(0)).expect("publish");
        assert!(
            !fs::read_to_string(&path)
                .expect("snapshot")
                .contains("someone@example.com"),
            "the account label stays out of the snapshot"
        );
        let readings = read_provider_usage_readings(&path, t(60));

        let codex = reading(&readings, "codex");
        assert_eq!(codex.state, "ok");
        assert_eq!(codex.detail, None);
        assert_eq!(codex.limit_reached, Some(true));
        assert_eq!(codex.windows.len(), 1);
        assert_eq!(codex.windows[0].kind, "weekly");
        assert_eq!(codex.windows[0].used_percent, 100.0);
        assert_eq!(
            codex.windows[0].resets_at.as_deref(),
            Some(
                (t(-30) + chrono::Duration::days(5))
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
                    .as_str()
            )
        );
        assert_eq!(
            codex.fetched_at.as_deref(),
            Some(t(-30).to_rfc3339_opts(SecondsFormat::Secs, true).as_str())
        );
        assert_eq!(
            codex.observed_at.as_deref(),
            Some(t(0).to_rfc3339_opts(SecondsFormat::Secs, true).as_str())
        );
        assert_eq!(codex.age_secs, Some(90));
    }

    /// AC-6: with nothing published, every provider says so — no empty list,
    /// no zeroes.
    #[test]
    fn a_missing_snapshot_reads_as_not_observed_for_every_provider() {
        let dir = tempfile::tempdir().expect("tempdir");

        let readings = read_provider_usage_readings(&dir.path().join("absent.json"), t(0));

        assert_eq!(readings.len(), 2);
        for provider in ["codex", "claude"] {
            let row = reading(&readings, provider);
            assert_eq!(row.state, "not_observed");
            assert!(row
                .detail
                .as_deref()
                .is_some_and(|detail| !detail.is_empty()));
            assert_eq!(row.limit_reached, None);
            assert!(row.windows.is_empty());
            let json = serde_json::to_value(row).expect("json");
            assert!(
                json.get("windows").is_none() && json.get("limit_reached").is_none(),
                "a row without a reading carries no numbers: {json}"
            );
        }
    }

    /// AC-6: every way the poller can fail to produce numbers keeps its own
    /// name and reason.
    #[test]
    fn a_reading_that_was_not_obtained_keeps_its_cause() {
        let stored = StoredProviderUsage {
            observed_at: t(0),
            accounts: vec![
                ProviderUsage::degraded(
                    UsageProvider::Codex,
                    UsageState::Unavailable {
                        reason: "HTTP 429".to_string(),
                    },
                ),
                ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::Disabled),
            ],
        };

        let readings = provider_usage_readings(Some(&stored), t(10));

        let codex = reading(&readings, "codex");
        assert_eq!(codex.state, "unavailable");
        assert_eq!(codex.detail.as_deref(), Some("HTTP 429"));
        assert!(codex.windows.is_empty());
        assert_eq!(codex.limit_reached, None);
        assert_eq!(codex.fetched_at, None);
        assert_eq!(codex.age_secs, Some(10));
        let claude = reading(&readings, "claude");
        assert_eq!(claude.state, "disabled");
        assert!(claude.detail.is_some());

        let no_data = StoredProviderUsage {
            observed_at: t(0),
            accounts: vec![ProviderUsage::degraded(
                UsageProvider::Codex,
                UsageState::NoData,
            )],
        };
        let readings = provider_usage_readings(Some(&no_data), t(10));
        assert_eq!(reading(&readings, "codex").state, "no_data");
        assert_eq!(
            reading(&readings, "claude").state,
            "not_observed",
            "a provider the tick has no row for was not observed"
        );

        // An account that answered but carried no windows is not `ok` with
        // nothing in it: it has no numbers, and its limit flag is kept.
        let mut windowless = codex_at_limit(t(0));
        windowless.windows.clear();
        let windowless = StoredProviderUsage {
            observed_at: t(0),
            accounts: vec![windowless],
        };
        let readings = provider_usage_readings(Some(&windowless), t(10));
        let codex = reading(&readings, "codex");
        assert_eq!(codex.state, "no_data");
        assert!(codex.detail.is_some());
        assert_eq!(codex.limit_reached, Some(true));

        // A cause recorded by a poller that has since stopped says so.
        let readings = provider_usage_readings(Some(&stored), t(DEFAULT_STALE_AFTER_SECS + 1));
        let codex = reading(&readings, "codex");
        assert_eq!(codex.state, "unavailable");
        assert!(codex
            .detail
            .as_deref()
            .is_some_and(|detail| detail.starts_with("HTTP 429; ") && detail.contains("poller")));
    }

    /// AC-5: numbers the poller stopped refreshing stay readable but are
    /// labelled stale, whichever clock went old.
    #[test]
    fn an_old_reading_is_labelled_stale_and_keeps_its_numbers() {
        let stopped_poller = StoredProviderUsage {
            observed_at: t(0),
            accounts: vec![codex_at_limit(t(-5))],
        };
        let readings =
            provider_usage_readings(Some(&stopped_poller), t(DEFAULT_STALE_AFTER_SECS + 1));
        let codex = reading(&readings, "codex");
        assert_eq!(codex.state, "stale");
        assert!(codex
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("poller")));
        assert_eq!(codex.windows.len(), 1);
        assert_eq!(codex.limit_reached, Some(true));

        let mut old_reading = codex_at_limit(t(-4000));
        old_reading.state = UsageState::Stale { age_secs: 4000 };
        let stale_at_the_source = StoredProviderUsage {
            observed_at: t(0),
            accounts: vec![old_reading],
        };
        let readings = provider_usage_readings(Some(&stale_at_the_source), t(5));
        let codex = reading(&readings, "codex");
        assert_eq!(codex.state, "stale");
        assert_eq!(codex.age_secs, Some(4005));
        assert_eq!(codex.windows.len(), 1);
    }

    /// AC-6: a snapshot that cannot be parsed is a failed read, not an empty
    /// account.
    #[test]
    fn an_unreadable_snapshot_reads_as_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("provider-usage.json");
        fs::write(&path, b"{not json").expect("write");

        let readings = read_provider_usage_readings(&path, t(0));

        assert_eq!(readings.len(), 2);
        for row in &readings {
            assert_eq!(row.state, "unavailable");
            assert!(row
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("snapshot")));
        }
    }

    #[test]
    fn the_snapshot_lives_under_the_gwt_home() {
        let path = provider_usage_snapshot_path();
        assert!(path.ends_with("usage/provider-usage.json"), "{path:?}");
        assert!(path.starts_with(crate::paths::gwt_home()));
    }
}
