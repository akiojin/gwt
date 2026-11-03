//! Owner PM delivery/lifecycle commits and shared read-only status projection.

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use crate::pm_registry::{self, PmLaunchProfile, PmRegistration};
use crate::{BackendEvent, PmAgentOption};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PmWakeInput {
    pub operation_id: String,
    pub wake_id: Option<String>,
    pub prompt: String,
    pub idle: bool,
    #[serde(default)]
    pub periodic_input: Option<PmPeriodicWakeInput>,
}

/// A selection-time physical observation; the owner rechecks every field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PmPeriodicWakeInput {
    pub session_id: String,
    pub incarnation: Option<u64>,
    pub revision: Option<u64>,
}

impl PmPeriodicWakeInput {
    pub(crate) fn matches(
        &self,
        session: Option<&str>,
        incarnation: u64,
        pty: &gwt_terminal::PtyHandle,
        interval_secs: u64,
    ) -> bool {
        session == Some(self.session_id.as_str())
            && self.incarnation == Some(incarnation)
            && self.revision.is_some()
            && pty.quiet_ordinary_input_revision(Duration::from_secs(interval_secs))
                == self.revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmWakeWrite {
    Injected,
    Unconfirmed,
    Deferred,
    Queued,
    Discarded,
}

/// Read-only result of one exact, physically completed PM lifecycle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PmLifecycleOutcome {
    pub session_id: String,
    pub deregistered: bool,
    pub respawn: bool,
    pub prefs: Box<pm_registry::PmPrefs>,
}

pub(crate) enum PmLifecycleCause {
    Deregister,
    Crash,
    Preserve,
}

/// Retained only by the closing runtime. An ambiguous durable-write error must
/// retry the captured registration change rather than count another crash.
pub(crate) enum PmLifecycleCommit {
    Pending {
        expected: Box<PmRegistration>,
        desired: Option<PmRegistration>,
        respawn: bool,
    },
    Complete(Option<PmLifecycleOutcome>),
}

impl PmLifecycleCommit {
    pub(crate) fn outcome(&self) -> Option<&PmLifecycleOutcome> {
        match self {
            Self::Complete(outcome) => outcome.as_ref(),
            Self::Pending { .. } => None,
        }
    }
}

pub(crate) fn finish_pm_lifecycle(
    root: &Path,
    session_id: &str,
    cause: PmLifecycleCause,
    completion: &mut Option<PmLifecycleCommit>,
) -> Result<(), String> {
    let prefs_path = pm_registry::pm_prefs_path_for_repo_path(root);
    if completion.is_none() {
        if matches!(cause, PmLifecycleCause::Preserve) {
            *completion = Some(PmLifecycleCommit::Complete(None));
            return Ok(());
        }
        let prefs = pm_registry::load_pm_prefs(&prefs_path).map_err(|error| error.to_string())?;
        let Some(expected) = prefs.registration.filter(|pm| pm.session_id == session_id) else {
            *completion = Some(PmLifecycleCommit::Complete(None));
            return Ok(());
        };
        let (desired, respawn) = match cause {
            PmLifecycleCause::Deregister => (None, false),
            PmLifecycleCause::Crash => {
                let mut desired = expected.clone();
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let respawn = pm_registry::apply_pm_crash_backoff(&mut desired, &now);
                (Some(desired), respawn)
            }
            PmLifecycleCause::Preserve => unreachable!(),
        };
        *completion = Some(PmLifecycleCommit::Pending {
            expected: Box::new(expected),
            desired,
            respawn,
        });
    }
    let Some(PmLifecycleCommit::Pending {
        expected,
        desired,
        respawn,
    }) = completion.as_ref()
    else {
        return Ok(());
    };
    let deregistered = desired.is_none();
    let respawn = *respawn;
    let (mut prefs, matched) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        if prefs.registration == *desired {
            return true;
        }
        if prefs.registration.as_ref() != Some(expected.as_ref()) {
            return false;
        }
        prefs.registration.clone_from(desired);
        true
    })
    .map_err(|error| error.to_string())?;
    if !matched {
        *completion = Some(PmLifecycleCommit::Complete(None));
        return Ok(());
    }
    if deregistered {
        // Preserve a replacement registration and every uncertain/local-work
        // case. The existing cleanup owns canonical-path, refresh and trust guards.
        match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(current) if current.registration.is_none() => {
                let worktree = pm_registry::pm_worktree_path_for_repo_path(root);
                match pm_registry::cleanup_pm_worktree_for_repo_path(root, |worktree, entry| {
                    crate::launch_runtime::intake_hook_config_is_disposable(worktree, entry)
                }) {
                    Ok(outcome) => {
                        tracing::info!(?outcome, worktree = %worktree.display(), "closed PM worktree cleanup completed")
                    }
                    Err(error) => {
                        tracing::warn!(%error, worktree = %worktree.display(), "PM worktree cleanup failed closed; keeping it")
                    }
                }
                prefs = pm_registry::load_pm_prefs(&prefs_path).unwrap_or(current);
            }
            Ok(current) => prefs = current,
            Err(error) => {
                tracing::warn!(%error, "PM registration became unreadable; retaining the closed PM worktree")
            }
        }
    }
    *completion = Some(PmLifecycleCommit::Complete(Some(PmLifecycleOutcome {
        session_id: session_id.into(),
        deregistered,
        respawn,
        prefs: Box::new(prefs),
    })));
    Ok(())
}

/// The former GUI idle-delivery permit now belongs to the physical owner.
/// Keep an early cancellation even when its HTTP request overtakes delivery.
#[derive(Default)]
pub(crate) struct PmWakePermits(HashMap<String, PmWakePermit>);
struct PmWakePermit {
    id: String,
    incarnation: u64,
    permitted: Arc<AtomicBool>,
    started: bool,
    mobile: Option<Arc<MobileInputTransaction>>,
}
impl PmWakePermits {
    pub(crate) fn finish(&mut self, operation: &str) {
        if let Some(permit) = self.0.get_mut(operation) {
            permit.started = false;
        }
    }

    pub(crate) fn cancel(
        &mut self,
        operation: &str,
        id: &str,
        incarnation: u64,
    ) -> Result<(), String> {
        let entry = self
            .0
            .entry(operation.into())
            .or_insert_with(|| PmWakePermit {
                id: id.into(),
                incarnation,
                permitted: Arc::new(AtomicBool::new(false)),
                started: false,
                mobile: None,
            });
        if entry.id != id || entry.incarnation != incarnation || entry.mobile.is_some() {
            return Err("PM wake delivery identity changed".into());
        }
        entry.permitted.store(false, Ordering::Release);
        Ok(())
    }

