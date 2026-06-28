//! SPEC #3200 — Issue Monitor Autonomous Mode: strong automated merge gate.
//! Composes CI required-check existence + gwt-verify matrix + independent review
//! into a fail-closed, reviewed-SHA-bound gate. Populated in Phase 3 (Gap #1/#6).
//!
//! This module also owns the deterministic **pre-launch acceptance-criteria
//! classifier** (FR-003(iii) / FR-014). It only decides, without invoking any
//! agent, whether an Issue carries a well-formed, machine-checkable
//! acceptance-criteria block and whether any criterion targets a visual surface.
//! Per-criterion verification is the review-time judgment (FR-015), kept
//! separate to break the chicken-and-egg between eligibility and review.

/// Outcome of the deterministic pre-launch acceptance-criteria classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceCriteria {
    /// Stable criterion ids found in the structured block (e.g. `AC-1`).
    pub ids: Vec<String>,
    /// True only when a well-formed acceptance-criteria block with at least one
    /// criterion is present. Absence / malformation ⇒ `false` ⇒ the Issue is
    /// ineligible for autonomous resolution (routes to `NeedsHuman`).
    pub machine_checkable: bool,
    /// True when any criterion is tagged as targeting a visual surface
    /// (`(visual)`), so review-time judgment must include visual assessment.
    pub visual_surface: bool,
}

impl AcceptanceCriteria {
    /// Capture the launch-time snapshot used to detect post-launch drift
    /// (SPEC #3200 T-018 / FR-014). Only the stable id set and the
    /// visual-surface flag are retained — these are the gate-relevant facts.
    pub fn snapshot(&self) -> AcceptanceSnapshot {
        AcceptanceSnapshot {
            ids: self.ids.clone(),
            visual_surface: self.visual_surface,
        }
    }
}

/// Acceptance-criteria snapshot captured at autonomous launch (SPEC #3200
/// T-018). Re-classified criteria are compared against it at gate time so an
/// Issue body edited after launch (criteria added/removed/changed, or a visual
/// tag toggled) is detected and fails the autonomous merge closed (FR-014).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AcceptanceSnapshot {
    /// Stable criterion ids present at launch.
    pub ids: Vec<String>,
    /// Whether any criterion targeted a visual surface at launch.
    pub visual_surface: bool,
}

impl AcceptanceSnapshot {
    /// Fail-closed equality: the current criteria match the snapshot iff they
    /// carry the exact same id set (order-independent) AND the same
    /// visual-surface flag. Any divergence ⇒ `false` ⇒ the gate must not pass.
    pub fn matches(&self, current: &AcceptanceCriteria) -> bool {
        if self.visual_surface != current.visual_surface {
            return false;
        }
        if self.ids.len() != current.ids.len() {
            return false;
        }
        let mut want = self.ids.clone();
        let mut have = current.ids.clone();
        want.sort();
        have.sort();
        want == have
    }
}

/// Heading lines (case-insensitive, trimmed of leading `#`/spaces) that open the
/// structured acceptance-criteria block.
const ACCEPTANCE_HEADINGS: &[&str] = &["acceptance criteria", "受け入れ基準", "受け入れシナリオ"];

fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    Some(trimmed.trim_start_matches('#').trim().to_ascii_lowercase())
}

