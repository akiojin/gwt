---
name: gwt-spec-analyze
description: Analyze a SPEC artifact set for completeness and consistency across `spec.md`, `plan.md`, `tasks.md`, and supporting artifacts. Detect missing traceability, unresolved clarifications, and constitution gaps before implementation, and distinguish auto-fixable gaps from true decision blockers.
---

# gwt SPEC Analyze

Use this skill as the final gate before implementation starts.

This skill is a **pre-implementation readiness gate only**. It does not certify that implementation is complete.

- `gwt-spec-analyze` is still non-implementation work.
- Do not implement code here.
- If artifacts are missing, distinguish between gaps that `gwt-spec-ops` can repair automatically and gaps that truly require user input.

## Required artifact set

- `spec.md`
- `plan.md`
- `tasks.md`
- `memory/constitution.md`

Optional but validated when present:

- `research.md`
- `data-model.md`
- `quickstart.md`
- `contracts/*`

## Mandatory checks

1. **Clarification completeness**
   - No critical `[NEEDS CLARIFICATION]` markers remain

2. **Spec completeness**
   - User Stories, Acceptance Scenarios, Edge Cases, Requirements, and Success Criteria exist

3. **Plan completeness**
   - `Constitution Check` exists
   - `Technical Context` and `Phased Implementation` are concrete

4. **Task traceability**
   - Every user story has tasks
   - Every acceptance scenario has verification coverage
   - Every contract/data-model change has matching tasks

5. **Constitution alignment**
   - Violations are either removed or explicitly tracked in `Complexity Tracking`

## Required output

```text
## Analysis Report: SPEC-<id>

Status: CLEAR | AUTO-FIXABLE | NEEDS-DECISION

Blocking items:
- A1. <artifact gap>
- A2. <traceability gap>

Next:
- `gwt-spec-ops`
- `gwt-spec-implement`
- ask user for decision
```

## Decision rule

- `CLEAR`: implementation may proceed through `gwt-spec-implement`
- `AUTO-FIXABLE`: `gwt-spec-ops` should repair the artifact set and rerun analysis
- `NEEDS-DECISION`: the report must point to the exact user decision or unresolved ambiguity

## Boundary

- `CLEAR` means the artifact set is ready for execution.
- `CLEAR` does **not** mean the SPEC may be marked complete.
- Completion requires a post-implementation reconciliation pass in `gwt-spec-implement` against:
  - `spec.md`
  - `tasks.md`
  - `checklists/acceptance.md`
  - `checklists/tdd.md`
  - latest progress files
  - executed verification evidence

## Operations

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --list
```
