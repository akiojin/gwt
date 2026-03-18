---
name: gwt-spec-analyze
description: Analyze a `gwt-spec` artifact set for completeness and consistency across `spec.md`, `plan.md`, `tasks.md`, and supporting artifacts. Detect missing traceability, unresolved clarifications, and constitution gaps before implementation.
---

# gwt SPEC Analyze

Use this skill as the final gate before implementation starts.

- `gwt-spec-analyze` is check-only.
- Do not implement code here.
- If artifacts are missing, point to the exact preceding skill that must run next.

## Required artifact set

- `doc:spec.md`
- `doc:plan.md`
- `doc:tasks.md`
- `memory/constitution.md`

Optional but validated when present:

- `doc:research.md`
- `doc:data-model.md`
- `doc:quickstart.md`
- `contract:*`

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
## Analysis Report: #<number>

Status: CLEAR | BLOCKED

Blocking items:
- A1. <artifact gap>
- A2. <traceability gap>

Next:
- `gwt-spec-clarify`
- `gwt-spec-plan`
- `gwt-spec-tasks`
- `gwt-spec-ops`
```

## Decision rule

- `CLEAR`: implementation may proceed through `gwt-spec-ops`
- `BLOCKED`: the report must point to the exact artifact or gate that failed