/// Deterministically classify the acceptance criteria in an Issue body.
///
/// The required block format is a heading from [`ACCEPTANCE_HEADINGS`] followed
/// by checklist items of the form `- [ ] AC-<id>: <text>` (optionally trailing
/// `(visual)`). Parsing stops at the next heading. No agent is invoked; this is
/// coarse machine-checkability only.
pub fn classify_acceptance_criteria(issue_body: &str) -> AcceptanceCriteria {
    let mut in_block = false;
    let mut ids: Vec<String> = Vec::new();
    let mut visual_surface = false;

    for line in issue_body.lines() {
        if let Some(heading) = heading_text(line) {
            // Entering the block iff this heading matches; any other heading
            // closes a previously open block.
            in_block = ACCEPTANCE_HEADINGS.iter().any(|h| heading == *h);
            continue;
        }
        if !in_block {
            continue;
        }
        let item = line.trim_start();
        // Checklist item: `- [ ] AC-..:` or `- [x] AC-..:` (and `*` bullets).
        let after_bullet = item
            .strip_prefix("- ")
            .or_else(|| item.strip_prefix("* "))
            .map(str::trim_start);
        let Some(rest) = after_bullet else { continue };
        let rest = rest
            .strip_prefix("[ ]")
            .or_else(|| rest.strip_prefix("[x]"))
            .or_else(|| rest.strip_prefix("[X]"))
            .map(str::trim_start)
            .unwrap_or(rest);
        // Require an explicit, stable `AC-<id>` token followed by `:`.
        let Some(after_ac) = rest.strip_prefix("AC-") else {
            continue;
        };
        let Some(colon) = after_ac.find(':') else {
            continue;
        };
        let id_part = after_ac[..colon].trim();
        if id_part.is_empty()
            || !id_part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            continue;
        }
        ids.push(format!("AC-{id_part}"));
        let body = after_ac[colon + 1..].to_ascii_lowercase();
        if body.contains("(visual)") || body.contains("[visual]") {
            visual_surface = true;
        }
    }

    AcceptanceCriteria {
        machine_checkable: !ids.is_empty(),
        ids,
        visual_surface,
    }
}

/// CI outcome for the reviewed SHA, as seen by the strong gate. Only a real,
/// non-vacuous success against the reviewed SHA may contribute to an autonomous
/// merge (SPEC #3200 FR-009). Every other state is fail-closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiOutcome {
    /// Required checks completed successfully. `passed_checks` lists the check
    /// contexts that actually passed — empty means vacuous green (fail-closed).
    Success { passed_checks: Vec<String> },
    /// At least one required check is still pending / running. Fail-closed.
    Pending,
    /// A required check failed.
    Failed,
    /// No required checks ran at all (vacuous green). Fail-closed.
    Vacuous,
}

/// The composed result of the strong automated gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Every gate condition held against the reviewed SHA — `skipped(autonomous-mode)`
    /// may be set and the merge driven. The ONLY value that authorizes an
    /// autonomous merge.
    Pass,
    /// Fail-closed for every other case, with a machine-grep-able reason.
    Fail(String),
}

/// All inputs the strong gate composes. Bundled so the evaluation reads as one
/// fail-closed conjunction and callers cannot forget a condition.
#[derive(Debug, Clone)]
pub struct AutonomousGateInputs {
    /// Base-branch protection (must be `Verified` with ≥1 required check).
    pub branch_protection: gwt_git::branch_protection::BranchProtectionStatus,
    /// CI outcome for the reviewed SHA.
    pub ci: CiOutcome,
    /// Independent-review verdict (must be `Pass`).
    pub review: crate::issue_monitor_review::ReviewGateOutcome,
    /// Whether the launch-time acceptance snapshot still matches the Issue body.
    pub acceptance_unchanged: bool,
    /// The SHA the independent review actually evaluated.
    pub reviewed_sha: String,
    /// The current branch HEAD SHA that would be merged.
    pub head_sha: String,
}

