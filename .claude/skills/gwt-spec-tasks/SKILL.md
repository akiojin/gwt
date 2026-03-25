---
name: gwt-spec-tasks
description: "Generate tasks.md for an existing SPEC from spec.md and plan.md, grouped by phase and user story with exact file paths, [P] parallel markers, and test-first ordering. Use directly or through gwt-spec-ops. Use when user says 'generate tasks', 'create tasks.md', 'break down the plan into tasks', or when plan.md is ready for task decomposition."
---

# gwt SPEC Tasks

Use this skill to turn the approved spec and plan artifacts into executable work items.

- If `plan.md` is missing, use `gwt-spec-plan` first.
- If clarification blockers remain in `spec.md`, use `gwt-spec-clarify` first.
- Do not invent scope that is not traceable to `spec.md` or `plan.md`.
- When traceability gaps are mechanical and obvious, repair `tasks.md` instead of stopping the workflow.

## Required inputs

- `spec.md`
- `plan.md`
- Optional: `research.md`, `data-model.md`, `quickstart.md`, `contracts/*`

## `tasks.md` structure

`tasks.md` must use this phase model:

1. Setup
2. Foundational
3. User Story phases (`US1`, `US2`, ...)
4. Polish / Cross-Cutting

Each task must include:

- Task ID
- `[P]` when parallelizable
- linked user story ID where applicable
- exact path or module
- concrete action

## Workflow

1. **Read the source artifacts.**
   - Extract user stories, acceptance scenarios, affected modules, and contracts.

2. **Lay out phase order.**
   - Setup before shared infra
   - Foundational before story-specific tasks
   - Story phases before final polish

3. **Generate test-first tasks.**
   - Add validation tasks before or alongside implementation tasks for each story.
   - Include contract/integration/e2e coverage when the spec implies it.

4. **Add exact implementation tasks.**
   - Use concrete file paths or modules.
   - Mark `[P]` only when the write scopes do not overlap.

5. **Validate traceability.**
   - Every user story must have implementation and verification tasks.
   - Every declared contract/data-model change must have at least one task.

6. **Write `tasks.md` and continue into analysis.**
   - Return the artifact to `gwt-spec-ops`, or proceed directly to `gwt-spec-analyze` when the flow is already active.

## Exit criteria

`tasks.md` is valid only when:

- every user story is covered
- no task is vague
- test tasks are present where acceptance scenarios require proof
- parallel markers are defensible

## Operations

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:tasks.md" \
  --body-file /tmp/tasks.md
```
