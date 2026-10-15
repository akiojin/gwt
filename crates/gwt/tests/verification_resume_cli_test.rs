//! Issue #5521: real admission companions resume three deferred callers.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use gwt_core::index_coordinator::{verification_coordinator_root_from, IndexCoordinator};
use gwt_core::process::hidden_command;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

struct Arena {
    _root: tempfile::TempDir,
    home: PathBuf,
    release: PathBuf,
    ready: PathBuf,
}

impl Arena {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(home.join(".gwt")).unwrap();
        fs::write(
            home.join(".gwt/config.toml"),
            "[verification]\nslots=3\ndisk_budget_bytes=0\n[build_artifact_gc]\nbelow_bytes=0\nbelow_percent=0\n",
        )
        .unwrap();
        Self {
            release: root.path().join("release"),
            ready: root.path().join("ready"),
            _root: root,
            home,
        }
    }

    fn repo(&self, label: &str) -> PathBuf {
        let repo = self._root.path().join(label);
        fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ],
        ] {
            let output = hidden_command("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(output.status.success(), "git fixture: {output:?}");
        }
        repo
    }

    fn spawn(&self, repo: &Path, session: &str, params: &Value, pane: Option<&str>) -> Child {
        let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
        // Every child has its own home and only the explicitly supplied launch context.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GWT_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("GWT_HOME", self.home.join(".gwt"))
            .env("GWT_SESSION_ID", session)
            .env("GWT_VERIFY_SPAWN_HOST", "inherit")
            .env("RESUME_CLI_READY", &self.ready)
            .env("RESUME_CLI_RELEASE", &self.release);
        if let Some(pane) = pane {
            let runtime = self.home.join(format!(
                ".gwt/sessions/runtime/{}/{session}.json",
                std::process::id()
            ));
            fs::create_dir_all(runtime.parent().unwrap()).unwrap();
            fs::write(&runtime, "").unwrap();
            command
                .env("GWT_SESSION_RUNTIME_PATH", runtime)
                .env("GWT_PANE_WS_URL", pane)
                .env("GWT_HOOK_FORWARD_TOKEN", format!("token-{session}"));
        }
        let mut child = command
            .current_dir(repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"schema_version": 1, "operation": "verify.run", "params": params})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        child
    }
}

struct Holder {
    child: Option<Child>,
    release: PathBuf,
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = fs::write(&self.release, "release");
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

fn collect(mut child: Child) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "gwtd did not return within its bounded admission run; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "gwtd failed: {output:?}");
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("invalid gwtd envelope: {error}; output: {output:?}"))
}

fn helper_command(name: &str) -> String {
    format!(
        "\"{}\" --ignored --exact verification_resume_cli_test::{name}",
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    )
}

#[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback's error type.
async fn own_pane(
    repo: &Path,
    session: &str,
    deliveries: mpsc::Sender<(String, String)>,
    idle: Arc<AtomicBool>,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/internal/pane-ws", listener.local_addr().unwrap());
    let session = session.to_string();
    let state = json!({"kind": "workspace_state", "workspace": {"tabs": [{
        "project_root": repo, "workspace": {"windows": [{
            "id": format!("tab::{session}"), "title": "fixture", "preset": "agent",
            "geometry": {"x": 0, "y": 0, "width": 800, "height": 600},
            "z_index": 1, "status": "idle", "session_id": session
        }]}
    }]}});
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (session, state, deliveries) = (session.clone(), state.clone(), deliveries.clone());
            let idle = Arc::clone(&idle);
            tokio::spawn(async move {
                let expected = format!("Bearer token-{session}");
                let mut socket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     response| {
                        assert_eq!(request.uri().path(), "/internal/pane-ws");
                        assert_eq!(request.headers()["authorization"], expected);
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let event: Value = serde_json::from_str(text.as_ref()).unwrap();
                    match event["kind"].as_str().unwrap() {
                        "list_windows" | "frontend_ready" => {
                            let mut state = state.clone();
                            state["workspace"]["tabs"][0]["workspace"]["windows"][0]["status"] =
                                json!(if idle.load(Ordering::Acquire) {
                                    "idle"
                                } else {
                                    "running"
                                });
                            socket
                                .send(Message::Text(state.to_string().into()))
                                .await
                                .unwrap();
                        }
                        "pane_send_input" => {
                            assert_eq!(
                                event["session_id"], session,
                                "notification must target its own Session"
                            );
                            deliveries
                                .send((
                                    session.clone(),
                                    event["text"].as_str().unwrap().to_string(),
                                ))
                                .unwrap();
                            socket.send(Message::Text(json!({"kind": "pane_send_result", "ok": true, "window_id": format!("tab::{session}")}).to_string().into())).await.unwrap();
                        }
                        kind => panic!("unexpected route {kind}; PM delivery is forbidden"),
                    }
                }
            });
        }
    });
    url
}

