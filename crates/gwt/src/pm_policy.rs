//! Read-only goal and roadmap diagnostics (SPEC #3431 T-113a/T-114a).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{
    issue_monitor::{IssueMonitorDependencyStatus, IssueMonitorInboxCoverage},
    pm_planning::{
        Goal, PlanningAction, PlanningChangeImpact, PlanningFieldChange, PmPlanningState,
        RoadmapHorizon, RoadmapItem,
    },
    IssueMonitorAgentStatus, IssueMonitorStatusSource, MonitorInboxState,
};

/// Compare the locked prior record with the proposed planning-only upsert.
/// Capture only changed fields; legacy values need redaction too.
pub(crate) fn assess_planning_change(
    state: &PmPlanningState,
    action: &PlanningAction,
) -> serde_json::Result<Option<PlanningChangeImpact>> {
    use serde_json::Value;

    fn redact(value: &mut Value) {
        match value {
            Value::String(text) => *text = gwt_core::error_ledger::redact_credentials(text),
            Value::Array(values) => values.iter_mut().for_each(redact),
            Value::Object(values) => values.values_mut().for_each(redact),
            _ => {}
        }
    }

    let (before, after, milestone_id, goal_id) = match action {
        PlanningAction::Goal(value) => (
            state
                .goals
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            None,
            Some(value.id.as_str()),
        ),
        PlanningAction::Milestone(value) => (
            state
                .milestones
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            Some(value.id.as_str()),
            None,
        ),
        PlanningAction::Link(value) => (
            state
                .roadmap_items
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            None,
            None,
        ),
        PlanningAction::Risk(value) => (
            state
                .risks
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            None,
            None,
        ),
        PlanningAction::Decision(value) => (
            state
                .decisions
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            None,
            None,
        ),
        PlanningAction::Policy(value) => (
            state
                .policies
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            None,
            None,
        ),
        PlanningAction::Recurring(value) => (
            state
                .recurring_items
                .iter()
                .find(|item| item.id == value.id)
                .map(serde_json::to_value)
                .transpose()?,
            serde_json::to_value(value)?,
            None,
            None,
        ),
    };
    let mut changes = std::collections::BTreeMap::new();
    if let Value::Object(fields) = &after {
        for (name, new) in fields {
            let old = before.as_ref().and_then(|record| record.get(name));
            if old != Some(new) {
                let mut field = PlanningFieldChange {
                    before: old.cloned().unwrap_or(Value::Null),
                    after: new.clone(),
                };
                // The recurring setting key is distinct from the request key;
                // retain its change without duplicating either raw key in audit.
                let field_name = if matches!(action, PlanningAction::Recurring(_))
                    && name == "idempotency_key"
                {
                    for value in [&mut field.before, &mut field.after] {
                        if let Value::String(key) = value {
                            *key = gwt_core::pm_authorization::digest(key.as_bytes());
                        }
                    }
                    "idempotency_key_digest"
                } else {
                    name.as_str()
                };
                redact(&mut field.before);
                redact(&mut field.after);
                changes.insert(field_name.to_owned(), field);
            }
        }
    }
    let mut issues = BTreeSet::new();
    if !changes.is_empty() {
        for record in before.iter().chain(std::iter::once(&after)) {
            if let Some(Value::Array(numbers)) = record.get("issues") {
                issues.extend(numbers.iter().filter_map(Value::as_u64));
            }
        }
        for item in &state.roadmap_items {
            if milestone_id.is_some_and(|id| item.milestone_id.as_deref() == Some(id))
                || goal_id.is_some_and(|id| item.goal_id.as_deref() == Some(id))
            {
                issues.extend(item.issues.iter().copied());
            }
        }
    }
    let mut missing_inputs = vec!["dependency_graph".into(), "active_owner_handoff".into()];
    if matches!(
        action,
        PlanningAction::Risk(_)
            | PlanningAction::Decision(_)
            | PlanningAction::Policy(_)
            | PlanningAction::Recurring(_)
    ) {
        missing_inputs.push("direct_issue_mapping".into());
    }
    Ok(Some(PlanningChangeImpact {
        record_created: before.is_none(),
        changes,
        directly_affected_issues: issues.into_iter().collect(),
        missing_inputs,
    }))
}

#[derive(Debug, Serialize)]
pub struct PlanningAssessment {
    pub planning_revision: u64,
    pub monitor: MonitorEvidence,
    pub capacity: CapacityObservation,
    pub missing_inputs: Vec<&'static str>,
    pub dependency_observations: Option<Vec<DependencyObservation>>,
    pub dependency_impacts: Option<Vec<DependencyImpactObservation>>,
    pub issue_traces: Vec<IssueTrace>,
    pub roadmap: RoadmapHorizons,
    pub roadmap_items: Vec<RoadmapDiagnostic>,
    pub goals: Vec<GoalDiagnostic>,
    pub milestones: Vec<MilestoneDiagnostic>,
    pub risks: Vec<RiskMetadataDiagnostic>,
    pub decisions: Vec<DecisionMetadataDiagnostic>,
    pub decision_revisions: Vec<DecisionRevisionObservation>,
    pub forecasts: Vec<ForecastObservation>,
    pub review_dates: Vec<ReviewDateObservation>,
    pub recurring_runs: Vec<RecurringRunObservation>,
}

/// Supplied Monitor diagnostics are not a complete graph or permission to launch.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyObservation {
    pub issue_number: u64,
    pub state: MonitorInboxState,
    pub dependency: Option<IssueMonitorDependencyStatus>,
    pub missing_inputs: Vec<&'static str>,
}

/// Direct dependents of a supplied blocker, not a complete impact graph.
#[derive(Debug, Serialize)]
pub struct DependencyImpactObservation {
    pub issue_number: u64,
    pub dependent_issues: Vec<u64>,
}

/// Authored metadata presence is not a risk assessment or action permission.
#[derive(Debug, Serialize)]
pub struct RiskMetadataDiagnostic {
    pub id: String,
    pub missing_inputs: Vec<&'static str>,
}

/// Authored metadata presence is not decision quality or approval.
#[derive(Debug, Serialize)]
pub struct DecisionMetadataDiagnostic {
    pub id: String,
    pub missing_inputs: Vec<&'static str>,
}

