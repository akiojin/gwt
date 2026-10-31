//! Host mobile authorization stays fenced through the native physical effect.
use super::{AppRuntime, BlockingTaskSpawner, ClientHub, ClientScope};
use gwt::mobile_connection::{AuthorizationError, MobileInputRequest};
use gwt::project_runtime::{
    client::ProjectRuntimeClient,
    events::{
        OwnerCommand, OwnerMobileInputIdentity, OwnerMobileInputOutcome, OwnerMobileInputPhase,
        OwnerMobileInputReady,
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

type InputResult = Result<OwnerMobileInputOutcome, AuthorizationError>;

impl AppRuntime {
    pub(crate) fn handle_mobile_input(
        &mut self,
        request: Arc<MobileInputRequest>,
        clients: &ClientHub,
        now: impl FnOnce() -> u64 + Send + 'static,
    ) -> Result<(), AuthorizationError> {
        let id = request.window_id().to_string();
        let completion = self.runtimes.get(&id).and_then(|runtime| {
            let address = self.window_lookup.get(&id)?;
            let context = self.project_context(&address.tab_id)?;
            let state = self.project_state(&context)?;
            Some((
                context,
                state.project_owner_connection_id,
                runtime.owner_identity.clone(),
                runtime.incarnation,
            ))
        });
        let task = self.mobile_input_task(request, clients.clone(), now)?;
        let proxy = self.proxy.clone();
        self.blocking_tasks
            .try_spawn(move || match task() {
                Ok(outcome) => {
                    if let Some((context, connection_id, identity, incarnation)) = completion {
                        proxy.send(crate::UserEvent::RemoteRuntimeCommandComplete(Box::new(
                            super::RemoteRuntimeCommandCompletion {
                                context,
                                connection_id,
                                identity,
                                id,
                                incarnation,
                                kind: super::RemoteRuntimeCommandKind::Input { reply_client: None },
                                approval_resolution_started: false,
                                result: Ok(outcome.observation),
                            },
                        )));
                    }
                }
                Err(error) => tracing::warn!(%error, "mobile terminal input rejected"),
            })
            .map_err(|_| AuthorizationError::Unavailable)
    }

    fn mobile_input_task(
        &self,
        request: Arc<MobileInputRequest>,
        clients: ClientHub,
        now: impl FnOnce() -> u64 + Send + 'static,
    ) -> Result<impl FnOnce() -> InputResult + Send + 'static, AuthorizationError> {
        self.mobile_input_task_with_observers(request, clients, now, || {}, |_, _| {})
    }

    fn mobile_input_task_with_observers(
        &self,
        request: Arc<MobileInputRequest>,
        clients: ClientHub,
        now: impl FnOnce() -> u64 + Send + 'static,
        on_wait: impl FnOnce() + Send + 'static,
        after_ready: impl FnOnce(&ProjectRuntimeClient, &OwnerMobileInputReady) + Send + 'static,
    ) -> Result<impl FnOnce() -> InputResult + Send + 'static, AuthorizationError> {
        use sha2::{Digest, Sha256};
        // Keep shared HTTP connection drivers active while this blocking
        // worker waits on authorization or another native control operation.
        let executor = match &self.blocking_tasks {
            BlockingTaskSpawner::Tokio(handle) => handle.clone(),
            #[cfg(test)]
            _ => tokio::runtime::Handle::try_current()
                .map_err(|_| AuthorizationError::Unavailable)?,
        };
        let turn = request.book_turn()?;
        let binding = clients.capture_project_binding(request.client_id());
        let target = self
            .runtimes
            .get(request.window_id())
            .filter(|_| self.project_key_for_window(request.window_id()) == Some(request.project()))
            .and_then(|runtime| {
                let address = self.window_lookup.get(request.window_id())?;
                let context = self.project_context(&address.tab_id)?;
                let state = self.project_state(&context)?;
                if state.project_owner_identity.as_ref() != Some(&runtime.owner_identity) {
                    return None;
                }
                let identity = OwnerMobileInputIdentity {
                    operation_id: uuid::Uuid::new_v4().to_string(),
                    host_instance_id: runtime.owner_identity.host_instance_id.clone(),
                    id: runtime.owner_pane_id.clone(),
                    incarnation: runtime.incarnation,
                    session_id: self
                        .active_agent_sessions
                        .get(request.window_id())
                        .map(|session| session.session_id.clone()),
                    data_sha256: hex::encode(Sha256::digest(request.data().as_bytes())),
                };
                Some((runtime.client.clone(), identity))
            });
        Ok(move || {
            turn.wait_with_observer(on_wait)?;
            let result = (|| {
                let current_project = || match clients.scope(request.client_id()) {
                    Some(ClientScope::Project(project)) => Some(project),
                    _ => None,
                };
                let (Some((client, identity)), Some(binding)) = (target, binding) else {
                    // Preserve the original attempted-sequence and error precedence
                    // for rejected live bindings, without preparing a physical input.
                    return request.dispatch(current_project, now, |_, _| {
                        Err(AuthorizationError::WrongProject)
                    });
                };
                let ready = executor.block_on(prepare_native_input(
                    &client,
                    &identity,
                    request.data().as_bytes().to_vec(),
                ))?;
                after_ready(&client, &ready);
                let mut effect_started = false;
                let mut final_outcome = None;
                let result = request.dispatch(current_project, now, |_, _| {
                    binding
                        .while_current(request.project(), || {
                            effect_started = true;
                            let outcome =
                                executor.block_on(commit_native_input(&client, &ready))?;
                            let result = if outcome.phase == OwnerMobileInputPhase::Written {
                                Ok(outcome.clone())
                            } else {
                                Err(AuthorizationError::Unavailable)
                            };
                            final_outcome = Some(outcome);
                            result
                        })
                        .ok_or(AuthorizationError::WrongProject)?
                });
                if !effect_started {
                    // No successful/unknown physical effect may escape its Host
                    // fences: commit_native_input resolves those before returning.
                    final_outcome = Some(executor.block_on(cancel_native_input(
                        &client,
                        &identity,
                        Some(&ready),
                    ))?);
                }
                // The definite physical outcome ends authorization. Receipt
                // retirement stays on this same worker/turn, outside its fences.
                if let Some(outcome) = final_outcome {
                    executor.block_on(settle_native_input(&client, &outcome.ready));
                }
                result
            })();
            turn.finish()?;
            result
        })
    }
}

