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
    pub account_label: Option<String>,
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
    let account_label = ["email", "name"].into_iter().find_map(|key| {
        oauth
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    Some(ClaudeCreds {
        access_token,
        subscription_type,
        account_label,
    })
}

fn creds_from_file(home: &Path) -> Option<ClaudeCreds> {
    let text = fs::read_to_string(home.join(".credentials.json")).ok()?;
    parse_creds_json(&text)
}

#[cfg(target_os = "macos")]
fn creds_from_keychain() -> Option<ClaudeCreds> {
    let out = crate::process::hidden_command("security")
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
pub fn claude_user_agent() -> Result<String, String> {
    let request = crate::process::ProcessPlanRequest::new("claude").arg("--version");
    let mut command =
        crate::process::resolved_command(request).map_err(|error| error.to_string())?;
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("claude --version exited with {}", output.status));
    }
    let raw = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    claude_user_agent_from_version_text(&raw)
}

fn claude_user_agent_from_version_text(raw: &str) -> Result<String, String> {
    let token = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| "claude --version returned empty output".to_string())?;
    let token = token.strip_prefix('v').unwrap_or(token);
    let version = semver::Version::parse(token)
        .map_err(|_| format!("claude --version returned invalid semver: {token}"))?;
    Ok(format!("claude-code/{version}"))
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
    let account_label = creds.account_label.clone();
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
                    account_label,
                    plan,
                    windows: acc.windows,
                    limit_reached: acc.limit_reached,
                    state: UsageState::Ok,
                    fetched_at: Some(now),
                },
                None => ProviderUsage {
                    provider: UsageProvider::ClaudeCode,
                    account_label,
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
            account_label,
            plan,
            windows: Vec::new(),
            limit_reached: false,
            state: map_status_to_state(r.status().as_u16()),
            fetched_at: Some(now),
        },
        Err(_) => ProviderUsage {
            provider: UsageProvider::ClaudeCode,
            account_label,
            plan,
            windows: Vec::new(),
            limit_reached: false,
            state: UsageState::Unavailable {
                reason: "request failed".to_string(),
            },
            fetched_at: Some(now),
        },
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

    #[test]
    fn user_agent_requires_a_valid_claude_semver() {
        assert_eq!(
            claude_user_agent_from_version_text("2.1.159 (Claude Code)"),
            Ok("claude-code/2.1.159".to_string())
        );
        assert!(claude_user_agent_from_version_text("").is_err());
        assert!(claude_user_agent_from_version_text(
            "echo Native binary not installed. Run the package postinstall."
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn live_user_agent_probe_resolves_real_bun_global_placeholder_fixture() {
        let _env = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = crate::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
            .expect("create real Windows Bun fixture");
        let _path = crate::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = crate::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = crate::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

        assert_eq!(claude_user_agent(), Ok("claude-code/2.1.210".to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn live_user_agent_probe_rejects_real_bun_global_placeholder_fixture_without_safe_target() {
        let _env = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = crate::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
            .expect("create real Windows Bun fixture");
        fixture
            .remove_safe_targets()
            .expect("remove safe redirect targets");
        let _path = crate::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = crate::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = crate::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

        let error = claude_user_agent().expect_err("unsafe placeholder must be rejected");
        assert!(error.contains("native-binary placeholder"));
    }

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
        let body = r#"{"claudeAiOauth":{"accessToken":"tok123","refreshToken":"r","expiresAt":1,"subscriptionType":"max","email":"claude@example.com"}}"#;
        let c = parse_creds_json(body).unwrap();
        assert_eq!(c.access_token, "tok123");
        assert_eq!(c.subscription_type.as_deref(), Some("max"));
        assert_eq!(c.account_label.as_deref(), Some("claude@example.com"));
    }

    #[test]
    fn creds_account_label_falls_back_to_name() {
        let body = r#"{"claudeAiOauth":{"accessToken":"tok123","name":"Claude User"}}"#;
        let c = parse_creds_json(body).unwrap();
        assert_eq!(c.account_label.as_deref(), Some("Claude User"));
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

    #[test]
    fn transcript_for_session_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        // No `projects/` dir at all → None (read_dir fails).
        assert!(transcript_for_session(dir.path(), "missing").is_none());
        // `projects/` exists but holds no matching transcript → None.
        fs::create_dir_all(dir.path().join("projects/x")).unwrap();
        assert!(transcript_for_session(dir.path(), "missing").is_none());
        assert!(read_claude_session(dir.path(), "missing").is_none());
    }

    #[test]
    fn transcripts_modified_since_orders_newest_first_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("projects/a");
        let p2 = dir.path().join("projects/b");
        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();
        fs::write(p1.join("old.jsonl"), "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(p2.join("new.jsonl"), "{}").unwrap();
        // Non-jsonl files are ignored.
        fs::write(p1.join("note.txt"), "x").unwrap();
        let cutoff = std::time::SystemTime::UNIX_EPOCH;
        let found = transcripts_modified_since(dir.path(), cutoff, 10);
        assert_eq!(found.len(), 2);
        assert!(found[0].ends_with("new.jsonl"));
        // `limit` caps the result and keeps the newest.
        let one = transcripts_modified_since(dir.path(), cutoff, 1);
        assert_eq!(one.len(), 1);
        assert!(one[0].ends_with("new.jsonl"));
        // A cutoff in the far future excludes everything.
        let far = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        assert!(transcripts_modified_since(dir.path(), far, 10).is_empty());
    }

    #[test]
    fn creds_from_file_reads_credentials_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"abc","subscriptionType":"pro"}}"#,
        )
        .unwrap();
        let c = creds_from_file(dir.path()).unwrap();
        assert_eq!(c.access_token, "abc");
        assert_eq!(c.subscription_type.as_deref(), Some("pro"));
        // Missing file → None.
        let empty = tempfile::tempdir().unwrap();
        assert!(creds_from_file(empty.path()).is_none());
    }

    #[test]
    fn parse_creds_json_accepts_unwrapped_shape() {
        // No `claudeAiOauth` wrapper → falls back to the top-level object.
        let c = parse_creds_json(r#"{"accessToken":"flat","subscriptionType":"team"}"#).unwrap();
        assert_eq!(c.access_token, "flat");
        assert_eq!(c.subscription_type.as_deref(), Some("team"));
    }

    #[test]
    fn parse_oauth_usage_skips_window_missing_utilization() {
        // `five_hour` is present but has no `utilization` → skipped; only the
        // valid `seven_day` window survives.
        let body =
            r#"{"five_hour":{"resets_at":"2026-06-02T16:10:00Z"},"seven_day":{"utilization":12}}"#;
        let acc = parse_oauth_usage(body, now()).unwrap();
        assert_eq!(acc.windows.len(), 1);
        assert_eq!(acc.windows[0].kind, WindowKind::Weekly);
    }
}
