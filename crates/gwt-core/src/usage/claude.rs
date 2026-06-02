//! Claude Code usage readers (SPEC-2970 FR-004/FR-005/FR-006/FR-017).
//!
//! Two sources, asymmetric:
//! - account-level: undocumented `GET /api/oauth/usage` with the OAuth token
//!   from `~/.claude/.credentials.json` (or macOS Keychain). Opt-in only.
//! - per-session: local transcript JSONL under
//!   `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`. No network / no
//!   credentials, so always available.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::model_context;
use super::types::{
    ProviderUsage, SessionUsage, UsageProvider, UsageState, UsageWindow, WindowKind,
};

const OAUTH_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Account windows parsed from the `/api/oauth/usage` response body.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeAccount {
    pub windows: Vec<UsageWindow>,
    pub limit_reached: bool,
}

/// OAuth credentials resolved from disk or Keychain.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeCreds {
    pub access_token: String,
    pub subscription_type: Option<String>,
}

fn claude_window(obj: &Value, kind: WindowKind, _now: DateTime<Utc>) -> Option<UsageWindow> {
    if obj.is_null() {
        return None;
    }
    let util = obj.get("utilization").and_then(Value::as_f64)? as f32;
    let reset = obj
        .get("resets_at")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    Some(UsageWindow::new(kind, util, reset))
}

/// Parse the `/api/oauth/usage` JSON body into account windows. Missing/`null`
/// sub-windows (e.g. `seven_day_opus`) are skipped.
pub fn parse_oauth_usage(body: &str, now: DateTime<Utc>) -> Option<ClaudeAccount> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut windows = Vec::new();
    for (key, kind) in [
        ("five_hour", WindowKind::FiveHour),
        ("seven_day", WindowKind::Weekly),
        ("seven_day_opus", WindowKind::OpusWeekly),
        ("seven_day_sonnet", WindowKind::SonnetWeekly),
    ] {
        if let Some(obj) = v.get(key) {
            if let Some(w) = claude_window(obj, kind, now) {
                windows.push(w);
            }
        }
    }
    if windows.is_empty() {
        return None;
    }
    let limit_reached = windows.iter().any(|w| w.used_percent >= 100.0);
    Some(ClaudeAccount {
        windows,
        limit_reached,
    })
}

/// Parse OAuth credentials JSON (`{ claudeAiOauth: { accessToken, ... } }`).
pub fn parse_creds_json(body: &str) -> Option<ClaudeCreds> {
    let v: Value = serde_json::from_str(body).ok()?;
    let oauth = v.get("claudeAiOauth").unwrap_or(&v);
    let access_token = oauth
        .get("accessToken")
        .and_then(Value::as_str)?
        .to_string();
    if access_token.is_empty() {
        return None;
    }
    let subscription_type = oauth
        .get("subscriptionType")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(ClaudeCreds {
        access_token,
        subscription_type,
    })
}

fn creds_from_file(home: &Path) -> Option<ClaudeCreds> {
    let text = fs::read_to_string(home.join(".credentials.json")).ok()?;
    parse_creds_json(&text)
}

#[cfg(target_os = "macos")]
fn creds_from_keychain() -> Option<ClaudeCreds> {
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    parse_creds_json(text.trim())
}

#[cfg(not(target_os = "macos"))]
fn creds_from_keychain() -> Option<ClaudeCreds> {
    None
}

/// Resolve Claude OAuth credentials. On macOS the Keychain holds the live
/// (refreshed) token while `~/.claude/.credentials.json` can be stale and
/// expired — yielding 401s — so the Keychain is preferred and the file is the
/// fallback. On other platforms the Keychain reader is a no-op and the file is
/// authoritative.
pub fn resolve_claude_creds(home: &Path) -> Option<ClaudeCreds> {
    creds_from_keychain().or_else(|| creds_from_file(home))
}

/// Build the `User-Agent` from the installed `claude --version`
/// (`"2.1.159 (Claude Code)"` → `claude-code/2.1.159`).
pub fn claude_user_agent() -> String {
    let version = Command::new("claude")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().next().map(str::to_string));
    match version {
        Some(v) if !v.is_empty() => format!("claude-code/{v}"),
        _ => "claude-code/unknown".to_string(),
    }
}

/// Map a non-success HTTP status to a degraded [`UsageState`].
pub fn map_status_to_state(status: u16) -> UsageState {
    let reason = match status {
        401 | 403 => "auth expired".to_string(),
        429 => "rate limited (429)".to_string(),
        other => format!("http {other}"),
    };
    UsageState::Unavailable { reason }
}

