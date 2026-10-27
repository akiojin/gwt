use super::*;
use std::collections::BTreeSet;

pub(super) fn control_custom_agent() -> gwt_agent::CustomCodingAgent {
    gwt_agent::CustomCodingAgent {
        id: "pane-contract-fixture".into(),
        display_name: "Pane contract fixture".into(),
        agent_type: gwt_agent::custom::CustomAgentType::Command,
        command: if cfg!(windows) {
            "powershell"
        } else {
            "/bin/sh"
        }
        .into(),
        default_args: if cfg!(windows) {
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "while ($null -ne [Console]::ReadLine()) {}".into(),
            ]
        } else {
            vec![
                "-c".into(),
                "stty -echo -icanon -isig; exec cat >/dev/null".into(),
            ]
        },
        mode_args: Some(gwt_agent::custom::ModeArgs {
            normal: Vec::new(),
            continue_mode: Vec::new(),
            resume: Vec::new(),
        }),
        skip_permissions_args: Vec::new(),
        env: HashMap::new(),
        supports_resume_picker: false,
    }
}

pub(super) fn control_launch_config(project: &Path, issue: u64) -> gwt_agent::LaunchConfig {
    let base = git_stdout(project, &["branch", "--show-current"]);
    let scope = gwt_core::paths::project_scope_hash(project);
    let worktree = project
        .parent()
        .unwrap()
        .join("pane-worktrees")
        .join(scope.to_string())
        .join(issue.to_string());
    let branch = format!("work/pane-fixture-{issue}");
    if !worktree.exists() {
        fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        run_git(
            project,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                &branch,
                worktree.to_str().unwrap(),
                "HEAD",
            ],
        );
    }
    let custom = control_custom_agent();
    let mut config =
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom(custom.id.clone()))
            .custom_agent(custom)
            .working_dir(&worktree)
            .branch(branch)
            .base_branch(base)
            .build();
    // Retain the builder's ordinary permission decision. The production owner
    // alone creates the producing Session, execution and transport capability.
    config.linked_issue_number = Some(issue);
    config.explicit_follow_up = true;
    config
}

pub(super) fn control_owner_launch(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    raw_id: &str,
    config: gwt_agent::LaunchConfig,
) -> gwt::project_runtime::test_support::OwnerRuntimeProbe {
    use gwt::project_runtime::events::OwnerCommand;
    configure_owner_fixture_start_gate(owner);
    let expects_producing =
        config.linked_issue_number.is_some() && !config.suppress_execution_control;
    let id = owner.pane_id(raw_id);
    let launched = owner_fixture_block_on(owner.command(OwnerCommand::LaunchAgent {
        id: id.clone(),
        operation_id: format!("pane-fixture-{}", uuid::Uuid::new_v4()),
        tab_id: id.split_once("::").unwrap().0.into(),
        config: Box::new(config),
        metadata: Box::default(),
        cols: 80,
        rows: 24,
    }))
    .unwrap();
    let probe = owner_fixture_block_on(owner.probe(&id, launched["incarnation"].as_u64().unwrap()))
        .unwrap();
    assert_eq!(probe.session.as_ref().unwrap().id, launched["session_id"]);
    assert_eq!(
        probe.session.as_ref().unwrap().execution_binding.is_some(),
        expects_producing
    );
    assert!(
        probe.grant.is_some(),
        "the production owner must issue the request capability"
    );
    probe
}

pub(super) fn control_init_repo(project: &Path) {
    fs::create_dir_all(project).unwrap();
    if !project.join(".git").exists() {
        init_repo(project);
    }
    if !gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(project)
        .output()
        .unwrap()
        .status
        .success()
    {
        run_git(
            project,
            &[
                "-c",
                "user.name=Pane fixture",
                "-c",
                "user.email=pane@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "pane fixture",
            ],
        );
    }
}

pub(super) fn control_seed_foreign_roster(project: &Path) {
    control_init_repo(project);
    seed_owner_fixture_roster(project, 8);
    gwt::mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(project),
        |prefs| {
            for (member, name) in prefs
                .agent_roster
                .as_mut()
                .unwrap()
                .members
                .iter_mut()
                .zip([
                    "India", "Juliett", "Kilo", "Lima", "Mike", "November", "Oscar", "Papa",
                ])
            {
                member.agent_member_id = format!("member-{name}");
                member.name = name.into();
            }
        },
    )
    .unwrap();
}

pub(super) fn control_owner_agent(
    project: &Path,
    raw_id: &str,
    issue: u64,
) -> (
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    control_init_repo(project);
    if gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(project))
        .unwrap()
        .agent_roster
        .is_none()
    {
        seed_owner_fixture_roster(project, 8);
    }
    let owner = Arc::new(
        owner_fixture_block_on(
            gwt::project_runtime::test_support::ProjectOwnerHarness::start(project),
        )
        .unwrap(),
    );
    let probe = control_owner_launch(&owner, raw_id, control_launch_config(project, issue));
    (owner, probe)
}

struct ControlSessionMutation {
    original: Option<gwt_agent::Session>,
    sessions: PathBuf,
}
impl ControlSessionMutation {
    fn new(pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe) -> Self {
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let original = gwt_agent::Session::load(
            &sessions.join(format!("{}.toml", pane.session.as_ref().unwrap().id)),
        )
        .unwrap();
        Self {
            original: Some(original),
            sessions,
        }
    }
    fn restore(mut self) {
        self.original.take().unwrap().save(&self.sessions).unwrap();
    }
}
impl Drop for ControlSessionMutation {
    fn drop(&mut self) {
        if let Some(original) = self.original.take() {
            // Undo only this fixture's deliberate hostile Session mutation
            // before its actual owner drains, including an assertion failure.
            let _ = original.save(&self.sessions);
        }
    }
}

pub(super) fn control_request(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    caller: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    request: AgentFrontendRequest,
) -> gwt::project_runtime::test_support::OwnerAgentReply {
    let client = owner_fixture_block_on(
        owner.agent_client(&format!("pane-client-{}", uuid::Uuid::new_v4())),
    )
    .unwrap();
    owner_fixture_block_on(client.request(caller.grant.as_ref().unwrap().clone(), request)).unwrap()
}

fn control_request_settled(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    caller: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    request: AgentFrontendRequest,
) -> gwt::project_runtime::test_support::OwnerAgentReply {
    let client = owner_fixture_block_on(
        owner.agent_client(&format!("completed-client-{}", uuid::Uuid::new_v4())),
    )
    .unwrap();
    let mut reply =
        owner_fixture_block_on(client.request(caller.grant.as_ref().unwrap().clone(), request))
            .unwrap();
    control_settled_self_closes(owner);
    let final_reply = client.drain().unwrap();
    reply.events.extend(final_reply.events);
    reply.close = final_reply.close.or(reply.close);
    reply
}

pub(super) fn control_close(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::Close {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
        }),
    )
    .unwrap();
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ControlRetainedSession {
    id: String,
    session: gwt_agent::Session,
    child_pid: u32,
    child_started_at: u64,
    owner_pid: u32,
}

pub(super) fn run_retained_owner_fixture() {
    let Some(root) = std::env::var_os("GWT_TEST_RETAINED_ROOT") else {
        return;
    };
    let home = std::env::var_os("GWT_TEST_RETAINED_HOME").unwrap();
    let _home = ScopedGwtHome::set(PathBuf::from(home));
    let project = PathBuf::from(root);
    let raw = std::env::var("GWT_TEST_RETAINED_RAW").unwrap();
    let (owner, bootstrap) = control_owner_agent(&project, "bootstrap", 900);
    let mut config = control_launch_config(&project, 901);
    config.linked_issue_number = None;
    config.explicit_follow_up = false;
    let target = control_owner_launch(&owner, &raw, config);
    let child_pid = target.pty.process_id().unwrap();
    let child_started_at = gwt::process::host_process_start_time(child_pid).unwrap();
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("fixture owns the only harness")
            .shutdown(),
    );
    assert!(bootstrap.pty.try_wait().unwrap().is_some());
    assert!(target.pty.try_wait().unwrap().is_some());
    let saved = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", target.session.as_ref().unwrap().id)),
    )
    .unwrap();
    assert_eq!(saved.status, gwt_agent::AgentStatus::Stopped);
    assert!(saved.restore_window_on_startup);
    let bootstrap_saved = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", bootstrap.session.as_ref().unwrap().id)),
    )
    .unwrap();
    let retained_members = [
        bootstrap_saved.agent_member_id.as_deref(),
        saved.agent_member_id.as_deref(),
    ];
    gwt::mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&project),
        |prefs| {
            for member in &mut prefs.agent_roster.as_mut().unwrap().members {
                if retained_members.contains(&Some(member.agent_member_id.as_str())) {
                    member.enabled = false;
                }
            }
            prefs.max_active_agents = prefs.agent_roster.as_ref().unwrap().enabled_count();
        },
    )
    .unwrap();
    fs::write(
        project.join("retained-owner-receipt.json"),
        serde_json::to_vec(&ControlRetainedSession {
            id: target.id,
            session: saved,
            child_pid,
            child_started_at,
            owner_pid: std::process::id(),
        })
        .unwrap(),
    )
    .unwrap();
}

fn control_retained_husk(project: &Path, raw: &str) -> ControlRetainedSession {
    control_init_repo(project);
    let home = gwt_core::test_support::gwt_home_override().expect("isolated fixture HOME");
    let child = gwt_core::process::hidden_command(std::env::current_exe().unwrap())
        .args([
            "app_runtime::tests::owner_retained_husk_helper",
            "--exact",
            "--nocapture",
        ])
        .env("GWT_TEST_RETAINED_ROOT", project)
        .env("GWT_TEST_RETAINED_HOME", &home)
        .env("GWT_TEST_RETAINED_RAW", raw)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .spawn()
        .unwrap();
    let old_host = child.id();
    let status = child.wait_with_output().unwrap().status;
    assert!(status.success(), "actual previous owner fixture failed");
    assert!(
        !gwt::process::is_host_process_alive(old_host),
        "the prior Host process must have actually exited"
    );
    let receipt: ControlRetainedSession =
        serde_json::from_slice(&fs::read(project.join("retained-owner-receipt.json")).unwrap())
            .unwrap();
    assert!(!gwt::process::exact_pty_process_tree_is_alive(
        receipt.child_pid,
        receipt.child_started_at
    ));
    let canonical = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", receipt.session.id)),
    )
    .unwrap();
    assert_eq!(canonical.id, receipt.session.id);
    assert_eq!(canonical.status, gwt_agent::AgentStatus::Stopped);
    assert!(canonical.restore_window_on_startup);
    receipt
}

pub(super) fn control_register_pm(
    project: &Path,
    pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    let session = pane.session.as_ref().unwrap();
    let mut registration = pm_registration_fixture(&session.id, &session.worktree_path);
    registration.agent_id = session.agent_id.command().to_string();
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(project),
        registration,
        |_| false,
    )
    .unwrap();
}

fn control_fleet_request(
    target: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
) -> AgentFrontendRequest {
    AgentFrontendRequest::CloseFleetPane {
        request_id: "84ef22cc-44ef-415d-b4c4-06a6d4cc7416".into(),
        id: target.id.clone(),
        session_id: target.session.as_ref().unwrap().id.clone(),
        child_pid: target.pty.process_id(),
        child_started_at: target
            .pty
            .process_id()
            .and_then(gwt::process::host_process_start_time),
    }
}

fn assert_fleet_refused(
    reply: &gwt::project_runtime::test_support::OwnerAgentReply,
    expected: &str,
) {
    assert_eq!(reply.close, None);
    assert!(
        reply
            .events
            .iter()
            .any(|event| event["kind"] == "fleet_pane_close_result"
                && event["ok"] == false
                && event["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains(expected))),
        "{:?}",
        reply.events
    );
}

fn control_projection(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    project: &Path,
) -> gwt::project_runtime::agent_dispatch::PaneProjection {
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let rows: gwt::PersistedWindowRegistry =
        serde_json::from_value(inventory["canonical_rows"].clone()).unwrap();
    let raw = pane.id.split_once("::").unwrap().1;
    let mut window = rows
        .windows
        .into_iter()
        .find(|window| window.id == raw)
        .unwrap();
    window.id = pane.id.clone();
    gwt::project_runtime::agent_dispatch::PaneProjection {
        project_root: project.to_path_buf(),
        tab_id: pane.id.split_once("::").unwrap().0.into(),
        window,
        active: None,
    }
}

pub(super) fn control_accept_self_close(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
) -> (
    crate::embedded_server::AgentSelfCloseDirectAcceptance,
    gwt::project_runtime::test_support::OwnerAgentReply,
) {
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();
    let reply = control_request(
        owner,
        pane,
        AgentFrontendRequest::CloseWindow {
            id: pane.id.clone(),
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            responder: Some(responder),
        },
    );
    (
        acceptance
            .try_recv()
            .expect("canonical dispatch completed")
            .expect("real owner accepts correlated self-close"),
        reply,
    )
}

fn control_settled_self_closes(owner: &gwt::project_runtime::test_support::ProjectOwnerHarness) {
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline bound the real actor/finalizer ACK wait after the origin releases its exact acceptance ticket
        tokio::time::timeout(
            gwt_core::deadline_budget::HANG_GUARD,
            owner.settled_self_closes(),
        )
        .await
        .unwrap()
        .unwrap();
    });
}

fn control_close_series(project: &Path, count: u64) {
    let (owner, caller) = control_owner_agent(project, "caller", 42);
    let targets = (0..count)
        .map(|index| {
            control_owner_launch(
                &owner,
                &format!("target-{index}"),
                control_launch_config(project, 100 + index),
            )
        })
        .collect::<Vec<_>>();
    let mut releases = Vec::new();
    let mut holders = Vec::new();
    for target in &targets {
        let pane = target.pane.clone();
        let (held_tx, held_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        holders.push(thread::spawn(move || {
            let _guard = pane.lock().unwrap();
            held_tx.send(()).unwrap();
            gwt_core::test_support::recv_event(
                &release_rx,
                "all real close/list requests finished before parser release",
            );
        }));
        held_rx.recv().unwrap();
        releases.push(release_tx);
    }
    owner_fixture_block_on(async {
        let (mut socket, _) = connect_async(caller.agent_pane_request().unwrap())
            .await
            .unwrap();
        socket
            .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
            .await
            .unwrap();
        let _ = next_test_agent_websocket_event(
            &mut socket,
            "workspace_state",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        for target in &targets {
            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({"kind":"close_window","id":target.id})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        socket
            .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
            .await
            .unwrap();
        let listed = next_test_agent_websocket_event(
            &mut socket,
            "workspace_state",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert!(listed["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == caller.id));
        assert!(
            targets
                .iter()
                .all(|target| target.pty.try_wait().unwrap().is_none()),
            "close/list must answer before any held target finalizer can reap"
        );
        for release in releases {
            release.send(()).unwrap();
        }
        let mut completed = BTreeSet::new();
        for _ in 0..count {
            let closed = next_test_agent_websocket_event(
                &mut socket,
                "pane_close_result",
                gwt_core::deadline_budget::HANG_GUARD,
            )
            .await;
            assert_eq!(closed["ok"], true);
            assert!(
                completed.insert(closed["window_id"].as_str().unwrap().to_owned()),
                "each actual close must acknowledge once"
            );
        }
        assert_eq!(
            completed,
            targets.iter().map(|target| target.id.clone()).collect()
        );
        socket
            .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
            .await
            .unwrap();
        let final_list = next_test_agent_websocket_event(
            &mut socket,
            "workspace_state",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert!(final_list["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| !completed.contains(row["id"].as_str().unwrap())));
        socket.close(None).await.unwrap();
    });
    for holder in holders {
        holder.join().unwrap();
    }
    for target in &targets {
        assert!(target.pty.try_wait().unwrap().is_some());
        let source = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir()
                .join(format!("{}.toml", target.session.as_ref().unwrap().id)),
        )
        .unwrap();
        assert_eq!(source.status, gwt_agent::AgentStatus::Stopped);
        assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_err());
    }
    control_close(&owner, &caller);
}

fn control_tracing_events(
    caller: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    run: impl FnOnce(),
) -> Vec<CapturedTracingEvent> {
    capture_tracing_events(|| {
        let trace = tracing::dispatcher::get_default(Clone::clone);
        let _trace = gwt::project_runtime::agent_frontend::capture_owner_agent_trace(
            caller.grant.as_ref().unwrap(),
            gwt::project_runtime::agent_frontend::OwnerAgentTraceRequest::Ready,
            trace,
        );
        run();
    })
}

fn issue_3713_read_board(project: &Path, session: &str) {
    let _session =
        gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session);
    let mut env = gwt::cli::TestEnv::new(project.to_path_buf());
    let command =
        gwt::cli::parse_board_args(&["show".into(), "--all".into(), "--json".into()]).unwrap();
    assert_eq!(gwt::cli::run(&mut env, command).unwrap(), 0);
    let _: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
}

#[test]
fn issue_3713_agent_self_close_keeps_capability_until_recent_owner_evidence_is_read() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let session = pane.session.as_ref().unwrap();
    gwt::board_provider::post_entry(
        &project,
        gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Reviewer",
            gwt_core::coordination::BoardEntryKind::Handoff,
            "Keep Issue 42 evidence until the ruling",
            None,
            None,
            vec![],
            vec!["42".into()],
        )
        .with_origin_session_id("reviewer-session"),
    )
    .unwrap();
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();
    let refused = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::CloseWindow {
            id: pane.id.clone(),
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            responder: Some(responder),
        },
    );
    assert_eq!(refused.close, None);
    let refusal = acceptance.try_recv().unwrap().unwrap_err();
    assert!(refusal.reason.contains("board.show"));
    assert_eq!(refusal.window_id, pane.id);
    assert!(pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    issue_3713_read_board(&project, &session.id);
    gwt::board_provider::post_entry(
        &project,
        gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Self",
            gwt_core::coordination::BoardEntryKind::Status,
            "Own completion does not require rereading",
            None,
            None,
            vec![],
            vec!["42".into()],
        )
        .with_origin_session_id(session.id.clone()),
    )
    .unwrap();
    let (accepted, _) = control_accept_self_close(&owner, &pane);
    assert!(!pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    drop(accepted);
    control_settled_self_closes(&owner);
}

#[test]
fn issue_3713_human_canvas_close_is_not_subject_to_agent_read_receipts() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    gwt::board_provider::post_entry(
        &project,
        gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Owner",
            gwt_core::coordination::BoardEntryKind::Handoff,
            "Unread evidence",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_session_id(pane.session.as_ref().unwrap().id.clone()),
    )
    .unwrap();
    control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());
}

#[test]
fn issue_3713_agent_peer_close_requires_reading_recent_target_session_evidence() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let peer = control_owner_launch(&owner, "peer", control_launch_config(&project, 43));
    gwt::board_provider::post_entry(
        &project,
        gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Peer",
            gwt_core::coordination::BoardEntryKind::Handoff,
            "Keep this pane for the ruling",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_session_id(peer.session.as_ref().unwrap().id.clone()),
    )
    .unwrap();
    let close = || AgentFrontendRequest::CloseWindow {
        id: peer.id.clone(),
        request_id: None,
        responder: None,
    };
    let refused = control_request(&owner, &pane, close());
    assert!(refused
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result"
            && event["ok"] == false
            && event["reason"].as_str().unwrap().contains("board.show")));
    assert!(owner_fixture_block_on(owner.probe(&peer.id, peer.incarnation)).is_ok());
    issue_3713_read_board(&project, "different-session");
    let refused = control_request(&owner, &pane, close());
    assert!(refused
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == false));
    issue_3713_read_board(&project, &pane.session.as_ref().unwrap().id);
    let accepted = control_request_settled(&owner, &pane, close());
    assert!(accepted
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == true));
    assert!(owner_fixture_block_on(owner.probe(&peer.id, peer.incarnation)).is_err());
    control_close(&owner, &pane);
}

