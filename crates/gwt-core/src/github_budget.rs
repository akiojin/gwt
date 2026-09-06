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
//!
//! Issue #3928 adds the persisted backoff: a refusal recorded here carries a
//! window that grows with every consecutive refusal (1 → 2 → 4 → 8 minutes,
//! capped at 15), and the spawn gate consults it before the in-process
//! [`QuotaGate`] — so a fresh `gwtd` process no longer re-spawns `gh` straight
//! into the secondary limit the previous one just hit. The ledger also keeps
//! the source of every spawn, so the per-minute count can be broken down by
//! caller, and paces bulk readers under the burst limit ([`BudgetLedger::burst_wait`]).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::github_quota::{self, GitHubQuota, QuotaGate, RateLimitBlock, RATE_LIMITED_ERROR_CODE};

/// Directory name under [`crate::paths::gwt_home`].
pub const BUDGET_DIR_NAME: &str = "github-budget";
const SPAWNS_FILE: &str = "spawns.jsonl";
const PROBE_FILE: &str = "probe.json";
const BLOCK_FILE: &str = "block.json";

/// Retention of the spawn ledger; nothing older than this is reported.
const LEDGER_RETENTION_SECS: i64 = 3600;
/// Compact the append-only spawn file once it grows past this many lines.
const LEDGER_COMPACT_LINES: usize = 4000;

/// Issue #3928 AC-1: the first unmeasured refusal waits this long.
pub const RATE_LIMIT_BACKOFF_BASE_SECS: i64 = 60;
/// Issue #3928 AC-1: the wait doubles per consecutive refusal up to this cap.
pub const RATE_LIMIT_BACKOFF_CAP_SECS: i64 = 15 * 60;
/// A refusal observed within this long after the previous window ended
/// continues its streak; a later one starts the schedule over.
const RATE_LIMIT_STREAK_WINDOW_SECS: i64 = 2 * RATE_LIMIT_BACKOFF_CAP_SECS;
/// Slack added to a burst wait so the oldest call has certainly left the window.
const BURST_WAIT_SLACK_SECS: i64 = 1;
/// Source recorded for a spawn that predates the source column.
const UNKNOWN_SPAWN_SOURCE: &str = "unknown";

/// The backoff, in seconds, for the `consecutive_refusals`-th refusal in a row.
///
/// `1 → 60s`, `2 → 120s`, `3 → 240s`, `4 → 480s`, then the cap.
pub fn backoff_secs(consecutive_refusals: u32) -> i64 {
    let exponent = consecutive_refusals.saturating_sub(1).min(16);
    (RATE_LIMIT_BACKOFF_BASE_SECS << exponent).min(RATE_LIMIT_BACKOFF_CAP_SECS)
}

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalConsumption {
    pub calls_last_minute: u64,
    pub calls_last_hour: u64,
    /// Issue #3928 AC-4: who spent the last minute, keyed by
    /// [`spawn_source`] (`<process> gh <command> <verb>`).
    #[serde(default)]
    pub sources_last_minute: BTreeMap<String, u64>,
}

/// The newest observed rate-limit refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedBlock {
    pub resource: String,
    pub limit: u64,
    pub remaining: u64,
    pub reset_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    /// Issue #3928 AC-1: how many refusals in a row this window closes, which
    /// is what sets its length.
    #[serde(default)]
    pub consecutive_refusals: u32,
}

