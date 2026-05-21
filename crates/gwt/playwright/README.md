# Playwright — SPEC-2356 Operator Design System / SPEC-1939 Phase 12 / 13 e2e

Visual regression baseline for the Operator Design System surfaces and
behaviour e2e for the Project Index health UX (SPEC-1939 Phase 12, with
the Phase 13 badge withdrawal applied — see "SPEC-1939 Phase 12 / 13 e2e"
below).

## Run

```bash
# from repo root — runs every spec; specs without GWT_PLAYWRIGHT_BASE_URL
# skip themselves so the suite is safe to invoke without a running gwt.
bash scripts/run-visual-tests.sh
```

## Update baseline (when intentional design change lands)

```bash
pnpm dlx --package @playwright/test@1.49.1 playwright test \
  --update-snapshots --config crates/gwt/playwright/playwright.config.ts
```

## Test layout

| Spec | カバー範囲 | スタイル |
|---|---|---|
| `tests/chrome.spec.ts` | Operator chrome smoke (Project Bar / Status Strip / hover-reveal peek 帯) | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/theme-toggle.spec.ts` | Dark↔Light 200ms 切替、xterm 追従 | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/reduced-motion.spec.ts` | Living Telemetry 縮退 | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/kanban.spec.ts` | SPEC-2017 Knowledge Bridge Kanban visual snapshots | embedded-frontend fixture |
| `tests/index-status.spec.ts` | SPEC-1939 Phase 13 Index status surface (per-tab dot / Settings.Index / per-cell rebuild) | embedded-frontend fixture |

`live-gwt` specs require the CI workflow (or a developer) to launch a
real `gwt` instance and export `GWT_PLAYWRIGHT_BASE_URL`. `embedded-frontend`
specs serve `crates/gwt/web/**` through Playwright `page.route` and stub
`WebSocket` directly, so they run unmodified under the standard
`Visual Regression` workflow.

## SPEC-1939 Phase 12 / 13 e2e

`tests/index-status.spec.ts` follows the SPEC-2017 Kanban fixture pattern
(`tests/kanban.spec.ts`): the spec serves the embedded frontend assets
through Playwright `page.route()` and replaces `WebSocket` with a
deterministic backend that emits canned `workspace_state` and
`project_index_status` events. **No live gwt process / xvfb / WebKit
required**, so the suite runs reliably under the existing
`Visual Regression` workflow without any extra orchestration.

> **Phase 13 scope change.** The project-bar `Index:` badge has been
> withdrawn (concept separation between repo-shared `issues`/`specs`
> and per-worktree `files`/`files-docs`). The earlier badge state /
> spinner / progress-toast tests have been removed; remaining coverage
> drives the per-tab dot aggregator and Settings.Index panel via the
> canonical `settings:open` event.

To reproduce locally, simply run:

```bash
bash scripts/run-visual-tests.sh --project=chromium-dark crates/gwt/playwright/tests/index-status.spec.ts
```

The spec covers (chromium-dark + chromium-light):

| Test | カバー範囲 |
|---|---|
| `project-bar Index badge has been withdrawn (Phase 13)` | Phase 13 撤去確認 (`#index-status` が DOM に存在しない) |
| `project tab dot reflects aggregated worktree health (T-IDX-107)` | T-IDX-107 single-worktree |
| `multi-worktree dot aggregates: unhealthy in one worktree turns the dot red` | T-IDX-107 multi-worktree |
| `Settings.Index renders the scope health table from project_index_status (T-IDX-106)` | T-IDX-106 panel + table (settings:open 直接 dispatch) |
| `Settings.Index scope-row Rebuild all dispatches without worktree_hash` | T-IDX-102 scope-row dispatch |
| `Settings.Index per-cell Rebuild dispatches rebuild_index_cell IPC (T-IDX-102/T-IDX-110)` | T-IDX-102 per-cell / T-IDX-110 retry |

### Optional production debugging seams

If you want to drive a real gwt instance (e.g. for screenshot-based UX
review on macOS), two env-controlled seams are still available:

- `GWT_INDEX_TEST_FIXTURE=<json>` — `aggregate_project_index_status_for_path`
  (`crates/gwt/src/index_worker.rs`) returns the parsed view immediately
  instead of running the real Python runner. Fixture JSON files in
  `fixtures/` (`index-status-repair-required.json`, `index-status-error.json`)
  are kept around for this manual workflow.
- `GWT_BROWSER_URL_FILE=<path>` — `crates/gwt/src/main.rs` writes the
  embedded server URL to that path so an external script can pick it up
  without parsing stderr.

Both seams are no-ops when the env vars are unset and never run in normal
production.
