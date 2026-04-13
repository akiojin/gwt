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
- Use `.claude/scripts/spec_artifact.py` for artifact operations.

## progress.md template

Append entries to `specs/SPEC-{id}/progress.md` after each verified task slice.

### Entry format

```markdown
## YYYY-MM-DD HH:MM — <brief summary>

**Progress:**
- <what was accomplished in this slice>
- <specific files changed>

**Done:**
- <verification results: test pass/fail, lint, build>
- <task IDs completed>

**Next:**
- <next task to execute>
- <any blockers or decisions needed>
```

### Rules

- Keep entries factual and incremental.
- Do not claim completion that the code does not support.
- Include verification command output (pass/fail counts).
- Reference specific task IDs from `tasks.md`.
- Append new entries; do not overwrite previous entries.

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
- `progress.md` claims "all tests pass" but `cargo test` output shows failures
- `checklists/acceptance.md` says "accepted" but the behavior is not implemented
- Task marked complete but the file listed in the task was not modified

If any of these are detected, revert the completion marker and return to implementation.
