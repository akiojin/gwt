//! Poll-owned incremental usage summaries; transcript text is never retained.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{
    claude, codex,
    consumption::{self, ConsumptionEvent, ProviderConsumption},
    incremental::JsonlCache,
    model_context, ProviderUsage, SessionUsage, UsageProvider, UsageState,
};

struct CodexTranscript {
    session: SessionUsage,
    account: Option<(codex::CodexAccount, Option<DateTime<Utc>>)>,
    events: Vec<ConsumptionEvent>,
    last_event_total: Option<Value>,
    observed_at: Option<DateTime<Utc>>,
    incomplete: bool,
}

impl Default for CodexTranscript {
    fn default() -> Self {
        Self {
            session: codex::parse_codex_session("", ""),
            account: None,
            events: Vec::new(),
            last_event_total: None,
            observed_at: None,
            incomplete: false,
        }
    }
}

impl CodexTranscript {
    fn apply(&mut self, line: &str) {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let payload = obj.get("payload").unwrap_or(&obj);
        let model = obj
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| payload.get("model").and_then(Value::as_str))
            .map(str::to_owned);
        if payload.get("type").and_then(Value::as_str) == Some("token_count") {
            let previous_model = self.session.model.take();
            self.session = codex::parse_codex_session("", line);
            self.session.model = previous_model;
            let observed_at = obj
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                .map(|ts| ts.to_utc());
            self.account = payload
                .get("rate_limits")
                .and_then(|limits| codex::parse_rate_limits_at(limits, observed_at))
                .map(|account| (account, observed_at));
            let info = payload.get("info");
            let total = info.and_then(|info| info.get("total_token_usage"));
            let last = info.and_then(|info| info.get("last_token_usage"));
            if total.is_some() || last.is_some() {
                self.observed_at = observed_at.or(self.observed_at);
                let duplicate = total.is_some() && total == self.last_event_total.as_ref();
                if !duplicate {
                    let events = consumption::parse_codex_events(line);
                    self.incomplete |= events.is_empty();
                    if !events.is_empty() {
                        self.last_event_total = total.cloned();
                        self.events.extend(events);
                    }
                }
            }
        }
        if model.is_some() {
            self.session.model = model;
        }
    }
}

struct ClaudeTranscript {
    session: SessionUsage,
    events: Vec<ConsumptionEvent>,
    observed_at: Option<DateTime<Utc>>,
    incomplete: bool,
}

impl Default for ClaudeTranscript {
    fn default() -> Self {
        Self {
            session: claude::parse_claude_transcript("", ""),
            events: Vec::new(),
            observed_at: None,
            incomplete: false,
        }
    }
}

impl ClaudeTranscript {
    fn apply(&mut self, line: &str) {
        let next = claude::parse_claude_transcript("", line);
        if next.state != UsageState::Ok {
            return;
        }
        let session = &mut self.session;
        let first = session.state != UsageState::Ok;
        session.state = UsageState::Ok;
        session.input_tokens += next.input_tokens;
        session.output_tokens += next.output_tokens;
        session.total_tokens += next.total_tokens;
        session.cache_read_tokens = if first {
            next.cache_read_tokens
        } else {
            session
                .cache_read_tokens
                .zip(next.cache_read_tokens)
                .map(|(a, b)| a + b)
        };
        session.cache_creation_tokens = if first {
            next.cache_creation_tokens
        } else {
            session
                .cache_creation_tokens
                .zip(next.cache_creation_tokens)
                .map(|(a, b)| a + b)
        };
        if next.model.is_some() {
            session.model = next.model;
        }
        if next.context_used_tokens.is_some() {
            session.context_used_tokens = next.context_used_tokens;
        }
        session.context_limit_tokens = session
            .model
            .as_deref()
            .and_then(model_context::context_limit);
        session.context_left_pct = SessionUsage::context_left_from(
            session.context_used_tokens,
            session.context_limit_tokens,
        );
        let events = consumption::parse_claude_events(line);
        self.incomplete |= events.is_empty();
        if let Some((at, _)) = events.last() {
            self.observed_at = Some(*at);
        }
        self.events.extend(events);
    }
}

