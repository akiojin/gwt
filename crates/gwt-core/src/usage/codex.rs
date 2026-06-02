//! Codex usage readers (SPEC-2970 FR-002/FR-003/FR-008/FR-016).
//!
//! Source of truth is the per-session rollout JSONL under
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`. Each
//! `token_count` event embeds:
//! - `rate_limits` — account-level 5h (`primary`) / weekly (`secondary`) +
//!   optional `code_review`, with `plan_type` and `rate_limit_reached_type`.
//! - `info.total_token_usage` — cumulative per-session tokens.
//! - `info.model_context_window` — the model's context size (so no static
//!   table lookup is needed for Codex).
//!
//! Reset instants come as `resets_at` (unix epoch seconds, observed) and we
//! also accept `resets_at` RFC3339 strings and relative `resets_in_seconds`
//! for forward/backward compatibility.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use super::types::{
    ProviderUsage, SessionUsage, UsageProvider, UsageState, UsageWindow, WindowKind,
};

/// Parsed account-level fields from a `rate_limits` object.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexAccount {
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
    pub limit_reached: bool,
}

fn parse_reset(window: &Value, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if let Some(epoch) = window.get("resets_at").and_then(Value::as_i64) {
        return DateTime::from_timestamp(epoch, 0);
    }
    if let Some(text) = window.get("resets_at").and_then(Value::as_str) {
        if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    if let Some(rel) = window.get("resets_in_seconds").and_then(Value::as_i64) {
        return Some(now + Duration::seconds(rel));
    }
    None
}

fn window_from(kind: WindowKind, obj: &Value, now: DateTime<Utc>) -> Option<UsageWindow> {
    let used = obj.get("used_percent").and_then(Value::as_f64)? as f32;
    Some(UsageWindow::new(kind, used, parse_reset(obj, now)))
}

/// Parse a `rate_limits` JSON object into account windows. Returns `None` when
/// the value is `null` (Codex exec mode) or has no recognized windows.
pub fn parse_rate_limits(rate_limits: &Value, now: DateTime<Utc>) -> Option<CodexAccount> {
    if rate_limits.is_null() {
        return None;
    }
    let mut windows = Vec::new();
    if let Some(primary) = rate_limits.get("primary") {
        if let Some(w) = window_from(WindowKind::FiveHour, primary, now) {
            windows.push(w);
        }
    }
    if let Some(secondary) = rate_limits.get("secondary") {
        if let Some(w) = window_from(WindowKind::Weekly, secondary, now) {
            windows.push(w);
        }
    }
    if let Some(review) = rate_limits.get("code_review") {
        if let Some(w) = window_from(WindowKind::CodeReviewWeekly, review, now) {
            windows.push(w);
        }
    }
    if windows.is_empty() {
        return None;
    }
    let plan = rate_limits
        .get("plan_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let limit_reached = rate_limits
        .get("rate_limit_reached_type")
        .map(|v| !v.is_null())
        .unwrap_or(false)
        || windows.iter().any(|w| w.used_percent >= 100.0);
    Some(CodexAccount {
        windows,
        plan,
        limit_reached,
    })
}

/// Scan rollout JSONL text and return the last `token_count` payload value.
fn last_token_count(jsonl: &str) -> Option<Value> {
    let mut last = None;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = obj.get("payload").unwrap_or(&obj);
        if payload.get("type").and_then(Value::as_str) == Some("token_count") {
            last = Some(payload.clone());
        }
    }
    last
}

/// Extract the last `model`/`cli` model name seen in the rollout (session_meta
/// or token_count payloads).
fn last_model(jsonl: &str) -> Option<String> {
    let mut model = None;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = obj.get("payload").unwrap_or(&obj);
        if let Some(m) = payload.get("model").and_then(Value::as_str) {
            model = Some(m.to_string());
        }
        if let Some(m) = obj.get("model").and_then(Value::as_str) {
            model = Some(m.to_string());
        }
    }
    model
}

/// Parse account usage from full rollout JSONL text.
pub fn parse_codex_account(jsonl: &str, now: DateTime<Utc>) -> Option<CodexAccount> {
    let tc = last_token_count(jsonl)?;
    let rl = tc.get("rate_limits")?;
    parse_rate_limits(rl, now)
}

/// Parse per-session usage from full rollout JSONL text.
pub fn parse_codex_session(session_id: &str, jsonl: &str) -> SessionUsage {
    let tc = last_token_count(jsonl);
    let info = tc.as_ref().and_then(|t| t.get("info"));
    let total = info.and_then(|i| i.get("total_token_usage"));
    let input = total
        .and_then(|t| t.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = total
        .and_then(|t| t.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = total
        .and_then(|t| t.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(input + output);

    let context_limit = info
        .and_then(|i| i.get("model_context_window"))
        .and_then(Value::as_u64);
    // Approximate current context occupancy from the last turn's prompt size.
    // `input_tokens` already includes the cached portion, so it must NOT be
    // summed with `cached_input_tokens` (that double-counts and pins ctx to 0%).
    let last = info.and_then(|i| i.get("last_token_usage"));
    let context_used = last.and_then(|l| l.get("input_tokens").and_then(Value::as_u64));
    let context_left_pct = SessionUsage::context_left_from(context_used, context_limit);

    let limit_reached = tc
        .as_ref()
        .and_then(|t| t.get("rate_limits"))
        .and_then(|rl| rl.get("rate_limit_reached_type"))
        .map(|v| !v.is_null())
        .unwrap_or(false);

    let state = if tc.is_some() {
        UsageState::Ok
    } else {
        UsageState::NoData
    };

    SessionUsage {
        session_id: session_id.to_string(),
        provider: UsageProvider::Codex,
        model: last_model(jsonl),
        input_tokens: input,
        output_tokens: output,
        total_tokens,
        context_used_tokens: context_used,
        context_limit_tokens: context_limit,
        context_left_pct,
        limit_reached,
        eligible: true,
        state,
    }
}

/// Resolve the effective Codex home directory.
pub fn codex_home() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|h| h.join(".codex"))
}

/// Find the most recently modified `rollout-*.jsonl` under `<home>/sessions`.
pub fn newest_rollout(home: &Path) -> Option<PathBuf> {
    let sessions = home.join("sessions");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    visit_rollouts(&sessions, &mut |path, mtime| match &newest {
        Some((best, _)) if *best >= mtime => {}
        _ => newest = Some((mtime, path)),
    });
    newest.map(|(_, p)| p)
}

/// Rollout paths under `<home>/sessions` modified at/after `cutoff`, newest
/// first, capped at `limit`. Used by daily/weekly consumption aggregation to
/// bound how many files are scanned.
pub fn rollouts_modified_since(
    home: &Path,
    cutoff: std::time::SystemTime,
    limit: usize,
) -> Vec<PathBuf> {
    let sessions = home.join("sessions");
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    visit_rollouts(&sessions, &mut |path, mtime| {
        if mtime >= cutoff {
            all.push((mtime, path));
        }
    });
    all.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    all.into_iter().take(limit).map(|(_, p)| p).collect()
}

/// Collect rollout paths under `<home>/sessions`, newest first, capped at
/// `limit`.
pub fn recent_rollouts(home: &Path, limit: usize) -> Vec<PathBuf> {
    let sessions = home.join("sessions");
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    visit_rollouts(&sessions, &mut |path, mtime| all.push((mtime, path)));
    all.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    all.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn visit_rollouts(dir: &Path, f: &mut impl FnMut(PathBuf, std::time::SystemTime)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rollouts(&path, f);
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
            .unwrap_or(false)
        {
            if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
                f(path, mtime);
            }
        }
    }
}

/// Locate a rollout file whose name contains the given session id (uuid).
pub fn rollout_for_session(home: &Path, session_id: &str) -> Option<PathBuf> {
    let sessions = home.join("sessions");
    let mut found = None;
    visit_rollouts(&sessions, &mut |path, _| {
        if found.is_none()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(session_id))
                .unwrap_or(false)
        {
            found = Some(path);
        }
    });
    found
}

/// Read account usage from recent rollouts under `home` (newest first), using
/// the freshest one that carries a usable `rate_limits` block. A brand-new
/// session's rollout may not have emitted `rate_limits` yet, so scanning a few
/// recent files avoids a spurious `NoData`. `rate_limits` is account-global, so
/// the most recent one is the freshest account state. Returns degraded
/// `NoData` when none have account data.
pub fn read_codex_account(home: &Path, now: DateTime<Utc>) -> ProviderUsage {
    for path in recent_rollouts(home, 8) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(acc) = parse_codex_account(&text, now) {
            return ProviderUsage {
                provider: UsageProvider::Codex,
                plan: acc.plan,
                windows: acc.windows,
                limit_reached: acc.limit_reached,
                state: UsageState::Ok,
                fetched_at: Some(now),
            };
        }
    }
    ProviderUsage::degraded(UsageProvider::Codex, UsageState::NoData)
}

/// Read per-session usage for the given session id from its rollout file.
pub fn read_codex_session(home: &Path, session_id: &str) -> Option<SessionUsage> {
    let path = rollout_for_session(home, session_id)?;
    let text = fs::read_to_string(&path).ok()?;
    Some(parse_codex_session(session_id, &text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap()
    }

    #[test]
    fn parses_epoch_resets_at_schema() {
        let rl = serde_json::json!({
            "limit_id": "codex",
            "primary": {"used_percent": 0.0, "window_minutes": 300, "resets_at": 1780376627},
            "secondary": {"used_percent": 23.0, "window_minutes": 10080, "resets_at": 1780899120},
            "plan_type": "pro",
            "rate_limit_reached_type": null
        });
        let acc = parse_rate_limits(&rl, now()).unwrap();
        assert_eq!(acc.plan.as_deref(), Some("pro"));
        assert!(!acc.limit_reached);
        assert_eq!(acc.windows.len(), 2);
        assert_eq!(acc.windows[0].kind, WindowKind::FiveHour);
        assert_eq!(acc.windows[1].kind, WindowKind::Weekly);
        assert_eq!(acc.windows[1].used_percent, 23.0);
        assert_eq!(
            acc.windows[0].resets_at,
            DateTime::from_timestamp(1780376627, 0)
        );
    }

    #[test]
    fn parses_relative_resets_in_seconds_schema() {
        let rl = serde_json::json!({
            "primary": {"used_percent": 10.0, "window_minutes": 299, "resets_in_seconds": 600},
            "secondary": {"used_percent": 5.0, "window_minutes": 10079, "resets_in_seconds": 3600}
        });
        let acc = parse_rate_limits(&rl, now()).unwrap();
        assert_eq!(
            acc.windows[0].resets_at,
            Some(now() + Duration::seconds(600))
        );
        assert_eq!(acc.plan, None);
    }

    #[test]
    fn includes_code_review_when_present() {
        let rl = serde_json::json!({
            "primary": {"used_percent": 1.0},
            "secondary": {"used_percent": 2.0},
            "code_review": {"used_percent": 6.0}
        });
        let acc = parse_rate_limits(&rl, now()).unwrap();
        assert_eq!(acc.windows.len(), 3);
        assert_eq!(acc.windows[2].kind, WindowKind::CodeReviewWeekly);
    }

    #[test]
    fn null_rate_limits_is_none() {
        assert!(parse_rate_limits(&Value::Null, now()).is_none());
    }

    #[test]
    fn limit_reached_from_reached_type() {
        let rl = serde_json::json!({
            "primary": {"used_percent": 100.0},
            "rate_limit_reached_type": "primary"
        });
        let acc = parse_rate_limits(&rl, now()).unwrap();
        assert!(acc.limit_reached);
    }

    #[test]
    fn account_from_jsonl_uses_last_token_count() {
        let jsonl = concat!(
            r#"{"type":"session_meta","payload":{"id":"abc","model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":1.0}},"info":{}}}"#,
            "\n",
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":42.0}},"info":{}}}"#,
            "\n",
        );
        let acc = parse_codex_account(jsonl, now()).unwrap();
        assert_eq!(acc.windows[0].used_percent, 42.0);
    }

    #[test]
    fn session_usage_aggregates_totals_and_context() {
        let jsonl = concat!(
            r#"{"type":"session_meta","payload":{"id":"abc","model":"gpt-5.5"}}"#,
            "\n",
            r#"{"payload":{"type":"token_count","rate_limits":null,"info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":20,"total_tokens":160},"last_token_usage":{"input_tokens":80,"cached_input_tokens":10},"model_context_window":400000}}}"#,
            "\n",
        );
        let s = parse_codex_session("abc", jsonl);
        assert_eq!(s.input_tokens, 100);
        assert_eq!(s.output_tokens, 20);
        assert_eq!(s.total_tokens, 160);
        assert_eq!(s.context_limit_tokens, Some(400000));
        // input_tokens already includes the cached portion; no double-count.
        assert_eq!(s.context_used_tokens, Some(80));
        assert_eq!(s.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(s.state, UsageState::Ok);
        assert!(s.eligible);
    }

    #[test]
    fn read_account_handles_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let acc = read_codex_account(dir.path(), now());
        assert_eq!(acc.state, UsageState::NoData);
        assert_eq!(acc.provider, UsageProvider::Codex);
    }

    #[test]
    fn newest_rollout_picks_latest_by_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/06/02");
        fs::create_dir_all(&day).unwrap();
        let older = day.join("rollout-2026-06-02T08-00-00-aaaa.jsonl");
        let newer = day.join("rollout-2026-06-02T09-00-00-bbbb.jsonl");
        fs::write(&older, r#"{"payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":1.0}},"info":{}}}"#).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&newer, r#"{"payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":2.0}},"info":{}}}"#).unwrap();
        let picked = newest_rollout(dir.path()).unwrap();
        assert_eq!(picked, newer);
        let acc = read_codex_account(dir.path(), now());
        assert_eq!(acc.windows[0].used_percent, 2.0);

        // session lookup by uuid fragment
        let found = rollout_for_session(dir.path(), "bbbb").unwrap();
        assert_eq!(found, newer);
    }

    #[test]
    fn account_skips_newest_rollout_without_rate_limits() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/06/02");
        fs::create_dir_all(&day).unwrap();
        let older = day.join("rollout-2026-06-02T08-00-00-aaaa.jsonl");
        let newer = day.join("rollout-2026-06-02T09-00-00-bbbb.jsonl");
        // Older has rate_limits; the newer (fresh session) has none yet.
        fs::write(
            &older,
            r#"{"payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":7.0}},"info":{}}}"#,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(
            &newer,
            r#"{"payload":{"type":"session_meta","payload":{"id":"bbbb"}}}"#,
        )
        .unwrap();
        let acc = read_codex_account(dir.path(), now());
        assert_eq!(acc.state, UsageState::Ok);
        assert_eq!(acc.windows[0].used_percent, 7.0);
    }

    #[test]
    fn rollouts_modified_since_orders_newest_first_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/05/28");
        fs::create_dir_all(&day).unwrap();
        let older = day.join("rollout-2026-05-28T08-00-00-aaaa.jsonl");
        let newer = day.join("rollout-2026-05-28T09-00-00-bbbb.jsonl");
        fs::write(&older, "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&newer, "{}").unwrap();
        // Files that are not `rollout-*.jsonl` are ignored.
        fs::write(day.join("other.jsonl"), "{}").unwrap();
        let found = rollouts_modified_since(dir.path(), std::time::SystemTime::UNIX_EPOCH, 10);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], newer);
        // `limit` caps to the newest.
        let one = rollouts_modified_since(dir.path(), std::time::SystemTime::UNIX_EPOCH, 1);
        assert_eq!(one, vec![newer]);
        // A future cutoff excludes everything.
        let far = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        assert!(rollouts_modified_since(dir.path(), far, 10).is_empty());
    }

    #[test]
    fn read_codex_session_reads_rollout_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/05/28");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout-2026-05-28T09-00-00-bbbb.jsonl"),
            r#"{"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}}"#,
        )
        .unwrap();
        let s = read_codex_session(dir.path(), "bbbb").unwrap();
        assert_eq!(s.input_tokens, 7);
        assert_eq!(s.total_tokens, 10);
        // Unknown session id → None.
        assert!(read_codex_session(dir.path(), "zzzz").is_none());
    }

    #[test]
    fn parse_codex_session_without_token_count_is_nodata() {
        let s = parse_codex_session("sid", r#"{"payload":{"type":"session_meta"}}"#);
        assert_eq!(s.state, UsageState::NoData);
        assert_eq!(s.total_tokens, 0);
        assert!(s.context_used_tokens.is_none());
    }
}
