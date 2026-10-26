#![cfg(unix)]
use gwt_core::daemon::{RuntimeScope, RuntimeTarget};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    process::Stdio,
    time::{Duration, Instant},
};

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct RealOwnerFixture {
    _temp: tempfile::TempDir,
    home: std::path::PathBuf,
    repo: std::path::PathBuf,
    scope: RuntimeScope,
    endpoint_path: std::path::PathBuf,
    child: ChildGuard,
    endpoint: Value,
    deadline: Instant,
}
impl RealOwnerFixture {
    fn start(startup_intent: Option<gwt_core::daemon::DaemonStartIntent>) -> Self {
        let temp = gwt_core::test_support::socket_tempdir().unwrap();
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&repo).unwrap();
        assert!(gwt_core::process::hidden_command("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        assert!(gwt_core::process::hidden_command("git")
            .args([
                "-c",
                "user.name=owner fixture",
                "-c",
                "user.email=owner@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture"
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        assert!(gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/example/owner-runtime-fixture.git"
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        let scope = RuntimeScope::from_project_root(&repo, RuntimeTarget::Host).unwrap();
        let path = scope.endpoint_path(&home.join(".gwt"));
        let mut command = gwt_core::process::hidden_command(env!("CARGO_BIN_EXE_gwtd"));
        command
            .stdin(Stdio::piped())
            .current_dir(&repo)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(home.join("daemon.stderr")).unwrap(),
            ));
        for key in [
            "GWT_HOME",
            "GWT_PROJECT_ROOT",
            "GWT_REPO_HASH",
            "GWT_SESSION_ID",
            "GWT_SESSION_KIND",
            "GWT_SESSION_RUNTIME_PATH",
            "GWT_WORKTREE_HASH",
            "GWT_DAEMON_SOCKET_DIR",
        ] {
            command.env_remove(key);
        }
        let mut child = ChildGuard(command.spawn().unwrap());
        child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", json!({"schema_version":1,"operation":"daemon.start","params":startup_intent.map(|intent| json!({"startup_intent":intent})).unwrap_or(json!({}))})).as_bytes())
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15); // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
        let endpoint: Value = loop {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(value) = serde_json::from_slice(&bytes) {
                    break value;
                }
            }
            assert!(
                Instant::now() < deadline, // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
                "daemon did not publish endpoint: {:?}; expected {}",
                fs::read_to_string(home.join("daemon.stderr")),
                path.display()
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        assert_eq!(endpoint["pid"], child.0.id());
        Self {
            _temp: temp,
            home,
            repo,
            scope,
            endpoint_path: path,
            child,
            endpoint,
            deadline,
        }
    }
}

#[test]
fn real_daemon_owns_shell_input_snapshot_resize_and_exact_close() {
    let fixture = RealOwnerFixture::start(None);
    let RealOwnerFixture {
        _temp,
        home,
        repo,
        scope,
        child,
        endpoint,
        deadline,
        ..
    } = fixture;
    assert_eq!(endpoint["startup_intent"], "standalone");
    let url = endpoint["project_transport_url"]
        .as_str()
        .expect("daemon must publish its project owner transport");
    let token = endpoint["auth_token"].as_str().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let client = reqwest::Client::new();
        let send = |token: String, request: Value| { let client = client.clone(); let url = url.to_string(); async move {
            client.post(format!("{url}/private/project/command")).bearer_auth(token).json(&request).send().await.unwrap()
        }};
        let request = |command| json!({"project_key":scope.repo_hash,"command":command});
        let owned_id = format!("{}::owned", scope.repo_hash);
        assert_eq!(send("wrong".into(), request(json!({"type":"health"}))).await.status(), 401);
        let launched: Value = send(token.into(), request(json!({"type":"launch_shell","id":owned_id,"command":"/bin/sh","args":["-c","read line; printf 'owner-output:%s\\n' \"$line\"; read end"],"cols":80,"rows":24}))).await.json().await.unwrap();
        let incarnation = launched["incarnation"].as_u64().unwrap();
        let pid = launched["child_pid"].as_u64().unwrap() as u32;
        assert_ne!(pid, std::process::id()); assert_ne!(pid, child.0.id());
        let parent = gwt_core::process::hidden_command("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().unwrap();
        assert!(parent.status.success());
        assert_eq!(String::from_utf8(parent.stdout).unwrap().trim().parse::<u32>().unwrap(), child.0.id(), "physical PTY process must be a child of the actual daemon");
        let exact = |kind: &str| json!({"type":kind,"id":owned_id,"incarnation":incarnation});
        let mut resize = exact("resize"); resize["cols"] = json!(100); resize["rows"] = json!(30);
        assert!(send(token.into(), request(resize)).await.status().is_success());
        let mut input = exact("input"); input["data"] = json!(b"hello\n".to_vec());
        assert!(send(token.into(), request(input)).await.status().is_success());
        loop {
            let snapshot: Value = send(token.into(), request(exact("snapshot"))).await.json().await.unwrap();
            if snapshot["screen"].as_str().unwrap().contains("owner-output:hello") { break; }
            assert!(Instant::now() < deadline, "PTY output missing"); tokio::time::sleep(Duration::from_millis(100)).await; // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
        }
        let mut stale = exact("close"); stale["incarnation"] = json!(incarnation + 1);
        assert_eq!(send(token.into(), request(stale)).await.status(), 409);
        assert!(gwt::process::is_process_alive(pid));
        assert!(send(token.into(), request(exact("close"))).await.status().is_success());
        assert!(!gwt::process::is_process_alive(pid), "close must reap owned child before acknowledgement");
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("owner-fixture".into()))
            .working_dir(&repo).branch("main").base_branch("main").build();
        config.command = "/bin/sh".into();
        config.args = vec!["-c".into(), "read line; printf 'agent-reply:%s\\n' \"$line\"; read end".into()];
        config.linked_issue_number = Some(42);
        config.explicit_follow_up = true;
        let response = send(token.into(), request(json!({"type":"launch_agent","id":format!("{}::agent",scope.repo_hash),"operation_id":"real-bound-launch","tab_id":scope.repo_hash,"config":config,"cols":80,"rows":24}))).await;
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "bound launch failed: {status} {text}");
        let launched: Value = serde_json::from_str(&text).unwrap();
        let incarnation = launched["incarnation"].as_u64().unwrap();
        let session_id = launched["session_id"].as_str().unwrap();
        let session = gwt_agent::Session::load(&home.join(".gwt/sessions").join(format!("{session_id}.toml"))).unwrap();
        assert!(session.execution_binding.is_some(), "fixture must prove a real producing Session");
        let sessions_dir = home.join(".gwt/sessions");
        let runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path_for_pid(&sessions_dir, child.0.id(), session_id)).unwrap();
        assert_eq!(launched["host_pid"], child.0.id(), "Session physical host must be this project daemon");
        assert!(!gwt_agent::runtime_state_path_for_pid(&sessions_dir, std::process::id(), session_id).exists(), "the GUI/test process cannot publish this Session's physical observation");
        assert_eq!(runtime.host_started_at, gwt::process::host_process_start_time(child.0.id()));
        assert_eq!(runtime.runtime_incarnation, Some(incarnation));
        assert_eq!(runtime.child_pid, launched["child_pid"].as_u64().map(|pid| pid as u32));
        let pid = runtime.child_pid.unwrap();
        assert!(send(token.into(), request(json!({"type":"close","id":launched["id"],"incarnation":incarnation}))).await.status().is_success());
        assert!(!gwt::process::is_process_alive(pid));
        let closed = gwt_agent::Session::load(&home.join(".gwt/sessions").join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(closed.status, gwt_agent::AgentStatus::Stopped);
        assert!(!closed.restore_window_on_startup, "public close must finish durable Session state before acknowledgement");
    });
}