/// A single poller's summaries, shared by account, session and consumption reads.
/// Directory indexes are rebuilt once per poll so newly created files are visible.
#[derive(Default)]
pub struct UsageCache {
    codex: JsonlCache<CodexTranscript>,
    claude: JsonlCache<ClaudeTranscript>,
    codex_paths: HashMap<PathBuf, Vec<PathBuf>>,
    claude_paths: HashMap<PathBuf, Vec<PathBuf>>,
    active_paths: HashSet<PathBuf>,
    consumption_paths: HashMap<UsageProvider, HashSet<PathBuf>>,
}

impl UsageCache {
    pub fn begin_poll(&mut self) {
        self.codex_paths.clear();
        self.claude_paths.clear();
        self.active_paths.clear();
    }

    /// Keep current session/account files and the most recent consumption scan.
    /// The latter runs less often, so its offsets survive intervening ticks.
    pub fn end_poll(&mut self) {
        let mut keep = self.active_paths.clone();
        for paths in self.consumption_paths.values() {
            keep.extend(paths.iter().cloned());
        }
        self.codex.retain(|path| keep.contains(path));
        self.claude.retain(|path| keep.contains(path));
    }

    fn paths(&mut self, home: &Path, provider: UsageProvider) -> &[PathBuf] {
        match provider {
            UsageProvider::Codex => self
                .codex_paths
                .entry(home.to_owned())
                .or_insert_with(|| codex::recent_rollouts(home, usize::MAX)),
            UsageProvider::ClaudeCode => {
                self.claude_paths.entry(home.to_owned()).or_insert_with(|| {
                    claude::transcripts_modified_since(home, SystemTime::UNIX_EPOCH, usize::MAX)
                })
            }
        }
    }

    pub fn codex_session(&mut self, home: &Path, session_id: &str) -> Option<SessionUsage> {
        let path = self
            .paths(home, UsageProvider::Codex)
            .iter()
            .find(|path| codex::rollout_matches_session(path, session_id))?
            .clone();
        self.active_paths.insert(path.clone());
        let mut usage = self
            .codex
            .read(&path, CodexTranscript::apply)?
            .session
            .clone();
        usage.session_id = session_id.to_owned();
        usage.provider_session_id = Some(session_id.to_owned());
        Some(usage)
    }

