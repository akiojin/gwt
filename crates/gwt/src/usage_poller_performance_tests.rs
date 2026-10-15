//! Opt-in synthetic before/after measurements for Issue #5503 AC-3.

use super::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use gwt_core::{
    test_support::{ScopedEnvVar, ScopedGwtHome},
    usage::consumption,
};

const LEDGER_ROWS: usize = 2646;
const ACTIVE_ROWS: usize = 32;
const REPEATS: usize = 3;

#[test]
#[ignore = "synthetic usage-poll wall/collecting-thread CPU benchmark"]
fn usage_poll_before_after_wall_and_thread_cpu() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || run_measurements(&handle))
            .await
            .unwrap();
    });
}

fn run_measurements(runtime: &tokio::runtime::Handle) {
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temporary = tempfile::tempdir().unwrap();
    // The override is thread-local: set it on the collecting worker itself.
    let _home = ScopedGwtHome::set(temporary.path());
    let codex_home = temporary.path().join("codex");
    let claude_home = temporary.path().join("claude");
    let _codex = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let _claude = ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &claude_home);
    let mut settings = Settings::default();
    settings.usage.codex_enabled = true;
    settings.usage.claude_account_enabled = false;
    settings.save_global().unwrap();
    let mut fixture = Fixture::create(temporary.path(), codex_home, claude_home);
    println!(
        "usage_poll_fixture ledger_rows={LEDGER_ROWS} active_rows={ACTIVE_ROWS} transcript_bytes={} repeats={REPEATS} cpu_scope=collecting_thread",
        fixture.transcript_bytes
    );

    let mut baseline = Baseline::default();
    let expected = sorted(baseline.poll(&fixture, true).0);
    assert_eq!(expected.sessions.len(), ACTIVE_ROWS);
    // Prime the shared ledger cache before either warm path is measured.
    measure("baseline_warm", &expected, || {
        baseline.poll(&fixture, false)
    });
    measure("baseline_forced", &expected, || {
        baseline.poll(&fixture, true)
    });

    let mut poller = fixture.poller();
    measure("new_cold", &expected, || {
        poller = fixture.poller();
        incremental_poll(runtime, &mut poller, fixture.now, false)
    });
    let warm_bytes = measure("new_warm", &expected, || {
        incremental_poll(runtime, &mut poller, fixture.now, false)
    });
    assert_eq!(
        warm_bytes, 0,
        "unchanged polls must read no transcript text"
    );
    let forced_bytes = measure("new_forced", &expected, || {
        incremental_poll(runtime, &mut poller, fixture.now, true)
    });
    assert_eq!(
        forced_bytes, 0,
        "forced scans must reuse unchanged summaries"
    );

    let appended_bytes = fixture.append_usage();
    let after_append = sorted(baseline.poll(&fixture, true).0);
    let actual_bytes = measure("new_appended", &after_append, || {
        incremental_poll(runtime, &mut poller, fixture.now, true)
    });
    assert_eq!(
        actual_bytes, appended_bytes,
        "the appended poll reads exactly the two new provider records"
    );
}

fn incremental_poll(
    runtime: &tokio::runtime::Handle,
    poller: &mut Poller,
    now: DateTime<Utc>,
    force: bool,
) -> (UsageSnapshot, u64) {
    let before = poller.transcripts.bytes_read();
    let snapshot = runtime.block_on(poller.poll_once(now, force));
    (snapshot, poller.transcripts.bytes_read() - before)
}

fn sorted(mut snapshot: UsageSnapshot) -> UsageSnapshot {
    snapshot
        .sessions
        .sort_by(|left, right| left.session_id.cmp(&right.session_id));
    snapshot
}

fn measure(
    label: &str,
    expected: &UsageSnapshot,
    mut work: impl FnMut() -> (UsageSnapshot, u64),
) -> u64 {
    let mut total_wall = Duration::ZERO;
    let mut total_cpu = Duration::ZERO;
    let mut total_bytes = 0;
    for repeat in 1..=REPEATS {
        let started = Instant::now();
        let cpu_started = thread_cpu_time();
        let (snapshot, bytes) = work();
        let cpu = thread_cpu_time() - cpu_started;
        let wall = started.elapsed();
        assert_eq!(&sorted(snapshot), expected, "{label} snapshot differs");
        total_wall += wall;
        total_cpu += cpu;
        total_bytes += bytes;
        println!(
            "usage_poll {label} repeat={repeat} wall_ms={:.3} thread_cpu_ms={:.3} transcript_bytes={bytes}",
            wall.as_secs_f64() * 1000.0,
            cpu.as_secs_f64() * 1000.0,
        );
    }
    println!(
        "usage_poll {label} mean_wall_ms={:.3} mean_thread_cpu_ms={:.3} total_transcript_bytes={total_bytes}",
        total_wall.as_secs_f64() * 1000.0 / REPEATS as f64,
        total_cpu.as_secs_f64() * 1000.0 / REPEATS as f64,
    );
    total_bytes
}

