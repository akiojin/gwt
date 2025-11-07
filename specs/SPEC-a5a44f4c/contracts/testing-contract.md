# Testing Contract: Releaseテスト安定化

## Protected Branch Flow Contract

| Aspect | Expectation |
| --- | --- |
| Input | `selectedBranch={ name: 'main', branchType: 'local', branchCategory: 'main' }` |
| Dependencies | `getRepositoryRoot` resolves `/repo`; `switchToProtectedBranch` mock resolves `'local' | 'remote' | 'none'` |
| Side effects | `navigateTo('ai-tool-selector')`, `refresh()` called once, footer message updated to success text |
| Assertions | `switchToProtectedBranch` called with `{ branchName: 'main', repoRoot: '/repo', remoteRef: null }` | 

## Navigation/Acceptance Contract

| Aspect | Expectation |
| --- | --- |
| Input | Integration/acceptance tests provide `mockBranches` & `mockWorktrees` arrays |
| Mock behavior | `isProtectedBranchName` returns boolean, `switchToProtectedBranch` returns `'none'` by default |
| Reset | Every `beforeEach` calls `mockReset` on both mocks |
| Failure mode | If mock not reset, test must fail fast with explicit error message |

## Spinner Contract

| Aspect | Expectation |
| --- | --- |
| Input | `createWorktree` invoked with `isNewBranch` toggle |
| Mock behavior | `execaMock` returns promise with `stdout`/`stderr` PassThrough; resolves within 1 tick |
| Spinner lifecycle | `startSpinner` called once, returns `stopSpinner` that is invoked when promise resolves |
| Assertions | `stopSpinner` called ≥1, `execaMock` called with `['git', 'worktree', 'add', ...]` |