    pub fn codex_session_at(
        &mut self,
        home: &Path,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Option<SessionUsage> {
        let mut usage = self.codex_session(home, session_id)?;
        let path = self
            .paths(home, UsageProvider::Codex)
            .iter()
            .find(|path| codex::rollout_matches_session(path, session_id))?
            .clone();
        let summary = self.codex.read(&path, CodexTranscript::apply)?;
        if usage.state == UsageState::Ok || !summary.events.is_empty() {
            usage.telemetry = Some(consumption::session_telemetry(
                UsageProvider::Codex,
                &summary.events,
                summary.observed_at,
                summary.incomplete,
                now,
            ));
        }
        Some(usage)
    }

    pub fn claude_session(&mut self, home: &Path, session_id: &str) -> Option<SessionUsage> {
        let filename = format!("{session_id}.jsonl");
        let path = self
            .paths(home, UsageProvider::ClaudeCode)
            .iter()
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name == filename.as_str())
            })?
            .clone();
        self.active_paths.insert(path.clone());
        let mut usage = self
            .claude
            .read(&path, ClaudeTranscript::apply)?
            .session
            .clone();
        usage.session_id = session_id.to_owned();
        usage.provider_session_id = Some(session_id.to_owned());
        Some(usage)
    }

    pub fn claude_session_at(
        &mut self,
        home: &Path,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Option<SessionUsage> {
        let mut usage = self.claude_session(home, session_id)?;
        let filename = format!("{session_id}.jsonl");
        let path = self
            .paths(home, UsageProvider::ClaudeCode)
            .iter()
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name == filename.as_str())
            })?
            .clone();
        let summary = self.claude.read(&path, ClaudeTranscript::apply)?;
        if usage.state == UsageState::Ok {
            usage.telemetry = Some(consumption::session_telemetry(
                UsageProvider::ClaudeCode,
                &summary.events,
                summary.observed_at,
                summary.incomplete,
                now,
            ));
        }
        Some(usage)
    }

    pub fn codex_account_since(
        &mut self,
        home: &Path,
        now: DateTime<Utc>,
        authenticated_at: Option<DateTime<Utc>>,
    ) -> ProviderUsage {
        codex::read_codex_account_with(home, now, authenticated_at, || {
            let paths: Vec<_> = self
                .paths(home, UsageProvider::Codex)
                .iter()
                .take(8)
                .cloned()
                .collect();
            paths
                .iter()
                .filter_map(|path| {
                    self.active_paths.insert(path.clone());
                    self.codex
                        .read(path, CodexTranscript::apply)?
                        .account
                        .clone()
                })
                .collect()
        })
    }

    pub fn codex_consumption(
        &mut self,
        home: &Path,
        week_start: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> ProviderConsumption {
        self.consumption(home, UsageProvider::Codex, week_start, now)
    }

    pub fn claude_consumption(
        &mut self,
        home: &Path,
        week_start: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> ProviderConsumption {
        self.consumption(home, UsageProvider::ClaudeCode, week_start, now)
    }

    fn consumption(
        &mut self,
        home: &Path,
        provider: UsageProvider,
        week_start: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> ProviderConsumption {
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(
                consumption::RANGE_DAYS as u64 * 24 * 60 * 60,
            ))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let paths: Vec<_> = self
            .paths(home, provider)
            .iter()
            .filter(|path| {
                path.metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|mtime| mtime >= cutoff)
            })
            .take(consumption::MAX_FILES)
            .cloned()
            .collect();
        self.consumption_paths
            .insert(provider, paths.iter().cloned().collect());
        let mut events = Vec::new();
        for path in paths {
            match provider {
                UsageProvider::Codex => {
                    if let Some(summary) = self.codex.read(&path, CodexTranscript::apply) {
                        events.extend_from_slice(&summary.events);
                    }
                }
                UsageProvider::ClaudeCode => {
                    if let Some(summary) = self.claude.read(&path, ClaudeTranscript::apply) {
                        events.extend_from_slice(&summary.events);
                    }
                }
            }
        }
        consumption::aggregate(provider, events, week_start, now)
    }

    pub fn bytes_read(&self) -> u64 {
        self.codex.bytes_read() + self.claude.bytes_read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{claude, codex, consumption};
    use std::{fs, io::Write};

    #[test]
    fn session_telemetry_keeps_thirty_minutes_and_does_not_mix_transcripts() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("sessions")).unwrap();
        let line = |timestamp: &str,
                    (total_input, total_cached, total_output),
                    (input, cached, output)| {
            serde_json::json!({"timestamp": timestamp, "payload": {"type": "token_count", "info": {
                "total_token_usage": {"input_tokens": total_input, "cached_input_tokens": total_cached, "output_tokens": total_output},
                "last_token_usage": {"input_tokens": input, "cached_input_tokens": cached, "output_tokens": output}
            }}}).to_string() + "\n"
        };
        fs::write(
            home.path().join("sessions/rollout-owned.jsonl"),
            line("2026-10-15T12:00:59.999Z", (10, 4, 2), (10, 4, 2))
                + &line("2026-10-15T12:01:00Z", (30, 9, 5), (20, 5, 3))
                + &line("2026-10-15T12:28:00Z", (60, 15, 9), (30, 6, 4))
                + &line("2026-10-15T12:28:00Z", (60, 15, 9), (30, 6, 4)),
        )
        .unwrap();
        fs::write(
            home.path().join("sessions/rollout-other.jsonl"),
            line(
                "2026-10-15T12:29:00Z",
                (9000, 8000, 1000),
                (9000, 8000, 1000),
            ),
        )
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-15T12:30:30Z")
            .unwrap()
            .to_utc();
        let mut cache = UsageCache::default();
        let usage = cache.codex_session_at(home.path(), "owned", now).unwrap();
        let telemetry = usage.telemetry.unwrap();
        assert_eq!(telemetry.minute_buckets.len(), 30);
        assert_eq!(telemetry.minute_buckets[0].tokens, Some(23));
        assert_eq!(telemetry.minute_buckets[26].tokens, Some(0));
        assert_eq!(telemetry.minute_buckets[27].tokens, Some(34));
        assert_eq!(telemetry.today_tokens, Some(69));
        assert_eq!(telemetry.today_input_tokens, Some(60));
        assert_eq!(telemetry.today_cache_read_tokens, Some(15));
        assert_eq!(telemetry.today_output_tokens, Some(9));
        assert_eq!(
            telemetry.observed_at,
            Some(now - chrono::Duration::seconds(150))
        );
        assert_eq!(telemetry.source, "codex_rollout");
        let bytes = cache.bytes_read();
        cache.begin_poll();
        assert!(cache.codex_session_at(home.path(), "owned", now).is_some());
        assert_eq!(cache.bytes_read(), bytes);
    }

    #[test]
    fn session_telemetry_distinguishes_unobserved_minutes_from_measured_zero() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("projects/project")).unwrap();
        let path = home.path().join("projects/project/session.jsonl");
        fs::write(&path, concat!(
            r#"{"timestamp":"2026-10-15T12:29:00Z","type":"assistant","message":{"usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
            "\n"
        )).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-15T12:30:30Z")
            .unwrap()
            .to_utc();
        let mut cache = UsageCache::default();
        let usage = cache
            .claude_session_at(home.path(), "session", now)
            .unwrap();
        let telemetry = usage.telemetry.unwrap();
        assert_eq!(telemetry.minute_buckets[27].tokens, None);
        assert_eq!(telemetry.minute_buckets[28].tokens, Some(0));
        assert_eq!(telemetry.today_tokens, Some(0));
        assert_eq!(usage.cache_read_tokens, Some(0));
        fs::OpenOptions::new().append(true).open(path).unwrap().write_all(
            b"{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n"
        ).unwrap();
        let usage = cache
            .claude_session_at(home.path(), "session", now)
            .unwrap();
        let telemetry = usage.telemetry.unwrap();
        assert_eq!(telemetry.quality, "partial");
        assert_eq!(telemetry.today_tokens, None);
        assert!(telemetry
            .minute_buckets
            .iter()
            .all(|bucket| bucket.tokens.is_none()));
        assert_eq!(usage.cache_read_tokens, None);
    }

    #[test]
    fn cached_and_stateless_session_readers_match_the_exact_rollout_id() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        let exact = sessions.join("rollout-older-short-id.jsonl");
        let distractor = sessions.join("rollout-newer-short-id-extra.jsonl");
        let text = r#"{"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100}}}}"#;
        fs::write(&exact, text).unwrap();
        fs::write(&distractor, text.replace("100", "900")).unwrap();
        for (path, seconds) in [(&exact, 1), (&distractor, 2)] {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new().set_modified(
                        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
                    ),
                )
                .unwrap();
        }
        let expected = Some(codex::parse_codex_session("short-id", text));
        let mut cache = UsageCache::default();
        assert_eq!(cache.codex_session(home.path(), "short-id"), expected);
        assert_eq!(codex::read_codex_session(home.path(), "short-id"), expected);
        fs::remove_file(exact).unwrap();
        cache.begin_poll();
        assert!(cache.codex_session(home.path(), "short-id").is_none());
        assert!(codex::read_codex_session(home.path(), "short-id").is_none());
    }

    #[test]
    fn cached_transcripts_match_full_read_and_only_read_appended_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join("codex");
        let claude_home = temp.path().join("claude");
        fs::create_dir_all(codex_home.join("sessions")).unwrap();
        fs::create_dir_all(claude_home.join("projects/project")).unwrap();
        let cx = codex_home.join("sessions/rollout-test-cx.jsonl");
        let cc = claude_home.join("projects/project/cc.jsonl");
        let cx_first = concat!(
            "{\"payload\":{\"model\":\"gpt-5\"}}\n",
            "{\"timestamp\":\"2026-10-15T00:00:00Z\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":100,\"output_tokens\":20},\"last_token_usage\":{\"input_tokens\":10,\"output_tokens\":2},\"model_context_window\":1000},\"rate_limits\":{\"primary\":{\"used_percent\":12,\"window_minutes\":300,\"resets_in_seconds\":60}}}}\n"
        );
        let cc_first = "{\"timestamp\":\"2026-10-15T00:00:00Z\",\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"cache_read_input_tokens\":5}}}\n";
        fs::write(&cx, cx_first).unwrap();
        fs::write(&cc, cc_first).unwrap();
        fs::write(
            codex_home.join("auth.json"),
            r#"{"tokens":{"account_id":"fixture-account"}}"#,
        )
        .unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-15T01:00:00Z")
            .unwrap()
            .to_utc();
        let boundary = Some(now - chrono::Duration::days(1));
        let mut cache = UsageCache::default();
        assert_eq!(
            cache.codex_session(&codex_home, "cx"),
            codex::read_codex_session(&codex_home, "cx")
        );
        assert_eq!(
            cache.claude_session(&claude_home, "cc"),
            claude::read_claude_session(&claude_home, "cc")
        );
        let bytes = cache.bytes_read();
        assert_eq!(
            cache.codex_account_since(&codex_home, now, boundary),
            codex::read_codex_account_since(&codex_home, now, boundary)
        );
        cache.end_poll();
        cache.begin_poll();
        assert_eq!(
            cache.codex_session(&codex_home, "cx"),
            codex::read_codex_session(&codex_home, "cx")
        );
        assert_eq!(
            cache.claude_session(&claude_home, "cc"),
            claude::read_claude_session(&claude_home, "cc")
        );
        assert_eq!(
            cache.bytes_read(),
            bytes,
            "unchanged poll reads no transcript bytes"
        );
        let cx_tail = "{\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":200,\"output_tokens\":30}}}}\n";
        let cc_tail = "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":20,\"output_tokens\":3}}}\n";
        fs::OpenOptions::new()
            .append(true)
            .open(&cx)
            .unwrap()
            .write_all(cx_tail.as_bytes())
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&cc)
            .unwrap()
            .write_all(cc_tail.as_bytes())
            .unwrap();
        cache.begin_poll();
        assert_eq!(
            cache.codex_session(&codex_home, "cx"),
            codex::read_codex_session(&codex_home, "cx")
        );
        assert_eq!(
            cache.claude_session(&claude_home, "cc"),
            claude::read_claude_session(&claude_home, "cc")
        );
        assert_eq!(
            cache.bytes_read() - bytes,
            (cx_tail.len() + cc_tail.len()) as u64
        );
        assert_eq!(
            cache.codex_account_since(&codex_home, now, boundary),
            codex::read_codex_account_since(&codex_home, now, boundary)
        );
        assert_eq!(
            cache.codex_account_since(&codex_home, now, Some(now)),
            codex::read_codex_account_since(&codex_home, now, Some(now))
        );
        let week = now - chrono::Duration::days(7);
        assert_eq!(
            cache.codex_consumption(&codex_home, week, now),
            consumption::read_codex_consumption(&codex_home, week, now)
        );
        assert_eq!(
            cache.claude_consumption(&claude_home, week, now),
            consumption::read_claude_consumption(&claude_home, week, now)
        );
        assert_eq!(
            cache.bytes_read() - bytes,
            (cx_tail.len() + cc_tail.len()) as u64,
            "consumption reuses session summaries"
        );
        fs::write(&cc, cc_first).unwrap();
        cache.begin_poll();
        assert_eq!(
            cache.claude_session(&claude_home, "cc"),
            claude::read_claude_session(&claude_home, "cc")
        );
        fs::remove_file(&cx).unwrap();
        cache.begin_poll();
        assert!(cache.codex_session(&codex_home, "cx").is_none());
    }
}