    pub(crate) fn start(
        &mut self,
        operation: &str,
        id: &str,
        incarnation: u64,
    ) -> Result<Arc<AtomicBool>, String> {
        if operation.trim().is_empty() {
            return Err("PM wake delivery id is empty".into());
        }
        let entry = self
            .0
            .entry(operation.into())
            .or_insert_with(|| PmWakePermit {
                id: id.into(),
                incarnation,
                permitted: Arc::new(AtomicBool::new(true)),
                started: false,
                mobile: None,
            });
        if entry.id != id
            || entry.incarnation != incarnation
            || entry.started
            || entry.mobile.is_some()
        {
            return Err("PM wake delivery is stale or already started".into());
        }
        entry.started = true;
        Ok(entry.permitted.clone())
    }
}

use super::events::{
    OwnerMobileInputIdentity, OwnerMobileInputOutcome, OwnerMobileInputPhase, OwnerMobileInputReady,
};

const MOBILE_INPUT_PREPARE_LIFETIME: Duration = Duration::from_secs(30);
const MAX_UNSETTLED_MOBILE_INPUTS: usize = 32;

#[derive(Default)]
struct MobileInputReplies {
    ready: Option<tokio::sync::oneshot::Sender<super::events::OwnerReply>>,
    finished: Option<tokio::sync::oneshot::Sender<super::events::OwnerReply>>,
}

/// One private transaction in the existing owner permit map. Its nonce proves
/// identity only; only the Host may authorize a Commit while holding its fences.
pub(crate) struct MobileInputTransaction {
    ready: OwnerMobileInputReady,
    deadline: Instant,
    permitted: Arc<AtomicBool>,
    outcome: Mutex<OwnerMobileInputOutcome>,
    replies: Mutex<MobileInputReplies>,
    changed: Condvar,
    #[cfg(any(test, feature = "test-gh-guard"))]
    drop_commit_reply: AtomicBool,
    #[cfg(any(test, feature = "test-gh-guard"))]
    commit_reply_lost: Arc<AtomicBool>,
}

impl MobileInputTransaction {
    fn new(identity: OwnerMobileInputIdentity, permitted: Arc<AtomicBool>) -> Self {
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let now = gwt_core::operation_deadline::now();
        let expires_at_mono_ms = u64::try_from(
            now.duration_since(*EPOCH.get_or_init(|| now)).as_millis()
                + MOBILE_INPUT_PREPARE_LIFETIME.as_millis(),
        )
        .expect("native monotonic input deadline fits u64");
        let ready = OwnerMobileInputReady {
            identity,
            nonce: uuid::Uuid::new_v4().to_string(),
            expires_at_mono_ms,
        };
        Self {
            outcome: Mutex::new(OwnerMobileInputOutcome {
                ready: ready.clone(),
                phase: OwnerMobileInputPhase::Preparing,
                observation: None,
                error: None,
            }),
            ready,
            replies: Mutex::default(),
            deadline: now + MOBILE_INPUT_PREPARE_LIFETIME,
            permitted,
            changed: Condvar::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            drop_commit_reply: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-gh-guard"))]
            commit_reply_lost: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn outcome(&self) -> Result<OwnerMobileInputOutcome, String> {
        self.outcome
            .lock()
            .map(|outcome| outcome.clone())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn reply_when_observed(
        &self,
        reply: tokio::sync::oneshot::Sender<super::events::OwnerReply>,
        preparing: bool,
    ) {
        let outcome = match self.outcome.lock() {
            Ok(outcome) => outcome,
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
                return;
            }
        };
        let wait = if preparing {
            outcome.phase == OwnerMobileInputPhase::Preparing
        } else {
            outcome.phase == OwnerMobileInputPhase::Committing
        };
        if !wait {
            Self::send_outcome(reply, &outcome);
            return;
        }
        match self.replies.lock() {
            Ok(mut replies) => {
                let slot = if preparing {
                    &mut replies.ready
                } else {
                    &mut replies.finished
                };
                if let Some(previous) = slot.replace(reply) {
                    // An overtaken HTTP observation gets its exact pending
                    // phase, never a duplicate effect or a false finished ACK.
                    Self::send_outcome(previous, &outcome);
                }
            }
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
            }
        }
    }

    fn send_outcome(
        reply: tokio::sync::oneshot::Sender<super::events::OwnerReply>,
        outcome: &OwnerMobileInputOutcome,
    ) {
        let _ = reply.send(serde_json::to_value(outcome).map_err(|error| error.to_string()));
    }

    pub(crate) fn matches(&self, ready: &OwnerMobileInputReady) -> bool {
        &self.ready == ready
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) fn suppress_commit_reply(
        &self,
        ready: &OwnerMobileInputReady,
    ) -> Result<Arc<AtomicBool>, String> {
        if !self.matches(ready) || self.outcome()?.phase != OwnerMobileInputPhase::Ready {
            return Err("test receipt boundary is not this exact Ready input".into());
        }
        self.drop_commit_reply.store(true, Ordering::Release);
        Ok(self.commit_reply_lost.clone())
    }

    pub(crate) fn can_write(&self) -> bool {
        self.permitted.load(Ordering::Acquire)
            && gwt_core::operation_deadline::now() < self.deadline
    }

    pub(crate) fn commit(&self, ready: &OwnerMobileInputReady) -> Result<(), String> {
        let mut outcome = self.outcome.lock().map_err(|error| error.to_string())?;
        if !self.matches(ready) {
            return Err("native input readiness identity changed".into());
        }
        if outcome.phase == OwnerMobileInputPhase::Ready
            && self.permitted.load(Ordering::Acquire)
            && gwt_core::operation_deadline::now() < self.deadline
        {
            outcome.phase = OwnerMobileInputPhase::Committing;
            self.changed.notify_all();
            return Ok(());
        }
        // Duplicate Commit observes the existing outcome; it never writes again.
        if outcome.phase == OwnerMobileInputPhase::Committing || outcome.phase.is_finished() {
            return Ok(());
        }
        Err("native input is not ready or has expired".into())
    }

    fn cancel(&self) {
        self.permitted.store(false, Ordering::Release);
        self.changed.notify_all();
    }

    /// Called with the actual physical writer held, without input_state locks.
    pub(crate) fn wait_for_commit(&self) -> Result<(), String> {
        let mut outcome = self.outcome.lock().map_err(|error| error.to_string())?;
        if !self.permitted.load(Ordering::Acquire)
            || gwt_core::operation_deadline::now() >= self.deadline
        {
            return Err("native prepared input was cancelled or expired".into());
        }
        outcome.phase = OwnerMobileInputPhase::Ready;
        if let Ok(mut replies) = self.replies.lock() {
            if let Some(reply) = replies.ready.take() {
                Self::send_outcome(reply, &outcome);
            }
        }
        self.changed.notify_all();
        loop {
            if !self.permitted.load(Ordering::Acquire)
                || gwt_core::operation_deadline::now() >= self.deadline
            {
                return Err("native prepared input was cancelled or expired".into());
            }
            if outcome.phase == OwnerMobileInputPhase::Committing {
                return Ok(());
            }
            let remaining = self
                .deadline
                .saturating_duration_since(gwt_core::operation_deadline::now());
            outcome = self
                .changed
                .wait_timeout(outcome, remaining)
                .map_err(|error| error.to_string())?
                .0;
        }
    }

    /// A finished result is published only after the reservation/writer was
    /// released. Cancel responses before this point are observations, not ACKs.
    pub(crate) fn finish(
        &self,
        attempted: bool,
        result: Result<(), String>,
        observation: Option<super::events::OwnerInputObservation>,
    ) {
        if let Ok(mut outcome) = self.outcome.lock() {
            outcome.phase = match (attempted, result.is_ok()) {
                (_, true) => OwnerMobileInputPhase::Written,
                (true, false) => OwnerMobileInputPhase::WriteFailed,
                (false, false) => OwnerMobileInputPhase::Prevented,
            };
            outcome.observation = observation;
            outcome.error = result.err();
            if let Ok(mut replies) = self.replies.lock() {
                if let Some(reply) = replies.ready.take() {
                    Self::send_outcome(reply, &outcome);
                }
                if let Some(reply) = replies.finished.take() {
                    #[cfg(any(test, feature = "test-gh-guard"))]
                    if self.drop_commit_reply.load(Ordering::Acquire) {
                        drop(reply);
                        self.commit_reply_lost.store(true, Ordering::Release);
                        self.changed.notify_all();
                        return;
                    }
                    Self::send_outcome(reply, &outcome);
                }
            }
            self.changed.notify_all();
        }
    }
}

impl PmWakePermits {
    pub(crate) fn prepare_mobile(
        &mut self,
        identity: &OwnerMobileInputIdentity,
    ) -> Result<(Arc<MobileInputTransaction>, bool), String> {
        if identity.operation_id.trim().is_empty() {
            return Err("native input operation id is empty".into());
        }
        if let Some(entry) = self.0.get(&identity.operation_id) {
            let mobile = entry
                .mobile
                .as_ref()
                .ok_or("input operation kind changed")?;
            if &mobile.ready.identity != identity {
                return Err("native input operation identity changed".into());
            }
            return Ok((mobile.clone(), false));
        }
        if self
            .0
            .values()
            .filter(|entry| entry.mobile.is_some())
            .count()
            >= MAX_UNSETTLED_MOBILE_INPUTS
        {
            return Err("native prepared input capacity reached".into());
        }
        let permitted = Arc::new(AtomicBool::new(true));
        let mobile = Arc::new(MobileInputTransaction::new(
            identity.clone(),
            permitted.clone(),
        ));
        self.0.insert(
            identity.operation_id.clone(),
            PmWakePermit {
                id: identity.id.clone(),
                incarnation: identity.incarnation,
                permitted,
                started: true,
                mobile: Some(mobile.clone()),
            },
        );
        Ok((mobile, true))
    }

