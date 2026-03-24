# Plan

## Summary

Define a post-implementation completion gate for the `gwt-spec-*` workflow so completion cannot be declared from `tasks.md` alone.

## Technical Context

- Workflow owner: `plugins/gwt/skills/gwt-spec-ops/SKILL.md`
- Readiness gate: `plugins/gwt/skills/gwt-spec-analyze/SKILL.md`
- Execution owner: `plugins/gwt/skills/gwt-spec-implement/SKILL.md`
- User-facing command docs: `plugins/gwt/commands/gwt-spec-*.md`
- Reference remediation case: #1654

## Constitution Check

- Spec before implementation: completion rules are fixed here before changing downstream skill behavior.
- Test-first: skill and script changes must add verification for checklist/state reconciliation before marking done.
- No workaround-first: treat false completion as a workflow bug, not as a documentation footnote.
- Minimal complexity: add one explicit completion gate instead of scattering ad hoc reminders across skills.

## Project Structure

- Workflow canonical: #1579
- Storage/API canonical: #1327
- This child spec: completion gate and artifact integrity

## Complexity Tracking

- Added complexity: one explicit exit-gate concept after implementation.
- Mitigation: removes ambiguity around when `tasks.md` may be marked complete.

## Phased Implementation

1. Specify completion-gate semantics and artifact invariants.
2. Update `gwt-spec-ops`, `gwt-spec-analyze`, `gwt-spec-implement`, and command docs.
3. Repair malformed checklist expectations and add verification.
4. Apply the new rules to #1654 as the first remediation case.
