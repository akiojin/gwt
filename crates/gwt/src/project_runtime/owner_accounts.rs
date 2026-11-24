//! Account changes belong to the owner of the exact PTY incarnation.

use super::{CloseCause, OwnerCommand, OwnerEvent, OwnerState, WindowProcessStatus, WindowRuntime};
use crate::account_switch::PoolSwitchAttempt;
use gwt_agent::{AccountSwitchReason, Session, SessionLaunchOrigin};
use gwt_core::timestamp::UtcTimestamp;
use std::time::Instant;
use tokio::sync::oneshot;

pub(crate) struct InputCapture {
    pub id: String,
    pub incarnation: u64,
    pub bytes: Vec<u8>,
    pub typed: bool,
    pub started: Instant,
}

#[derive(Default)]
pub(super) struct AccountSwitchState {
    attempts: PoolSwitchAttempt,
    request: Option<SwitchRequest>,
    retired: Option<RetiredAccountPane>,
    replay: Option<String>,
    replay_incarnation: Option<u64>,
    input: SubmittedInput,
    turn_completed_at: Option<Instant>,
    input_writes: usize,
    next_check: Option<UtcTimestamp>,
    waiting_window: Option<crate::IssueMonitorWindowObservation>,
}

impl AccountSwitchState {
    fn record_input(&mut self, incarnation: u64, bytes: &[u8], typed: bool, started: Instant) {
        if self
            .turn_completed_at
            .is_some_and(|completed| started <= completed)
        {
            return;
        }
        if self.replay_incarnation == Some(incarnation) {
            self.replay = None;
            self.replay_incarnation = None;
        }
        if typed {
            if let Ok(text) = std::str::from_utf8(bytes) {
                let (body, submit) = super::super::agent_dispatch::split_pane_submit(text);
                if submit.is_some() && !body.trim().is_empty() {
                    self.input.submitted = Some(body.into());
                }
            }
        } else {
            self.input.observe(bytes);
        }
        if self.replay.is_none()
            && self
                .request
                .as_ref()
                .is_some_and(|request| request.reason == AccountSwitchReason::RateLimit)
        {
            self.replay = self.input.submitted.clone();
        }
    }

    fn mark_turn_completed(&mut self, completed: Instant) {
        self.turn_completed_at = Some(completed);
        self.input.submitted = None;
    }

    fn take_replay(&mut self, incarnation: u64) -> Option<String> {
        if self.replay_incarnation != Some(incarnation) {
            return None;
        }
        self.replay_incarnation = None;
        self.replay.take()
    }
}

struct SwitchRequest {
    incarnation: u64,
    account_id: String,
    reason: AccountSwitchReason,
    selected: Option<String>,
    probing: bool,
}

struct RetiredAccountPane {
    source: Session,
    cols: u16,
    rows: u16,
}

/// Plain composer input is kept only in memory. Editing/navigation for which we
/// cannot reconstruct the provider's composer invalidates automatic replay.
#[derive(Default)]
struct SubmittedInput {
    draft: Vec<u8>,
    ambiguous: bool,
    submitted: Option<String>,
    pasting: bool,
}

impl SubmittedInput {
    fn observe(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let marker = if self.pasting {
                b"\x1b[201~"
            } else {
                b"\x1b[200~"
            };
            if let Some(tail) = bytes.strip_prefix(marker) {
                self.pasting = !self.pasting;
                bytes = tail;
                continue;
            }
            let byte = bytes[0];
            bytes = &bytes[1..];
            if self.pasting && matches!(byte, b'\r' | b'\n' | b'\t') && self.draft.len() < 64 * 1024
            {
                self.draft.push(byte);
                continue;
            }
            match byte {
                b'\r' | b'\n' => {
                    if self.ambiguous || !self.draft.is_empty() {
                        self.submitted = (!self.ambiguous)
                            .then(|| String::from_utf8(std::mem::take(&mut self.draft)).ok())
                            .flatten()
                            .filter(|text| !text.trim().is_empty());
                    }
                    self.draft.clear();
                    self.ambiguous = false;
                }
                0x03 | 0x15 => {
                    self.draft.clear();
                    self.ambiguous = false;
                    self.pasting = false;
                }
                0x20..=0x7e | 0x80..=0xff if self.draft.len() < 64 * 1024 => {
                    self.draft.push(byte);
                }
                _ => self.ambiguous = true,
            }
        }
    }
}

