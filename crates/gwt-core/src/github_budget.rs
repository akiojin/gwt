//! GitHub API budget observation and demand-side throttling (Issue #3891).
//!
//! [`crate::github_quota`] reacts to a refusal that already happened. This
//! module lets callers see the budget *before* spending it and decide to skip a
//! non-essential call:
//!
//! - **Ledger** ([`BudgetLedger`]): a machine-local directory under
//!   `~/.gwt/github-budget/` shared by every gwt process on the machine (GUI,
//!   every `gwtd` invocation, every worktree). Each budget-spending `gh` spawn
//!   appends one line to `spawns.jsonl`; the newest `gh api rate_limit` payload
//!   is kept in `probe.json`; the newest rate-limit refusal in `block.json`.
//!   The gwt processes are short-lived and independent, so an in-process
//!   counter would see almost nothing — the file is the only place the
//!   machine's whole consumption is visible.
//! - **Snapshot** ([`BudgetSnapshot`]): the primary budgets GitHub reports
//!   (limit / remaining / reset per resource), plus a local approximation of
//!   the secondary (per-minute) limit that GitHub does *not* expose. The
//!   approximation is labelled as such ([`SECONDARY_LIMIT_NOTE`]).
//! - **Throttle** ([`throttle_reason`]): a pure decision from a snapshot and a
//!   [`ThrottlePolicy`]. Callers of non-essential reads (the PM's periodic
//!   `pr.list` inventory) skip the live call and report the reason.
//!
//! The budget belongs to the account, not to a project, so the ledger lives
//! at the gwt home level rather than under a project directory.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::github_quota::{GitHubQuota, RateLimitBlock, RATE_LIMITED_ERROR_CODE};

/// Directory name under [`crate::paths::gwt_home`].
pub const BUDGET_DIR_NAME: &str = "github-budget";
const SPAWNS_FILE: &str = "spawns.jsonl";
const PROBE_FILE: &str = "probe.json";
const BLOCK_FILE: &str = "block.json";

/// Retention of the spawn ledger; nothing older than this is reported.
const LEDGER_RETENTION_SECS: i64 = 3600;
/// Compact the append-only spawn file once it grows past this many lines.
const LEDGER_COMPACT_LINES: usize = 4000;

/// Explicit statement, carried in every snapshot, that the per-minute
/// (secondary) limit is estimated locally because GitHub does not expose it.
pub const SECONDARY_LIMIT_NOTE: &str = "secondary (per-minute) limit remaining is not exposed by \
GitHub; calls_last_minute / calls_last_hour are approximated from this machine's local spawn \
ledger and do not include other machines sharing the account";

/// Resources this module reports: the two budgets gwt spends.
const REPORTED_RESOURCES: &[&str] = &["graphql", "core"];

/// One primary budget window as reported by `gh api rate_limit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceWindow {
    pub limit: u64,
    pub remaining: u64,
    pub reset_at: DateTime<Utc>,
}

/// The newest `gh api rate_limit` payload, reduced to the reported resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeSnapshot {
    pub probed_at: DateTime<Utc>,
    pub resources: BTreeMap<String, ResourceWindow>,
}

/// Parse a `gh api rate_limit` payload into every reported resource.
pub fn parse_rate_limit_probe_all(payload: &str, now: DateTime<Utc>) -> Option<ProbeSnapshot> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let entries = value.get("resources")?.as_object()?;
    let mut resources = BTreeMap::new();
    for name in REPORTED_RESOURCES {
        let Some(entry) = entries.get(*name) else {
            continue;
        };
        let Some(reset) = entry.get("reset").and_then(serde_json::Value::as_i64) else {
            continue;
        };
        let Some(reset_at) = Utc.timestamp_opt(reset, 0).single() else {
            continue;
        };
        resources.insert(
            (*name).to_string(),
            ResourceWindow {
                limit: entry
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                remaining: entry
                    .get("remaining")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                reset_at,
            },
        );
    }
    if resources.is_empty() {
        return None;
    }
    Some(ProbeSnapshot {
        probed_at: now,
        resources,
    })
}