/// SPEC #3200 T-083/T-064/T-065/T-066 (FR-009..FR-016): the strong automated
/// gate. Returns [`GateDecision::Pass`] ONLY when every condition holds against
/// the reviewed SHA:
///
/// 1. base-branch protection is `Verified` with ≥1 required check,
/// 2. CI is a real, non-vacuous success covering every required check,
/// 3. the independent review verdict is `Pass`,
/// 4. the acceptance criteria are unchanged since launch (no tamper),
/// 5. the reviewed SHA equals the head SHA and is non-empty (TOCTOU binding).
///
/// Any divergence ⇒ [`GateDecision::Fail`] (fail-closed). This is the only
/// function permitted to authorize `skipped(autonomous-mode)`.
pub fn evaluate_autonomous_gate(inputs: &AutonomousGateInputs) -> GateDecision {
    use crate::issue_monitor_review::ReviewGateOutcome;

    let required_checks = match &inputs.branch_protection {
        gwt_git::branch_protection::BranchProtectionStatus::Verified { required_checks }
            if !required_checks.is_empty() =>
        {
            required_checks
        }
        _ => {
            return GateDecision::Fail("base-branch protection is not verified".to_string());
        }
    };

    let passed_checks = match &inputs.ci {
        CiOutcome::Success { passed_checks } if !passed_checks.is_empty() => passed_checks,
        CiOutcome::Success { .. } => {
            return GateDecision::Fail("CI reported success with no checks (vacuous)".to_string());
        }
        CiOutcome::Pending => return GateDecision::Fail("CI is still pending".to_string()),
        CiOutcome::Failed => return GateDecision::Fail("CI failed".to_string()),
        CiOutcome::Vacuous => {
            return GateDecision::Fail("CI ran no required checks (vacuous)".to_string());
        }
    };

    // Every branch-protection-required check must have actually passed.
    if let Some(missing) = required_checks
        .iter()
        .find(|required| !passed_checks.iter().any(|passed| passed == *required))
    {
        return GateDecision::Fail(format!("required check {missing} did not pass"));
    }

    if inputs.review != ReviewGateOutcome::Pass {
        return GateDecision::Fail("independent review did not pass".to_string());
    }

    if !inputs.acceptance_unchanged {
        return GateDecision::Fail("acceptance criteria changed after launch".to_string());
    }

    if inputs.reviewed_sha.is_empty() || inputs.reviewed_sha != inputs.head_sha {
        return GateDecision::Fail(format!(
            "reviewed SHA {:?} is not the head SHA {:?} (TOCTOU)",
            inputs.reviewed_sha, inputs.head_sha
        ));
    }

    GateDecision::Pass
}

/// Classify a `gh pr view --json statusCheckRollup` body against the set of
/// `required_checks` into a [`CiOutcome`] (SPEC #3200 T-073/FR-009). Fail-closed:
///
/// - empty `required_checks` ⇒ `Vacuous` (nothing actually gates),
/// - any required check absent or not COMPLETED ⇒ `Pending`,
/// - any required check COMPLETED with a non-success conclusion ⇒ `Failed`,
/// - an unparseable rollup ⇒ `Pending` (never a pass),
/// - only every required check COMPLETED + success ⇒ `Success`.
pub fn classify_ci_rollup(rollup_json: &str, required_checks: &[String]) -> CiOutcome {
    if required_checks.is_empty() {
        return CiOutcome::Vacuous;
    }
    let items: Vec<serde_json::Value> = match serde_json::from_str(rollup_json) {
        Ok(serde_json::Value::Array(items)) => items,
        _ => return CiOutcome::Pending,
    };

    let mut passed = Vec::new();
    for required in required_checks {
        let Some(item) = items
            .iter()
            .find(|item| ci_item_name(item) == Some(required.as_str()))
        else {
            // Required check has not been reported yet ⇒ still pending.
            return CiOutcome::Pending;
        };
        match ci_item_state(item) {
            CiItemState::Success => passed.push(required.clone()),
            CiItemState::Failed => return CiOutcome::Failed,
            CiItemState::Pending => return CiOutcome::Pending,
        }
    }
    CiOutcome::Success {
        passed_checks: passed,
    }
}

enum CiItemState {
    Success,
    Failed,
    Pending,
}