#[test]
fn agent_pane_input_with_equal_session_ids_targets_authenticated_project_only() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let foreign_project = temp.path().join("foreign");
    control_seed_foreign_roster(&foreign_project);
    let (foreign_owner, foreign) = control_owner_agent(&foreign_project, "agent", 43);
    let own_projection = control_projection(&owner, &pane, &project);
    let mut foreign_projection = control_projection(&foreign_owner, &foreign, &foreign_project);
    // An adversarial read projection may repeat a Session id from another
    // project. It never installs that identity into either physical owner.
    foreign_projection.window.session_id = own_projection.window.session_id.clone();
    let selected = gwt::project_runtime::agent_dispatch::principal_session_window(
        &[foreign_projection, own_projection],
        pane.grant.as_ref().unwrap().principal(),
    )
    .unwrap();
    assert_eq!(selected, pane.id);
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::SendInput {
            text: "scoped-draft".into(),
        },
    );
    assert_eq!(reply.close, None);
    assert!(reply
        .events
        .iter()
        .any(|event| event["kind"] == "pane_send_result"
            && event["ok"] == true
            && event["window_id"] == pane.id));
    assert!(pane.pane.lock().unwrap().has_unsent_user_input());
    assert!(!foreign.pane.lock().unwrap().has_unsent_user_input());
    control_close(&owner, &pane);
    control_close(&foreign_owner, &foreign);
}

#[test]
fn agent_pane_close_rejects_window_from_foreign_project() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let foreign_project = temp.path().join("foreign");
    control_seed_foreign_roster(&foreign_project);
    let (foreign_owner, foreign) = control_owner_agent(&foreign_project, "foreign", 44);
    let peer = control_owner_launch(&owner, "peer", control_launch_config(&project, 43));
    let denied = control_request_settled(
        &owner,
        &pane,
        AgentFrontendRequest::CloseWindow {
            id: foreign.id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert_eq!(denied.close, None);
    assert!(
        denied
            .events
            .iter()
            .any(|event| event["kind"] == "pane_close_result"
                && event["ok"] == false
                && event["window_id"] == foreign.id
                && event["reason"].is_string()),
        "foreign close must answer explicitly: {:?}",
        denied.events
    );
    assert!(owner_fixture_block_on(foreign_owner.probe(&foreign.id, foreign.incarnation)).is_ok());
    let accepted = control_request_settled(
        &owner,
        &pane,
        AgentFrontendRequest::CloseWindow {
            id: peer.id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(
        accepted
            .events
            .iter()
            .any(|event| event["kind"] == "pane_close_result"
                && event["ok"] == true
                && event["window_id"] == peer.id),
        "completed exact close must reply to its requesting client: {:?}",
        accepted.events
    );
    assert!(owner_fixture_block_on(owner.probe(&peer.id, peer.incarnation)).is_err());
    assert!(owner_fixture_block_on(foreign_owner.probe(&foreign.id, foreign.incarnation)).is_ok());
    control_close(&owner, &pane);
    control_close(&foreign_owner, &foreign);
}

/// Issue #3629 AC-10/AC-12: every agent-route close outcome must answer the
/// requesting client with an explicit `pane_close_result` instead of the
/// silent empty event list that made `pane.close` undiagnosable.
#[test]
fn agent_pane_close_replies_with_pane_close_result() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let peer = control_owner_launch(&owner, "peer", control_launch_config(&project, 43));
    let request = || AgentFrontendRequest::CloseWindow {
        id: peer.id.clone(),
        request_id: None,
        responder: None,
    };
    let accepted = control_request_settled(&owner, &pane, request());
    assert!(accepted
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result"
            && event["ok"] == true
            && event["window_id"] == peer.id));
    let absent = control_request_settled(&owner, &pane, request());
    assert!(
        absent
            .events
            .iter()
            .any(|event| event["kind"] == "pane_close_result"
                && event["ok"] == false
                && event["window_id"] == peer.id
                && event["reason"].is_string()),
        "an already-removed target must still produce an explicit receipt: {:?}",
        absent.events
    );
    control_close(&owner, &pane);
}

#[test]
fn fleet_pane_inventory_scopes_actual_runtime_generations() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let foreign_project = temp.path().join("foreign");
    control_seed_foreign_roster(&foreign_project);
    let (foreign_owner, foreign) = control_owner_agent(&foreign_project, "agent", 43);
    let reply = control_request(&owner, &pane, AgentFrontendRequest::ListFleetPanes);
    assert_eq!(reply.close, None);
    assert_eq!(reply.events.len(), 1);
    let response = &reply.events[0];
    assert_eq!(response["kind"], "fleet_pane_list");
    let rows = response["panes"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "foreign owner panes must not leak");
    let session = pane.session.as_ref().unwrap();
    let child = pane.pty.process_id().unwrap();
    assert_eq!(rows[0]["window_id"], pane.id);
    assert_eq!(
        rows[0]["project_root"],
        dunce::canonicalize(&project)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(rows[0]["session_id"], session.id);
    assert_eq!(rows[0]["runtime_present"], true);
    assert_eq!(rows[0]["child_pid"], child);
    assert_eq!(
        rows[0]["child_started_at"],
        gwt::process::host_process_start_time(child).unwrap()
    );
    assert_eq!(
        rows[0]["worktree_path"],
        session.worktree_path.to_string_lossy().as_ref()
    );
    control_close(&owner, &pane);
    control_close(&foreign_owner, &foreign);
}

#[test]
fn fleet_pane_close_preserves_successors_and_requires_registered_pm() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pm) = control_owner_agent(&project, "pm", 42);
    let mut inspection = control_launch_config(&project, 43);
    inspection.linked_issue_number = None;
    inspection.explicit_follow_up = false;
    let target = control_owner_launch(&owner, "target", inspection);
    control_register_pm(&project, &pm);
    let observer = control_owner_launch(&owner, "observer", control_launch_config(&project, 44));
    let human_tab = sample_project_tab("human", "Project", project.clone(), ProjectKind::Git, &[]);
    let mut human_runtime = sample_runtime(temp.path(), vec![human_tab], Some("human"));
    let human = human_runtime.handle_frontend_event(
        "close-client".into(),
        FrontendEvent::CloseFleetPane {
            request_id: "84ef22cc-44ef-415d-b4c4-06a6d4cc7416".into(),
            id: target.id.clone(),
            session_id: target.session.as_ref().unwrap().id.clone(),
            child_pid: target.pty.process_id(),
            child_started_at: target
                .pty
                .process_id()
                .and_then(gwt::process::host_process_start_time),
        },
    );
    assert!(human.iter().any(|event| matches!(&event.event, BackendEvent::FleetPaneCloseResult { ok: false, reason: Some(reason), .. } if reason.contains("authenticated agent"))));
    let denied = control_request_settled(&owner, &observer, control_fleet_request(&target));
    assert_fleet_refused(&denied, "registered PM");
    let mut request = control_fleet_request(&target);
    if let AgentFrontendRequest::CloseFleetPane { session_id, .. } = &mut request {
        *session_id = "predecessor-session".into();
    }
    assert_fleet_refused(
        &control_request_settled(&owner, &pm, request),
        "session changed",
    );
    let mut request = control_fleet_request(&target);
    if let AgentFrontendRequest::CloseFleetPane {
        child_started_at, ..
    } = &mut request
    {
        *child_started_at = child_started_at.map(|time| time + 1);
    }
    assert_fleet_refused(
        &control_request_settled(&owner, &pm, request),
        "runtime changed",
    );
    let mut request = control_fleet_request(&target);
    if let AgentFrontendRequest::CloseFleetPane {
        child_pid,
        child_started_at,
        ..
    } = &mut request
    {
        *child_pid = None;
        *child_started_at = None;
    }
    assert_fleet_refused(
        &control_request_settled(&owner, &pm, request),
        "runtime changed",
    );
    assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_ok());
    let closed = control_request_settled(&owner, &pm, control_fleet_request(&target));
    assert!(
        closed
            .events
            .iter()
            .any(|event| event["kind"] == "fleet_pane_close_result" && event["ok"] == true),
        "{:?}",
        closed.events
    );
    assert!(target.pty.try_wait().unwrap().is_some());
    control_close(&owner, &observer);
    control_close(&owner, &pm);
}

#[test]
fn fleet_pane_close_protects_authority_acquired_after_inventory() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pm) = control_owner_agent(&project, "pm", 42);
    let mut inspection = control_launch_config(&project, 43);
    inspection.linked_issue_number = None;
    inspection.explicit_follow_up = false;
    let target = control_owner_launch(&owner, "target", inspection);
    control_register_pm(&project, &pm);
    let selected = control_request(&owner, &pm, AgentFrontendRequest::ListFleetPanes);
    assert!(selected.events[0]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["window_id"] == target.id));
    let session = target.session.as_ref().unwrap();
    gwt::cli::execution_state::materialize_at_launch(
        &session.worktree_path,
        gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        4179,
        &session.id,
        "gwt-execute",
        false,
    )
    .unwrap();
    let refused = control_request_settled(&owner, &pm, control_fleet_request(&target));
    assert_fleet_refused(&refused, "execution holder");
    assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_ok());
    gwt::cli::execution_state::settle(
        &session.worktree_path,
        &session.id,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    )
    .unwrap();
    let closed = control_request_settled(&owner, &pm, control_fleet_request(&target));
    assert!(closed
        .events
        .iter()
        .any(|event| event["kind"] == "fleet_pane_close_result" && event["ok"] == true));
    assert!(target.pty.try_wait().unwrap().is_some());
    control_close(&owner, &pm);
}

#[test]
fn fleet_pane_close_protects_claim_acquired_after_inventory() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pm) = control_owner_agent(&project, "pm", 42);
    let mut inspection = control_launch_config(&project, 43);
    inspection.linked_issue_number = None;
    inspection.explicit_follow_up = false;
    let target = control_owner_launch(&owner, "target", inspection);
    control_register_pm(&project, &pm);
    let selected = control_request(&owner, &pm, AgentFrontendRequest::ListFleetPanes);
    assert!(selected.events[0]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["window_id"] == target.id));
    gwt::mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&project),
        |prefs| {
            prefs.launched_issues.push(gwt::IssueMonitorLaunchedIssue {
                issue_number: 4179,
                window_id: target.id.clone(),
            });
        },
    )
    .unwrap();
    assert_fleet_refused(
        &control_request_settled(&owner, &pm, control_fleet_request(&target)),
        "claim holder",
    );
    assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_ok());
    control_close(&owner, &target);
    control_close(&owner, &pm);
}

#[test]
fn fleet_pane_close_refuses_busy_lifecycle_fence_and_recovers() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pm) = control_owner_agent(&project, "pm", 42);
    let mut inspection = control_launch_config(&project, 43);
    inspection.linked_issue_number = None;
    inspection.explicit_follow_up = false;
    let target = control_owner_launch(&owner, "target", inspection);
    control_register_pm(&project, &pm);
    gwt::cli::trusted_store::with_write_lease(
        &target.session.as_ref().unwrap().worktree_path,
        || {
            let refused = control_request_settled(&owner, &pm, control_fleet_request(&target));
            assert_fleet_refused(&refused, "lease");
            assert_fleet_refused(&refused, "retry");
            assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_ok());
            Ok(())
        },
    )
    .unwrap();
    let closed = control_request_settled(&owner, &pm, control_fleet_request(&target));
    assert!(closed
        .events
        .iter()
        .any(|event| event["kind"] == "fleet_pane_close_result" && event["ok"] == true));
    assert!(target.pty.try_wait().unwrap().is_some());
    control_close(&owner, &pm);
}

#[test]
fn fleet_pane_close_matches_absent_runtime_without_wildcards() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let target = control_retained_husk(&project, "target");
    let (owner, pm) = control_owner_agent(&project, "pm", 42);
    control_register_pm(&project, &pm);
    let inventory = control_request(&owner, &pm, AgentFrontendRequest::ListFleetPanes);
    let target_row = inventory.events[0]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["window_id"] == target.id)
        .unwrap();
    assert_eq!(target_row["runtime_present"], false);
    assert!(target_row["child_pid"].is_null());
    let request = AgentFrontendRequest::CloseFleetPane {
        request_id: "84ef22cc-44ef-415d-b4c4-06a6d4cc7416".into(),
        id: target.id.clone(),
        session_id: target.session.id.clone(),
        child_pid: None,
        child_started_at: None,
    };
    let reply = control_request_settled(&owner, &pm, request);
    assert!(
        reply
            .events
            .iter()
            .any(|event| event["kind"] == "fleet_pane_close_result" && event["ok"] == true),
        "{:?}",
        reply.events
    );
    let remaining = control_request(&owner, &pm, AgentFrontendRequest::ListWindows);
    assert!(
        remaining.events[0]["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != target.id)
    );
    control_close(&owner, &pm);
}