/// Fetch account usage from the undocumented OAuth usage endpoint.
pub async fn fetch_claude_account(
    creds: &ClaudeCreds,
    user_agent: &str,
    now: DateTime<Utc>,
) -> ProviderUsage {
    let plan = creds.subscription_type.clone();
    let client = reqwest::Client::new();
    let resp = client
        .get(OAUTH_USAGE_URL)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", creds.access_token),
        )
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let body = r.text().await.unwrap_or_default();
            match parse_oauth_usage(&body, now) {
                Some(acc) => ProviderUsage {
                    provider: UsageProvider::ClaudeCode,
                    plan,
                    windows: acc.windows,
                    limit_reached: acc.limit_reached,
                    state: UsageState::Ok,
                    fetched_at: Some(now),
                },
                None => ProviderUsage {
                    provider: UsageProvider::ClaudeCode,
                    plan,
                    windows: Vec::new(),
                    limit_reached: false,
                    state: UsageState::Unavailable {
                        reason: "unparseable usage".to_string(),
                    },
                    fetched_at: Some(now),
                },
            }
        }
        Ok(r) => ProviderUsage {
            provider: UsageProvider::ClaudeCode,
            plan,
            windows: Vec::new(),
            limit_reached: false,
            state: map_status_to_state(r.status().as_u16()),
            fetched_at: Some(now),
        },
        Err(_) => ProviderUsage::degraded(
            UsageProvider::ClaudeCode,
            UsageState::Unavailable {
                reason: "request failed".to_string(),
            },
        ),
    }
}

/// Parse per-session usage from Claude transcript JSONL text.
pub fn parse_claude_transcript(session_id: &str, jsonl: &str) -> SessionUsage {
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cache_create = 0u64;
    let mut cache_read = 0u64;
    let mut model: Option<String> = None;
    let mut context_used: Option<u64> = None;
    let mut saw_assistant = false;

    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(msg) = obj.get("message") else {
            continue;
        };
        saw_assistant = true;
        if let Some(m) = msg.get("model").and_then(Value::as_str) {
            model = Some(m.to_string());
        }
        let Some(usage) = msg.get("usage") else {
            continue;
        };
        let inp = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let outp = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cc = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cr = usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        input += inp;
        output += outp;
        cache_create += cc;
        cache_read += cr;
        // The latest assistant turn's prompt footprint approximates the
        // current context occupancy.
        context_used = Some(inp + cc + cr);
    }

    let total_tokens = input + output + cache_create + cache_read;
    let context_limit = model.as_deref().and_then(model_context::context_limit);
    let context_left_pct = SessionUsage::context_left_from(context_used, context_limit);

    SessionUsage {
        session_id: session_id.to_string(),
        provider: UsageProvider::ClaudeCode,
        model,
        input_tokens: input,
        output_tokens: output,
        total_tokens,
        context_used_tokens: context_used,
        context_limit_tokens: context_limit,
        context_left_pct,
        limit_reached: false,
        eligible: true,
        state: if saw_assistant {
            UsageState::Ok
        } else {
            UsageState::NoData
        },
    }
}

/// Resolve the Claude home directory. Honors `CLAUDE_CONFIG_DIR` (Claude
/// Code's own override) before falling back to `~/.claude`, mirroring how
/// [`super::codex::codex_home`] honors `CODEX_HOME`.
pub fn claude_home() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|h| h.join(".claude"))
}

