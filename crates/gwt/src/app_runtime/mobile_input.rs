//! Only synchronous terminal input is admitted on the mobile queue.
use super::{AppRuntime, ClientHub, ClientScope, OutboundEvent};
use gwt::mobile_connection::{AuthorizationError, MobileInputRequest};

impl AppRuntime {
    pub(crate) fn handle_mobile_input(
        &mut self,
        request: &MobileInputRequest,
        clients: &ClientHub,
        now: impl FnOnce() -> u64,
    ) -> Result<Vec<OutboundEvent>, AuthorizationError> {
        request.dispatch(
            || match clients.scope(request.client_id()) {
                Some(ClientScope::Project(project)) => Some(project),
                _ => None,
            },
            now,
            |id, data| {
                if self.project_key_for_window(id) != Some(request.project()) {
                    return Err(AuthorizationError::WrongProject);
                }
                // This handler performs the actual synchronous PTY operation.
                // Never convert this request to Frontend or deferred work.
                Ok(self.terminal_input_events(id, data))
            },
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::{combined_window_id, tests::sample_runtime};
    use super::*;
    use crate::UserEvent;
    use gwt_core::test_support::ScopedGwtHome;
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    use tempfile::tempdir;
    #[test]
    fn mobile_input_queue_rechecks_authority_before_writing_real_pty() {
        use gwt::mobile_connection::{
            AuthorizationError, MobileAuthorization, MobileConnection, MobileInputRequest,
        };
        use ring::{
            rand::SystemRandom,
            signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
        };
        let temp = tempdir().unwrap();
        let _gwt_home = ScopedGwtHome::set(temp.path());
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
        let pane = gwt_terminal::Pane::new(
            window_id.clone(),
            "/bin/cat".into(),
            vec![],
            80,
            24,
            HashMap::new(),
            Some(PathBuf::from("/")),
        )
        .unwrap();
        let mut reader = pane.reader().unwrap();
        let pane = Arc::new(Mutex::new(pane));
        app.runtimes.insert(
            window_id.clone(),
            crate::app_runtime::WindowRuntime::new(
                crate::app_runtime::next_window_runtime_incarnation(),
                pane.clone(),
            ),
        );
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let output = std::thread::spawn(move || {
            use std::io::Read;
            let mut bytes = [0; 256];
            let mut observed = Vec::new();
            while let Ok(len) = reader.read(&mut bytes) {
                if len == 0 {
                    break;
                }
                observed.extend_from_slice(&bytes[..len]);
                if String::from_utf8_lossy(&observed).contains("mobile-second") {
                    let _ = read_tx.send(());
                    observed.clear();
                }
            }
        });
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let pending = auth
            .request_pairing("Phone", key.public_key().as_ref(), project.clone(), 1000)
            .unwrap();
        let device = auth.approve(&pending.id, 1000).unwrap();
        let challenge = auth.challenge(device.id(), &project, 1000).unwrap();
        let signature = key.sign(&rng, &challenge.signing_payload).unwrap();
        let session = auth
            .authenticate(&challenge.id, signature.as_ref(), 1000)
            .unwrap();
        let auth = Arc::new(std::sync::RwLock::new(auth));
        let connection = Arc::new(MobileConnection::new(auth.clone(), session));
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
        let first = request(0, "mobile-first\n");
        let second = request(1, "mobile-second\n");
        let mut queue = crate::app_runtime::AppEventQueue::default();
        queue.push_back(UserEvent::RuntimeDaemonEnsureTick).unwrap();
        queue
            .push_back(UserEvent::MobileInput(first.clone()))
            .unwrap();
        queue.push_back(UserEvent::MobileInput(second)).unwrap();
        for _ in 0..2 {
            let Some(UserEvent::MobileInput(input)) = queue.pop_front() else {
                panic!("both inputs must remain foreground FIFO")
            };
            app.handle_mobile_input(&input, &clients, || 1000).unwrap();
        }
        gwt_core::test_support::recv_event(&read_rx, "mobile input reached the real PTY");
        let revision = pane.lock().unwrap().pty().ordinary_input_revision();
        assert_eq!(revision, Some(2));
        assert_eq!(
            app.handle_mobile_input(&first, &clients, || 1000)
                .unwrap_err(),
            AuthorizationError::Replay
        );
        let foreign = MobileInputRequest::new(
            connection.clone(),
            "phone".into(),
            project.clone(),
            2,
            "foreign::pane".into(),
            "must-not-write\n".into(),
        );
        assert_eq!(
            app.handle_mobile_input(&foreign, &clients, || 1000)
                .unwrap_err(),
            AuthorizationError::WrongProject
        );
        let pending = request(3, "must-not-write\n");
        assert_eq!(
            app.handle_mobile_input(&pending, &clients, || 1300)
                .unwrap_err(),
            AuthorizationError::Expired
        );
        clients.unregister("phone");
        assert_eq!(
            app.handle_mobile_input(&pending, &clients, || 1000)
                .unwrap_err(),
            AuthorizationError::WrongProject
        );
        clients.register_scoped(
            "phone".into(),
            crate::app_runtime::ClientScope::Project(project.clone()),
        );
        auth.write().unwrap().revoke(device.id()).unwrap();
        assert_eq!(
            app.handle_mobile_input(&request(4, "revoked\n"), &clients, || 1000)
                .unwrap_err(),
            AuthorizationError::Revoked
        );
        assert_eq!(
            pane.lock().unwrap().pty().ordinary_input_revision(),
            revision
        );
        pane.lock().unwrap().kill().unwrap();
        output.join().unwrap();
    }
}