#[test]
fn gui_presence_drains_last_detach_and_explicit_standalone_promotion_survives() {
    use gwt::project_runtime::{client::ProjectRuntimeClient, events::OwnerCommand};
    use gwt_core::daemon::{DaemonEndpoint, DaemonStartIntent};
    let mut fixture = RealOwnerFixture::start(Some(DaemonStartIntent::GuiManaged));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let client =
            ProjectRuntimeClient::from_current_project(&fixture.repo, &fixture.home.join(".gwt"))
                .await
                .unwrap();
        let a = client.gui_events().await.unwrap();
        let b = client.gui_events().await.unwrap();
        let mut config =
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("owner-fixture".into()))
                .working_dir(&fixture.repo)
                .branch("main")
                .base_branch("main")
                .build();
        config.command = "/bin/sh".into();
        config.args = vec!["-c".into(), "read line".into()];
        config.linked_issue_number = Some(42);
        config.explicit_follow_up = true;
        let launched = client
            .command(OwnerCommand::LaunchAgent {
                id: format!("{}::resident-pane", fixture.scope.repo_hash),
                operation_id: "gui-lifetime-launch".into(),
                tab_id: fixture.scope.repo_hash.clone(),
                config: Box::new(config),
                metadata: Box::default(),
                cols: 80,
                rows: 24,
            })
            .await
            .unwrap();
        let session_path = fixture
            .home
            .join(".gwt/sessions")
            .join(format!("{}.toml", launched["session_id"].as_str().unwrap()));
        let before = gwt_agent::Session::load(&session_path).unwrap();
        assert!(before.execution_binding.is_some());
        assert!(
            before.restore_window_on_startup,
            "fixture must have saved restore intent"
        );
        let pane_pid = launched["child_pid"].as_u64().unwrap() as u32;
        assert_eq!(
            client.command(OwnerCommand::Health).await.unwrap()["gui_presence_count"],
            2
        );
        drop(a);
        let deadline = Instant::now() + gwt_core::deadline_budget::HANG_GUARD; // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
        loop {
            if client.command(OwnerCommand::Health).await.unwrap()["gui_presence_count"] == 1 {
                break;
            }
            assert!(
                Instant::now() < deadline, // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
                "first GUI detach was not observed"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(fixture.child.0.try_wait().unwrap().is_none());
        assert!(
            gwt::process::is_process_alive(pane_pid),
            "another GUI's detach cannot reap the shared pane"
        );
        drop(b);
        loop {
            if let Some(status) = fixture.child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < deadline, // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
                "last GUI detach did not drain and stop the daemon"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!gwt::process::is_process_alive(pane_pid));
        let stopped = gwt_agent::Session::load(&session_path).unwrap();
        assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
        assert!(
            stopped.restore_window_on_startup,
            "GUI lifetime drain must preserve saved restore intent"
        );
        assert!(
            !fixture.endpoint_path.exists(),
            "successful drain removes only its endpoint"
        );
    });

    let mut fixture = RealOwnerFixture::start(Some(DaemonStartIntent::GuiManaged));
    let endpoint: DaemonEndpoint = serde_json::from_value(fixture.endpoint.clone()).unwrap();
    runtime.block_on(async {
        let client = ProjectRuntimeClient::new(&endpoint).unwrap();
        let gui = client.gui_events().await.unwrap();
        let mut promotion = ChildGuard(
            gwt_core::process::hidden_command(env!("CARGO_BIN_EXE_gwtd"))
                .current_dir(&fixture.repo)
                .env("HOME", &fixture.home)
                .env("USERPROFILE", &fixture.home)
                .env_remove("GWT_HOME")
                .env_remove("GWT_PROJECT_ROOT")
                .env_remove("GWT_REPO_HASH")
                .env_remove("GWT_WORKTREE_HASH")
                .env_remove("GWT_DAEMON_SOCKET_DIR")
                .env_remove("GWT_SESSION_ID")
                .env_remove("GWT_SESSION_KIND")
                .env_remove("GWT_SESSION_RUNTIME_PATH")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        promotion
            .0
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"schema_version\":1,\"operation\":\"daemon.start\",\"params\":{}}\n")
            .unwrap();
        let deadline = Instant::now() + gwt_core::deadline_budget::HANG_GUARD; // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
        loop {
            if let Some(status) = promotion.0.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "explicit standalone reuse must authenticate and promote"
                );
                break;
            }
            assert!(
                Instant::now() < deadline, // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
                "standalone reuse did not complete"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(gui);
        loop {
            let health = client.command(OwnerCommand::Health).await.unwrap();
            let healed = fs::read(&fixture.endpoint_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<DaemonEndpoint>(&bytes).ok());
            if health["gui_presence_count"] == 0
                && healed.is_some_and(|endpoint| {
                    endpoint.startup_intent == DaemonStartIntent::Standalone
                })
            {
                assert_eq!(health["startup_intent"], "standalone");
                break;
            }
            assert!(
                Instant::now() < deadline, // test-hygiene: allow-wall-clock-deadline Bounds observable process or IPC progress; success depends on the actual owner receipt.
                "standalone promotion was not healed into the descriptor"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            fixture.child.0.try_wait().unwrap().is_none(),
            "promoted standalone owner remains resident without a GUI"
        );
    });
}

