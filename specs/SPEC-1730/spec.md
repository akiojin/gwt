# Feature Specification: gwt-spec workflow completion gate

## Background

- #1579 is the canonical embedded-skill workflow spec, but the current workflow only defines the pre-implementation `CLEAR` gate.
- #1654 exposed a missing completion gate: `tasks.md` and progress comments were marked complete while the implementation still diverged from `doc:spec.md`, `checklist:acceptance.md`, and `checklist:tdd.md`.
- The current flow separates `gwt-spec-ops`, `gwt-spec-analyze`, and `gwt-spec-implement`, but it does not require a post-implementation audit before completion is declared.
- This child spec defines the artifact-integrity and completion-gate rules for the `gwt-spec-*` workflow itself.

## User Stories

### User Story 1 - Implementation completion must be evidence-backed (Priority: P0)

As a maintainer, I want a SPEC to be marked complete only after tasks, acceptance, TDD, progress, and verification all agree.

**Acceptance Scenarios**

1. Given `doc:tasks.md` is all `[x]`, when completion is declared, then `checklist:acceptance.md` and `checklist:tdd.md` must also reflect the same completion state.
2. Given implementation still violates `doc:spec.md`, when someone tries to mark the SPEC done, then the workflow must route back to `gwt-spec-ops` instead of allowing completion.
3. Given a progress comment says `implementation is complete`, when the completion gate has not passed, then the workflow must treat that comment as invalid and require correction.

### User Story 2 - The workflow must distinguish preflight from exit gates (Priority: P0)

As an implementer, I want `gwt-spec-analyze` to stay a preflight check while a separate mandatory completion audit governs exit.

**Acceptance Scenarios**

1. Given a `CLEAR` analysis result, when implementation starts, then `gwt-spec-implement` may execute but cannot use that same `CLEAR` as the final completion proof.
2. Given implementation has finished, when the exit audit runs, then it must compare code and verification evidence against `doc:spec.md`, `doc:tasks.md`, and `checklist:*` artifacts.
3. Given the exit audit finds divergence, when completion is blocked, then the next step is `gwt-spec-ops` for artifact repair or task rollback.

### User Story 3 - Acceptance and TDD artifacts must stay machine-usable (Priority: P1)

As a workflow maintainer, I want checklist artifacts to be structured and current so they can participate in the completion gate.

**Acceptance Scenarios**

1. Given `checklist:tdd.md` exists, when the workflow reads it, then it must be in a clear, current, non-corrupted format.
2. Given acceptance scenarios exist in `doc:spec.md`, when `doc:tasks.md` is generated or updated, then verification tasks must map back to them.
3. Given a checklist artifact is stale or malformed, when the workflow reaches completion, then the SPEC cannot be marked done.

## Edge Cases

- `doc:tasks.md` is all complete but one or more acceptance items remain unchecked.
- Progress comments contain outdated `Done` statements after requirements changed.
- A spec issue inherited corrupted or partial `checklist:tdd.md` content from earlier migrations.
- An implementation is partially correct and needs task rollback rather than a brand-new spec.

## Functional Requirements

- **FR-001**: `#1579` remains the canonical workflow owner for `gwt-spec-*` behavior.
- **FR-002**: `gwt-spec-analyze` must be documented as a pre-implementation readiness gate only.
- **FR-003**: `gwt-spec-implement` must include a mandatory post-implementation completion gate before tasks or progress can declare completion.
- **FR-004**: The completion gate must reconcile `doc:spec.md`, `doc:tasks.md`, `checklist:acceptance.md`, `checklist:tdd.md`, progress comments, and executed verification.
- **FR-005**: If reconciliation fails, the workflow must route back to `gwt-spec-ops` and must not leave the SPEC in a completed state.
- **FR-006**: TDD and acceptance checklists must remain structured, readable, and current enough to support the completion gate.
- **FR-007**: Completion comments such as `implementation is complete` must be treated as workflow outputs that require evidence, not as source-of-truth on their own.

## Non-Functional Requirements

- **NFR-001**: Completion-gate rules must stay aligned across skill docs, command docs, and issue artifact conventions.
- **NFR-002**: The workflow must prefer rollback of false completion state over silently broadening the SPEC.
- **NFR-003**: The completion audit must be specific enough that a future implementer does not need to invent exit criteria.

## Success Criteria

- **SC-001**: The workflow distinguishes `pre-implementation CLEAR` from `post-implementation completion`.
- **SC-002**: A SPEC cannot be marked complete while acceptance/TDD/task/progress artifacts disagree.
- **SC-003**: Corrupted or stale checklist artifacts are identified as blockers rather than ignored.
- **SC-004**: `#1654` can be remediated under the new completion rules without inventing a second shell spec.