#[test]
fn recover_restored_window_protects_replacement_and_normal_launches() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _custom_agents = ScopedEnvVar::unset(gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV);
    let project = temp.path().join("project");
    let source = control_retained_husk(&project, "restored");
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let source_path = sessions.join(format!("{}.toml", source.session.id));
    let mut saved = gwt_agent::Session::load(&source_path).unwrap();
    // The benign Custom provider's native conversation is fixture metadata;
    // its physical Session and retained child proof were created by the owner.
    saved.agent_session_id = Some("fixture-native-conversation".into());
    saved.save(&sessions).unwrap();
    fs::write(
        saved.worktree_path.join("WORK.md"),
        "retained in-progress provider fixture\n",
    )
    .unwrap();
    gwt_agent::save_stored_custom_agents_to_path(
        &gwt_core::paths::gwt_config_path(),
        &[gwt_agent::StoredCustomAgent::new(control_custom_agent())],
    )
    .unwrap();
    let (owner, caller) = control_owner_agent(&project, "caller", 42);
    let normal = control_owner_launch(&owner, "normal", control_launch_config(&project, 43));
    let request = |pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
                   session_id: String,
                   birth: u64| AgentFrontendRequest::RecoverRestoredWindow {
        id: pane.id.clone(),
        session_id,
        child_pid: pane.pty.process_id().unwrap(),
        child_started_at: birth,
    };
    let normal_birth = normal
        .pty
        .process_id()
        .and_then(gwt::process::host_process_start_time)
        .unwrap();
    let assert_refused = |reply: gwt::project_runtime::test_support::OwnerAgentReply,
                          expected: &str| {
        assert_eq!(reply.close, None);
        assert!(
            reply
                .events
                .iter()
                .any(|event| event["kind"] == "pane_close_result"
                    && event["ok"] == false
                    && event["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.contains(expected))),
            "{:?}",
            reply.events
        );
    };
    assert_refused(
        control_request_settled(
            &owner,
            &caller,
            request(&normal, "replaced-session".into(), normal_birth),
        ),
        "session changed",
    );
    let mutation = ControlSessionMutation::new(&normal);
    let mut negative = mutation.original.as_ref().unwrap().clone();
    for origin in [
        gwt_agent::SessionLaunchOrigin::Launch,
        gwt_agent::SessionLaunchOrigin::UserRestart,
        gwt_agent::SessionLaunchOrigin::Unknown,
    ] {
        negative.launch_origin = origin;
        negative.save(&sessions).unwrap();
        assert_refused(
            control_request_settled(
                &owner,
                &caller,
                request(&normal, negative.id.clone(), normal_birth),
            ),
            "automatic restore",
        );
    }
    negative.launch_origin = gwt_agent::SessionLaunchOrigin::AutomaticRestore;
    negative.restore_source_session_id = None;
    negative.save(&sessions).unwrap();
    assert_refused(
        control_request_settled(
            &owner,
            &caller,
            request(&normal, negative.id.clone(), normal_birth),
        ),
        "automatic restore",
    );
    mutation.restore();
    let mut restore_events = owner.subscribe();
    let receipt = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::RestoreSavedSession {
            id: source.id.clone(),
            operation_id: "actual-recovery-restore".into(),
            source_session_id: saved.id.clone(),
            expected_source: Box::new(saved.clone()),
            origin: gwt_agent::SessionLaunchOrigin::AutomaticRestore,
            cols: 80,
            rows: 24,
        },
    ))
    .unwrap();
    let mut restore_failures = Vec::new();
    while let Ok(event) = restore_events.try_recv() {
        if let gwt::project_runtime::events::OwnerEvent::LaunchFailed { id, detail, .. } = event {
            if id == source.id {
                restore_failures.push(detail);
            }
        }
    }
    assert!(
        receipt.get("restore_refusal").is_none(),
        "actual restore must be admitted: {receipt:?}; failures: {restore_failures:?}"
    );
    let restored =
        owner_fixture_block_on(owner.probe(&source.id, receipt["incarnation"].as_u64().unwrap()))
            .unwrap();
    let restored_saved = gwt_agent::Session::load(
        &sessions.join(format!("{}.toml", restored.session.as_ref().unwrap().id)),
    )
    .unwrap();
    assert_eq!(
        restored_saved.launch_origin,
        gwt_agent::SessionLaunchOrigin::AutomaticRestore
    );
    assert_eq!(
        restored_saved.restore_source_session_id.as_deref(),
        Some(saved.id.as_str())
    );
    let birth = restored
        .pty
        .process_id()
        .and_then(gwt::process::host_process_start_time)
        .unwrap();
    assert_refused(
        control_request_settled(
            &owner,
            &caller,
            request(&restored, restored_saved.id.clone(), birth + 1),
        ),
        "process changed",
    );
    let foreign_project = temp.path().join("foreign");
    control_seed_foreign_roster(&foreign_project);
    let (foreign_owner, foreign) = control_owner_agent(&foreign_project, "foreign", 44);
    assert_refused(
        control_request_settled(
            &foreign_owner,
            &foreign,
            request(&restored, restored_saved.id.clone(), birth),
        ),
        "project scope",
    );
    assert_refused(
        control_request_settled(
            &owner,
            &restored,
            request(&restored, restored_saved.id.clone(), birth),
        ),
        "correlated acceptance",
    );
    assert!(owner_fixture_block_on(owner.probe(&restored.id, restored.incarnation)).is_ok());
    let closed = control_request_settled(
        &owner,
        &caller,
        request(&restored, restored_saved.id, birth),
    );
    assert_eq!(
        closed
            .events
            .iter()
            .filter(|event| event["kind"] == "pane_close_result" && event["ok"] == true)
            .count(),
        1
    );
    assert!(restored.pty.try_wait().unwrap().is_some());
    control_close(&owner, &normal);
    control_close(&owner, &caller);
    control_close(&foreign_owner, &foreign);
}

/// Issue #5248 AC-2/AC-3: stop has already removed the Monitor binding, so
/// successful pane close must still publish the complete surviving canvas.
#[cfg(unix)]
#[test]
fn stopped_monitor_pane_close_publishes_complete_own_project_canvas() {
    use std::collections::BTreeSet;
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (project, owner, delivery_id) =
        owner_monitor_fixture(temp.path(), 5248, false, &["pane-contract-fixture"]);
    let target = owner_monitor_launch(&project, &owner, 5248, "agent-closed", delivery_id);
    let mut config = control_launch_config(&project, 5249);
    config.launch_route = gwt_agent::LaunchRoute::Autonomous;
    let survivor = control_owner_launch(&owner, "agent-live", config);
    let pm = control_owner_launch(&owner, "pm", control_launch_config(&project, 5250));
    control_register_pm(&project, &pm);
    let foreign_project = temp.path().join("foreign-project");
    control_seed_foreign_roster(&foreign_project);
    let (foreign_owner, foreign) = control_owner_agent(&foreign_project, "agent-foreign", 5251);
    let before = owner_monitor_read(&project, 5248);
    let source = before.launch_identity(5248);
    gwt::daemon_publisher::publish_durable_issue_monitor_control(&project,
        gwt::runtime_daemon_events::issue_monitor_payload("control", serde_json::json!({"window_closed": {
            "window_id":target.id,"issue_number":5248,"claim_id":source.claim_id,"delivery_id":source.delivery_id,
        }}), std::process::id())).unwrap();
    assert_eq!(
        owner_monitor_read(&project, 5248).launched_window_issue(&target.id),
        None
    );
    let closed = control_request_settled(
        &owner,
        &pm,
        AgentFrontendRequest::CloseWindow {
            id: target.id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(closed
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == true));
    assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_err());
    assert!(target.pty.try_wait().unwrap().is_some());
    owner_fixture_block_on(owner.wait_monitor_idle()).unwrap();
    let snapshot = owner
        .monitor_snapshot()
        .unwrap()
        .expect("the canonical Monitor driver received its canvas");
    let own_key = survivor.id.split_once("::").unwrap().0;
    assert_eq!(snapshot.project_tab_id, own_key);
    assert!(
        snapshot
            .windows
            .iter()
            .find(|window| window.window_id == survivor.id)
            .unwrap()
            .monitor_owned,
        "the complete surviving canvas retains autonomous Monitor provenance"
    );
    assert_eq!(
        snapshot
            .windows
            .iter()
            .map(|window| window.window_id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([survivor.id.as_str(), pm.id.as_str()]),
        "the closed pane and foreign project leave the canvas; every own surviving pane remains"
    );
    assert!(
        snapshot
            .windows
            .iter()
            .all(|window| window.window_id.split_once("::").unwrap().0 == own_key),
        "the complete canvas is fixed to its own project"
    );
    assert!(owner_fixture_block_on(foreign_owner.probe(&foreign.id, foreign.incarnation)).is_ok());
    control_close(&foreign_owner, &foreign);
    control_close(&owner, &survivor);
    control_close(&owner, &pm);
    owner_fixture_block_on(owner.shutdown());
}

/// Issue #3705 AC-1/AC-2: consecutive close of live-PTY panes must keep the
/// agent pane route answering. The hang was the GUI event loop joining PTY
/// reader threads and waiting for child exit on each close; `pane.list` then
/// sat behind that queue until the websocket timed out.
#[test]
fn consecutive_live_pty_pane_closes_keep_agent_route_responsive() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    control_close_series(&project, 4);
}

/// Issue #3783 AC-2/AC-3/AC-4: accepting a pane close is an in-memory
/// lifecycle transition. PTY termination and durable Work/Session cleanup
/// belong to independent background finalizers, so neither a large projection
/// nor two consecutive closes can delay the pane bridge acknowledgement.
#[test]
fn consecutive_agent_pane_closes_queue_finalizers_before_disk_projection_work() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    super::super::workspace_views::reset_full_active_work_projection_builds();
    control_close_series(&project, 2);
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "the actual owner route must not call the GUI's disk-backed projection builder"
    );
}

/// Issue #3783: closing a whole project tab is the same accepted lifecycle
/// transition as closing its panes one by one. The Tao thread removes the tab
/// and queues one finalizer per live runtime without killing or waiting on a
/// PTY inline.
#[test]
fn close_project_tab_queues_all_window_finalizers_before_pty_teardown() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Agent],
    );
    let raw_window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect::<Vec<_>>();
    let window_ids = raw_window_ids
        .iter()
        .map(|raw_id| combined_window_id("tab-project", raw_id))
        .collect::<Vec<_>>();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut ptys = Vec::new();
    for (index, (raw_id, window_id)) in raw_window_ids.iter().zip(&window_ids).enumerate() {
        runtime.register_window("tab-project", raw_id);
        insert_test_pane_runtime(&mut runtime, window_id);
        ptys.push(owner_runtime_probe(&runtime, window_id).pty.clone());
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: format!("session-project-close-{index}"),
                agent_id: "codex".to_string(),
                branch_name: format!("work/project-close-{index}"),
                display_name: "Codex".to_string(),
                worktree_path: project.clone(),
                agent_project_root: project.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-project".to_string(),
            },
        );
    }

    let events = runtime.close_project_tab_events("tab-project");

    assert!(
        !events.is_empty(),
        "the accepted tab close is broadcast immediately"
    );
    assert!(
        runtime.tabs.is_empty(),
        "the tab is removed at the ACK boundary"
    );
    assert!(
        runtime.runtimes.is_empty(),
        "runtime ownership moves into finalizers"
    );
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        window_ids.len(),
        "each live pane owns one queued finalizer"
    );
    assert!(
        ptys.iter()
            .all(|pty| pty.try_wait().expect("probe child").is_none()),
        "project close must not kill PTYs on the Tao thread"
    );

    let queued = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .drain(..)
        .collect::<Vec<_>>();
    for task in queued {
        task();
    }
    assert!(
        ptys.iter()
            .all(|pty| pty.try_wait().expect("probe reaped child").is_some()),
        "queued project-close finalizers terminate and reap every PTY"
    );
    assert!(
        runtime
            .window_lifecycle_generations
            .lock()
            .expect("window lifecycle generations")
            .is_empty(),
        "completed finalizers settle every closed window generation"
    );
}

/// Issue #4234 AC-3: closing a pane must release every buffer keyed by that
/// pane. Before this, an ordinary close dropped the runtime and the session but
/// left ~20 window-keyed maps holding an entry per window for the life of the
/// process — and because window ids are reassigned lowest-free, a later window
/// with the same id would read that residue as its own state.
#[test]
fn closing_a_window_releases_every_window_scoped_buffer() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, _finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    insert_test_pane_runtime(&mut runtime, &window_id);
    seed_window_scoped_state(&mut runtime, &window_id);
    assert!(
        !runtime.window_scoped_state_residue(&window_id).is_empty(),
        "the fixture must actually populate window-scoped state"
    );

    assert!(runtime.close_window_outcome(&window_id).closed);

    let residue = runtime.window_scoped_state_residue(&window_id);
    assert!(
        residue.is_empty(),
        "closing {window_id} left window-scoped state behind: {residue:?}"
    );
}

/// Issue #4234 AC-4: opening and closing panes N times returns the window-keyed
/// state to its starting level instead of growing once per window.
#[test]
fn opening_and_closing_windows_returns_window_scoped_state_to_baseline() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab("tab-project", "Project", project, ProjectKind::Git, &[]);
    let (mut runtime, recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let baseline = runtime.window_scoped_state_entry_count();
    let mut opened = Vec::new();
    for _ in 0..8 {
        let raw_id = runtime
            .tab_mut("tab-project")
            .expect("tab")
            .workspace
            .add_window(WindowPreset::Agent)
            .id;
        let id = combined_window_id("tab-project", &raw_id);
        runtime.register_window("tab-project", &raw_id);
        insert_test_pane_runtime(&mut runtime, &id);
        let pty = owner_runtime_probe(&runtime, &id).pty;
        seed_window_scoped_state(&mut runtime, &id);
        assert!(runtime.close_window_outcome(&id).closed);
        // The same physical id becomes reusable only after the real close ACK.
        drain_queued_blocking_tasks(&finalizers);
        apply_recorded_window_close_finalized(&mut runtime, &recorded);
        assert!(pty.try_wait().unwrap().is_some());
        opened.push(id);
    }

    let residue = opened
        .iter()
        .flat_map(|id| {
            runtime
                .window_scoped_state_residue(id)
                .into_iter()
                .map(move |field| format!("{id}:{field}"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        runtime.window_scoped_state_entry_count(),
        baseline,
        "window-scoped state grew across 8 open/close rounds; residue: {residue:?}"
    );
}

/// Issue #3783 generation fence: a queued predecessor close may finish after
/// the canvas has reused the same public window id for a successor. The old
/// finalizer owns only its captured PTY and must not deregister or revoke the
/// successor writer generation.
#[test]
fn queued_close_finalizer_preserves_same_window_successor_writer_generation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, predecessor) = control_owner_agent(&project, "agent", 42);
    let old_writer = predecessor.writer.as_ref().unwrap().clone();
    control_close(&owner, &predecessor);
    let successor = control_owner_launch(&owner, "agent", control_launch_config(&project, 43));
    assert_ne!(successor.incarnation, predecessor.incarnation);
    assert!(owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Close {
            id: predecessor.id.clone(),
            incarnation: predecessor.incarnation,
        }
    ))
    .is_err());
    let current =
        owner_fixture_block_on(owner.probe(&successor.id, successor.incarnation)).unwrap();
    let writer = current.writer.unwrap();
    assert!(Arc::ptr_eq(&writer.handle, &successor.pty));
    assert!(Arc::ptr_eq(&writer.current, &successor.current));
    assert!(successor.current.load(std::sync::atomic::Ordering::Acquire));
    assert!(!old_writer
        .current
        .load(std::sync::atomic::Ordering::Acquire));
    assert!(
        successor.pty.reserve_input_transaction().is_ok(),
        "the predecessor close replay must not revoke successor input"
    );
    assert!(predecessor.pty.try_wait().unwrap().is_some());
    assert!(successor.pty.try_wait().unwrap().is_none());
    control_close(&owner, &successor);
}

/// Issue #3783 authority fence: an unreadable predecessor Session is not
/// evidence that the Session id is unbound. A later same-id successor must
/// remain Running when the old local PTY finalizer eventually executes.
#[test]
fn queued_close_finalizer_with_unavailable_predecessor_session_preserves_same_id_successor() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, predecessor) = control_owner_agent(&project, "agent", 42);
    let mutation = ControlSessionMutation::new(&predecessor);
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let id = predecessor.session.as_ref().unwrap().id.clone();
    fs::write(
        sessions.join(format!("{id}.toml")),
        b"not valid session toml",
    )
    .unwrap();
    let refused = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Close {
            id: predecessor.id.clone(),
            incarnation: predecessor.incarnation,
        },
    ));
    let predecessor_survived = predecessor.pty.try_wait().unwrap().is_none();
    mutation.restore();
    let recovered = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Close {
            id: predecessor.id.clone(),
            incarnation: predecessor.incarnation,
        },
    ));
    assert!(
        refused.is_err(),
        "an unreadable producing source must never be treated as an unbound Session"
    );
    assert!(
        predecessor_survived,
        "unproven authority must refuse before mutating its physical child"
    );
    recovered.expect("the exact retained retirement must recover after fixture source repair");
    let mut resume = control_launch_config(&project, 42);
    resume.session_mode = gwt_agent::SessionMode::Continue;
    resume.predecessor_session_id = Some(id.clone());
    let successor = control_owner_launch(&owner, "agent", resume);
    let before = gwt_agent::Session::load(&sessions.join(format!("{id}.toml"))).unwrap();
    assert_eq!(before.id, successor.session.as_ref().unwrap().id);
    assert_eq!(before.status, gwt_agent::AgentStatus::Running);
    assert!(before.restore_window_on_startup);
    assert!(owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Close {
            id: predecessor.id,
            incarnation: predecessor.incarnation,
        }
    ))
    .is_err());
    let after = gwt_agent::Session::load(&sessions.join(format!("{id}.toml"))).unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(before).unwrap(),
        "a stale predecessor completion must preserve the exact current Session"
    );
    assert!(successor.pty.try_wait().unwrap().is_none());
    control_close(&owner, &successor);
}

