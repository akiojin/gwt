fn refresh_test_context(
    project_root: PathBuf,
    generation: u64,
) -> crate::app_runtime::ProjectContext {
    crate::app_runtime::ProjectContext {
        tab_id: "tab-a".to_string(),
        project_key: gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
        project_root,
        generation,
    }
}

#[test]
fn board_refresh_cache_discards_views_and_late_finish_from_previous_generation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let old = refresh_test_context(root, 1);
    let new = crate::app_runtime::ProjectContext {
        generation: 2,
        ..old.clone()
    };
    let (view, _) = gwt_core::coordination::refresh_scoped_board_view(
        &old.project_root,
        &gwt_core::coordination::BoardAudienceScope::All,
        None,
    )
    .unwrap();
    let mut queue = crate::BoardRefreshQueue::default();
    queue.begin(&old).unwrap();
    queue.finish(
        &old,
        std::collections::HashMap::from([("tab-a::board-1".to_string(), view)]),
    );
    assert!(
        queue.begin(&new).unwrap().is_empty(),
        "a reopened Board with the same window ID must rebuild its view"
    );
    assert!(!queue.finish(&new, Default::default()));
    queue.begin(&new).unwrap();
    let reopened = crate::app_runtime::ProjectContext {
        generation: 3,
        ..new.clone()
    };
    assert!(
        queue.begin(&reopened).is_some(),
        "a new incarnation must not wait for the old job"
    );
    assert!(!queue.finish(&new, Default::default()));
    assert!(
        queue.begin(&reopened).is_none(),
        "an old completion must not clear the new refresh in flight"
    );
    assert!(queue.finish(&reopened, Default::default()));
}

#[test]
fn workspace_projection_reload_callback_rejects_closed_project_generation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&root);
    projection.title = "captured worker snapshot".to_string();
    gwt_core::workspace_projection::save_workspace_projection(&root, &projection).unwrap();
    let (mut app, recorded) = sample_runtime_with_events(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "A",
            root,
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    let (spawner, tasks) = crate::app_runtime::BlockingTaskSpawner::queued();
    let old = app.project_context("tab-a").unwrap();
    super::spawn_workspace_projection_reload(&spawner, app.proxy.clone(), old);
    app.project_tab_incarnations
        .get_mut("tab-a")
        .unwrap()
        .generation += 1;
    app.refresh_project_state("tab-a");
    tasks.lock().unwrap().pop().unwrap()();
    let completion = recorded.lock().unwrap().pop().unwrap();
    assert!(
        app.accept_project_completion(completion).is_none(),
        "the actual reload worker must not update a reopened project"
    );
    super::spawn_workspace_projection_reload(
        &spawner,
        app.proxy.clone(),
        app.project_context("tab-a").unwrap(),
    );
    tasks.lock().unwrap().pop().unwrap()();
    assert!(
        matches!(app.accept_project_completion(recorded.lock().unwrap().pop().unwrap()), Some(UserEvent::WorkspaceProjectionLoaded { projection: Some(projection), .. }) if projection.title == "captured worker snapshot")
    );
}

#[cfg(unix)]
#[test]
fn daemon_subscription_stays_observer_until_startup_canvas_is_ready() {
    use std::sync::mpsc as std_mpsc;

    use gwt_core::daemon::{
        persist_endpoint, ClientFrame, DaemonEndpoint, DaemonFrame, IpcHandshakeRequest,
        IpcHandshakeResponse, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use gwt_core::test_support::ScopedEnvVar;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = gwt_core::test_support::socket_tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    // The subscriber resolves its endpoint on its own OS thread.
    let _process_home = ScopedEnvVar::set("HOME", temp.path());
    let _process_profile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let scope = RuntimeScope::from_project_root(&root, RuntimeTarget::Host).unwrap();
    let socket = temp.path().join("daemon.sock");
    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        std::process::id(),
        socket.to_string_lossy().to_string(),
        "canvas-readiness-test".to_string(),
        "test-daemon".to_string(),
    );
    persist_endpoint(
        &scope.endpoint_path(&gwt_core::paths::gwt_home()),
        &endpoint,
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (subscriptions_tx, subscriptions_rx) = std_mpsc::channel();
    let (disconnected_tx, disconnected_rx) = std_mpsc::channel();
    let listener = runtime.block_on(async {
        tokio::net::UnixListener::bind(gwt_core::test_support::checked_socket_path(&socket))
            .unwrap()
    });
    let server = runtime.spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: IpcHandshakeRequest = serde_json::from_str(line.trim_end()).unwrap();
            assert_eq!(request.scope, endpoint.scope);
            assert_eq!(request.auth_token, endpoint.auth_token);
            let handshake = IpcHandshakeResponse {
                protocol_version: DAEMON_PROTOCOL_VERSION,
                daemon_version: endpoint.daemon_version.clone(),
                accepted: true,
                rejection_reason: None,
            };
            write
                .write_all(format!("{}\n", serde_json::to_string(&handshake).unwrap()).as_bytes())
                .await
                .unwrap();
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let subscription: ClientFrame = serde_json::from_str(line.trim_end()).unwrap();
            for frame in [
                DaemonFrame::Event {
                    channel: "board".to_string(),
                    payload: serde_json::json!({}),
                },
                DaemonFrame::Ack,
            ] {
                write
                    .write_all(format!("{}\n", serde_json::to_string(&frame).unwrap()).as_bytes())
                    .await
                    .unwrap();
            }
            subscriptions_tx.send(subscription).unwrap();
            line.clear();
            assert_eq!(reader.read_line(&mut line).await.unwrap(), 0);
            disconnected_tx.send(()).unwrap();
        }
    });
    let (mut app, recorded) = sample_runtime_with_events(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "A",
            root,
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    app.deferred_issue_monitor_launches = Some(Vec::new());
    app.startup_generation_reaper_pending = true;
    let mut subscribers = super::BoardDaemonSubscriberRegistry::default();
    subscribers.sync(&app, app.proxy.clone());
    assert!(
        matches!(
            subscriptions_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            ClientFrame::Subscribe { channels } if channels == super::daemon_subscriber_channels()
        ),
        "bootstrap must observe events without advertising that it can create agent windows"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !recorded.lock().unwrap().iter().any(|event| {
        matches!(
            recorded_project_payload(event),
            UserEvent::BoardProjectionChanged { .. }
        )
    }) {
        assert!(Instant::now() < deadline, "pre-canvas Board event was lost");
        std::thread::yield_now();
    }

    app.startup_generation_reaper_pending = false;
    subscribers.sync(&app, app.proxy.clone());
    assert_eq!(
        disconnected_rx.recv_timeout(Duration::from_millis(100)),
        Err(std_mpsc::RecvTimeoutError::Timeout),
        "reaper completion alone must keep the observer connection"
    );

    app.deferred_issue_monitor_launches = None;
    subscribers.sync(&app, app.proxy.clone());
    disconnected_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(
        matches!(
            subscriptions_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            ClientFrame::SubscribeMaterializer { channels }
                if channels == super::daemon_subscriber_channels()
        ),
        "the completed canvas drain must enable agent-window materialization"
    );
    subscribers.sync(&app, app.proxy.clone());
    assert_eq!(subscribers.subscribers.len(), 1);
    assert_eq!(
        disconnected_rx.recv_timeout(Duration::from_millis(100)),
        Err(std_mpsc::RecvTimeoutError::Timeout),
        "repeated ready sync must retain the same materializer connection"
    );
    subscribers.shutdown();
    disconnected_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    server.abort();
    runtime.block_on(async {
        let _ = server.await;
    });
}
