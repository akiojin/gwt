# Progress Tracking Reference

This reference applies to **SPEC mode only**. Standalone mode does not use SPEC artifacts.

## tasks.md update format

When a task is completed, update `tasks.md` to reflect completion:

```markdown
- [x] Task description (completed)
- [ ] Task description (remaining)
```

Rules:

- Update tasks.md after each task slice is verified, not before.
- Do not mark tasks complete until the implementation passes verification.
- Do not batch-mark multiple tasks; update incrementally.
- Use JSON operation `issue.spec.edit` for artifact updates.

## Progress Tracking

Update tasks-section checkboxes with JSON operation `issue.spec.edit`.

### Rules

- Update incrementally from verified facts.
- Do not claim completion unless the code supports it.
- Include verification command output, including pass/fail counts.
- Reference the concrete task IDs from `tasks.md`.

## Completion markers

### Task-level completion

A task is complete when:

1. The failing test was written (Red)
2. The implementation makes it pass (Green)
3. Refactoring is done with all tests green
4. Verification commands pass (`cargo test`, `cargo clippy`, `cargo fmt`)

### Phase-level completion

A phase is complete when all tasks in that phase are marked `[x]` and verified.

### SPEC-level completion

A SPEC is complete only after the Completion Gate (Phase 5) passes.
Do not mark the SPEC complete during progress tracking; that is the gate's responsibility.

## False completion detection

Watch for these signals that indicate premature completion claims:

- Tasks marked `[x]` but related tests are not in the codebase
- The tasks section claims "all tests pass" but `cargo test` output shows a failure
- The tasks section marks acceptance as "accepted" while the behavior is still unimplemented
- Task marked complete but the file listed in the task was not modified

If any of these are detected, revert the completion marker and return to implementation.
