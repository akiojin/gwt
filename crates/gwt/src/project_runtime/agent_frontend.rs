//! Generation and durable-authority gates shared by the daemon and GUI adapters.
//! The accepted operation executes inside the existing authority fences.
use crate::{
    agent_capability::{
        AgentCapabilityGrant, AgentCapabilityIssuer, AgentDurableAuthority, AgentFrontendRequest,
        AgentPmSendResponder, AgentSelfCloseResponder, AgentSessionPrincipal,
    },
    project_transport::OutboundEvent,
    BackendEvent,
};
use std::path::Path;

#[derive(Debug)]
pub enum AgentFrontendDispatchOutcome {
    Dispatched(Vec<OutboundEvent>),
    StaleCapability,
    ExecutionAuthorityUnavailable,
}

/// Only the owning runtime adapter supplies the physical operation.
pub enum AcceptedAgentRequest {
    Principal {
        principal: AgentSessionPrincipal,
        request: AgentFrontendRequest,
    },
    SelfClose {
        grant: AgentCapabilityGrant,
        id: String,
        request_id: String,
        responder: AgentSelfCloseResponder,
    },
    PmSend {
        grant: AgentCapabilityGrant,
        operation_id: String,
        window_id: String,
        text: String,
        responder: Option<AgentPmSendResponder>,
    },
}

/// Run the physical mutation inside `dispatch`, before returning its events.
/// Enqueueing a later mutation from this closure would release its fences early.
pub fn dispatch_if_current(
    issuer: Option<&AgentCapabilityIssuer>,
    sessions_dir: &Path,
    client_id: String,
    grant: AgentCapabilityGrant,
    request: AgentFrontendRequest,
    dispatch: impl FnOnce(String, AcceptedAgentRequest) -> Vec<OutboundEvent>,
) -> AgentFrontendDispatchOutcome {
    let Some(issuer) = issuer else {
        return AgentFrontendDispatchOutcome::StaleCapability;
    };
    if !issuer.grant_is_current(&grant) {
        tracing::warn!(
            target: "gwt_security",
            "queued agent pane request rejected after capability rotation or revoke"
        );
        return AgentFrontendDispatchOutcome::StaleCapability;
    }
    if request.requires_producing_authority() && !grant.principal().authorizes_producing_mutation()
    {
        tracing::warn!(
            target: "gwt_security",
            "observation-only agent pane request rejected before runtime mutation"
        );
        return AgentFrontendDispatchOutcome::StaleCapability;
    }
    let durable_mutation =
        grant.principal().authorizes_producing_mutation() && request.mutates_host_state();
    if durable_mutation {
        match issuer.durable_authority(&grant) {
            AgentDurableAuthority::Current => {}
            AgentDurableAuthority::Stale | AgentDurableAuthority::ObservationOnly => {
                tracing::warn!(
                    target: "gwt_security",
                    "queued agent pane request rejected by durable execution binding fence"
                );
                return AgentFrontendDispatchOutcome::StaleCapability;
            }
            AgentDurableAuthority::PendingActivation | AgentDurableAuthority::Unavailable => {
                tracing::warn!(
                    target: "gwt_security",
                    "queued agent pane request rejected because execution authority is unavailable"
                );
                return AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable;
            }
        }
        #[cfg(any(test, feature = "test-gh-guard"))]
        run_agent_dispatch_test_hook(&AGENT_AFTER_DURABLE_CHECK_TEST_HOOK);
        #[cfg(any(test, feature = "test-gh-guard"))]
        run_owner_agent_dispatch_test_hook(&grant, OwnerAgentDispatchTestStage::AfterDurableCheck);
        if request.requires_producing_authority() {
            let binding = grant
                .principal()
                .active_execution_binding()
                .expect("producing principal carries an active binding")
                .clone();
            return match crate::cli::execution_state::with_current_active_execution_binding_lease_wait(
                    sessions_dir,
                    &binding,
                    std::time::Duration::ZERO,
                    || {
                        #[cfg(any(test, feature = "test-gh-guard"))]
                        run_agent_dispatch_test_hook(&AGENT_LEASED_MUTATION_TEST_HOOK);
                    #[cfg(any(test, feature = "test-gh-guard"))]
                    run_owner_agent_dispatch_test_hook(&grant, OwnerAgentDispatchTestStage::LeasedMutation);
                        dispatch_current(issuer, client_id, grant, request, dispatch)
                    },
                ) {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => {
                        tracing::warn!(
                            target: "gwt_security",
                            "queued agent pane request rejected after leased execution authority changed"
                        );
                        AgentFrontendDispatchOutcome::StaleCapability
                    }
                    Err(_) => {
                        tracing::warn!(
                            target: "gwt_security",
                            "queued agent pane request rejected because leased execution authority is unavailable"
                        );
                        AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable
                    }
                };
        }
    }

    dispatch_current(issuer, client_id, grant, request, dispatch)
}

