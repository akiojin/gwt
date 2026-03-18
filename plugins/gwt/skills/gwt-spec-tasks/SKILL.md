---
name: gwt-spec-tasks
description: Generate `tasks.md` for an existing `gwt-spec` from `spec.md` and `plan.md`, grouped by phase and user story with exact file paths, `[P]` parallel markers, and test-first ordering. Use after `gwt-spec-plan`.
---

# gwt SPEC Tasks

Use this skill to turn the approved spec and plan artifacts into executable work items.

- If `plan.md` is missing, use `gwt-spec-plan` first.
- If clarification blockers remain in `spec.md`, use `gwt-spec-clarify` first.
- Do not invent scope that is not traceable to `spec.md` or `plan.md`.

## Required inputs

- `doc:spec.md`
- `doc:plan.md`
- Optional: `doc:research.md`, `doc:data-model.md`, `doc:quickstart.md`, `contract:*`

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

6. **Write `tasks.md` and hand off to `gwt-spec-analyze`.**

## Exit criteria

`tasks.md` is valid only when:

- every user story is covered
- no task is vague
- test tasks are present where acceptance scenarios require proof
- parallel markers are defensible
