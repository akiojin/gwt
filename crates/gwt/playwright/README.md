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

## SPEC-1939 Phase 12 e2e seams

`tests/index-status.spec.ts` exercises behaviour rather than snapshots, so
the suite needs a *running* gwt instance with two env vars set. The CI
workflow (`.github/workflows/index-status-e2e.yml`) wires both seams; for
local reproduction:

```bash
# 1. Build gwt (release recommended — debug also works).
cargo build -p gwt --release

# 2. Pick a fixture for the seeded badge state.
export GWT_INDEX_TEST_FIXTURE="$PWD/crates/gwt/playwright/fixtures/index-status-repair-required.json"

# 3. Tell gwt to write its embedded server URL to a known path.
export GWT_BROWSER_URL_FILE=/tmp/gwt-browser-url
rm -f "$GWT_BROWSER_URL_FILE"

# 4. Launch gwt under a display server. xvfb on Linux, native on macOS.
target/release/gwt &
GWT_PID=$!

# 5. Wait for the URL handoff file (gwt writes it after the server is up).
while [ ! -s "$GWT_BROWSER_URL_FILE" ]; do sleep 1; done
export GWT_PLAYWRIGHT_BASE_URL="$(cat "$GWT_BROWSER_URL_FILE")"

# 6. Run the spec.
npx playwright test \
  --config crates/gwt/playwright/playwright.config.ts \
  crates/gwt/playwright/tests/index-status.spec.ts

# 7. Tear down.
kill "$GWT_PID"
```

`fixtures/` contents:

| Fixture | seeded `state` | 用途 |
|---|---|---|
| `index-status-repair-required.json` | `repair_required` | T-IDX-109 happy-path badge transitions |
| `index-status-error.json` | `error` | T-IDX-110 manual-retry path (set `GWT_INDEX_FIXTURE_KIND=error`) |
