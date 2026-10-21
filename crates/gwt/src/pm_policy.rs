//! Read-only goal and roadmap diagnostics (SPEC #3431 T-113a/T-114a).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::{
    issue_monitor::IssueMonitorInboxCoverage,
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
    pub issue_traces: Vec<IssueTrace>,
    pub roadmap: RoadmapHorizons,
    pub roadmap_items: Vec<RoadmapDiagnostic>,
    pub goals: Vec<GoalDiagnostic>,
    pub milestones: Vec<MilestoneDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct MonitorEvidence {
    pub source: Option<IssueMonitorStatusSource>,
    pub last_scan_at: Option<String>,
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
    pub missing_inputs: Vec<&'static str>,
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
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
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
            let since = chrono::DateTime::parse_from_rfc3339(&wait.since).ok();
            let expiry = chrono::DateTime::parse_from_rfc3339(&wait.expires_at).ok();
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
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
    status
        .inbox
        .iter()
        .filter(|row| row.state == MonitorInboxState::BlockedByClaim)
        .map(|row| {
            let expiry = row
                .claim_expires_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
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
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
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
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
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
    let observed_work_issues = [
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
    pub diagnostics: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct GoalDiagnostic {
    pub id: String,
    pub roadmap_item_ids: Vec<String>,
    pub issues: Vec<u64>,
    pub observed_issue_states: Vec<MilestoneIssueStates>,
    pub unobserved_issues: Vec<u64>,
    pub missing_inputs: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct MilestoneDiagnostic {
    pub id: String,
    pub issues: Vec<u64>,
    pub observed_issue_states: Vec<MilestoneIssueStates>,
    pub unobserved_issues: Vec<u64>,
    pub target_date: Option<String>,
    pub exit_criteria: Vec<String>,
    pub reported_health: Option<String>,
    pub reported_confidence: Option<String>,
    pub health: &'static str,
    pub confidence: &'static str,
    pub missing_inputs: Vec<&'static str>,
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
    } else {
        missing_inputs.push("monitor_observation");
        missing_inputs.push("complete_monitor_coverage");
    }
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
            RoadmapDiagnostic {
                id: item.id.clone(),
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
                issues: milestone.issues.clone(),
                observed_issue_states,
                unobserved_issues,
                target_date: milestone.target_date.clone(),
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
        capacity: observe_capacity(monitor, &missing_inputs),
        monitor: MonitorEvidence {
            source: monitor.map(|status| status.source),
            last_scan_at: monitor.and_then(|status| status.last_scan_at.clone()),
            active_launches_incomplete: monitor
                .is_none_or(|status| status.active_launches_incomplete),
            inbox_coverage: monitor.and_then(|status| status.inbox_coverage.clone()),
        },
        missing_inputs,
        issue_traces,
        roadmap,
        roadmap_items,
        goals,
        milestones,
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
            {"issue_number":2,"state":"hold_excluded","attempts":0},
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
            json!(["needs_human", "hold_excluded"])
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
            {"issue_number":1,"state":"blocked_by_claim","attempts":0},
            {"issue_number":2,"state":"needs_human","attempts":0,"error_message":"private reason"},
            {"issue_number":2,"state":"needs_human","attempts":0},
            {"issue_number":2,"state":"hold_excluded","attempts":0},
            {"issue_number":3,"state":"queued","attempts":0},
            {"issue_number":4,"state":"merged","attempts":0},
            {"issue_number":5,"state":"released","attempts":0},
            {"issue_number":6,"state":"skipped","attempts":0},
            {"issue_number":99,"state":"agent_failed","attempts":0}
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
                {"issue_number":2,"states":["needs_human","hold_excluded"]},
                {"issue_number":3,"states":["queued"]},
                {"issue_number":4,"states":["merged"]},
                {"issue_number":5,"states":["released"]},
                {"issue_number":6,"states":["skipped"]}
            ])
        );
        assert_eq!(milestone["unobserved_issues"], json!([7, 8]));
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
            {"issue_number":1,"state":"queued","attempts":0},
            {"issue_number":2,"state":"needs_human","attempts":0,"error_message":"private reason"},
            {"issue_number":2,"state":"needs_human","attempts":0},
            {"issue_number":2,"state":"merged","attempts":0},
            {"issue_number":3,"state":"hold_excluded","attempts":0},
            {"issue_number":99,"state":"released","attempts":0}
        ]))
        .unwrap();
        let before = (state.clone(), serde_json::to_value(&monitor).unwrap());
        let report = serde_json::to_value(assess_planning(&state, Some(&monitor))).unwrap();
        assert_eq!(
            report["goals"][0],
            json!({
                "id":"g", "roadmap_item_ids":["a","invalid","last"], "issues":[1,2,3,5],
                "observed_issue_states":[
                    {"issue_number":1,"states":["queued"]},
                    {"issue_number":2,"states":["needs_human","merged"]},
                    {"issue_number":3,"states":["hold_excluded"]}
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
                [6, "launched", null, ["activity_after_scan"]]
            ])
        );
        assert_eq!(rows[0]["last_activity_at"], "2026-10-21T09:00:00+09:00");
        assert_eq!(rows[4]["last_activity_at"], Value::Null);
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
        for scan in [None, Some("invalid scan".into())] {
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
            {"issue_number":3,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:01:00.900Z"},
            {"issue_number":9,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:01:00Z"},
            {"issue_number":4,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"2026-10-21T00:00:59.900Z"},
            {"issue_number":5,"state":"blocked_by_claim","attempts":0},
            {"issue_number":6,"state":"blocked_by_claim","attempts":0,"claim_expires_at":"private invalid timestamp","exclusion_reason":"private reason"},
            {"issue_number":7,"state":"queued","attempts":0,"claim_expires_at":"2026-10-21T00:02:00Z"},
            {"issue_number":8,"state":"launched","attempts":0,"claim_expires_at":"2026-10-21T00:02:00Z"}
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
                [6, null, ["claim_block_started_at", "claim_expires_at"]]
            ])
        );
        assert_eq!(rows[0]["claim_expires_at"], "2026-10-21T09:02:00+09:00");
        assert_eq!(rows[5]["claim_expires_at"], Value::Null);
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
        for scan in [None, Some("invalid scan".into())] {
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
            wait(6,"launched","2026-10-21T00:01:00.100Z","2026-10-21T00:02:00Z",true),
            wait(7,"launched","2026-10-21T00:00:00Z","2026-10-20T23:59:59Z",false),
            wait(8,"queued","2026-10-21T00:00:00Z","2026-10-21T00:02:00Z",true),
            {"issue_number":10,"state":"launched","attempts":0}
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
                [7, false, 60, null, ["wait_interval"]]
            ])
        );
        assert_eq!(rows[0]["since"], "2026-10-21T09:00:00+09:00");
        assert_eq!(rows[3]["since"], Value::Null);
        assert_eq!(rows[4]["expires_at"], Value::Null);
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
        for scan in [None, Some("invalid scan".into())] {
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
    fn absent_and_degraded_monitor_observations_never_prove_complete_coverage() {
        let state = fixture();
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(absent["capacity"].get("claim_blocks"), Some(&Value::Null));
        assert_eq!(absent["capacity"].get("declared_waits"), Some(&Value::Null));
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