    pub(crate) fn mobile(
        &self,
        identity: &OwnerMobileInputIdentity,
    ) -> Result<Arc<MobileInputTransaction>, String> {
        let mobile = self
            .0
            .get(&identity.operation_id)
            .and_then(|entry| entry.mobile.as_ref())
            .ok_or("native input operation is absent")?;
        if &mobile.ready.identity != identity {
            return Err("native input operation identity changed".into());
        }
        Ok(mobile.clone())
    }

    pub(crate) fn cancel_mobile(
        &mut self,
        identity: &OwnerMobileInputIdentity,
    ) -> Result<OwnerMobileInputOutcome, String> {
        let (mobile, created) = self.prepare_mobile(identity)?;
        mobile.cancel();
        if created {
            mobile.finish(
                false,
                Err("cancelled before native preparation".into()),
                None,
            );
        }
        mobile.outcome()
    }

    pub(crate) fn settle_mobile(&mut self, ready: &OwnerMobileInputReady) -> Result<(), String> {
        if !self.0.contains_key(&ready.identity.operation_id) {
            return Ok(());
        }
        let mobile = self.mobile(&ready.identity)?;
        if !mobile.matches(ready) || !mobile.outcome()?.phase.is_finished() {
            return Err("native input is not physically settled".into());
        }
        self.0.remove(&ready.identity.operation_id);
        Ok(())
    }