/// The check name for a rollup item: `name` for a CheckRun, `context` for a
/// legacy StatusContext.
fn ci_item_name(item: &serde_json::Value) -> Option<&str> {
    item.get("name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| item.get("context").and_then(serde_json::Value::as_str))
}

/// Normalize a rollup item to success / failed / pending. A CheckRun is success
/// only when `status == COMPLETED` and `conclusion` is a success variant; a
/// StatusContext is success only when `state == SUCCESS`. Anything still running
/// is pending; any completed non-success is failed.
fn ci_item_state(item: &serde_json::Value) -> CiItemState {
    // Legacy StatusContext: a `state` field carries the whole verdict.
    if let Some(state) = item.get("state").and_then(serde_json::Value::as_str) {
        return match state.to_ascii_uppercase().as_str() {
            "SUCCESS" => CiItemState::Success,
            "PENDING" | "EXPECTED" => CiItemState::Pending,
            _ => CiItemState::Failed,
        };
    }
    // CheckRun: gate on status first, then conclusion.
    let status = item
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_ascii_uppercase();
    if status != "COMPLETED" {
        return CiItemState::Pending;
    }
    match item
        .get("conclusion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_ascii_uppercase()
        .as_str()
    {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => CiItemState::Success,
        _ => CiItemState::Failed,
    }
}

/// The monitor action implied by a strong-gate evaluation (SPEC #3200 T-086).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateAction {
    /// Gate passed — authorize the autonomous merge through Deliver.
    Deliver,
    /// CI is still running — re-check on the next scan. Consumes NO attempt
    /// (waiting is not a failure).
    WaitForCi,
    /// Agent-remediable failure (CI failed, review rejected, or HEAD advanced so
    /// the review is stale) — run a bounded Deliver-Fix / re-review attempt.
    Remediate(String),
    /// Structural failure the agent cannot fix (branch protection unavailable, or
    /// the human edited the acceptance criteria after launch) — escalate to a
    /// human.
    Escalate(String),
}

/// Route a strong-gate evaluation to the monitor action (SPEC #3200 T-086).
///
/// - all conditions hold ⇒ [`GateAction::Deliver`];
/// - branch protection not verifiable ⇒ [`GateAction::Escalate`] (repo settings
///   are not agent-fixable);
/// - acceptance criteria changed after launch ⇒ [`GateAction::Escalate`] (a human
///   moved the spec — autonomy must not chase a moving target);
/// - HEAD advanced past the reviewed SHA ⇒ [`GateAction::Remediate`] (re-review
///   the new SHA, bounded by attempts);
/// - CI still pending ⇒ [`GateAction::WaitForCi`] (no attempt consumed);
/// - CI failed or review rejected ⇒ [`GateAction::Remediate`] (bounded fix loop).
pub fn route_autonomous_gate(inputs: &AutonomousGateInputs) -> GateAction {
    if evaluate_autonomous_gate(inputs) == GateDecision::Pass {
        return GateAction::Deliver;
    }
    if !inputs.branch_protection.is_verified() {
        return GateAction::Escalate("base-branch protection is not verified".to_string());
    }
    if !inputs.acceptance_unchanged {
        return GateAction::Escalate("acceptance criteria changed after launch".to_string());
    }
    if inputs.reviewed_sha.is_empty() || inputs.reviewed_sha != inputs.head_sha {
        return GateAction::Remediate(
            "HEAD advanced past the reviewed SHA — re-review".to_string(),
        );
    }
    if matches!(inputs.ci, CiOutcome::Pending) {
        return GateAction::WaitForCi;
    }
    GateAction::Remediate("CI failed or independent review rejected — remediate".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_block_is_machine_checkable() {
        let body = "## Background\nsome text\n\n## Acceptance Criteria\n- [ ] AC-1: the API returns 200\n- [ ] AC-2: the list is sorted\n\n## Notes\n";
        let c = classify_acceptance_criteria(body);
        assert!(c.machine_checkable);
        assert_eq!(c.ids, vec!["AC-1", "AC-2"]);
        assert!(!c.visual_surface);
    }

    #[test]
    fn japanese_heading_and_visual_tag_detected() {
        let body = "## 受け入れ基準\n- [ ] AC-1: ボタンが表示される (visual)\n- [x] AC-2: 値が保存される\n";
        let c = classify_acceptance_criteria(body);
        assert!(c.machine_checkable);
        assert_eq!(c.ids, vec!["AC-1", "AC-2"]);
        assert!(c.visual_surface, "(visual) tag marks a visual surface");
    }

    #[test]
    fn missing_block_is_not_machine_checkable() {
        let body = "Just a free-text issue describing a bug with no structured criteria.";
        let c = classify_acceptance_criteria(body);
        assert!(!c.machine_checkable);
        assert!(c.ids.is_empty());
    }

    #[test]
    fn malformed_items_without_ac_ids_are_ignored() {
        // Heading present but items lack stable AC-<id>: tokens.
        let body = "## Acceptance Criteria\n- it should work\n- [ ] returns ok\n- AC- : empty id\n";
        let c = classify_acceptance_criteria(body);
        assert!(!c.machine_checkable, "no well-formed AC-<id> criterion");
        assert!(c.ids.is_empty());
    }

    #[test]
    fn parsing_stops_at_next_heading() {
        let body = "## Acceptance Criteria\n- [ ] AC-1: real\n## Out of Scope\n- [ ] AC-9: not a criterion\n";
        let c = classify_acceptance_criteria(body);
        assert_eq!(
            c.ids,
            vec!["AC-1"],
            "AC-9 under a later heading is excluded"
        );
    }

    #[test]
    fn snapshot_captures_ids_and_visual_flag() {
        let body = "## Acceptance Criteria\n- [ ] AC-1: x\n- [ ] AC-2: y (visual)\n";
        let snapshot = classify_acceptance_criteria(body).snapshot();
        assert_eq!(snapshot.ids, vec!["AC-1", "AC-2"]);
        assert!(snapshot.visual_surface);
    }

    #[test]
    fn snapshot_matches_identical_criteria() {
        let body = "## Acceptance Criteria\n- [ ] AC-1: x\n- [ ] AC-2: y\n";
        let snapshot = classify_acceptance_criteria(body).snapshot();
        // Re-classifying the same body yields criteria the snapshot accepts.
        assert!(snapshot.matches(&classify_acceptance_criteria(body)));
    }

    #[test]
    fn snapshot_rejects_post_launch_criteria_drift() {
        // A snapshot taken at launch must FAIL CLOSED when the Issue body's
        // criteria are later edited (added / removed / visual-tag changed),
        // so a post-launch tamper cannot pass the autonomous gate (FR-014).
        let at_launch =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot();
        let added = classify_acceptance_criteria(
            "## Acceptance Criteria\n- [ ] AC-1: x\n- [ ] AC-2: new\n",
        );
        assert!(!at_launch.matches(&added), "added criterion must not match");
        let removed =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-9: different\n");
        assert!(!at_launch.matches(&removed), "changed id must not match");
        let visual =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x (visual)\n");
        assert!(
            !at_launch.matches(&visual),
            "visual-surface change must not match"
        );
    }

    #[test]
    fn snapshot_order_independent_for_ids() {
        // The same set of criterion ids in a different order is still a match;
        // ordering is not semantically meaningful, the id set is.
        let a =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n- [ ] AC-2: y\n")
                .snapshot();
        let b =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-2: y\n- [ ] AC-1: x\n");
        assert!(a.matches(&b), "id set equality is order-independent");
    }

    mod gate {
        use super::super::*;
        use crate::issue_monitor_review::ReviewGateOutcome;
        use gwt_git::branch_protection::BranchProtectionStatus;

        fn verified() -> BranchProtectionStatus {
            BranchProtectionStatus::Verified {
                required_checks: vec!["build".to_string(), "test".to_string()],
            }
        }

        fn real_ci() -> CiOutcome {
            CiOutcome::Success {
                passed_checks: vec!["build".to_string(), "test".to_string()],
            }
        }

        fn all_pass_inputs() -> AutonomousGateInputs {
            AutonomousGateInputs {
                branch_protection: verified(),
                ci: real_ci(),
                review: ReviewGateOutcome::Pass,
                acceptance_unchanged: true,
                reviewed_sha: "abc123".to_string(),
                head_sha: "abc123".to_string(),
            }
        }

        #[test]
        fn all_conditions_pass_yields_pass() {
            assert_eq!(
                evaluate_autonomous_gate(&all_pass_inputs()),
                GateDecision::Pass
            );
        }

        fn req(ids: &[&str]) -> Vec<String> {
            ids.iter().map(|s| s.to_string()).collect()
        }

        #[test]
        fn ci_rollup_all_required_success_is_success() {
            let rollup = r#"[
                {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"},
                {"__typename":"StatusContext","context":"test","state":"SUCCESS"}
            ]"#;
            assert_eq!(
                classify_ci_rollup(rollup, &req(&["build", "test"])),
                CiOutcome::Success {
                    passed_checks: req(&["build", "test"])
                }
            );
        }

        #[test]
        fn ci_rollup_pending_required_check_is_pending() {
            let rollup = r#"[
                {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"},
                {"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":null}
            ]"#;
            assert_eq!(
                classify_ci_rollup(rollup, &req(&["build", "test"])),
                CiOutcome::Pending
            );
        }

        #[test]
        fn ci_rollup_failed_required_check_is_failed() {
            let rollup = r#"[
                {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"FAILURE"}
            ]"#;
            assert_eq!(
                classify_ci_rollup(rollup, &req(&["build"])),
                CiOutcome::Failed
            );
        }

        #[test]
        fn ci_rollup_missing_required_check_is_pending() {
            // A required check absent from the rollup has not run yet ⇒ pending,
            // never a pass.
            let rollup = r#"[{"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]"#;
            assert_eq!(
                classify_ci_rollup(rollup, &req(&["build", "test"])),
                CiOutcome::Pending
            );
        }

        #[test]
        fn ci_rollup_no_required_checks_is_vacuous() {
            assert_eq!(classify_ci_rollup("[]", &req(&[])), CiOutcome::Vacuous);
        }

        #[test]
        fn ci_rollup_unparseable_fails_closed() {
            // A rollup we cannot read must not be treated as success.
            assert_eq!(
                classify_ci_rollup("not json", &req(&["build"])),
                CiOutcome::Pending
            );
        }

        #[test]
        fn route_all_pass_delivers() {
            assert_eq!(
                route_autonomous_gate(&all_pass_inputs()),
                GateAction::Deliver
            );
        }

        #[test]
        fn route_unverified_protection_escalates() {
            let inputs = AutonomousGateInputs {
                branch_protection: BranchProtectionStatus::Absent,
                ..all_pass_inputs()
            };
            assert!(matches!(
                route_autonomous_gate(&inputs),
                GateAction::Escalate(_)
            ));
        }

        #[test]
        fn route_acceptance_drift_escalates() {
            let inputs = AutonomousGateInputs {
                acceptance_unchanged: false,
                ..all_pass_inputs()
            };
            assert!(matches!(
                route_autonomous_gate(&inputs),
                GateAction::Escalate(_)
            ));
        }

        #[test]
        fn route_head_advance_remediates() {
            // A new commit after review is agent-fixable: re-review the new SHA.
            let inputs = AutonomousGateInputs {
                head_sha: "def456".to_string(),
                ..all_pass_inputs()
            };
            assert!(matches!(
                route_autonomous_gate(&inputs),
                GateAction::Remediate(_)
            ));
        }

        #[test]
        fn route_ci_pending_waits_without_consuming_attempt() {
            let inputs = AutonomousGateInputs {
                ci: CiOutcome::Pending,
                ..all_pass_inputs()
            };
            assert_eq!(route_autonomous_gate(&inputs), GateAction::WaitForCi);
        }

        #[test]
        fn route_ci_failure_and_review_rejection_remediate() {
            let ci_failed = AutonomousGateInputs {
                ci: CiOutcome::Failed,
                ..all_pass_inputs()
            };
            assert!(matches!(
                route_autonomous_gate(&ci_failed),
                GateAction::Remediate(_)
            ));
            let review_rejected = AutonomousGateInputs {
                review: ReviewGateOutcome::Fail("nope".to_string()),
                ..all_pass_inputs()
            };
            assert!(matches!(
                route_autonomous_gate(&review_rejected),
                GateAction::Remediate(_)
            ));
        }

        #[test]
        fn unverified_branch_protection_fails_closed() {
            for bp in [
                BranchProtectionStatus::Absent,
                BranchProtectionStatus::Unreadable("403".to_string()),
                BranchProtectionStatus::Verified {
                    required_checks: vec![],
                },
            ] {
                let inputs = AutonomousGateInputs {
                    branch_protection: bp,
                    ..all_pass_inputs()
                };
                assert!(matches!(
                    evaluate_autonomous_gate(&inputs),
                    GateDecision::Fail(_)
                ));
            }
        }

        #[test]
        fn non_real_ci_fails_closed() {
            for ci in [
                CiOutcome::Pending,
                CiOutcome::Failed,
                CiOutcome::Vacuous,
                CiOutcome::Success {
                    passed_checks: vec![],
                },
            ] {
                let inputs = AutonomousGateInputs {
                    ci,
                    ..all_pass_inputs()
                };
                assert!(matches!(
                    evaluate_autonomous_gate(&inputs),
                    GateDecision::Fail(_)
                ));
            }
        }

        #[test]
        fn ci_missing_a_required_check_fails_closed() {
            // Vacuous-green defense: every branch-protection required check must
            // have actually passed; a missing one is not a real green.
            let inputs = AutonomousGateInputs {
                ci: CiOutcome::Success {
                    passed_checks: vec!["build".to_string()], // "test" missing
                },
                ..all_pass_inputs()
            };
            assert!(matches!(
                evaluate_autonomous_gate(&inputs),
                GateDecision::Fail(_)
            ));
        }

        #[test]
        fn review_not_pass_fails_closed() {
            let inputs = AutonomousGateInputs {
                review: ReviewGateOutcome::Fail("rejected".to_string()),
                ..all_pass_inputs()
            };
            assert!(matches!(
                evaluate_autonomous_gate(&inputs),
                GateDecision::Fail(_)
            ));
        }

        #[test]
        fn acceptance_drift_fails_closed() {
            let inputs = AutonomousGateInputs {
                acceptance_unchanged: false,
                ..all_pass_inputs()
            };
            assert!(matches!(
                evaluate_autonomous_gate(&inputs),
                GateDecision::Fail(_)
            ));
        }

        #[test]
        fn reviewed_sha_mismatch_fails_closed_toctou() {
            // TOCTOU: the merge SHA must be exactly the reviewed SHA. A HEAD
            // advance after review must invalidate the gate.
            let advanced = AutonomousGateInputs {
                head_sha: "def456".to_string(),
                ..all_pass_inputs()
            };
            assert!(matches!(
                evaluate_autonomous_gate(&advanced),
                GateDecision::Fail(_)
            ));
            let empty = AutonomousGateInputs {
                reviewed_sha: String::new(),
                head_sha: String::new(),
                ..all_pass_inputs()
            };
            assert!(
                matches!(evaluate_autonomous_gate(&empty), GateDecision::Fail(_)),
                "empty SHAs never satisfy the binding"
            );
        }
    }
}