fn owner_is_gone(client: &ProjectRuntimeClient) -> bool {
    client.identity().is_some_and(|identity| {
        gwt::process::host_process_liveness(identity.pid) == Some(false)
            || gwt::process::host_process_start_time(identity.pid)
                .is_some_and(|birth| birth != identity.host_started_at)
    })
}

fn input_outcome(
    value: serde_json::Value,
    identity: &OwnerMobileInputIdentity,
    expected: Option<&OwnerMobileInputReady>,
) -> Result<OwnerMobileInputOutcome, AuthorizationError> {
    let outcome: OwnerMobileInputOutcome =
        serde_json::from_value(value).map_err(|_| AuthorizationError::Unavailable)?;
    if &outcome.ready.identity != identity || expected.is_some_and(|ready| ready != &outcome.ready)
    {
        return Err(AuthorizationError::Unavailable);
    }
    Ok(outcome)
}

async fn settle_native_input(client: &ProjectRuntimeClient, ready: &OwnerMobileInputReady) {
    // This is receipt retirement, after a definite physical outcome. Retrying
    // it cannot write bytes and must not accidentally remove a new nonce.
    let started = Instant::now();
    let mut reported = false;
    while !owner_is_gone(client) {
        if client
            .mobile_input_command(OwnerCommand::SettleMobileInput {
                ready: ready.clone(),
            })
            .await
            .is_ok()
        {
            return;
        }
        if !reported && started.elapsed() >= Duration::from_secs(30) {
            reported = true;
            tracing::error!(operation_id = %ready.identity.operation_id,
                "native input finished but receipt retirement is unresolved; retaining its scheduling turn");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_native_input(
    client: &ProjectRuntimeClient,
    identity: &OwnerMobileInputIdentity,
    expected: Option<&OwnerMobileInputReady>,
    preparing: bool,
) -> Result<OwnerMobileInputOutcome, AuthorizationError> {
    let started = Instant::now();
    let mut reported = false;
    loop {
        if let Ok(value) = client
            .mobile_input_command(OwnerCommand::MobileInputOutcome {
                identity: identity.clone(),
            })
            .await
        {
            if let Ok(outcome) = input_outcome(value, identity, expected) {
                if outcome.phase.is_finished()
                    || (preparing && outcome.phase == OwnerMobileInputPhase::Ready)
                {
                    return Ok(outcome);
                }
            }
        }
        if owner_is_gone(client) {
            return Err(AuthorizationError::Unavailable);
        }
        if !reported && started.elapsed() >= Duration::from_secs(30) {
            reported = true;
            tracing::error!(operation_id = %identity.operation_id,
                "native mobile input is unresolved; retaining its turn and any accepted Host fences");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn cancel_native_input(
    client: &ProjectRuntimeClient,
    identity: &OwnerMobileInputIdentity,
    expected: Option<&OwnerMobileInputReady>,
) -> Result<OwnerMobileInputOutcome, AuthorizationError> {
    let started = Instant::now();
    let mut reported = false;
    loop {
        if let Ok(value) = client
            .mobile_input_command(OwnerCommand::CancelMobileInput {
                identity: identity.clone(),
            })
            .await
        {
            let Ok(outcome) = input_outcome(value, identity, expected) else {
                if owner_is_gone(client) {
                    return Err(AuthorizationError::Unavailable);
                }
                if !reported {
                    reported = true;
                    tracing::error!(operation_id = %identity.operation_id,
                        "native input cancellation identity mismatch; retaining Host fences");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            let outcome = if outcome.phase.is_finished() {
                outcome
            } else {
                wait_native_input(client, identity, Some(&outcome.ready), false).await?
            };
            return Ok(outcome);
        }
        if owner_is_gone(client) {
            return Err(AuthorizationError::Unavailable);
        }
        if !reported && started.elapsed() >= Duration::from_secs(30) {
            reported = true;
            tracing::error!(operation_id = %identity.operation_id,
                "native input cancellation is unresolved; retaining its turn and any accepted Host fences");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn prepare_native_input(
    client: &ProjectRuntimeClient,
    identity: &OwnerMobileInputIdentity,
    data: Vec<u8>,
) -> Result<OwnerMobileInputReady, AuthorizationError> {
    let result = client
        .mobile_input_command(OwnerCommand::PrepareMobileInput {
            identity: identity.clone(),
            data,
        })
        .await;
    let outcome = match result {
        Ok(value) => match input_outcome(value, identity, None) {
            Ok(outcome) => outcome,
            Err(_) => {
                let outcome = cancel_native_input(client, identity, None).await?;
                settle_native_input(client, &outcome.ready).await;
                return Err(AuthorizationError::Unavailable);
            }
        },
        Err(_) => {
            let outcome = cancel_native_input(client, identity, None).await?;
            settle_native_input(client, &outcome.ready).await;
            return Err(AuthorizationError::Unavailable);
        }
    };
    let outcome = if outcome.phase == OwnerMobileInputPhase::Preparing {
        wait_native_input(client, identity, Some(&outcome.ready), true).await?
    } else {
        outcome
    };
    if outcome.phase != OwnerMobileInputPhase::Ready {
        settle_native_input(client, &outcome.ready).await;
        return Err(AuthorizationError::Unavailable);
    }
    Ok(outcome.ready)
}

async fn commit_native_input(
    client: &ProjectRuntimeClient,
    ready: &OwnerMobileInputReady,
) -> Result<OwnerMobileInputOutcome, AuthorizationError> {
    let result = client
        .mobile_input_command(OwnerCommand::CommitMobileInput {
            ready: ready.clone(),
        })
        .await;
    let outcome = match result {
        Ok(value) => match input_outcome(value, &ready.identity, Some(ready)) {
            Ok(outcome) if outcome.phase.is_finished() => outcome,
            Ok(_) => wait_native_input(client, &ready.identity, Some(ready), false).await?,
            Err(_) => cancel_native_input(client, &ready.identity, Some(ready)).await?,
        },
        Err(_) => cancel_native_input(client, &ready.identity, Some(ready)).await?,
    };
    Ok(outcome)
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::{
        combined_window_id,
        tests::{
            configure_owner_fixture_start_gate, drain_browser_owner_input, owner_fixture_block_on,
            sample_runtime,
        },
        BlockingTaskSpawner,
    };
    use super::*;
    use crate::UserEvent;
    use gwt_core::test_support::ScopedGwtHome;
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Mutex, RwLock,
        },
        time::Duration,
    };
    use tempfile::tempdir;

    fn paired_connection(
        auth: &Arc<RwLock<gwt::mobile_connection::MobileAuthorization>>,
        project: &gwt_core::repo_hash::ProjectKey,
    ) -> (
        Arc<gwt::mobile_connection::MobileConnection>,
        gwt::mobile_connection::DeviceRecord,
    ) {
        use ring::{
            rand::SystemRandom,
            signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
        };
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let (device, session) = {
            let mut authority = auth.write().unwrap();
            let pending = authority
                .request_pairing("Phone", key.public_key().as_ref(), project.clone(), 1000)
                .unwrap();
            let device = authority.approve(&pending.id, 1000).unwrap();
            let challenge = authority.challenge(device.id(), project, 1000).unwrap();
            let signature = key.sign(&rng, &challenge.signing_payload).unwrap();
            let session = authority
                .authenticate(&challenge.id, signature.as_ref(), 1000)
                .unwrap();
            (device, session)
        };
        (
            Arc::new(gwt::mobile_connection::MobileConnection::new(
                auth.clone(),
                session,
            )),
            device,
        )
    }

    #[test]
    fn mobile_input_queue_rechecks_authority_before_writing_real_pty() {
        use gwt::mobile_connection::{AuthorizationError, MobileAuthorization, MobileInputRequest};
        let io = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let _io = io.enter();
        let temp = tempdir().unwrap();
        let _gwt_home = ScopedGwtHome::set(temp.path());
        std::fs::create_dir_all(temp.path().join("project")).unwrap();
        let mut workspace = gwt::WindowRegistry::from_persisted(gwt::empty_workspace_state());
        workspace.add_window(gwt::WindowPreset::Shell);
        let tab = super::super::ProjectTabRuntime {
            id: "mobile".into(),
            title: "Mobile".into(),
            project_root: temp.path().join("project"),
            kind: gwt::ProjectKind::Git,
            workspace,
            migration_pending: false,
            main_worktree_root_cache: Arc::new(std::sync::OnceLock::new()),
        };
        let raw_id = tab.workspace.persisted().windows[0].id.clone();
        let window_id = combined_window_id("mobile", &raw_id);
        let mut app = sample_runtime(temp.path(), vec![tab], Some("mobile"));
        let project = app.project_key_for_tab("mobile").unwrap().clone();
        use gwt::project_runtime::{events::OwnerCommand, test_support::ProjectOwnerHarness};
        let owner = Arc::new(
            owner_fixture_block_on(ProjectOwnerHarness::start(&temp.path().join("project")))
                .unwrap(),
        );
        configure_owner_fixture_start_gate(&owner);
        let id = owner.pane_id("mobile-cat");
        let launched = owner_fixture_block_on(owner.command(OwnerCommand::LaunchShell {
            id: id.clone(),
            command: "/bin/cat".into(),
            args: vec![],
            cols: 80,
            rows: 24,
        }))
        .unwrap();
        let probe =
            owner_fixture_block_on(owner.probe(&id, launched["incarnation"].as_u64().unwrap()))
                .unwrap();
        let client = owner.client();
        let context = app.test_context();
        let state = app.project_state_mut(&context).unwrap();
        state.project_owner_connection_id = 1;
        state.project_owner_identity = client.identity().cloned();
        let snapshot = owner_fixture_block_on(owner.command(OwnerCommand::Snapshot {
            id: id.clone(),
            incarnation: probe.incarnation,
        }))
        .unwrap();
        app.install_remote_owner_projection(
            &context,
            &window_id,
            client,
            serde_json::from_value(snapshot).unwrap(),
            None,
        );
        app.blocking_tasks = BlockingTaskSpawner::queued().0;
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let auth = Arc::new(std::sync::RwLock::new(auth));
        let (connection, device) = paired_connection(&auth, &project);
        let clients = ClientHub::default();
        clients.register_scoped(
            "phone".into(),
            crate::app_runtime::ClientScope::Project(project.clone()),
        );
        let request = |seq, data: &str| {
            Arc::new(MobileInputRequest::new(
                connection.clone(),
                "phone".into(),
                project.clone(),
                seq,
                window_id.clone(),
                data.into(),
            ))
        };
        let dispatch = |app: &AppRuntime, input: &Arc<MobileInputRequest>, time| {
            app.mobile_input_task(input.clone(), clients.clone(), move || time)
                .and_then(|task| task())
        };
        let first = request(0, "mobile-first\n");
        let second = request(1, "mobile-second\n");
        let mut queue = crate::app_runtime::AppEventQueue::default();
        queue.push_back(UserEvent::RuntimeDaemonEnsureTick).unwrap();
        queue
            .push_back(UserEvent::MobileInput(first.clone()))
            .unwrap();
        queue.push_back(UserEvent::MobileInput(second)).unwrap();
        let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
        let lost_reply = Arc::new(Mutex::new(None));
        let mut tasks = Vec::new();
        for index in 0..2 {
            let Some(UserEvent::MobileInput(input)) = queue.pop_front() else {
                panic!("both inputs must remain foreground FIFO")
            };
            let waiting_tx = waiting_tx.clone();
            let owner = owner.clone();
            let lost_reply = lost_reply.clone();
            let physical_input = probe.pty.clone();
            tasks.push(
                app.mobile_input_task_with_observers(
                    input,
                    clients.clone(),
                    || 1000,
                    move || {
                        if index == 1 {
                            waiting_tx.send(()).unwrap();
                        }
                    },
                    move |_, ready| {
                        assert_eq!(physical_input.ordinary_input_revision(), Some(index));
                        if index == 1 {
                            *lost_reply.lock().unwrap() = Some(
                                owner_fixture_block_on(
                                    owner.suppress_mobile_commit_reply(ready.clone()),
                                )
                                .unwrap(),
                            );
                        }
                    },
                )
                .unwrap(),
            );
        }
        let second_task = tasks.pop().unwrap();
        let first_task = tasks.pop().unwrap();
        let second_home = temp.path().to_path_buf();
        let first_home = second_home.clone();
        let (first_outcome, second_outcome) = std::thread::scope(|scope| {
            let second = scope.spawn(move || {
                let _home = ScopedGwtHome::set(second_home);
                second_task()
            });
            // The second worker is actually at the noncurrent Condvar turn,
            // before the first worker is allowed to start.
            gwt_core::test_support::recv_event(
                &waiting_rx,
                "second mobile worker waits for its FIFO turn",
            );
            assert_eq!(probe.pty.ordinary_input_revision(), Some(0));
            let first = scope.spawn(move || {
                let _home = ScopedGwtHome::set(first_home);
                first_task()
            });
            (
                first.join().unwrap().unwrap(),
                second.join().unwrap().unwrap(),
            )
        });
        assert_eq!(first_outcome.phase, OwnerMobileInputPhase::Written);
        assert_eq!(second_outcome.phase, OwnerMobileInputPhase::Written);
        let written = second_outcome.observation.unwrap();
        assert_eq!(written.revision, 2);
        assert!(!written.has_unsent_user_input);
        assert!(written.human_input_elapsed_ms.is_some());
        assert!(lost_reply
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire));
        // test-hygiene: allow-wall-clock-deadline hang guard for actual cat output parsed by its sole native reader
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let observed = loop {
            let observed = probe.pane.lock().unwrap().screen().contents();
            if observed.contains("mobile-second") {
                break observed;
            }
            // test-hygiene: allow-wall-clock-deadline hang guard only; actual native cat output, not elapsed time, proves delivery
            assert!(std::time::Instant::now() < deadline, "{observed:?}");
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(observed.find("mobile-first") < observed.find("mobile-second"));
        let revision = probe.pty.ordinary_input_revision();
        assert_eq!(revision, Some(2));
        assert_eq!(
            dispatch(&app, &first, 1000).unwrap_err(),
            AuthorizationError::Replay
        );
        let foreign = Arc::new(MobileInputRequest::new(
            connection.clone(),
            "phone".into(),
            project.clone(),
            2,
            "foreign::pane".into(),
            "must-not-write\n".into(),
        ));
        assert_eq!(
            dispatch(&app, &foreign, 1000).unwrap_err(),
            AuthorizationError::WrongProject
        );
        let pending = request(3, "must-not-write\n");
        assert_eq!(
            dispatch(&app, &pending, 1300).unwrap_err(),
            AuthorizationError::Expired
        );
        clients.unregister("phone");
        assert_eq!(
            dispatch(&app, &pending, 1000).unwrap_err(),
            AuthorizationError::WrongProject
        );
        clients.register_scoped(
            "phone".into(),
            crate::app_runtime::ClientScope::Project(project.clone()),
        );
        let ready_request = |connection| {
            Arc::new(MobileInputRequest::new(
                connection,
                "phone".into(),
                project.clone(),
                0,
                window_id.clone(),
                "must-not-write\n".into(),
            ))
        };
        let assert_unchanged = || {
            assert_eq!(probe.pty.ordinary_input_revision(), revision);
            assert!(!probe
                .pane
                .lock()
                .unwrap()
                .screen()
                .contents()
                .contains("must-not-write"));
        };

        // Host time is read after native Ready and both authorization fences.
        let (expiring_connection, _) = paired_connection(&auth, &project);
        let clock = Arc::new(AtomicU64::new(1000));
        let read_clock = clock.clone();
        let expiring = app
            .mobile_input_task_with_observers(
                ready_request(expiring_connection),
                clients.clone(),
                move || read_clock.load(Ordering::Acquire),
                || {},
                move |_, _| {
                    clock.store(1300, Ordering::Release);
                },
            )
            .unwrap();
        assert_eq!(expiring().unwrap_err(), AuthorizationError::Expired);
        assert_unchanged();

        let (revoking_connection, revoking_device) = paired_connection(&auth, &project);
        let authority = auth.clone();
        let revoking = app
            .mobile_input_task_with_observers(
                ready_request(revoking_connection),
                clients.clone(),
                || 1000,
                || {},
                move |_, _| {
                    authority
                        .write()
                        .unwrap()
                        .revoke(revoking_device.id())
                        .unwrap();
                },
            )
            .unwrap();
        assert_eq!(revoking().unwrap_err(), AuthorizationError::Revoked);
        assert_unchanged();

        let (disconnecting_connection, _) = paired_connection(&auth, &project);
        let disconnecting_clients = clients.clone();
        let disconnecting = app
            .mobile_input_task_with_observers(
                ready_request(disconnecting_connection),
                clients.clone(),
                || 1000,
                || {},
                move |_, _| {
                    disconnecting_clients.unregister("phone");
                },
            )
            .unwrap();
        assert_eq!(
            disconnecting().unwrap_err(),
            AuthorizationError::WrongProject
        );
        assert_unchanged();
        clients.register_scoped("phone".into(), ClientScope::Project(project.clone()));

        let (rebinding_connection, _) = paired_connection(&auth, &project);
        let rebinding_clients = clients.clone();
        let foreign_project = gwt_core::paths::project_scope_hash(&temp.path().join("foreign"));
        let rebinding = app
            .mobile_input_task_with_observers(
                ready_request(rebinding_connection),
                clients.clone(),
                || 1000,
                || {},
                move |_, _| {
                    rebinding_clients
                        .register_scoped("phone".into(), ClientScope::Project(foreign_project));
                },
            )
            .unwrap();
        assert_eq!(rebinding().unwrap_err(), AuthorizationError::WrongProject);
        assert_unchanged();
        clients.register_scoped("phone".into(), ClientScope::Project(project.clone()));

        // Replacing the registration with the same Project must also invalidate
        // the captured old client lifetime; looking up its string id is insufficient.
        let (reconnected_connection, _) = paired_connection(&auth, &project);
        let reconnected_clients = clients.clone();
        let reconnected_project = project.clone();
        let reconnected = app
            .mobile_input_task_with_observers(
                ready_request(reconnected_connection),
                clients.clone(),
                || 1000,
                || {},
                move |_, _| {
                    reconnected_clients
                        .register_scoped("phone".into(), ClientScope::Project(reconnected_project));
                },
            )
            .unwrap();
        assert_eq!(reconnected().unwrap_err(), AuthorizationError::WrongProject);
        assert_unchanged();

        let (deadline_connection, _) = paired_connection(&auth, &project);
        let native_now = gwt_core::operation_deadline::now();
        owner_fixture_block_on(owner.set_operation_clock(Some(native_now))).unwrap();
        let deadline_owner = owner.clone();
        let deadline_input = app
            .mobile_input_task_with_observers(
                ready_request(deadline_connection),
                clients.clone(),
                || 1000,
                || {},
                move |_, _| {
                    owner_fixture_block_on(
                        deadline_owner
                            .set_operation_clock(Some(native_now + Duration::from_secs(31))),
                    )
                    .unwrap();
                },
            )
            .unwrap();
        assert_eq!(
            deadline_input().unwrap_err(),
            AuthorizationError::Unavailable
        );
        owner_fixture_block_on(owner.set_operation_clock(None)).unwrap();
        assert_unchanged();

        // Use the actual private input protocol: early cancellation must fence
        // an overtaken Prepare, and a finished cancellation must reject late Commit.
        use sha2::{Digest, Sha256};
        let native = owner.client();
        let private_identity = |data: &[u8]| OwnerMobileInputIdentity {
            operation_id: uuid::Uuid::new_v4().to_string(),
            host_instance_id: native.identity().unwrap().host_instance_id.clone(),
            id: probe.id.clone(),
            incarnation: probe.incarnation,
            session_id: probe.session.as_ref().map(|session| session.id.clone()),
            data_sha256: hex::encode(Sha256::digest(data)),
        };
        let data = b"must-not-write\n".to_vec();
        assert!(owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::PrepareMobileInput {
                identity: private_identity(&data),
                data: b"changed-body\n".to_vec(),
            }
        ))
        .is_err());
        assert_unchanged();
        let early_identity = private_identity(&data);
        let cancelled =
            owner_fixture_block_on(cancel_native_input(&native, &early_identity, None)).unwrap();
        assert_eq!(cancelled.phase, OwnerMobileInputPhase::Prevented);
        let late_prepare = owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::PrepareMobileInput {
                identity: early_identity.clone(),
                data: data.clone(),
            },
        ))
        .unwrap();
        assert_eq!(
            input_outcome(late_prepare, &early_identity, Some(&cancelled.ready))
                .unwrap()
                .phase,
            OwnerMobileInputPhase::Prevented
        );
        let late_commit = owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::CommitMobileInput {
                ready: cancelled.ready.clone(),
            },
        ))
        .unwrap();
        assert_eq!(
            input_outcome(late_commit, &early_identity, Some(&cancelled.ready))
                .unwrap()
                .phase,
            OwnerMobileInputPhase::Prevented
        );
        owner_fixture_block_on(settle_native_input(&native, &cancelled.ready));
        assert_unchanged();

        // A retired operation id may receive a new preparation nonce; its old
        // Commit and a real old ACK cannot be adopted by that fresh operation.
        let fresh_ready =
            owner_fixture_block_on(prepare_native_input(&native, &early_identity, data)).unwrap();
        assert_ne!(fresh_ready.nonce, cancelled.ready.nonce);
        owner_fixture_block_on(native.command(OwnerCommand::Health)).unwrap();
        assert_eq!(probe.pty.ordinary_input_revision(), revision);
        assert!(owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::CommitMobileInput {
                ready: cancelled.ready,
            }
        ))
        .is_err());
        assert_eq!(
            input_outcome(
                serde_json::to_value(&second_outcome).unwrap(),
                &early_identity,
                Some(&fresh_ready)
            )
            .unwrap_err(),
            AuthorizationError::Unavailable
        );
        let pending_cancel = owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::CancelMobileInput {
                identity: early_identity.clone(),
            },
        ))
        .unwrap();
        assert_eq!(
            input_outcome(pending_cancel, &early_identity, Some(&fresh_ready))
                .unwrap()
                .phase,
            OwnerMobileInputPhase::Ready,
            "cancel request is not a physical worker completion ACK"
        );
        let final_cancel = owner_fixture_block_on(cancel_native_input(
            &native,
            &early_identity,
            Some(&fresh_ready),
        ))
        .unwrap();
        assert_eq!(final_cancel.phase, OwnerMobileInputPhase::Prevented);
        owner_fixture_block_on(settle_native_input(&native, &fresh_ready));
        assert_unchanged();

        let (abandoned_connection, _) = paired_connection(&auth, &project);
        let abandoned = app
            .mobile_input_task(
                ready_request(abandoned_connection.clone()),
                clients.clone(),
                || 1000,
            )
            .unwrap();
        drop(abandoned);
        assert_eq!(
            app.mobile_input_task(ready_request(abandoned_connection), clients.clone(), || {
                1000
            },)
                .err()
                .unwrap(),
            AuthorizationError::Unavailable
        );
        assert_unchanged();

        // A queued native RPC must not retain authorization from this Host call.
        app.handle_mobile_input(request(4, "late-after-revoke\n"), &clients, || 1000)
            .unwrap();
        auth.write().unwrap().revoke(device.id()).unwrap();
        drain_browser_owner_input(&mut app);
        assert_eq!(
            dispatch(&app, &request(4, "revoked\n"), 1000).unwrap_err(),
            AuthorizationError::Revoked
        );
        let after_revoke = probe.pty.ordinary_input_revision();

        // Reuse the existing producing-owner fixture to replace a real Agent
        // Session at the same native pane address, rather than inventing ids.
        let (mut agent_app, agent_probe, agent_window) =
            super::super::tests::owner_browser_input_fixture(temp.path());
        let agent_owner = agent_app
            .test_project_owners
            .values()
            .next()
            .unwrap()
            .clone();
        let agent_project = agent_app
            .project_key_for_window(&agent_window)
            .unwrap()
            .clone();
        let (agent_connection, _) = paired_connection(&auth, &agent_project);
        let agent_clients = ClientHub::default();
        agent_clients.register_scoped(
            "agent-phone".into(),
            ClientScope::Project(agent_project.clone()),
        );
        let old_session = agent_probe.session.as_ref().unwrap();
        let mut config = gwt_agent::AgentLaunchBuilder::new(old_session.agent_id.clone())
            .working_dir(&old_session.worktree_path)
            .branch(&old_session.branch)
            .build();
        config.command = old_session.launch_command.clone();
        config.args = old_session.launch_args.clone();
        let replacement = Arc::new(Mutex::new(None));
        let saved_replacement = replacement.clone();
        let replace_owner = agent_owner.clone();
        let stale_agent = agent_app
            .mobile_input_task_with_observers(
                Arc::new(MobileInputRequest::new(
                    agent_connection,
                    "agent-phone".into(),
                    agent_project,
                    0,
                    agent_window,
                    "must-not-write\n".into(),
                )),
                agent_clients,
                || 1000,
                || {},
                move |_, ready| {
                    owner_fixture_block_on(replace_owner.command(OwnerCommand::Close {
                        id: ready.identity.id.clone(),
                        incarnation: ready.identity.incarnation,
                    }))
                    .unwrap();
                    let launched =
                        owner_fixture_block_on(replace_owner.command(OwnerCommand::LaunchAgent {
                            id: ready.identity.id.clone(),
                            operation_id: uuid::Uuid::new_v4().to_string(),
                            tab_id: "replacement".into(),
                            config: Box::new(config),
                            metadata: Box::default(),
                            cols: 80,
                            rows: 24,
                        }))
                        .unwrap();
                    *saved_replacement.lock().unwrap() = Some(
                        owner_fixture_block_on(replace_owner.probe(
                            &ready.identity.id,
                            launched["incarnation"].as_u64().unwrap(),
                        ))
                        .unwrap(),
                    );
                },
            )
            .unwrap();
        assert_eq!(stale_agent().unwrap_err(), AuthorizationError::Unavailable);
        let replacement = replacement.lock().unwrap().take().unwrap();
        assert_eq!(replacement.id, agent_probe.id);
        assert_ne!(replacement.incarnation, agent_probe.incarnation);
        assert_ne!(replacement.session.as_ref().unwrap().id, old_session.id);
        assert_eq!(replacement.pty.ordinary_input_revision(), Some(0));
        assert!(!agent_probe.current.load(Ordering::Acquire));

        // A real, foreign Session with the still-current shell incarnation
        // must fail the independent Session check, even without an inc mismatch.
        let foreign_data = b"must-not-write\n".to_vec();
        let mut foreign_session = private_identity(&foreign_data);
        foreign_session.session_id = Some(replacement.session.as_ref().unwrap().id.clone());
        assert!(owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::PrepareMobileInput {
                identity: foreign_session,
                data: foreign_data,
            }
        ))
        .is_err());
        let mut foreign_owner = private_identity(b"must-not-write\n");
        foreign_owner.host_instance_id = agent_owner
            .client()
            .identity()
            .unwrap()
            .host_instance_id
            .clone();
        assert!(owner_fixture_block_on(native.mobile_input_command(
            OwnerCommand::PrepareMobileInput {
                identity: foreign_owner,
                data: b"must-not-write\n".to_vec(),
            }
        ))
        .is_err());
        assert_unchanged();
        owner_fixture_block_on(agent_owner.command(OwnerCommand::Close {
            id: replacement.id,
            incarnation: replacement.incarnation,
        }))
        .unwrap();
        agent_app.test_project_owners.clear();
        owner_fixture_block_on(Arc::try_unwrap(agent_owner).ok().unwrap().shutdown());
        owner_fixture_block_on(owner.command(OwnerCommand::Close {
            id,
            incarnation: probe.incarnation,
        }))
        .unwrap();
        owner_fixture_block_on(Arc::try_unwrap(owner).ok().unwrap().shutdown());
        assert_eq!(
            after_revoke, revision,
            "revoked queued input must not reach the actual native PTY"
        );
    }
}