    pub(crate) fn cancel_mobile_for_pane(&self, id: Option<&str>) {
        for mobile in self.0.values().filter_map(|entry| entry.mobile.as_ref()) {
            if id.is_none_or(|id| mobile.ready.identity.id == id) {
                mobile.cancel();
            }
        }
    }
}

/// A current unfinished turn is busy even when its output and tool set are quiet.
pub fn pm_wake_runtime_is_busy(path: &Path, incarnation: u64) -> bool {
    let Ok(state) = gwt_agent::SessionRuntimeState::load(path) else {
        return false;
    };
    if state.runtime_incarnation.is_none() {
        return false;
    }
    if state.runtime_incarnation != Some(incarnation) {
        return true;
    }
    state.status != gwt_agent::AgentStatus::Idle
        || (state.source_event.as_deref() == Some("Stop")
            && state
                .last_completed_hook_event_at
                .is_none_or(|at| at < state.updated_at))
}

pub(crate) fn release_acknowledged_wake(
    root: &Path,
    id: &str,
    session: &str,
    entry: &super::agent_dispatch::PtyWriterEntry,
    operation: Option<&str>,
) -> Result<bool, String> {
    if !entry.current.load(Ordering::Acquire) {
        return Ok(false);
    }
    let operation_id = entry
        .pm_delivery_replay
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .filter(|guard| {
            guard.retained_input.is_some()
                && guard.target_session_id == session
                && operation.is_none_or(|operation| guard.operation_id == operation)
        })
        .map(|guard| guard.operation_id.clone());
    let Some(operation_id) = operation_id else {
        return Ok(false);
    };
    let verified = pm_registry::pm_delivery_receipt_for_operation(
        &pm_registry::pm_delivery_receipts_path_for_repo_path(root),
        &operation_id,
    )
    .map_err(|error| error.to_string())?
    .is_some_and(|receipt| {
        receipt.target_session_id == session
            && receipt.target_window_id == id
            && receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
    });
    if !verified {
        return Ok(false);
    }
    let released = {
        let mut guard = entry
            .pm_delivery_replay
            .lock()
            .map_err(|error| error.to_string())?;
        if guard.as_ref().is_some_and(|guard| {
            guard.operation_id == operation_id && guard.target_session_id == session
        }) {
            guard.take()
        } else {
            None
        }
    };
    // Reservation drop can acquire the physical writer; release outside the mutex.
    drop(released);
    Ok(true)
}

/// Resolve current escalation subjects and commit the protected prompt in the
/// same project owner that holds its exact physical writer.
pub(crate) fn deliver_pm_wake(
    root: &Path,
    id: &str,
    entry: &Arc<super::agent_dispatch::PtyWriterEntry>,
    writers: &super::agent_dispatch::PtyWriterRegistry,
    delivery: &PmWakeInput,
    permit: &Arc<AtomicBool>,
    runtime: (
        &Path,
        Option<&str>,
        u64,
        &std::sync::Mutex<gwt_terminal::Pane>,
        u64,
    ),
) -> Result<PmWakeWrite, String> {
    use super::agent_dispatch::{
        await_pm_delivery_status, drive_verified_pane_submit, PmDeliveryReplayGuard,
        PM_DELIVERY_TERMINAL_MARGIN, PM_DELIVERY_UNCONFIRMED_REASON,
    };
    let (sessions_dir, target_session, incarnation, pane, output_seq) = runtime;
    let operation_deadline = Instant::now() + Duration::from_secs(2);
    let _operation_budget = (!delivery.idle)
        .then(|| gwt_core::operation_deadline::ScopedOperationDeadline::enter(operation_deadline));
    let deadline = operation_deadline - PM_DELIVERY_TERMINAL_MARGIN;
    if let Some(session) = target_session {
        if release_acknowledged_wake(root, id, session, entry, Some(&delivery.operation_id))? {
            return Ok(PmWakeWrite::Injected);
        }
        if pm_registry::pm_delivery_receipt_for_operation(
            &pm_registry::pm_delivery_receipts_path_for_repo_path(root),
            &delivery.operation_id,
        )
        .map_err(|error| error.to_string())?
        .is_some_and(|receipt| {
            receipt.target_session_id == session
                && receipt.target_window_id == id
                && receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
        }) {
            return Ok(PmWakeWrite::Injected);
        }
    }
    let owns_input = entry
        .pm_delivery_replay
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .is_some_and(|guard| {
            guard.operation_id == delivery.operation_id && guard.retained_input.is_some()
        });
    if !owns_input {
        if let Some(input) = &delivery.periodic_input {
            let prefs = pm_registry::load_pm_prefs(&pm_registry::pm_prefs_path_for_repo_path(root))
                .map_err(|error| error.to_string())?;
            if !input.matches(
                target_session,
                incarnation,
                &entry.handle,
                prefs.settings.loop_interval_secs_clamped(),
            ) {
                return Ok(PmWakeWrite::Discarded);
            }
        }
    }
    pm_registry::with_active_pm_loop(&pm_registry::pm_prefs_path_for_repo_path(root), || {
        let loop_state =
            pm_registry::load_pm_loop_state(&pm_registry::pm_loop_state_path_for_repo_path(root))
                .map_err(|error| error.to_string())?;
        if loop_state.last_wake_id != delivery.wake_id && !owns_input {
            return Ok(PmWakeWrite::Discarded);
        }
        if !permit.load(Ordering::Acquire) || (entry.handle.has_unsent_user_input() && !owns_input)
        {
            return Ok(
                if delivery.periodic_input.is_some()
                    && !owns_input
                    && entry.handle.has_unsent_user_input()
                {
                    PmWakeWrite::Discarded
                } else {
                    PmWakeWrite::Deferred
                },
            );
        }
        if !delivery.idle {
            let Some(target_session) = target_session else {
                return Ok(PmWakeWrite::Discarded);
            };
            let operation_id = delivery
                .wake_id
                .as_deref()
                .ok_or("PM wake has no operation id")?;
            if operation_id != delivery.operation_id {
                return Err("PM wake operation id does not match its wake".into());
            }
            let runtime_path = gwt_agent::runtime_state_path(sessions_dir, target_session);
            let busy = || pm_wake_runtime_is_busy(&runtime_path, incarnation);
            let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(root);
            let existing =
                pm_registry::pm_delivery_receipt_for_operation(&receipt_path, operation_id)
                    .map_err(|error| error.to_string())?;
            if existing.as_ref().is_some_and(|receipt| {
                receipt.target_session_id == target_session
                    && receipt.target_window_id == id
                    && receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
            }) {
                return Ok(PmWakeWrite::Injected);
            }
            if busy() {
                return Ok(PmWakeWrite::Deferred);
            }
            let mut replay = if existing.is_some() {
                let mut guard = entry
                    .pm_delivery_replay
                    .lock()
                    .map_err(|error| error.to_string())?;
                if guard.as_ref().is_some_and(|guard| {
                    guard.operation_id == operation_id
                        && guard
                            .retained_input
                            .as_ref()
                            .is_some_and(|(_, complete)| !complete)
                }) {
                    return Ok(PmWakeWrite::Unconfirmed);
                }
                if guard.as_ref().is_some_and(|guard| {
                    guard.operation_id == operation_id && guard.target_session_id == target_session
                }) {
                    guard.take()
                } else {
                    None
                }
            } else {
                None
            };
            if existing.is_some() && replay.is_none() {
                return Ok(
                    if existing.as_ref().is_some_and(|receipt| {
                        matches!(
                            receipt.status,
                            pm_registry::PmDeliveryReceiptStatus::Refused
                                | pm_registry::PmDeliveryReceiptStatus::Lost
                        )
                    }) {
                        PmWakeWrite::Discarded
                    } else {
                        PmWakeWrite::Unconfirmed
                    },
                );
            }
            let ordinary_revision = replay
                .as_ref()
                .map(|guard| guard.ordinary_revision)
                .or_else(|| {
                    delivery
                        .periodic_input
                        .as_ref()
                        .and_then(|input| input.revision)
                })
                .or_else(|| entry.handle.ordinary_input_revision())
                .ok_or("PM wake input revision unavailable")?;
            let retained = replay
                .as_mut()
                .and_then(|guard| guard.retained_input.take());
            let continuing = retained.is_some();
            if !continuing
                && (entry.handle.has_unsent_user_input()
                    || entry.handle.ordinary_input_revision() != Some(ordinary_revision))
            {
                return Ok(PmWakeWrite::Discarded);
            }
            let reservation = match retained {
                Some((reservation, _)) => reservation,
                None => Arc::clone(&entry.handle)
                    .reserve_input_transaction()
                    .map_err(|error| error.to_string())?,
            };
            if !continuing
                && replay.as_ref().is_some_and(|guard| {
                    reservation.previous_transaction_revision() != guard.transaction_revision
                })
            {
                return Ok(PmWakeWrite::Discarded);
            }
            let (prompt, body_sha256) = if let Some(receipt) = existing {
                if receipt.target_session_id != target_session || receipt.target_window_id != id {
                    return Ok(PmWakeWrite::Discarded);
                }
                ("\r".to_string(), receipt.body_sha256)
            } else {
                if pane.lock().map_err(|error| error.to_string())?.output_seq() != output_seq {
                    return Ok(PmWakeWrite::Deferred);
                }
                let prompt = pm_wake_delivery_prompt(root, &delivery.prompt, false);
                if !permit.load(Ordering::Acquire) || entry.handle.has_unsent_user_input() {
                    return Ok(PmWakeWrite::Deferred);
                }
                let body = prompt.trim_end_matches('\r');
                let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
                let prepared = pm_registry::PmDeliveryReceipt {
                    operation_id: operation_id.to_string(),
                    recorded_at: chrono::Utc::now().to_rfc3339(),
                    status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                    principal_session_id: format!("gwt-wake:{operation_id}"),
                    target_window_id: id.to_string(),
                    target_session_id: target_session.to_string(),
                    body_sha256: body_sha256.clone(),
                    reason: None,
                };
                if !matches!(
                    pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared)
                        .map_err(|error| error.to_string())?,
                    pm_registry::PmDeliveryPrepareOutcome::Prepared
                ) {
                    return Ok(PmWakeWrite::Unconfirmed);
                }
                (
                    format!("{body} [gwt-delivery:{operation_id}:{body_sha256}]\r"),
                    body_sha256,
                )
            };
            let mut body_written = false;
            let mut body_attempted = false;
            let result = drive_verified_pane_submit(
                &prompt,
                reservation.bracketed_paste_enabled(),
                2,
                |bytes| {
                    if busy() || !permit.load(Ordering::Acquire) {
                        return Err("PM wake held by newer turn activity".into());
                    }
                    let current = writers.read().map_err(|error| error.to_string())?;
                    if !current
                        .get(id)
                        .is_some_and(|writer| Arc::ptr_eq(writer, entry))
                    {
                        return Err("PM wake target generation changed".into());
                    }
                    reservation
                        .write_input_authorized(bytes, |commit| {
                            let fail = |details: &str| gwt_terminal::TerminalError::PtyIoError {
                                details: details.into(),
                            };
                            if Instant::now() >= deadline {
                                return Err(fail(
                                    "PM wake deadline expired before physical commit",
                                ));
                            }
                            let current_prefs = pm_registry::load_pm_prefs(
                                &pm_registry::pm_prefs_path_for_repo_path(root),
                            )
                            .map_err(|error| fail(&error.to_string()))?;
                            let current_pm = current_prefs.registration;
                            if busy() || !permit.load(Ordering::Acquire) {
                                return Err(fail(
                                    "PM wake held by newer activity at physical commit",
                                ));
                            }
                            if Instant::now() >= deadline
                                || current_pm.as_ref().is_none_or(|registration| {
                                    registration.session_id != target_session
                                })
                            {
                                return Err(fail("registered PM changed before physical commit"));
                            }
                            let mut mark_attempted = || body_attempted |= bytes != b"\r";
                            if body_written || continuing {
                                commit(&mut mark_attempted)
                            } else {
                                let quiet_for = if delivery.periodic_input.is_some() {
                                    Duration::from_secs(
                                        current_prefs.settings.loop_interval_secs_clamped(),
                                    )
                                } else {
                                    Duration::ZERO
                                };
                                reservation.with_quiet_ordinary_input(
                                    ordinary_revision,
                                    quiet_for,
                                    || commit(&mut mark_attempted),
                                )
                            }
                        })
                        .map_err(|error| error.to_string())?;
                    body_written |= bytes != b"\r";
                    Ok(())
                },
                |settle| {
                    thread::sleep(settle.min(deadline.saturating_duration_since(Instant::now())))
                },
                || {
                    pm_registry::pm_delivery_receipt_for_operation(&receipt_path, operation_id)
                        .map(|receipt| {
                            receipt.is_some_and(|receipt| {
                                receipt.target_session_id == target_session
                                    && receipt.body_sha256 == body_sha256
                                    && receipt.status
                                        == pm_registry::PmDeliveryReceiptStatus::Verified
                            })
                        })
                        .map_err(|error| error.to_string())
                },
            );
            let status = await_pm_delivery_status(&receipt_path, operation_id, deadline);
            if status
                .as_ref()
                .is_ok_and(|status| *status == pm_registry::PmDeliveryReceiptStatus::Verified)
            {
                return Ok(PmWakeWrite::Injected);
            }
            if body_attempted || continuing {
                let guard = PmDeliveryReplayGuard {
                    operation_id: operation_id.to_string(),
                    target_session_id: target_session.to_string(),
                    ordinary_revision,
                    transaction_revision: reservation.transaction_revision(),
                    retained_input: Some((reservation, body_written || continuing)),
                };
                let previous = entry
                    .pm_delivery_replay
                    .lock()
                    .map_err(|error| error.to_string())?
                    .replace(guard);
                drop(previous);
                // Close the ACK/guard-publication race: an exact hook that saw
                // no retained guard has already committed its durable receipt.
                if release_acknowledged_wake(root, id, target_session, entry, Some(operation_id))? {
                    return Ok(PmWakeWrite::Injected);
                }
            }
            status?;
            let reason = result
                .err()
                .unwrap_or_else(|| PM_DELIVERY_UNCONFIRMED_REASON.to_string());
            let reason = if body_attempted && !body_written {
                format!("PM wake body write was incomplete; recovery is readback-only: {reason}")
            } else {
                reason
            };
            pm_registry::finish_pm_delivery_receipt(
                &receipt_path,
                operation_id,
                target_session,
                &body_sha256,
                if body_attempted || replay.is_some() {
                    pm_registry::PmDeliveryReceiptStatus::Unconfirmed
                } else {
                    pm_registry::PmDeliveryReceiptStatus::Refused
                },
                Some(&reason),
            )
            .map_err(|error| error.to_string())?;
            tracing::warn!(operation_id, %reason, "PM wake has no exact turn acknowledgement");
            return Ok(if body_attempted || replay.is_some() {
                PmWakeWrite::Unconfirmed
            } else {
                PmWakeWrite::Discarded
            });
        }
        let prompt = pm_wake_delivery_prompt(root, &delivery.prompt, true);
        if !permit.load(Ordering::Acquire)
            || (entry.handle.has_unsent_user_input() && !owns_input)
            || target_session.is_some_and(|session| {
                pm_wake_runtime_is_busy(
                    &gwt_agent::runtime_state_path(sessions_dir, session),
                    incarnation,
                )
            })
        {
            return Ok(PmWakeWrite::Deferred);
        }
        // Reuse the canonical paste/reservation/submit implementation and its
        // actual-commit current-generation fence; no GUI physical writer exists.
        super::agent_dispatch::write_current_pane_input(writers, id, entry, &prompt)?;
        if delivery.idle {
            let loop_path = pm_registry::pm_loop_state_path_for_repo_path(root);
            let current =
                pm_registry::load_pm_loop_state(&loop_path).map_err(|error| error.to_string())?;
            if current.last_wake_id == delivery.wake_id {
                pm_registry::save_pm_loop_state(
                    &loop_path,
                    &pm_registry::PmLoopState {
                        last_wake_at: Some(
                            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        ),
                        last_wake_id: current.last_wake_id,
                        last_user_prompt_at: current.last_user_prompt_at,
                        ..Default::default()
                    },
                )
                .map_err(|error| error.to_string())?;
            }
        }
        Ok(PmWakeWrite::Injected)
    })
    .map_err(|error| error.to_string())?
    .unwrap_or(Ok(PmWakeWrite::Deferred))
}