fn dispatch_current(
    issuer: &AgentCapabilityIssuer,
    client_id: String,
    grant: AgentCapabilityGrant,
    request: AgentFrontendRequest,
    dispatch: impl FnOnce(String, AcceptedAgentRequest) -> Vec<OutboundEvent>,
) -> AgentFrontendDispatchOutcome {
    match request {
        AgentFrontendRequest::CloseWindow {
            id,
            request_id: Some(request_id),
            responder: Some(responder),
        } => AgentFrontendDispatchOutcome::Dispatched(dispatch(
            client_id,
            AcceptedAgentRequest::SelfClose {
                grant,
                id,
                request_id,
                responder,
            },
        )),
        AgentFrontendRequest::CloseWindow {
            id,
            request_id: Some(_),
            ..
        }
        | AgentFrontendRequest::CloseWindow {
            id,
            responder: Some(_),
            ..
        } => {
            tracing::warn!(
                target: "gwt_security",
                "correlated agent self-close rejected without an origin response channel"
            );
            // Issue #3629 AC-12: even this should-never-happen refusal
            // answers explicitly instead of silently producing no events.
            AgentFrontendDispatchOutcome::Dispatched(vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneCloseResult {
                    ok: false,
                    window_id: id,
                    reason: Some(
                        "a correlated self-close was not routed with its response channel"
                            .to_string(),
                    ),
                },
            )])
        }
        AgentFrontendRequest::CloseWindow {
            id,
            request_id: None,
            responder: None,
        } => {
            // Issue #3816: peer close may fence the target capability in
            // `queue_window_close_finalizer`. Accept the caller generation
            // first, then release the registry read lock before target
            // teardown needs the same registry's write lock.
            let Some(principal) = issuer.accept_current_grant(&grant) else {
                return AgentFrontendDispatchOutcome::StaleCapability;
            };
            #[cfg(any(test, feature = "test-gh-guard"))]
            run_agent_dispatch_test_hook(&AGENT_PEER_CLOSE_AFTER_ACCEPTANCE_TEST_HOOK);
            #[cfg(any(test, feature = "test-gh-guard"))]
            run_owner_agent_dispatch_test_hook(
                &grant,
                OwnerAgentDispatchTestStage::PeerCloseAccepted,
            );
            tracing::debug!(
                target: "gwt.pane.teardown",
                stage = "peer_close_grant_accepted",
                window_id = %id,
                "accepted peer pane close before target capability handoff"
            );
            AgentFrontendDispatchOutcome::Dispatched(dispatch(
                client_id,
                AcceptedAgentRequest::Principal {
                    principal,
                    request: AgentFrontendRequest::CloseWindow {
                        id,
                        request_id: None,
                        responder: None,
                    },
                },
            ))
        }
        request @ (AgentFrontendRequest::RecoverRestoredWindow { .. }
        | AgentFrontendRequest::CloseFleetPane { .. }) => {
            // Recovery uses peer teardown too: release the capability
            // registry read lock before closing the target window.
            let Some(principal) = issuer.accept_current_grant(&grant) else {
                return AgentFrontendDispatchOutcome::StaleCapability;
            };
            AgentFrontendDispatchOutcome::Dispatched(dispatch(
                client_id,
                AcceptedAgentRequest::Principal { principal, request },
            ))
        }
        AgentFrontendRequest::PmSendInput {
            operation_id,
            window_id,
            text,
            responder,
        } => AgentFrontendDispatchOutcome::Dispatched(dispatch(
            client_id,
            AcceptedAgentRequest::PmSend {
                grant,
                operation_id,
                window_id,
                text,
                responder,
            },
        )),
        request => {
            let principal = grant.principal().clone();
            issuer
                .with_current_grant(&grant, || {
                    dispatch(
                        client_id,
                        AcceptedAgentRequest::Principal { principal, request },
                    )
                })
                .map_or(AgentFrontendDispatchOutcome::StaleCapability, |events| {
                    AgentFrontendDispatchOutcome::Dispatched(events)
                })
        }
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
type AgentDispatchTestHook = std::cell::RefCell<Option<Box<dyn FnOnce()>>>;
#[cfg(any(test, feature = "test-gh-guard"))]
std::thread_local! {
    static AGENT_AFTER_DURABLE_CHECK_TEST_HOOK: AgentDispatchTestHook = const { AgentDispatchTestHook::new(None) };
    static AGENT_LEASED_MUTATION_TEST_HOOK: AgentDispatchTestHook = const { AgentDispatchTestHook::new(None) };
    static AGENT_PEER_CLOSE_AFTER_ACCEPTANCE_TEST_HOOK: AgentDispatchTestHook = const { AgentDispatchTestHook::new(None) };
}

