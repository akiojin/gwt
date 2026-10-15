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
}

impl Default for CodexTranscript {
    fn default() -> Self {
        Self {
            session: codex::parse_codex_session("", ""),
            account: None,
            events: Vec::new(),
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
            self.events.extend(consumption::parse_codex_events(line));
        }
        if model.is_some() {
            self.session.model = model;
        }
    }
}

struct ClaudeTranscript {
    session: SessionUsage,
    events: Vec<ConsumptionEvent>,
}

impl Default for ClaudeTranscript {
    fn default() -> Self {
        Self {
            session: claude::parse_claude_transcript("", ""),
            events: Vec::new(),
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
        session.state = UsageState::Ok;
        session.input_tokens += next.input_tokens;
        session.output_tokens += next.output_tokens;
        session.total_tokens += next.total_tokens;
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
        self.events.extend(consumption::parse_claude_events(line));
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
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(session_id))
            })?
            .clone();
        self.active_paths.insert(path.clone());
        let mut usage = self
            .codex
            .read(&path, CodexTranscript::apply)?
            .session
            .clone();
        usage.session_id = session_id.to_owned();
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
