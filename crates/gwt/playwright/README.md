# Playwright — SPEC-2356 Operator Design System / SPEC-1939 Phase 12 e2e

Visual regression baseline for the Operator Design System surfaces and
behaviour e2e for the Project Index status badge (SPEC-1939 Phase 12).

## Run

```bash
# from repo root — runs every spec; specs without GWT_PLAYWRIGHT_BASE_URL
# skip themselves so the suite is safe to invoke without a running gwt.
npm run test:visual
```

## Update baseline (when intentional design change lands)

```bash
npx playwright test --update-snapshots --config crates/gwt/playwright/playwright.config.ts
```

## Test layout

| Spec | カバー範囲 | スタイル |
|---|---|---|
| `tests/chrome.spec.ts` | Operator chrome smoke (Project Bar / Status Strip / hover-reveal peek 帯) | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/theme-toggle.spec.ts` | Dark↔Light 200ms 切替、xterm 追従 | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/reduced-motion.spec.ts` | Living Telemetry 縮退 | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/kanban.spec.ts` | SPEC-2017 Knowledge Bridge Kanban visual snapshots | embedded-frontend fixture |
| `tests/index-status.spec.ts` | SPEC-1939 Phase 12 Index status badge / Settings.Index / project tab dot / per-cell rebuild | embedded-frontend fixture |

`live-gwt` specs require the CI workflow (or a developer) to launch a
real `gwt` instance and export `GWT_PLAYWRIGHT_BASE_URL`. `embedded-frontend`
specs serve `crates/gwt/web/**` through Playwright `page.route` and stub
`WebSocket` directly, so they run unmodified under the standard
`Visual Regression` workflow.

## SPEC-1939 Phase 12 e2e

`tests/index-status.spec.ts` follows the SPEC-2017 Kanban fixture pattern
(`tests/kanban.spec.ts`): the spec serves the embedded frontend assets
through Playwright `page.route()` and replaces `WebSocket` with a
deterministic backend that emits canned `workspace_state` and
`project_index_status` events. **No live gwt process / xvfb / WebKit
required**, so the suite runs reliably under the existing
`Visual Regression` workflow without any extra orchestration.

To reproduce locally, simply run:

```bash
npm run test:visual -- --project=chromium-dark crates/gwt/playwright/tests/index-status.spec.ts
```

The spec covers (chromium-dark + chromium-light):

| Test | カバー範囲 |
|---|---|
| `repair_required surfaces the red badge as a clickable button` | T-IDX-105 button + ARIA + label |
| `repairing surfaces the yellow badge with a spinner glyph` | T-IDX-104 yellow + spinner |
| `ready surfaces the green badge with the steady-state label` | T-IDX-104 green |
| `skipped keeps the badge hidden so non-git projects do not flash chrome` | T-IDX-104 hidden 分岐 |
| `error surfaces the red badge with the failure title` | T-IDX-104 error title |
| `badge click dispatches settings:open with target=index` | T-IDX-105 dispatch |
| `badge transitions repair_required -> repairing -> ready over WebSocket events` | T-IDX-109 state machine |
| `project tab dot reflects aggregated worktree health` | T-IDX-107 single-worktree |
| `multi-worktree dot aggregates: unhealthy in one worktree turns the dot red` | T-IDX-107 multi-worktree |
| `badge click opens Settings.Index tab and renders the scope health table` | T-IDX-106 panel + table |
| `Settings.Index scope-row Rebuild all dispatches without worktree_hash` | T-IDX-102 scope-row dispatch |
| `Settings.Index per-cell Rebuild dispatches rebuild_index_cell IPC` | T-IDX-102 per-cell / T-IDX-110 retry |
| `repairing click shows a progress toast` | T-IDX-108 toast |

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