#[test]
fn three_deferred_agents_resume_through_their_own_panes_and_finish() {
    let arena = Arena::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let holder_repo = arena.repo("holder");
    let mut holder = Holder {
        child: Some(arena.spawn(
            &holder_repo,
            "holder",
            &json!({
                "commands": [helper_command("resume_holder_parks")], "max_wait_secs": 0
            }),
            None,
        )),
        release: arena.release.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while !arena.ready.exists() {
        assert!(
            Instant::now() < deadline,
            "holder did not acquire admission"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let (sent, received) = mpsc::channel();
    let commands = vec![
        helper_command("resume_command_passes"),
        "git --version".to_string(),
    ];
    let params = json!({"commands": commands, "headed_e2e_commands": [], "max_wait_secs": 30});
    let mut agents = BTreeMap::new();
    let mut waiters = Vec::new();
    for index in 0..3 {
        let session = format!("resume-agent-{index}");
        let repo = arena.repo(&session);
        let pane = runtime.block_on(own_pane(
            &repo,
            &session,
            sent.clone(),
            Arc::new(AtomicBool::new(true)),
        ));
        let mut deferred = params.clone();
        deferred["max_wait_secs"] = json!(0);
        waiters.push(arena.spawn(&repo, &session, &deferred, Some(&pane)));
        agents.insert(session, (repo, pane));
    }
    for waiter in waiters {
        let result = collect(waiter);
        assert_eq!(result["status"], "deferred", "{result}");
    }
    assert!(
        received.try_recv().is_err(),
        "no notification while the full pool is held"
    );
    fs::write(&arena.release, "release").unwrap();
    collect(holder.child.take().unwrap());
    let mut counts = BTreeMap::new();
    let mut retries = Vec::new();
    while counts.len() < 3 {
        let (session, text) = received
            .recv_timeout(Duration::from_secs(30))
            .expect("watchdog self-resume notification");
        let (repo, pane) = agents
            .get(&session)
            .expect("only a fixture agent may be notified");
        if !text.contains("Requested commands:") {
            assert!(text.contains("Do not start another verify.run"), "{text}");
            continue;
        }
        let count = counts.entry(session.clone()).or_insert(0);
        *count += 1;
        assert_eq!(*count, 1, "deferred notification replayed for {session}");
        assert!(
            text.contains(&serde_json::to_string(&commands).unwrap()),
            "full matrix missing: {text}"
        );
        // Only deferred capacity notifications retry; live admission waits for
        // the existing tool result instead of starting a duplicate run.
        retries.push((
            repo.clone(),
            arena.spawn(repo, &session, &params, Some(pane)),
        ));
    }
    for (repo, retry) in retries {
        collect(retry);
        let record: gwt::cli::verification_record::VerificationRunRecord = serde_json::from_slice(
            &fs::read(repo.join(gwt::cli::verification_record::VERIFICATION_RUN_STATE_RELATIVE))
                .unwrap(),
        )
        .unwrap();
        assert!(record.all_passed);
        assert_eq!(
            record
                .commands
                .iter()
                .map(|command| &command.command)
                .collect::<Vec<_>>(),
            commands.iter().collect::<Vec<_>>()
        );
    }
    assert_eq!(counts.len(), 3);
    assert!(counts.values().all(|count| *count == 1));
    for (session, text) in received.try_iter() {
        assert!(agents.contains_key(&session), "unknown Session {session}");
        assert!(
            !text.contains("Requested commands:"),
            "deferred notification replayed for {session}"
        );
        assert!(text.contains("Do not start another verify.run"), "{text}");
    }
}

#[test]
fn admitted_waiter_notifies_once_after_its_busy_pane_becomes_idle() {
    let arena = Arena::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let holder_repo = arena.repo("holder");
    let mut holder = Holder {
        child: Some(arena.spawn(
            &holder_repo,
            "holder",
            &json!({
                "commands": [helper_command("resume_holder_parks")], "max_wait_secs": 0
            }),
            None,
        )),
        release: arena.release.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while !arena.ready.exists() {
        assert!(
            Instant::now() < deadline,
            "holder did not acquire admission"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let repo = arena.repo("live-waiter");
    let idle = Arc::new(AtomicBool::new(false));
    let (sent, received) = mpsc::channel();
    let pane = runtime.block_on(own_pane(&repo, "live-waiter", sent, Arc::clone(&idle)));
    let mut waiter = arena.spawn(
        &repo,
        "live-waiter",
        &json!({"commands": [helper_command("resume_command_passes")], "max_wait_secs": 60}),
        Some(&pane),
    );
    let coordinator = IndexCoordinator::open_verification(
        verification_coordinator_root_from(&arena.home.join(".gwt")),
        3,
    )
    .unwrap();
    loop {
        // Observe an actual admission wait beyond its five-second notification
        // threshold, rather than assuming the runner started after a sleep.
        let waiting = coordinator
            .heavy_pool_status()
            .unwrap()
            .queue
            .iter()
            .any(|entry| entry.resident && entry.attempt_id.is_some() && entry.waiting_ms >= 6_000);
        if waiting {
            break;
        }
        assert!(Instant::now() < deadline, "waiter did not enter admission");
        assert!(waiter.try_wait().unwrap().is_none(), "waiter exited early");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        received.try_recv().is_err(),
        "busy panes must not be notified"
    );
    fs::write(&arena.release, "release").unwrap();
    collect(holder.child.take().unwrap());
    collect(waiter);
    let record: gwt::cli::verification_record::VerificationRunRecord = serde_json::from_slice(
        &fs::read(repo.join(gwt::cli::verification_record::VERIFICATION_RUN_STATE_RELATIVE))
            .unwrap(),
    )
    .unwrap();
    assert!(record.all_passed, "the admitted attempt must finish first");
    assert!(
        received.try_recv().is_err(),
        "busy panes must not be notified"
    );
    idle.store(true, Ordering::Release);
    let (session, text) = received
        .recv_timeout(Duration::from_secs(30))
        .expect("live admission must notify when the original pane becomes idle");
    assert_eq!(session, "live-waiter");
    assert!(text.contains("Do not start another verify.run"), "{text}");
    assert!(
        received.try_recv().is_err(),
        "notify the original attempt once"
    );
}

#[test]
#[ignore = "self-exec Heavy command controlled by the parent test"]
fn resume_holder_parks() {
    fs::write(std::env::var_os("RESUME_CLI_READY").unwrap(), "ready").unwrap();
    let release = PathBuf::from(std::env::var_os("RESUME_CLI_RELEASE").unwrap());
    let deadline = Instant::now() + Duration::from_secs(90);
    while !release.exists() {
        assert!(Instant::now() < deadline, "holder fixture was not released");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "self-exec command used by the resumed full matrix"]
fn resume_command_passes() {}
