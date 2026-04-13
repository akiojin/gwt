# Completion Gate Reference

The completion gate runs at the end of Phase 5 before declaring work done.
In SPEC mode, it reconciles all artifacts. In standalone mode, it verifies user criteria.

## SPEC mode reconciliation checks

### 1. tasks.md agreement

- Every task marked `[x]` in `tasks.md` has corresponding implementation in the codebase.
- No tasks are marked complete without verification evidence.
- Remaining `[ ]` tasks are documented as out-of-scope or deferred with rationale.

### 2. spec.md alignment

- Each functional requirement (FR-*) in `spec.md` maps to at least one completed task.
- Each acceptance scenario in `spec.md` has a corresponding test or verification step.
- No implemented behavior contradicts `spec.md`.

### 3. Checklist verification

#### checklists/acceptance.md

- Each acceptance criterion reflects actual tested behavior.
- No criterion is marked "accepted" without a corresponding passing test.

#### checklists/tdd.md

- Each entry reflects actual verification evidence (test names, command output).
- No entry claims coverage that does not exist.

### 4. progress.md consistency

- Progress entries do not claim completion that artifacts or code do not support.
- The final progress entry summarizes the overall implementation state accurately.

### 5. Code verification

Run all verification commands and confirm success:

```bash
cargo test -p gwt-core -p gwt-tui
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt -- --check
cargo build -p gwt-tui
```

## Standalone mode checks

### 1. Acceptance criteria met

- Each criterion stated by the user has been implemented and verified.
- Ask the user to confirm if criteria were informal or ambiguous.

### 2. Test coverage

- Every new behavior has at least one test.
- All tests pass.

### 3. Code quality

- Lint and format checks pass.
- Build succeeds.
- No unresolved warnings or errors.

## Failure handling

### Artifact disagreement (SPEC mode)

If any reconciliation check fails:

1. Do NOT proceed to PR.
2. Do NOT declare completion.
3. Route back to `gwt-spec-ops` to repair the artifact set.
4. After repair, re-run the completion gate.

### Verification failure (both modes)

If any verification command fails:

1. Diagnose the failure (test bug vs. implementation bug vs. spec gap).
2. Return to Phase 2 (TDD Loop) to fix.
3. Re-run Phase 3 (Verification) after fix.
4. Only proceed when all checks pass.

### False completion signals

Do not treat these as completion authority:

- GitHub Issue bodies or comments (contextual references only)
- PR descriptions (may be outdated)
- Verbal user confirmation without code evidence

The source of truth is always: code + tests + verification output.

## Gate output

Use the current user's language for the final gate summary and any user-facing
completion text.

After the gate passes, produce the exit report as specified in the main SKILL.md.
If the gate fails, produce a diagnostic report indicating which checks failed and what
action is needed.