/// Locate the transcript file for a session id under `~/.claude/projects/*`.
pub fn transcript_for_session(home: &Path, session_id: &str) -> Option<PathBuf> {
    let projects = home.join("projects");
    let entries = fs::read_dir(projects).ok()?;
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            let candidate = entry.path().join(format!("{session_id}.jsonl"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Transcript paths under `~/.claude/projects/*` modified at/after `cutoff`,
/// newest first, capped at `limit`. Used by daily/weekly consumption.
pub fn transcripts_modified_since(
    home: &Path,
    cutoff: std::time::SystemTime,
    limit: usize,
) -> Vec<PathBuf> {
    let projects = home.join("projects");
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    if let Ok(dirs) = fs::read_dir(&projects) {
        for dir in dirs.flatten() {
            if !dir.path().is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(dir.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if let Ok(mtime) = file.metadata().and_then(|m| m.modified()) {
                    if mtime >= cutoff {
                        all.push((mtime, path));
                    }
                }
            }
        }
    }
    all.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    all.into_iter().take(limit).map(|(_, p)| p).collect()
}

/// Read per-session usage for a Claude session id from its transcript.
pub fn read_claude_session(home: &Path, session_id: &str) -> Option<SessionUsage> {
    let path = transcript_for_session(home, session_id)?;
    let text = fs::read_to_string(&path).ok()?;
    Some(parse_claude_transcript(session_id, &text))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verbatim shape returned by the live `/api/oauth/usage` endpoint
    // (2026-06-02): RFC3339 `+00:00` offsets, many null sub-windows, and a
    // sonnet window whose `resets_at` is null.
    #[test]
    fn parses_live_endpoint_shape() {
        let body = r#"{"five_hour":{"utilization":28.0,"resets_at":"2026-06-02T05:00:00.497878+00:00"},"seven_day":{"utilization":6.0,"resets_at":"2026-06-03T21:00:00.497905+00:00"},"seven_day_oauth_apps":null,"seven_day_opus":null,"seven_day_sonnet":{"utilization":0.0,"resets_at":null},"seven_day_cowork":null,"tangelo":null,"extra_usage":{"is_enabled":false}}"#;
        let acc = parse_oauth_usage(body, now()).unwrap();
        // five_hour + seven_day + seven_day_sonnet (opus is null → skipped).
        assert_eq!(acc.windows.len(), 3);
        let five = acc
            .windows
            .iter()
            .find(|w| w.kind == WindowKind::FiveHour)
            .unwrap();
        assert_eq!(five.used_percent, 28.0);
        assert!(five.resets_at.is_some());
        let week = acc
            .windows
            .iter()
            .find(|w| w.kind == WindowKind::Weekly)
            .unwrap();
        assert_eq!(week.used_percent, 6.0);
        let sonnet = acc
            .windows
            .iter()
            .find(|w| w.kind == WindowKind::SonnetWeekly)
            .unwrap();
        assert_eq!(sonnet.used_percent, 0.0);
        assert!(sonnet.resets_at.is_none());
        assert!(!acc.limit_reached);
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000, 0).unwrap()
    }

    #[test]
    fn parses_all_windows_with_iso_reset() {
        let body = r#"{
            "five_hour": {"utilization": 48, "resets_at": "2026-06-02T16:10:00Z"},
            "seven_day": {"utilization": 18, "resets_at": "2026-06-05T09:00:00Z"},
            "seven_day_opus": {"utilization": 31, "resets_at": "2026-06-05T09:00:00Z"},
            "seven_day_sonnet": null
        }"#;
        let acc = parse_oauth_usage(body, now()).unwrap();
        assert_eq!(acc.windows.len(), 3);
        assert_eq!(acc.windows[0].kind, WindowKind::FiveHour);
        assert_eq!(acc.windows[0].used_percent, 48.0);
        assert!(acc.windows[0].resets_at.is_some());
        assert_eq!(acc.windows[2].kind, WindowKind::OpusWeekly);
        assert!(!acc.limit_reached);
    }

    #[test]
    fn limit_reached_when_window_full() {
        let body = r#"{"five_hour": {"utilization": 100}, "seven_day": {"utilization": 50}}"#;
        let acc = parse_oauth_usage(body, now()).unwrap();
        assert!(acc.limit_reached);
    }

    #[test]
    fn empty_or_bad_body_is_none() {
        assert!(parse_oauth_usage("{}", now()).is_none());
        assert!(parse_oauth_usage("not json", now()).is_none());
    }

    #[test]
    fn creds_parsed_from_oauth_shape() {
        let body = r#"{"claudeAiOauth":{"accessToken":"tok123","refreshToken":"r","expiresAt":1,"subscriptionType":"max"}}"#;
        let c = parse_creds_json(body).unwrap();
        assert_eq!(c.access_token, "tok123");
        assert_eq!(c.subscription_type.as_deref(), Some("max"));
    }

    #[test]
    fn creds_empty_token_rejected() {
        assert!(parse_creds_json(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_creds_json("{}").is_none());
    }

    #[test]
    fn status_mapping() {
        assert_eq!(
            map_status_to_state(429),
            UsageState::Unavailable {
                reason: "rate limited (429)".into()
            }
        );
        assert_eq!(
            map_status_to_state(401),
            UsageState::Unavailable {
                reason: "auth expired".into()
            }
        );
        assert_eq!(
            map_status_to_state(500),
            UsageState::Unavailable {
                reason: "http 500".into()
            }
        );
    }

    #[test]
    fn transcript_aggregates_usage_and_context() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"role":"user"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":30,"cache_read_input_tokens":10}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":5,"output_tokens":40,"cache_creation_input_tokens":0,"cache_read_input_tokens":140}}}"#,
            "\n",
        );
        let s = parse_claude_transcript("sid", jsonl);
        assert_eq!(s.input_tokens, 105);
        assert_eq!(s.output_tokens, 60);
        assert_eq!(s.total_tokens, 105 + 60 + 30 + 150);
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-7"));
        // last turn context = 5 + 0 + 140
        assert_eq!(s.context_used_tokens, Some(145));
        assert_eq!(s.context_limit_tokens, Some(200_000));
        assert!(s.context_left_pct.is_some());
        assert_eq!(s.state, UsageState::Ok);
    }

    #[test]
    fn transcript_without_assistant_is_nodata() {
        let s = parse_claude_transcript("sid", r#"{"type":"user","message":{}}"#);
        assert_eq!(s.state, UsageState::NoData);
        assert_eq!(s.total_tokens, 0);
    }

    #[test]
    fn transcript_path_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects/-Users-akiojin-x");
        fs::create_dir_all(&proj).unwrap();
        let file = proj.join("sid-123.jsonl");
        fs::write(&file, r#"{"type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":1,"output_tokens":1}}}"#).unwrap();
        let found = transcript_for_session(dir.path(), "sid-123").unwrap();
        assert_eq!(found, file);
        let s = read_claude_session(dir.path(), "sid-123").unwrap();
        assert_eq!(s.input_tokens, 1);
    }
}
