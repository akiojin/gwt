//! GitHub API quota classification and rate-limit suppression (Issue #3604).
//!
//! Every `gh` invocation in gwt funnels through
//! [`crate::process_console::spawn_logged`]. That single choke point is where
//! this module plugs in, so the whole workspace gets the same behaviour without
//! each call site re-implementing it:
//!
//! - **Classify** the argv into the quota it will consume ([`GitHubQuota`]).
//!   `gh pr view` / `gh issue list` and friends spend the *GraphQL* budget;
//!   `gh api repos/...` spends the *REST core* budget. The two are independent,
//!   and the incident in Issue #3604 had GraphQL at `0/5000` while REST still
//!   had `4994` left — so exhausting one must never disable the other.
//! - **Identify** a rate-limit failure ([`is_rate_limit_stderr`]) instead of
//!   flattening it into a generic network error.
//! - **Explain** it: `gh` never prints the reset time, so on detection the
//!   caller probes `gh api rate_limit` — a free endpoint that does not consume
//!   either budget — and records the authoritative window ([`RateLimitBlock`]).
//! - **Suppress** further calls against the exhausted resource until that reset
//!   time passes ([`QuotaGate`]), so an exhausted quota stops generating
//!   pointless spawns and log noise.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

/// Machine-readable prefix that marks a rate-limit failure. Callers (and the
/// humans reading their errors) can match on this instead of guessing from a
/// `network error` blob.
pub const RATE_LIMITED_ERROR_CODE: &str = "github_rate_limited";

/// Free probe used to resolve the authoritative reset window. `rate_limit` is
/// documented as not counting against any budget.
pub const RATE_LIMIT_PROBE_ARGS: &[&str] = &["api", "rate_limit"];

/// How long to suppress calls when the reset window could not be measured.
/// Deliberately short: an unmeasured block should re-probe soon rather than
/// stall real work for a full GitHub hour.
const UNKNOWN_RESET_BACKOFF_SECS: i64 = 60;

/// Which GitHub budget a `gh` invocation spends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitHubQuota {
    /// `gh pr` / `gh issue` / `gh search` / `gh api graphql` — the 5000
    /// points-per-hour GraphQL budget.
    GraphQl,
    /// `gh api <rest-path>` — the 5000 requests-per-hour REST core budget.
    Rest,
    /// Neither budget: `gh api rate_limit`, `gh auth`, `gh --version`.
    Free,
}

impl GitHubQuota {
    /// The GitHub `rate_limit` resource key this quota maps to, or `None` when
    /// the quota spends no budget.
    pub fn resource_name(self) -> Option<&'static str> {
        match self {
            Self::GraphQl => Some("graphql"),
            Self::Rest => Some("core"),
            Self::Free => None,
        }
    }
}

/// Classify a `gh` argv (without the `gh` program name) into the quota it
/// spends.
///
/// Unrecognised subcommands fall back to [`GitHubQuota::Rest`] on purpose: the
/// GraphQL gate must never suppress a command we failed to understand.
pub fn classify_gh_args<S: AsRef<str>>(args: &[S]) -> GitHubQuota {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let Some(subcommand) = args.iter().copied().find(|arg| !arg.starts_with('-')) else {
        // `gh --version`, `gh --help`.
        return GitHubQuota::Free;
    };

    match subcommand {
        "api" => classify_api_args(&args),
        // gh implements the whole pr / issue / search surface (reads and
        // mutations alike) on top of GraphQL.
        "pr" | "issue" | "search" | "project" => GitHubQuota::GraphQl,
        "auth" | "config" | "completion" | "help" | "version" | "alias" => GitHubQuota::Free,
        _ => GitHubQuota::Rest,
    }
}

fn classify_api_args(args: &[&str]) -> GitHubQuota {
    if args.contains(&"graphql") {
        return GitHubQuota::GraphQl;
    }
    if args.iter().any(|arg| is_rate_limit_endpoint(arg)) {
        return GitHubQuota::Free;
    }
    GitHubQuota::Rest
}

