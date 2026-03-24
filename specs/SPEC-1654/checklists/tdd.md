# TDD Checklist

- [x] Add shell state tests for flat tabs + `agentCanvas` + `branchBrowser`
- [x] Add Agent Canvas tests for tiles, viewport, popup, and relation edges in the current interaction model
- [x] Add Branch Browser tests for `local / remote / all` projections and create/focus actions
- [x] Add migration tests from old agent/terminal tabs to canvas tiles
- [x] Add multi-window restore tests for window-local shell state under the remediated shell model
- [x] Add e2e coverage for Branch Browser -> worktree -> agent/terminal tile flows
- [x] Add regression coverage that `Agent Canvas` and `Branch Browser` are full-window single-surface tabs with no persistent side-by-side detail panes
- [x] Add startup responsiveness E2E with slow background issue-cache warmup and a 1000ms interactive budget
- [x] Add maximize/restore responsiveness E2E with a 300ms interactive budget and no heavy refetch regression