#[test]
fn close_finalizer_falls_back_when_blocking_spawner_fails() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("injected close scheduler failure");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = owner_runtime_probe(&runtime, &window_id).pty.clone();

    assert!(!runtime
        .issue_monitor_idle_pane_close_events(&window_id, Some(4117), "review_verdict_published")
        .is_empty());

    let deadline = Instant::now() + Duration::from_secs(3);
    while pty.try_wait().expect("probe fallback child").is_none() {
        assert!(
            Instant::now() < deadline,
            "fallback finalizer must terminate and reap the detached PTY"
        );
        // test-hygiene: allow-short-duration bounded OS child-exit polling; the detached child has no exit notification in this fixture
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn close_finalizer_queues_on_process_worker_when_both_schedulers_fail() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("injected close scheduler failure");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = owner_runtime_probe(&runtime, &window_id).pty.clone();
    let release_process_worker =
        super::super::pty_io::hold_process_close_finalizer_worker_for_test();
    let _raw_spawn_failure =
        super::super::pty_io::force_close_finalizer_thread_spawn_failure_for_test();

    assert!(!runtime
        .close_window_after_issue_monitor_finalize_events(&window_id)
        .is_empty());
    assert!(
        pty.try_wait().expect("probe queued child").is_none(),
        "an accepted close must not run its finalizer inline when both per-close schedulers fail"
    );

    drop(release_process_worker);
    let deadline = Instant::now() + Duration::from_secs(3);
    while pty
        .try_wait()
        .expect("probe process-worker child")
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "the process-owned worker must eventually terminate and reap the detached PTY"
        );
        // test-hygiene: allow-short-duration bounded OS child-exit polling after explicit worker release; state observation establishes completion
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn exact_close_finalizer_holds_handoff_through_generation_gated_cleanup() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let session = pane.session.as_ref().unwrap();
    let hook_issuer = pane.issuer.clone();
    let hook_project = project.clone();
    let hook_session = session.id.clone();
    let hook_binding = session.execution_binding.as_ref().unwrap().clone();
    let hook_owner = owner.clone();
    let id = pane.id.clone();
    let mut config = control_launch_config(&project, 42);
    config.session_mode = gwt_agent::SessionMode::Continue;
    config.predecessor_session_id = Some(session.id.clone());
    let hook_config = config.clone();
    let (observed_tx, observed_rx) = mpsc::sync_channel(1);
    let _hook = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        pane.grant.as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::BeforeRegistryCleanup,
        move || {
            let issuing_refused = hook_issuer
                .issue_bound(&hook_project, &hook_session, hook_binding)
                .is_err();
            let replacement_refused = owner_fixture_block_on(hook_owner.command(
                gwt::project_runtime::events::OwnerCommand::LaunchAgent {
                    id: id.clone(),
                    operation_id: "too-early-same-session-successor".into(),
                    tab_id: id.split_once("::").unwrap().0.into(),
                    config: Box::new(hook_config),
                    metadata: Box::default(),
                    cols: 80,
                    rows: 24,
                },
            ))
            .is_err();
            observed_tx
                .send((issuing_refused, replacement_refused))
                .unwrap();
        },
    );
    control_close(&owner, &pane);
    let (issuing_refused, replacement_refused) = gwt_core::test_support::recv_event(
        &observed_rx,
        "real cleanup reached its exact handoff observation",
    );
    assert!(
        issuing_refused,
        "exact capability handoff must stay held through canonical registry cleanup"
    );
    assert!(
        replacement_refused,
        "a same-id physical successor must wait until cleanup ACK"
    );
    assert!(pane.pty.try_wait().unwrap().is_some());
    assert!(
        pane.issuer
            .issue_bound(
                &project,
                &session.id,
                session.execution_binding.as_ref().unwrap().clone()
            )
            .is_ok(),
        "the committed cleanup ACK must release the exact holder handoff"
    );
    let successor = control_owner_launch(&owner, "agent", config);
    assert_eq!(successor.session.as_ref().unwrap().id, session.id);
    assert_ne!(successor.incarnation, pane.incarnation);
    assert!(
        owner_fixture_block_on(
            owner.command(gwt::project_runtime::events::OwnerCommand::Close {
                id: pane.id,
                incarnation: pane.incarnation,
            })
        )
        .is_err(),
        "old close replay must not remove the actual same-Session successor"
    );
    assert!(owner_fixture_block_on(owner.probe(&successor.id, successor.incarnation)).is_ok());
    assert!(successor.pty.reserve_input_transaction().is_ok());
    control_close(&owner, &successor);
}

/// Issue #3755 AC-1/AC-2: one pane whose output/status worker currently owns
/// the Pane mutex must not hold the Tao event loop hostage. The authenticated
/// read sync still has to return an unrelated launch-error pane and finish the
/// request with an explicit availability receipt.
#[test]
fn agent_pane_sync_skips_a_contended_runtime_without_hiding_error_panes() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, busy) = control_owner_agent(&project, "busy", 375501);
    let caller = control_owner_launch(&owner, "reader", control_launch_config(&project, 375502));
    let error_id = owner.pane_id("failed");
    let mut failed = control_launch_config(&project, 375503);
    // Retained manual diagnostics are inspection launches. A failed producing
    // genesis correctly compensates its Session instead of retaining this row.
    failed.linked_issue_number = None;
    failed.explicit_follow_up = false;
    // A missing target can fail after the real PTY gate has started. Fail the
    // gate executable itself to exercise the pre-PTY installation diagnostic.
    owner_fixture_block_on(
        owner.set_pty_start_gate(project.join("nonexistent-agent-executable"), Vec::new()),
    )
    .unwrap();
    let result = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: error_id.clone(),
            operation_id: "failed-snapshot-launch".into(),
            tab_id: error_id.split_once("::").unwrap().0.into(),
            config: Box::new(failed),
            metadata: Box::default(),
            cols: 80,
            rows: 24,
        },
    ));
    let receipt = result.expect("actual retained manual launch-failure receipt");
    let failure: gwt::project_runtime::events::OwnerLaunchFailure =
        serde_json::from_value(receipt["launch_failure"].clone())
            .unwrap_or_else(|error| panic!("exact manual diagnostic required: {error}; {receipt}"));
    assert_eq!(failure.id, error_id);
    assert_eq!(receipt["incarnation"], failure.incarnation);
    assert_eq!(receipt["session_id"], failure.session_id);
    assert!(failure.detail.contains("nonexistent-agent-executable"));
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert_eq!(inventory["panes"].as_array().unwrap().len(), 2);
    assert!(inventory["panes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|pane| pane["id"] != error_id));
    assert!(inventory["pending"].as_array().unwrap().is_empty());
    let (locked_tx, locked_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let pane = busy.pane.clone();
    let holder = thread::spawn(move || {
        let _guard = pane.lock().unwrap();
        locked_tx.send(()).unwrap();
        gwt_core::test_support::recv_event(&release_rx, "authenticated Ready returned");
    });
    locked_rx.recv().unwrap();
    let mut reply = None;
    let logs = control_tracing_events(&caller, || {
        reply = Some(control_request(
            &owner,
            &caller,
            AgentFrontendRequest::Ready,
        ));
    });
    let reply = reply.unwrap();
    // The lock cannot release before the real dispatch returned. This proves
    // nonblocking behavior without a runner-speed assertion.
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(reply.close, None);
    assert!(
        reply
            .events
            .iter()
            .any(|event| event["kind"] == "terminal_snapshot" && event["id"] == error_id),
        "a captured failed-launch diagnostic must survive another busy pane: {:?}",
        reply.events
    );
    let complete = reply
        .events
        .iter()
        .find(|event| event["kind"] == "pane_sync_complete")
        .unwrap();
    assert_eq!(complete["busy_window_ids"], serde_json::json!([busy.id]));
    for key in [
        "empty_window_ids",
        "unavailable_window_ids",
        "failed_window_ids",
    ] {
        assert_eq!(complete[key], serde_json::json!([]));
    }
    assert!(logs.iter().any(|event| event.target == "gwt.pane.sync"
        && event.fields.get("outcome").map(String::as_str) == Some("busy")
        && event.fields.contains_key("elapsed_ms")));
    control_close(&owner, &busy);
    control_close(&owner, &caller);
}

/// Issue #3755 AC-2: completion outcomes are disjoint. A blank live terminal
/// still has a replayable vt100 snapshot, a missing runtime is unavailable,
/// and a poisoned parser mutex is an internal failure rather than busy.
#[test]
fn agent_pane_sync_completion_distinguishes_snapshot_unavailable_and_poisoned() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    control_init_repo(&project);
    // A saved window without a Session or runtime is truthful startup input,
    // rather than a fabricated running process proof.
    let tab = sample_project_tab_with_window_at(
        "saved",
        "unavailable",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    gwt::persistence::save_workspace_state(
        &gwt::persistence::workspace_state_path(&project),
        tab.workspace.persisted(),
    )
    .unwrap();
    let (owner, empty) = control_owner_agent(&project, "empty", 375504);
    let failed = control_owner_launch(&owner, "poisoned", control_launch_config(&project, 375505));
    let failed_pane = failed.pane.clone();
    let _ = thread::spawn(move || {
        let _guard = failed_pane.lock().unwrap();
        panic!("intentional pane mutex poison");
    })
    .join();
    let mut reply = None;
    let logs = control_tracing_events(&empty, || {
        reply = Some(control_request(&owner, &empty, AgentFrontendRequest::Ready));
    });
    let reply = reply.unwrap();
    assert_eq!(reply.close, None);
    assert!(reply
        .events
        .iter()
        .any(|event| event["kind"] == "terminal_snapshot" && event["id"] == empty.id));
    let complete = reply
        .events
        .iter()
        .find(|event| event["kind"] == "pane_sync_complete")
        .unwrap();
    assert_eq!(complete["empty_window_ids"], serde_json::json!([]));
    assert_eq!(complete["busy_window_ids"], serde_json::json!([]));
    assert_eq!(
        complete["unavailable_window_ids"],
        serde_json::json!([owner.pane_id("unavailable")])
    );
    assert_eq!(
        complete["failed_window_ids"],
        serde_json::json!([failed.id])
    );
    assert!(logs.iter().any(|event| event.target == "gwt.pane.sync"
        && event.fields.get("outcome").map(String::as_str) == Some("poisoned")
        && event.fields.contains_key("elapsed_ms")));
    // Repair only the fixture mutex poison after observing the production
    // failure receipt, so ordinary exact-owner teardown can reap the child.
    failed.pane.clear_poison();
    control_close(&owner, &failed);
    control_close(&owner, &empty);
}

/// Issue #3755 AC-3 / Issue #3988: removing mutex waits must not hide a new
/// multi-pane serialization stall. The original guard was a 300ms wall-clock
/// budget, which turned every saturated CI runner red without saying anything
/// about the invariant, so the guard is structural now. Two properties make a
/// stall impossible and both are decidable without a clock: a pane whose mutex
/// is held is reported busy instead of awaited, and every snapshot the dispatch
/// emits stays clamped to the scrollback replay cap however much the pane
/// printed. The only timeout left is a deadlock guard on the holder thread, so
/// a regression that waits on the mutex fails an assertion instead of hanging
/// the test binary.
#[test]
fn agent_pane_sync_with_full_scrollback_stays_bounded() {
    // Rows of `long_running_test_pane`, which the snapshot replays on top of
    // the capped scrollback.
    const PANE_ROWS: usize = 24;
    // The holder releases as soon as the dispatch returns, so this only bounds
    // the failing path where the dispatch waits on the mutex forever.
    const HOLDER_DEADLOCK_GUARD: Duration = Duration::from_secs(60);
    // Print well past the cap so the clamp is exercised rather than assumed.
    let printed_lines = SNAPSHOT_SCROLLBACK_REPLAY_LIMIT + PANE_ROWS + 1_000;

    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, first) = control_owner_agent(&project, "pane-0", 375510);
    let mut panes = vec![first];
    for number in 1..4 {
        panes.push(control_owner_launch(
            &owner,
            &format!("pane-{number}"),
            control_launch_config(&project, 375510 + number),
        ));
    }
    let replay = (0..printed_lines)
        .map(|line| format!("snapshot-line-{line:06}\r\n"))
        .collect::<String>();
    for pane in &panes {
        pane.pane.lock().unwrap().process_bytes(replay.as_bytes());
    }
    let contended_window_id = panes[0].id.clone();
    let contended_pane = panes[0].pane.clone();
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    let holder = thread::spawn(move || {
        let _guard = contended_pane.lock().unwrap();
        held_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(HOLDER_DEADLOCK_GUARD);
    });
    held_rx.recv().unwrap();
    let reply = control_request(&owner, &panes[1], AgentFrontendRequest::Ready);
    let _ = release_tx.send(());
    holder.join().unwrap();
    assert_eq!(reply.close, None);
    let complete = reply
        .events
        .iter()
        .find(|event| event["kind"] == "pane_sync_complete")
        .unwrap();
    assert_eq!(
        complete["busy_window_ids"],
        serde_json::json!([contended_window_id])
    );
    for key in [
        "empty_window_ids",
        "unavailable_window_ids",
        "failed_window_ids",
    ] {
        assert_eq!(complete[key], serde_json::json!([]));
    }
    let snapshots = reply
        .events
        .iter()
        .filter(|event| event["kind"] == "terminal_snapshot")
        .map(|event| {
            (
                event["id"].as_str().unwrap().to_owned(),
                event["data_base64"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut expected_snapshot_ids = panes
        .iter()
        .skip(1)
        .map(|pane| pane.id.clone())
        .collect::<Vec<_>>();
    expected_snapshot_ids.sort();
    assert_eq!(snapshots.keys().cloned().collect::<Vec<_>>(), expected_snapshot_ids,
        "every uncontended full-scrollback pane must be snapshotted by the same authenticated dispatch");
    let newest_line = format!("snapshot-line-{:06}", printed_lines - 1);
    for (window_id, data_base64) in &snapshots {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .expect("snapshot base64");
        let snapshot = String::from_utf8_lossy(&decoded);
        let replayed_lines = snapshot.matches("snapshot-line-").count();
        assert!(
            replayed_lines <= SNAPSHOT_SCROLLBACK_REPLAY_LIMIT + PANE_ROWS,
            "{window_id} replayed {replayed_lines} lines, past the \
             {SNAPSHOT_SCROLLBACK_REPLAY_LIMIT}-row scrollback replay cap"
        );
        assert!(
            snapshot.contains(&newest_line),
            "{window_id} dropped the newest line {newest_line} from its snapshot"
        );
        assert!(
            !snapshot.contains("snapshot-line-000000"),
            "{window_id} replayed a line older than the scrollback replay cap"
        );
    }
    for pane in &panes {
        control_close(&owner, pane);
    }
}

/// Issue #3755 AC-2/AC-3: close removes the runtime and window synchronously,
/// but a contended Pane mutex is cleanup work and must never be awaited by the
/// GUI event loop.
#[test]
fn agent_pane_close_bypasses_a_contended_pane_lock() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, caller) = control_owner_agent(&project, "caller", 42);
    let target = control_owner_launch(&owner, "target", control_launch_config(&project, 43));
    let target_pane = target.pane.clone();
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder = thread::spawn(move || {
        let _guard = target_pane.lock().unwrap();
        held_tx.send(()).unwrap();
        gwt_core::test_support::recv_event(
            &release_rx,
            "actual owner answered list while close finalizer was blocked",
        );
    });
    held_rx.recv().unwrap();
    let logs = capture_tracing_events(|| {
        let trace = tracing::dispatcher::get_default(Clone::clone);
        owner_fixture_block_on(async {
            let (mut socket, _) = connect_async(caller.agent_pane_request().unwrap())
                .await
                .unwrap();
            socket
                .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
                .await
                .unwrap();
            let _ = next_test_agent_websocket_event(
                &mut socket,
                "workspace_state",
                gwt_core::deadline_budget::HANG_GUARD,
            )
            .await;
            let _trace = gwt::project_runtime::agent_frontend::capture_owner_agent_trace(
                caller.grant.as_ref().unwrap(),
                gwt::project_runtime::agent_frontend::OwnerAgentTraceRequest::CloseWindow,
                trace,
            );
            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({"kind":"close_window","id":target.id})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
                .await
                .unwrap();
            let listed = next_test_agent_websocket_event(
                &mut socket,
                "workspace_state",
                gwt_core::deadline_budget::HANG_GUARD,
            )
            .await;
            assert!(listed["workspace"]["tabs"][0]["workspace"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == caller.id));
            release_tx.send(()).unwrap();
            let closed = next_test_agent_websocket_event(
                &mut socket,
                "pane_close_result",
                gwt_core::deadline_budget::HANG_GUARD,
            )
            .await;
            assert_eq!(closed["ok"], true);
            assert_eq!(closed["window_id"], target.id);
            socket.close(None).await.unwrap();
        });
    });
    holder.join().unwrap();
    assert!(target.pty.try_wait().unwrap().is_some());
    let session = target.session.as_ref().unwrap();
    let proof = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
        &gwt_core::paths::gwt_sessions_dir(),
        &session.id,
    ))
    .unwrap();
    assert_eq!(proof.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(
        proof
            .execution_identity
            .as_ref()
            .map(|identity| &identity.execution_binding),
        session.execution_binding.as_ref()
    );
    assert_eq!(proof.runtime_incarnation, Some(target.incarnation));
    let kill = logs
        .iter()
        .find(|event| {
            event.target == "gwt.pane.teardown"
                && event.fields.get("stage").map(String::as_str) == Some("pty_kill")
                && event.fields.get("outcome").map(String::as_str) == Some("completed")
        })
        .expect("actual owner kill diagnostic");
    assert_eq!(kill.fields.get("window_id"), Some(&target.id));
    assert_eq!(kill.fields.get("ok").map(String::as_str), Some("true"));
    assert!(kill.fields.contains_key("elapsed_ms"));
    control_close(&owner, &caller);
}

/// Issue #3629 AC-12: an uncorrelated self-close stays refused, but the
/// refusal must be explicit so the CLI stops guessing at session correlation.
#[test]
fn agent_pane_close_reports_uncorrelated_self_close_refusal() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let refused = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::CloseWindow {
            id: pane.id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(refused
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result"
            && event["ok"] == false
            && event["reason"]
                .as_str()
                .unwrap()
                .contains("correlated acceptance")));
    assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_ok());
    assert!(pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    control_close(&owner, &pane);
}

/// Issue #3503 / #3552 AC-1: the pane the PM could never clean up had failed
/// *before* its PTY started, so it carries no session binding at all. The
/// self-session protection compares the window's real binding, so `None` is
/// not "this may be you" — it is a peer pane and it closes. Locking both
/// halves in one test keeps the guard identity-shaped, so it cannot drift back
/// into the state- or authority-shaped gate that made every PM cleanup close
/// fail regardless of the target pane's state. `pane.stop` resolves to this
/// same command, so it is covered here too.
#[test]
fn agent_pane_close_removes_a_launch_failed_peer_pane_without_session_binding() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    control_init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "saved",
        "failed",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    gwt::persistence::save_workspace_state(
        &gwt::persistence::workspace_state_path(&project),
        tab.workspace.persisted(),
    )
    .unwrap();
    let (owner, bootstrap) = control_owner_agent(&project, "bootstrap", 42);
    let mut config = control_launch_config(&project, 43);
    config.linked_issue_number = None;
    config.explicit_follow_up = false;
    let caller = control_owner_launch(&owner, "pm", config);
    assert!(!caller
        .grant
        .as_ref()
        .unwrap()
        .principal()
        .authorizes_producing_mutation());
    let failed_id = owner.pane_id("failed");
    let reply = control_request_settled(
        &owner,
        &caller,
        AgentFrontendRequest::CloseWindow {
            id: failed_id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(
        reply
            .events
            .iter()
            .any(|event| event["kind"] == "pane_close_result"
                && event["ok"] == true
                && event["window_id"] == failed_id),
        "{:?}",
        reply.events
    );
    let refused = control_request(
        &owner,
        &caller,
        AgentFrontendRequest::CloseWindow {
            id: caller.id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(refused
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == false));
    assert!(owner_fixture_block_on(owner.probe(&caller.id, caller.incarnation)).is_ok());
    control_close(&owner, &caller);
    control_close(&owner, &bootstrap);
}

/// Issue #3629 AC-9: a husk window (workspace record without a lookup entry,
/// left behind by an app restart) must still close through the agent route.
#[test]
fn agent_pane_close_removes_husk_window_missing_from_lookup() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    control_init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "saved",
        "husk",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    gwt::persistence::save_workspace_state(
        &gwt::persistence::workspace_state_path(&project),
        tab.workspace.persisted(),
    )
    .unwrap();
    let (owner, caller) = control_owner_agent(&project, "caller", 42);
    let id = owner.pane_id("husk");
    let closed = control_request_settled(
        &owner,
        &caller,
        AgentFrontendRequest::CloseWindow {
            id: id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(
        closed
            .events
            .iter()
            .any(|event| event["kind"] == "pane_close_result"
                && event["ok"] == true
                && event["window_id"] == id),
        "{:?}",
        closed.events
    );
    let listed = control_request(&owner, &caller, AgentFrontendRequest::ListWindows);
    assert!(
        listed.events[0]["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != id)
    );
    control_close(&owner, &caller);
}

#[test]
fn agent_pane_close_removes_restored_session_husk_without_claiming_tree_exit() {
    for boundary in ["restored", "view_only"] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let project = temp.path().join("project");
        let target = control_retained_husk(&project, "historical");
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let path = sessions.join(format!("{}.toml", target.session.id));
        let mut historical = gwt_agent::Session::load(&path).unwrap();
        // Model the historical TOML left stale by a crashed old Host. This
        // scalar is not live proof: the real prior Host and child both exited.
        historical.status = gwt_agent::AgentStatus::Running;
        historical.save(&sessions).unwrap();
        let (owner, caller) = control_owner_agent(&project, "caller", 42);
        let reply = control_request_settled(
            &owner,
            &caller,
            AgentFrontendRequest::CloseWindow {
                id: target.id.clone(),
                request_id: None,
                responder: None,
            },
        );
        let durable = gwt_agent::Session::load(&path).unwrap();
        let listed = control_request(&owner, &caller, AgentFrontendRequest::ListWindows);
        control_close(&owner, &caller);
        assert_eq!(reply.close, None);
        let receipt = reply
            .events
            .iter()
            .find(|event| event["window_id"] == target.id)
            .expect("exact client-scoped historical-view result");
        assert_eq!(
            receipt["kind"], "pane_husk_removed",
            "{boundary}: {receipt:?}"
        );
        assert!(
            receipt.get("ok").is_none(),
            "view removal must not claim process-tree exit"
        );
        historical.restore_window_on_startup = false;
        assert_eq!(
            serde_json::to_value(durable).unwrap(),
            serde_json::to_value(historical).unwrap(),
            "historical Session metadata/status must stay unchanged except the restore flag"
        );
        assert!(
            listed.events[0]["workspace"]["tabs"][0]["workspace"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["id"] != target.id)
        );
        let reference = BackendEvent::PaneCloseResult {
            ok: false,
            window_id: String::new(),
            reason: None,
        }
        .delivery_policy();
        let husk = BackendEvent::PaneHuskRemoved {
            window_id: target.id,
        }
        .delivery_policy();
        assert_eq!(husk.delivery, reference.delivery);
        assert_eq!(husk.backpressure, reference.backpressure);
    }
    // The canonical historical-view guard must reject real live/current-Host
    // evidence. Raw GUI rows cannot bypass that guard after owner normalization.
    for boundary in ["running", "live", "current_host", "unknown"] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let project = temp.path().join("project");
        if boundary == "unknown" {
            let target = control_retained_husk(&project, "uncertain");
            let sessions = gwt_core::paths::gwt_sessions_dir();
            let path = gwt_agent::runtime_state_path_for_pid(
                &sessions,
                target.owner_pid,
                &target.session.id,
            );
            let original = fs::read(&path).unwrap();
            let snapshot =
                gwt_agent::Session::load(&sessions.join(format!("{}.toml", target.session.id)))
                    .unwrap();
            let before = fs::read(sessions.join(format!("{}.toml", target.session.id))).unwrap();
            fs::write(&path, b"unreadable actual prior runtime proof").unwrap();
            let result = gwt::session_finalizer::remove_restored_stopped_view(&sessions, &snapshot);
            fs::write(&path, original).unwrap();
            assert!(!matches!(result, Ok(true)));
            assert_eq!(
                fs::read(sessions.join(format!("{}.toml", target.session.id))).unwrap(),
                before
            );
            continue;
        }
        let (owner, pane) = control_owner_agent(&project, "current", 42);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let session_id = pane.session.as_ref().unwrap().id.clone();
        if boundary == "current_host" {
            owner_fixture_block_on(Arc::try_unwrap(owner).ok().unwrap().shutdown());
            assert!(pane.pty.try_wait().unwrap().is_some());
            let current =
                gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))).unwrap();
            let before = fs::read(sessions.join(format!("{session_id}.toml"))).unwrap();
            assert!(
                !gwt::session_finalizer::remove_restored_stopped_view(&sessions, &current).unwrap()
            );
            assert_eq!(
                fs::read(sessions.join(format!("{session_id}.toml"))).unwrap(),
                before
            );
        } else {
            assert!(pane.pty.try_wait().unwrap().is_none());
            let current =
                gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))).unwrap();
            let before = fs::read(sessions.join(format!("{session_id}.toml"))).unwrap();
            assert!(
                !gwt::session_finalizer::remove_restored_stopped_view(&sessions, &current).unwrap()
            );
            assert_eq!(
                fs::read(sessions.join(format!("{session_id}.toml"))).unwrap(),
                before
            );
            assert!(pane.pty.try_wait().unwrap().is_none());
            control_close(&owner, &pane);
        }
    }
}