#[cfg(any(test, feature = "test-gh-guard"))]
#[doc(hidden)]
pub fn set_agent_after_durable_check_test_hook(hook: impl FnOnce() + 'static) {
    AGENT_AFTER_DURABLE_CHECK_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(any(test, feature = "test-gh-guard"))]
#[doc(hidden)]
pub fn set_agent_leased_mutation_test_hook(hook: impl FnOnce() + 'static) {
    AGENT_LEASED_MUTATION_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(any(test, feature = "test-gh-guard"))]
#[doc(hidden)]
pub fn set_agent_peer_close_after_acceptance_test_hook(hook: impl FnOnce() + 'static) {
    AGENT_PEER_CLOSE_AFTER_ACCEPTANCE_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(any(test, feature = "test-gh-guard"))]
fn run_agent_dispatch_test_hook(slot: &'static std::thread::LocalKey<AgentDispatchTestHook>) {
    slot.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

// Worker-thread fixtures attach observers to a captured real owner principal;
// unlike a process-global hook they cannot affect another project's request.
#[cfg(any(test, feature = "test-gh-guard"))]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum OwnerAgentDispatchTestStage {
    BeforeDispatch,
    BeforeDispatchResult,
    AfterDurableCheck,
    LeasedMutation,
    PeerCloseAccepted,
    BeforeRegistryCleanup,
    BeforeFreshWorkCommit,
    AfterFreshActivationBeforePublication,
    BeforeGenesisReceiptCleanup,
}

