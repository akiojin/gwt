# TDD Workflow Reference

## Red-Green-Refactor Loop

### Red: Write a failing test

1. **Identify the behavior** — What observable change does this task introduce?
2. **Choose the narrowest scope** — Test one behavior per test function.
3. **Write the test** — Assert the expected outcome before any implementation exists.
4. **Run and confirm failure** — The test must fail, and the failure message must indicate
   the expected behavior is missing (not a compilation error or unrelated failure).
5. **If the test passes** — The behavior already exists. Do not write redundant implementation.
   Move to the next task.

### Green: Implement the minimum

1. **Write only enough code** to make the failing test pass.
2. **Do not generalize** — Avoid implementing features that no test demands.
3. **Do not refactor** — Keep the code working first; clean up in the next step.
4. **Run the test** — Confirm the new test passes.
5. **Run adjacent tests** — Confirm no regressions in the same module.

### Refactor: Clean up while green

1. **Eliminate duplication** introduced during the Green step.
2. **Apply project conventions** — `cargo fmt`, naming, module organization.
3. **Simplify** — Remove unnecessary abstractions or dead code paths.
4. **Run the full test suite** — All tests must remain green after refactoring.
5. **Do not add behavior** — Refactoring changes structure, not behavior.

## Test design principles

### One assertion per behavior

Each test should verify a single logical behavior. Multiple asserts are acceptable when they
verify different facets of the same behavior (e.g., checking both the return value and a
side effect of the same operation).

### Descriptive test names

Test names should describe the scenario and expected outcome:

- `test_worktree_create_returns_path_when_branch_exists`
- `test_config_load_uses_default_when_file_missing`

### Arrange-Act-Assert pattern

```text
Arrange: Set up preconditions and inputs
Act:     Execute the behavior under test
Assert:  Verify the expected outcome
```

### Test isolation

- Each test must be independent of other tests.
- Do not rely on test execution order.
- Clean up any file system or state side effects.

### Reuse existing test infrastructure

- Check for existing test helpers, fixtures, and utility functions before creating new ones.
- Add to existing test modules when the new test logically belongs there.

## When to skip tests

Tests may be skipped ONLY when the change does not alter observable behavior:

| Change type | Tests required? |
|---|---|
| New feature (`feat:`) | Yes |
| Bug fix (`fix:`) | Yes |
| Refactor (`refactor:`) | Yes (existing tests must pass) |
| Documentation (`docs:`) | No |
| CI/config (`chore:`) | No |
| Formatting only | No |
| README / CLAUDE.md | No |

When in doubt, write the test. A skipped test is a potential regression.

## Per-environment guidance

Execution systems are environment-specific. Defer the broad verification
matrix to `gwt-verify` (see the active runtime's `gwt-verify/SKILL.md` and
its `references/test-matrix.md`). The TDD inner loop below still belongs to
`gwt-build-spec` — it owns *writing* the RED test, getting it GREEN, and
refactoring — while `gwt-verify` owns *which* tests run for the broader
matrix at Phase 3.

### Rust-specific guidance

#### Test placement

- Unit tests: `#[cfg(test)] mod tests` within the source file
- Integration tests: `crates/*/tests/` directory
- Prefer unit tests for internal logic, integration tests for cross-module behavior

#### Common test patterns

- Use `#[test]` for synchronous tests
- Use `#[tokio::test]` for async tests
- Use `tempfile` crate for file system tests
- Use `assert_eq!`, `assert_ne!`, `assert!(matches!(...))` for assertions

#### Running tests during the TDD inner loop

- Narrow scope first: `cargo test -p gwt-core -- test_name`
- Module scope: `cargo test -p gwt-core`
- Full suite: defer to `gwt-verify --mode full` (it picks the union of
  matched crates / frontend suites / Playwright when applicable, instead of
  hard-coding a static cargo list here).

### Frontend (WebView) guidance

- JS unit / smoke / bundle tests live under `crates/gwt/web/__tests__/` and
  are run via `pnpm test:frontend-unit` / `-smoke` / `-bundle`.
- Visual regression / browser-UI verification uses Playwright
  (`pnpm test:visual`) under `crates/gwt/playwright/`. Playwright is **only**
  for WebView/browser UI surfaces — never for Rust crates, gwtd CLI, or
  release scripts.
- The full per-surface matrix is canonical in
  `gwt-verify`'s `references/test-matrix.md`.

### Release-system guidance

- `pnpm test:release-flow` and `pnpm test:release-assets` cover release
  scripts under `scripts/`. They are invoked only when a release-system
  file is in the change set (handled automatically by `gwt-verify --mode
  pre-pr`).
