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

## Rust-specific guidance

### Test placement

- Unit tests: `#[cfg(test)] mod tests` within the source file
- Integration tests: `crates/*/tests/` directory
- Prefer unit tests for internal logic, integration tests for cross-module behavior

### Common test patterns

- Use `#[test]` for synchronous tests
- Use `#[tokio::test]` for async tests
- Use `tempfile` crate for file system tests
- Use `assert_eq!`, `assert_ne!`, `assert!(matches!(...))` for assertions

### Running tests

- Narrow scope first: `cargo test -p gwt-core -- test_name`
- Module scope: `cargo test -p gwt-core`
- Full suite: `cargo test -p gwt-core -p gwt-tui`
