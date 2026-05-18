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

#### Acceptance verification (tasks section)

- Each acceptance criterion reflects actual tested behavior.
- No criterion is marked "accepted" without a corresponding passing test.

#### TDD verification (tasks section)

- Each entry reflects actual verification evidence (test names, command output).
- No entry claims coverage that does not exist.

### 4. tasks section consistency

- tasks セクションのチェックボックスがアーティファクトやコードで裏付けられていない完了を主張していない。
- 最終的な tasks セクションの状態が全体の実装状況を正確に要約している。

### 5. Code verification

Delegate to `gwt-verify --mode full` and require `Overall: PASS` in the
evidence bundle. The sub-skill picks the matrix per changed surface (see
`.claude/skills/gwt-verify/references/test-matrix.md`) — cargo for Rust
crates, `pnpm test:frontend-*` for frontend JS, `pnpm test:visual` for
WebView/browser UI surfaces only (Playwright is not invoked for non-browser
surfaces), `pnpm test:release-*` for release-system changes, and
`pnpm lint:skills` for skill assets.

If `gwt-verify` returns `Overall: FAIL` or `failed: tooling-missing`, the
gate fails. Do not declare completion. Route the failure for repair per
the failure-handling section below.

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
3. Route back to `gwt-discussion` to repair the artifact set, then re-run
   `gwt-plan-spec` if the planning artifacts changed.
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
