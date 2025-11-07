# Research: Releaseテスト安定化（保護ブランチ＆スピナー）

## Decision 1: Hoisted-safe mocks for worktree utilities
- **Rationale**: Vitest hoists `vi.mock` calls to the top of the module, causing Temporal Dead Zone errors when the factory references `const` variables declared later. Wrapping shared mocks inside `vi.hoisted(() => ({ ... }))` initializes them before the hoisted factory executes, preventing "initialization before access" failures without restructuring every test.
- **Alternatives considered**:
  - Inline `vi.fn()` definitions inside each `vi.mock` factory → rejected because we need to reuse the same mock instances outside the factory for assertions (`mockReset`, `mockImplementation`).
  - Convert files to CommonJS and disable hoisting → rejected; repo is ESM-only and disabling hoist would diverge from other tests.

## Decision 2: Stub git access in `App.protected-branch.test.tsx`
- **Rationale**: `handleProtectedBranchSwitch` awaits `getRepositoryRoot`, which shells out to `git rev-parse`. In Vitest happy-dom environment there is no Git repo context, so the promise rejects before `switchToProtectedBranch` runs; the test then fails even though the UI logic is correct. Spying on `getRepositoryRoot` to resolve `/repo` keeps the test purely at the UI level and allows `switchToProtectedBranch` to be asserted.
- **Alternatives considered**:
  - Mock the entire `git.ts` module via `vi.mock('../../../git.js')` → rejected; the component under test already imports `getRepositoryRoot` eagerly, so module-level mocks would have to be configured before every import and would complicate other tests that rely on real helpers.
  - Change production code to catch errors and skip `switchToProtectedBranch` → rejected; hides real runtime failures.

## Decision 3: Fully mock `execa` for spinner test
- **Rationale**: `execa` is exported as a read-only property on an ESM namespace object. `vi.spyOn` attempts to replace the property descriptor and throws "Cannot redefine property". Declaring `const { execaMock } = vi.hoisted(...); vi.mock('execa', () => ({ execa: execaMock }));` intercepts imports before evaluation, so we can safely provide a custom async process that surfaces stdout/stderr streams for the spinner assertions.
- **Alternatives considered**:
  - Use `vi.importActual('execa')` and wrap native method → still fails because property remains non-configurable.
  - Replace `import('execa')` with `await import('../../mocks/execa')` inside the test → diverges from real code paths and risks missing regressions where `execa` integration changes.

## Decision 4: Async control flow in spinner test
- **Rationale**: The spinner test relies on a timeout to emit data and resolve the mocked `execa` promise. Using `await Promise.resolve()` twice (microtask queue flush) after invoking `createWorktree` ensures `stopSpinner` is called before assertions. This mirrors the zero-delay timers already present in the mock implementation.
- **Alternatives considered**:
  - Use `vitest.useFakeTimers()` and `advanceTimersByTime(0)` → unnecessary complexity for a single test.

## Decision 5: Documentation & agent context sync
- **Rationale**: Updating Spec Kit artifacts (plan/data-model/quickstart/contracts) and running `update-agent-context.sh claude` keeps Codex/Claude guidance aligned with the current stack, preventing future agents from repeating the same mock mistakes.
- **Alternatives considered**: Manual README updates only → would violate Spec Kit workflow and risk configuration drift.