#[test]
fn queued_agent_pane_request_rechecks_generation_before_runtime_dispatch() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let queued = pane.grant.as_ref().unwrap().clone();
    let session = pane.session.as_ref().unwrap();
    let rotated = pane.issuer.issue(&project, &session.id).unwrap();
    let stale_client = owner_fixture_block_on(owner.agent_client("stale-inspector")).unwrap();
    let stale =
        owner_fixture_block_on(stale_client.request(queued.clone(), AgentFrontendRequest::Ready))
            .unwrap();
    assert_eq!(
        stale.close,
        Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE),
        "a queued request whose owner capability generation was rotated must be rejected as stale"
    );
    let shared_stale = gwt::project_runtime::agent_frontend::dispatch_if_current(
        Some(&pane.issuer),
        &gwt_core::paths::gwt_sessions_dir(),
        "stale-inspector".into(),
        queued,
        AgentFrontendRequest::Ready,
        |_, _| panic!("shared gate dispatched a revoked queued grant"),
    );
    assert!(matches!(
        shared_stale,
        gwt::project_runtime::agent_frontend::AgentFrontendDispatchOutcome::StaleCapability
    ));
    let current = pane.issuer.grant_for_test(&rotated.token).unwrap();
    let client = owner_fixture_block_on(owner.agent_client("current-inspector")).unwrap();
    let reply =
        owner_fixture_block_on(client.request(current, AgentFrontendRequest::Ready)).unwrap();
    assert_eq!(reply.close, None);
    assert!(
        reply
            .events
            .iter()
            .any(|event| event["kind"] == "workspace_state"),
        "the current observation grant must still receive its scoped snapshot"
    );
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::Close {
            id: pane.id,
            incarnation: pane.incarnation,
        }),
    )
    .unwrap();
}

#[test]
fn current_peer_pane_close_releases_caller_grant_lock_before_target_handoff() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "caller", 42);
    let target = control_owner_launch(&owner, "target", control_launch_config(&project, 43));
    let old_grant = pane.grant.as_ref().unwrap().clone();
    let session = pane.session.as_ref().unwrap();
    let issued = pane
        .issuer
        .issue_bound(
            &project,
            &session.id,
            session.execution_binding.as_ref().unwrap().clone(),
        )
        .unwrap();
    let current = pane.issuer.grant_for_test(&issued.token).unwrap();
    let stale_client = owner_fixture_block_on(owner.agent_client("stale-close")).unwrap();
    let stale = owner_fixture_block_on(stale_client.request(
        old_grant,
        AgentFrontendRequest::CloseWindow {
            id: target.id.clone(),
            request_id: None,
            responder: None,
        },
    ))
    .unwrap();
    assert_eq!(
        stale.close,
        Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE)
    );
    assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_ok());
    let client = owner_fixture_block_on(owner.agent_client("accepted-close")).unwrap();
    let close = owner_fixture_block_on(client.request(
        current.clone(),
        AgentFrontendRequest::CloseWindow {
            id: target.id.clone(),
            request_id: None,
            responder: None,
        },
    ))
    .unwrap();
    assert_eq!(close.close, None);
    control_settled_self_closes(&owner);
    let mut events = close.events;
    events.extend(client.drain().unwrap().events);
    assert!(events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == true));
    let reclose = owner_fixture_block_on(client.request(
        current.clone(),
        AgentFrontendRequest::CloseWindow {
            id: target.id.clone(),
            request_id: None,
            responder: None,
        },
    ))
    .unwrap();
    assert!(reclose
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == false));
    let listed =
        owner_fixture_block_on(client.request(current, AgentFrontendRequest::ListWindows)).unwrap();
    assert_eq!(listed.close, None);
    assert!(listed
        .events
        .iter()
        .any(|event| event["kind"] == "workspace_state"));
    assert!(!target.current.load(std::sync::atomic::Ordering::Acquire));
    assert!(target.pty.try_wait().unwrap().is_some());
    // The fixture rotated this bearer outside the owner's runtime. Release
    // that exact fixture capability before the caller's normal teardown.
    assert!(pane.issuer.revoke_token(&issued.token));
    control_close(&owner, &pane);
}

/// Issue #3816: once the caller grant has crossed the acceptance snapshot,
/// rotating that caller must not cancel the already accepted peer operation.
#[test]
fn accepted_peer_close_snapshot_survives_caller_rotation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "caller", 42);
    let target = control_owner_launch(&owner, "target", control_launch_config(&project, 43));
    let (accepted_tx, accepted_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let _hook = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        pane.grant.as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::PeerCloseAccepted,
        move || {
            accepted_tx.send(()).unwrap();
            gwt_core::test_support::recv_event(&resume_rx, "caller rotated after real acceptance");
        },
    );
    let client = owner_fixture_block_on(owner.agent_client("peer-close")).unwrap();
    let grant = pane.grant.as_ref().unwrap().clone();
    let worker_owner = owner.clone();
    let target_id = target.id.clone();
    let worker_home = temp.path().to_path_buf();
    let worker = thread::spawn(move || {
        let _home = ScopedGwtHome::set(&worker_home);
        let mut reply = owner_fixture_block_on(client.request(
            grant,
            AgentFrontendRequest::CloseWindow {
                id: target_id,
                request_id: None,
                responder: None,
            },
        ))
        .unwrap();
        control_settled_self_closes(&worker_owner);
        reply.events.extend(client.drain().unwrap().events);
        reply
    });
    gwt_core::test_support::recv_event(&accepted_rx, "real dispatch crossed acceptance fence");
    let rotation_issuer = pane.issuer.clone();
    let rotation_project = project.clone();
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let (rotation_tx, rotation_rx) = mpsc::sync_channel(1);
    let rotation = thread::spawn(move || {
        rotation_tx
            .send(rotation_issuer.issue(&rotation_project, &session_id))
            .unwrap();
    });
    gwt_core::test_support::recv_event(
        &rotation_rx,
        "caller rotation must not wait for accepted target handoff",
    )
    .unwrap();
    rotation.join().unwrap();
    assert!(!pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    resume_tx.send(()).unwrap();
    let reply = worker.join().unwrap();
    assert_eq!(reply.close, None);
    assert!(reply
        .events
        .iter()
        .any(|event| event["kind"] == "pane_close_result" && event["ok"] == true));
    assert!(owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).is_err());
    assert!(target.pty.try_wait().unwrap().is_some());
    control_close(&owner, &pane);
}

