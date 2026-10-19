//! Read-only goal and roadmap diagnostics (SPEC #3431 T-113a/T-114a).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::{
    issue_monitor::IssueMonitorInboxCoverage,
    pm_planning::{PmPlanningState, RoadmapHorizon, RoadmapItem},
    IssueMonitorAgentStatus, IssueMonitorStatusSource, MonitorInboxState,
};

#[derive(Debug, Serialize)]
pub struct PlanningAssessment {
    pub planning_revision: u64,
    pub monitor: MonitorEvidence,
    pub missing_inputs: Vec<&'static str>,
    pub issue_traces: Vec<IssueTrace>,
    pub roadmap: RoadmapHorizons,
    pub roadmap_items: Vec<RoadmapDiagnostic>,
    pub milestones: Vec<MilestoneDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct MonitorEvidence {
    pub source: Option<IssueMonitorStatusSource>,
    pub last_scan_at: Option<String>,
    pub active_launches_incomplete: bool,
    pub inbox_coverage: Option<IssueMonitorInboxCoverage>,
}

#[derive(Debug, Serialize)]
pub struct IssueTrace {
    pub issue_number: u64,
    pub observed_queued_or_active: bool,
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
pub struct MilestoneDiagnostic {
    pub id: String,
    pub issues: Vec<u64>,
    pub target_date: Option<String>,
    pub exit_criteria: Vec<String>,
    pub reported_health: Option<String>,
    pub reported_confidence: Option<String>,
    pub health: &'static str,
    pub confidence: &'static str,
    pub missing_inputs: Vec<&'static str>,
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
                roadmap_item_ids: links.iter().map(|(_, item)| item.id.clone()).collect(),
                alignment,
                diagnostics: diagnostics.into_iter().collect(),
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
            if state
                .milestones
                .iter()
                .filter(|other| other.id == milestone.id)
                .count()
                > 1
            {
                missing_inputs.push("unique_milestone_id");
            }
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
        milestones,
    }
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
                    if [
                        &goal.outcome,
                        &goal.metric,
                        &goal.target,
                        &goal.horizon,
                        &goal.owner,
                        &goal.status,
                    ]
                    .iter()
                    .any(|value| value.trim().is_empty())
                    {
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
    fn absent_and_degraded_monitor_observations_never_prove_complete_coverage() {
        let state = fixture();
        let absent = serde_json::to_value(assess_planning(&state, None)).unwrap();
        assert_eq!(absent["monitor"]["source"], Value::Null);
        assert_eq!(absent["monitor"]["active_launches_incomplete"], true);
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
