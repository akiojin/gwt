# Quality Gate

Reference for Phase 4 of `gwt-plan`. Defines the pre-implementation readiness checks,
three-way verdict, and auto-fix behavior.

## Purpose

The quality gate is the final checkpoint before implementation begins. It verifies that
the artifact set (spec.md, plan.md, tasks.md, and supporting artifacts) is complete,
consistent, and actionable.

This gate does NOT certify that implementation is complete. Post-implementation
reconciliation happens in `gwt-build`.

## Five Mandatory Checks

### Check 1: Clarification Completeness

- Scan `spec.md` for `[NEEDS CLARIFICATION]` markers
- **Pass:** no critical markers remain (informational markers are acceptable)
- **Fail:** any critical marker blocks implementation

### Check 2: Spec Completeness

Verify that `spec.md` contains all required sections:

- User Stories
- Acceptance Scenarios
- Edge Cases
- Requirements (functional and non-functional)
- Success Criteria

**Pass:** all sections present and non-empty.
**Fail:** any required section is missing or empty.

### Check 3: Plan Completeness

Verify that `plan.md` contains:

- Constitution Check (present and non-empty)
- Technical Context (concrete, not placeholder)
- Phased Implementation (coherent build order)

**Pass:** all three are present and substantive.
**Fail:** any is missing, empty, or contains only placeholder text.

### Check 4: Task Traceability

Cross-reference `tasks.md` against `spec.md`, `plan.md`, and supporting artifacts:

- Every user story has implementation and verification tasks
- Every acceptance scenario has verification coverage
- Every contract/data-model change has matching tasks

**Pass:** full coverage with no orphan tasks and no uncovered requirements.
**Fail:** any gap in coverage.

> This check is skipped in lightweight mode (no spec.md).

### Check 5: Constitution Alignment

Verify against `.gwt/memory/constitution.md`:

- No rule violations exist without explicit justification
- Justified exceptions are recorded in `Complexity Tracking` section of `plan.md`

**Pass:** all rules satisfied or explicitly justified.
**Fail:** unjustified violation found.

## Three-Way Verdict

After running all five checks, issue exactly one verdict:

### CLEAR

All checks pass. The artifact set is ready for implementation.

**Next action:** suggest proceeding to `gwt-build`.

### AUTO-FIXABLE

One or more checks fail, but all failures are mechanical and can be repaired without
user input. Examples:

- Missing test task for a clearly specified acceptance scenario
- Traceability gap where the mapping is obvious
- Missing section in plan.md that can be derived from existing artifacts
- Constitution Check section is empty but no violations exist

**Next action:** repair the artifacts in-place and rerun the quality gate. Do not ask
the user for permission to fix mechanical gaps.

### NEEDS-DECISION

One or more checks fail and at least one failure requires a user decision. Examples:

- Ambiguous acceptance scenario that could be interpreted multiple ways
- Constitution violation where the tradeoff is not obvious
- Missing user story that the spec implies but does not state
- Conflicting requirements between spec sections

**Next action:** present the exact decision points to the user. Do not guess.

## Report Format

```text
## Analysis Report: SPEC-<id>

Status: CLEAR | AUTO-FIXABLE | NEEDS-DECISION

### Check Results

1. Clarification completeness: PASS | FAIL — <detail>
2. Spec completeness: PASS | FAIL — <detail>
3. Plan completeness: PASS | FAIL — <detail>
4. Task traceability: PASS | FAIL — <detail>
5. Constitution alignment: PASS | FAIL — <detail>

### Blocking Items

- A1. <artifact gap or decision needed>
- A2. <artifact gap or decision needed>

### Auto-Fixed Items (if AUTO-FIXABLE)

- F1. <what was repaired>
- F2. <what was repaired>

### Next

- gwt-build (on CLEAR)
- self-repair and rerun (on AUTO-FIXABLE)
- ask user for decision (on NEEDS-DECISION)
```

## Auto-Fix Behavior

When the verdict is AUTO-FIXABLE:

1. Repair each mechanical gap directly in the artifact files
2. Log each repair in the report under "Auto-Fixed Items"
3. Rerun all five checks after repairs
4. Issue a new verdict — the second pass must be CLEAR or NEEDS-DECISION
5. Do not loop more than twice — if the second pass is still AUTO-FIXABLE,
   escalate to NEEDS-DECISION