#[cfg(any(test, feature = "test-gh-guard"))]
type OwnerAgentHookKey = (std::path::PathBuf, String, OwnerAgentDispatchTestStage);
#[cfg(any(test, feature = "test-gh-guard"))]
type OwnerAgentHookMap = std::collections::HashMap<OwnerAgentHookKey, Box<dyn FnOnce() + Send>>;
#[cfg(any(test, feature = "test-gh-guard"))]
fn owner_agent_hooks() -> &'static std::sync::Mutex<OwnerAgentHookMap> {
    static HOOKS: std::sync::OnceLock<std::sync::Mutex<OwnerAgentHookMap>> =
        std::sync::OnceLock::new();
    HOOKS.get_or_init(Default::default)
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub struct OwnerAgentDispatchTestHookGuard(OwnerAgentHookKey);
#[cfg(any(test, feature = "test-gh-guard"))]
impl Drop for OwnerAgentDispatchTestHookGuard {
    fn drop(&mut self) {
        owner_agent_hooks()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub fn set_owner_agent_dispatch_test_hook(
    grant: &AgentCapabilityGrant,
    stage: OwnerAgentDispatchTestStage,
    hook: impl FnOnce() + Send + 'static,
) -> OwnerAgentDispatchTestHookGuard {
    let key = (
        grant.principal().canonical_project_root().to_path_buf(),
        grant.principal().session_id().to_owned(),
        stage,
    );
    assert!(owner_agent_hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key.clone(), Box::new(hook))
        .is_none());
    OwnerAgentDispatchTestHookGuard(key)
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn run_owner_agent_dispatch_test_hook(
    grant: &AgentCapabilityGrant,
    stage: OwnerAgentDispatchTestStage,
) {
    let key = (
        grant.principal().canonical_project_root().to_path_buf(),
        grant.principal().session_id().to_owned(),
        stage,
    );
    let hook = owner_agent_hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
type OwnerAgentTraceKey = (std::path::PathBuf, String, OwnerAgentTraceRequest);
#[cfg(any(test, feature = "test-gh-guard"))]
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum OwnerAgentTraceRequest {
    Ready,
    CloseWindow,
    Other,
}
#[cfg(any(test, feature = "test-gh-guard"))]
fn owner_agent_traces(
) -> &'static std::sync::Mutex<std::collections::HashMap<OwnerAgentTraceKey, tracing::Dispatch>> {
    static TRACES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<OwnerAgentTraceKey, tracing::Dispatch>>,
    > = std::sync::OnceLock::new();
    TRACES.get_or_init(Default::default)
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub struct OwnerAgentTraceGuard(OwnerAgentTraceKey);
#[cfg(any(test, feature = "test-gh-guard"))]
impl Drop for OwnerAgentTraceGuard {
    fn drop(&mut self) {
        owner_agent_traces()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub fn capture_owner_agent_trace(
    grant: &AgentCapabilityGrant,
    request: OwnerAgentTraceRequest,
    trace: tracing::Dispatch,
) -> OwnerAgentTraceGuard {
    let key = (
        grant.principal().canonical_project_root().to_path_buf(),
        grant.principal().session_id().to_owned(),
        request,
    );
    assert!(owner_agent_traces()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key.clone(), trace)
        .is_none());
    OwnerAgentTraceGuard(key)
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn enter_owner_agent_trace(
    grant: &AgentCapabilityGrant,
    request: &AgentFrontendRequest,
) -> Option<tracing::dispatcher::DefaultGuard> {
    let request = match request {
        AgentFrontendRequest::Ready => OwnerAgentTraceRequest::Ready,
        AgentFrontendRequest::CloseWindow { .. } => OwnerAgentTraceRequest::CloseWindow,
        _ => OwnerAgentTraceRequest::Other,
    };
    let key = (
        grant.principal().canonical_project_root().to_path_buf(),
        grant.principal().session_id().to_owned(),
        request,
    );
    let trace = owner_agent_traces()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);
    trace.as_ref().map(tracing::dispatcher::set_default)
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn run_owner_agent_cleanup_test_hook(root: &Path, session_id: &str) {
    let key = (
        dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
        session_id.to_owned(),
        OwnerAgentDispatchTestStage::BeforeRegistryCleanup,
    );
    let hook = owner_agent_hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn run_owner_genesis_receipt_cleanup_test_hook(root: &Path, session_id: &str) {
    let key = (
        dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
        session_id.to_owned(),
        OwnerAgentDispatchTestStage::BeforeGenesisReceiptCleanup,
    );
    let hook = owner_agent_hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn run_owner_fresh_work_commit_test_hook(root: &Path, session_id: &str) {
    let key = (
        dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
        session_id.to_owned(),
        OwnerAgentDispatchTestStage::BeforeFreshWorkCommit,
    );
    let hook = owner_agent_hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn run_owner_fresh_activation_test_hook(root: &Path, session_id: &str) {
    let key = (
        dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
        session_id.to_owned(),
        OwnerAgentDispatchTestStage::AfterFreshActivationBeforePublication,
    );
    let hook = owner_agent_hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&key);
    if let Some(hook) = hook {
        hook();
    }
}
