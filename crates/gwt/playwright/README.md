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

| Spec | カバー範囲 |
|---|---|
| `tests/chrome.spec.ts` | Project Bar / Status Strip / Sidebar Layers / Drawer |
| `tests/command-palette.spec.ts` | ⌘P 開閉、fuzzy filter、Enter 実行 |
| `tests/living-telemetry.spec.ts` | active/idle/blocked 遷移、pulse rim、counter sync |
| `tests/theme-toggle.spec.ts` | Dark↔Light 200ms 切替、xterm 追従 |
| `tests/mission-briefing.spec.ts` | 起動 splash、reduced-motion 縮退 |
| `tests/reduced-motion.spec.ts` | Living Telemetry 縮退 |
| `tests/forced-colors.spec.ts` | forced-colors fallback |
| `tests/adoption-surfaces.spec.ts` | 各サーフェス × Dark/Light スナップショット |
| `tests/index-status.spec.ts` | SPEC-1939 Phase 12 — Index status badge transitions, click → settings:open |

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

The spec covers:

| Test | カバー範囲 |
|---|---|
| `repair_required surfaces the red badge as a clickable button` | T-IDX-105 button + ARIA + label |
| `repairing surfaces the yellow badge with a spinner glyph` | T-IDX-104 yellow + spinner |
| `error surfaces the red badge with the failure title` | T-IDX-104 error title |
| `badge click dispatches settings:open with target=index` | T-IDX-105 dispatch |
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