impl ObservedBlock {
    fn to_rate_limit_block(&self) -> RateLimitBlock {
        RateLimitBlock {
            resource: self.resource.clone(),
            limit: self.limit,
            remaining: self.remaining,
            reset_at: self.reset_at,
        }
    }
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
    /// The newest rate-limit refusal observed on this machine, whichever
    /// resource it was against.
    pub last_block: Option<ObservedBlock>,
    /// Issue #3928: the open refusal window per resource. The two budgets are
    /// independent, so a REST refusal must not answer a question about GraphQL
    /// — which is what reading only [`Self::last_block`] would do.
    pub blocks: BTreeMap<String, ObservedBlock>,
    /// Always [`SECONDARY_LIMIT_NOTE`].
    pub secondary_note: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct SpawnRecord {
    at: DateTime<Utc>,
    resource: String,
    #[serde(default)]
    source: String,
}

/// Issue #3928 AC-4: the source label recorded for a `gh` spawn — the process
/// that spawned it and the command shape (`gwtd gh issue view`,
/// `gwt gh api repos`). Coarse on purpose: it names a caller class, not one
/// call, so the per-minute breakdown stays readable.
pub fn spawn_source<S: AsRef<str>>(args: &[S]) -> String {
    let mut shape = String::from("gh");
    let mut positional = args
        .iter()
        .map(AsRef::as_ref)
        .filter(|arg| !arg.starts_with('-'));
    if let Some(subcommand) = positional.next() {
        shape.push(' ');
        shape.push_str(subcommand);
        if let Some(verb) = positional.next() {
            shape.push(' ');
            // `gh api repos/o/r/pulls?state=all` → `gh api repos`.
            let verb = verb
                .trim_start_matches('/')
                .split(['/', '?'])
                .next()
                .unwrap_or(verb);
            shape.push_str(verb);
        }
    }
    format!("{} {shape}", process_origin())
}

/// The file stem of the running executable (`gwt`, `gwtd`), so the breakdown
/// separates the GUI's resync from the CLI's reads.
fn process_origin() -> &'static str {
    use std::sync::OnceLock;
    static ORIGIN: OnceLock<String> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| UNKNOWN_SPAWN_SOURCE.to_string())
    })
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
        self.record_spawn_from(quota, UNKNOWN_SPAWN_SOURCE, now);
    }

    /// [`Self::record_spawn`] with the caller class the count is attributed to
    /// (Issue #3928 AC-4); see [`spawn_source`].
    pub fn record_spawn_from(&self, quota: GitHubQuota, source: &str, now: DateTime<Utc>) {
        let Some(resource) = quota.resource_name() else {
            return;
        };
        let record = SpawnRecord {
            at: now,
            resource: resource.to_string(),
            source: source.to_string(),
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
    /// re-discovers the same exhaustion, and return the window every caller
    /// must honour.
    ///
    /// Issue #3928 AC-1: an unmeasured refusal — the secondary limit, where
    /// GitHub reports the hourly budget as untouched — gets a window from the
    /// exponential schedule ([`backoff_secs`]) keyed by how many refusals in a
    /// row this one is. A measured primary exhaustion keeps GitHub's own reset.
    /// Either way an active window is never shortened.
    pub fn record_block(&self, block: &RateLimitBlock, now: DateTime<Utc>) -> RateLimitBlock {
        let mut blocks = self.read_blocks();
        let previous = blocks.get(&block.resource).cloned();
        let consecutive_refusals = match &previous {
            Some(observed)
                if now <= observed.reset_at + Duration::seconds(RATE_LIMIT_STREAK_WINDOW_SECS) =>
            {
                observed.consecutive_refusals.saturating_add(1)
            }
            _ => 1,
        };
        let measured_primary = block.limit > 0 && block.remaining == 0 && block.reset_at > now;
        let mut reset_at = if measured_primary {
            block.reset_at
        } else {
            now + Duration::seconds(backoff_secs(consecutive_refusals))
        };
        if let Some(observed) = &previous {
            reset_at = reset_at.max(observed.reset_at);
        }
        let observed = ObservedBlock {
            resource: block.resource.clone(),
            limit: block.limit,
            remaining: block.remaining,
            reset_at,
            observed_at: now,
            consecutive_refusals,
        };
        let recorded = observed.to_rate_limit_block();
        blocks.insert(block.resource.clone(), observed);
        let _ = write_json_atomic(&self.dir.join(BLOCK_FILE), &blocks);
        recorded
    }

    /// Forget `quota`'s persisted window after a call against it succeeded:
    /// GitHub answered, so the schedule starts over at the next refusal. The
    /// other resource's window is left in force — the budgets are independent.
    pub fn clear_block(&self, quota: GitHubQuota) {
        let Some(resource) = quota.resource_name() else {
            return;
        };
        let mut blocks = self.read_blocks();
        if blocks.remove(resource).is_none() {
            return;
        }
        if blocks.is_empty() {
            let _ = std::fs::remove_file(self.dir.join(BLOCK_FILE));
            return;
        }
        let _ = write_json_atomic(&self.dir.join(BLOCK_FILE), &blocks);
    }

    /// The persisted window still open for `quota`, or `None` when the budget
    /// is usable as far as this machine knows.
    pub fn active_block(&self, quota: GitHubQuota, now: DateTime<Utc>) -> Option<RateLimitBlock> {
        let resource = quota.resource_name()?;
        self.read_blocks()
            .get(resource)
            .filter(|observed| observed.reset_at > now)
            .map(ObservedBlock::to_rate_limit_block)
    }

    /// Issue #3928 AC-3: how long a bulk reader must wait before its next
    /// `quota` call keeps this machine strictly under the policy's per-minute
    /// burst, or `None` when it may go now. Counts every process's spawns, so
    /// a resync shares the budget with the Monitor scan instead of adding to it.
    pub fn burst_wait(
        &self,
        quota: GitHubQuota,
        policy: &ThrottlePolicy,
        now: DateTime<Utc>,
    ) -> Option<std::time::Duration> {
        let resource = quota.resource_name()?;
        let limit = usize::try_from(policy.burst_calls_per_minute).ok()?;
        if limit < 2 {
            return None;
        }
        let minute_start = now - Duration::seconds(60);
        let mut recent: Vec<DateTime<Utc>> = self
            .read_spawns()
            .into_iter()
            .filter(|record| {
                record.resource == resource && record.at > minute_start && record.at <= now
            })
            .map(|record| record.at)
            .collect();
        // After this call the window must hold fewer than `limit` calls, so at
        // most `limit - 2` of the current ones may remain.
        if recent.len() < limit - 1 {
            return None;
        }
        recent.sort_unstable();
        let frees_at =
            recent[recent.len() + 1 - limit] + Duration::seconds(60 + BURST_WAIT_SLACK_SECS);
        (frees_at - now).to_std().ok()
    }

    /// Every persisted refusal window, keyed by GitHub resource.
    ///
    /// Reads the pre-Issue #3928 shape too — a bare [`ObservedBlock`] for the
    /// single newest refusal — so an upgrade does not silently drop an open
    /// window and resume spawning into it.
    fn read_blocks(&self) -> BTreeMap<String, ObservedBlock> {
        let path = self.dir.join(BLOCK_FILE);
        if let Some(blocks) = read_json::<BTreeMap<String, ObservedBlock>>(&path) {
            return blocks;
        }
        read_json::<ObservedBlock>(&path)
            .map(|observed| BTreeMap::from([(observed.resource.clone(), observed)]))
            .unwrap_or_default()
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
                let source = if record.source.is_empty() {
                    UNKNOWN_SPAWN_SOURCE.to_string()
                } else {
                    record.source
                };
                *entry.sources_last_minute.entry(source).or_default() += 1;
            }
        }
        let blocks = self.read_blocks();
        BudgetSnapshot {
            taken_at: now,
            probe,
            probe_age_secs,
            local,
            last_block: blocks
                .values()
                .max_by_key(|observed| observed.observed_at)
                .cloned(),
            blocks,
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
        .blocks
        .get(resource)
        .filter(|block| block.reset_at > now)
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

/// Pre-spawn decision across both memories (Issue #3928 AC-1): the in-process
/// gate first, then the window another process persisted. Returns the detail
/// to fail with, or `None` when the call may proceed.
pub fn suppressed_spawn_detail<S: AsRef<str>>(
    gate: &QuotaGate,
    ledger: &BudgetLedger,
    args: &[S],
    now: DateTime<Utc>,
) -> Option<String> {
    github_quota::suppressed_spawn_detail(gate, args, now).or_else(|| {
        ledger
            .active_block(github_quota::classify_gh_args(args), now)
            .map(|block| block.detail(now))
    })
}

/// Post-failure decision for a `gh` call that did not flow through the
/// process console: the persisted counterpart of
/// [`github_quota::observe_failure`].
///
/// When `stderr` is a rate-limit refusal, the window is computed with the
/// exponential schedule, persisted on `ledger`, remembered on `gate`, and the
/// annotated stderr (leading with the machine-readable detail) is returned.
/// An ordinary failure yields `None` and is left untouched.
pub fn observe_refusal<S, F>(
    gate: &QuotaGate,
    ledger: &BudgetLedger,
    args: &[S],
    stderr: &str,
    now: DateTime<Utc>,
    probe: F,
) -> Option<String>
where
    S: AsRef<str>,
    F: FnOnce() -> Option<String>,
{
    let quota = github_quota::classify_gh_args(args);
    if quota == GitHubQuota::Free || !github_quota::is_rate_limit_stderr(stderr) {
        return None;
    }
    let snapshot =
        probe().and_then(|payload| github_quota::parse_rate_limit_probe(&payload, quota));
    let block = ledger.record_block(&github_quota::block_from_probe(quota, snapshot, now), now);
    let annotated = github_quota::annotate_rate_limited_stderr(&block, stderr, now);
    gate.record_exhaustion(block);
    Some(annotated)
}

/// Issue #3928 AC-4: one resource's budget state as a status surface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudgetStatus {
    /// Whether a non-essential call would be skipped right now.
    pub throttled: bool,
    /// The [`throttle_reason`], when throttled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// End of the persisted refusal window, when one is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_until: Option<DateTime<Utc>>,
    /// Seconds until that window ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<i64>,
    /// Refusals in a row behind the open window; `0` when none is open.
    pub consecutive_refusals: u32,
    /// This machine's spawns against the resource in the last minute.
    pub calls_last_minute: u64,
    /// The per-minute burst the policy treats as a secondary-limit risk.
    pub burst_limit: u64,
    /// `calls_last_minute` broken down by [`spawn_source`].
    pub sources_last_minute: BTreeMap<String, u64>,
}

/// Issue #3928 AC-4: the budget state per reported resource (`graphql`,
/// `core`), for `issue.monitor.status` and any other surface a PM reads.
pub fn status_by_resource(
    snapshot: &BudgetSnapshot,
    policy: &ThrottlePolicy,
    now: DateTime<Utc>,
) -> BTreeMap<String, ResourceBudgetStatus> {
    [GitHubQuota::GraphQl, GitHubQuota::Rest]
        .into_iter()
        .filter_map(|quota| {
            let resource = quota.resource_name()?;
            let reason = throttle_reason(snapshot, quota, policy, now);
            let window = snapshot
                .blocks
                .get(resource)
                .filter(|block| block.reset_at > now);
            let local = snapshot.local.get(resource).cloned().unwrap_or_default();
            Some((
                resource.to_string(),
                ResourceBudgetStatus {
                    throttled: reason.is_some(),
                    reason,
                    backoff_until: window.map(|block| block.reset_at),
                    retry_after_secs: window
                        .map(|block| (block.reset_at - now).num_seconds().max(0)),
                    consecutive_refusals: window
                        .map(|block| block.consecutive_refusals)
                        .unwrap_or(0),
                    calls_last_minute: local.calls_last_minute,
                    burst_limit: policy.burst_calls_per_minute,
                    sources_last_minute: local.sources_last_minute,
                },
            ))
        })
        .collect()
}