/// Issue #3816 AC-3/AC-5/AC-6: exercise the real TCP WebSocket bridge, its
/// capability gate, the Tao-shaped UserEvent queue, and AppRuntime dispatch as
/// one loop. Wire-level close (also used by the parser-proven `pane.stop`
/// alias), list/read, and self pane send must remain responsive on the same
/// authenticated connection.
#[test]
fn real_agent_pane_websocket_stays_responsive_after_peer_close() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "caller", 42);
    let target = control_owner_launch(&owner, "target", control_launch_config(&project, 43));
    pane.pane
        .lock()
        .unwrap()
        .process_bytes(b"caller snapshot remains readable after peer close\r\n");
    let target_pane = target.pane.clone();
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder = thread::spawn(move || {
        let _guard = target_pane.lock().unwrap();
        held_tx.send(()).unwrap();
        gwt_core::test_support::recv_event(
            &release_rx,
            "wire close/list/read returned while target finalizer was contended",
        );
    });
    held_rx.recv().unwrap();
    let socket_request = pane.agent_pane_request().unwrap();
    owner_fixture_block_on(async {
        let (mut socket, _) = connect_async(socket_request).await.unwrap();
        socket
            .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
            .await
            .unwrap();
        let _ = next_test_agent_websocket_event(
            &mut socket,
            "workspace_state",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        for _ in 0..2 {
            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({"kind":"close_window","id":target.id})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        let reclose = next_test_agent_websocket_event(
            &mut socket,
            "pane_close_result",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert_eq!(
            reclose["ok"], false,
            "the accepted close waits for its exact finalizer; re-close still answers"
        );
        socket
            .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
            .await
            .unwrap();
        let listed = next_test_agent_websocket_event(
            &mut socket,
            "workspace_state",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert!(listed["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|window| window["id"] == pane.id));
        socket
            .send(WebSocketMessage::Text(
                r#"{"kind":"frontend_ready"}"#.into(),
            ))
            .await
            .unwrap();
        let snapshot = next_test_agent_websocket_event(
            &mut socket,
            "terminal_snapshot",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert_eq!(snapshot["id"], pane.id);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(snapshot["data_base64"].as_str().unwrap())
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes)
            .contains("caller snapshot remains readable after peer close"));
        socket.send(WebSocketMessage::Text(serde_json::json!({"kind":"pane_send_input","session_id":pane.session.as_ref().unwrap().id,"text":"status after peer close"}).to_string().into())).await.unwrap();
        let sent = next_test_agent_websocket_event(
            &mut socket,
            "pane_send_result",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert_eq!(sent["ok"], true);
        assert_eq!(sent["window_id"], pane.id);
        // All wire requests returned before the held parser was released.
        release_tx.send(()).unwrap();
        let closed = next_test_agent_websocket_event(
            &mut socket,
            "pane_close_result",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert_eq!(closed["ok"], true);
        assert_eq!(closed["window_id"], target.id);
        assert!(target.pty.try_wait().unwrap().is_some());
        socket
            .send(WebSocketMessage::Text(r#"{"kind":"list_windows"}"#.into()))
            .await
            .unwrap();
        let final_list = next_test_agent_websocket_event(
            &mut socket,
            "workspace_state",
            gwt_core::deadline_budget::HANG_GUARD,
        )
        .await;
        assert!(final_list["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|window| window["id"] != target.id));
        socket.close(None).await.unwrap();
    });
    holder.join().unwrap();
    assert!(pane.pane.lock().unwrap().has_unsent_user_input());
    control_close(&owner, &pane);
}

/// Issue #3667 AC-1/AC-3 at the runtime dispatch layer: a settled grant (an
/// in-memory Active binding whose durable record is stale) still observes its
/// scoped state while producing mutation stays refused.
#[test]
fn settled_grant_observes_scoped_state_but_mutation_is_refused() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 2359);
    let mutation = ControlSessionMutation::new(&pane);
    let session = pane.session.as_ref().unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    gwt_agent::rotate_session_execution_capability(&sessions, &session.id).unwrap();
    let observed = control_request(&owner, &pane, AgentFrontendRequest::Ready);
    assert_eq!(
        observed.close, None,
        "settled observation must not require durable producing authority"
    );
    assert!(observed
        .events
        .iter()
        .any(|event| event["kind"] == "workspace_state"));
    let refused = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::SendInput {
            text: "must-not-dispatch".into(),
        },
    );
    assert_eq!(
        refused.close,
        Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE)
    );
    assert!(!pane.pane.lock().unwrap().has_unsent_user_input());
    mutation.restore();
    control_close(&owner, &pane);
}

#[test]
fn agent_pane_list_windows_returns_scoped_state_without_terminal_snapshots() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let foreign_project = temp.path().join("foreign-project");
    let authenticated_project = temp.path().join("authenticated-project");
    let (foreign_owner, foreign) = control_owner_agent(&foreign_project, "foreign", 43);
    control_seed_foreign_roster(&authenticated_project);
    let (owner, caller) = control_owner_agent(&authenticated_project, "authenticated", 42);
    caller
        .pane
        .lock()
        .unwrap()
        .process_bytes(b"pane list must not serialize this snapshot\r\n");
    let pane = caller.pane.clone();
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder = thread::spawn(move || {
        let _pane = pane.lock().unwrap();
        held_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).is_ok()
    });
    held_rx.recv().unwrap();
    let reply = control_request(&owner, &caller, AgentFrontendRequest::ListWindows);
    release_tx
        .send(())
        .expect("list response must not wait for the snapshot mutex");
    assert!(
        holder.join().unwrap(),
        "list response waited for the terminal snapshot mutex"
    );
    assert_eq!(reply.close, None);
    assert_eq!(
        reply.events.len(),
        1,
        "pane list must return exactly one scoped workspace_state reply"
    );
    assert_eq!(reply.events[0]["kind"], "workspace_state");
    let workspace = &reply.events[0]["workspace"];
    assert_eq!(workspace["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(
        Path::new(workspace["tabs"][0]["project_root"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        authenticated_project.canonicalize().unwrap()
    );
    assert_eq!(
        workspace["tabs"][0]["workspace"]["windows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        workspace["tabs"][0]["workspace"]["windows"][0]["id"],
        caller.id
    );
    assert_eq!(
        workspace["tabs"][0]["workspace"]["windows"][0]["lane_kind"], "unknown",
        "pane list must use the persisted no-I/O projection instead of resolving Git worktree form"
    );
    let foreign_reply =
        control_request(&foreign_owner, &foreign, AgentFrontendRequest::ListWindows);
    let foreign_workspace = &foreign_reply.events[0]["workspace"];
    let tab_view = |value: &serde_json::Value| gwt::ProjectTabView {
        id: value["id"].as_str().unwrap().into(),
        project_key: value["project_key"].as_str().unwrap().into(),
        title: value["title"].as_str().unwrap().into(),
        project_root: value["project_root"].as_str().unwrap().into(),
        project_scope: value["project_scope"].as_str().unwrap().into(),
        kind: serde_json::from_value(value["kind"].clone()).unwrap(),
        workspace: gwt::WorkspaceView {
            windows: value["workspace"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|wire| {
                    let mut window: gwt::persistence::PersistedWindowState =
                        serde_json::from_value(wire.clone()).unwrap();
                    // These fields intentionally never deserialize from persisted layouts.
                    // Reconstruct the actual wire view to compare the shared scope filter.
                    window.agent_color = wire
                        .get("agent_color")
                        .map(|value| serde_json::from_value(value.clone()).unwrap());
                    window.runtime_started_at_ms = wire["runtime_started_at_ms"].as_u64();
                    window.is_pm = wire["is_pm"].as_bool().unwrap();
                    window
                })
                .collect(),
            work_items: Vec::new(),
        },
        running_agent_count: value["running_agent_count"].as_u64().unwrap() as u32,
        running_agents: serde_json::from_value(value["running_agents"].clone()).unwrap(),
    };
    let shared = gwt::project_runtime::agent_dispatch::workspace_state_reply(
        "pane-client",
        caller.grant.as_ref().unwrap().principal(),
        workspace["app_version"].as_str().unwrap().into(),
        vec![
            (
                foreign_project.clone(),
                tab_view(&foreign_workspace["tabs"][0]),
            ),
            (
                authenticated_project.clone(),
                tab_view(&workspace["tabs"][0]),
            ),
        ],
    );
    let BackendEvent::WorkspaceState {
        workspace: shared_workspace,
    } = &shared.event
    else {
        panic!("shared pane projection must return workspace_state");
    };
    assert_eq!(
        serde_json::to_value(shared_workspace).unwrap(),
        workspace.clone()
    );
    let ready = control_request(&owner, &caller, AgentFrontendRequest::Ready);
    let ready_workspace = ready
        .events
        .iter()
        .find(|event| event["kind"] == "workspace_state")
        .unwrap();
    assert_eq!(
        ready_workspace["workspace"]["tabs"][0]["workspace"]["windows"][0]["lane_kind"],
        "execution",
        "existing frontend_ready payload must retain resolved worktree-form semantics"
    );
    assert!(owner_fixture_block_on(foreign_owner.probe(&foreign.id, foreign.incarnation)).is_ok());
    control_close(&owner, &caller);
    control_close(&foreign_owner, &foreign);
}

#[test]
fn queued_bound_agent_pane_request_rechecks_durable_authority_before_runtime_dispatch() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 2359);
    let mutation = ControlSessionMutation::new(&pane);
    let session = pane.session.as_ref().unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    gwt_agent::rotate_session_execution_capability(&sessions, &session.id).unwrap();
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::SendInput {
            text: "must-not-reach-pty\r".into(),
        },
    );
    assert_eq!(
        reply.close,
        Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE),
        "a process-local current token with stale durable authority must fail closed"
    );
    assert!(!pane.pane.lock().unwrap().has_unsent_user_input());
    // Undo this test's deliberate epoch mutation before reaping its exact
    // still-live owner child. No new process or grant is fabricated.
    mutation.restore();
    control_close(&owner, &pane);
}

#[test]
fn bound_agent_pane_rotation_after_precheck_rejects_before_pty_write() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 2359);
    let mutation = ControlSessionMutation::new(&pane);
    let session = pane.session.as_ref().unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let hook_sessions = sessions.clone();
    let hook_session = session.id.clone();
    let (rotation_tx, rotation_rx) = mpsc::sync_channel(1);
    let _hook = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        pane.grant.as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::AfterDurableCheck,
        move || {
            rotation_tx
                .send(gwt_agent::rotate_session_execution_capability(
                    &hook_sessions,
                    &hook_session,
                ))
                .unwrap();
        },
    );
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::SendInput {
            text: "must-not-reach-pty\r".into(),
        },
    );
    rotation_rx.recv().unwrap().unwrap();
    assert_eq!(reply.close, Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE), "epoch rotation between preliminary check and leased dispatch must reject before PTY mutation");
    assert!(!pane.pane.lock().unwrap().has_unsent_user_input());
    mutation.restore();
    control_close(&owner, &pane);
}

#[test]
fn bound_agent_pane_write_holds_epoch_lease_until_actual_pty_mutation_finishes() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 2359);
    let mutation = ControlSessionMutation::new(&pane);
    let session = pane.session.as_ref().unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let initial_epoch = session
        .execution_binding
        .as_ref()
        .unwrap()
        .capability_generation;
    let hook_session = session.id.clone();
    let hook_sessions = sessions.clone();
    let (lease_checked_tx, lease_checked_rx) = mpsc::sync_channel(1);
    let (rotation_done_tx, rotation_done_rx) = mpsc::sync_channel(1);
    let rotation_done_rx = Arc::new(Mutex::new(rotation_done_rx));
    let _hook = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        pane.grant.as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::LeasedMutation,
        move || {
            thread::spawn(move || {
                let lease = gwt_agent::with_session_path_lease_wait(
                    &hook_sessions,
                    &hook_session,
                    Duration::ZERO,
                    |_| Ok(()),
                );
                lease_checked_tx
                    .send(lease.map_err(|error| error.kind()))
                    .unwrap();
                rotation_done_tx
                    .send(gwt_agent::rotate_session_execution_capability(
                        &hook_sessions,
                        &hook_session,
                    ))
                    .unwrap();
            });
            let lease = gwt_core::test_support::recv_event(
                &lease_checked_rx,
                "a separate thread probed the actual held Session lease",
            );
            assert_eq!(lease, Err(std::io::ErrorKind::WouldBlock),
                "the exact Session lease must remain exclusive while the physical PTY mutation is accepted");
        },
    );
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::SendInput {
            text: "linearized-input\r".into(),
        },
    );
    assert_eq!(reply.close, None);
    assert!(reply
        .events
        .iter()
        .any(|event| event["kind"] == "pane_send_result"
            && event["ok"] == true
            && event["window_id"] == pane.id));
    let rotated = rotation_done_rx
        .lock()
        .unwrap()
        .recv_timeout(gwt_core::deadline_budget::HANG_GUARD) // test-hygiene: allow-wall-clock-deadline wait only for the actual cross-thread epoch rotation after the leased PTY command has completed
        .unwrap()
        .unwrap();
    assert_eq!(rotated.capability_generation, initial_epoch + 1);
    mutation.restore();
    control_close(&owner, &pane);
}

#[test]
fn bound_agent_pane_dispatch_does_not_wait_for_a_contended_session_lease_on_tao_thread() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 2359);
    let session = pane.session.as_ref().unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let holder_sessions = sessions.clone();
    let holder_session = session.id.clone();
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder = thread::spawn(move || {
        gwt_agent::with_session_path_lease_wait(
            &holder_sessions,
            &holder_session,
            gwt_core::deadline_budget::HANG_GUARD,
            |_| {
                held_tx.send(()).unwrap();
                gwt_core::test_support::recv_event(
                    &release_rx,
                    "real AgentFrontend dispatch refused contended authority",
                );
                Ok(())
            },
        )
        .unwrap();
    });
    held_rx.recv().unwrap();
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::SendInput {
            text: "must-not-block-owner\r".into(),
        },
    );
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(
        reply.close,
        Some(gwt::project_transport::AGENT_AUTHORITY_UNAVAILABLE_CLOSE),
        "the actual request must finish before the contended lease is released"
    );
    assert!(!pane.pane.lock().unwrap().has_unsent_user_input());
    control_close(&owner, &pane);
}

#[test]
fn queued_correlated_self_close_rechecks_generation_before_acceptance() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let stale_grant = pane.grant.as_ref().unwrap().clone();
    pane.issuer
        .issue(&project, &pane.session.as_ref().unwrap().id)
        .unwrap();
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();
    let client = owner_fixture_block_on(owner.agent_client("stale-self-close")).unwrap();
    let reply = owner_fixture_block_on(client.request(
        stale_grant,
        AgentFrontendRequest::CloseWindow {
            id: pane.id.clone(),
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            responder: Some(responder),
        },
    ))
    .unwrap();
    assert_eq!(
        reply.close,
        Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE)
    );
    assert!(acceptance.try_recv().is_err());
    assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_ok());
    control_close(&owner, &pane);
}

#[test]
fn accepted_agent_self_close_waits_for_direct_ack_then_commits_exactly_once() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let (accepted, reply) = control_accept_self_close(&owner, &pane);
    assert_eq!(reply.close, None);
    assert!(
        reply.events.is_empty(),
        "self-close uses its direct response channel"
    );
    let session = pane.session.as_ref().unwrap();
    let grant = pane.grant.as_ref().unwrap();
    assert!(!pane.issuer.grant_is_current(grant));
    assert!(
        pane.issuer.issue(&project, &session.id).is_err(),
        "Closing must reserve the principal until exact finalization"
    );
    assert!(
        owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_ok(),
        "the child must remain owned before the direct ACK attempt"
    );
    let ticket = accepted.disarm();
    let replay = ticket.clone();
    owner.commit_self_close(ticket).unwrap();
    control_settled_self_closes(&owner);
    assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_err());
    assert!(
        pane.pty.try_wait().unwrap().is_some(),
        "exact self-close must reap its child"
    );
    owner.commit_self_close(replay).unwrap();
    control_settled_self_closes(&owner);
    assert!(
        pane.issuer.issue(&project, &session.id).is_ok(),
        "finalization must release the Closing principal"
    );
}

/// Issue #3783: the correlated self-close ACK moves its bearer into a
/// self-close ticket before Tao commits the window removal. The background
/// close must transfer that ticket into the exact-holder handoff instead of
/// retrying the now-absent bearer token and skipping durable Session cleanup.
#[test]
fn accepted_bound_agent_self_close_persists_terminal_session_in_finalizer() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let session = pane.session.as_ref().unwrap();
    assert_eq!(
        pane.issuer.durable_authority(pane.grant.as_ref().unwrap()),
        crate::embedded_server::AgentDurableAuthority::Current
    );
    let before = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
    )
    .unwrap();
    assert_eq!(before.execution_binding, session.execution_binding);
    let (accepted, reply) = control_accept_self_close(&owner, &pane);
    assert_eq!(reply.close, None);
    drop(accepted);
    control_settled_self_closes(&owner);
    let durable = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
    )
    .unwrap();
    assert_eq!(durable.status, gwt_agent::AgentStatus::Stopped);
    assert!(!durable.restore_window_on_startup);
    assert!(pane.pty.try_wait().unwrap().is_some());
    let proof = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
        &gwt_core::paths::gwt_sessions_dir(),
        &session.id,
    ))
    .unwrap();
    assert_eq!(
        proof
            .execution_identity
            .as_ref()
            .map(|identity| &identity.execution_binding),
        session.execution_binding.as_ref()
    );
    assert_eq!(proof.runtime_incarnation, Some(pane.incarnation));
}

#[test]
fn agent_self_close_disconnect_before_acceptance_rolls_back_the_grant() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let (responder, acceptance) = AgentSelfCloseResponder::channel();
    drop(acceptance);
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::CloseWindow {
            id: pane.id.clone(),
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            responder: Some(responder),
        },
    );
    assert_eq!(reply.close, None);
    assert!(reply.events.is_empty());
    assert!(pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_ok());
    control_settled_self_closes(&owner);
    control_close(&owner, &pane);
}