fn is_rate_limit_endpoint(arg: &str) -> bool {
    arg.trim_start_matches('/')
        .split('?')
        .next()
        .is_some_and(|path| path == "rate_limit")
}

/// Substrings (lowercased) that GitHub / `gh` emit when a request was refused
/// for quota reasons rather than for a transport or lookup reason.
const RATE_LIMIT_MARKERS: &[&str] = &[
    "rate limit exceeded",
    "rate limit already exceeded",
    "secondary rate limit",
    "http 429",
    "too many requests",
];

/// Whether a `gh` stderr body reports a rate-limit refusal.
pub fn is_rate_limit_stderr(stderr: &str) -> bool {
    let lowered = stderr.to_ascii_lowercase();
    RATE_LIMIT_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// An exhausted GitHub budget and the window it recovers in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitBlock {
    /// GitHub `rate_limit` resource key (`graphql` / `core`).
    pub resource: String,
    /// Budget size for the window. `0` when it could not be measured.
    pub limit: u64,
    /// Budget left at the time of measurement.
    pub remaining: u64,
    /// Absolute time the window resets.
    pub reset_at: DateTime<Utc>,
}

impl RateLimitBlock {
    /// Seconds a caller must wait before retrying. Never negative.
    pub fn retry_after_secs(&self, now: DateTime<Utc>) -> i64 {
        (self.reset_at - now).num_seconds().max(0)
    }

    /// Machine-readable one-line explanation: leads with
    /// [`RATE_LIMITED_ERROR_CODE`] and always carries the absolute reset time
    /// plus the remaining seconds (Issue #3604 AC-1 / AC-2).
    pub fn detail(&self, now: DateTime<Utc>) -> String {
        let mut detail = format!("{RATE_LIMITED_ERROR_CODE}: resource={}", self.resource);
        if self.limit > 0 {
            detail.push_str(&format!(" limit={}", self.limit));
        }
        detail.push_str(&format!(" remaining={}", self.remaining));
        detail.push_str(&format!(
            " reset_at={}",
            self.reset_at.to_rfc3339_opts(SecondsFormat::Secs, true)
        ));
        detail.push_str(&format!(" retry_after_secs={}", self.retry_after_secs(now)));
        detail
    }
}

/// Parse `gh api rate_limit` output into the block for `quota`'s resource.
pub fn parse_rate_limit_probe(payload: &str, quota: GitHubQuota) -> Option<RateLimitBlock> {
    let resource = quota.resource_name()?;
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let entry = value.get("resources")?.get(resource)?;
    let reset = entry.get("reset").and_then(serde_json::Value::as_i64)?;
    Some(RateLimitBlock {
        resource: resource.to_string(),
        limit: entry
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        remaining: entry
            .get("remaining")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        reset_at: Utc.timestamp_opt(reset, 0).single()?,
    })
}

/// Build the block to record for `quota` from an optional probe result.
///
/// Two cases fall back to a short conservative backoff instead of the probed
/// window, because in both of them the window reset is the wrong answer:
///
/// - **The probe is unusable** (offline, `gh` missing, malformed payload, an
///   already-elapsed reset). AC-1 must still hold even when AC-2's exact window
///   is unavailable, so the failure stays identified and simply retries soon.
/// - **The budget still has room.** GitHub refuses bursts with a *secondary*
///   rate limit while the hourly budget is untouched; its own guidance is to
///   wait a few minutes. Inheriting the hourly reset there would idle every
///   GraphQL call for up to an hour over a few seconds of burst.
pub fn block_from_probe(
    quota: GitHubQuota,
    probe: Option<RateLimitBlock>,
    now: DateTime<Utc>,
) -> RateLimitBlock {
    if let Some(block) = probe {
        if block.reset_at > now && block.remaining == 0 {
            return block;
        }
    }
    RateLimitBlock {
        resource: quota.resource_name().unwrap_or("unknown").to_string(),
        limit: 0,
        remaining: 0,
        reset_at: now + chrono::Duration::seconds(UNKNOWN_RESET_BACKOFF_SECS),
    }
}

