//! Durable, fail-closed safe-switch intentions; execution belongs to the Host.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSwitchRequest {
    pub operation_id: String,
    pub member_id: String,
    pub expected_roster_revision: u64,
    pub target_issue_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentSwitchAction {
    Read {
        #[serde(default)]
        request_id: Option<String>,
    },
    Request {
        operation_id: String,
        member_id: String,
        expected_roster_revision: u64,
        target_issue_number: u64,
    },
    Cancel {
        operation_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSwitchStage {
    Requested,
    Checkpointing,
    CheckpointVerified,
    StoppingCurrent,
    LaunchingNext,
    Completed,
    Cancelled,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwitchSource {
    pub assignment: crate::agent_names::AgentMemberAssignment,
    pub window_id: String,
    pub claim_id: String,
    pub confirmation: IssueMonitorLaunchConfirmation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwitchCheckpoint {
    pub verified_head: String,
    pub work_id: String,
    pub board_entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwitchIntent {
    pub request: AgentSwitchRequest,
    pub source: AgentSwitchSource,
    pub stage: AgentSwitchStage,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_stop: Option<IssueMonitorRuntimeStopRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<AgentSwitchCheckpoint>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub runtime_exit_confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_input_revision: Option<u64>,
}

/// User-facing state intentionally omits the execution proof and filesystem identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwitchView {
    pub operation_id: String,
    pub member_id: String,
    pub source_issue_number: u64,
    pub source_session_id: String,
    pub target_issue_number: u64,
    pub stage: AgentSwitchStage,
    pub created_at: String,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_work_id: Option<String>,
    #[serde(default)]
    pub can_resume_saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwitchSnapshot {
    pub project_key: String,
    pub intents: Vec<AgentSwitchView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl AgentSwitchIntent {
    pub(crate) fn has_admission_pin(&self) -> bool {
        matches!(
            self.stage,
            AgentSwitchStage::StoppingCurrent
                | AgentSwitchStage::LaunchingNext
                | AgentSwitchStage::Unknown
        )
    }

    pub(super) fn admission_exclusion(
        &self,
        number: Option<u64>,
        member: Option<&str>,
        source_resume: bool,
    ) -> Option<String> {
        let selected_member = member == Some(self.request.member_id.as_str());
        let source = number.is_some() && number == self.source.assignment.issue_number;
        let target = number == Some(self.request.target_issue_number);
        if !self.has_admission_pin() || (!source && !target && !selected_member) {
            return None;
        }
        let allowed = selected_member
            && source != target
            && ((source && source_resume)
                || (target
                    && self.stage == AgentSwitchStage::LaunchingNext
                    && self.runtime_exit_confirmed));
        (!allowed).then(|| format!("agent_switch_pending: member {} is pinned by switch {}; wait for the selected target or explicitly resume its saved source work", self.request.member_id, self.request.operation_id))
    }
}

impl IssueMonitorState {
    /// Plan against existing candidate observations without publishing source absence.
    /// Normal claim and materialization gates run again after physical exit.
    pub fn agent_switch_target_exclusions(&self, intent: &AgentSwitchIntent) -> Vec<String> {
        let mut proposed = self.clone();
        if let Some(member) = proposed.agent_roster.as_mut().and_then(|roster| {
            roster
                .members
                .iter_mut()
                .find(|member| member.agent_member_id == intent.request.member_id)
        }) {
            if member.assignment.as_ref() == Some(&intent.source.assignment) {
                member.assignment = None;
            }
        }
        if proposed
            .agent_switch_intents
            .get(&intent.request.operation_id)
            .is_some_and(|saved| saved.request == intent.request && saved.source == intent.source)
        {
            proposed
                .agent_switch_intents
                .remove(&intent.request.operation_id);
        }
        let mut reasons = proposed.claim_candidate_exclusions(intent.request.target_issue_number);
        let target = intent.request.target_issue_number;
        let now = chrono::Utc::now().to_rfc3339();
        if proposed.issue_is_closed(target)
            || proposed
                .inbox_item(target)
                .is_some_and(|item| item.issue.state != IssueMonitorIssueState::Open)
        {
            reasons.push("target_closed: wait for a canonical Reopened revision".into());
        }
        if !proposed.has_launch_profile() {
            reasons
                .push("launch_profile_missing: configure an Issue Monitor launch profile".into());
        }
        if proposed.launch_auth_required {
            reasons.push("github_auth_required: restore Issue Monitor authentication".into());
        }
        if let Some(hold) = proposed.launch_admission_hold_at(&now) {
            reasons.push(format!("launch_admission_held: {hold:?}"));
        }
        if !proposed.retry_ready_for_saved_profile(target, &now) {
            reasons.push("target_retry_or_tier_held: wait for the normal launch policy".into());
        }
        if let Some(record) = proposed.permission_readiness_block_for_launch(target, &now) {
            reasons.push(format!("permission_readiness: {}", record.reason));
        }
        // The Host rechecks the exact source runtime before stopping it.
        let occupied_after_source_exit = self.occupied_slot_count().saturating_sub(1);
        if self.effective_max_active_agents() <= occupied_after_source_exit {
            reasons.push("target_capacity_held: no normal launch slot after source exit".into());
        }
        if !self.config.enabled {
            reasons.push("issue_monitor_disabled: enable Issue Monitor before switching".into());
        }
        reasons
    }
}

/// Shared UI/CLI transaction. Runtime and Session observations happen before the writer lock.
pub fn mutate_project_agent_switch(
    root: &Path,
    sessions_dir: &Path,
    action: AgentSwitchAction,
) -> Result<AgentSwitchSnapshot, String> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let project = gwt_core::paths::project_scope_hash(root).to_string();
    if let AgentSwitchAction::Read { request_id } = &action {
        return load_issue_monitor_prefs(&path)
            .map(|prefs| {
                let mut snapshot = prefs.agent_switch_snapshot(&project);
                snapshot.request_id = request_id.clone();
                snapshot
            })
            .map_err(|error| error.to_string());
    }
    let host = crate::process::current_hostname();
    let request = match &action {
        AgentSwitchAction::Request {
            operation_id,
            member_id,
            expected_roster_revision,
            target_issue_number,
        } => Some(AgentSwitchRequest {
            operation_id: operation_id.clone(),
            member_id: member_id.clone(),
            expected_roster_revision: *expected_roster_revision,
            target_issue_number: *target_issue_number,
        }),
        _ => None,
    };
    let operation = match &action {
        AgentSwitchAction::Request { operation_id, .. }
        | AgentSwitchAction::Cancel { operation_id } => operation_id,
        AgentSwitchAction::Read { .. } => unreachable!("read returned before mutation"),
    };
    let prepared = if let Some(request) = &request {
        let mut observed = load_issue_monitor_prefs(&path).map_err(|error| error.to_string())?;
        let existing = observed.agent_switch_intents.contains_key(operation);
        let preview = observed.request_agent_switch(request.clone(), &host)?;
        if existing {
            // Retries are about the original operation, never a new Session incarnation.
            None
        } else {
            let session_id = preview
                .source
                .assignment
                .session_id
                .as_deref()
                .ok_or("Source Session is unavailable")?;
            let session =
                gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
                    .map_err(|error| error.to_string())?;
            let session_root = session
                .project_state_root
                .as_deref()
                .unwrap_or(&session.worktree_path);
            if session.id != session_id
                || crate::issue_monitor_prefs_path_for_repo_path(session_root) != path
                || crate::pm_registry::pane_is_pm(
                    root,
                    Some(&session.worktree_path),
                    Some(&session.id),
                )
            {
                return Err("Source must be an implementation Session in this project".into());
            }
            let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)?
                .ok_or("Source execution identity is unavailable")?;
            let stop = crate::monitor_duplicate_runtime::capture_stop_request(
                root,
                preview
                    .source
                    .assignment
                    .issue_number
                    .ok_or("Source Issue is unavailable")?,
                &preview.source.window_id,
                Some(session_id),
                &preview.created_at,
            )
            .map_err(|error| error.to_string())?
            .ok_or("Exact source runtime is unavailable; keep the current Session")?;
            if stop.execution_identity != identity || !stop_matches_source(&preview.source, &stop) {
                return Err("Source runtime changed before switch request".into());
            }
            Some((preview, stop))
        }
    } else {
        None
    };
    let (saved, _) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        if let Some(request) = request {
            let existing = prefs.agent_switch_intents.contains_key(operation);
            let accepted = prefs
                .request_agent_switch(request, &host)
                .map_err(io::Error::other)?;
            if !existing {
                let (preview, stop) = prepared
                    .as_ref()
                    .ok_or_else(|| io::Error::other("Switch operation changed before commit"))?;
                if accepted.source != preview.source || !stop_matches_source(&accepted.source, stop)
                {
                    return Err(io::Error::other(
                        "Source assignment changed before switch commit",
                    ));
                }
                prefs
                    .agent_switch_intents
                    .get_mut(operation)
                    .expect("request inserted operation")
                    .runtime_stop = Some(stop.clone());
            }
        } else {
            prefs
                .cancel_agent_switch(operation)
                .map_err(io::Error::other)?;
        }
        Ok(())
    })
    .map_err(|error| error.to_string())?;
    let readback = load_issue_monitor_prefs(&path).map_err(|error| error.to_string())?;
    if saved.agent_switch_intents.get(operation) != readback.agent_switch_intents.get(operation) {
        return Err("Switch operation changed during readback; read its status before retrying (the request may already be saved)".into());
    }
    Ok(readback.agent_switch_snapshot(&project))
}

fn stop_matches_source(source: &AgentSwitchSource, stop: &IssueMonitorRuntimeStopRequest) -> bool {
    let assignment = &source.assignment;
    assignment.issue_number == Some(stop.issue_number)
        && assignment.session_id.as_deref() == Some(&stop.session_id)
        && source.window_id == stop.window_id
        && assignment.host_pid == stop.host_pid
        && assignment.host_started_at == Some(stop.host_started_at)
        && stop.execution_identity.session_id == stop.session_id
        && stop.execution_identity.execution_binding.session_id == stop.session_id
        && stop.execution_identity.linked_issue_number == Some(stop.issue_number)
        && stop.runtime_incarnation > 0
        && stop.child_pid > 0
        && stop.child_started_at > 0
}

impl IssueMonitorPrefs {
    fn resolve_agent_switch_source(
        &self,
        member_id: &str,
        host: &str,
    ) -> Result<AgentSwitchSource, String> {
        let unavailable = || {
            "Current member assignment or launch identity is unavailable or ambiguous".to_string()
        };
        let roster = self.agent_roster.as_ref().ok_or_else(unavailable)?;
        roster.validate()?;
        let member = roster
            .members
            .iter()
            .find(|member| member.agent_member_id == member_id && member.enabled)
            .ok_or_else(unavailable)?;
        let assignment = member.assignment.as_ref().ok_or_else(unavailable)?;
        let session = assignment
            .session_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(unavailable)?;
        let issue = assignment
            .issue_number
            .filter(|number| *number > 0)
            .ok_or_else(unavailable)?;
        if host.is_empty()
            || assignment
                .project_key
                .as_deref()
                .is_some_and(|key| self.agent_registry_project_key.as_deref() != Some(key))
            || assignment.host_id != host
            || assignment.host_pid == 0
            || assignment.host_started_at.is_none()
            || assignment.reservation_id.is_empty()
            || assignment
                .delivery_id
                .as_ref()
                .is_none_or(|id| id.is_empty())
            || roster
                .members
                .iter()
                .filter(|member| {
                    member.assignment.as_ref().is_some_and(|other| {
                        other.session_id.as_deref() == Some(session)
                            || (other.issue_number == Some(issue)
                                && other.project_key.as_deref().is_none_or(|key| {
                                    self.agent_registry_project_key.as_deref() == Some(key)
                                }))
                    })
                })
                .count()
                != 1
        {
            return Err(unavailable());
        }
        let launches: Vec<_> = self
            .launched_issues
            .iter()
            .filter(|launch| launch.issue_number == issue)
            .collect();
        let [launch] = launches.as_slice() else {
            return Err(unavailable());
        };
        let claim_id = self
            .launched_claims
            .get(&issue)
            .filter(|id| !id.is_empty())
            .ok_or_else(unavailable)?;
        let confirmation = self
            .launch_confirmations
            .get(&issue)
            .ok_or_else(unavailable)?;
        if launch.window_id.is_empty()
            || self.launch_bindings.get(&launch.window_id) != Some(&issue)
            || self
                .launch_bindings
                .values()
                .filter(|number| **number == issue)
                .count()
                != 1
            || confirmation.window_id != launch.window_id
            || confirmation.claim_id.as_ref() != Some(claim_id)
            || confirmation.delivery_id != assignment.delivery_id
        {
            return Err(unavailable());
        }
        Ok(AgentSwitchSource {
            assignment: assignment.clone(),
            window_id: launch.window_id.clone(),
            claim_id: claim_id.clone(),
            confirmation: confirmation.clone(),
        })
    }

    /// Call under the preferences writer lock. Only an intention is persisted.
    pub fn request_agent_switch(
        &mut self,
        request: AgentSwitchRequest,
        host: &str,
    ) -> Result<AgentSwitchIntent, String> {
        if !uuid::Uuid::parse_str(&request.operation_id)
            .is_ok_and(|id| id.hyphenated().to_string() == request.operation_id)
        {
            return Err("operation_id must be a canonical lowercase hyphenated UUID".into());
        }
        if let Some(existing) = self.agent_switch_intents.get(&request.operation_id) {
            return if existing.request == request {
                Ok(existing.clone())
            } else {
                Err("operation_id already names a different switch request".into())
            };
        }
        if self.agent_switch_intents.values().any(|intent| {
            intent.request.member_id == request.member_id
                && !matches!(
                    intent.stage,
                    AgentSwitchStage::Completed
                        | AgentSwitchStage::Cancelled
                        | AgentSwitchStage::Failed
                )
        }) {
            return Err("This member already has an unresolved switch request".into());
        }
        if self
            .agent_roster
            .as_ref()
            .is_none_or(|roster| roster.revision != request.expected_roster_revision)
        {
            return Err("Agent roster changed; read the current schedule before retrying".into());
        }
        let source = self.resolve_agent_switch_source(&request.member_id, host)?;
        let intent = AgentSwitchIntent {
            request,
            source,
            stage: AgentSwitchStage::Requested,
            created_at: chrono::Utc::now().to_rfc3339(),
            reason: None,
            runtime_stop: None,
            checkpoint: None,
            runtime_exit_confirmed: false,
            source_input_revision: None,
        };
        if !self.agent_switch_target_available(&intent, host) {
            return Err("Switch target is no longer queued and reserved to this member, or is already in flight".into());
        }
        self.agent_switch_intents
            .insert(intent.request.operation_id.clone(), intent.clone());
        Ok(intent)
    }

    pub fn agent_switch_source_unchanged(&self, intent: &AgentSwitchIntent, host: &str) -> bool {
        self.resolve_agent_switch_source(&intent.request.member_id, host)
            .ok()
            .as_ref()
            == Some(&intent.source)
    }

    pub fn agent_switch_target_available(&self, intent: &AgentSwitchIntent, host: &str) -> bool {
        let target = intent.request.target_issue_number;
        target > 0 && intent.source.assignment.issue_number != Some(target)
            && self.agent_roster.as_ref().is_some_and(|roster| roster.members.iter().any(|member| member.agent_member_id == intent.request.member_id && member.enabled && member.assignment.as_ref().is_some_and(|assignment| assignment.host_id == host && assignment.project_key.as_deref().is_none_or(|key| self.agent_registry_project_key.as_deref() == Some(key)))))
            && self.agent_issue_schedule.validate().is_ok()
            && self.agent_issue_schedule.reservations.iter().any(|reservation| reservation.host_id == host && reservation.agent_member_id == intent.request.member_id && reservation.issue_number == target)
            && self.terminal_queues.get(host).is_some_and(|queue| queue.entries.iter().filter(|entry| entry.number == target).count() == 1)
            && !self.pending_effects.iter().any(|effect| matches!(effect.payload, IssueMonitorEffectPayload::AcquireClaim { issue_number, .. } if issue_number == target))
            && !self.launching_issues.iter().any(|entry| entry.issue_number == target)
            && !self.pending_launch_deliveries.iter().any(|entry| entry.issue_number == target)
            && !self.launched_issues.iter().any(|entry| entry.issue_number == target)
            && !self.launch_bindings.values().any(|number| *number == target)
            && !self.agent_roster.iter().flat_map(|roster| &roster.members).any(|member| member.assignment.as_ref().is_some_and(|assignment| assignment.issue_number == Some(target) && assignment.project_key.as_deref().is_none_or(|key| self.agent_registry_project_key.as_deref() == Some(key))))
    }

    fn can_resume_agent_switch_source(&self, intent: &AgentSwitchIntent, host: &str) -> bool {
        // Continue Work keeps its existing authority and liveness checks.
        matches!(
            intent.stage,
            AgentSwitchStage::Unknown | AgentSwitchStage::LaunchingNext
        ) && intent.checkpoint.is_some()
            && intent.source.assignment.host_id == host
            && intent
                .source
                .assignment
                .issue_number
                .is_some_and(|number| number > 0)
            && !self.scheduled_issue_has_pending_claim(intent.request.target_issue_number)
            && !self.scheduled_issue_in_flight(intent.request.target_issue_number)
            && !self
                .launch_bindings
                .values()
                .any(|number| *number == intent.request.target_issue_number)
    }

    pub(crate) fn agent_switch_admission_exclusion(
        &self,
        number: Option<u64>,
        member: &str,
        inherited: bool,
        host: &str,
    ) -> Option<String> {
        self.agent_switch_intents.values().find_map(|intent| {
            let source_resume = inherited && self.can_resume_agent_switch_source(intent, host);
            intent.admission_exclusion(number, Some(member), source_resume)
        })
    }

    /// Called only at the end of a successful explicit source reservation transaction.
    pub(crate) fn abandon_agent_switch_for_source_resume(
        &mut self,
        number: Option<u64>,
        member: &str,
        inherited: bool,
        host: &str,
    ) {
        let abandoned: Vec<_> = self
            .agent_switch_intents
            .values()
            .filter(|intent| {
                inherited
                    && intent.request.member_id == member
                    && intent.source.assignment.issue_number == number
                    && self.can_resume_agent_switch_source(intent, host)
            })
            .map(|intent| intent.request.operation_id.clone())
            .collect();
        for operation in abandoned {
            let intent = self
                .agent_switch_intents
                .get_mut(&operation)
                .expect("saved source was checked");
            intent.stage = AgentSwitchStage::Failed;
            intent.reason =
                Some("Saved source work was explicitly resumed; switch abandoned".into());
        }
    }

    pub fn cancel_agent_switch(&mut self, operation: &str) -> Result<AgentSwitchIntent, String> {
        let intent = self
            .agent_switch_intents
            .get_mut(operation)
            .ok_or("Unknown switch operation")?;
        if intent.stage == AgentSwitchStage::Cancelled {
            return Ok(intent.clone());
        }
        if !matches!(
            intent.stage,
            AgentSwitchStage::Requested
                | AgentSwitchStage::Checkpointing
                | AgentSwitchStage::CheckpointVerified
        ) {
            return Err("Switch cannot be cancelled after stopping has begun".into());
        }
        intent.stage = AgentSwitchStage::Cancelled;
        intent.reason = Some("Cancelled before stopping the current Session".into());
        Ok(intent.clone())
    }

    pub fn transition_agent_switch(
        &mut self,
        operation: &str,
        next: AgentSwitchStage,
        reason: Option<String>,
    ) -> Result<AgentSwitchIntent, String> {
        use AgentSwitchStage::*;
        let intent = self
            .agent_switch_intents
            .get(operation)
            .ok_or("Unknown switch operation")?;
        let current = intent.stage;
        if current != next
            && !(matches!(
                (current, next),
                (Requested, Checkpointing)
                    | (CheckpointVerified, StoppingCurrent)
                    | (StoppingCurrent, LaunchingNext)
                    | (LaunchingNext, Completed)
            ) || (matches!(next, Failed | Unknown)
                && !matches!(current, Completed | Cancelled | Failed | Unknown)))
        {
            return Err("Unsafe switch stage transition".into());
        }
        if next == Checkpointing && intent.runtime_stop.is_none() {
            return Err(
                "Exact initial runtime identity is unavailable; keep the current Session".into(),
            );
        }
        if next == StoppingCurrent
            && (intent.checkpoint.is_none()
                || intent.runtime_stop.is_none()
                || !self.agent_switch_source_unchanged(intent, &intent.source.assignment.host_id)
                || !self.agent_switch_target_available(intent, &intent.source.assignment.host_id))
        {
            return Err("Switch checkpoint, source, or target changed before stop".into());
        }
        if next == LaunchingNext && (!intent.runtime_exit_confirmed || intent.checkpoint.is_none())
        {
            return Err("Current runtime exit has not been physically confirmed".into());
        }
        let intent = self
            .agent_switch_intents
            .get_mut(operation)
            .expect("operation was checked");
        intent.stage = next;
        intent.reason = reason;
        Ok(intent.clone())
    }

    /// Store the Host-verified receipt only for the originally captured runtime.
    pub fn verify_agent_switch_checkpoint(
        &mut self,
        operation: &str,
        checkpoint: AgentSwitchCheckpoint,
        stop: IssueMonitorRuntimeStopRequest,
        host: &str,
    ) -> Result<AgentSwitchIntent, String> {
        let intent = self
            .agent_switch_intents
            .get(operation)
            .ok_or("Unknown switch operation")?;
        if intent.stage == AgentSwitchStage::CheckpointVerified {
            return if intent.checkpoint.as_ref() == Some(&checkpoint)
                && intent.runtime_stop.as_ref() == Some(&stop)
                && intent.source.assignment.host_id == host
                && stop_matches_source(&intent.source, &stop)
            {
                Ok(intent.clone())
            } else {
                Err("Checkpoint retry differs from the saved receipt or original runtime".into())
            };
        }
        if intent.stage != AgentSwitchStage::Checkpointing
            || intent.runtime_stop.as_ref() != Some(&stop)
            || !self.agent_switch_source_unchanged(intent, host)
            || !self.agent_switch_target_available(intent, host)
            || !stop_matches_source(&intent.source, &stop)
            || !matches!(checkpoint.verified_head.len(), 40 | 64)
            || !checkpoint
                .verified_head
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || checkpoint.work_id.trim().is_empty()
            || checkpoint.board_entry_id.trim().is_empty()
        {
            return Err(
                "Checkpoint does not match the saved source and exact runtime identity".into(),
            );
        }
        let intent = self
            .agent_switch_intents
            .get_mut(operation)
            .expect("operation was checked");
        intent.checkpoint = Some(checkpoint);
        intent.stage = AgentSwitchStage::CheckpointVerified;
        intent.reason = None;
        Ok(intent.clone())
    }

    /// Startup never resumes an unexecuted intent or repeats a destructive action.
    pub fn recover_agent_switches_after_restart(&mut self, host: &str) -> Vec<String> {
        let mut changed = Vec::new();
        for (operation, intent) in &mut self.agent_switch_intents {
            if intent.source.assignment.host_id != host {
                continue;
            }
            intent.stage = match intent.stage {
                AgentSwitchStage::Requested
                | AgentSwitchStage::Checkpointing
                | AgentSwitchStage::CheckpointVerified => AgentSwitchStage::Cancelled,
                AgentSwitchStage::StoppingCurrent | AgentSwitchStage::LaunchingNext => {
                    AgentSwitchStage::Unknown
                }
                _ => continue,
            };
            intent.reason = Some("Host restarted; saved intent will not be replayed".into());
            changed.push(operation.clone());
        }
        changed
    }

    pub fn agent_switch_snapshot(&self, project_key: &str) -> AgentSwitchSnapshot {
        let host = crate::process::current_hostname();
        let mut intents: Vec<_> = self
            .agent_switch_intents
            .values()
            .map(|intent| AgentSwitchView {
                operation_id: intent.request.operation_id.clone(),
                member_id: intent.request.member_id.clone(),
                source_issue_number: intent.source.assignment.issue_number.unwrap_or_default(),
                source_session_id: intent
                    .source
                    .assignment
                    .session_id
                    .clone()
                    .unwrap_or_default(),
                target_issue_number: intent.request.target_issue_number,
                stage: intent.stage,
                created_at: intent.created_at.clone(),
                reason: intent.reason.clone(),
                saved_work_id: intent
                    .checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoint.work_id.clone()),
                can_resume_saved: self.can_resume_agent_switch_source(intent, &host),
            })
            .collect();
        intents.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.operation_id.cmp(&b.operation_id))
        });
        AgentSwitchSnapshot {
            project_key: project_key.into(),
            intents,
            request_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_prefs() -> serde_json::Value {
        let mut value = serde_json::to_value(IssueMonitorPrefs::default()).unwrap();
        value["agent_switch_intents"] = serde_json::json!({
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa": {
                "request": {"operation_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "member_id":"member", "expected_roster_revision":1, "target_issue_number":42},
                "source": {"assignment":{"reservation_id":"reservation", "session_id":"session", "issue_number":41, "spec_number":null, "delivery_id":"delivery", "host_id":"host", "host_pid":1, "host_started_at":2}, "window_id":"window", "claim_id":"claim", "confirmation":{"window_id":"window", "claim_id":"claim", "delivery_id":"delivery", "confirmed_at":"2026-10-25T00:00:00Z"}},
                "stage":"requested", "source_input_revision":7
            }
        });
        value
    }

    fn fixture() -> (IssueMonitorPrefs, AgentSwitchRequest, AgentSwitchIntent) {
        let mut prefs: IssueMonitorPrefs = serde_json::from_value(stored_prefs()).unwrap();
        let intent = prefs.agent_switch_intents.pop_first().unwrap().1;
        prefs.agent_roster = Some(crate::agent_names::AgentRoster {
            schema_version: 1,
            revision: 1,
            members: vec![crate::agent_names::AgentMember {
                agent_id: None,
                agent_member_id: "member".into(),
                name: "Member".into(),
                enabled: true,
                created_at: "2026-10-25T00:00:00Z".into(),
                origin_session_id: None,
                last_issue_number: Some(41),
                last_spec_number: None,
                last_owner_project_key: None,
                assignment: Some(intent.source.assignment.clone()),
            }],
        });
        prefs.launched_issues.push(IssueMonitorLaunchedIssue {
            issue_number: 41,
            window_id: "window".into(),
        });
        prefs.launch_bindings.insert("window".into(), 41);
        prefs.launched_claims.insert(41, "claim".into());
        prefs
            .launch_confirmations
            .insert(41, intent.source.confirmation.clone());
        prefs.terminal_queues.insert(
            "host".into(),
            IssueMonitorTerminalQueue {
                entries: vec![IssueMonitorTerminalQueueEntry {
                    number: 42,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        prefs
            .agent_issue_schedule
            .reservations
            .push(AgentIssueReservation {
                host_id: "host".into(),
                agent_member_id: "member".into(),
                issue_number: 42,
                reason: AgentScheduleReason::QueueOrder,
            });
        (prefs, intent.request.clone(), intent)
    }

    fn stop_fixture() -> IssueMonitorRuntimeStopRequest {
        serde_json::from_value(serde_json::json!({
            "issue_number":41, "window_id":"window", "session_id":"session",
            "host_pid":1, "host_started_at":2, "runtime_incarnation":3,
            "child_pid":4, "child_started_at":5, "requested_at":"2026-10-25T00:00:00Z",
            "execution_identity": {"session_id":"session", "worktree_path":"/repo", "branch":"issue-41",
                "agent_id":{"type":"Codex"}, "linked_issue_number":41,
                "execution_binding":{"schema_version":1, "session_id":"session", "repo_hash":"repo",
                    "owner_kind":"issue", "owner_number":41, "identity":{"generation_id":"generation", "binding_id":"binding", "ledger_head_hash":"ledger"}, "capability_generation":1}}
        })).unwrap()
    }

    #[test]
    fn foreign_project_owner_numbers_do_not_block_local_switch() {
        let (mut prefs, request, mut intent) = fixture();
        prefs.agent_registry_project_key = Some("bbbbbbbbbbbbbbbb".into());
        let roster = prefs.agent_roster.as_mut().unwrap();
        roster.schema_version = 2;
        roster.members[0].assignment.as_mut().unwrap().project_key =
            Some("bbbbbbbbbbbbbbbb".into());
        intent.source.assignment.project_key = Some("bbbbbbbbbbbbbbbb".into());
        let mut foreign = roster.members[0].clone();
        foreign.agent_member_id = "foreign-member".into();
        foreign.name = "Foreign member".into();
        let assignment = foreign.assignment.as_mut().unwrap();
        assignment.project_key = Some("aaaaaaaaaaaaaaaa".into());
        assignment.reservation_id = "foreign-reservation".into();
        assignment.session_id = Some("foreign-session".into());
        assignment.delivery_id = Some("foreign-delivery".into());
        roster.members.push(foreign);
        assert!(
            prefs.agent_switch_source_unchanged(&intent, "host"),
            "the same source number in another project is not an ambiguous local source"
        );

        prefs.agent_roster.as_mut().unwrap().members[1]
            .assignment
            .as_mut()
            .unwrap()
            .issue_number = Some(42);
        let foreign_assignment = prefs.agent_roster.as_ref().unwrap().members[1]
            .assignment
            .clone();
        assert!(
            prefs.agent_switch_target_available(&intent, "host"),
            "another project's Issue 42 does not occupy the local switch target"
        );
        assert!(prefs
            .clone()
            .request_agent_switch(request.clone(), "host")
            .is_ok());
        assert_eq!(
            prefs.agent_roster.as_ref().unwrap().members[1].assignment,
            foreign_assignment
        );

        let mut local_target_busy = prefs.clone();
        local_target_busy.agent_roster.as_mut().unwrap().members[1]
            .assignment
            .as_mut()
            .unwrap()
            .project_key = Some("bbbbbbbbbbbbbbbb".into());
        assert!(
            !local_target_busy.agent_switch_target_available(&intent, "host"),
            "a true local Issue 42 assignment still blocks the target"
        );
        let mut foreign_source = prefs;
        foreign_source.agent_roster.as_mut().unwrap().members[0]
            .assignment
            .as_mut()
            .unwrap()
            .project_key = Some("aaaaaaaaaaaaaaaa".into());
        assert!(
            foreign_source
                .request_agent_switch(request, "host")
                .is_err(),
            "the current project cannot switch a member assigned to another project"
        );
    }

    #[test]
    fn agent_switch_intents_survive_preferences_and_state_roundtrip() {
        let value = stored_prefs();
        let expected = value["agent_switch_intents"].clone();
        let prefs: IssueMonitorPrefs = serde_json::from_value(value).unwrap();
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        monitor.rebase_daemon_driver_prefs(&monitor.prefs());
        monitor.rebase_gui_observer_prefs(&monitor.prefs());
        assert_eq!(
            serde_json::to_value(monitor.prefs()).unwrap()["agent_switch_intents"],
            expected
        );
    }

    #[test]
    fn agent_switch_request_is_idempotent_and_conflicts_preserve_current() {
        let (mut prefs, request, _) = fixture();
        let before = prefs.clone();
        let intent = prefs.request_agent_switch(request.clone(), "host").unwrap();
        assert_eq!(intent.stage, AgentSwitchStage::Requested);
        assert_eq!(
            prefs.request_agent_switch(request.clone(), "host").unwrap(),
            intent
        );
        let mut changed = request.clone();
        changed.target_issue_number = 43;
        assert!(prefs.request_agent_switch(changed, "host").is_err());
        let mut another = request;
        another.operation_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into();
        assert!(prefs.request_agent_switch(another, "host").is_err());
        prefs.agent_switch_intents.clear();
        assert_eq!(
            prefs, before,
            "request cannot mutate launch/claim/queue state"
        );
    }

    #[test]
    fn agent_switch_request_requires_canonical_uuid() {
        let (prefs, request, _) = fixture();
        for noncanonical in [
            request.operation_id.to_uppercase(),
            request.operation_id.replace('-', ""),
        ] {
            let mut request = request.clone();
            request.operation_id = noncanonical;
            let mut unchanged = prefs.clone();
            assert!(unchanged.request_agent_switch(request, "host").is_err());
            assert_eq!(unchanged, prefs);
        }
    }

    #[test]
    fn agent_switch_target_planning_uses_normal_candidate_readiness_without_mutation() {
        let (mut prefs, _, mut intent) = fixture();
        let host = crate::process::current_hostname();
        prefs.enabled = true;
        prefs.launch_profile =
            Some(serde_json::from_value(serde_json::json!({"agent_id":"codex"})).unwrap());
        prefs.agent_roster.as_mut().unwrap().members[0]
            .assignment
            .as_mut()
            .unwrap()
            .host_id = host.clone();
        prefs.agent_issue_schedule.reservations[0].host_id = host.clone();
        let queue = prefs.terminal_queues.remove("host").unwrap();
        prefs.terminal_queues.insert(host.clone(), queue);
        intent.source.assignment.host_id = host;
        intent.stage = AgentSwitchStage::StoppingCurrent;
        prefs
            .agent_switch_intents
            .insert(intent.request.operation_id.clone(), intent.clone());
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        monitor.set_source_repository(Some(("fixture", "queue")));
        let issue = IssueMonitorIssue {
            number: 42,
            author_login: Some("fixture".into()),
            title: "Ready target".into(),
            labels: vec!["gwt-spec".into(), "gwt-queue".into()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: IssueMonitorReadiness::Ready,
            updated_at: Some("2026-10-26T00:00:00Z".into()),
        };
        monitor.record_candidate(issue.clone());
        monitor.set_agent_capacity(AgentCapacity {
            measurement_complete: true,
            machine_budget: Some(2),
            recommended_worker_limit: 1,
            ..AgentCapacity::default()
        });
        assert_eq!(monitor.effective_max_active_agents(), 1);
        assert_eq!(monitor.occupied_slot_count(), 1);
        let before = serde_json::to_value(&monitor).unwrap();
        assert!(
            monitor.agent_switch_target_exclusions(&intent).is_empty(),
            "planning ignores only the saved source member assignment and switch pin"
        );
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before);
        let exclusions = |candidate: &IssueMonitorState| {
            let before = serde_json::to_value(candidate).unwrap();
            let reasons = candidate.agent_switch_target_exclusions(&intent);
            assert_eq!(
                serde_json::to_value(candidate).unwrap(),
                before,
                "planning cannot mutate state or pending effects"
            );
            reasons
        };
        let mut holds = Vec::new();
        let mut zero_capacity = monitor.clone();
        zero_capacity.set_agent_capacity(AgentCapacity {
            measurement_complete: true,
            ..AgentCapacity::default()
        });
        assert_eq!(zero_capacity.effective_max_active_agents(), 0);
        holds.push(("zero capacity", !exclusions(&zero_capacity).is_empty()));
        let mut crowded = monitor.clone();
        assert!(crowded.record_monitor_runtime_windows(
            101,
            1001,
            BTreeMap::from([(41, 1), (43, 1)]),
            BTreeMap::from([("window".into(), 41), ("other-runtime".into(), 43)]),
            "2026-10-26T00:00:00Z",
        ));
        assert_eq!(crowded.occupied_slot_count(), 2);
        holds.push(("other occupied slot", !exclusions(&crowded).is_empty()));
        let mut manual = monitor.clone();
        manual.apply_machine_policy(&crate::machine_agent_settings::Policy {
            mode: IssueMonitorMaxActiveMode::Manual,
            manual_limit: Some(1),
            last_change: None,
        });
        assert!(
            exclusions(&manual).is_empty(),
            "Manual capacity one allows replacing its exact one source slot"
        );
        let mut missing_profile = monitor.clone();
        missing_profile.launch_profiles.clear();
        holds.push(("missing profile", !exclusions(&missing_profile).is_empty()));
        let mut drain = monitor.clone();
        drain.set_update_drain(
            IssueMonitorUpdateDrainReason::Manual,
            "99.0.0",
            "2026-10-26T00:00:00Z",
        );
        holds.push(("update drain", !exclusions(&drain).is_empty()));
        let mut quota = monitor.clone();
        quota
            .provider_quota_holds
            .insert("codex".into(), "9999-01-01T00:00:00Z".into());
        holds.push(("provider quota", !exclusions(&quota).is_empty()));
        let mut unsupported = monitor.clone();
        unsupported.launch_profiles[0].agent_id = "openclaw".into();
        holds.push((
            "permission incompatible",
            !exclusions(&unsupported).is_empty(),
        ));
        let mut automatic = monitor.clone();
        automatic.launch_auto = true;
        automatic.launch_tiers = vec![
            automatic.launch_profiles.clone(),
            vec![serde_json::from_value(serde_json::json!({"agent_id":"claude"})).unwrap()],
        ];
        automatic.launch_profiles.clear();
        automatic
            .provider_quota_holds
            .insert("claude".into(), "9999-01-01T00:00:00Z".into());
        holds.push(("SPEC tier unavailable", !exclusions(&automatic).is_empty()));
        automatic.provider_quota_holds.clear();
        assert!(
            exclusions(&automatic).is_empty(),
            "a healthy automatic tier is usable without a legacy profile pool"
        );
        let mut closed = monitor.clone();
        let mut closed_issue = issue.clone();
        closed_issue.state = IssueMonitorIssueState::Closed;
        closed.record_candidate(closed_issue);
        holds.push(("current closed revision", !exclusions(&closed).is_empty()));
        let mut durable_closed = monitor.clone();
        durable_closed.closure_records.insert(
            42,
            IssueClosureRecord {
                issue_number: 42,
                generation: 1,
                state: IssueClosureState::Closed,
                evidence: IssueClosureEvidence::ExplicitRevision,
                issue_updated_at: Some("2026-10-26T00:00:01Z".into()),
                reopened_after_close: false,
            },
        );
        holds.push(("durable closed", !exclusions(&durable_closed).is_empty()));
        durable_closed.transition_issue_closure(
            42,
            IssueClosureState::Reopened,
            IssueClosureEvidence::ExplicitRevision,
            Some("2026-10-26T00:00:02Z".into()),
        );
        assert!(
            exclusions(&durable_closed).is_empty(),
            "a canonical newer Reopened revision lifts the durable close"
        );
        assert!(
            holds.iter().all(|(_, held)| *held),
            "normal target holds were missed: {holds:?}"
        );
        let mut not_ready = monitor.clone();
        let mut changed = issue.clone();
        changed.readiness = IssueMonitorReadiness::NotReady;
        not_ready.record_candidate(changed);
        assert!(!not_ready.agent_switch_target_exclusions(&intent).is_empty());
        assert!(monitor.record_blocked_by_claim(
            issue,
            "another-holder",
            "2026-10-27T00:00:00Z",
            Some("foreign-claim")
        ));
        assert!(!monitor.agent_switch_target_exclusions(&intent).is_empty());
        let mut disabled = not_ready;
        disabled.config.enabled = false;
        assert!(disabled
            .agent_switch_target_exclusions(&intent)
            .iter()
            .any(|reason| reason.contains("disabled")));
    }

    #[test]
    fn agent_switch_rejects_stale_source_target_and_caller_identity() {
        let (prefs, request, _) = fixture();
        assert!(prefs
            .clone()
            .request_agent_switch(request.clone(), "host")
            .is_ok());
        let changes: [fn(&mut IssueMonitorPrefs); 7] = [
            |p| p.agent_roster.as_mut().unwrap().revision += 1,
            |p| p.agent_roster.as_mut().unwrap().members[0].enabled = false,
            |p| {
                p.agent_roster.as_mut().unwrap().members[0]
                    .assignment
                    .as_mut()
                    .unwrap()
                    .session_id = None
            },
            |p| p.launch_bindings.clear(),
            |p| p.launch_confirmations.get_mut(&41).unwrap().delivery_id = Some("other".into()),
            |p| p.terminal_queues.clear(),
            |p| p.agent_issue_schedule.reservations[0].agent_member_id = "other".into(),
        ];
        for change in changes {
            let mut changed = prefs.clone();
            change(&mut changed);
            let before = changed.clone();
            assert!(changed
                .request_agent_switch(request.clone(), "host")
                .is_err());
            assert_eq!(changed, before);
        }
        assert!(prefs
            .clone()
            .request_agent_switch(request.clone(), "remote")
            .is_err());
        let mut raw = serde_json::to_value(request).unwrap();
        raw["session_id"] = "caller-session".into();
        assert!(serde_json::from_value::<AgentSwitchRequest>(raw).is_err());
    }

    #[test]
    fn agent_switch_source_matches_full_assignment_and_launch_generation() {
        let (prefs, _, intent) = fixture();
        assert!(prefs.agent_switch_source_unchanged(&intent, "host"));
        assert!(prefs.agent_switch_target_available(&intent, "host"));
        let mut changed = prefs.clone();
        changed.agent_roster.as_mut().unwrap().members[0]
            .assignment
            .as_mut()
            .unwrap()
            .reservation_id = "successor".into();
        assert!(!changed.agent_switch_source_unchanged(&intent, "host"));
        let mut changed = prefs.clone();
        changed.launched_claims.insert(41, "successor".into());
        assert!(!changed.agent_switch_source_unchanged(&intent, "host"));
        let mut changed = prefs;
        changed.launched_issues.push(IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "target".into(),
        });
        assert!(!changed.agent_switch_target_available(&intent, "host"));
    }

    #[test]
    fn agent_switch_cancels_before_stop_and_never_replays_after_restart() {
        let (mut prefs, request, mut intent) = fixture();
        let operation = request.operation_id.clone();
        prefs.request_agent_switch(request, "host").unwrap();
        assert_eq!(
            prefs.cancel_agent_switch(&operation).unwrap().stage,
            AgentSwitchStage::Cancelled
        );
        assert_eq!(
            prefs.cancel_agent_switch(&operation).unwrap().stage,
            AgentSwitchStage::Cancelled
        );
        for stage in [
            AgentSwitchStage::Requested,
            AgentSwitchStage::Checkpointing,
            AgentSwitchStage::CheckpointVerified,
            AgentSwitchStage::StoppingCurrent,
            AgentSwitchStage::LaunchingNext,
        ] {
            intent.stage = stage;
            prefs
                .agent_switch_intents
                .insert(operation.clone(), intent.clone());
            assert_eq!(
                prefs.recover_agent_switches_after_restart("remote"),
                Vec::<String>::new()
            );
            assert_eq!(
                prefs.recover_agent_switches_after_restart("host"),
                vec![operation.clone()]
            );
            let recovered = &prefs.agent_switch_intents[&operation];
            if matches!(
                stage,
                AgentSwitchStage::StoppingCurrent | AgentSwitchStage::LaunchingNext
            ) {
                assert_eq!(recovered.stage, AgentSwitchStage::Unknown);
                assert!(prefs.cancel_agent_switch(&operation).is_err());
            } else {
                assert_eq!(recovered.stage, AgentSwitchStage::Cancelled);
            }
            assert!(prefs
                .recover_agent_switches_after_restart("host")
                .is_empty());
        }
    }

    #[test]
    fn agent_switch_read_echo_is_transient_and_legacy_read_stays_compatible() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("project");
        fs::create_dir_all(&root).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let prefs = IssueMonitorPrefs::default();
        crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
        let before = fs::read(&path).unwrap();
        let action: AgentSwitchAction =
            serde_json::from_value(serde_json::json!({"kind":"read", "request_id":"read-one"}))
                .expect("correlated read");
        let read = mutate_project_agent_switch(&root, &gwt_core::paths::gwt_sessions_dir(), action)
            .unwrap();
        assert_eq!(
            serde_json::to_value(read).unwrap()["request_id"],
            "read-one"
        );
        let action: AgentSwitchAction =
            serde_json::from_value(serde_json::json!({"kind":"read"})).unwrap();
        let read = mutate_project_agent_switch(&root, &gwt_core::paths::gwt_sessions_dir(), action)
            .unwrap();
        assert!(serde_json::to_value(read)
            .unwrap()
            .get("request_id")
            .is_none());
        assert!(serde_json::to_value(prefs.agent_switch_snapshot("project"))
            .unwrap()
            .get("request_id")
            .is_none());
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "read correlation is never persisted"
        );
    }

    #[test]
    fn agent_switch_snapshot_orders_creation_and_hides_runtime_proof() {
        let (mut prefs, _, mut intent) = fixture();
        intent.created_at = "2026-10-25T00:00:00Z".into();
        prefs
            .agent_switch_intents
            .insert(intent.request.operation_id.clone(), intent.clone());
        intent.request.operation_id = "00000000-0000-4000-8000-000000000000".into();
        intent.created_at = "2026-10-25T00:00:01Z".into();
        intent.stage = AgentSwitchStage::Unknown;
        intent.runtime_exit_confirmed = false;
        intent.source.assignment.host_id = crate::process::current_hostname();
        intent.checkpoint = Some(AgentSwitchCheckpoint {
            verified_head: "a".repeat(40),
            work_id: "saved-work".into(),
            board_entry_id: "board".into(),
        });
        prefs
            .agent_switch_intents
            .insert(intent.request.operation_id.clone(), intent.clone());
        let view = prefs.agent_switch_snapshot("project");
        assert_eq!(view.project_key, "project");
        assert_eq!(
            view.intents.last().unwrap().operation_id,
            intent.request.operation_id
        );
        assert_eq!(
            view.intents.last().unwrap().saved_work_id.as_deref(),
            Some("saved-work")
        );
        assert!(view.intents.last().unwrap().can_resume_saved);
        let raw = serde_json::to_string(&view).unwrap();
        assert!(
            !raw.contains("reservation")
                && !raw.contains("claim")
                && !raw.contains("runtime_stop")
                && !raw.contains("source_input_revision")
        );
        prefs
            .agent_switch_intents
            .get_mut(&intent.request.operation_id)
            .unwrap()
            .stage = AgentSwitchStage::LaunchingNext;
        assert!(
            prefs
                .agent_switch_snapshot("project")
                .intents
                .last()
                .unwrap()
                .can_resume_saved,
            "saved work recovery stays available while a target is waiting"
        );
    }

    #[test]
    fn agent_switch_checkpoint_requires_original_runtime_and_ordered_stages() {
        let (mut prefs, _, mut intent) = fixture();
        let operation = intent.request.operation_id.clone();
        let stop = stop_fixture();
        let receipt = AgentSwitchCheckpoint {
            verified_head: "a".repeat(40),
            work_id: "work".into(),
            board_entry_id: "board".into(),
        };
        intent.runtime_stop = Some(stop.clone());
        prefs.agent_switch_intents.insert(operation.clone(), intent);
        assert!(prefs
            .transition_agent_switch(&operation, AgentSwitchStage::StoppingCurrent, None)
            .is_err());
        assert_eq!(
            prefs
                .transition_agent_switch(&operation, AgentSwitchStage::Checkpointing, None)
                .unwrap()
                .stage,
            AgentSwitchStage::Checkpointing
        );
        let before = prefs.clone();
        let mut successor = stop.clone();
        successor.runtime_incarnation += 1;
        assert!(prefs
            .verify_agent_switch_checkpoint(&operation, receipt.clone(), successor, "host")
            .is_err());
        assert_eq!(prefs, before);
        assert_eq!(
            prefs
                .verify_agent_switch_checkpoint(&operation, receipt.clone(), stop.clone(), "host")
                .unwrap()
                .checkpoint,
            Some(receipt.clone())
        );
        let verified = prefs.clone();
        assert_eq!(
            prefs
                .verify_agent_switch_checkpoint(&operation, receipt.clone(), stop.clone(), "host")
                .unwrap(),
            verified.agent_switch_intents[&operation]
        );
        assert_eq!(prefs, verified, "an identical ACK retry is readback only");
        let mut changed = receipt;
        changed.work_id = "other-work".into();
        assert!(prefs
            .verify_agent_switch_checkpoint(&operation, changed, stop.clone(), "host")
            .is_err());
        let mut successor = stop.clone();
        successor.runtime_incarnation += 1;
        assert!(prefs
            .verify_agent_switch_checkpoint(
                &operation,
                verified.agent_switch_intents[&operation]
                    .checkpoint
                    .clone()
                    .unwrap(),
                successor,
                "host"
            )
            .is_err());
        assert_eq!(prefs, verified);
        for next in [
            AgentSwitchStage::StoppingCurrent,
            AgentSwitchStage::LaunchingNext,
            AgentSwitchStage::Completed,
        ] {
            if next == AgentSwitchStage::LaunchingNext {
                assert!(
                    prefs
                        .transition_agent_switch(&operation, next, None)
                        .is_err(),
                    "stop request acknowledgement is not physical exit confirmation"
                );
                prefs
                    .agent_switch_intents
                    .get_mut(&operation)
                    .unwrap()
                    .runtime_exit_confirmed = true;
            }
            assert_eq!(
                prefs
                    .transition_agent_switch(&operation, next, None)
                    .unwrap()
                    .stage,
                next
            );
            assert!(prefs.cancel_agent_switch(&operation).is_err());
        }
        assert_eq!(
            prefs.monitor_runtime_stop_requests, before.monitor_runtime_stop_requests,
            "intent transitions do not enqueue stop effects"
        );
        assert_eq!(prefs.launched_issues, before.launched_issues);
    }

    #[test]
    fn agent_switch_service_old_uuid_retry_cannot_replace_runtime_proof() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        let (mut prefs, request, mut intent) = fixture();
        intent.runtime_stop = Some(stop_fixture());
        prefs
            .agent_switch_intents
            .insert(request.operation_id.clone(), intent.clone());
        // A retry must preserve the old proof even after its source is gone.
        prefs.agent_roster.as_mut().unwrap().members[0].assignment = None;
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        save_issue_monitor_prefs(&path, &prefs).unwrap();
        let action: AgentSwitchAction = serde_json::from_value(serde_json::json!({
            "kind":"request", "operation_id":request.operation_id, "member_id":request.member_id,
            "expected_roster_revision":request.expected_roster_revision, "target_issue_number":request.target_issue_number,
        })).unwrap();
        let snapshot =
            mutate_project_agent_switch(&root, &gwt_core::paths::gwt_sessions_dir(), action)
                .unwrap();
        assert_eq!(
            snapshot.project_key,
            gwt_core::paths::project_scope_hash(&root).to_string()
        );
        assert_eq!(
            load_issue_monitor_prefs(&path)
                .unwrap()
                .agent_switch_intents[&intent.request.operation_id],
            intent
        );
        let before = fs::read(&path).unwrap();
        assert_eq!(
            mutate_project_agent_switch(
                &root,
                &gwt_core::paths::gwt_sessions_dir(),
                AgentSwitchAction::Read { request_id: None }
            )
            .unwrap(),
            snapshot
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "read never reconciles or writes preferences"
        );
    }
}