impl OwnerState {
    pub(super) fn request_account_switch(
        &mut self,
        id: &str,
        incarnation: u64,
        deleted: Option<&str>,
    ) -> Result<(), String> {
        let runtime = self.exact(id, incarnation)?;
        let agent = runtime
            .agent
            .as_ref()
            .ok_or("This pane has no execution account.")?;
        let account = agent
            .snapshot
            .execution_account
            .as_ref()
            .ok_or("This pane has no execution account.")?;
        if deleted.is_some_and(|deleted| deleted != account.account_id) {
            return Err("The account used by the pane has changed.".into());
        }
        let account_id = account.account_id.clone();
        let state = self.account_switches.entry(id.into()).or_default();
        if state.request.is_none() {
            state.request = Some(SwitchRequest {
                incarnation,
                account_id,
                reason: AccountSwitchReason::Manual,
                selected: None,
                probing: false,
            });
            state.replay = None;
            state.replay_incarnation = None;
        }
        Ok(())
    }

    pub(super) fn request_quota_account_switch(
        &mut self,
        id: &str,
        failure: &crate::IssueMonitorFailure,
        now: UtcTimestamp,
    ) -> bool {
        let crate::IssueMonitorFailure::ProviderUsageLimit { resets_at, .. } = failure else {
            return false;
        };
        let Some(runtime) = self.runtimes.get(id) else {
            return false;
        };
        let Some(agent) = runtime.agent.as_ref() else {
            return false;
        };
        let Some(account) = agent.snapshot.execution_account.as_ref() else {
            return false;
        };
        if self.account_switches.get(id).is_some_and(|state| {
            state.request.as_ref().is_some_and(|request| {
                request.incarnation == runtime.incarnation
                    && request.reason == AccountSwitchReason::RateLimit
            })
        }) {
            return true;
        }
        let Ok(store) = crate::execution_environments::load() else {
            return false;
        };
        if store
            .accounts
            .iter()
            .filter(|a| a.tool == agent.active.agent_id)
            .count()
            < 2
        {
            return false;
        }
        let state = self.account_switches.entry(id.into()).or_default();
        let resets_at = resets_at
            .as_deref()
            .and_then(UtcTimestamp::parse_rfc3339)
            .filter(|reset| *reset > now);
        state.attempts.record_limit(&account.account_id, resets_at);
        state.request = Some(SwitchRequest {
            incarnation: runtime.incarnation,
            account_id: account.account_id.clone(),
            reason: AccountSwitchReason::RateLimit,
            selected: None,
            probing: false,
        });
        state.next_check = None;
        state.replay_incarnation = None;
        if state.replay.is_none() {
            state.replay = if agent.feedback.is_some() {
                Some("Please continue the work.\r".into())
            } else {
                state.input.submitted.clone()
            };
        }
        true
    }

