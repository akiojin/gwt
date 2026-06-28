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
}
