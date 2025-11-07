# Data Model: Releaseテスト安定化（保護ブランチ＆スピナー）

| Entity | Description | Key Fields | Relationships |
| --- | --- | --- | --- |
| `ProtectedBranchMock` | Shared Vitest mock for `isProtectedBranchName` & `switchToProtectedBranch`. Lives inside `vi.hoisted`. | `mock`: `vi.fn`, `resolves`: `'none' | 'local' | 'remote'`, `callHistory`: array | Used by integration + acceptance tests; reset in `beforeEach`, inspected via `const mocked = mock as Mock`. |
| `RepoRootStub` | Spy produced by `vi.spyOn(gitModule, 'getRepositoryRoot')`. | `mockResolvedValue('/repo')`, `restore()` | Scoped to `App.protected-branch.test.tsx`; ensures UI flow invokes Git helper without shelling out. |
| `BranchActionFlow` | Aggregated props captured by `BranchActionSelectorScreen` spy. | `latestProps`, `onUseExisting`, `onCreateNew` | Receives `ProtectedBranchMock` output; emits navigation side-effects verified in tests. |
| `ExecaMockProcess` | Custom promise returned by mocked `execa`. | `stdout: PassThrough`, `stderr: PassThrough`, `resolve(data)` | Consumed by `worktree.createWorktree` inside spinner test; triggers `startSpinner`/`stopSpinner`. |
| `SpinnerCallbacks` | Pair of functions from `startSpinner`. | `startSpinnerSpy`, `stopSpinner` (vi.fn) | Observed to ensure spinner lifecycle matches CLI UX. |

## State Transitions

1. **Protected Branch Selection**
   - `BranchListScreen.onSelect` → sets `selectedBranch` (App state) → `isProtectedSelection` queries `ProtectedBranchMock` → enters branch-action screen.
   - When `onUseExisting` fires: `RepoRootStub` resolves path → `switchToProtectedBranch` mock resolves `'local'|'remote'|'none'` → navigation to `'ai-tool-selector'`.

2. **Spinner Lifecycle**
   - `createWorktree` is called with spinner wrapper → `startSpinner` returns `stopSpinner` stub.
   - `ExecaMockProcess` emits `stdout` event then resolves promise → `stopSpinner` invoked → assertions verify spinner completed and `execa` invoked once.

## Validation Rules

- `ProtectedBranchMock` must always return boolean for `isProtectedBranchName`; default is `false` but tests can override per scenario.
- `RepoRootStub` must resolve to absolute path; relative paths risk `switchToProtectedBranch` failure.
- `ExecaMockProcess` must resolve to object containing `stdout` and `stderr`; missing properties break spinner log parsing.