/// Per-resource record of exhausted budgets.
///
/// Blocks are keyed by GitHub resource, so exhausting GraphQL leaves REST (and
/// the free probe) fully available — the asymmetry Issue #3604 measured.
#[derive(Debug, Default)]
pub struct QuotaGate {
    blocks: Mutex<BTreeMap<String, RateLimitBlock>>,
}

impl QuotaGate {
    /// Record an exhausted budget. A later reset extends an active block; an
    /// earlier one never shortens it, so a stale probe cannot re-open the gate.
    pub fn record_exhaustion(&self, block: RateLimitBlock) {
        let mut blocks = self
            .blocks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match blocks.get(&block.resource) {
            Some(existing) if existing.reset_at >= block.reset_at => {}
            _ => {
                blocks.insert(block.resource.clone(), block);
            }
        }
    }

    /// The active block for `quota`, or `None` when the budget is usable.
    pub fn active_block(&self, quota: GitHubQuota, now: DateTime<Utc>) -> Option<RateLimitBlock> {
        let resource = quota.resource_name()?;
        let mut blocks = self
            .blocks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match blocks.get(resource) {
            Some(block) if block.reset_at > now => Some(block.clone()),
            Some(_) => {
                blocks.remove(resource);
                None
            }
            None => None,
        }
    }

    /// Clear `quota`'s block after a call against it succeeded — the window
    /// recovered earlier than the recorded reset, or the probe over-estimated.
    pub fn record_success(&self, quota: GitHubQuota) {
        let Some(resource) = quota.resource_name() else {
            return;
        };
        self.blocks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(resource);
    }
}

/// Process-global gate shared by every `gh` spawn.
pub fn global() -> &'static QuotaGate {
    static GLOBAL: OnceLock<QuotaGate> = OnceLock::new();
    GLOBAL.get_or_init(QuotaGate::default)
}

/// Pre-spawn decision: the detail to fail with when `args` would spend an
/// exhausted budget, or `None` when the call may proceed.
pub fn suppressed_spawn_detail<S: AsRef<str>>(
    gate: &QuotaGate,
    args: &[S],
    now: DateTime<Utc>,
) -> Option<String> {
    let quota = classify_gh_args(args);
    gate.active_block(quota, now).map(|block| block.detail(now))
}

/// Post-failure decision for a `gh` call that did **not** flow through
/// [`crate::process_console::spawn_logged`].
///
/// Returns the identified, reset-carrying replacement for `stderr` (and records
/// the block on `gate`) when the failure was a rate-limit refusal; `None` when
/// it was an ordinary failure the caller should report unchanged.
///
/// `probe` is only invoked once a rate limit is confirmed, so an ordinary
/// failure never pays for the extra `gh api rate_limit` round trip. It must
/// return the raw probe payload.
pub fn observe_failure<S, F>(
    gate: &QuotaGate,
    args: &[S],
    stderr: &str,
    now: DateTime<Utc>,
    probe: F,
) -> Option<String>
where
    S: AsRef<str>,
    F: FnOnce() -> Option<String>,
{
    let quota = classify_gh_args(args);
    if quota == GitHubQuota::Free || !is_rate_limit_stderr(stderr) {
        return None;
    }
    let snapshot = probe().and_then(|payload| parse_rate_limit_probe(&payload, quota));
    let block = block_from_probe(quota, snapshot, now);
    let annotated = annotate_rate_limited_stderr(&block, stderr, now);
    gate.record_exhaustion(block);
    Some(annotated)
}

/// Post-spawn presentation: lead the failure with the machine-readable detail
/// while preserving GitHub's own wording underneath.
pub fn annotate_rate_limited_stderr(
    block: &RateLimitBlock,
    original_stderr: &str,
    now: DateTime<Utc>,
) -> String {
    let detail = block.detail(now);
    let original = original_stderr.trim();
    if original.is_empty() {
        detail
    } else {
        format!("{detail}\n{original}")
    }
}