/// Numeric revision distance is not evidence of decision freshness or approval.
#[derive(Debug, Serialize)]
pub struct DecisionRevisionObservation {
    pub id: String,
    pub source_revision: Option<u64>,
    pub revisions_since_source: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

/// Saved forecast claims are not independently verified history or confidence.
#[derive(Debug, Serialize)]
pub struct ForecastObservation {
    pub id: String,
    pub source_revision: u64,
    pub sample_size: u64,
    pub reported_estimate: Option<String>,
    pub reported_confidence: Option<String>,
    pub reported_missing_inputs: Vec<String>,
    pub revisions_since_source: Option<u64>,
    pub confidence: &'static str,
    pub missing_inputs: Vec<&'static str>,
}

/// A saved next-run timestamp reaching the scan is not execution evidence.
#[derive(Debug, Serialize)]
pub struct RecurringRunObservation {
    pub id: String,
    pub next_run: Option<String>,
    pub due_at_scan: Option<bool>,
    pub configuration_missing_inputs: Vec<&'static str>,
    pub missing_inputs: Vec<&'static str>,
}

fn parse_observed_instant(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    // Chrono silently discards fractional digits beyond nanoseconds.
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .filter(|_| {
            value.split_once('.').is_none_or(|(_, fraction)| {
                fraction.bytes().take_while(u8::is_ascii_digit).count() <= 9
            })
        })
}

fn observe_recurring_runs(
    state: &PmPlanningState,
    scan: Option<&str>,
) -> Vec<RecurringRunObservation> {
    let scan = scan.and_then(parse_observed_instant);
    state
        .recurring_items
        .iter()
        .map(|item| {
            let mut configuration_missing_inputs = Vec::new();
            if item.id.trim().is_empty() {
                configuration_missing_inputs.push("id");
            } else if state
                .recurring_items
                .iter()
                .filter(|other| other.id == item.id)
                .count()
                > 1
            {
                configuration_missing_inputs.push("unique_recurring_id");
            }
            for (name, value) in [
                ("cadence", &item.cadence),
                ("owner", &item.owner),
                ("miss_policy", &item.miss_policy),
                ("idempotency_key", &item.idempotency_key),
            ] {
                if value.trim().is_empty() {
                    configuration_missing_inputs.push(name);
                }
            }
            if !item.idempotency_key.trim().is_empty()
                && state
                    .recurring_items
                    .iter()
                    .filter(|other| other.idempotency_key == item.idempotency_key)
                    .count()
                    > 1
            {
                configuration_missing_inputs.push("unique_idempotency_key");
            }
            let next_run = item.next_run.as_deref().and_then(parse_observed_instant);
            let mut missing_inputs = Vec::new();
            if next_run.is_none() {
                missing_inputs.push("next_run");
            }
            if scan.is_none() {
                missing_inputs.push("monitor_scan_time");
            }
            RecurringRunObservation {
                id: item.id.clone(),
                next_run: item.next_run.clone(),
                due_at_scan: scan.zip(next_run).map(|(scan, next_run)| scan >= next_run),
                configuration_missing_inputs,
                missing_inputs,
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct MonitorEvidence {
    pub source: Option<IssueMonitorStatusSource>,
    pub last_scan_at: Option<String>,
    pub scan_aborted: Option<bool>,
    pub active_launches_incomplete: bool,
    pub inbox_coverage: Option<IssueMonitorInboxCoverage>,
}

/// Supplied admission facts, sharing the assessment's Monitor provenance.
/// Listed owner sets cannot establish physical occupancy or full WIP.
#[derive(Debug, Serialize)]
pub struct CapacityObservation {
    pub implementation_launches: Option<Vec<u64>>,
    pub review_issues: Option<Vec<u64>>,
    pub pending_claim_issues: Option<Vec<u64>>,
    pub needs_human_issues: Option<Vec<u64>>,
    pub occupied_slot_count: Option<usize>,
    pub max_active: Option<usize>,
    pub observed_work_issues: Vec<u64>,
    pub launch_activity: Option<Vec<LaunchActivityObservation>>,
    pub claim_blocks: Option<Vec<ClaimBlockObservation>>,
    pub declared_waits: Option<Vec<DeclaredWaitObservation>>,
    pub retry_deadlines: Option<Vec<RetryDeadlineObservation>>,
    pub wip_limits: Vec<WipLimitObservation>,
    pub idle_windows: Option<Vec<IdleWindowObservation>>,
    pub terminal_queue: Option<Vec<QueueResidencyObservation>>,
    pub missing_inputs: Vec<&'static str>,
}

/// Host queue membership at a supplied scan, not execution waiting time.
#[derive(Debug, Serialize)]
pub struct QueueResidencyObservation {
    pub issue_number: u64,
    pub queued_at: String,
    pub age_seconds_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
    pub age_limits: Vec<QueueAgeLimitObservation>,
}

/// Saved thresholds compare host membership age, never canonical starvation.
#[derive(Debug, Serialize)]
pub struct QueueAgeLimitObservation {
    pub id: String,
    pub version: u64,
    pub starvation_after_seconds: Option<u64>,
    pub assessment: &'static str,
    pub missing_inputs: Vec<&'static str>,
}

fn observe_terminal_queue(
    state: &PmPlanningState,
    status: &IssueMonitorAgentStatus,
    monitor_missing_inputs: &[&'static str],
) -> Option<Vec<QueueResidencyObservation>> {
    let view = status.gui_status.as_ref()?;
    let scan = status
        .last_scan_at
        .as_deref()
        .and_then(parse_observed_instant);
    Some(
        view.terminal_queue
            .iter()
            .map(|entry| {
                let queued = parse_observed_instant(&entry.queued_at);
                let mut missing_inputs = Vec::new();
                if queued.is_none() {
                    missing_inputs.push("queued_at");
                }
                if scan.is_none() {
                    missing_inputs.push("monitor_scan_time");
                }
                let age_seconds_at_scan = scan.zip(queued).and_then(|(scan, queued)| {
                    match scan.signed_duration_since(queued).to_std() {
                        Ok(duration) => Some(duration.as_secs()),
                        Err(_) => {
                            missing_inputs.push("queued_after_scan");
                            None
                        }
                    }
                });
                let age_limits = state
                    .policies
                    .iter()
                    .map(|policy| {
                        let mut missing = monitor_missing_inputs.to_vec();
                        missing.extend_from_slice(&missing_inputs);
                        if policy.id.trim().is_empty() {
                            missing.push("policy_id");
                        } else if state
                            .policies
                            .iter()
                            .filter(|other| other.id == policy.id)
                            .count()
                            != 1
                        {
                            missing.push("unique_policy_id");
                        }
                        if policy.starvation_after_seconds.is_none() {
                            missing.push("starvation_after_seconds");
                        }
                        let reached = missing.is_empty()
                            && age_seconds_at_scan
                                .zip(policy.starvation_after_seconds)
                                .is_some_and(|(age, threshold)| age >= threshold);
                        QueueAgeLimitObservation {
                            id: policy.id.clone(),
                            version: policy.version,
                            starvation_after_seconds: policy.starvation_after_seconds,
                            assessment: if reached {
                                "observed_age_threshold_reached"
                            } else {
                                "unknown"
                            },
                            missing_inputs: missing,
                        }
                    })
                    .collect();
                QueueResidencyObservation {
                    issue_number: entry.number,
                    queued_at: entry.queued_at.clone(),
                    age_seconds_at_scan,
                    missing_inputs,
                    age_limits,
                }
            })
            .collect(),
    )
}

/// Idle classification belongs to the Monitor; elapsed time grants no authority.
#[derive(Debug, Serialize)]
pub struct IdleWindowObservation {
    pub window_id: String,
    pub issue_number: Option<u64>,
    pub idle_kind: crate::issue_monitor::IssueMonitorIdleKind,
    pub bound: bool,
    pub pane_present: bool,
    pub idle_since: Option<String>,
    pub idle_seconds_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

fn observe_idle_windows(status: &IssueMonitorAgentStatus) -> Vec<IdleWindowObservation> {
    let scan = status
        .last_scan_at
        .as_deref()
        .and_then(parse_observed_instant);
    status
        .idle_windows
        .iter()
        .map(|window| {
            let since = parse_observed_instant(&window.idle_since);
            let mut missing_inputs = Vec::new();
            if scan.is_none() {
                missing_inputs.push("monitor_scan_time");
            }
            if since.is_none() {
                missing_inputs.push("idle_since");
            }
            let idle_seconds_at_scan = scan.zip(since).and_then(|(scan, since)| {
                match scan.signed_duration_since(since).to_std() {
                    Ok(duration) => Some(duration.as_secs()),
                    Err(_) => {
                        missing_inputs.push("idle_after_scan");
                        None
                    }
                }
            });
            IdleWindowObservation {
                window_id: window.window_id.clone(),
                issue_number: window.issue_number,
                idle_kind: window.idle_kind,
                bound: window.bound,
                pane_present: window.pane_present,
                idle_since: since.map(|value| value.to_rfc3339()),
                idle_seconds_at_scan,
                missing_inputs,
            }
        })
        .collect()
}

/// A saved-policy comparison, never a canonical WIP or admission verdict.
#[derive(Debug, Serialize)]
pub struct WipLimitObservation {
    pub id: String,
    pub version: u64,
    pub wip_limit: Option<u32>,
    pub observed_issue_count: Option<usize>,
    pub assessment: &'static str,
    pub missing_inputs: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct RetryDeadlineObservation {
    pub issue_number: u64,
    pub state: MonitorInboxState,
    pub retry_not_before: Option<String>,
    pub remaining_seconds_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

/// A recorded retry deadline does not establish whether a hold is in force.
fn observe_retry_deadlines(status: &IssueMonitorAgentStatus) -> Vec<RetryDeadlineObservation> {
    let scan = status
        .last_scan_at
        .as_deref()
        .and_then(parse_observed_instant);
    status
        .inbox
        .iter()
        .filter_map(|row| {
            row.retry_not_before
                .as_ref()
                .map(|deadline| (row, deadline))
        })
        .map(|(row, deadline)| {
            let expiry = parse_observed_instant(deadline);
            let mut missing_inputs = vec!["retry_hold_started_at"];
            if scan.is_none() {
                missing_inputs.push("monitor_scan_time");
            }
            if expiry.is_none() {
                missing_inputs.push("retry_not_before");
            }
            RetryDeadlineObservation {
                issue_number: row.issue_number,
                state: row.state,
                retry_not_before: expiry.map(|value| value.to_rfc3339()),
                remaining_seconds_at_scan: scan.zip(expiry).map(|(scan, expiry)| {
                    expiry
                        .signed_duration_since(scan)
                        .to_std()
                        .unwrap_or_default()
                        .as_secs()
                }),
                missing_inputs,
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct LaunchActivityObservation {
    pub issue_number: u64,
    pub state: MonitorInboxState,
    pub last_activity_at: Option<String>,
    pub inactivity_seconds_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct DeclaredWaitObservation {
    pub issue_number: u64,
    pub state: MonitorInboxState,
    pub in_force: bool,
    pub since: Option<String>,
    pub expires_at: Option<String>,
    pub elapsed_seconds_at_scan: Option<u64>,
    pub remaining_seconds_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

/// Protection is Monitor-owned; timing only describes its supplied snapshot.
fn observe_declared_waits(status: &IssueMonitorAgentStatus) -> Vec<DeclaredWaitObservation> {
    let scan = status
        .last_scan_at
        .as_deref()
        .and_then(parse_observed_instant);
    status
        .inbox
        .iter()
        .filter(|row| {
            matches!(
                row.state,
                MonitorInboxState::Launching | MonitorInboxState::Launched
            )
        })
        .filter_map(|row| row.waiting.as_ref().map(|wait| (row, wait)))
        .map(|(row, wait)| {
            let since = parse_observed_instant(&wait.since);
            let expiry = parse_observed_instant(&wait.expires_at);
            let mut missing_inputs = Vec::new();
            if scan.is_none() {
                missing_inputs.push("monitor_scan_time");
            }
            if since.is_none() {
                missing_inputs.push("wait_since");
            }
            if expiry.is_none() {
                missing_inputs.push("wait_expires_at");
            }
            let elapsed_seconds_at_scan = scan.zip(since).and_then(|(scan, since)| {
                match scan.signed_duration_since(since).to_std() {
                    Ok(duration) => Some(duration.as_secs()),
                    Err(_) => {
                        missing_inputs.push("wait_after_scan");
                        None
                    }
                }
            });
            let invalid_interval = since
                .zip(expiry)
                .is_some_and(|(since, expiry)| expiry < since);
            let remaining_seconds_at_scan = if invalid_interval {
                missing_inputs.push("wait_interval");
                None
            } else {
                scan.zip(expiry).map(|(scan, expiry)| {
                    expiry
                        .signed_duration_since(scan)
                        .to_std()
                        .unwrap_or_default()
                        .as_secs()
                })
            };
            DeclaredWaitObservation {
                issue_number: row.issue_number,
                state: row.state,
                in_force: wait.in_force,
                since: since.map(|value| value.to_rfc3339()),
                expires_at: expiry.map(|value| value.to_rfc3339()),
                elapsed_seconds_at_scan,
                remaining_seconds_at_scan,
                missing_inputs,
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct ClaimBlockObservation {
    pub issue_number: u64,
    pub claim_expires_at: Option<String>,
    pub remaining_seconds_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

/// A deadline is not a release receipt; the snapshot has no block start time.
fn observe_claim_blocks(status: &IssueMonitorAgentStatus) -> Vec<ClaimBlockObservation> {
    let scan = status
        .last_scan_at
        .as_deref()
        .and_then(parse_observed_instant);
    status
        .inbox
        .iter()
        .filter(|row| row.state == MonitorInboxState::BlockedByClaim)
        .map(|row| {
            let expiry = row
                .claim_expires_at
                .as_deref()
                .and_then(parse_observed_instant);
            let mut missing_inputs = vec!["claim_block_started_at"];
            if scan.is_none() {
                missing_inputs.push("monitor_scan_time");
            }
            if expiry.is_none() {
                missing_inputs.push("claim_expires_at");
            }
            let remaining_seconds_at_scan = scan.zip(expiry).map(|(scan, expiry)| {
                expiry
                    .signed_duration_since(scan)
                    .to_std()
                    .unwrap_or_default()
                    .as_secs()
            });
            ClaimBlockObservation {
                issue_number: row.issue_number,
                claim_expires_at: expiry.map(|value| value.to_rfc3339()),
                remaining_seconds_at_scan,
                missing_inputs,
            }
        })
        .collect()
}

/// Timing belongs to the supplied snapshot, never the reader's wall clock.
fn observe_launch_activity(status: &IssueMonitorAgentStatus) -> Vec<LaunchActivityObservation> {
    let scan = status
        .last_scan_at
        .as_deref()
        .and_then(parse_observed_instant);
    status
        .inbox
        .iter()
        .filter(|row| {
            matches!(
                row.state,
                MonitorInboxState::Launching | MonitorInboxState::Launched
            )
        })
        .map(|row| {
            let activity = row
                .last_activity_at
                .as_deref()
                .and_then(parse_observed_instant);
            let mut missing_inputs = Vec::new();
            if scan.is_none() {
                missing_inputs.push("monitor_scan_time");
            }
            if activity.is_none() {
                missing_inputs.push("last_activity_at");
            }
            let inactivity_seconds_at_scan = match (scan, activity) {
                (Some(scan), Some(activity)) => {
                    match scan.signed_duration_since(activity).to_std() {
                        Ok(duration) => Some(duration.as_secs()),
                        Err(_) => {
                            missing_inputs.push("activity_after_scan");
                            None
                        }
                    }
                }
                _ => None,
            };
            LaunchActivityObservation {
                issue_number: row.issue_number,
                state: row.state,
                last_activity_at: activity.map(|value| value.to_rfc3339()),
                inactivity_seconds_at_scan,
                missing_inputs,
            }
        })
        .collect()
}

fn observe_capacity(
    state: &PmPlanningState,
    monitor: Option<&IssueMonitorAgentStatus>,
    monitor_missing_inputs: &[&'static str],
) -> CapacityObservation {
    let implementation_launches = monitor.map(|status| status.active_launches.clone());
    let review_issues = monitor.map(|status| {
        status
            .review_windows
            .iter()
            .map(|window| window.issue_number)
            .collect::<Vec<_>>()
    });
    let pending_claim_issues = monitor.and_then(|status| status.pending_claim_issues.clone());
    let needs_human_issues = monitor.map(|status| {
        status
            .needs_human
            .iter()
            .copied()
            .chain(
                status
                    .inbox
                    .iter()
                    .filter(|row| row.state == MonitorInboxState::NeedsHuman)
                    .map(|row| row.issue_number),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    });
    let observed_work_issues: Vec<_> = [
        &implementation_launches,
        &review_issues,
        &pending_claim_issues,
        &needs_human_issues,
    ]
    .into_iter()
    .flatten()
    .flatten()
    .copied()
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect();
    let scan_valid = monitor
        .and_then(|status| status.last_scan_at.as_deref())
        .and_then(parse_observed_instant)
        .is_some();
    let wip_limits = state
        .policies
        .iter()
        .map(|policy| {
            let mut missing_inputs = vec!["verification_observation", "end_to_end_wip"];
            missing_inputs.extend_from_slice(monitor_missing_inputs);
            if !scan_valid && !missing_inputs.contains(&"monitor_scan_time") {
                missing_inputs.push("monitor_scan_time");
            }
            let valid_id = !policy.id.trim().is_empty();
            if !valid_id {
                missing_inputs.push("policy_id");
            }
            let unique_id = state
                .policies
                .iter()
                .filter(|other| other.id == policy.id)
                .count()
                == 1;
            if !unique_id {
                missing_inputs.push("unique_policy_id");
            }
            if policy.wip_limit.is_none() {
                missing_inputs.push("wip_limit");
            }
            let observed_exceeded = valid_id
                && unique_id
                && scan_valid
                && monitor_missing_inputs.is_empty()
                && policy
                    .wip_limit
                    .is_some_and(|limit| observed_work_issues.len() > limit as usize);
            WipLimitObservation {
                id: policy.id.clone(),
                version: policy.version,
                wip_limit: policy.wip_limit,
                observed_issue_count: monitor.map(|_| observed_work_issues.len()),
                assessment: if observed_exceeded {
                    "observed_exceeded"
                } else {
                    "unknown"
                },
                missing_inputs,
            }
        })
        .collect();
    let occupied_slot_count = monitor.and_then(|status| status.occupied_slot_count);
    let mut missing_inputs = monitor_missing_inputs.to_vec();
    missing_inputs.extend(["verification_observation", "end_to_end_wip"]);
    if pending_claim_issues.is_none() {
        missing_inputs.push("pending_claim_issues");
    }
    if occupied_slot_count.is_none() {
        missing_inputs.push("occupied_slot_count");
    }
    CapacityObservation {
        implementation_launches,
        review_issues,
        pending_claim_issues,
        needs_human_issues,
        occupied_slot_count,
        max_active: monitor.map(|status| status.max_active),
        observed_work_issues,
        launch_activity: monitor.map(observe_launch_activity),
        claim_blocks: monitor.map(observe_claim_blocks),
        declared_waits: monitor.map(observe_declared_waits),
        retry_deadlines: monitor.map(observe_retry_deadlines),
        wip_limits,
        idle_windows: monitor.map(observe_idle_windows),
        terminal_queue: monitor
            .and_then(|status| observe_terminal_queue(state, status, monitor_missing_inputs)),
        missing_inputs,
    }
}

#[derive(Debug, Serialize)]
pub struct IssueTrace {
    pub issue_number: u64,
    pub observed_queued_or_active: bool,
    pub observed_inbox_states: Vec<MonitorInboxState>,
    pub roadmap_item_ids: Vec<String>,
    pub alignment: &'static str,
    pub diagnostics: Vec<&'static str>,
}

#[derive(Debug, Default, Serialize)]
pub struct RoadmapHorizons {
    pub now: Vec<String>,
    pub next: Vec<String>,
    pub later: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RoadmapDiagnostic {
    pub id: String,
    pub dependency_observations: Option<Vec<DependencyObservation>>,
    pub issues: Vec<u64>,
    pub observed_issue_states: Vec<MilestoneIssueStates>,
    pub unobserved_issues: Vec<u64>,
    pub diagnostics: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct GoalDiagnostic {
    pub id: String,
    pub roadmap_item_ids: Vec<String>,
    pub dependency_observations: Option<Vec<DependencyObservation>>,
    pub issues: Vec<u64>,
    pub observed_issue_states: Vec<MilestoneIssueStates>,
    pub unobserved_issues: Vec<u64>,
    pub missing_inputs: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct MilestoneDiagnostic {
    pub id: String,
    pub dependency_observations: Option<Vec<DependencyObservation>>,
    pub dependency_impacts: Option<Vec<DependencyImpactObservation>>,
    pub issues: Vec<u64>,
    pub observed_issue_states: Vec<MilestoneIssueStates>,
    pub unobserved_issues: Vec<u64>,
    pub target_date: Option<String>,
    pub target_date_observation: MilestoneTargetDateObservation,
    pub exit_criteria: Vec<String>,
    pub reported_health: Option<String>,
    pub reported_confidence: Option<String>,
    pub health: &'static str,
    pub confidence: &'static str,
    pub missing_inputs: Vec<&'static str>,
}

/// Calendar passage alone cannot establish unfinished work or milestone health.
#[derive(Debug, Serialize)]
pub struct MilestoneTargetDateObservation {
    pub scan_date_utc: Option<String>,
    pub days_past_target_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

fn observe_milestone_target_date(
    target: Option<&str>,
    scan: Option<&str>,
) -> MilestoneTargetDateObservation {
    let observation = observe_calendar_date(target, scan, "target_date");
    MilestoneTargetDateObservation {
        scan_date_utc: observation.scan_date_utc,
        days_past_target_at_scan: observation.days_past_date_at_scan,
        missing_inputs: observation.missing_inputs,
    }
}

/// Calendar passage is not evidence that a review occurred or remains pending.
#[derive(Debug, Serialize)]
pub struct ReviewDateObservation {
    pub kind: &'static str,
    pub id: String,
    pub review_date: Option<String>,
    pub scan_date_utc: Option<String>,
    pub days_past_review_at_scan: Option<u64>,
    pub missing_inputs: Vec<&'static str>,
}

struct CalendarDateObservation {
    scan_date_utc: Option<String>,
    days_past_date_at_scan: Option<u64>,
    missing_inputs: Vec<&'static str>,
}

fn observe_calendar_date(
    target: Option<&str>,
    scan: Option<&str>,
    date_input: &'static str,
) -> CalendarDateObservation {
    let target = target.and_then(|value| {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .filter(|date| value.len() == 10 && date.format("%Y-%m-%d").to_string() == value)
    });
    let scan = scan
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc).date_naive());
    let mut missing_inputs = Vec::new();
    if target.is_none() {
        missing_inputs.push(date_input);
    }
    if scan.is_none() {
        missing_inputs.push("monitor_scan_time");
    }
    CalendarDateObservation {
        scan_date_utc: scan.map(|date| date.to_string()),
        days_past_date_at_scan: scan
            .zip(target)
            .map(|(scan, target)| scan.signed_duration_since(target).num_days().max(0) as u64),
        missing_inputs,
    }
}

fn observe_review_dates(state: &PmPlanningState, scan: Option<&str>) -> Vec<ReviewDateObservation> {
    state
        .risks
        .iter()
        .map(|risk| ("risk", &risk.id, &risk.review_date))
        .chain(
            state
                .decisions
                .iter()
                .map(|decision| ("decision", &decision.id, &decision.review_date)),
        )
        .map(|(kind, id, review_date)| {
            let observation = observe_calendar_date(review_date.as_deref(), scan, "review_date");
            ReviewDateObservation {
                kind,
                id: id.clone(),
                review_date: review_date.clone(),
                scan_date_utc: observation.scan_date_utc,
                days_past_review_at_scan: observation.days_past_date_at_scan,
                missing_inputs: observation.missing_inputs,
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct MilestoneIssueStates {
    pub issue_number: u64,
    pub states: Vec<MonitorInboxState>,
}

fn observe_inbox_states(
    issue_number: u64,
    monitor: Option<&IssueMonitorAgentStatus>,
) -> Vec<MonitorInboxState> {
    let mut states = Vec::new();
    for row in monitor.into_iter().flat_map(|status| &status.inbox) {
        if row.issue_number == issue_number && !states.contains(&row.state) {
            states.push(row.state);
        }
    }
    states
}

fn observe_issues(
    members: &BTreeSet<u64>,
    monitor: Option<&IssueMonitorAgentStatus>,
) -> (Vec<MilestoneIssueStates>, Vec<u64>) {
    let mut observed_issue_states = Vec::new();
    let mut unobserved_issues = Vec::new();
    for &issue_number in members {
        let states = observe_inbox_states(issue_number, monitor);
        if states.is_empty() {
            unobserved_issues.push(issue_number);
        } else {
            observed_issue_states.push(MilestoneIssueStates {
                issue_number,
                states,
            });
        }
    }
    (observed_issue_states, unobserved_issues)
}

/// Assess only the supplied snapshots. Alignment is structural, never an
/// authorization to prioritize, launch or complete work. Monitor provenance
/// remains independent of the planning revision; neither snapshot proves the
/// other is current. Missing observations are never a complete empty census.
pub fn assess_planning(
    state: &PmPlanningState,
    monitor: Option<&IssueMonitorAgentStatus>,
) -> PlanningAssessment {
    let mut observed = BTreeSet::new();
    let mut missing_inputs = Vec::new();
    if let Some(status) = monitor {
        observed.extend(status.queue.iter().copied());
        observed.extend(status.active_launches.iter().copied());
        observed.extend(status.pending_claim_issues.iter().flatten().copied());
        observed.extend(
            status
                .review_windows
                .iter()
                .map(|window| window.issue_number),
        );
        observed.extend(
            status
                .inbox
                .iter()
                .filter(|row| {
                    matches!(
                        row.state,
                        MonitorInboxState::Queued
                            | MonitorInboxState::Launching
                            | MonitorInboxState::Launched
                            | MonitorInboxState::BlockedByClaim
                    )
                })
                .map(|row| row.issue_number),
        );
        if status.source != IssueMonitorStatusSource::Daemon
            || status.active_launches_incomplete
            || status.inbox_coverage.as_ref().is_none_or(|coverage| {
                coverage.source != crate::IssueMonitorCandidateSource::Live
                    || !coverage.missing_issue_numbers.is_empty()
            })
        {
            missing_inputs.push("complete_monitor_coverage");
        }
        if status
            .last_scan_at
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            missing_inputs.push("monitor_scan_time");
        }
        if status.scan_aborted {
            missing_inputs.push("successful_monitor_scan");
        }
    } else {
        missing_inputs.push("monitor_observation");
        missing_inputs.push("complete_monitor_coverage");
    }
    let dependency_observations: Option<Vec<_>> = monitor.map(|status| {
        status
            .inbox
            .iter()
            .map(|row| DependencyObservation {
                issue_number: row.issue_number,
                state: row.state,
                dependency: row.dependency.clone(),
                missing_inputs: if row.dependency.is_none() {
                    vec!["dependency_status"]
                } else {
                    vec![]
                },
            })
            .collect()
    });
    let dependency_impacts: Option<Vec<DependencyImpactObservation>> =
        dependency_observations.as_ref().map(|rows| {
            let mut impacts = BTreeMap::<u64, BTreeSet<u64>>::new();
            for row in rows {
                if let Some(dependency) = &row.dependency {
                    for blocker in &dependency.blocked_by {
                        impacts
                            .entry(blocker.number)
                            .or_default()
                            .insert(row.issue_number);
                    }
                }
            }
            impacts
                .into_iter()
                .map(|(issue_number, dependents)| DependencyImpactObservation {
                    issue_number,
                    dependent_issues: dependents.into_iter().collect(),
                })
                .collect()
        });
    let mut issues = observed.clone();
    let mut roadmap = RoadmapHorizons::default();
    let roadmap_items: Vec<_> = state
        .roadmap_items
        .iter()
        .map(|item| {
            issues.extend(item.issues.iter().copied());
            match item.horizon {
                RoadmapHorizon::Now => &mut roadmap.now,
                RoadmapHorizon::Next => &mut roadmap.next,
                RoadmapHorizon::Later => &mut roadmap.later,
            }
            .push(item.id.clone());
            // Each row owns its direct references even when IDs are ambiguous.
            let members = item.issues.iter().copied().collect();
            let (observed_issue_states, unobserved_issues) = observe_issues(&members, monitor);
            RoadmapDiagnostic {
                id: item.id.clone(),
                dependency_observations: dependency_observations.as_ref().map(|rows| {
                    rows.iter()
                        .filter(|row| members.contains(&row.issue_number))
                        .cloned()
                        .collect()
                }),
                issues: item.issues.clone(),
                observed_issue_states,
                unobserved_issues,
                diagnostics: diagnose_link(state, item),
            }
        })
        .collect();
    for milestone in &state.milestones {
        issues.extend(milestone.issues.iter().copied());
    }
    let issue_traces = issues
        .into_iter()
        .map(|issue_number| {
            let links: Vec<_> = state
                .roadmap_items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.issues.contains(&issue_number))
                .collect();
            let mut diagnostics: BTreeSet<_> = links
                .iter()
                .flat_map(|(index, _)| roadmap_items[*index].diagnostics.iter().copied())
                .collect();
            if links.len() > 1 {
                diagnostics.insert("multiple_roadmap_links");
            }
            let alignment = if links.is_empty() {
                diagnostics.insert("missing_trace");
                "orphan"
            } else if diagnostics.is_empty() {
                "aligned"
            } else {
                "invalid"
            };
            IssueTrace {
                issue_number,
                observed_queued_or_active: observed.contains(&issue_number),
                observed_inbox_states: observe_inbox_states(issue_number, monitor),
                roadmap_item_ids: links.iter().map(|(_, item)| item.id.clone()).collect(),
                alignment,
                diagnostics: diagnostics.into_iter().collect(),
            }
        })
        .collect();
    let goals = state
        .goals
        .iter()
        .map(|goal| {
            let mut missing_inputs = vec!["metric_evidence"];
            let nonempty_id = !goal.id.trim().is_empty();
            if !nonempty_id {
                missing_inputs.push("id");
            }
            let unique_id = state
                .goals
                .iter()
                .filter(|other| other.id == goal.id)
                .count()
                == 1;
            if !unique_id {
                missing_inputs.push("unique_goal_id");
            }
            missing_inputs.extend(missing_goal_metadata(goal));
            let links: Vec<_> = state
                .roadmap_items
                .iter()
                .filter(|item| {
                    nonempty_id && unique_id && item.goal_id.as_deref() == Some(goal.id.as_str())
                })
                .collect();
            let members: BTreeSet<_> = links
                .iter()
                .flat_map(|item| item.issues.iter().copied())
                .collect();
            let (observed_issue_states, unobserved_issues) = observe_issues(&members, monitor);
            GoalDiagnostic {
                id: goal.id.clone(),
                roadmap_item_ids: links.iter().map(|item| item.id.clone()).collect(),
                dependency_observations: dependency_observations.as_ref().map(|rows| {
                    rows.iter()
                        .filter(|row| members.contains(&row.issue_number))
                        .cloned()
                        .collect()
                }),
                issues: members.into_iter().collect(),
                observed_issue_states,
                unobserved_issues,
                missing_inputs,
            }
        })
        .collect();
    let milestones = state
        .milestones
        .iter()
        .map(|milestone| {
            let mut missing_inputs = vec!["exit_criteria_evidence", "metric_evidence"];
            if milestone.id.trim().is_empty() {
                missing_inputs.push("id");
            }
            let unique_id = state
                .milestones
                .iter()
                .filter(|other| other.id == milestone.id)
                .count()
                == 1;
            if !unique_id {
                missing_inputs.push("unique_milestone_id");
            }
            // Direct references are membership facts, not goal alignment or
            // completion evidence. An ambiguous ID cannot resolve a link.
            let mut members: BTreeSet<_> = milestone.issues.iter().copied().collect();
            if unique_id && !milestone.id.trim().is_empty() {
                for item in &state.roadmap_items {
                    if item.milestone_id.as_deref() == Some(milestone.id.as_str()) {
                        members.extend(item.issues.iter().copied());
                    }
                }
            }
            let (observed_issue_states, unobserved_issues) = observe_issues(&members, monitor);
            for (name, value) in [
                ("target_date", &milestone.target_date),
                ("health", &milestone.health),
                ("confidence", &milestone.confidence),
            ] {
                if value.as_deref().is_none_or(|value| value.trim().is_empty()) {
                    missing_inputs.push(name);
                }
            }
            if milestone.exit_criteria.is_empty()
                || milestone
                    .exit_criteria
                    .iter()
                    .any(|value| value.trim().is_empty())
            {
                missing_inputs.push("exit_criteria");
            }
            MilestoneDiagnostic {
                id: milestone.id.clone(),
                dependency_observations: dependency_observations.as_ref().map(|rows| {
                    rows.iter()
                        .filter(|row| members.contains(&row.issue_number))
                        .cloned()
                        .collect()
                }),
                dependency_impacts: dependency_impacts.as_ref().map(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            let dependent_issues: Vec<_> = row
                                .dependent_issues
                                .iter()
                                .copied()
                                .filter(|number| members.contains(number))
                                .collect();
                            (!dependent_issues.is_empty()).then_some(DependencyImpactObservation {
                                issue_number: row.issue_number,
                                dependent_issues,
                            })
                        })
                        .collect()
                }),
                issues: milestone.issues.clone(),
                observed_issue_states,
                unobserved_issues,
                target_date: milestone.target_date.clone(),
                target_date_observation: observe_milestone_target_date(
                    milestone.target_date.as_deref(),
                    monitor.and_then(|status| status.last_scan_at.as_deref()),
                ),
                exit_criteria: milestone.exit_criteria.clone(),
                reported_health: milestone.health.clone(),
                reported_confidence: milestone.confidence.clone(),
                health: "unknown",
                confidence: "unknown",
                missing_inputs,
            }
        })
        .collect();
    PlanningAssessment {
        planning_revision: state.revision,
        capacity: observe_capacity(state, monitor, &missing_inputs),
        monitor: MonitorEvidence {
            source: monitor.map(|status| status.source),
            last_scan_at: monitor.and_then(|status| status.last_scan_at.clone()),
            scan_aborted: monitor.map(|status| status.scan_aborted),
            active_launches_incomplete: monitor
                .is_none_or(|status| status.active_launches_incomplete),
            inbox_coverage: monitor.and_then(|status| status.inbox_coverage.clone()),
        },
        missing_inputs,
        dependency_observations,
        dependency_impacts,
        issue_traces,
        roadmap,
        roadmap_items,
        goals,
        milestones,
        risks: state
            .risks
            .iter()
            .map(|risk| {
                let mut missing_inputs = Vec::new();
                if risk.id.trim().is_empty() {
                    missing_inputs.push("id");
                } else if state
                    .risks
                    .iter()
                    .filter(|other| other.id == risk.id)
                    .count()
                    > 1
                {
                    missing_inputs.push("unique_risk_id");
                }
                for (name, value) in [
                    ("severity", &risk.severity),
                    ("probability", &risk.probability),
                    ("impact", &risk.impact),
                    ("owner", &risk.owner),
                ] {
                    if value.trim().is_empty() {
                        missing_inputs.push(name);
                    }
                }
                RiskMetadataDiagnostic {
                    id: risk.id.clone(),
                    missing_inputs,
                }
            })
            .collect(),
        decisions: state
            .decisions
            .iter()
            .map(|decision| {
                let mut missing_inputs = Vec::new();
                if decision.id.trim().is_empty() {
                    missing_inputs.push("id");
                } else if state
                    .decisions
                    .iter()
                    .filter(|other| other.id == decision.id)
                    .count()
                    > 1
                {
                    missing_inputs.push("unique_decision_id");
                }
                if !decision
                    .options
                    .iter()
                    .any(|value| !value.trim().is_empty())
                {
                    missing_inputs.push("options");
                }
                for (name, value) in [
                    ("rationale", &decision.rationale),
                    ("owner", &decision.owner),
                ] {
                    if value.trim().is_empty() {
                        missing_inputs.push(name);
                    }
                }
                DecisionMetadataDiagnostic {
                    id: decision.id.clone(),
                    missing_inputs,
                }
            })
            .collect(),
        decision_revisions: state
            .decisions
            .iter()
            .map(|decision| {
                let revisions_since_source = decision
                    .source_revision
                    .and_then(|source| state.revision.checked_sub(source));
                let missing_inputs = match decision.source_revision {
                    None => vec!["source_revision"],
                    Some(_) if revisions_since_source.is_none() => {
                        vec!["source_revision_ahead_of_planning"]
                    }
                    Some(_) => vec![],
                };
                DecisionRevisionObservation {
                    id: decision.id.clone(),
                    source_revision: decision.source_revision,
                    revisions_since_source,
                    missing_inputs,
                }
            })
            .collect(),
        forecasts: state
            .latest_forecasts
            .iter()
            .map(|forecast| {
                let revisions_since_source = state.revision.checked_sub(forecast.source_revision);
                let mut missing_inputs = vec!["forecast_evidence"];
                if forecast.id.trim().is_empty() {
                    missing_inputs.push("id");
                } else if state
                    .latest_forecasts
                    .iter()
                    .filter(|other| other.id == forecast.id)
                    .count()
                    > 1
                {
                    missing_inputs.push("unique_forecast_id");
                }
                if forecast.sample_size == 0 {
                    missing_inputs.push("sample_history");
                }
                if revisions_since_source.is_none() {
                    missing_inputs.push("source_revision_ahead_of_planning");
                }
                ForecastObservation {
                    id: forecast.id.clone(),
                    source_revision: forecast.source_revision,
                    sample_size: forecast.sample_size,
                    reported_estimate: forecast.estimate.clone(),
                    reported_confidence: forecast.confidence.clone(),
                    reported_missing_inputs: forecast.missing_inputs.clone(),
                    revisions_since_source,
                    confidence: "unknown",
                    missing_inputs,
                }
            })
            .collect(),
        review_dates: observe_review_dates(
            state,
            monitor.and_then(|status| status.last_scan_at.as_deref()),
        ),
        recurring_runs: observe_recurring_runs(
            state,
            monitor.and_then(|status| status.last_scan_at.as_deref()),
        ),
    }
}

fn missing_goal_metadata(goal: &Goal) -> Vec<&'static str> {
    [
        ("outcome", &goal.outcome),
        ("metric", &goal.metric),
        ("target", &goal.target),
        ("horizon", &goal.horizon),
        ("owner", &goal.owner),
        ("status", &goal.status),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.trim().is_empty().then_some(name))
    .collect()
}

fn diagnose_link(state: &PmPlanningState, item: &RoadmapItem) -> Vec<&'static str> {
    let mut diagnostics = Vec::new();
    if item.id.trim().is_empty() {
        diagnostics.push("missing_roadmap_id");
    }
    if state
        .roadmap_items
        .iter()
        .filter(|other| other.id == item.id)
        .count()
        > 1
    {
        diagnostics.push("duplicate_roadmap_id");
    }
    match (&item.goal_id, &item.category) {
        (Some(_), Some(_)) => diagnostics.push("ambiguous_trace"),
        (None, None) => diagnostics.push("missing_trace"),
        (None, Some(category)) => {
            if category != "maintenance" && category != "compliance" {
                diagnostics.push("unsupported_category");
            }
        }
        (Some(id), None) => {
            let goals: Vec<_> = state
                .goals
                .iter()
                .filter(|goal| &goal.id == id && !id.trim().is_empty())
                .collect();
            match goals.as_slice() {
                [] => diagnostics.push("missing_goal"),
                [goal] => {
                    if !missing_goal_metadata(goal).is_empty() {
                        diagnostics.push("incomplete_goal");
                    }
                }
                _ => diagnostics.push("ambiguous_goal"),
            }
        }
    }
    if let Some(id) = &item.milestone_id {
        match state
            .milestones
            .iter()
            .filter(|milestone| &milestone.id == id && !id.trim().is_empty())
            .count()
        {
            0 => diagnostics.push("missing_milestone"),
            1 => {}
            _ => diagnostics.push("ambiguous_milestone"),
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm_planning::PmPlanningState;
    use serde_json::{json, Value};

    #[test]
    fn dependency_observations_preserve_canonical_facts_without_evaluating_readiness() {
        let state = PmPlanningState::default();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":9,"state":"blocked_by_dependency","attempts":0,
             "dependency":{"blocked_by":[{"number":3},{"number":1},{"number":3}],"cycle":true,"bundle_with":[7,5]},
             "error_message":"private diagnostic","claim_id":"private claim"},
            {"issue_number":9,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]}},
            {"issue_number":4,"state":"blocked_by_dependency","attempts":0},
            {"issue_number":5,"state":"released","attempts":0,
             "dependency":{"blocked_by":[],"cycle":true,"bundle_with":[6]}},
            {"issue_number":3,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":1}],"cycle":false,"bundle_with":[]}},
            {"issue_number":8,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":3}],"cycle":false,"bundle_with":[]}},
            {"issue_number":9,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":1},{"number":3}],"cycle":false,"bundle_with":[]}}
        ]))
        .unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let expected = json!([
            {"issue_number":9,"state":"blocked_by_dependency",
             "dependency":{"blocked_by":[{"number":3},{"number":1},{"number":3}],"cycle":true,"bundle_with":[7,5]},
             "missing_inputs":[]},
            {"issue_number":9,"state":"queued",
             "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]},"missing_inputs":[]},
            {"issue_number":4,"state":"blocked_by_dependency","dependency":null,
             "missing_inputs":["dependency_status"]},
            {"issue_number":5,"state":"released",
             "dependency":{"blocked_by":[],"cycle":true,"bundle_with":[6]},"missing_inputs":[]},
            {"issue_number":3,"state":"queued",
             "dependency":{"blocked_by":[{"number":1}],"cycle":false,"bundle_with":[]},"missing_inputs":[]},
            {"issue_number":8,"state":"queued",
             "dependency":{"blocked_by":[{"number":3}],"cycle":false,"bundle_with":[]},"missing_inputs":[]},
            {"issue_number":9,"state":"queued",
             "dependency":{"blocked_by":[{"number":1},{"number":3}],"cycle":false,"bundle_with":[]},"missing_inputs":[]}
        ]);
        assert_eq!(report.get("dependency_observations"), Some(&expected));
        // 8 depends on 3, which depends on 1: do not invent the transitive 1 -> 8 edge.
        let impacts = json!([
            {"issue_number":1,"dependent_issues":[3,9]},
            {"issue_number":3,"dependent_issues":[8,9]}
        ]);
        assert_eq!(report.get("dependency_impacts"), Some(&impacts));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["dependency_observations"], expected);
        assert_eq!(degraded.get("dependency_impacts"), Some(&impacts));
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        assert!(degraded["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_monitor_coverage")));
        monitor.scan_aborted = true;
        let aborted_before = serde_json::to_value(&monitor).unwrap();
        let aborted = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(aborted.get("dependency_impacts"), Some(&impacts));
        assert_eq!(aborted["monitor"]["scan_aborted"], true);
        assert!(aborted["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("successful_monitor_scan")));
        assert_eq!(serde_json::to_value(&monitor).unwrap(), aborted_before);
        monitor.inbox.clear();
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(empty.get("dependency_observations"), Some(&json!([])));
        assert_eq!(empty.get("dependency_impacts"), Some(&json!([])));
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(absent.get("dependency_observations"), Some(&Value::Null));
        assert_eq!(absent.get("dependency_impacts"), Some(&Value::Null));
    }

    #[test]
    fn terminal_queue_residency_preserves_supplied_rows_without_claiming_wait_time() {
        let state = PmPlanningState::default();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2026-10-22T01:00:00.500Z".into());
        let inputs = [
            (9, "2026-10-22T01:59:00+01:00", Some(60), vec![]),
            (3, "2026-10-22T01:00:00.500Z", Some(0), vec![]),
            (
                9,
                "2026-10-22T01:00:00.500000001Z",
                None,
                vec!["queued_after_scan"],
            ),
            (4, "invalid", None, vec!["queued_at"]),
            (5, "", None, vec!["queued_at"]),
            (
                6,
                "2026-10-22T00:59:00.5000000001Z",
                None,
                vec!["queued_at"],
            ),
            (7, "2026-10-22T00:59:59.900Z", Some(0), vec![]),
        ];
        monitor.gui_status.as_mut().unwrap().terminal_queue = inputs
            .iter()
            .map(
                |(number, queued_at, _, _)| crate::issue_monitor::IssueMonitorTerminalQueueEntry {
                    number: *number,
                    queued_at: (*queued_at).into(),
                    ..Default::default()
                },
            )
            .collect();
        let before = serde_json::to_value(&monitor).unwrap();
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["capacity"]["terminal_queue"]
            .as_array()
            .expect("queue observations");
        assert_eq!(rows.len(), inputs.len());
        for (row, (number, queued_at, seconds, missing)) in rows.iter().zip(&inputs) {
            assert_eq!(
                *row,
                json!({"issue_number":number,"queued_at":queued_at,
                "age_seconds_at_scan":seconds,"missing_inputs":missing,"age_limits":[]})
            );
        }
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["terminal_queue"],
            report["capacity"]["terminal_queue"]
        );
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        monitor.source = IssueMonitorStatusSource::Daemon;
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before);
        assert_eq!(state, PmPlanningState::default());
        for scan in [
            None,
            Some("invalid"),
            Some("2026-10-22T01:00:00.5000000001Z"),
        ] {
            monitor.last_scan_at = scan.map(str::to_owned);
            let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            assert_eq!(
                report["capacity"]["terminal_queue"][0]["age_seconds_at_scan"],
                Value::Null
            );
            assert_eq!(
                report["capacity"]["terminal_queue"][0]["missing_inputs"],
                json!(["monitor_scan_time"])
            );
            assert_eq!(
                report["capacity"]["terminal_queue"][3]["missing_inputs"],
                json!(["queued_at", "monitor_scan_time"])
            );
        }
        monitor.gui_status.as_mut().unwrap().terminal_queue.clear();
        assert_eq!(
            serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap()["capacity"]
                ["terminal_queue"],
            json!([])
        );
        monitor.gui_status = None;
        for observation in [Some(&monitor), None] {
            assert_eq!(
                serde_json::to_value(assess_planning(&state, observation)).unwrap()["capacity"]
                    .get("terminal_queue"),
                Some(&Value::Null)
            );
        }
    }

    #[test]
    fn queue_age_limits_compare_saved_thresholds_without_claiming_starvation() {
        let state = PmPlanningState {
            policies: serde_json::from_value(json!([
                {"id":"equal","version":3,"starvation_after_seconds":60,"wip_limit":0},
                {"id":"below","starvation_after_seconds":61},
                {"id":"above","starvation_after_seconds":59},
                {"id":"zero","starvation_after_seconds":0}, {"id":"unset"},
                {"id":" ","starvation_after_seconds":0},
                {"id":"dup","starvation_after_seconds":0},
                {"id":"dup","version":2,"starvation_after_seconds":0}
            ]))
            .unwrap(),
            ..Default::default()
        };
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.active_launches = vec![9];
        monitor.last_scan_at = Some("2026-10-25T01:01:00Z".into());
        monitor.inbox_coverage = Some(serde_json::from_value(json!({
            "github_target_count":0,"inbox_row_count":0,"missing_issue_numbers":[],"source":"live"
        })).unwrap());
        monitor.gui_status.as_mut().unwrap().terminal_queue = [
            "2026-10-25T01:00:00Z",
            "2026-10-25T01:01:00Z",
            "invalid",
            "2026-10-25T01:01:01Z",
        ]
        .map(
            |queued_at| crate::issue_monitor::IssueMonitorTerminalQueueEntry {
                number: 9,
                queued_at: queued_at.into(),
                ..Default::default()
            },
        )
        .into();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["capacity"]["terminal_queue"].as_array().unwrap();
        let limits = rows[0]["age_limits"]
            .as_array()
            .expect("per-policy age limits");
        assert_eq!(
            limits[0],
            json!({"id":"equal","version":3,
            "starvation_after_seconds":60,"assessment":"observed_age_threshold_reached","missing_inputs":[]})
        );
        assert_eq!(
            limits
                .iter()
                .map(|row| row["assessment"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "observed_age_threshold_reached",
                "unknown",
                "observed_age_threshold_reached",
                "observed_age_threshold_reached",
                "unknown",
                "unknown",
                "unknown",
                "unknown"
            ]
        );
        for (index, missing) in [
            (4, "starvation_after_seconds"),
            (5, "policy_id"),
            (6, "unique_policy_id"),
            (7, "unique_policy_id"),
        ] {
            assert!(limits[index]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!(missing)));
        }
        assert_eq!(
            rows[1]["age_limits"][3]["assessment"],
            "observed_age_threshold_reached"
        );
        for (index, missing) in [(2, "queued_at"), (3, "queued_after_scan")] {
            assert_eq!(rows[index]["age_limits"][3]["assessment"], "unknown");
            assert!(rows[index]["age_limits"][3]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!(missing)));
        }
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        // Failed scans retain these facts but their advanced timestamp must not
        // qualify the saved-policy comparisons. Ordinary errors are unrelated.
        monitor.last_error = Some("unrelated issue diagnostic".into());
        let ordinary_error = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(ordinary_error, report);
        assert_eq!(
            report["capacity"]["wip_limits"][0]["assessment"],
            "observed_exceeded"
        );
        monitor.scan_aborted = true;
        let aborted_before = serde_json::to_value(&monitor).unwrap();
        let aborted = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let aborted_age = &aborted["capacity"]["terminal_queue"][0];
        assert_eq!(aborted_age["age_limits"][0]["assessment"], "unknown");
        assert_eq!(
            aborted["capacity"]["wip_limits"][0]["assessment"],
            "unknown"
        );
        for diagnostic in [
            &aborted["missing_inputs"],
            &aborted["capacity"]["missing_inputs"],
            &aborted["capacity"]["wip_limits"][0]["missing_inputs"],
            &aborted_age["age_limits"][0]["missing_inputs"],
        ] {
            assert!(diagnostic
                .as_array()
                .unwrap()
                .contains(&json!("successful_monitor_scan")));
        }
        assert_eq!(aborted["monitor"].get("scan_aborted"), Some(&json!(true)));
        assert_eq!(report["monitor"].get("scan_aborted"), Some(&json!(false)));
        assert_eq!(
            aborted["monitor"]["last_scan_at"],
            report["monitor"]["last_scan_at"]
        );
        assert_eq!(aborted["capacity"]["observed_work_issues"], json!([9]));
        for field in [
            "issue_number",
            "queued_at",
            "age_seconds_at_scan",
            "missing_inputs",
        ] {
            assert_eq!(aborted_age[field], rows[0][field]);
        }
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), aborted_before);
        assert_eq!(
            serde_json::to_value(assess_planning(&state, None)).unwrap()["monitor"]
                .get("scan_aborted"),
            Some(&Value::Null)
        );
        monitor.scan_aborted = false;
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["terminal_queue"][0]["age_limits"][0]["assessment"],
            "unknown"
        );
        assert!(
            degraded["capacity"]["terminal_queue"][0]["age_limits"][0]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!("complete_monitor_coverage"))
        );
        monitor.source = IssueMonitorStatusSource::Daemon;
        monitor.active_launches_incomplete = true;
        assert_eq!(
            serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap()["capacity"]
                ["terminal_queue"][0]["age_limits"][0]["assessment"],
            "unknown"
        );
    }

    fn fixture() -> PmPlanningState {
        serde_json::from_value(json!({
            "revision":7,
            "goals":[{"id":"g","outcome":"Ship","metric":"adoption","target":"10",
                "horizon":"quarter","owner":"team","status":"active"}, {"id":"empty"}],
            "roadmap_items":[
                {"id":"a","goal_id":"g","horizon":"now","issues":[1]},
                {"id":"b","category":"maintenance","horizon":"next","issues":[2]},
                {"id":"c","category":"compliance","horizon":"later","issues":[3]},
                {"id":"d","goal_id":"missing","issues":[5]},
                {"id":"e","goal_id":"g","category":"maintenance","issues":[6]},
                {"id":"f","category":"feature","issues":[7]},
                {"id":"h","goal_id":"empty","issues":[8]},
                {"id":"i","goal_id":"g","issues":[9]},
                {"id":"j","category":"maintenance","issues":[9]},
                {"id":"k","goal_id":"g","milestone_id":"missing","issues":[10]}
            ],
            "milestones":[{"id":"m","target_date":"2027-01-01","exit_criteria":["Tests"],
                "health":"green","confidence":"high","issues":[1]}]
        }))
        .unwrap()
    }

    fn row(report: &Value, number: u64) -> &Value {
        report["issue_traces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["issue_number"] == number)
            .unwrap()
    }

    #[test]
    fn traces_goals_categories_and_orphans_without_justifying_ambiguous_work() {
        let state = fixture();
        let before = state.clone();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.queue = vec![1, 2, 4];
        monitor.active_launches = vec![3, 3, 5];
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":2,"state":"needs_human","attempts":0,"error_message":"private reason"},
            {"issue_number":2,"state":"source_excluded","attempts":0},
            {"issue_number":2,"state":"needs_human","attempts":0},
            {"issue_number":4,"state":"blocked_by_claim","attempts":0},
            {"issue_number":5,"state":"agent_failed","attempts":0},
            {"issue_number":99,"state":"agent_failed","attempts":0}
        ]))
        .unwrap();
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(state, before);
        assert_eq!(report["planning_revision"], 7);
        assert_eq!(report["issue_traces"].as_array().unwrap().len(), 10);
        for n in [1, 2, 3] {
            assert_eq!(row(&report, n)["alignment"], "aligned");
        }
        assert_eq!(row(&report, 4)["alignment"], "orphan");
        for (n, reason) in [
            (5, "missing_goal"),
            (6, "ambiguous_trace"),
            (7, "unsupported_category"),
            (8, "incomplete_goal"),
            (9, "multiple_roadmap_links"),
        ] {
            assert_eq!(row(&report, n)["alignment"], "invalid");
            assert!(row(&report, n)["diagnostics"]
                .as_array()
                .unwrap()
                .contains(&json!(reason)));
        }
        assert_eq!(row(&report, 4)["observed_queued_or_active"], true);
        assert_eq!(row(&report, 10)["observed_queued_or_active"], false);
        assert_eq!(row(&report, 1)["observed_inbox_states"], json!([]));
        assert_eq!(
            row(&report, 2)["observed_inbox_states"],
            json!(["needs_human", "source_excluded"])
        );
        assert_eq!(
            row(&report, 4)["observed_inbox_states"],
            json!(["blocked_by_claim"])
        );
        assert_eq!(
            row(&report, 5)["observed_inbox_states"],
            json!(["agent_failed"])
        );
        assert!(!report.to_string().contains("private reason"));
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["issue_traces"], report["issue_traces"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        assert_eq!(report["roadmap"]["now"], json!(["a"]));
        assert_eq!(report["roadmap"]["next"], json!(["b"]));
        assert!(report["roadmap_items"][9]["diagnostics"]
            .as_array()
            .unwrap()
            .contains(&json!("missing_milestone")));
    }

    #[test]
    fn roadmap_observations_use_each_rows_direct_issues_even_with_ambiguous_ids() {
        let state: PmPlanningState = serde_json::from_value(json!({
            "roadmap_items":[
                {"id":"same","horizon":"now","issues":[3,1,3,2]},
                {"id":"same","horizon":"next","issues":[4]},
                {"id":"","horizon":"later","issues":[6]},
                {"id":"empty","horizon":"later","issues":[]}
            ],
            "milestones":[{"id":"m","issues":[5]}]
        }))
        .unwrap();
        let before = state.clone();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.queue = vec![2];
        monitor.active_launches = vec![4];
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":3,"state":"source_excluded","attempts":0,
             "dependency":{"blocked_by":[{"number":9}],"cycle":true,"bundle_with":[7,5]},
             "error_message":"private diagnostic"},
            {"issue_number":5,"state":"merged","attempts":0},
            {"issue_number":1,"state":"launched","attempts":0,
             "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]}},
            {"issue_number":6,"state":"needs_human","attempts":0},
            {"issue_number":1,"state":"needs_human","attempts":0},
            {"issue_number":1,"state":"launched","attempts":0,
             "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]}}
        ]))
        .unwrap();
        let monitor_before = serde_json::to_value(&monitor).unwrap();
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = &report["roadmap_items"];
        assert_eq!(
            rows[0].get("dependency_observations"),
            Some(&json!([
                {"issue_number":3,"state":"source_excluded",
                 "dependency":{"blocked_by":[{"number":9}],"cycle":true,"bundle_with":[7,5]},
                 "missing_inputs":[]},
                {"issue_number":1,"state":"launched",
                 "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]},"missing_inputs":[]},
                {"issue_number":1,"state":"needs_human","dependency":null,
                 "missing_inputs":["dependency_status"]},
                {"issue_number":1,"state":"launched",
                 "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]},"missing_inputs":[]}
            ]))
        );
        assert_eq!(rows[1].get("dependency_observations"), Some(&json!([])));
        assert_eq!(
            rows[2].get("dependency_observations"),
            Some(
                &json!([{"issue_number":6,"state":"needs_human","dependency":null,
                         "missing_inputs":["dependency_status"]}])
            )
        );
        assert_eq!(rows[3].get("dependency_observations"), Some(&json!([])));
        assert!(!report.to_string().contains("private diagnostic"));
        assert_eq!(serde_json::to_value(&monitor).unwrap(), monitor_before);
        assert_eq!(rows[0]["issues"], json!([3, 1, 3, 2]));
        assert_eq!(
            rows[0]["observed_issue_states"],
            json!([
                {"issue_number":1,"states":["launched","needs_human"]},
                {"issue_number":3,"states":["source_excluded"]}
            ])
        );
        assert_eq!(rows[0]["unobserved_issues"], json!([2]));
        assert_eq!(
            rows[0]["diagnostics"],
            json!(["duplicate_roadmap_id", "missing_trace"])
        );
        assert_eq!(rows[1]["issues"], json!([4]));
        assert_eq!(rows[1]["observed_issue_states"], json!([]));
        assert_eq!(rows[1]["unobserved_issues"], json!([4]));
        for field in ["issues", "observed_issue_states", "unobserved_issues"] {
            assert_eq!(rows[3][field], json!([]));
        }
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        for row in absent["roadmap_items"].as_array().unwrap() {
            assert_eq!(row.get("dependency_observations"), Some(&Value::Null));
        }
        assert_eq!(
            absent["roadmap_items"][0]["observed_issue_states"],
            json!([])
        );
        assert_eq!(
            absent["roadmap_items"][0]["unobserved_issues"],
            json!([1, 2, 3])
        );
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["roadmap_items"], *rows);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        monitor.inbox.retain(|row| row.issue_number == 5);
        let unrelated = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        monitor.inbox.clear();
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        for observation in [unrelated, empty] {
            for row in observation["roadmap_items"].as_array().unwrap() {
                assert_eq!(row.get("dependency_observations"), Some(&json!([])));
            }
        }
        assert_eq!(state, before);
    }

    #[test]
    fn forecasts_preserve_authored_values_without_certifying_evidence() {
        let mut state: PmPlanningState = serde_json::from_value(json!({
            "revision":5,
            "latest_forecasts":[
                {"id":"same", "source_revision":0, "sample_size":0,
                 "estimate":"two days", "confidence":"high", "missing_inputs":["cost", "cost"]},
                {"id":" \t", "source_revision":5, "sample_size":1},
                {"id":"same", "source_revision":6, "sample_size":2},
                {"id":"Same", "source_revision":3, "sample_size":u64::MAX},
                {"id":" same ", "source_revision":u64::MAX, "sample_size":1}
            ]
        }))
        .unwrap();
        let before = state.clone();
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        let rows = report["forecasts"]
            .as_array()
            .expect("forecast diagnostics");
        assert_eq!(rows.len(), state.latest_forecasts.len());
        for (row, saved) in rows.iter().zip(&state.latest_forecasts) {
            assert_eq!(row["id"], saved.id);
            assert_eq!(row["source_revision"], saved.source_revision);
            assert_eq!(row["sample_size"], saved.sample_size);
            assert_eq!(row["reported_estimate"], json!(saved.estimate));
            assert_eq!(row["reported_confidence"], json!(saved.confidence));
            assert_eq!(row["reported_missing_inputs"], json!(saved.missing_inputs));
            assert_eq!(row["confidence"], "unknown");
        }
        assert_eq!(
            rows.iter()
                .map(|row| &row["revisions_since_source"])
                .collect::<Vec<_>>(),
            vec![&json!(5), &json!(0), &Value::Null, &json!(2), &Value::Null]
        );
        assert_eq!(
            rows.iter()
                .map(|row| &row["missing_inputs"])
                .collect::<Vec<_>>(),
            vec![
                &json!(["forecast_evidence", "unique_forecast_id", "sample_history"]),
                &json!(["forecast_evidence", "id"]),
                &json!([
                    "forecast_evidence",
                    "unique_forecast_id",
                    "source_revision_ahead_of_planning"
                ]),
                &json!(["forecast_evidence"]),
                &json!(["forecast_evidence", "source_revision_ahead_of_planning"])
            ]
        );
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let monitor_before = serde_json::to_value(&monitor).unwrap();
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["forecasts"], report["forecasts"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        assert_eq!(state, before);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), monitor_before);
        state.revision = u64::MAX;
        let maximum = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(maximum["forecasts"][0]["revisions_since_source"], u64::MAX);
        assert_eq!(maximum["forecasts"][4]["revisions_since_source"], 0);
        assert_eq!(
            serde_json::to_value(assess_planning(&PmPlanningState::default(), None)).unwrap()
                ["forecasts"],
            json!([])
        );
    }

    #[test]
    fn decision_revisions_compare_saved_numbers_without_claiming_freshness() {
        let state = PmPlanningState {
            revision: 5,
            decisions: [
                ("same", Some(0)),
                ("", None),
                ("same", Some(5)),
                ("future", Some(6)),
                ("past", Some(3)),
                ("maximum", Some(u64::MAX)),
            ]
            .into_iter()
            .map(|(id, source_revision)| crate::pm_planning::Decision {
                id: id.into(),
                source_revision,
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        let before = state.clone();
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(
            report["decision_revisions"],
            json!([
                {"id":"same", "source_revision":0, "revisions_since_source":5, "missing_inputs":[]},
                {"id":"", "source_revision":null, "revisions_since_source":null, "missing_inputs":["source_revision"]},
                {"id":"same", "source_revision":5, "revisions_since_source":0, "missing_inputs":[]},
                {"id":"future", "source_revision":6, "revisions_since_source":null, "missing_inputs":["source_revision_ahead_of_planning"]},
                {"id":"past", "source_revision":3, "revisions_since_source":2, "missing_inputs":[]},
                {"id":"maximum", "source_revision":u64::MAX, "revisions_since_source":null, "missing_inputs":["source_revision_ahead_of_planning"]}
            ])
        );
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let monitor_before = serde_json::to_value(&monitor).unwrap();
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["decision_revisions"], report["decision_revisions"]);
        assert_eq!(degraded["decisions"], report["decisions"]);
        assert_eq!(degraded["review_dates"], report["review_dates"]);
        assert_eq!(state, before);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), monitor_before);
        let mut maximum = state;
        maximum.revision = u64::MAX;
        let report = serde_json::to_value(assess_planning(&maximum, None)).unwrap();
        assert_eq!(
            report["decision_revisions"][0]["revisions_since_source"],
            u64::MAX
        );
        assert_eq!(
            serde_json::to_value(assess_planning(&PmPlanningState::default(), None)).unwrap()
                ["decision_revisions"],
            json!([])
        );
    }

    #[test]
    fn decision_metadata_reports_missing_fields_without_approving_decisions() {
        let complete = crate::pm_planning::Decision {
            id: "same".into(),
            options: vec!["".into(), "opaque".into()],
            rationale: "unspecified".into(),
            owner: "someone".into(),
            ..Default::default()
        };
        let state = PmPlanningState {
            decisions: vec![
                crate::pm_planning::Decision {
                    id: " \t".into(),
                    options: vec![" \n".into()],
                    rationale: " \t".into(),
                    owner: " \n".into(),
                    ..Default::default()
                },
                crate::pm_planning::Decision {
                    options: vec![],
                    ..complete.clone()
                },
                complete.clone(),
                crate::pm_planning::Decision {
                    id: " same ".into(),
                    ..complete.clone()
                },
                crate::pm_planning::Decision {
                    id: "Same".into(),
                    ..complete
                },
            ],
            ..Default::default()
        };
        let before = state.clone();
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(
            report["decisions"],
            json!([
                {"id":" \t", "missing_inputs":["id", "options", "rationale", "owner"]},
                {"id":"same", "missing_inputs":["unique_decision_id", "options"]},
                {"id":"same", "missing_inputs":["unique_decision_id"]},
                {"id":" same ", "missing_inputs":[]},
                {"id":"Same", "missing_inputs":[]}
            ])
        );
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let monitor_before = serde_json::to_value(&monitor).unwrap();
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["decisions"], report["decisions"]);
        assert_eq!(degraded["review_dates"], report["review_dates"]);
        assert_eq!(state, before);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), monitor_before);
        assert_eq!(
            serde_json::to_value(assess_planning(&PmPlanningState::default(), None)).unwrap()
                ["decisions"],
            json!([])
        );
    }

    #[test]
    fn risk_metadata_reports_missing_fields_without_evaluating_risk() {
        let complete = crate::pm_planning::Risk {
            id: "same".into(),
            severity: "opaque".into(),
            probability: "unspecified".into(),
            impact: "freeform".into(),
            owner: "someone".into(),
            ..Default::default()
        };
        let state = PmPlanningState {
            risks: vec![
                crate::pm_planning::Risk {
                    id: " \t".into(),
                    ..Default::default()
                },
                crate::pm_planning::Risk {
                    probability: " \n".into(),
                    ..complete.clone()
                },
                complete.clone(),
                crate::pm_planning::Risk {
                    id: " same ".into(),
                    ..complete.clone()
                },
                crate::pm_planning::Risk {
                    id: "Same".into(),
                    ..complete
                },
            ],
            ..Default::default()
        };
        let before = state.clone();
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(
            report["risks"],
            json!([
                {"id":" \t", "missing_inputs":["id", "severity", "probability", "impact", "owner"]},
                {"id":"same", "missing_inputs":["unique_risk_id", "probability"]},
                {"id":"same", "missing_inputs":["unique_risk_id"]},
                {"id":" same ", "missing_inputs":[]},
                {"id":"Same", "missing_inputs":[]}
            ])
        );
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let monitor_before = serde_json::to_value(&monitor).unwrap();
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["risks"], report["risks"]);
        assert_eq!(degraded["review_dates"], report["review_dates"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        assert_eq!(state, before);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), monitor_before);
        assert_eq!(
            serde_json::to_value(assess_planning(&PmPlanningState::default(), None)).unwrap()
                ["risks"],
            json!([])
        );
    }

    #[test]
    fn preserves_reported_milestone_values_but_never_invents_evidence() {
        let mut state = fixture();
        state.milestones.push(Default::default());
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        let milestone = &report["milestones"][0];
        assert_eq!(milestone["reported_health"], "green");
        assert_eq!(milestone["reported_confidence"], "high");
        assert_eq!(milestone["health"], "unknown");
        assert_eq!(milestone["confidence"], "unknown");
        assert_eq!(milestone["observed_issue_states"], json!([]));
        assert_eq!(milestone["unobserved_issues"], json!([1]));
        assert!(milestone["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("exit_criteria_evidence")));
        for key in ["target_date", "exit_criteria", "health", "confidence"] {
            assert!(report["milestones"][1]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!(key)));
        }
    }

    #[test]
    fn milestone_target_dates_compare_utc_calendar_days_without_health_verdicts() {
        let state: PmPlanningState = serde_json::from_value(json!({
            "milestones":[
                {"id":"same","target_date":"2024-02-28","health":"green","confidence":"high"},
                {"id":"same","target_date":"2024-03-01"},
                {"id":"future","target_date":"2024-03-02"},
                {"id":"invalid","target_date":"2024-02-30"},
                {"id":"short","target_date":"2024-2-28"},
                {"id":"blank","target_date":" "},
                {"id":"absent"}
            ]
        }))
        .unwrap();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2024-02-29T20:30:00-04:00".into());
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["milestones"].as_array().unwrap();
        assert_eq!(rows.len(), state.milestones.len());
        assert_eq!(
            rows[0]["target_date_observation"],
            json!({
                "scan_date_utc":"2024-03-01", "days_past_target_at_scan":2, "missing_inputs":[]
            })
        );
        for row in &rows[1..3] {
            assert_eq!(
                row["target_date_observation"]["days_past_target_at_scan"],
                0
            );
        }
        for row in &rows[3..] {
            assert_eq!(
                row["target_date_observation"],
                json!({
                    "scan_date_utc":"2024-03-01", "days_past_target_at_scan":null,
                    "missing_inputs":["target_date"]
                })
            );
        }
        assert_eq!(rows[0]["reported_health"], "green");
        assert_eq!(rows[0]["reported_confidence"], "high");
        for (row, saved) in rows.iter().zip(&state.milestones) {
            assert_eq!(row["id"], saved.id);
            assert_eq!(row["target_date"], json!(saved.target_date));
            assert_eq!(row["health"], "unknown");
            assert_eq!(row["confidence"], "unknown");
        }
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.last_scan_at = Some("2024-03-01T00:30:00Z".into());
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["milestones"], report["milestones"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [None, Some("invalid")] {
            monitor.last_scan_at = scan.map(str::to_owned);
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            assert_eq!(
                untimed["milestones"][0]["target_date_observation"],
                json!({
                    "scan_date_utc":null, "days_past_target_at_scan":null,
                    "missing_inputs":["monitor_scan_time"]
                })
            );
        }
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(absent["milestones"][0]["target_date"], "2024-02-28");
        assert_eq!(
            absent["milestones"][0]["target_date_observation"],
            json!({
                "scan_date_utc":null, "days_past_target_at_scan":null,
                "missing_inputs":["monitor_scan_time"]
            })
        );
    }

    #[test]
    fn recurring_runs_compare_saved_instants_without_scheduling_work() {
        let state: PmPlanningState = serde_json::from_value(json!({"recurring_items":[
            {"id":"same", "next_run":"2024-03-01T00:30:00Z",
             "cadence":"daily", "owner":"team", "miss_policy":"skip", "idempotency_key":"tick"},
            {"id":"same", "next_run":"2024-02-29T20:30:00.000000001-04:00",
             "cadence":"\t ", "owner":" ", "idempotency_key":"other"},
            {"id":"", "next_run":"2024-03-01T00:30:00.000000002Z",
             "cadence":"opaque cadence", "owner":"pm", "miss_policy":"manual choice", "idempotency_key":" tick "},
            {"id":" \t ", "next_run":"2024-03-01T00:30:00.0000000011Z", "idempotency_key":" \n "},
            {"id":"invalid", "next_run":"2024-03-01T00:30:00",
             "cadence":"weekly", "owner":"team", "miss_policy":"skip", "idempotency_key":"tick"},
            {"id":"same ", "next_run":""},
            {"id":""},
            {"id":"unique", "cadence":"opaque cadence", "owner":"pm", "miss_policy":"manual choice", "idempotency_key":"unique"}
        ]}))
        .unwrap();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2024-03-01T00:30:00.000000001Z".into());
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["recurring_runs"]
            .as_array()
            .expect("recurring observations");
        let configuration_missing_inputs = [
            json!(["unique_recurring_id", "unique_idempotency_key"]),
            json!(["unique_recurring_id", "cadence", "owner", "miss_policy"]),
            json!(["id"]),
            json!(["id", "cadence", "owner", "miss_policy", "idempotency_key"]),
            json!(["unique_idempotency_key"]),
            json!(["cadence", "owner", "miss_policy", "idempotency_key"]),
            json!(["id", "cadence", "owner", "miss_policy", "idempotency_key"]),
            json!([]),
        ];
        assert_eq!(rows.len(), state.recurring_items.len());
        for (index, item) in state.recurring_items.iter().enumerate() {
            assert_eq!(
                rows[index],
                json!({
                    "id":item.id, "next_run":item.next_run,
                    "configuration_missing_inputs":configuration_missing_inputs[index],
                    "due_at_scan":([Some(true), Some(true), Some(false), None, None, None, None, None][index]),
                    "missing_inputs":(if index < 3 {json!([])} else {json!(["next_run"])})
                })
            );
        }
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["recurring_runs"], report["recurring_runs"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [
            None,
            Some("invalid"),
            Some("2024-03-01T00:30:00.0000000010Z"),
        ] {
            monitor.last_scan_at = scan.map(str::to_owned);
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            for (index, item) in state.recurring_items.iter().enumerate() {
                assert_eq!(
                    untimed["recurring_runs"][index],
                    json!({
                        "id":item.id, "next_run":item.next_run, "due_at_scan":null,
                        "configuration_missing_inputs":configuration_missing_inputs[index],
                        "missing_inputs":(if index < 3 {json!(["monitor_scan_time"])}
                            else {json!(["next_run", "monitor_scan_time"])})
                    })
                );
            }
        }
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(
            absent["recurring_runs"],
            serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap()
                ["recurring_runs"]
        );
        assert_eq!(
            serde_json::to_value(assess_planning(&PmPlanningState::default(), None)).unwrap()
                ["recurring_runs"],
            json!([])
        );
    }

    #[test]
    fn review_dates_observe_saved_risks_and_decisions_without_review_verdicts() {
        let state: PmPlanningState = serde_json::from_value(json!({
            "risks":[
                {"id":"same","review_date":"2024-02-28"},
                {"id":"same","review_date":"2024-03-01"},
                {"id":"future","review_date":"2024-03-02"}
            ],
            "decisions":[
                {"id":"same","review_date":"2024-02-29"},
                {"id":"invalid","review_date":"2024-02-30"},
                {"id":"short","review_date":"2024-2-28"},
                {"id":"blank","review_date":" "},
                {"id":""}
            ]
        }))
        .unwrap();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2024-02-29T20:30:00-04:00".into());
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["review_dates"]
            .as_array()
            .expect("review observations");
        let saved = state
            .risks
            .iter()
            .map(|r| ("risk", &r.id, &r.review_date))
            .chain(
                state
                    .decisions
                    .iter()
                    .map(|d| ("decision", &d.id, &d.review_date)),
            );
        assert_eq!(rows.len(), 8);
        for (index, (kind, id, date)) in saved.enumerate() {
            assert_eq!(
                rows[index],
                json!({
                    "kind":kind, "id":id, "review_date":date, "scan_date_utc":"2024-03-01",
                    "days_past_review_at_scan":([Some(2),Some(0),Some(0),Some(1),None,None,None,None][index]),
                    "missing_inputs":if index < 4 {json!([])} else {json!(["review_date"])}
                })
            );
        }
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.last_scan_at = Some("2024-03-01T00:30:00Z".into());
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["review_dates"], report["review_dates"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [None, Some("invalid")] {
            monitor.last_scan_at = scan.map(str::to_owned);
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            assert_eq!(untimed["review_dates"][0]["scan_date_utc"], Value::Null);
            assert_eq!(
                untimed["review_dates"][0]["days_past_review_at_scan"],
                Value::Null
            );
            assert_eq!(
                untimed["review_dates"][0]["missing_inputs"],
                json!(["monitor_scan_time"])
            );
            assert_eq!(
                untimed["review_dates"][7]["missing_inputs"],
                json!(["review_date", "monitor_scan_time"])
            );
        }
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(
            absent["review_dates"],
            serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap()["review_dates"]
        );
        assert_eq!(absent["review_dates"][0]["review_date"], "2024-02-28");
        assert_eq!(
            serde_json::to_value(assess_planning(&PmPlanningState::default(), None)).unwrap()
                ["review_dates"],
            json!([])
        );
    }

    #[test]
    fn milestone_observations_preserve_exact_states_and_direct_references() {
        let mut state = fixture();
        state.milestones[0].issues = vec![7, 2, 1, 2];
        state.roadmap_items.push(
            serde_json::from_value(json!({
                "id":"member", "milestone_id":"m", "issues":[8, 6, 5, 4, 3, 2]
            }))
            .unwrap(),
        );
        state.roadmap_items.push(
            serde_json::from_value(json!({
                "id":"unrelated", "milestone_id":"other", "issues":[99]
            }))
            .unwrap(),
        );
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.queue = vec![8]; // Queue membership alone is not an inbox observation.
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":1,"state":"blocked_by_claim","attempts":0,
             "dependency":{"blocked_by":[{"number":77},{"number":55},{"number":77}],"cycle":true,"bundle_with":[88]}},
            {"issue_number":2,"state":"needs_human","attempts":0,"error_message":"private reason",
             "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]}},
            {"issue_number":6,"state":"skipped","attempts":0},
            {"issue_number":2,"state":"needs_human","attempts":0,
             "dependency":{"blocked_by":[{"number":77}],"cycle":false,"bundle_with":[]}},
            {"issue_number":2,"state":"source_excluded","attempts":0,
             "dependency":{"blocked_by":[{"number":77}],"cycle":false,"bundle_with":[]}},
            {"issue_number":99,"state":"agent_failed","attempts":0,
             "dependency":{"blocked_by":[{"number":77},{"number":66}],"cycle":false,"bundle_with":[]}},
            {"issue_number":3,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":77},{"number":1}],"cycle":false,"bundle_with":[]}},
            {"issue_number":4,"state":"merged","attempts":0,
             "dependency":{"blocked_by":[{"number":77}],"cycle":false,"bundle_with":[]}},
            {"issue_number":5,"state":"released","attempts":0}
        ]))
        .unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let milestone = &report["milestones"][0];
        assert_eq!(milestone["issues"], json!([7, 2, 1, 2]));
        assert_eq!(
            milestone["observed_issue_states"],
            json!([
                {"issue_number":1,"states":["blocked_by_claim"]},
                {"issue_number":2,"states":["needs_human","source_excluded"]},
                {"issue_number":3,"states":["queued"]},
                {"issue_number":4,"states":["merged"]},
                {"issue_number":5,"states":["released"]},
                {"issue_number":6,"states":["skipped"]}
            ])
        );
        assert_eq!(milestone["unobserved_issues"], json!([7, 8]));
        let dependencies = milestone["dependency_observations"].as_array().unwrap();
        assert_eq!(
            dependencies
                .iter()
                .map(|row| &row["issue_number"])
                .collect::<Vec<_>>(),
            vec![1, 2, 6, 2, 2, 3, 4, 5]
        );
        assert_eq!(
            dependencies[0],
            json!({
                "issue_number":1,"state":"blocked_by_claim",
                "dependency":{"blocked_by":[{"number":77},{"number":55},{"number":77}],"cycle":true,"bundle_with":[88]},
                "missing_inputs":[]
            })
        );
        assert_eq!(
            dependencies[1]["dependency"],
            json!({
                "blocked_by":[],"cycle":false,"bundle_with":[]
            })
        );
        assert_eq!(dependencies[1]["missing_inputs"], json!([]));
        assert_eq!(dependencies[2]["dependency"], Value::Null);
        assert_eq!(
            dependencies[2]["missing_inputs"],
            json!(["dependency_status"])
        );
        // Keep external blockers; filter dependents, without expanding 55 -> 1 -> 3.
        let impacts = json!([
            {"issue_number":1,"dependent_issues":[3]},
            {"issue_number":55,"dependent_issues":[1]},
            {"issue_number":77,"dependent_issues":[1,2,3,4]}
        ]);
        assert_eq!(milestone.get("dependency_impacts"), Some(&impacts));
        assert_eq!(
            report["dependency_impacts"][3]["dependent_issues"],
            json!([1, 2, 3, 4, 99])
        );
        assert_eq!(milestone["health"], "unknown");
        assert_eq!(milestone["confidence"], "unknown");
        assert!(!report.to_string().contains("private reason"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["milestones"], report["milestones"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        assert!(degraded["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_monitor_coverage")));
        monitor.scan_aborted = true;
        let aborted_before = serde_json::to_value(&monitor).unwrap();
        let aborted = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(aborted["milestones"], report["milestones"]);
        assert_eq!(aborted["monitor"]["scan_aborted"], true);
        assert!(aborted["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("successful_monitor_scan")));
        assert_eq!(serde_json::to_value(&monitor).unwrap(), aborted_before);
        monitor.inbox.retain(|row| row.issue_number == 99);
        assert_eq!(monitor.inbox.len(), 1);
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            empty["milestones"][0].get("dependency_observations"),
            Some(&json!([]))
        );
        assert_eq!(
            empty["milestones"][0].get("dependency_impacts"),
            Some(&json!([]))
        );
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(
            absent["milestones"][0].get("dependency_impacts"),
            Some(&Value::Null)
        );
        assert_eq!(
            absent["milestones"][0].get("dependency_observations"),
            Some(&Value::Null)
        );
    }

    #[test]
    fn goal_observations_follow_only_explicit_references_without_metric_claims() {
        let state: PmPlanningState = serde_json::from_value(json!({
            "goals":[{"id":"g"},
                {"id":"other","outcome":"Ship","metric":"adoption","target":"10",
                    "horizon":"quarter","owner":"team","status":"active"},
                {"id":"unlinked","outcome":" ","metric":"\t","target":"\n",
                    "horizon":" ","owner":" ","status":" "}],
            "milestones":[{"id":"m","issues":[99]}],
            "roadmap_items":[
                {"id":"a","goal_id":"g","issues":[2,1,2]},
                {"id":"invalid","goal_id":"g","category":"maintenance","issues":[3]},
                {"id":"milestone-only","milestone_id":"m","issues":[99]},
                {"id":"other","goal_id":"other","issues":[99]},
                {"id":"category-only","category":"compliance","issues":[99]},
                {"id":"last","goal_id":"g","issues":[5]},
                {"id":"whitespace","goal_id":" g ","issues":[99]}
            ]
        }))
        .unwrap();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.queue = vec![5];
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":2,"state":"needs_human","attempts":0,"error_message":"private reason",
             "dependency":{"blocked_by":[{"number":77}],"cycle":true,"bundle_with":[88]}},
            {"issue_number":99,"state":"released","attempts":0},
            {"issue_number":1,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]}},
            {"issue_number":2,"state":"needs_human","attempts":0},
            {"issue_number":2,"state":"merged","attempts":0},
            {"issue_number":3,"state":"source_excluded","attempts":0}
        ]))
        .unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            report["goals"][0],
            json!({
                "id":"g", "roadmap_item_ids":["a","invalid","last"], "issues":[1,2,3,5],
                "dependency_observations":[
                    {"issue_number":2,"state":"needs_human",
                     "dependency":{"blocked_by":[{"number":77}],"cycle":true,"bundle_with":[88]},
                     "missing_inputs":[]},
                    {"issue_number":1,"state":"queued",
                     "dependency":{"blocked_by":[],"cycle":false,"bundle_with":[]},"missing_inputs":[]},
                    {"issue_number":2,"state":"needs_human","dependency":null,"missing_inputs":["dependency_status"]},
                    {"issue_number":2,"state":"merged","dependency":null,"missing_inputs":["dependency_status"]},
                    {"issue_number":3,"state":"source_excluded","dependency":null,"missing_inputs":["dependency_status"]}
                ],
                "observed_issue_states":[
                    {"issue_number":1,"states":["queued"]},
                    {"issue_number":2,"states":["needs_human","merged"]},
                    {"issue_number":3,"states":["source_excluded"]}
                ],
                "unobserved_issues":[5], "missing_inputs":["metric_evidence",
                    "outcome","metric","target","horizon","owner","status"]
            })
        );
        assert_eq!(
            report["goals"][1]["missing_inputs"],
            json!(["metric_evidence"])
        );
        assert_eq!(report["goals"][2]["roadmap_item_ids"], json!([]));
        assert_eq!(
            report["goals"][2]["missing_inputs"],
            report["goals"][0]["missing_inputs"]
        );
        assert!(!report.to_string().contains("private reason"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(absent["goals"][0]["observed_issue_states"], json!([]));
        assert_eq!(
            absent["goals"][0].get("dependency_observations"),
            Some(&Value::Null)
        );
        assert_eq!(absent["goals"][0]["unobserved_issues"], json!([1, 2, 3, 5]));
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(degraded["goals"], report["goals"]);
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        assert!(degraded["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_monitor_coverage")));
        monitor.inbox.retain(|row| row.issue_number == 99);
        let unrelated = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            unrelated["goals"][0].get("dependency_observations"),
            Some(&json!([]))
        );
    }

    #[test]
    fn ambiguous_or_empty_goal_ids_resolve_no_roadmap_references() {
        let state: PmPlanningState = serde_json::from_value(json!({
            "goals":[{"id":"duplicate"},{"id":"duplicate"},{"id":" "}],
            "roadmap_items":[{"id":"a","goal_id":"duplicate","issues":[1]},
                {"id":"b","goal_id":" ","issues":[2]}]
        }))
        .unwrap();
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        let goals = report["goals"].as_array().expect("goal diagnostics");
        assert_eq!(goals.len(), 3);
        for goal in goals {
            assert_eq!(goal.get("dependency_observations"), Some(&Value::Null));
            for key in [
                "roadmap_item_ids",
                "issues",
                "observed_issue_states",
                "unobserved_issues",
            ] {
                assert_eq!(goal[key], json!([]));
            }
            assert!(goal["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!("metric_evidence")));
        }
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":1,"state":"queued","attempts":0},
            {"issue_number":2,"state":"queued","attempts":0}
        ]))
        .unwrap();
        let supplied = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        for goal in supplied["goals"].as_array().unwrap() {
            assert_eq!(goal.get("dependency_observations"), Some(&json!([])));
        }
        assert_eq!(
            report["goals"][0]["missing_inputs"],
            json!([
                "metric_evidence",
                "unique_goal_id",
                "outcome",
                "metric",
                "target",
                "horizon",
                "owner",
                "status"
            ])
        );
        assert_eq!(
            report["goals"][2]["missing_inputs"],
            json!([
                "metric_evidence",
                "id",
                "outcome",
                "metric",
                "target",
                "horizon",
                "owner",
                "status"
            ])
        );
    }

    #[test]
    fn ambiguous_or_empty_milestone_ids_do_not_expand_roadmap_membership() {
        let state: PmPlanningState = serde_json::from_value(json!({
            "milestones":[{"id":"duplicate","issues":[1]},
                {"id":"duplicate","issues":[2]}, {"id":"","issues":[3]}],
            "roadmap_items":[{"id":"a","milestone_id":"duplicate","issues":[99]},
                {"id":"b","milestone_id":"","issues":[99]}]
        }))
        .unwrap();
        let report = serde_json::to_value(assess_planning(&state, None)).unwrap();
        for (index, number) in [1, 2, 3].into_iter().enumerate() {
            assert_eq!(
                report["milestones"][index]["unobserved_issues"],
                json!([number])
            );
        }
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":1,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":77}],"cycle":false,"bundle_with":[]}},
            {"issue_number":2,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":77}],"cycle":false,"bundle_with":[]}},
            {"issue_number":3,"state":"queued","attempts":0,
             "dependency":{"blocked_by":[{"number":77}],"cycle":false,"bundle_with":[]}},
            {"issue_number":99,"state":"blocked_by_dependency","attempts":0,
             "dependency":{"blocked_by":[{"number":66},{"number":77}],"cycle":false,"bundle_with":[]}}
        ]))
        .unwrap();
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        for (index, number) in [1, 2, 3].into_iter().enumerate() {
            let rows = report["milestones"][index]["dependency_observations"]
                .as_array()
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["issue_number"], number);
            assert_eq!(
                report["milestones"][index].get("dependency_impacts"),
                Some(&json!([{"issue_number":77,"dependent_issues":[number]}]))
            );
        }
    }

    #[test]
    fn capacity_preserves_admission_facts_without_claiming_full_wip() {
        let state = fixture();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.active_launches = vec![3, 1, 3];
        monitor.review_windows = serde_json::from_value(json!([
            {"issue_number":3,"pr_number":30,"dispatched_at":"2026-10-21T00:00:00Z"},
            {"issue_number":8,"pr_number":80,"dispatched_at":"2026-10-21T00:00:00Z"},
            {"issue_number":8,"pr_number":81,"dispatched_at":"2026-10-21T00:00:00Z"}
        ]))
        .unwrap();
        monitor.pending_claim_issues = Some(vec![9, 3, 9]);
        monitor.needs_human = vec![10, 10];
        monitor.queue = vec![20];
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":11,"state":"needs_human","attempts":0,"error_message":"private reason"},
            {"issue_number":99,"state":"merged","attempts":0}
        ]))
        .unwrap();
        monitor.occupied_slot_count = Some(17); // Includes unlisted/unknown panes.
        monitor.max_active = 4;
        monitor.last_scan_at = Some("2026-10-21T00:00:00Z".into());
        monitor.inbox_coverage = Some(serde_json::from_value(json!({
            "github_target_count":2,"inbox_row_count":2,"missing_issue_numbers":[],"source":"live"
        })).unwrap());
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            report["capacity"],
            json!({
                "implementation_launches":[3,1,3], "review_issues":[3,8,8],
                "pending_claim_issues":[9,3,9], "needs_human_issues":[10,11],
                "occupied_slot_count":17, "max_active":4,
                "observed_work_issues":[1,3,8,9,10,11],
                "launch_activity":[],
                "claim_blocks":[],
                "declared_waits":[],
                "retry_deadlines":[],
                "wip_limits":[],
                "idle_windows":[],
                "terminal_queue":[],
                "missing_inputs":["verification_observation","end_to_end_wip"]
            })
        );
        assert!(!report.to_string().contains("private reason"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["observed_work_issues"],
            report["capacity"]["observed_work_issues"]
        );
        assert!(degraded["capacity"]["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_monitor_coverage")));
        monitor.active_launches.clear();
        monitor.review_windows.clear();
        monitor.pending_claim_issues = None;
        monitor.needs_human.clear();
        monitor.inbox.clear();
        monitor.occupied_slot_count = None;
        let legacy = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(legacy["capacity"]["implementation_launches"], json!([]));
        assert_eq!(legacy["capacity"]["observed_work_issues"], json!([]));
        for key in ["pending_claim_issues", "occupied_slot_count"] {
            assert_eq!(legacy["capacity"][key], Value::Null);
            assert!(legacy["capacity"]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!(key)));
        }
        monitor.source = IssueMonitorStatusSource::Daemon;
        monitor.active_launches_incomplete = false;
        monitor.pending_claim_issues = Some(vec![]);
        monitor.occupied_slot_count = Some(0);
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(empty["capacity"]["occupied_slot_count"], 0);
        assert_eq!(empty["capacity"]["pending_claim_issues"], json!([]));
        assert_eq!(
            empty["capacity"]["missing_inputs"],
            json!(["verification_observation", "end_to_end_wip"])
        );
        monitor.last_scan_at = None;
        let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert!(untimed["capacity"]["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("monitor_scan_time")));
    }

    #[test]
    fn wip_limits_compare_observed_owners_without_certifying_full_wip() {
        let mut state = fixture();
        state.policies = serde_json::from_value(json!([
            {"id":"small","version":3,"wip_limit":1},
            {"id":"equal","wip_limit":2}, {"id":"large","wip_limit":3},
            {"id":"unset"}, {"id":"zero","wip_limit":0},
            {"id":" ","wip_limit":0},
            {"id":"dup","version":1,"wip_limit":0},
            {"id":"dup","version":2,"wip_limit":0}
        ]))
        .unwrap();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.active_launches = vec![8, 2, 8];
        monitor.queue = vec![9];
        monitor.occupied_slot_count = Some(17);
        monitor.last_scan_at = Some("2026-10-21T00:00:00Z".into());
        monitor.inbox_coverage = Some(serde_json::from_value(json!({
            "github_target_count":0,"inbox_row_count":0,"missing_issue_numbers":[],"source":"live"
        })).unwrap());
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let evaluate = |monitor: Option<&IssueMonitorAgentStatus>| {
            serde_json::to_value(assess_planning(&state, monitor)).unwrap()
        };
        let report = evaluate(Some(&monitor));
        let rows = report["capacity"]["wip_limits"].as_array().unwrap();
        assert_eq!(
            rows[0],
            json!({"id":"small","version":3,"wip_limit":1,
            "observed_issue_count":2,"assessment":"observed_exceeded",
            "missing_inputs":["verification_observation","end_to_end_wip"]})
        );
        assert_eq!(
            rows.iter()
                .map(|row| row["assessment"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "observed_exceeded",
                "unknown",
                "unknown",
                "unknown",
                "observed_exceeded",
                "unknown",
                "unknown",
                "unknown"
            ]
        );
        for (index, diagnostic) in [
            (3, "wip_limit"),
            (5, "policy_id"),
            (6, "unique_policy_id"),
            (7, "unique_policy_id"),
        ] {
            assert!(rows[index]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!(diagnostic)));
        }
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        let absent = evaluate(None);
        assert_eq!(
            absent["capacity"]["wip_limits"].as_array().unwrap().len(),
            8
        );
        assert_eq!(
            absent["capacity"]["wip_limits"][0]["observed_issue_count"],
            Value::Null
        );
        assert_eq!(absent["capacity"]["wip_limits"][0]["assessment"], "unknown");
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = evaluate(Some(&monitor));
        assert_eq!(
            degraded["capacity"]["wip_limits"][0]["assessment"],
            "unknown"
        );
        assert!(degraded["capacity"]["wip_limits"][0]["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_monitor_coverage")));
        monitor.source = IssueMonitorStatusSource::Daemon;
        monitor.active_launches_incomplete = true;
        assert_eq!(
            evaluate(Some(&monitor))["capacity"]["wip_limits"][0]["assessment"],
            "unknown"
        );
        monitor.active_launches_incomplete = false;
        for scan in [
            None,
            Some("invalid"),
            Some("2026-10-21T00:00:00.1234567891Z"),
        ]
        .map(|value| value.map(str::to_owned))
        {
            monitor.last_scan_at = scan;
            let before = serde_json::to_value(&monitor).unwrap();
            let untimed = evaluate(Some(&monitor));
            assert_eq!(
                untimed["capacity"]["wip_limits"][0]["assessment"],
                "unknown"
            );
            assert!(untimed["capacity"]["wip_limits"][0]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!("monitor_scan_time")));
            assert_eq!(
                untimed["monitor"]["last_scan_at"],
                json!(monitor.last_scan_at)
            );
            assert_eq!(
                untimed["capacity"]["wip_limits"][0]["observed_issue_count"],
                2
            );
            assert_eq!(serde_json::to_value(&monitor).unwrap(), before);
        }
        monitor.last_scan_at = Some("2026-10-21T00:00:00.123456789Z".into());
        assert_eq!(
            evaluate(Some(&monitor))["capacity"]["wip_limits"][0]["assessment"],
            "observed_exceeded"
        );
        monitor.active_launches.clear();
        assert_eq!(
            evaluate(Some(&monitor))["capacity"]["wip_limits"][4]["assessment"],
            "unknown"
        );
    }

    #[test]
    fn idle_windows_preserve_monitor_facts_with_scan_relative_aging() {
        let state = fixture();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2026-10-21T00:01:00Z".into());
        let window = |id: &str, issue, kind, since, bound, pane_present| {
            json!({
                "window_id":id, "issue_number":issue, "idle_kind":kind,
                "idle_since":since, "bound":bound, "pane_present":pane_present
            })
        };
        monitor.idle_windows = serde_json::from_value(json!([
            window(
                "same",
                Some(9),
                "stuck_unknown",
                "2026-10-21T09:00:00.000000001+09:00",
                true,
                true
            ),
            window(
                "same",
                Some(9),
                "binding_pending",
                "2026-10-21T09:01:00+09:00",
                true,
                false
            ),
            window(
                "orphan",
                None,
                "execution_settled",
                "invalid private timestamp",
                false,
                true
            ),
            window(
                "future",
                Some(3),
                "binding_dead",
                "2026-10-21T00:01:00.000000001Z",
                false,
                false
            ),
            window(
                "review",
                Some(3),
                "review_verdict_published",
                "2026-10-21T00:00:00Z",
                true,
                true
            ),
            window(
                "unsupported precision",
                Some(3),
                "binding_dead",
                "2026-10-21T00:01:00.0000000001Z",
                false,
                false
            )
        ]))
        .unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let evaluate = |monitor: Option<&IssueMonitorAgentStatus>| {
            serde_json::to_value(assess_planning(&state, monitor)).unwrap()
        };
        let report = evaluate(Some(&monitor));
        let rows = report["capacity"]["idle_windows"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (&row["window_id"], &row["idle_seconds_at_scan"]))
                .collect::<Vec<_>>(),
            vec![
                (&json!("same"), &json!(59)),
                (&json!("same"), &json!(0)),
                (&json!("orphan"), &Value::Null),
                (&json!("future"), &Value::Null),
                (&json!("review"), &json!(60)),
                (&json!("unsupported precision"), &Value::Null)
            ]
        );
        for (index, row) in rows.iter().enumerate() {
            for key in [
                "window_id",
                "issue_number",
                "idle_kind",
                "bound",
                "pane_present",
            ] {
                assert_eq!(row[key], before.1["idle_windows"][index][key], "{key}");
            }
        }
        assert_eq!(rows[0]["idle_since"], "2026-10-21T09:00:00.000000001+09:00");
        assert_eq!(rows[2]["idle_since"], Value::Null);
        assert_eq!(rows[2]["missing_inputs"], json!(["idle_since"]));
        assert_eq!(rows[3]["missing_inputs"], json!(["idle_after_scan"]));
        assert_eq!(rows[5]["idle_since"], Value::Null);
        assert_eq!(rows[5]["missing_inputs"], json!(["idle_since"]));
        assert!(!report.to_string().contains("invalid private timestamp"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        assert_eq!(
            evaluate(None)["capacity"].get("idle_windows"),
            Some(&Value::Null)
        );
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        let degraded = evaluate(Some(&monitor));
        assert_eq!(
            degraded["capacity"]["idle_windows"],
            report["capacity"]["idle_windows"]
        );
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [
            None,
            Some("invalid"),
            Some("2026-10-21T00:01:00.0000000001Z"),
        ]
        .map(|value| value.map(str::to_owned))
        {
            monitor.last_scan_at = scan;
            let untimed = evaluate(Some(&monitor));
            let first = &untimed["capacity"]["idle_windows"][0];
            assert_eq!(first["idle_seconds_at_scan"], Value::Null);
            assert_eq!(first["missing_inputs"], json!(["monitor_scan_time"]));
        }
        monitor.idle_windows.clear();
        assert_eq!(
            evaluate(Some(&monitor))["capacity"]["idle_windows"],
            json!([])
        );
    }

    #[test]
    fn launch_activity_is_relative_to_scan_without_inventing_liveness() {
        let state = fixture();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2026-10-21T00:01:00Z".into());
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":9,"state":"launched","attempts":0,"last_activity_at":"2026-10-21T09:00:00+09:00"},
            {"issue_number":3,"state":"launching","attempts":0,"last_activity_at":"2026-10-21T00:00:59.100Z"},
            {"issue_number":9,"state":"launched","attempts":0,"last_activity_at":"2026-10-21T00:01:00Z"},
            {"issue_number":4,"state":"launched","attempts":0},
            {"issue_number":5,"state":"launched","attempts":0,"last_activity_at":"private invalid timestamp"},
            {"issue_number":6,"state":"launched","attempts":0,"last_activity_at":"2026-10-21T00:01:00.100Z"},
            {"issue_number":10,"state":"launched","attempts":0,"last_activity_at":"2026-10-21T00:01:00.0000000001Z"},
            {"issue_number":11,"state":"launched","attempts":0,"last_activity_at":"2026-10-21T00:01:00.000000001Z"},
            {"issue_number":7,"state":"queued","attempts":0,"last_activity_at":"2026-10-21T00:00:00Z"},
            {"issue_number":8,"state":"needs_human","attempts":0,"error_message":"private reason"}
        ])).unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["capacity"]["launch_activity"].as_array().unwrap();
        let facts: Vec<_> = rows
            .iter()
            .map(|row| {
                json!([
                    row["issue_number"],
                    row["state"],
                    row["inactivity_seconds_at_scan"],
                    row["missing_inputs"]
                ])
            })
            .collect();
        assert_eq!(
            json!(facts),
            json!([
                [9, "launched", 60, []],
                [3, "launching", 0, []],
                [9, "launched", 0, []],
                [4, "launched", null, ["last_activity_at"]],
                [5, "launched", null, ["last_activity_at"]],
                [6, "launched", null, ["activity_after_scan"]],
                [10, "launched", null, ["last_activity_at"]],
                [11, "launched", null, ["activity_after_scan"]]
            ])
        );
        assert_eq!(rows[0]["last_activity_at"], "2026-10-21T09:00:00+09:00");
        assert_eq!(rows[4]["last_activity_at"], Value::Null);
        assert_eq!(rows[6]["last_activity_at"], Value::Null);
        assert!(!report.to_string().contains("private"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["launch_activity"],
            report["capacity"]["launch_activity"]
        );
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [
            None,
            Some("invalid scan".into()),
            Some("2026-10-21T00:00:59.9999999999Z".into()),
        ] {
            monitor.last_scan_at = scan;
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            let row = &untimed["capacity"]["launch_activity"][0];
            assert_eq!(row["inactivity_seconds_at_scan"], Value::Null);
            assert_eq!(row["missing_inputs"], json!(["monitor_scan_time"]));
        }
    }

    #[test]
    fn claim_blocks_report_deadlines_without_implying_release_or_blocked_age() {
        let state = fixture();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2026-10-21T00:01:00Z".into());
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":9,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T09:02:00+09:00","blocked_by_owner":"private owner"},
            {"issue_number":3,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:01:00.999999999Z"},
            {"issue_number":9,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:01:00Z"},
            {"issue_number":4,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:00:59.900Z"},
            {"issue_number":5,"state":"blocked_by_claim","attempts":0},
            {"issue_number":6,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"private invalid timestamp","exclusion_reason":"private reason"},
            {"issue_number":7,"state":"queued","attempts":0,"claim_expires_at":"2026-10-21T00:02:00Z"},
            {"issue_number":8,"state":"launched","attempts":0,"claim_expires_at":"2026-10-21T00:02:00Z"},
            {"issue_number":9,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:01:00.0000000001Z"}
        ])).unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["capacity"]["claim_blocks"].as_array().unwrap();
        let facts: Vec<_> = rows
            .iter()
            .map(|row| {
                json!([
                    row["issue_number"],
                    row["remaining_seconds_at_scan"],
                    row["missing_inputs"]
                ])
            })
            .collect();
        assert_eq!(
            json!(facts),
            json!([
                [9, 60, ["claim_block_started_at"]],
                [3, 0, ["claim_block_started_at"]],
                [9, 0, ["claim_block_started_at"]],
                [4, 0, ["claim_block_started_at"]],
                [5, null, ["claim_block_started_at", "claim_expires_at"]],
                [6, null, ["claim_block_started_at", "claim_expires_at"]],
                [9, null, ["claim_block_started_at", "claim_expires_at"]]
            ])
        );
        assert_eq!(rows[0]["claim_expires_at"], "2026-10-21T09:02:00+09:00");
        assert_eq!(rows[5]["claim_expires_at"], Value::Null);
        assert_eq!(rows[6]["claim_expires_at"], Value::Null);
        assert!(!report.to_string().contains("private"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["claim_blocks"],
            report["capacity"]["claim_blocks"]
        );
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [
            None,
            Some("invalid scan".into()),
            Some("2026-10-21T00:01:00.0000000001Z".into()),
        ] {
            monitor.last_scan_at = scan;
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            assert_eq!(
                untimed["capacity"]["claim_blocks"][0]["remaining_seconds_at_scan"],
                Value::Null
            );
            assert_eq!(
                untimed["capacity"]["claim_blocks"][0]["missing_inputs"],
                json!(["claim_block_started_at", "monitor_scan_time"])
            );
        }
        monitor.inbox.clear();
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(empty["capacity"]["claim_blocks"], json!([]));
    }

    #[test]
    fn declared_waits_preserve_protection_and_measure_only_snapshot_time() {
        let state = fixture();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2026-10-21T00:01:00Z".into());
        let wait = |number, state, since, expires, in_force| {
            json!({
                "issue_number":number,"state":state,"attempts":0,
                "waiting":{"reason":"private reason","resume_condition":"private resume",
                    "since":since,"expires_at":expires,"in_force":in_force}
            })
        };
        monitor.inbox = serde_json::from_value(json!([
            wait(9,"launched","2026-10-21T09:00:00+09:00","2026-10-21T00:02:00Z",false),
            wait(3,"launching","2026-10-21T00:00:59.100Z","2026-10-21T00:01:00.900Z",true),
            wait(9,"launched","2026-10-21T00:00:00Z","2026-10-21T00:00:59.900Z",true),
            wait(4,"launched","private invalid since","2026-10-21T00:02:00Z",false),
            wait(5,"launched","2026-10-21T00:00:00Z","private invalid expiry",false),
            wait(6,"launched","2026-10-21T00:01:00.000000001Z","2026-10-21T00:02:00Z",true),
            wait(7,"launched","2026-10-21T00:00:00Z","2026-10-20T23:59:59Z",false),
            wait(8,"queued","2026-10-21T00:00:00Z","2026-10-21T00:02:00Z",true),
            {"issue_number":10,"state":"launched","attempts":0},
            wait(11,"launched","2026-10-21T00:01:00.0000000001Z","2026-10-21T00:02:00Z",true),
            wait(12,"launched","2026-10-21T00:00:00Z","2026-10-21T00:01:00.0000000001Z",true)
        ]))
        .unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["capacity"]["declared_waits"].as_array().unwrap();
        let facts: Vec<_> = rows
            .iter()
            .map(|row| {
                json!([
                    row["issue_number"],
                    row["in_force"],
                    row["elapsed_seconds_at_scan"],
                    row["remaining_seconds_at_scan"],
                    row["missing_inputs"]
                ])
            })
            .collect();
        assert_eq!(
            json!(facts),
            json!([
                [9, false, 60, 60, []],
                [3, true, 0, 0, []],
                [9, true, 60, 0, []],
                [4, false, null, 60, ["wait_since"]],
                [5, false, 60, null, ["wait_expires_at"]],
                [6, true, null, 60, ["wait_after_scan"]],
                [7, false, 60, null, ["wait_interval"]],
                [11, true, null, 60, ["wait_since"]],
                [12, true, 60, null, ["wait_expires_at"]]
            ])
        );
        assert_eq!(rows[0]["since"], "2026-10-21T09:00:00+09:00");
        assert_eq!(rows[3]["since"], Value::Null);
        assert_eq!(rows[4]["expires_at"], Value::Null);
        assert_eq!(rows[7]["since"], Value::Null);
        assert_eq!(rows[8]["expires_at"], Value::Null);
        assert!(!report.to_string().contains("private"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["declared_waits"],
            report["capacity"]["declared_waits"]
        );
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [
            None,
            Some("invalid scan".into()),
            Some("2026-10-21T00:01:00.0000000001Z".into()),
        ] {
            monitor.last_scan_at = scan;
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            let first = &untimed["capacity"]["declared_waits"][0];
            assert_eq!(first["elapsed_seconds_at_scan"], Value::Null);
            assert_eq!(first["remaining_seconds_at_scan"], Value::Null);
            assert_eq!(first["missing_inputs"], json!(["monitor_scan_time"]));
        }
        monitor.inbox.clear();
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(empty["capacity"]["declared_waits"], json!([]));
    }

    #[test]
    fn retry_deadlines_describe_supplied_timing_without_evaluating_holds() {
        let state = fixture();
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.last_scan_at = Some("2026-10-21T00:01:00Z".into());
        monitor.inbox = serde_json::from_value(json!([
            {"issue_number":9,"state":"queued","attempts":0,
                "retry_not_before":"2026-10-21T09:02:00+09:00","retry_hold_reason":"private reason"},
            {"issue_number":3,"state":"needs_human","attempts":0,
                "retry_not_before":"2026-10-21T00:01:00.999999999Z"},
            {"issue_number":9,"state":"launched","attempts":0,
                "retry_not_before":"2026-10-21T00:00:59.900Z"},
            {"issue_number":4,"state":"queued","attempts":0,"retry_not_before":"unknown"},
            {"issue_number":5,"state":"queued","attempts":0,"retry_not_before":"private invalid time"},
            {"issue_number":6,"state":"queued","attempts":0},
            {"issue_number":7,"state":"queued","attempts":0,
                "retry_not_before":"2026-10-21T00:01:00.0000000001Z"}
        ])).unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        let rows = report["capacity"]["retry_deadlines"].as_array().unwrap();
        let facts: Vec<_> = rows
            .iter()
            .map(|row| {
                json!([
                    row["issue_number"],
                    row["state"],
                    row["remaining_seconds_at_scan"],
                    row["missing_inputs"]
                ])
            })
            .collect();
        assert_eq!(
            json!(facts),
            json!([
                [9, "queued", 60, ["retry_hold_started_at"]],
                [3, "needs_human", 0, ["retry_hold_started_at"]],
                [9, "launched", 0, ["retry_hold_started_at"]],
                [
                    4,
                    "queued",
                    null,
                    ["retry_hold_started_at", "retry_not_before"]
                ],
                [
                    5,
                    "queued",
                    null,
                    ["retry_hold_started_at", "retry_not_before"]
                ],
                [
                    7,
                    "queued",
                    null,
                    ["retry_hold_started_at", "retry_not_before"]
                ]
            ])
        );
        assert_eq!(rows[0]["retry_not_before"], "2026-10-21T09:02:00+09:00");
        assert_eq!(rows[3]["retry_not_before"], Value::Null);
        assert_eq!(rows[5]["retry_not_before"], Value::Null);
        assert!(!report.to_string().contains("private"));
        assert_eq!(state, before.0);
        assert_eq!(serde_json::to_value(&monitor).unwrap(), before.1);
        monitor.source = IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        let degraded = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            degraded["capacity"]["retry_deadlines"],
            report["capacity"]["retry_deadlines"]
        );
        assert_eq!(degraded["monitor"]["source"], "degraded_cache");
        for scan in [
            None,
            Some("invalid scan".into()),
            Some("2026-10-21T00:01:00.0000000001Z".into()),
        ] {
            monitor.last_scan_at = scan;
            let untimed = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
            assert_eq!(
                untimed["capacity"]["retry_deadlines"][0]["remaining_seconds_at_scan"],
                Value::Null
            );
            assert_eq!(
                untimed["capacity"]["retry_deadlines"][0]["missing_inputs"],
                json!(["retry_hold_started_at", "monitor_scan_time"])
            );
        }
        monitor.inbox.clear();
        let empty = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(empty["capacity"]["retry_deadlines"], json!([]));
    }

    #[test]
    fn absent_and_degraded_monitor_observations_never_prove_complete_coverage() {
        let state = fixture();
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(absent["capacity"].get("idle_windows"), Some(&Value::Null));
        assert_eq!(absent["capacity"].get("claim_blocks"), Some(&Value::Null));
        assert_eq!(absent["capacity"].get("declared_waits"), Some(&Value::Null));
        assert_eq!(
            absent["capacity"].get("retry_deadlines"),
            Some(&Value::Null)
        );
        assert_eq!(row(&absent, 1)["observed_inbox_states"], json!([]));
        assert_eq!(absent["monitor"]["source"], Value::Null);
        assert_eq!(absent["monitor"]["active_launches_incomplete"], true);
        for key in [
            "implementation_launches",
            "review_issues",
            "pending_claim_issues",
            "needs_human_issues",
            "occupied_slot_count",
            "max_active",
            "launch_activity",
        ] {
            assert_eq!(absent["capacity"][key], Value::Null);
        }
        assert_eq!(absent["capacity"]["observed_work_issues"], json!([]));
        for key in [
            "monitor_observation",
            "verification_observation",
            "end_to_end_wip",
        ] {
            assert!(absent["capacity"]["missing_inputs"]
                .as_array()
                .unwrap()
                .contains(&json!(key)));
        }
        assert!(absent["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("monitor_observation")));
        let mut monitor = crate::IssueMonitorState::new(Default::default()).agent_status();
        monitor.source = crate::IssueMonitorStatusSource::DegradedCache;
        monitor.active_launches_incomplete = true;
        monitor.last_scan_at = Some("2026-10-19T14:00:00Z".into());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(report["monitor"]["source"], "degraded_cache");
        assert_eq!(report["monitor"]["last_scan_at"], "2026-10-19T14:00:00Z");
        assert!(report["missing_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_monitor_coverage")));
    }
}