/// Local approximation of one resource's recent consumption.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalConsumption {
    pub calls_last_minute: u64,
    pub calls_last_hour: u64,
}

/// The newest observed rate-limit refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedBlock {
    pub resource: String,
    pub limit: u64,
    pub remaining: u64,
    pub reset_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
}

/// Everything a caller can know about the budget right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetSnapshot {
    pub taken_at: DateTime<Utc>,
    /// Primary budgets from the newest probe, if any was ever taken.
    pub probe: Option<ProbeSnapshot>,
    /// Seconds since that probe.
    pub probe_age_secs: Option<i64>,
    /// Local consumption per resource (`graphql`, `core`), always present.
    pub local: BTreeMap<String, LocalConsumption>,
    /// The newest rate-limit refusal observed on this machine.
    pub last_block: Option<ObservedBlock>,
    /// Always [`SECONDARY_LIMIT_NOTE`].
    pub secondary_note: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct SpawnRecord {
    at: DateTime<Utc>,
    resource: String,
}

/// Handle on the machine-local budget directory.
#[derive(Debug, Clone)]
pub struct BudgetLedger {
    dir: PathBuf,
}

impl BudgetLedger {
    /// The ledger under an explicit directory (tests, tooling).
    pub fn at(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// The machine-wide ledger under `~/.gwt/github-budget/`.
    pub fn global() -> Self {
        Self::at(&crate::paths::gwt_home().join(BUDGET_DIR_NAME))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Record one budget-spending spawn. Free calls are not recorded. Failure
    /// to write is swallowed: bookkeeping must never fail the call it counts.
    pub fn record_spawn(&self, quota: GitHubQuota, now: DateTime<Utc>) {
        let Some(resource) = quota.resource_name() else {
            return;
        };
        let record = SpawnRecord {
            at: now,
            resource: resource.to_string(),
        };
        let Ok(mut line) = serde_json::to_string(&record) else {
            return;
        };
        line.push('\n');
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let path = self.dir.join(SPAWNS_FILE);
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if appended.is_ok() {
            self.compact_if_needed(&path, now);
        }
    }

    /// Persist the newest probe so every process sees the same primary budget.
    pub fn record_probe(&self, probe: &ProbeSnapshot) {
        let _ = write_json_atomic(&self.dir.join(PROBE_FILE), probe);
    }

    /// Persist the newest refusal so a later process can throttle before it
    /// re-discovers the same exhaustion.
    pub fn record_block(&self, block: &RateLimitBlock, now: DateTime<Utc>) {
        let observed = ObservedBlock {
            resource: block.resource.clone(),
            limit: block.limit,
            remaining: block.remaining,
            reset_at: block.reset_at,
            observed_at: now,
        };
        let _ = write_json_atomic(&self.dir.join(BLOCK_FILE), &observed);
    }

    /// Read everything back. A missing or corrupt ledger is an empty one.
    pub fn snapshot(&self, now: DateTime<Utc>) -> BudgetSnapshot {
        let probe: Option<ProbeSnapshot> = read_json(&self.dir.join(PROBE_FILE));
        let probe_age_secs = probe
            .as_ref()
            .map(|probe| (now - probe.probed_at).num_seconds().max(0));
        let mut local: BTreeMap<String, LocalConsumption> = REPORTED_RESOURCES
            .iter()
            .map(|name| ((*name).to_string(), LocalConsumption::default()))
            .collect();
        let hour_start = now - Duration::seconds(LEDGER_RETENTION_SECS);
        let minute_start = now - Duration::seconds(60);
        for record in self.read_spawns() {
            if record.at <= hour_start || record.at > now {
                continue;
            }
            let Some(entry) = local.get_mut(&record.resource) else {
                continue;
            };
            entry.calls_last_hour += 1;
            if record.at > minute_start {
                entry.calls_last_minute += 1;
            }
        }
        BudgetSnapshot {
            taken_at: now,
            probe,
            probe_age_secs,
            local,
            last_block: read_json(&self.dir.join(BLOCK_FILE)),
            secondary_note: SECONDARY_LIMIT_NOTE,
        }
    }

    fn read_spawns(&self) -> Vec<SpawnRecord> {
        std::fs::read_to_string(self.dir.join(SPAWNS_FILE))
            .map(|raw| {
                raw.lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn compact_if_needed(&self, path: &Path, now: DateTime<Utc>) {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        if raw.lines().count() <= LEDGER_COMPACT_LINES {
            return;
        }
        let keep_after = now - Duration::seconds(LEDGER_RETENTION_SECS);
        let mut kept = String::new();
        for line in raw.lines() {
            let Ok(record) = serde_json::from_str::<SpawnRecord>(line) else {
                continue;
            };
            if record.at > keep_after {
                kept.push_str(line);
                kept.push('\n');
            }
        }
        // A concurrent appender may lose a line here; the ledger is an
        // approximation by design and one lost count is acceptable.
        let _ = write_atomic(path, kept.as_bytes());
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let raw = std::fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            Err(error)
        }
    }
}

/// Thresholds for [`throttle_reason`].
#[derive(Debug, Clone, PartialEq)]
pub struct ThrottlePolicy {
    /// Fraction of a resource's limit that must stay available for essential
    /// calls; a non-essential call is skipped below it.
    pub reserve_fraction: f64,
    /// Local budget-spending spawns per minute (per resource) treated as a
    /// burst that would trip the secondary limit.
    pub burst_calls_per_minute: u64,
    /// A probe older than this says nothing about the current window.
    pub probe_max_age_secs: i64,
}

impl Default for ThrottlePolicy {
    fn default() -> Self {
        Self {
            reserve_fraction: 0.2,
            burst_calls_per_minute: 60,
            probe_max_age_secs: 15 * 60,
        }
    }
}

impl ThrottlePolicy {
    /// The reserve, in calls, for a window of `limit`.
    pub fn reserve_for(&self, limit: u64) -> u64 {
        (limit as f64 * self.reserve_fraction).ceil() as u64
    }
}

/// Whether the snapshot's probe is missing or older than the policy allows —
/// the caller should take a fresh (free) probe before deciding.
pub fn probe_is_stale(snapshot: &BudgetSnapshot, policy: &ThrottlePolicy) -> bool {
    snapshot
        .probe_age_secs
        .is_none_or(|age| age > policy.probe_max_age_secs)
}

/// Why a non-essential call against `quota` should be skipped right now, or
/// `None` when it may proceed.
///
/// Three independent signals, checked in order of certainty: an active
/// refusal window, a fresh probe below the reserve, and a local burst. A
/// stale probe is treated as unknown, never as exhausted.
pub fn throttle_reason(
    snapshot: &BudgetSnapshot,
    quota: GitHubQuota,
    policy: &ThrottlePolicy,
    now: DateTime<Utc>,
) -> Option<String> {
    let resource = quota.resource_name()?;
    if let Some(block) = snapshot
        .last_block
        .as_ref()
        .filter(|block| block.resource == resource && block.reset_at > now)
    {
        return Some(format!(
            "{RATE_LIMITED_ERROR_CODE}: resource={resource} reset_at={} retry_after_secs={}",
            block.reset_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            (block.reset_at - now).num_seconds().max(0)
        ));
    }
    if !probe_is_stale(snapshot, policy) {
        if let Some(window) = snapshot
            .probe
            .as_ref()
            .and_then(|probe| probe.resources.get(resource))
        {
            let reserve = policy.reserve_for(window.limit);
            if window.remaining < reserve {
                return Some(format!(
                    "budget_reserve: resource={resource} remaining={} reserve={reserve} limit={} reset_at={}",
                    window.remaining,
                    window.limit,
                    window.reset_at.to_rfc3339_opts(SecondsFormat::Secs, true)
                ));
            }
        }
    }
    if let Some(local) = snapshot.local.get(resource) {
        if local.calls_last_minute >= policy.burst_calls_per_minute {
            return Some(format!(
                "local_burst: resource={resource} calls_last_minute={} burst_limit={}",
                local.calls_last_minute, policy.burst_calls_per_minute
            ));
        }
    }
    None
}