    pub(super) fn remember_account_input(
        &mut self,
        id: &str,
        incarnation: u64,
        bytes: &[u8],
        typed: bool,
        started: Instant,
    ) {
        let live_account = self.runtimes.get(id).is_some_and(|runtime| {
            runtime.incarnation == incarnation
                && runtime
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.snapshot.execution_account.is_some())
        });
        let retiring_account = self.is_account_switch_waiting(id)
            && self.account_switches.get(id).is_some_and(|state| {
                state
                    .request
                    .as_ref()
                    .is_some_and(|request| request.incarnation == incarnation)
            });
        if !live_account && !retiring_account {
            return;
        }
        self.account_switches
            .entry(id.into())
            .or_default()
            .record_input(incarnation, bytes, typed, started);
    }

    pub(super) fn complete_account_turn(&mut self, id: &str, now: UtcTimestamp) {
        let Some(state) = self.account_switches.get_mut(id) else {
            return;
        };
        let quota_pending = state
            .request
            .as_ref()
            .is_some_and(|request| request.reason == AccountSwitchReason::RateLimit)
            || self.runtimes.get(id).is_some_and(|runtime| {
                let agent = runtime
                    .agent
                    .as_ref()
                    .map(|agent| agent.active.agent_id.as_str());
                runtime.pane.lock().ok().is_some_and(|pane| {
                    gwt_core::usage::detect_provider_limit_notice(&pane.screen().contents(), now)
                        .is_some_and(|notice| {
                            notice.provider == super::super::runtime_events::quota_provider(agent)
                        })
                })
            });
        if !quota_pending {
            state.mark_turn_completed(Instant::now());
        }
    }

    pub(super) fn account_input_started(&mut self, id: &str, incarnation: u64) {
        if self.runtimes.get(id).is_some_and(|runtime| {
            runtime.incarnation == incarnation
                && runtime
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.snapshot.execution_account.is_some())
        }) {
            self.account_switches
                .entry(id.into())
                .or_default()
                .input_writes += 1;
        }
    }

    pub(super) fn account_input_finished(&mut self, id: &str) {
        if let Some(state) = self.account_switches.get_mut(id) {
            state.input_writes = state.input_writes.saturating_sub(1);
        }
    }

    pub(super) fn next_account_switch_deadline(
        &self,
        now: Instant,
        wall: UtcTimestamp,
    ) -> Option<Instant> {
        self.account_switches
            .values()
            .filter(|state| {
                state.retired.is_some()
                    && state
                        .request
                        .as_ref()
                        .is_some_and(|request| !request.probing)
            })
            .filter_map(|state| state.next_check)
            .min()
            .map(|at| now + std::time::Duration::try_from(at - wall).unwrap_or_default())
    }

    pub(super) fn drive_account_switches(&mut self, now: UtcTimestamp) {
        let due: Vec<_> = self
            .account_switches
            .iter()
            .filter_map(|(id, state)| {
                let request = state.request.as_ref()?;
                let ready = !request.probing
                    && state.input_writes == 0
                    && !self.closing.contains_key(id)
                    && !self.pending.contains_key(id)
                    && !self.readiness.contains_key(id)
                    && if state.retired.is_some() {
                        state.next_check.is_some_and(|at| at <= now)
                    } else {
                        self.runtimes
                            .get(id)
                            .is_some_and(|runtime| ready_to_switch(runtime, request))
                    };
                ready.then(|| id.clone())
            })
            .collect();
        for id in due {
            self.probe_account_pool(&id);
        }
        let replays: Vec<_> = self
            .account_switches
            .iter()
            .filter_map(|(id, state)| {
                let incarnation = state.replay_incarnation?;
                let runtime = self.runtimes.get(id)?;
                (runtime.incarnation == incarnation
                    && state.input_writes == 0
                    && !self.readiness.contains_key(id)
                    && !self.closing.contains_key(id)
                    && Self::runtime_status(runtime) == WindowProcessStatus::Idle)
                    .then(|| (id.clone(), incarnation))
            })
            .collect();
        for (id, incarnation) in replays {
            self.replay_account_input(&id, incarnation);
        }
    }

    fn probe_account_pool(&mut self, id: &str) {
        let Some(state) = self.account_switches.get_mut(id) else {
            return;
        };
        let Some(request) = state.request.as_mut() else {
            return;
        };
        let source = if let Some(retired) = state.retired.as_ref() {
            &retired.source
        } else {
            &self.runtimes[id]
                .agent
                .as_ref()
                .expect("ready account switch")
                .snapshot
        };
        let tool = source.agent_id.command().to_owned();
        let allow_fresh_start = request.reason == AccountSwitchReason::RateLimit
            && self
                .runtimes
                .get(id)
                .is_some_and(|runtime| runtime.hook_status == Some(WindowProcessStatus::Starting));
        if source.exact_resume_session_id().is_none()
            && !allow_fresh_start
            && state.retired.is_none()
        {
            // The authenticated native conversation may have been saved by the
            // provider after the launch snapshot was installed.
            self.refresh_owner_session_snapshot(id);
            if self.runtimes[id]
                .agent
                .as_ref()
                .is_none_or(|agent| agent.snapshot.exact_resume_session_id().is_none())
            {
                self.account_switches
                    .get_mut(id)
                    .expect("queued switch")
                    .request = None;
                self.account_switch_notice(
                    id,
                    "The provider has no conversation available to resume.",
                );
                return;
            }
        }
        let request = self
            .account_switches
            .get_mut(id)
            .expect("queued switch")
            .request
            .as_mut()
            .expect("queued switch request");
        request.probing = true;
        let id = id.to_owned();
        let incarnation = request.incarnation;
        let sender = self.sender.clone();
        self.input_jobs += 1;
        super::spawn_owner_worker(move || {
            let pool = crate::execution_environments::load().map(|store| {
                store
                    .accounts
                    .into_iter()
                    .filter(|account| {
                        account.tool == tool
                            && crate::account_homes::pool_sign_in(account).is_some()
                    })
                    .map(|account| account.id)
                    .collect::<Vec<_>>()
            });
            let _ = sender.send(super::OwnerMessage::AccountPoolProbed {
                id,
                incarnation,
                pool,
            });
        });
    }

    pub(super) fn account_pool_probed(
        &mut self,
        id: &str,
        incarnation: u64,
        pool: std::io::Result<Vec<String>>,
    ) {
        self.input_jobs -= 1;
        let Some(state) = self.account_switches.get_mut(id) else {
            return;
        };
        let Some(request) = state
            .request
            .as_mut()
            .filter(|request| request.incarnation == incarnation)
        else {
            return;
        };
        request.probing = false;
        if state.input_writes != 0
            || self.stopping
            || (state.retired.is_none()
                && self
                    .runtimes
                    .get(id)
                    .is_none_or(|runtime| !ready_to_switch(runtime, request)))
        {
            return;
        }
        let Ok(pool) = pool else {
            state.request = None;
            state.next_check = None;
            self.account_switch_notice(id, "Could not read execution accounts for the switch.");
            return;
        };
        let now = UtcTimestamp::now();
        let pool = pool.iter().map(String::as_str);
        // An explicit request looks at the configured priority. A quota
        // refusal instead walks forward from the account that was refused.
        request.selected = match request.reason {
            AccountSwitchReason::Manual => {
                pool.clone().find(|account| *account != request.account_id)
            }
            AccountSwitchReason::RateLimit => {
                state
                    .attempts
                    .next_account(pool.clone(), &request.account_id, now)
            }
        }
        .map(str::to_owned);
        if request.selected.is_none() && request.reason == AccountSwitchReason::Manual {
            state.request = None;
            self.account_switch_notice(
                id,
                "There is no other signed-in account available to resume.",
            );
            return;
        }
        state.next_check = request
            .selected
            .is_none()
            .then(|| state.attempts.next_reset(pool, now))
            .flatten();
        if state.retired.is_some() {
            self.launch_switched_account(id);
            return;
        }
        let Some(incarnation) = self.runtimes.get(id).map(|runtime| runtime.incarnation) else {
            return;
        };
        let detail = if request.selected.is_some() {
            "Resuming with another account".into()
        } else if let Some(reset) = state.next_check {
            format!(
                "Waiting to resume (account resets at {})",
                reset.to_rfc3339()
            )
        } else {
            "Waiting to resume (account reset time unknown)".into()
        };
        let agent = self.runtimes[id]
            .agent
            .as_ref()
            .expect("ready account switch");
        state.waiting_window = Some(crate::IssueMonitorWindowObservation {
            monitor_owned: agent.snapshot.launch_route == gwt_agent::LaunchRoute::Autonomous,
            window_id: id.into(),
            issue_number: agent.snapshot.linked_issue_number,
            status: WindowProcessStatus::Waiting,
            review_dispatch: agent.snapshot.review_dispatch,
            hold_reason: Some(detail.clone()),
            last_output_at: None,
            session_id: Some(agent.snapshot.id.clone()),
        });
        self.publish_status(
            id.into(),
            incarnation,
            WindowProcessStatus::Waiting,
            Some(detail),
            false,
        );
        let runtime = self.runtimes.remove(id).expect("ready account switch");
        let (reply, _) = oneshot::channel();
        self.start_close(id.into(), runtime, CloseCause::AccountSwitch, reply);
    }

    pub(super) fn account_switch_retired(&mut self, id: &str, runtime: &WindowRuntime) {
        let Some(state) = self.account_switches.get_mut(id) else {
            return;
        };
        if state
            .request
            .as_ref()
            .is_none_or(|request| request.incarnation != runtime.incarnation)
        {
            return;
        }
        let Some(agent) = runtime.agent.as_ref() else {
            return;
        };
        let source = gwt_agent::session_ledger::load_session_candidate(
            &gwt_core::paths::gwt_sessions_dir(),
            &agent.snapshot.id,
        );
        let Some(source) = source else {
            self.account_switches.remove(id);
            self.account_switch_notice(
                id,
                "The saved conversation could not be loaded after stopping its account.",
            );
            return;
        };
        let (rows, cols) = runtime
            .pane
            .lock()
            .map(|pane| pane.screen().size())
            .unwrap_or((24, 80));
        state.retired = Some(RetiredAccountPane { source, cols, rows });
        self.launch_switched_account(id);
    }

    fn launch_switched_account(&mut self, id: &str) {
        let Some(state) = self.account_switches.get_mut(id) else {
            return;
        };
        let Some(request) = state.request.as_mut() else {
            return;
        };
        let Some(account_id) = request.selected.take() else {
            return;
        };
        let reason = request.reason;
        let retired = state.retired.take().expect("closed account pane");
        state.request = None;
        state.next_check = None;
        let lineage = crate::project_execution_policy::session_lineage(&self.root, &retired.source);
        if let Err(error) = crate::project_execution_policy::admission(&self.root, &lineage) {
            self.account_switches.remove(id);
            self.account_switch_notice(id, &error);
            return;
        }
        let (reply, mut receipt) = oneshot::channel();
        self.restore_saved_session_with_account(
            OwnerCommand::RestoreSavedSession {
                id: id.into(),
                operation_id: uuid::Uuid::new_v4().to_string(),
                source_session_id: retired.source.id.clone(),
                expected_source: Box::new(retired.source),
                origin: SessionLaunchOrigin::UserRestart,
                cols: retired.cols,
                rows: retired.rows,
            },
            reply,
            Some((account_id, reason)),
        );
        if let Ok(Err(error)) = receipt.try_recv() {
            self.discard_account_switch(id);
            self.account_switch_notice(id, &error);
        }
    }

    pub(super) fn discard_account_switch(&mut self, id: &str) {
        self.account_switches.remove(id);
    }

    pub(super) fn cancel_account_switch_request(&mut self, id: &str, incarnation: u64) {
        if self.account_switches.get(id).is_some_and(|state| {
            state
                .request
                .as_ref()
                .is_some_and(|request| request.incarnation == incarnation)
        }) {
            self.discard_account_switch(id);
        }
    }

    pub(super) fn arm_account_input_replay(&mut self, id: &str, incarnation: u64) {
        if let Some(state) = self.account_switches.get_mut(id) {
            state.replay_incarnation = state.replay.as_ref().map(|_| incarnation);
            state.waiting_window = None;
        }
    }

    pub(super) fn account_switch_observations(
        &self,
    ) -> impl Iterator<Item = crate::IssueMonitorWindowObservation> + '_ {
        self.account_switches
            .values()
            .filter_map(|state| state.waiting_window.clone())
    }

    pub(super) fn is_account_switch_waiting(&self, id: &str) -> bool {
        self.account_switches
            .get(id)
            .is_some_and(|state| state.waiting_window.is_some())
    }

    pub(super) fn replay_account_input(&mut self, id: &str, incarnation: u64) {
        let Some(state) = self.account_switches.get_mut(id) else {
            return;
        };
        if self
            .runtimes
            .get(id)
            .is_none_or(|runtime| runtime.pty.has_unsent_user_input())
        {
            return;
        }
        let Some(mut text) = state.take_replay(incarnation) else {
            return;
        };
        if !text.ends_with('\r') {
            text.push('\r');
        }
        let (reply, _) = oneshot::channel();
        self.request(
            super::OwnerRequest {
                project_key: self.project_key.clone(),
                command: OwnerCommand::SendInput {
                    id: id.into(),
                    incarnation,
                    text,
                },
            },
            reply,
        );
    }

    fn account_switch_notice(&self, id: &str, message: &str) {
        tracing::info!(
            window_id = id,
            message,
            "execution account switch unavailable"
        );
        let _ = self.events.send(OwnerEvent::Notice {
            level: "warning".into(),
            message: message.into(),
            issue_number: self.runtimes.get(id).and_then(|runtime| {
                runtime
                    .agent
                    .as_ref()
                    .and_then(|agent| agent.snapshot.linked_issue_number)
            }),
        });
    }
}

