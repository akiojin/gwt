# Tasks

## Phase 1: Workflow semantics

- [ ] T001 Confirm #1579 remains the canonical parent spec for embedded workflow behavior
- [ ] T002 Define explicit post-implementation completion-gate semantics for `gwt-spec-*`
- [ ] T003 Define rollback rules for false completion state in issue artifacts and progress comments

## Phase 2: Skill and command updates

- [ ] T101 Update `gwt-spec-ops` to require completion-gate reconciliation before final done state
- [ ] T102 Update `gwt-spec-analyze` docs so `CLEAR` is scoped to pre-implementation readiness only
- [ ] T103 Update `gwt-spec-implement` docs to require post-implementation artifact/code reconciliation
- [ ] T104 Update `plugins/gwt/commands/gwt-spec-*.md` to match the new exit rules

## Phase 3: Checklist integrity

- [ ] T201 Define valid `checklist:tdd.md` structure for workflow-owned specs
- [ ] T202 Add validation expectations for stale or malformed checklist artifacts
- [ ] T203 Document how acceptance scenarios map back to verification tasks and completion claims

## Phase 4: Remediation rollout

- [ ] T301 Re-open #1654 completion state under the new gate
- [ ] T302 Repair `#1654` task/checklist/progress consistency before declaring completion again
- [ ] T303 Use #1654 as the first acceptance case for the new completion-gate workflow

## Phase 5: Verification

- [ ] T401 Verify skill docs and command docs describe the same owner and exit behavior
- [ ] T402 Verify malformed or stale checklist artifacts are treated as blockers
- [ ] T403 Verify #1654 cannot return to done state until artifacts and implementation agree