pub fn pm_wake_delivery_prompt(root: &Path, prompt: &str, idle: bool) -> String {
    let mut prompt = format!(
        "{}{}\r",
        prompt.trim_end_matches('\r'),
        open_escalation_prompt_section(root)
    );
    if idle && prompt.len() > 960 {
        let mut end = 959;
        while !prompt.is_char_boundary(end) {
            end -= 1;
        }
        prompt.truncate(end);
        prompt.push('\r');
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    record_test_wake_prompt(root, &prompt);
    prompt
}

#[cfg(any(test, feature = "test-gh-guard"))]
type PromptCaptureMap =
    std::sync::Mutex<HashMap<std::path::PathBuf, std::sync::Weak<std::sync::Mutex<Vec<String>>>>>;
#[cfg(any(test, feature = "test-gh-guard"))]
static PROMPT_CAPTURES: std::sync::OnceLock<PromptCaptureMap> = std::sync::OnceLock::new();

/// Read-only fixture observation of the production owner's resolved prompt.
#[cfg(any(test, feature = "test-gh-guard"))]
#[doc(hidden)]
pub struct PmWakePromptCapture {
    root: std::path::PathBuf,
    prompts: Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(any(test, feature = "test-gh-guard"))]
impl PmWakePromptCapture {
    pub fn begin(root: &Path) -> Self {
        let root = gwt_core::paths::normalize_windows_child_process_path(root);
        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        PROMPT_CAPTURES
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(root.clone(), Arc::downgrade(&prompts));
        Self { root, prompts }
    }
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
impl Drop for PmWakePromptCapture {
    fn drop(&mut self) {
        if let Some(captures) = PROMPT_CAPTURES.get() {
            captures.lock().unwrap().remove(&self.root);
        }
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
fn record_test_wake_prompt(root: &Path, prompt: &str) {
    let root = gwt_core::paths::normalize_windows_child_process_path(root);
    let capture = PROMPT_CAPTURES.get().and_then(|captures| {
        captures
            .lock()
            .unwrap()
            .get(&root)
            .and_then(std::sync::Weak::upgrade)
    });
    if let Some(prompts) = capture {
        prompts.lock().unwrap().push(prompt.into());
    }
}

const ESCALATION_PROMPT_BODY_CHARS: usize = 220;
const ESCALATION_PROMPT_MAX_ROWS: usize = 5;

fn current_open_escalations(
    project_root: &Path,
) -> Vec<gwt_core::board_escalation::BoardEscalation> {
    let Ok(store) = gwt_core::coordination::load_escalation_store(project_root) else {
        return Vec::new();
    };
    store
        .open_escalations()
        .into_iter()
        .filter(|escalation| {
            let Some(session_id) = escalation.origin_session_id.as_deref() else {
                return true;
            };
            let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
            let Ok(session) = gwt_agent::Session::load(&path) else {
                return true;
            };
            if session.id != session_id {
                return true;
            }
            if matches!(
                session.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            ) {
                return false;
            }
            let Some(owner) = session.linked_issue_number else {
                return true;
            };
            if !escalation.owner_issue_numbers().contains(&owner) {
                return true;
            }
            !crate::cli::execution_state::load(&session.worktree_path)
                .ok()
                .flatten()
                .is_some_and(|record| {
                    crate::cli::execution_state::integrity_ok(&record)
                        && record.owner_number == owner
                        && record.primary_session_id != session_id
                })
        })
        .collect()
}

pub fn open_escalation_prompt_section(project_root: &Path) -> String {
    let open = current_open_escalations(project_root);
    if open.is_empty() {
        return String::new();
    }
    let shown = open.len().min(ESCALATION_PROMPT_MAX_ROWS);
    let lines = gwt_core::board_escalation::render_open_escalation_lines(
        &open[..shown],
        ESCALATION_PROMPT_BODY_CHARS,
    );
    let overflow = if open.len() > shown {
        format!(" (+{} more in issue.monitor.status)", open.len() - shown)
    } else {
        String::new()
    };
    format!("{}{count}){overflow} — decide fresh launch / spec ruling / tool fix for each, then post the resolution with params.resolves: {lines}",
        pm_registry::PM_ESCALATION_PROMPT_PREFIX, count = open.len(), lines = lines.join(" || "))
}

pub fn has_open_board_escalations(project_root: &Path) -> bool {
    !current_open_escalations(project_root).is_empty()
}

pub fn pm_status_from_launch_prefs(
    sessions_dir: &Path,
    prefs: &pm_registry::PmPrefs,
    running: Option<&PmRegistration>,
) -> BackendEvent {
    let configured = prefs.settings.launch_profile_or_default();
    let loop_interval_secs = prefs.settings.loop_interval_secs_clamped();
    let running_profile =
        running.map(|registration| pm_running_profile_at(sessions_dir, registration));
    let agent_options = pm_agent_options(&configured.agent_id);
    BackendEvent::PmStatus {
        available: true,
        auto_start: prefs.settings.auto_start,
        paused: prefs.settings.paused,
        loop_interval_secs,
        loop_interval_secs_decimal: loop_interval_secs.to_string(),
        agent_options,
        configured_agent_id: configured.agent_id,
        configured_model: configured.model,
        configured_reasoning: configured.reasoning,
        running_agent_id: running_profile
            .as_ref()
            .map(|profile| profile.agent_id.clone()),
        running_model: running_profile
            .as_ref()
            .and_then(|profile| profile.model.clone()),
        running_reasoning: running_profile
            .as_ref()
            .and_then(|profile| profile.reasoning.clone()),
        is_running: running_profile.is_some(),
        // #4486 AC-7: a live PM cannot be blocked, so the panel never has
        // to reconcile "running" with a stale block.
        start_block: running_profile
            .is_none()
            .then(|| prefs.start_block.clone())
            .flatten(),
    }
}

/// Resolve the launch identity of the conversation that is running now.
/// The registration deliberately stays small; the durable Session already
/// owns agent/model/reasoning. Legacy or temporarily unreadable Session
/// records retain the registered agent and expose unknown tuning.
fn pm_running_profile_at(sessions_dir: &Path, registration: &PmRegistration) -> PmLaunchProfile {
    let session_path = sessions_dir.join(format!("{}.toml", registration.session_id));
    gwt_agent::Session::load(&session_path)
        .map(|session| {
            PmLaunchProfile {
                agent_id: session.agent_id.command().to_string(),
                model: session.model,
                reasoning: session.reasoning_level,
                version: session.tool_version,
            }
            .normalized()
        })
        .unwrap_or_else(|_| PmLaunchProfile {
            agent_id: registration.agent_id.clone(),
            ..PmLaunchProfile::default()
        })
}

/// Selectable PM agents: the ones that can resolve `$gwt-pm`, narrowed to
/// what is actually installed.
///
/// `configured` is always offered even when it is not on PATH, so the
/// picker can still show (and keep) the project's current choice instead of
/// silently presenting a different agent as the configured one.
pub fn pm_agent_options(configured: &str) -> Vec<PmAgentOption> {
    pm_registry::PM_SUPPORTED_AGENTS
        .iter()
        .filter(|id| **id == configured || which::which(id).is_ok())
        .map(|id| PmAgentOption {
            id: (*id).to_string(),
            name: gwt_agent::builtin_agent_descriptor_for_command(id)
                .map_or_else(|| (*id).to_string(), |d| d.id.display_name().to_string()),
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::project_runtime::{events::OwnerCommand, test_support::ProjectOwnerHarness};

    #[test]
    fn physical_owner_preserves_early_idle_cancellation_and_composer_fence() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            gwt_core::process::resolved_command(gwt_core::process::ProcessPlanRequest::new("git"))
                .unwrap()
                .args(["init", "-q"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let owner = ProjectOwnerHarness::start(&root).await.unwrap();
            let id = owner.pane_id("pm-wake");
            let launched = owner
                .command(OwnerCommand::LaunchShell {
                    id: id.clone(),
                    command: "/bin/sh".into(),
                    args: vec![
                        "-c".into(),
                        "stty -echo -icanon -isig; printf 'PM_WAKE_READY\\n'; exec cat >/dev/null"
                            .into(),
                    ],
                    cols: 80,
                    rows: 24,
                })
                .await
                .unwrap();
            let incarnation = launched["incarnation"].as_u64().unwrap();
            let health = owner.command(OwnerCommand::Health).await.unwrap();
            let host_instance_id = health["host_instance_id"].as_str().unwrap().to_string();
            // test-hygiene: allow-wall-clock-deadline Readiness comes from a real child/PTY whose completion crosses an OS pipe.
            let deadline = std::time::Instant::now() + gwt_core::deadline_budget::HANG_GUARD;
            loop {
                let snapshot = owner
                    .command(OwnerCommand::Snapshot {
                        id: id.clone(),
                        incarnation,
                    })
                    .await
                    .unwrap();
                if snapshot["screen"]
                    .as_str()
                    .unwrap()
                    .contains("PM_WAKE_READY")
                {
                    break;
                }
                assert!(
                    // test-hygiene: allow-wall-clock-deadline the actual owner PTY readiness marker is an OS/IPC response; this guard only prevents a hung fixture
                    std::time::Instant::now() < deadline,
                    "real PM fixture did not become ready"
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            pm_registry::save_pm_loop_state(
                &pm_registry::pm_loop_state_path_for_repo_path(&root),
                &Default::default(),
            )
            .unwrap();
            owner
                .command(OwnerCommand::CancelPmWake {
                    id: id.clone(),
                    incarnation,
                    host_instance_id: host_instance_id.clone(),
                    operation_id: "early-cancel".into(),
                })
                .await
                .unwrap();
            let cancelled = owner
                .command(OwnerCommand::PmWakeInput {
                    id: id.clone(),
                    incarnation,
                    host_instance_id: host_instance_id.clone(),
                    delivery: PmWakeInput {
                        operation_id: "early-cancel".into(),
                        wake_id: None,
                        prompt: "CANCELLED\r".into(),
                        idle: true,
                        periodic_input: None,
                    },
                })
                .await
                .unwrap();
            assert_eq!(
                cancelled["status"], "deferred",
                "a missing wake_id must not erase the early cancellation fence"
            );
            let input = owner
                .command(OwnerCommand::Input {
                    id: id.clone(),
                    incarnation,
                    data: b"draft".to_vec(),
                })
                .await
                .unwrap();
            assert_eq!(input["has_unsent_user_input"], true);
            let busy = owner
                .command(OwnerCommand::PmWakeInput {
                    id: id.clone(),
                    incarnation,
                    host_instance_id: host_instance_id.clone(),
                    delivery: PmWakeInput {
                        operation_id: "busy".into(),
                        wake_id: None,
                        prompt: "BUSY\r".into(),
                        idle: false,
                        periodic_input: None,
                    },
                })
                .await
                .unwrap();
            assert_eq!(busy["status"], "deferred");
            owner
                .command(OwnerCommand::Input {
                    id: id.clone(),
                    incarnation,
                    data: vec![3],
                })
                .await
                .unwrap();
            let stale = owner
                .command(OwnerCommand::PmWakeInput {
                    id: id.clone(),
                    incarnation: incarnation + 1,
                    host_instance_id: host_instance_id.clone(),
                    delivery: PmWakeInput {
                        operation_id: "stale".into(),
                        wake_id: None,
                        prompt: "STALE\r".into(),
                        idle: false,
                        periodic_input: None,
                    },
                })
                .await;
            assert!(
                stale.is_err(),
                "a replacement incarnation cannot receive the captured wake"
            );
            let delivered = owner
                .command(OwnerCommand::PmWakeInput {
                    id: id.clone(),
                    incarnation,
                    host_instance_id,
                    delivery: PmWakeInput {
                        operation_id: "fresh".into(),
                        wake_id: None,
                        prompt: "FRESH\r".into(),
                        idle: true,
                        periodic_input: None,
                    },
                })
                .await
                .unwrap();
            assert_eq!(delivered["status"], "injected");
            assert!(
                pm_registry::load_pm_loop_state(&pm_registry::pm_loop_state_path_for_repo_path(
                    &root
                ))
                .unwrap()
                .last_wake_at
                .is_some(),
                "idle loop rearm commits at the actual owner before its reply"
            );
            owner
                .command(OwnerCommand::Close { id, incarnation })
                .await
                .unwrap();
            owner.shutdown().await;
        });
    }
}

#[derive(Clone)]
pub(crate) struct BoardDecisionNotice {
    pub entry_id: String,
    pub session_id: String,
}

pub(crate) fn board_notification_operation_id(entry_id: &str, session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"gwt-board-notification-v1");
    for value in [entry_id, session_id] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    // UUIDv8: this is an application-defined deterministic identity.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

pub(crate) fn deliver_board_notification(
    root: &Path,
    window_id: String,
    notification: BoardDecisionNotice,
    max_submit_attempts: usize,
    expected: Arc<super::agent_dispatch::PtyWriterEntry>,
    writers: super::agent_dispatch::PtyWriterRegistry,
    ordinary_revision: u64,
) -> Result<(), String> {
    use gwt_core::coordination::{BoardEntryKind, BoardMentionTargetKind};
    use std::time::{Duration, Instant};
    let saved = crate::board_provider::load_snapshot(root).map_err(|error| error.to_string())?;
    if !saved.board.entries.iter().any(|entry| {
        entry.id == notification.entry_id
            && entry.kind == BoardEntryKind::Decision
            && entry.mentions.iter().any(|mention| {
                mention.target_kind == BoardMentionTargetKind::Session
                    && mention.target == notification.session_id
            })
    }) {
        return Err("Board decision does not mention this Session".into());
    }
    let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(root);
    let deadline = Instant::now() + Duration::from_secs(2);
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
    let operation_id =
        board_notification_operation_id(&notification.entry_id, &notification.session_id);
    let short_id = notification
        .entry_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect::<String>();
    let body = format!(
        "[gwt Board notification — not an owner message] Board decision {short_id} mentions this session; run board.show before continuing."
    );
    // Hash the complete Board source label too. The shared exact
    // UserPromptSubmit ACK already accepts this protected marker.
    let body_sha256 = pm_registry::pm_delivery_prompt_sha256(&body);
    let prompt = format!("{body} [gwt-delivery:{operation_id}:{body_sha256}]\r");
    let prepared = pm_registry::PmDeliveryReceipt {
        operation_id: operation_id.clone(),
        recorded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        status: pm_registry::PmDeliveryReceiptStatus::Prepared,
        principal_session_id: format!("gwt-board:{}", notification.entry_id),
        target_window_id: window_id.clone(),
        target_session_id: notification.session_id.clone(),
        body_sha256: body_sha256.clone(),
        reason: None,
    };
    match pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared) {
        Ok(pm_registry::PmDeliveryPrepareOutcome::Prepared) => {}
        // Watcher/daemon duplicates and restarts never stage a second
        // body, including when a prior attempt was unconfirmed.
        Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(_)) => return Ok(()),
        Err(error) => {
            tracing::warn!(%error, operation_id, "Board notification receipt prepare failed");
            return Ok(());
        }
    }
    let mut attempted = false;
    let mut lost = false;
    let result = (|| {
        if !writers
            .read()
            .map_err(|error| error.to_string())?
            .get(&window_id)
            .is_some_and(|entry| {
                Arc::ptr_eq(entry, &expected) && entry.current.load(Ordering::Acquire)
            })
        {
            lost = true;
            return Err("Board notification target generation changed".to_string());
        }
        let reservation = Arc::clone(&expected.handle)
            .reserve_input_transaction()
            .map_err(|error| error.to_string())?;
        super::agent_dispatch::drive_verified_pane_submit(
            &prompt,
            reservation.bracketed_paste_enabled(),
            max_submit_attempts,
            |bytes| {
                reservation
                    .write_input_authorized(bytes, |commit| {
                        let current = writers.read().map_err(|error| {
                            gwt_terminal::TerminalError::PtyIoError {
                                details: error.to_string(),
                            }
                        })?;
                        if !current.get(&window_id).is_some_and(|entry| {
                            Arc::ptr_eq(entry, &expected) && entry.current.load(Ordering::Acquire)
                        }) {
                            lost = true;
                            return Err(gwt_terminal::TerminalError::PtyIoError {
                                details: "Board notification target generation changed".into(),
                            });
                        }
                        if Instant::now() >= deadline {
                            return Err(gwt_terminal::TerminalError::PtyIoError {
                                details: "Board notification delivery deadline expired".into(),
                            });
                        }
                        reservation.with_unchanged_ordinary_input(ordinary_revision, || {
                            commit(&mut || attempted = true)
                        })
                    })
                    .map_err(|error| error.to_string())
            },
            |settle| {
                std::thread::sleep(settle.min(deadline.saturating_duration_since(Instant::now())))
            },
            || {
                pm_registry::pm_delivery_receipt_for_operation(&receipt_path, &operation_id)
                    .map(|receipt| {
                        receipt.is_some_and(|receipt| {
                            receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
                        })
                    })
                    .map_err(|error| error.to_string())
            },
        )
    })();
    let (status, reason) = match result {
        Ok(super::agent_dispatch::VerifiedPaneSubmitOutcome::Verified { .. }) => {
            (pm_registry::PmDeliveryReceiptStatus::Verified, None)
        }
        Ok(super::agent_dispatch::VerifiedPaneSubmitOutcome::Unverified { .. }) => (
            pm_registry::PmDeliveryReceiptStatus::Unconfirmed,
            Some("Board notification has no exact Session acknowledgement".to_string()),
        ),
        Err(error) => (
            if lost {
                pm_registry::PmDeliveryReceiptStatus::Lost
            } else if attempted {
                pm_registry::PmDeliveryReceiptStatus::Unconfirmed
            } else {
                pm_registry::PmDeliveryReceiptStatus::Refused
            },
            Some(error),
        ),
    };
    if let Err(error) = pm_registry::finish_pm_delivery_receipt(
        &receipt_path,
        &operation_id,
        &notification.session_id,
        &body_sha256,
        status,
        reason.as_deref(),
    ) {
        tracing::warn!(%error, operation_id, "Board notification receipt finish failed");
    }
    Ok(())
}