#[test]
fn real_daemon_agent_frontend_uses_its_physical_owner_and_bound_session() {
    use futures_util::{SinkExt, StreamExt};
    use gwt::project_runtime::{client::ProjectRuntimeClient, events::OwnerCommand};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let fixture = RealOwnerFixture::start(None);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let client=ProjectRuntimeClient::from_current_project(&fixture.repo,&fixture.home.join(".gwt")).await.unwrap();
        let id=format!("{}::agent-dispatch",fixture.scope.repo_hash);
        let capability_path=fixture.home.join("agent-capability-fixture");
        let mut config=gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("owner-fixture".into()))
            .working_dir(&fixture.repo).branch("main").base_branch("main").build();
        config.command="/bin/sh".into();
        config.args=vec!["-c".into(),
            r#"printf '%s\n%s\n' "$GWT_PANE_WS_URL" "$GWT_HOOK_FORWARD_TOKEN" > "$1"; printf 'agent-dispatch-ready\n'; read line; printf 'agent-dispatch:%s\n' "$line"; read end"#.into(),
            "owner-fixture".into(),capability_path.display().to_string()];
        config.linked_issue_number=Some(42);config.explicit_follow_up=true;
        let launched=client.command(OwnerCommand::LaunchAgent {
            id:id.clone(),operation_id:"real-agent-dispatch".into(),tab_id:fixture.scope.repo_hash.clone(),
            config:Box::new(config),metadata:Box::default(),cols:80,rows:24,
        }).await.unwrap();
        assert_eq!(launched["awaiting_readiness"],false,"fixture must launch an active ordinary producing Session, not an unfinished activation");
        let incarnation=launched["incarnation"].as_u64().unwrap();
        let session_id=launched["session_id"].as_str().unwrap();
        let child_pid=launched["child_pid"].as_u64().unwrap() as u32;
        let sessions=fixture.home.join(".gwt/sessions");
        let session_path=sessions.join(format!("{session_id}.toml"));
        let bound=gwt_agent::Session::load(&session_path).unwrap();
        assert!(bound.execution_binding.is_some(),"agent frontend must exercise real producing authority");
        let registry_path=fixture.home.join(".gwt/projects").join(&fixture.scope.repo_hash).join("workspace.json");
        let original=gwt::persistence::load_workspace_state(&registry_path).unwrap();
        let authoritative=original.windows.iter().find(|row|row.id=="agent-dispatch").unwrap().clone();
        client.command(OwnerCommand::UpdateLayout {layout:gwt::persistence::empty_workspace_state()}).await.unwrap();
        assert!(gwt::persistence::load_workspace_state(&registry_path).unwrap().windows.iter().any(|row|row==&authoritative),"stale GUI omission must preserve owner process/husk rows");
        let mut layout=original;
        let mut forged=authoritative.clone();forged.id="forged-process".into();forged.session_id=Some("foreign-session".into());
        layout.windows.push(forged);
        let proposed=layout.windows.iter_mut().find(|row|row.id=="agent-dispatch").unwrap();
        proposed.session_id=Some("foreign-session".into());proposed.agent_name=Some("forged-name".into());
        proposed.status=gwt::WindowProcessStatus::Stopped;proposed.preset=gwt::WindowPreset::Logs;proposed.persist=false;
        client.command(OwnerCommand::UpdateLayout {layout}).await.unwrap();
        let retained=gwt::persistence::load_workspace_state(&registry_path).unwrap();
        assert!(retained.windows.iter().any(|row|row==&authoritative),"GUI presentation cannot rewrite an owner process identity");
        assert!(!retained.windows.iter().any(|row|row.id=="forged-process"),"GUI layout cannot create/resurrect a Session-bound process");
        let capability=loop {
            if let Ok(value)=fs::read_to_string(&capability_path) {
                if value.ends_with('\n') && value.lines().count()==2 {break value;}
            }
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            assert!(Instant::now()<fixture.deadline,"owned child did not receive its capability");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let mut lines=capability.lines();let url=lines.next().unwrap();let token=lines.next().unwrap();
        assert!(!token.is_empty());
        let mut request=url.into_client_request().unwrap();
        request.headers_mut().insert("Authorization",format!("Bearer {token}").parse().unwrap());
        let (mut socket,_)=tokio_tungstenite::connect_async(request).await.unwrap();
        socket.send(Message::Text(json!({"kind":"list_windows"}).to_string().into())).await.unwrap();
        // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
        let workspace=tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let Message::Text(text)=socket.next().await.unwrap().unwrap() else {continue};
                let value:Value=serde_json::from_str(&text).unwrap();
                if value["kind"]=="workspace_state" {break value;}
            }
        }).await.expect("daemon owner must dispatch agent pane.list");
        assert_eq!(workspace["workspace"]["tabs"][0]["id"],fixture.scope.repo_hash);
        assert_eq!(workspace["workspace"]["tabs"][0]["workspace"]["windows"][0]["id"],id);
        assert_eq!(workspace["workspace"]["tabs"][0]["workspace"]["windows"][0]["session_id"],session_id);
        socket.send(Message::Text(json!({"kind":"pane_name","candidate":"ignored","request_id":"owner-name"}).to_string().into())).await.unwrap();
        // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
        let name=tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let Message::Text(text)=socket.next().await.unwrap().unwrap() else {continue};
                let value:Value=serde_json::from_str(&text).unwrap();
                if value["kind"]=="pane_name_result" {break value;}
            }
        }).await.expect("daemon owner must dispatch current bound name grant");
        assert_eq!(name["ok"],true,"{name}");
        assert_eq!(name["agent_name"].as_str(),gwt_agent::Session::load(&session_path).unwrap().agent_name.as_deref());
        loop {
            let snapshot=client.command(OwnerCommand::Snapshot {id:id.clone(),incarnation}).await.unwrap();
            if snapshot["screen"].as_str().unwrap().contains("agent-dispatch-ready") {break;}
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            assert!(Instant::now()<fixture.deadline,"child readiness output missing");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        loop {
            socket.send(Message::Text(json!({"kind":"frontend_ready"}).to_string().into())).await.unwrap();
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            let read=tokio::time::timeout(Duration::from_secs(10),async {
                let mut snapshot_seen=false;
                loop {
                    let Message::Text(text)=socket.next().await.unwrap().unwrap() else {continue};
                    let value:Value=serde_json::from_str(&text).unwrap();
                    if value["kind"]=="terminal_snapshot" {
                        assert_eq!(value["id"],id);assert!(!value["data_base64"].as_str().unwrap().is_empty());
                        snapshot_seen=true;
                    }
                    if value["kind"]=="pane_sync_complete" {break snapshot_seen;}
                }
            }).await.expect("agent read must return owner snapshots and synchronization completion");
            if read {break;}
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            assert!(Instant::now()<fixture.deadline,"owner snapshot stayed busy");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        socket.send(Message::Text(json!({"kind":"pane_send_input","session_id":session_id,"text":"physical-owner-input\r"}).to_string().into())).await.unwrap();
        // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
        let sent=tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let Message::Text(text)=socket.next().await.unwrap().unwrap() else {continue};
                let value:Value=serde_json::from_str(&text).unwrap();
                if value["kind"]=="pane_send_result" {break value;}
            }
        }).await.expect("accepted input must finish inside the current bound grant");
        assert_eq!(sent["ok"],true,"{sent}");assert_eq!(sent["window_id"],id);
        loop {
            let snapshot=client.command(OwnerCommand::Snapshot {id:id.clone(),incarnation}).await.unwrap();
            if snapshot["screen"].as_str().unwrap().contains("agent-dispatch:physical-owner-input") {break;}
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            assert!(Instant::now()<fixture.deadline,"actual owner child did not receive the submit");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let correlation=uuid::Uuid::new_v4().to_string();
        socket.send(Message::Text(json!({"kind":"close_window","id":id,"request_id":correlation}).to_string().into())).await.unwrap();
        // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
        let accepted=tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let Message::Text(text)=socket.next().await.unwrap().unwrap() else {continue};
                let value:Value=serde_json::from_str(&text).unwrap();
                if value["kind"]=="pane_close_accepted" {break value;}
            }
        }).await.expect("self-close must answer before committing its captured teardown");
        assert_eq!(accepted["request_id"],correlation);assert_eq!(accepted["window_id"],id);
        assert!(!accepted.to_string().contains(token));
        loop {
            let closed=gwt_agent::Session::load(&session_path).unwrap();
            if !gwt::process::is_process_alive(child_pid) && closed.status==gwt_agent::AgentStatus::Stopped && !closed.restore_window_on_startup {break;}
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            assert!(Instant::now()<fixture.deadline,"accepted exact self-close did not finish its physical/durable work");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        loop {
            let registry=gwt::persistence::load_workspace_state(&registry_path).unwrap();
            if !registry.windows.iter().any(|row|row.id=="agent-dispatch") {break;}
            // test-hygiene: allow-wall-clock-deadline Real daemon child/PTY and authenticated socket observation has no in-process completion seam.
            assert!(Instant::now()<fixture.deadline,"exact close did not durably remove its owner registry row");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}
