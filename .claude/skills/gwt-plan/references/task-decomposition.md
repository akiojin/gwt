# Task Decomposition Rules

Reference for Phase 3 of `gwt-plan`. Defines how to generate `tasks.md` from the
architecture design and plan artifacts.

## Canonical Phase Order

Tasks are grouped into phases in this fixed order:

1. **Setup** — tooling, configuration, scaffolding, dependency additions
2. **Foundational** — shared infrastructure, core types, base traits
3. **User Story phases** (`US1`, `US2`, ...) — story-specific implementation
4. **Polish / Cross-Cutting** — cleanup, documentation, integration tests, CI updates

Setup and Foundational phases run before any story-specific work. Polish runs last.

## Task ID Format

Every task gets a unique ID in `T-NNN` format:

- `T-001`, `T-002`, ... for Setup
- `T-010`, `T-011`, ... for Foundational
- `T-100`, `T-101`, ... for US1; `T-200`, `T-201`, ... for US2; etc.
- `T-900`, `T-901`, ... for Polish

The numbering scheme leaves gaps for insertion without renumbering.

## Task Structure

Each task must include:

```markdown
- [ ] **T-NNN** [P] (US-X) `path/to/file_or_module` — Concrete action description
```

- `[P]` — present only when the task is parallelizable
- `(US-X)` — linked user story ID (omit for Setup/Foundational/Polish)
- Path — exact file path or module name
- Action — specific, unambiguous instruction

## Test-First Ordering

Within each user story phase, tasks follow this order:

1. **Test task** — write the test that proves the acceptance scenario
2. **Implementation task** — write the code that makes the test pass
3. **Integration task** — verify the feature works with adjacent components

Test tasks reference the acceptance scenario they verify.

## `[P]` Parallel Markers

A task may be marked `[P]` only when:

- Its write scope does not overlap with any other `[P]` task in the same phase
- It does not depend on the output of another `[P]` task
- Two implementers could work on it simultaneously without merge conflicts

Common parallel patterns:

- Independent test files for different modules
- Separate UI component implementations
- Non-overlapping config/scaffolding tasks

Do not mark tasks `[P]` speculatively. When in doubt, leave sequential.

## Traceability Validation

Before writing `tasks.md`, verify:

1. **User story coverage** — every user story in `spec.md` has at least one
   implementation task and one verification task
2. **Acceptance scenario coverage** — every acceptance scenario has a test task
   that explicitly references it
3. **Contract coverage** — every contract in `contracts/*` has a task that
   implements or updates it
4. **Data model coverage** — every entity change in `data-model.md` has a
   corresponding implementation task
5. **No orphan tasks** — every task traces back to a requirement in `spec.md`
   or a decision in `plan.md`

If a gap is found:

- If mechanical and obvious (e.g., missing test for a clearly specified scenario),
  add the task automatically
- If ambiguous (e.g., unclear which acceptance scenario a task serves), flag it
  for the quality gate

## `tasks.md` Template

```markdown
# Tasks: SPEC-<id>

## Setup

- [ ] **T-001** `path` — Description
- [ ] **T-002** [P] `path` — Description

## Foundational

- [ ] **T-010** `path` — Description

## US1: <User Story Title>

- [ ] **T-100** (US-1) `path` — Write test for acceptance scenario AS-1
- [ ] **T-101** (US-1) `path` — Implement feature to pass T-100
- [ ] **T-102** [P] (US-1) `path` — Write integration test

## US2: <User Story Title>

- [ ] **T-200** (US-2) `path` — Write test for acceptance scenario AS-2
- [ ] **T-201** (US-2) `path` — Implement feature to pass T-200

## Polish / Cross-Cutting

- [ ] **T-900** `path` — Update documentation
- [ ] **T-901** `path` — CI integration
```