struct Fixture {
    codex_home: PathBuf,
    claude_home: PathBuf,
    now: DateTime<Utc>,
    authenticated_at: DateTime<Utc>,
    account_id: Option<String>,
    first_codex: PathBuf,
    first_claude: PathBuf,
    transcript_bytes: u64,
    account_bytes: u64,
}

impl Fixture {
    fn create(root: &Path, codex_home: PathBuf, claude_home: PathBuf) -> Self {
        let now = Utc::now();
        fs::create_dir_all(codex_home.join("sessions")).unwrap();
        fs::create_dir_all(claude_home.join("projects/benchmark")).unwrap();
        fs::write(
            codex_home.join("auth.json"),
            r#"{"tokens":{"account_id":"synthetic-benchmark-account"}}"#,
        )
        .unwrap();
        let codex_text = transcript(UsageProvider::Codex, now);
        let claude_text = transcript(UsageProvider::ClaudeCode, now);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&sessions).unwrap();
        let mut transcript_bytes = 0;
        let template = gwt_agent::Session::new(root, "work/benchmark", AgentId::Codex);
        for index in 0..LEDGER_ROWS {
            let codex_provider = index % 2 == 0;
            let agent = if codex_provider {
                AgentId::Codex
            } else {
                AgentId::ClaudeCode
            };
            let mut session = template.clone();
            session.agent_id = agent;
            session.id = format!("benchmark-{index:04}");
            session.status = if index < ACTIVE_ROWS {
                AgentStatus::Running
            } else {
                AgentStatus::Stopped
            };
            if index < ACTIVE_ROWS {
                let sid = format!("{}-{index:02}", if codex_provider { "cx" } else { "cc" });
                let (path, text) = if codex_provider {
                    (
                        codex_home.join(format!("sessions/rollout-{sid}.jsonl")),
                        &codex_text,
                    )
                } else {
                    (
                        claude_home.join(format!("projects/benchmark/{sid}.jsonl")),
                        &claude_text,
                    )
                };
                fs::write(path, text).unwrap();
                transcript_bytes += text.len() as u64;
                session.agent_session_id = Some(sid);
            }
            session.save(&sessions).unwrap();
        }
        let account_bytes = account_source_bytes(&codex_home);
        Self {
            account_id: codex::read_auth_account_identity(&codex_home),
            first_codex: codex_home.join("sessions/rollout-cx-00.jsonl"),
            first_claude: claude_home.join("projects/benchmark/cc-01.jsonl"),
            codex_home,
            claude_home,
            now,
            authenticated_at: now - chrono::Duration::seconds(60),
            transcript_bytes,
            account_bytes,
        }
    }

    fn poller(&self) -> Poller {
        Poller {
            codex_auth_observed: true,
            codex_account_id: self.account_id.clone(),
            codex_authenticated_at: Some(self.authenticated_at),
            ..Default::default()
        }
    }

    fn append_usage(&mut self) -> u64 {
        let codex = usage_line(UsageProvider::Codex, self.now, true);
        let claude = usage_line(UsageProvider::ClaudeCode, self.now, true);
        for (path, line) in [(&self.first_codex, &codex), (&self.first_claude, &claude)] {
            fs::OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(line.as_bytes())
                .unwrap();
        }
        let appended = (codex.len() + claude.len()) as u64;
        self.transcript_bytes += appended;
        self.account_bytes = account_source_bytes(&self.codex_home);
        appended
    }
}

fn account_source_bytes(home: &Path) -> u64 {
    codex::recent_rollouts(home, 8)
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .sum()
}