#[test]
fn correlated_agent_self_close_rejects_same_project_peer_window() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let peer = control_owner_launch(&owner, "peer", control_launch_config(&project, 43));
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();
    let reply = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::CloseWindow {
            id: peer.id.clone(),
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            responder: Some(responder),
        },
    );
    assert_eq!(reply.close, None);
    assert!(
        acceptance.try_recv().unwrap().is_err(),
        "a correlated peer target must be explicitly refused"
    );
    assert!(pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    assert!(owner_fixture_block_on(owner.probe(&peer.id, peer.incarnation)).is_ok());
    control_close(&owner, &peer);
    control_close(&owner, &pane);
}

#[test]
fn correlated_agent_self_close_fails_closed_for_ambiguous_session_windows() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let own = control_projection(&owner, &pane, &project);
    let mut duplicate = own.clone();
    duplicate.window.id = owner.pane_id("duplicate-view");
    // Simulate the corrupt read model at the canonical acceptance boundary;
    // the duplicate is never admitted as another physical Session or runtime.
    let result = gwt::project_runtime::agent_dispatch::begin_self_close(
        &[own, duplicate],
        &gwt_core::paths::gwt_sessions_dir(),
        &pane.issuer,
        pane.grant.as_ref().unwrap(),
        &pane.id,
        &uuid::Uuid::new_v4().to_string(),
        Some(pane.incarnation),
    );
    assert!(
        result.is_err(),
        "ambiguous Session windows must refuse before capability consumption"
    );
    assert!(pane.issuer.grant_is_current(pane.grant.as_ref().unwrap()));
    assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_ok());
    control_close(&owner, &pane);
}

#[test]
fn accepted_agent_self_close_survives_external_stop_before_commit() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let (accepted, _) = control_accept_self_close(&owner, &pane);
    // A second owner close may complete after acceptance but before origin ACK.
    control_close(&owner, &pane);
    drop(accepted);
    control_settled_self_closes(&owner);
    assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_err());
    assert!(pane.pty.try_wait().unwrap().is_some());
}

#[test]
fn accepted_agent_self_close_does_not_remove_same_id_successor_session() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let (accepted, _) = control_accept_self_close(&owner, &pane);
    control_close(&owner, &pane);
    let successor = control_owner_launch(&owner, "agent", control_launch_config(&project, 43));
    assert_eq!(successor.id, pane.id);
    assert_ne!(
        successor.session.as_ref().unwrap().id,
        pane.session.as_ref().unwrap().id
    );
    assert_ne!(successor.incarnation, pane.incarnation);
    drop(accepted);
    control_settled_self_closes(&owner);
    let current =
        owner_fixture_block_on(owner.probe(&successor.id, successor.incarnation)).unwrap();
    assert_eq!(
        current.session.as_ref().unwrap().id,
        successor.session.as_ref().unwrap().id
    );
    assert!(
        successor.pty.try_wait().unwrap().is_none(),
        "the exact same-id successor must survive stale acceptance"
    );
    control_close(&owner, &successor);
}

#[test]
fn accepted_agent_self_close_does_not_remove_same_address_same_session_successor() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, pane) = control_owner_agent(&project, "agent", 42);
    let (accepted, _) = control_accept_self_close(&owner, &pane);
    let ticket = accepted.disarm();
    let replay = ticket.clone();
    control_close(&owner, &pane);
    // The Closing principal is reserved until its actual ticket is settled.
    // Reusing a GUI address can no longer fabricate a physical generation.
    assert!(pane
        .issuer
        .issue(&project, &pane.session.as_ref().unwrap().id)
        .is_err());
    owner.commit_self_close(ticket).unwrap();
    control_settled_self_closes(&owner);
    let mut resume = control_launch_config(&project, 42);
    resume.session_mode = gwt_agent::SessionMode::Continue;
    resume.predecessor_session_id = Some(pane.session.as_ref().unwrap().id.clone());
    let successor = control_owner_launch(&owner, "agent", resume);
    assert_eq!(successor.id, pane.id);
    assert_eq!(
        successor.session.as_ref().unwrap().id,
        pane.session.as_ref().unwrap().id,
        "the actual continuation must reuse the Session rather than forge a read model successor"
    );
    assert_ne!(successor.incarnation, pane.incarnation);
    owner.commit_self_close(replay).unwrap();
    control_settled_self_closes(&owner);
    assert!(owner_fixture_block_on(owner.probe(&successor.id, successor.incarnation)).is_ok());
    assert!(successor.pty.try_wait().unwrap().is_none());
    control_close(&owner, &successor);
}

#[test]
fn agent_name_assigns_only_self_and_reserves_names_across_all_tabs() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let own = temp.path().join("own");
    let foreign = temp.path().join("foreign");
    let (foreign_owner, foreign_pane) = control_owner_agent(&foreign, "agent", 43);
    control_seed_foreign_roster(&own);
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&own),
    )
    .unwrap();
    let mut naming_events = owner.subscribe();
    let caller = control_owner_launch(&owner, "agent", control_launch_config(&own, 42));
    assert!(
        std::iter::from_fn(|| naming_events.try_recv().ok())
            .any(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::Invalidated)),
        "initial owner naming must invalidate its public projection"
    );
    let foreign_reply = control_request(
        &foreign_owner,
        &foreign_pane,
        AgentFrontendRequest::Name {
            candidate: "Orven".into(),
            request_id: "foreign-name".into(),
        },
    );
    let first_assigned = foreign_reply.events[0]["agent_name"]
        .as_str()
        .unwrap()
        .to_owned();
    let foreign_before = fs::read(gwt::persistence::workspace_state_path(&foreign)).unwrap();
    let named = control_request(
        &owner,
        &caller,
        AgentFrontendRequest::Name {
            candidate: "Orven".into(),
            request_id: "name-request".into(),
        },
    );
    assert_eq!(named.close, None);
    let result = named
        .events
        .iter()
        .find(|event| event["kind"] == "pane_name_result")
        .unwrap();
    assert_eq!(result["request_id"], "name-request");
    assert_eq!(result["ok"], true);
    assert!(result["error"].is_null());
    let assigned = result["agent_name"].as_str().unwrap().to_owned();
    assert_ne!(
        assigned.chars().next().unwrap().to_ascii_lowercase(),
        first_assigned.chars().next().unwrap().to_ascii_lowercase()
    );
    assert!(!first_assigned.starts_with("Orven"));
    assert!(!assigned.starts_with("Orven"));
    assert_eq!(
        fs::read(gwt::persistence::workspace_state_path(&foreign)).unwrap(),
        foreign_before,
        "a scoped name request must not mutate another owner's row"
    );
    let saved = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", caller.session.as_ref().unwrap().id)),
    )
    .unwrap();
    assert_eq!(saved.agent_name.as_deref(), Some(assigned.as_str()));
    assert_eq!(
        named
            .events
            .iter()
            .filter(|event| event["kind"] == "workspace_state")
            .count(),
        0,
        "a name already assigned at owner launch needs no second projection mutation"
    );
    let retry = control_request(
        &owner,
        &caller,
        AgentFrontendRequest::Name {
            candidate: "Mint".into(),
            request_id: "retry".into(),
        },
    );
    assert_eq!(retry.events.len(), 1);
    assert_eq!(retry.events[0]["ok"], true);
    assert_eq!(retry.events[0]["agent_name"], assigned);
    control_close(&owner, &caller);
    control_close(&foreign_owner, &foreign_pane);
}

#[test]
fn agent_name_refuses_pm_and_missing_without_mutation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, pm, ordinary) =
        super::managed_launch_tests::managed_owner_fixture(temp.path(), false);
    let before = fs::read(gwt::persistence::workspace_state_path(&root)).unwrap();
    let target = pm.issuer.issue(&root, "missing").unwrap();
    let missing = pm.issuer.grant_for_test(&target.token).unwrap();
    let missing_error = gwt::project_runtime::agent_dispatch::prepare_pane_name(
        &[
            control_projection(&owner, &pm, &root),
            control_projection(&owner, &ordinary, &root),
        ],
        missing.principal(),
        &gwt_core::paths::gwt_sessions_dir(),
    )
    .err()
    .expect("a missing Session has no pane naming target");
    assert!(missing_error.contains("not bound"));
    let client = owner_fixture_block_on(owner.agent_client("missing-name")).unwrap();
    let refused = owner_fixture_block_on(client.request(
        missing,
        AgentFrontendRequest::Name {
            candidate: "Falcon".into(),
            request_id: "refused".into(),
        },
    ))
    .unwrap();
    assert_eq!(
        refused.close,
        Some(gwt::project_transport::AGENT_STALE_BINDING_CLOSE),
        "a grant without an actual durable Session must fail before naming dispatch"
    );
    assert!(refused.events.is_empty());
    assert_eq!(
        fs::read(gwt::persistence::workspace_state_path(&root)).unwrap(),
        before
    );
    let refused = control_request(
        &owner,
        &pm,
        AgentFrontendRequest::Name {
            candidate: "Falcon".into(),
            request_id: "pm".into(),
        },
    );
    assert_eq!(refused.close, None);
    assert_eq!(refused.events[0]["ok"], false);
    assert!(refused.events[0]["error"].as_str().unwrap().contains("PM"));
    assert_eq!(
        fs::read(gwt::persistence::workspace_state_path(&root)).unwrap(),
        before,
        "refused PM or missing-Session naming must leave canonical identities untouched"
    );
    control_close(&owner, &ordinary);
    control_close(&owner, &pm);
}

#[test]
fn agent_name_projects_into_active_work_without_replacing_provider_identity() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut view = active_work_projection_from_saved(
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(temp.path()),
    );
    view.agents.push(workspace_test_agent_with_conversation(
        "named",
        "2026-10-15T00:00:00Z",
        "conversation",
    ));
    view.unassigned_agents
        .push(workspace_test_agent_with_conversation(
            "other",
            "2026-10-15T00:00:00Z",
            "conversation",
        ));
    super::super::workspace_views::attach_agent_names(
        &mut view,
        &HashMap::from([("named".into(), "Falcon".into())]),
    );
    assert_eq!(view.agents[0].agent_name.as_deref(), Some("Falcon"));
    assert_eq!(view.agents[0].agent_id, "codex");
    assert_eq!(view.agents[0].display_name, "Codex");
    assert!(view.unassigned_agents[0].agent_name.is_none());
}

fn legacy_name_session_row(
    root: &Path,
    raw: &str,
    preset: WindowPreset,
    name: Option<&str>,
) -> (gwt::PersistedWindowState, gwt_agent::Session) {
    let mut session = gwt_agent::Session::new(root, "work/legacy", gwt_agent::AgentId::Codex);
    session.project_state_root = Some(root.to_path_buf());
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.restore_window_on_startup = true;
    session.agent_name = name.map(str::to_owned);
    session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
    let mut row = sample_window(raw, preset, WindowProcessStatus::Stopped);
    row.session_id = Some(session.id.clone());
    row.agent_name = name.map(str::to_owned);
    (row, session)
}

fn save_legacy_name_rows(root: &Path, windows: Vec<gwt::PersistedWindowState>) {
    gwt::persistence::save_workspace_state_durable(
        &gwt::persistence::workspace_state_path(root),
        &gwt::PersistedWindowRegistry {
            windows,
            next_z_index: 3,
        },
    )
    .unwrap();
}

#[test]
fn agent_name_backfills_registered_pm_without_joining_the_member_pool() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    control_init_repo(&root);
    seed_owner_fixture_roster(&root, 8);
    let worktree = create_detached_pm_worktree_fixture(&root);
    let (mut row, mut session) = legacy_name_session_row(&root, "pm", WindowPreset::Agent, None);
    session.worktree_path = worktree.clone();
    session.agent_name = Some("Falcon".into());
    session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
    row.agent_name = None;
    let mut implementation = sample_window(
        "implementation",
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    implementation.agent_name = Some("Flora".into());
    save_legacy_name_rows(&root, vec![row, implementation]);
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&root),
        pm_registration_fixture(&session.id, &worktree),
        |_| false,
    )
    .unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let roster = gwt::agent_names::read_project_roster(&root, &sessions).unwrap();
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    let saved =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap();
    assert_eq!(
        saved
            .windows
            .iter()
            .find(|row| row.id == "pm")
            .unwrap()
            .agent_name
            .as_deref(),
        Some("Falcon")
    );
    let mut events = owner.subscribe();
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::UpdateLayout { layout: saved }),
    )
    .unwrap();
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::Invalidated)),
        "the assigned PM name stays stable"
    );
    let saved = gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id))).unwrap();
    assert_eq!(saved.agent_name, session.agent_name);
    assert!(saved.agent_member_id.is_none());
    assert_eq!(
        gwt::agent_names::read_project_roster(&root, &sessions).unwrap(),
        roster,
        "PM display naming must not change members, assignments or capacity"
    );
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn agent_name_invalidates_projection_payload_and_watcher_revision() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    control_init_repo(&root);
    seed_owner_fixture_roster(&root, 8);
    let (row, session) = legacy_name_session_row(&root, "agent", WindowPreset::Agent, None);
    save_legacy_name_rows(&root, vec![row]);
    let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
    let unavailable = session_path.with_extension("unavailable");
    fs::rename(&session_path, &unavailable).unwrap();
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    let identity = owner.client().identity().cloned().unwrap();
    let mut owner_events = owner.subscribe();
    fs::rename(&unavailable, &session_path).unwrap();
    let mut tab = sample_project_tab_with_window_at(
        "tab",
        "agent",
        root.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    tab.workspace.set_session_id("agent", Some(session.id));
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.active_work_projection_broadcast_for_tab("tab");
    for _ in 0..2 {
        let task = tasks.lock().unwrap().remove(0);
        task();
    }
    let old_completion = recorded
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match recorded_project_payload(event) {
            UserEvent::ActiveWorkProjectionPrepared(prepared) => Some(prepared.as_ref().clone()),
            _ => None,
        })
        .expect("old unnamed projection completion");
    let state = runtime.project_state_for_tab("tab").unwrap();
    let revision = state.workspace_projection_revision.get();
    state
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab".into(), Arc::from("stale unnamed payload"));
    let layout =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap();
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::UpdateLayout { layout }),
    )
    .unwrap();
    let changed = std::iter::from_fn(|| owner_events.try_recv().ok())
        .find(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::Invalidated))
        .expect("actual owner published the naming change");
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = Some(identity.clone());
    runtime.handle_project_owner_event(&context, 1, &identity, changed);
    let state = runtime.project_state_for_tab("tab").unwrap();
    assert!(!state
        .active_work_projection_payload_cache
        .borrow()
        .contains_key("tab"));
    assert!(state.workspace_projection_revision.get() > revision);
    assert!(
        runtime
            .handle_active_work_projection_prepared(old_completion)
            .prepared_dispatch
            .is_none(),
        "naming supersedes an in-flight unnamed snapshot"
    );
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn runtime_names_legacy_agent_windows_without_a_pane_name_request() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    control_init_repo(&root);
    seed_owner_fixture_roster(&root, 8);
    let (unnamed, session) = legacy_name_session_row(&root, "claude", WindowPreset::Claude, None);
    let mut second = sample_window("named", WindowPreset::Agent, WindowProcessStatus::Stopped);
    second.agent_name = Some("Falcon".into());
    save_legacy_name_rows(&root, vec![unnamed, second]);
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    let saved =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap();
    let assigned = saved
        .windows
        .iter()
        .find(|row| row.id == "claude")
        .unwrap()
        .agent_name
        .as_ref()
        .expect("actual owner startup assignment");
    assert_ne!(assigned.chars().next().unwrap().to_ascii_lowercase(), 'f');
    assert_eq!(
        saved
            .windows
            .iter()
            .find(|row| row.id == "named")
            .unwrap()
            .agent_name
            .as_deref(),
        Some("Falcon")
    );
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
    )
    .unwrap();
    assert_eq!(session.agent_name.as_ref(), Some(assigned));
    let mut events = owner.subscribe();
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::UpdateLayout { layout: saved }),
    )
    .unwrap();
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::Invalidated)),
        "the same owner layout must not publish another naming change"
    );
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn agent_name_keeps_bound_session_independent_of_roster_readback() {
    use gwt::project_runtime::events::OwnerCommand;

    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    control_init_repo(&root);
    seed_owner_fixture_roster(&root, 1);
    gwt::mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&root),
        |prefs| prefs.agent_roster.as_mut().unwrap().members[0].name = "Falcon".into(),
    )
    .unwrap();
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    let pane = control_owner_launch(&owner, "agent", control_launch_config(&root, 42));
    let pane = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).unwrap();
    let session = pane.session.as_ref().unwrap();
    let binding = session.execution_binding.as_ref().unwrap();
    let execution_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    assert!(pane.pending_fresh_execution.is_none());
    assert_eq!(binding.owner_kind, "issue");
    assert_eq!(binding.owner_number, 42);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &session.worktree_path,
            execution_owner
        )
        .unwrap(),
        Some(binding.identity.clone())
    );
    assert_eq!(
        gwt::cli::execution_state::diagnose_owner(&session.worktree_path, execution_owner)
            .ecr_status,
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active)
    );
    assert!(pane.has_capability_bearer(pane.grant.as_ref().unwrap()));
    let positive = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::Name {
            candidate: "Falcon".into(),
            request_id: "bound-name-control".into(),
        },
    );
    assert_eq!(positive.close, None);
    assert!(positive
        .events
        .iter()
        .any(|event| event["request_id"] == "bound-name-control"
            && event["ok"] == true
            && event["agent_name"] == "Falcon"));
    assert_eq!(session.agent_name.as_deref(), Some("Falcon"));
    assert!(session.agent_member_id.is_some());
    assert_ne!(pane.pty.process_id().unwrap(), std::process::id());
    assert!(pane.pty.try_wait().unwrap().is_none());
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let session_path = sessions.join(format!("{}.toml", session.id));
    let before = fs::read(&session_path).unwrap();
    let prefs = gwt::issue_monitor_prefs_path_for_repo_path(&root);
    let original_prefs = fs::read(&prefs).unwrap();
    fs::write(&prefs, "unreadable roster").unwrap();
    let result = control_request(
        &owner,
        &pane,
        AgentFrontendRequest::Name {
            candidate: "Falcon".into(),
            request_id: "bound-name".into(),
        },
    );
    let layout =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap();
    owner_fixture_block_on(owner.command(OwnerCommand::UpdateLayout { layout })).unwrap();
    let after = fs::read(&session_path).unwrap();
    let saved = gwt_agent::Session::load(&session_path).unwrap();
    let unreadable_prefs = fs::read_to_string(&prefs).unwrap();
    // Restore only the injected fault before normal physical cleanup, including
    // the RED run where naming currently rejects the already-bound Session.
    fs::write(&prefs, original_prefs).unwrap();
    control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());
    assert_eq!(result.close, None);
    let named = result
        .events
        .iter()
        .find(|event| event["request_id"] == "bound-name")
        .unwrap();
    assert_eq!(
        named["ok"], true,
        "bound Session naming must not read the roster: {named}"
    );
    assert_eq!(named["agent_name"], "Falcon");
    assert_eq!(unreadable_prefs, "unreadable roster");
    assert_eq!(after, before, "naming must not rewrite the bound Session");
    assert_eq!(saved.agent_name, session.agent_name);
    assert_eq!(saved.agent_member_id, session.agent_member_id);
}

