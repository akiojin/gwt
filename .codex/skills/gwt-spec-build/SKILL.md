---
name: gwt-spec-build
description: "Use when a legacy prompt or internal handoff refers to gwt-spec-build. Prefer gwt-build-spec for visible implementation work and gwt-fix-issue when the request starts from an existing GitHub Issue."
---

# gwt-spec-build

Implement code using strict TDD (Red-Green-Refactor) methodology. Operates in two modes:

Visible owner: `gwt-build-spec`.

- **SPEC mode**: driven by an existing SPEC directory with `tasks.md`, full progress tracking
- **Standalone mode**: user provides the task directly, TDD loop enforced, no SPEC artifacts needed

## Execution ownership

Once started, keep moving until one of these is true:

- the scoped tasks are complete
- a real product or scope decision blocks the next task
- a merge conflict or reviewer request cannot be resolved with high confidence
- required auth or tooling is unavailable
- a spec-implementation mismatch is discovered that changes the design surface
  (not just a typo fix) — use `gwt-spec-brainstorm` to investigate and discuss

Routine CI failures, update-branch merges, and test-first edits are handled autonomously.

Use the current user's language for task summaries, completion reports, task
check updates, and any user-facing text generated while executing the workflow,
unless an existing artifact must keep its established language.

## Mode detection

Determine the mode at entry:

- **SPEC mode** if a SPEC ID is provided or discoverable from the current branch/context, AND
  `specs/SPEC-{id}/tasks.md` exists
- **Standalone mode** otherwise

## Phase 1: Context Load

### SPEC mode

1. Load the SPEC directory: `spec.md`, `plan.md`, `tasks.md`, `progress.md` (if present).
2. Load supporting artifacts: `research.md`, `data-model.md`, `quickstart.md`, `contracts/*`, `checklists/*`.
3. Identify the next incomplete task slice in dependency order:
   - Setup before Foundational work
   - Foundational before story-specific work
4. Use `.claude/scripts/spec_artifact.py` for artifact read/update operations.

### Standalone mode

1. Accept the user's task description, requirements, and acceptance criteria.
2. Identify target files and test locations from the task description and codebase exploration.
3. No SPEC artifacts are required or created.

## Phase 2: TDD Loop (Red-Green-Refactor)

This phase is **non-optional** in both modes. See `references/tdd-workflow.md` for detailed methodology.

### Step 1: Red — Write a failing test

- Add or update the narrowest failing test that proves the task.
- Reuse existing test suites when they already cover the target behavior.
- Rust: place tests in `crates/*/tests/` or `#[cfg(test)]` modules.
- Run the test and confirm it **fails** for the expected reason.
- If the test passes immediately, the behavior already exists — skip to the next task.

### Step 2: Green — Implement the minimum

- Write the minimum code to make the failing test pass.
- Edit only the files implied by the task (or `tasks.md` in SPEC mode).
- Do not silently widen scope beyond the current task.
- In SPEC mode, keep changes aligned with `spec.md` and `plan.md`.
- Run the test and confirm it **passes**.

### Step 3: Refactor

- Clean up the implementation while keeping all tests green.
- Apply project conventions: `cargo fmt`, naming, module structure.
- Run the full relevant test suite to catch regressions.

### When to skip tests

Tests may be skipped **only** when the task does not change observable behavior:

- Documentation-only changes (`docs:`)
- Configuration/CI changes (`chore:`)
- Formatting-only changes
- CLAUDE.md / README updates

All other changes require the TDD loop.

### Repeat

- In SPEC mode: continue task-by-task in dependency order until the scoped tasks are done.
- In standalone mode: continue until the user's stated task is complete.

## Phase 3: Verification

Run the smallest meaningful validation set first, then broaden:

1. **Tests**: `cargo test -p gwt-core -p gwt-tui` (or narrower scope first)
2. **Lint**: `cargo clippy --all-targets --all-features -- -D warnings`
3. **Format**: `cargo fmt -- --check`
4. **Build**: `cargo build -p gwt-tui`

In SPEC mode, also verify:

- Implementation matches `spec.md` acceptance scenarios
- No scope creep beyond `tasks.md`
- If a failure indicates a spec gap (not a code bug), route back to `gwt-design-spec`

## Phase 4: PR Flow

Handle PR operations autonomously using the `gwt-manage-pr` skill:

- If there is no active PR for the branch, or prior PRs are already merged, use `gwt-manage-pr` to create one.
- If the current PR has CI failures, conflicts, or review blockers, use `gwt-manage-pr` to fix.
- Let `gwt-manage-pr` handle routine merge/push/fix loops.

Do not create a PR until Phase 3 verification passes.

## Phase 5: Completion Gate

### SPEC mode

Reconcile implemented behavior against all SPEC artifacts. See `references/completion-gate.md` for details.

Required checks:

- Every completed task in `tasks.md` matches the implementation
- `checklists/acceptance.md` reflects actual accepted behavior
- `checklists/tdd.md` reflects actual verification evidence
- `progress.md` entries do not claim completion unsupported by code
- If artifacts disagree, return to `gwt-design-spec` — do not proceed to PR

Update execution tracking:

- Mark completed tasks in `tasks.md`
- Append progress to `specs/SPEC-{id}/progress.md` using the Progress / Done / Next template
- Use `.claude/scripts/spec_artifact.py` for artifact updates

### Standalone mode

- Confirm all user-stated acceptance criteria are met
- Confirm all tests pass
- Confirm lint and type checks pass
- Summarize what was built and what was verified

### Chain suggestion

On completion, suggest `gwt-arch-review` for code review if available, or proceed to `gwt-manage-pr` if not already done.

## Stop conditions

Stop only when:

- the next task depends on a product or scope decision that is not inferable
- a merge conflict or review request is ambiguous enough to risk wrong behavior
- required repo/auth/tooling access is unavailable
- implementation reveals a gap between spec.md and the actual behavior
  (acceptance scenario inaccuracy, missing data-model section, undocumented
  registration table, dependency chain not captured in tasks.md, etc.) that
  changes the design surface — not just a typo fix. Route to
  `gwt-spec-brainstorm` if the gap requires user discussion, or update the
  SPEC artifact directly if the fix is mechanical

## Exit report

```text
## <Build Report in the current user's language>

Mode: SPEC-<id> | Standalone
Completed tasks: <N> | <summary>
Updated files: <paths>
Verification:
- <command/result>

Next:
- `gwt-manage-pr` (create/update PR)
- `gwt-arch-review` (code review)
- return to `gwt-design-spec` (artifact repair)
- ask user for decision
```