fn transcript(provider: UsageProvider, now: DateTime<Utc>) -> String {
    let padding = format!(
        "{}\n",
        serde_json::json!({"type":"message", "text":"x".repeat(4096)})
    );
    let usage = usage_line(provider, now, false);
    let mut text = String::new();
    let mut lines = 0;
    while text.len() < 1024 * 1024 {
        text.push_str(&padding);
        if lines % 32 == 0 {
            text.push_str(&usage);
        }
        lines += 1;
    }
    text
}

fn usage_line(provider: UsageProvider, now: DateTime<Utc>, appended: bool) -> String {
    let value = match provider {
        UsageProvider::Codex => serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "payload": {
                "type":"token_count", "model":"gpt-5",
                "info": {
                    "total_token_usage":{"input_tokens": if appended {2000} else {1000}, "output_tokens":100},
                    "last_token_usage":{"input_tokens":20,"output_tokens":10,"cached_input_tokens":2},
                    "model_context_window":200000
                },
                "rate_limits":{"primary":{"used_percent":10,"window_minutes":300,"resets_at":now.timestamp()+3600}}
            }
        }),
        UsageProvider::ClaudeCode => serde_json::json!({
            "timestamp":now.to_rfc3339(), "type":"assistant",
            "message":{"model":"claude-sonnet-4","usage":{"input_tokens":if appended {40} else {20},"output_tokens":10,"cache_read_input_tokens":5}}
        }),
    };
    format!("{value}\n")
}

#[derive(Default)]
struct Baseline {
    consumption: Option<Vec<ProviderConsumption>>,
}

impl Baseline {
    /// The previous pipeline owns/clones ledger rows and rereads full files.
    /// Byte totals are fixture lengths for each invoked full transcript reader;
    /// auth, settings and ledger bytes are excluded from both byte metrics.
    fn poll(&mut self, fixture: &Fixture, force: bool) -> (UsageSnapshot, u64) {
        let config = Settings::load().unwrap().usage;
        let _identity = codex::read_auth_account_identity(&fixture.codex_home);
        let accounts = vec![
            codex::read_codex_account_since(
                &fixture.codex_home,
                fixture.now,
                Some(fixture.authenticated_at),
            ),
            ProviderUsage::degraded(UsageProvider::ClaudeCode, UsageState::Disabled),
        ];
        let mut bytes = fixture.account_bytes + fixture.transcript_bytes;
        if force || self.consumption.is_none() {
            self.consumption = Some(vec![
                consumption::read_codex_consumption(
                    &fixture.codex_home,
                    week_start_for(&accounts, UsageProvider::Codex, fixture.now),
                    fixture.now,
                ),
                consumption::read_claude_consumption(
                    &fixture.claude_home,
                    week_start_for(&accounts, UsageProvider::ClaudeCode, fixture.now),
                    fixture.now,
                ),
            ]);
            bytes += fixture.transcript_bytes;
        }
        let dir = gwt_core::paths::gwt_sessions_dir();
        let sessions = crate::session_ledger_cache::SessionLedgerCache::new().load(&dir);
        let mut out = Vec::new();
        for session in sessions {
            if !is_active_status(session.status) {
                continue;
            }
            let Some(provider) = session_provider(&session.agent_id) else {
                continue;
            };
            let Some(sid) = session.agent_session_id.as_deref() else {
                continue;
            };
            let usage = match provider {
                UsageProvider::Codex if config.codex_enabled => {
                    codex::read_codex_session(&fixture.codex_home, sid)
                }
                UsageProvider::ClaudeCode => claude::read_claude_session(&fixture.claude_home, sid),
                _ => None,
            };
            if let Some(mut usage) = usage {
                usage.session_id = session.id;
                usage.eligible = session.backend_id.is_none();
                out.push(usage);
            }
        }
        (
            UsageSnapshot {
                accounts,
                sessions: out,
                consumption: self.consumption.clone().unwrap(),
            },
            bytes,
        )
    }
}

#[cfg(windows)]
fn thread_cpu_time() -> Duration {
    use windows::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: all outputs are valid and this current-thread pseudo handle is borrowed.
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .unwrap();
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    // test-hygiene: allow-short-duration converts measured CPU ticks, not a wall-clock wait
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

#[cfg(unix)]
fn thread_cpu_time() -> Duration {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is valid output storage for the calling thread's CPU clock.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) },
        0
    );
    Duration::new(
        u64::try_from(time.tv_sec).unwrap(),
        u32::try_from(time.tv_nsec).unwrap(),
    )
}