#[test]
fn agent_name_reserves_live_initials_and_backfills_named_legacy_sessions() {
    use gwt::project_runtime::{events::OwnerCommand, launch_payload::OwnerLaunchMetadata};

    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    control_init_repo(&root);
    seed_owner_fixture_roster(&root, 1);
    configure_owner_fixture_tools(&root, &["pane-contract-fixture"]);
    gwt::mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&root),
        |prefs| {
            prefs.agent_roster.as_mut().unwrap().members[0].name = "Falcon".into();
        },
    )
    .unwrap();
    let history = gwt_core::paths::gwt_project_dir_for_repo_path(&root).join("agent-names.jsonl");
    // The explicitly seeded fixture member has already received this name.
    gwt::agent_names::remember(&history, &HashSet::from(["Falcon".into()])).unwrap();
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    let mut config = control_launch_config(&root, 42);
    config.linked_issue_number = None;
    config.explicit_follow_up = false;
    let source = control_owner_launch(&owner, "agent", config);
    let source_id = source.session.as_ref().unwrap().id.clone();
    owner_runtime_status_event(&owner, &source, "Waiting", "PermissionRequest");
    let live = owner_fixture_block_on(owner.command(OwnerCommand::Snapshot {
        id: source.id.clone(),
        incarnation: source.incarnation,
    }))
    .unwrap();
    assert_eq!(live["agent_name"], "Falcon");
    assert_eq!(live["session_id"], source_id);
    assert_eq!(live["child_pid"], source.pty.process_id().unwrap());
    assert_eq!(
        live["status"],
        serde_json::json!(WindowProcessStatus::Waiting)
    );
    assert!(source.pty.try_wait().unwrap().is_none());
    let mut row =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap()
            .windows
            .into_iter()
            .find(|row| row.session_id.as_deref() == Some(source_id.as_str()))
            .unwrap();
    let history_before = fs::read(&history).unwrap();

    // Review inherited names still use the original global allocator layer;
    // registered member UUID/tool reservation is not this negative's oracle.
    let mut config = control_launch_config(&root, 43);
    config.linked_issue_number = None;
    config.explicit_follow_up = false;
    config.suppress_execution_control = true;
    config
        .env_vars
        .insert("GWT_REVIEW_DISPATCH".into(), "1".into());
    let target_id = owner.pane_id("conflicting-initial");
    let rejected = owner_fixture_block_on(owner.command(OwnerCommand::LaunchAgent {
        id: target_id.clone(),
        operation_id: "retained-initial-conflict".into(),
        tab_id: target_id.split_once("::").unwrap().0.into(),
        config: Box::new(config),
        metadata: Box::new(OwnerLaunchMetadata {
            inherited_agent_name: Some("Flora".into()),
            ..Default::default()
        }),
        cols: 80,
        rows: 24,
    }));
    // A RED run may expose a real conflicting child. Drain that exact owned
    // candidate and source normally before the required negative assertion.
    let unexpected = rejected
        .as_ref()
        .ok()
        .and_then(|receipt| receipt["incarnation"].as_u64())
        .and_then(|incarnation| owner_fixture_block_on(owner.probe(&target_id, incarnation)).ok());
    let unexpected_identity = unexpected.as_ref().map(|candidate| {
        (
            candidate.id.clone(),
            candidate.incarnation,
            candidate.pty.process_id(),
            candidate
                .session
                .as_ref()
                .and_then(|session| session.agent_name.clone()),
        )
    });
    if let Some(candidate) = unexpected {
        control_close(&owner, &candidate);
        assert!(candidate.pty.try_wait().unwrap().is_some());
    }
    let retained = owner_fixture_block_on(owner.probe(&source.id, source.incarnation)).unwrap();
    assert!(Arc::ptr_eq(&retained.pty, &source.pty));
    assert!(retained.pty.try_wait().unwrap().is_none());
    assert_eq!(retained.status, WindowProcessStatus::Waiting);
    control_close(&owner, &source);
    assert!(source.pty.try_wait().unwrap().is_some());
    assert!(owner_fixture_block_on(owner.probe(&source.id, source.incarnation)).is_err());
    owner_fixture_block_on(owner.shutdown());
    assert!(rejected.is_err(),
        "another inherited Agent name must not duplicate the retained live initial; actual candidate: {unexpected_identity:?}; response: {rejected:?}");
    assert!(
        rejected
            .unwrap_err()
            .to_ascii_lowercase()
            .contains("initial"),
        "refusal must be the global name reservation"
    );
    assert_eq!(
        fs::read(&history).unwrap(),
        history_before,
        "a rejected inherited name must preserve the existing name history"
    );
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let mut stopped =
        gwt_agent::Session::load(&sessions.join(format!("{source_id}.toml"))).unwrap();
    assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(stopped.agent_name.as_deref(), Some("Falcon"));
    assert!(fs::read_to_string(&history)
        .unwrap()
        .lines()
        .any(|line| line == "\"Falcon\""));

    // Replay the same drained Session and actual owner row in the old optional
    // metadata format, retaining their physical source identity.
    stopped.agent_name = None;
    stopped.agent_member_id = None;
    stopped.restore_window_on_startup = true;
    stopped.save(&sessions).unwrap();
    row.status = gwt::persistence::WindowState::Stopped;
    save_legacy_name_rows(&root, vec![row]);
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    let saved = gwt_agent::Session::load(&sessions.join(format!("{source_id}.toml"))).unwrap();
    assert_eq!(saved.agent_name.as_deref(), Some("Falcon"));
    let layout =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap();
    let mut events = owner.subscribe();
    owner_fixture_block_on(owner.command(OwnerCommand::UpdateLayout { layout })).unwrap();
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::Invalidated)),
        "Session backfill needs no UI repaint"
    );
    assert_eq!(
        gwt_agent::Session::load(&sessions.join(format!("{source_id}.toml")))
            .unwrap()
            .agent_name
            .as_deref(),
        Some("Falcon")
    );
    assert_eq!(fs::read(&history).unwrap(), history_before);
    owner_fixture_block_on(owner.shutdown());
}

fn switch_source_retirement_fixture(
    root: &Path,
) -> (
    AppRuntime,
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
    gwt::issue_monitor::IssueMonitorRuntimeStopRequest,
) {
    let project = root.join("project");
    control_init_repo(&project);
    seed_owner_fixture_roster(&project, 8);
    let tab = sample_project_tab_with_window_at(
        "source",
        "agent",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let (mut runtime, _) = sample_runtime_with_events(root, vec![tab], Some("source"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let id = combined_window_id("source", "agent");
    drive_owner_agent_fixture(
        &mut runtime,
        &id,
        control_launch_config(&project, 43),
        Default::default(),
    );
    let owner = runtime.test_project_owners[&project].clone();
    let remote = &runtime.runtimes[&id];
    let mut probe =
        owner_fixture_block_on(owner.probe(&remote.owner_pane_id, remote.incarnation)).unwrap();
    let source = probe.session.as_ref().unwrap();
    let mut ready = runtime_hook_state_for_event("Waiting", "SessionStart", &source.id);
    ready.project_root = Some(project.display().to_string());
    ready.branch = Some(source.branch.clone());
    ready.continuation_readiness_nonce = probe
        .pending_fresh_execution
        .as_ref()
        .map(|pending| pending.readiness_nonce.clone());
    assert!(owner_fixture_block_on(probe.runtime_hook(&ready))
        .unwrap()
        .is_success());
    // test-hygiene: allow-wall-clock-deadline Bound canonical readiness activation; the owner's pending receipt establishes completion.
    probe = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound the actual child/owner completion; output or the exact owner receipt establishes progress.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let current = owner.probe(&probe.id, probe.incarnation).await.unwrap();
                if current.pending_fresh_execution.is_none() {
                    return current;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Switch fixture must activate through authenticated SessionStart")
    });
    let source = probe.session.as_ref().unwrap();
    let proof = gwt::issue_monitor::IssueMonitorRuntimeStopRequest {
        issue_number: 43,
        window_id: probe.id.clone(),
        session_id: source.id.clone(),
        host_pid: std::process::id(),
        host_started_at: gwt::process::host_process_start_time(std::process::id()).unwrap(),
        runtime_incarnation: probe.incarnation,
        child_pid: probe.pty.process_id().unwrap(),
        child_started_at: gwt::process::host_process_start_time(probe.pty.process_id().unwrap())
            .unwrap(),
        requested_at: Utc::now().to_rfc3339(),
        execution_identity: gwt_agent::SessionExecutionIdentity::from_session(source)
            .unwrap()
            .unwrap(),
    };
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    (runtime, owner, probe, proof)
}

fn switch_removed_event(
    events: &mut tokio::sync::broadcast::Receiver<gwt::project_runtime::events::OwnerEvent>,
    id: &str,
) -> gwt::project_runtime::events::OwnerEvent {
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound exact owner finalization; an authenticated removal event establishes completion.
        let mut observed = Vec::new();
        // test-hygiene: allow-wall-clock-deadline Bound actual exact owner finalization; the matching removal event establishes completion.
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.unwrap();
                observed.push(format!("{event:?}"));
                if matches!(&event, gwt::project_runtime::events::OwnerEvent::WindowRemoved { id: removed, .. } if removed == id) {
                    return event;
                }
            }
        }).await;
        result.unwrap_or_else(|error| panic!("the owner must publish exact Switch source removal: {error}; observed={observed:?}"))
    })
}

#[test]
fn switch_source_exit_retires_exact_diagnostic_pane_and_husk_once() {
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for keep_runtime in [true, false] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (mut runtime, owner, source, proof) = switch_source_retirement_fixture(temp.path());
        let id = combined_window_id("source", "agent");
        let mut events = owner.subscribe();
        owner_fixture_block_on(owner.agent_switch_source_exited(proof.clone(), source.pty.clone()))
            .unwrap();
        assert!(
            owner_fixture_block_on(owner.probe(&source.id, source.incarnation)).is_ok(),
            "a live tree is never retirement authority"
        );
        assert!(runtime.window_lookup.contains_key(&id));
        if !keep_runtime {
            let session = source.session.as_ref().unwrap();
            let mut failed = runtime_hook_state_for_event("Error", "PostToolUse", &session.id);
            failed.project_root = Some(
                runtime
                    .tab("source")
                    .unwrap()
                    .project_root
                    .display()
                    .to_string(),
            );
            failed.branch = Some(session.branch.clone());
            assert!(owner_fixture_block_on(source.runtime_hook(&failed))
                .unwrap()
                .is_success());
            owner_fixture_block_on(
                owner.command(gwt::project_runtime::events::OwnerCommand::Health),
            )
            .unwrap();
        }
        assert!(source
            .pty
            .terminate_and_wait_for_tree(Duration::from_secs(5))
            .unwrap());
        if !keep_runtime {
            owner_fixture_block_on(async {
                // test-hygiene: allow-wall-clock-deadline Bound the pre-existing diagnostic finalizer; its exact close receipt proves the runtime is retired.
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if let gwt::project_runtime::events::OwnerEvent::Closed {
                            id: closed,
                            incarnation,
                            error,
                        } = events.recv().await.unwrap()
                        {
                            if closed == source.id && incarnation == source.incarnation {
                                assert!(error.is_none(), "diagnostic close failed: {error:?}");
                                break;
                            }
                        }
                    }
                })
                .await
                .expect("actual diagnostic source retirement");
            });
            assert!(owner_fixture_block_on(owner.probe(&source.id, source.incarnation)).is_err());
            runtime.runtimes.remove(&id);
            runtime.active_agent_sessions.remove(&id);
        }
        owner_fixture_block_on(owner.agent_switch_source_exited(proof.clone(), source.pty.clone()))
            .unwrap();
        let removed = switch_removed_event(&mut events, &source.id);
        let context = runtime.test_context();
        let identity = owner.client().identity().unwrap().clone();
        assert!(!runtime.handle_project_owner_event(&context, 1, &identity, removed.clone()).is_empty(), "the exact physically exited source must leave the canvas even without a writer registration");
        assert!(!runtime.window_lookup.contains_key(&id));
        assert!(runtime
            .handle_project_owner_event(&context, 1, &identity, removed)
            .is_empty());
        owner_fixture_block_on(owner.agent_switch_source_exited(proof, source.pty.clone()))
            .unwrap();
        assert!(
            !std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(
                event,
                gwt::project_runtime::events::OwnerEvent::WindowRemoved { .. }
            )),
            "the exact source is finalized once"
        );
        assert!(
            owner_fixture_block_on(
                owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes)
            )
            .unwrap()["panes"]
                .as_array()
                .unwrap()
                .is_empty(),
            "normal admission must see the released physical slot"
        );
    }
}

#[test]
fn switch_source_exit_preserves_reused_session_and_runtime() {
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for replace_session in [true, false] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (mut runtime, owner, source, proof) = switch_source_retirement_fixture(temp.path());
        let id = combined_window_id("source", "agent");
        let mut events = owner.subscribe();
        assert!(source
            .pty
            .terminate_and_wait_for_tree(Duration::from_secs(5))
            .unwrap());
        let removed = if replace_session {
            runtime
                .tab_mut("source")
                .unwrap()
                .workspace
                .set_session_id("agent", Some("successor-session".into()));
            owner_fixture_block_on(owner.agent_switch_source_exited(proof, source.pty.clone()))
                .unwrap();
            switch_removed_event(&mut events, &source.id)
        } else {
            control_close(&owner, &source);
            let removed = switch_removed_event(&mut events, &source.id);
            let project = runtime.tab("source").unwrap().project_root.clone();
            drive_owner_agent_fixture(
                &mut runtime,
                &id,
                control_launch_config(&project, 44),
                Default::default(),
            );
            owner_fixture_block_on(owner.agent_switch_source_exited(proof, source.pty.clone()))
                .unwrap();
            let current = &runtime.runtimes[&id];
            let current =
                owner_fixture_block_on(owner.probe(&current.owner_pane_id, current.incarnation))
                    .unwrap();
            assert!(!Arc::ptr_eq(&current.pty, &source.pty));
            assert!(
                !current.pty.tree_exit_confirmed(),
                "a stale Switch must not stop the replacement tree"
            );
            removed
        };
        let context = runtime.test_context();
        let identity = owner.client().identity().unwrap().clone();
        assert!(runtime
            .handle_project_owner_event(&context, 1, &identity, removed)
            .is_empty());
        assert!(runtime.window_lookup.contains_key(&id));
        if !replace_session {
            let current = &runtime.runtimes[&id];
            let current =
                owner_fixture_block_on(owner.probe(&current.owner_pane_id, current.incarnation))
                    .unwrap();
            control_close(&owner, &current);
        }
    }
}