fn ready_to_switch(runtime: &WindowRuntime, request: &SwitchRequest) -> bool {
    runtime.incarnation == request.incarnation
        && runtime.agent.as_ref().is_some_and(|agent| {
            agent.pending_fresh.is_none()
                && agent.pending_continue.is_none()
                && !agent.binding_repair_inflight
                && !agent.monitor_commit_inflight
                && !agent.monitor_answer_inflight
        })
        && runtime.policy.approval.is_none()
        && (request.reason == AccountSwitchReason::RateLimit
            || (OwnerState::runtime_status(runtime) == WindowProcessStatus::Idle
                && !runtime.pty.has_unsent_user_input()))
}

#[cfg(test)]
mod tests {
    use super::{AccountSwitchState, Instant, SubmittedInput};

    #[test]
    fn replay_uses_the_last_submitted_turn_and_disarms_ambiguous_edits() {
        let mut input = SubmittedInput::default();
        input.observe("こんにちは".as_bytes());
        input.observe(b"\r");
        input.observe(b"unfinished");
        assert_eq!(input.submitted.as_deref(), Some("こんにちは"));
        input.observe(b"\x1b[A\r");
        assert_eq!(input.submitted, None);
        input.observe(b"\x1b[200~line 1\nline 2\x1b[201~\r");
        assert_eq!(input.submitted.as_deref(), Some("line 1\nline 2"));
    }

    #[test]
    fn account_switch_replay_uses_only_the_new_incarnation_and_never_a_completed_turn() {
        let mut state = AccountSwitchState::default();
        let input_started = Instant::now();
        state.record_input(1, b"finished turn\r", true, input_started);
        assert_eq!(state.input.submitted.as_deref(), Some("finished turn"));
        state.mark_turn_completed(input_started);
        state.record_input(1, b"finished turn\r", true, input_started);
        assert!(state.input.submitted.is_none());

        state.replay = Some("rejected turn".into());
        state.replay_incarnation = Some(2);
        assert_eq!(state.take_replay(1), None);
        assert_eq!(state.take_replay(2).as_deref(), Some("rejected turn"));
        assert_eq!(state.take_replay(2), None);
    }
}
