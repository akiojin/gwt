//! Limits gwt itself detected, kept per pool account (Issue #7841 T-301).
//!
//! A provider's own limit notice on an agent's screen is the one limit signal
//! gwt has for every tool, including those with no official usage source. The
//! runtime that saw the notice records it here under the account the session
//! ran on; the usage poller attaches the latest one to that account's row, so
//! the Accounts screen shows when the account last hit its limit and when it
//! said access returns.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{LimitObservation, ProviderUsage, UsageProvider};

/// The latest limit observed for one pool account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountLimit {
    pub provider: UsageProvider,
    pub account_id: String,
    #[serde(flatten)]
    pub observation: LimitObservation,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredAccountLimits {
    #[serde(default)]
    accounts: Vec<AccountLimit>,
}

/// One file per host: accounts are the host's, not any project's.
pub fn account_limits_path() -> PathBuf {
    crate::paths::gwt_home()
        .join("usage")
        .join("account-limits.json")
}

/// The recorded limits. A missing or unreadable file records nothing.
pub fn read_account_limits(path: &Path) -> Vec<AccountLimit> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<StoredAccountLimits>(&bytes).ok())
        .map(|stored| stored.accounts)
        .unwrap_or_default()
}

/// Keep `limit` as the account's latest observation. An older observation
/// never replaces a newer one.
pub fn record_account_limit(path: &Path, limit: AccountLimit) -> io::Result<()> {
    let mut accounts = read_account_limits(path);
    match accounts.iter_mut().find(|recorded| {
        recorded.provider == limit.provider && recorded.account_id == limit.account_id
    }) {
        Some(recorded) if recorded.observation.detected_at >= limit.observation.detected_at => {
            return Ok(())
        }
        Some(recorded) => *recorded = limit,
        None => accounts.push(limit),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::atomic_file::write_atomic(
        path,
        &serde_json::to_vec(&StoredAccountLimits { accounts })?,
    )
}

/// Attach each account's latest recorded limit to its usage row.
pub fn attach_account_limits(rows: &mut [ProviderUsage], limits: &[AccountLimit]) {
    for row in rows {
        row.last_limit = limits
            .iter()
            .find(|limit| limit.provider == row.provider && limit.account_id == row.pool_account())
            .map(|limit| limit.observation.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::UsageState;

    fn limit(provider: UsageProvider, account_id: &str, detected: &str) -> AccountLimit {
        AccountLimit {
            provider,
            account_id: account_id.to_string(),
            observation: LimitObservation {
                detected_at: detected.parse().unwrap(),
                resets_at: Some("2026-11-24T06:00:00Z".parse().unwrap()),
            },
        }
    }

    #[test]
    fn the_latest_limit_of_each_account_is_kept_and_attached_to_its_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage").join("account-limits.json");
        assert!(read_account_limits(&path).is_empty());

        let first = limit(
            UsageProvider::ClaudeCode,
            "claude-2",
            "2026-11-24T01:00:00Z",
        );
        let later = limit(
            UsageProvider::ClaudeCode,
            "claude-2",
            "2026-11-24T02:00:00Z",
        );
        record_account_limit(&path, first.clone()).unwrap();
        record_account_limit(&path, later.clone()).unwrap();
        record_account_limit(&path, first).unwrap();
        let other = limit(UsageProvider::GrokBuild, "grok", "2026-11-24T03:00:00Z");
        record_account_limit(&path, other.clone()).unwrap();
        assert_eq!(read_account_limits(&path), [later.clone(), other.clone()]);

        let mut rows = [
            ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData),
            ProviderUsage {
                pool_account_id: Some("claude-2".into()),
                ..ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::NoData)
            },
            ProviderUsage::degraded(
                UsageProvider::GrokBuild,
                UsageState::Unknown {
                    reason: "no official usage source".into(),
                },
            ),
        ];
        attach_account_limits(&mut rows, &read_account_limits(&path));
        assert_eq!(rows[0].last_limit, None, "the default account hit no limit");
        assert_eq!(rows[1].last_limit, Some(later.observation));
        assert_eq!(rows[2].last_limit, Some(other.observation));
    }

    #[test]
    fn an_unreadable_file_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account-limits.json");
        fs::write(&path, "not json").unwrap();
        assert!(read_account_limits(&path).is_empty());
    }
}
