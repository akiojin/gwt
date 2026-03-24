# Research

## Current shell inventory

### Canonical top-level shell surfaces
- `Agent Canvas`
- `Branch Browser`
- `Settings`
- `Issues`
- `PRs`
- `Version History`
- `Project Index`
- `Issue Spec`
- modal / overlay surfaces: `Launch Agent`, `Report`, `Cleanup`, `Migration`, `About`

### Current execution surfaces
- `assistant` tile on Agent Canvas
- `worktree` tiles on Agent Canvas
- `agent` session tiles on Agent Canvas
- `terminal` session tiles on Agent Canvas
- worktree detail popup / assistant detail overlay

## E2E suite inventory

Current `gwt-gui/e2e/*.spec.ts`: 20 spec files / 179 tests.

### Headed current-shell suites already migrated and passing
- `agent-canvas-browser.spec.ts` (3)
- `agent-terminal.spec.ts` (8)
- `branch-worktree.spec.ts` (8)
- `dialogs-common.spec.ts` (11)
- `project-management.spec.ts` (21)
- `responsive-performance.spec.ts` (9)

Subtotal: 6 spec files / 60 tests.

### Suites still requiring current-shell migration or coverage review
- `open-project-smoke.spec.ts` (7)
- `issue-management.spec.ts` (9)
- `pr-management.spec.ts` (17)
- `git-view.spec.ts` (9)
- `cleanup-migration.spec.ts` (9)
- `project-mode.spec.ts` (13)
- `status-bar.spec.ts` (6)
- `settings-config.spec.ts` (30)
- `windows-shell-selection.spec.ts` (6)
- `issue-cache-sync.spec.ts` (3)
- `pr-unknown-retry.spec.ts` (4)
- `voice-input-settings.spec.ts` (3)
- `tab-layout.spec.ts` (3)
- `tab-switch-performance.spec.ts` (1)

Subtotal: 14 spec files / 119 tests.

## Coverage interpretation

- A numeric E2E coverage target is not trustworthy until the stale suites above are either migrated to the current shell or explicitly retired.
- The current-shell headed batch covers the highest-signal shell / dialog / performance surfaces, but it does not represent full-screen/full-case coverage for the whole GUI.
- Backend hot paths also need non-frontend validation, so `worktree_ops` benchmarks are now part of the validation inventory.

## Key gaps still visible
- Legacy selectors and expectations still exist in stale suites (`branch-item`, `worktree-summary-panel`, old shell flows).
- Full-suite E2E coverage cannot be claimed until those stale suites are reconciled with the current shell.
- Backend perf validation is still narrow; branch inventory/worktree fast-path benchmarks exist, but more backend hot paths may need explicit perf fixtures later.
