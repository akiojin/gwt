## Phase 2: TDD Loop (Red-Green-Refactor)

This phase is **non-optional** in both modes. See [references/tdd-workflow.md](tdd-workflow.md) for detailed methodology.

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
