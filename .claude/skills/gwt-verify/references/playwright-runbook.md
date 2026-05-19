# Playwright Runbook

`gwt-verify` invokes Playwright **only** for WebView/browser UI surfaces (see
`test-matrix.md`). This document covers the operational details required to
run `pnpm test:visual` correctly in both headless (default) and headed
(`--headed`) modes.

## Project Layout

The Playwright project lives under `crates/gwt/playwright/`:

- `playwright.config.ts` — Chromium dark + Chromium light projects, snapshot
  template `{snapshotDir}/{testFilePath}/{projectName}/{platform}/{arg}{ext}`.
- `tests/*.spec.ts` — visual / interaction tests against the embedded gwt
  frontend.
- `snapshots/` — committed baseline images.
- `fixtures/` — WS server responses used by individual tests.

## GUI Server Bring-up

`pnpm test:visual` expects a running gwt WebView server at the URL declared
by `GWT_PLAYWRIGHT_BASE_URL`. The skill is responsible for getting that
server ready before invoking Playwright.

```bash
# 1. Build (debug mode is fine for verify runs)
cargo build -p gwt --bin gwt

# 2. Start the GUI server in the background
./target/debug/gwt &
gui_pid=$!

# 3. Capture the published URL line:
#    gwt browser URL: http://127.0.0.1:<port>/
# 4. curl the URL until it returns HTTP 200 (timeout 30s)
# 5. export GWT_PLAYWRIGHT_BASE_URL=<that url>
# 6. Run Playwright
pnpm test:visual

# 7. Stop the GUI server
kill "$gui_pid"
```

If `GWT_PLAYWRIGHT_BASE_URL` is already set in the environment (e.g. by a
preceding test harness) the skill must reuse it instead of spawning a new
server, to avoid port collisions.

## Headless vs Headed

| Invocation | Behavior |
|---|---|
| `pnpm test:visual` (default) | Headless Chromium dark + light projects. Same conditions as CI. |
| `pnpm test:visual --headed` | Headed Chromium. Used only when the caller passed `--headed` to `gwt-verify`. The skill must surface the GUI URL to the user so they can also inspect the running app. |
| `pnpm test:visual --update-snapshots` | Snapshot baseline update. **Not** part of `gwt-verify` — the caller must run it explicitly as a separate manual step. |

## Snapshot Diff Policy

- Visual regression failures are evidence-bundle FAIL, not snapshot
  regeneration triggers.
- The skill never passes `--update-snapshots` automatically. Baseline updates
  must be performed by a human-driven step with intent, then committed
  separately so the change shows up in the PR review.
- `maxDiffPixelRatio: 0.005` in `playwright.config.ts` is the canonical
  tolerance; the skill does not override it.

## Project Selection

Both `chromium-dark` and `chromium-light` projects run by default. To narrow
during quick iteration, the operator may run `pnpm test:visual --project
chromium-dark` manually; the skill itself always runs both so coverage
matches CI.

## Common Failure Modes

| Symptom | Likely cause | Action |
|---|---|---|
| `Error: connect ECONNREFUSED 127.0.0.1:<port>` | GUI server not yet ready | Re-check the HTTP 200 wait step before invoking Playwright. |
| `Snapshot doesn't match` | Real UI regression OR legitimate redesign | Treat as FAIL; do not auto-regenerate snapshots. Triage in `gwt-discussion` or escalate to the user. |
| Headed mode shows no window | Running inside a headless host (CI, container) | Detect missing display and downgrade to `Overall: FAIL` with `Headed verification: refused (no display)`. Do not silently fall back to headless. |
| `browserType.launch: Executable doesn't exist` | Chromium not installed for current Playwright version | Trigger the tooling-bootstrap path (`pnpm exec playwright install --with-deps chromium`). |

## Boundary

Playwright is for WebView/browser UI verification. Never invoke it for:

- Pure Rust changes (`crates/*` outside `crates/gwt/web/**` and
  `crates/gwt/playwright/**`).
- gwtd CLI smoke (use `cargo run -p gwt --bin gwtd -- --version` instead).
- Backend domain logic (use `cargo test -p <crate>`).
- Release scripts (use `pnpm test:release-flow` / `test:release-assets`).
- Documentation-only changes (markdownlint only).

This boundary is enforced by `gwt-verify` at command-selection time per the
canonical surface table.
