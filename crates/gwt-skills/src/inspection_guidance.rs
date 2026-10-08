//! The Independent Intake Inspection reviewer checklist
//! (Issue #4541 AC-5, SPEC-3248 FR-189).
//!
//! The deterministic lint in `gwt::cli::spec_artifact_lint` already decides
//! numbering, traceability, supersede annotations, and marker health, so this
//! checklist is deliberately *not* a superset of it. It carries the four
//! items the 2026-07-09 intake evidence showed a machine cannot decide:
//!
//! 1. the supersede notes the lint flags actually say the right thing,
//! 2. no other SPEC contradicts this one — which needs a search, not a parse,
//! 3. the section really landed on GitHub, not just in the local cache,
//! 4. a "wired" entry point has a production call site (SPEC #3214 FR-010
//!    shipped state + surface with no caller and was counted as done).
//!
//! This is managed guidance: the checklist ships with gwt rather than living
//! in a project's AGENTS.md, so it reaches a reviewer in any gwt-managed
//! worktree.

/// One reviewer obligation and the evidence that discharges it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecklistItem {
    /// Stable machine id — tests and findings reference this, not the prose.
    pub id: &'static str,
    pub title: &'static str,
    /// What the reviewer must show, not merely assert.
    pub evidence: &'static str,
}

/// The four mandatory reviewer checklist items (FR-189).
///
/// Order is part of the contract: the reviewer works down from the artifact's
/// own consistency to its relationship with the rest of the repository.
pub const REVIEWER_CHECKLIST: &[ChecklistItem] = &[
    ChecklistItem {
        id: "supersede_annotation",
        title: "Supersede inline annotations",
        evidence: "For every Amendment in this artifact, quote the inline note left on each superseded FR / Out of Scope decision / plan summary. A lint pass is not enough: confirm the note names the superseding Amendment.",
    },
    ChecklistItem {
        id: "cross_spec_conflict_search",
        title: "Cross-SPEC contradiction search",
        evidence: "Record the gwt-search runs (queries and the SPEC / Issue numbers they returned) used to look for another owner that contradicts this artifact. Name the owners checked, not just the fact that a search happened.",
    },
    ChecklistItem {
        id: "github_entity_readback",
        title: "GitHub entity readback",
        evidence: "Show a fresh readback of the written sections from the GitHub entity (post-pull), with the observed content hash. A local cache re-read is not readback evidence, and a bypass-path write must be confirmed this way.",
    },
    ChecklistItem {
        id: "specced_but_unwired",
        title: "Specced-but-unwired verification",
        evidence: "For every task claiming an entry point is wired, show the grep evidence for its production call site. State + surface without a caller is implemented-but-not-wired and does not complete the task.",
    },
];

/// The checklist rendered for a reviewer prompt or a `gwtd` operation result.
#[must_use]
pub fn render_reviewer_checklist() -> String {
    let mut out = String::from(
        "## Independent Intake Inspection — reviewer checklist\n\n\
         Read-only: do not edit Issues, files, Board, git, PRs, or Work.\n\
         Deterministic artifact lint already ran; its findings are in the\n\
         Finding Disposition Ledger and are not yours to re-derive.\n\n",
    );
    for (index, item) in REVIEWER_CHECKLIST.iter().enumerate() {
        out.push_str(&format!(
            "{}. **{}** (`{}`)\n   - {}\n",
            index + 1,
            item.title,
            item.id,
            item.evidence
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-5: the four items are fixed by test, so removing one fails here
    /// before it can quietly leave the reviewer prompt.
    #[test]
    fn the_checklist_carries_exactly_the_four_mandatory_items() {
        let ids: Vec<&str> = REVIEWER_CHECKLIST.iter().map(|item| item.id).collect();
        assert_eq!(
            ids,
            vec![
                "supersede_annotation",
                "cross_spec_conflict_search",
                "github_entity_readback",
                "specced_but_unwired",
            ]
        );
    }

    #[test]
    fn every_item_states_the_evidence_it_needs() {
        for item in REVIEWER_CHECKLIST {
            assert!(!item.title.is_empty(), "{} has no title", item.id);
            assert!(
                item.evidence.len() > 40,
                "{} must say what evidence discharges it",
                item.id
            );
        }
    }

    #[test]
    fn the_rendered_checklist_contains_every_item() {
        let rendered = render_reviewer_checklist();
        for item in REVIEWER_CHECKLIST {
            assert!(
                rendered.contains(item.id),
                "rendered checklist is missing `{}`",
                item.id
            );
            assert!(
                rendered.contains(item.title),
                "rendered checklist is missing '{}'",
                item.title
            );
        }
    }

    #[test]
    fn the_rendered_checklist_states_the_read_only_contract() {
        let rendered = render_reviewer_checklist();
        assert!(rendered.contains("Read-only"));
        assert!(rendered.contains("Finding Disposition Ledger"));
    }

    #[test]
    fn the_checklist_ids_are_unique() {
        let mut ids: Vec<&str> = REVIEWER_CHECKLIST.iter().map(|item| item.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "checklist ids must be unique");
    }
}
