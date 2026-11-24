# Playwright — WebView behavior E2E

Rendered behavior coverage for the Operator Design System surfaces, project
entry flow, and Project Index health UX. The suite exercises both dark and
light themes with deterministic embedded-backend fixtures; live-backend specs
run when `GWT_PLAYWRIGHT_BASE_URL` is set.

## Run

The visual test runner supports Node.js 22.x and 26.x. CI runs the unit and
smoke tests on Node.js 22 and the browser specs inside the official Playwright
image, which ships Node.js 26. Check `node --version` before running the
suite. If another version is active, put Node.js 22.x or 26.x first on `PATH`
and try again.

The pinned Playwright must stay at 1.65.1 or later for Node.js 26. Playwright
1.49.1 bundled yauzl 2.10.0 and fd-slicer 1.1.0, whose read stream set
`destroyed = true` on itself before it pushed the end of the data. Node.js 22
still delivers the data buffered in such a stream; Node.js 26 does not, and
never emits `end`. A trace with an entry whose compressed size is over 64 KB
(a font, for example) then never got its central directory, and a test
recorded with `--trace=on` passed its assertions and timed out (Issues #7494
and #7791).

```bash
# from repo root — runs every spec; specs without GWT_PLAYWRIGHT_BASE_URL
# skip themselves so the suite is safe to invoke without a running gwt.
bash scripts/run-visual-tests.sh
```

## Test layout

| Spec | カバー範囲 | スタイル |
|---|---|---|
| `tests/chrome.spec.ts` | Operator chrome smoke (Project Bar / Status Strip / hover-reveal peek 帯) | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/theme-toggle.spec.ts` | Dark↔Light 200ms 切替、xterm 追従 | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/reduced-motion.spec.ts` | Living Telemetry 縮退 | live-gwt (skip-if-no-`GWT_PLAYWRIGHT_BASE_URL`) |
| `tests/kanban.spec.ts` | SPEC-2017 Knowledge Bridge list/detail behavior | embedded-frontend fixture |
| `tests/index-status.spec.ts` | SPEC-1939 Phase 13 Index status surface (per-tab dot / Settings.Index / per-cell rebuild) | embedded-frontend fixture |
| `tests/home-conversation.spec.ts` | Issue #7639 Home (PM conversation) / Projects view of the page at `/` | embedded-frontend fixture |
| `tests/first-run-setup.spec.ts` | Issue #7731 first-run setup on the page at `/` (tool gate, Project Manager row, reopening a step from Settings) | embedded-frontend fixture |
| `tests/agent-history-plans.spec.ts` | Issue #7649 agent history and planned assignments on the project's Agents screen; the ledger shows neither | embedded-frontend fixture |
| `tests/task-experience.spec.ts` | SPEC #7007 T-022 (Issue #7640) verifies Home decisions, PM conversation history, the Agents terminal and direct Notifications | isolated checkout (`GWT_PLAYWRIGHT_TASK_EXPERIENCE=1`, real `decision.*` gwtd operations) |
| `tests/v3-regression.spec.ts` | SPEC #7007 T-036 (Issue #7654) drives every v3 screen, checks that decision counts agree across header, Decisions, Home and status bar before and after an answer, and that the PM view is gone | isolated checkout (`GWT_PLAYWRIGHT_TASK_EXPERIENCE=1`, real `decision.*` gwtd operations) |
| `tests/terminal-requirements.spec.ts` | SPEC #7007 T-036 (Issue #7654) reproduces terminal requirements (2) scroll cost and (3) wide characters | embedded-frontend fixture |
| `tests/project-open-surfaces.spec.ts` | Project Picker / Open Folder / Clone / non-Git onboarding | embedded-frontend fixture through the shared live harness |

`live-gwt` specs require the CI workflow (or a developer) to launch a
real `gwt` instance and export `GWT_PLAYWRIGHT_BASE_URL`. `embedded-frontend`
specs serve `crates/gwt/web/**` through Playwright `page.route` and stub
`WebSocket` directly, so they run unmodified under the standard frontend
WebView E2E job.

## SPEC #7007 v3 regression (T-036)

The v3 regression is the set of specs below, each run headed in both themes
with zero console and page errors. Build the checkout binaries first
(`cargo build -p gwt --bin gwt --bin gwtd`).

```bash
GWT_PLAYWRIGHT_TASK_EXPERIENCE=1 bash scripts/run-visual-tests.sh --headed --workers=1 \
  crates/gwt/playwright/tests/v3-regression.spec.ts \
  crates/gwt/playwright/tests/task-experience.spec.ts \
  crates/gwt/playwright/tests/terminal-requirements.spec.ts \
  crates/gwt/playwright/tests/terminal-input-disconnect.spec.ts \
  crates/gwt/playwright/tests/agent-window-scroll.spec.ts \
  crates/gwt/playwright/tests/home-conversation.spec.ts \
  crates/gwt/playwright/tests/first-run-setup.spec.ts \
  crates/gwt/playwright/tests/agent-history-plans.spec.ts \
  crates/gwt/playwright/tests/project-themes.spec.ts \
  crates/gwt/playwright/tests/provider-usage.spec.ts
```

### Terminal requirements (design items 58 and 185)

Each requirement passes when its reproduction procedure runs and the problem
does not occur. Live specs need a running gwt (`GWT_PLAYWRIGHT_BASE_URL`).

| Requirement | Reproduction procedure |
|---|---|
| (1) Output is scheduled in frames and batches, so heavy output does not block input or drawing | `terminal-output-batching-live.spec.ts` (eight real PTYs stream while previews converge at a bounded cadence); `terminal-input-continuity-live.spec.ts` (ASCII, IME and paste reach the PTY once through three minutes of output) |
| (2) Scrolling does not get heavier over time | `terminal-requirements.spec.ts` (2): the same wheel scroll is timed before and after 25 times the output; the rendered rows stay the screen's rows |
| (3) Wide characters leave no ghost and never overlap | `terminal-requirements.spec.ts` (3): wide text is overwritten by narrower text, half a wide cell and a cleared line, and no glyph or overlap remains; `terminal-grid-responsiveness-live.spec.ts` (split UTF-8 through the live pipeline) |
| (4) Scrollback replay on reconnect is not cut short | `terminal-scrollback-live.spec.ts` (history survives the real owner snapshot); `agent-window-scroll.spec.ts` (scrollback after a `terminal_snapshot` reset while hidden) |
| (5) Terminal queries are answered correctly, with no input-band artifact | `cargo test -p gwt-terminal owner_output_consumes` (the host answers DA1, OSC 10/11 and cursor position itself, so the browser never answers twice); `terminal-codex-composer-live.spec.ts` (the composer paints before the browser attaches) |
| (6) A lost session does not stick, and input is never dropped silently | `terminal-input-disconnect.spec.ts` (typing while disconnected is refused with a persistent notice, and input works again after reconnect) |

## SPEC-1939 Phase 12 / 13 e2e

`tests/index-status.spec.ts` follows the SPEC-2017 Kanban fixture pattern
(`tests/kanban.spec.ts`): the spec serves the embedded frontend assets
through Playwright `page.route()` and replaces `WebSocket` with a
deterministic backend that emits canned `workspace_state` and
`project_index_status` events. **No live gwt process / xvfb / WebKit
required**, so the suite runs reliably under the existing frontend WebView E2E
job without any extra orchestration.

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
