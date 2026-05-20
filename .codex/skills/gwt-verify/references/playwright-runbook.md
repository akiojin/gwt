# Browser / WebView UI Runner Runbook

`gwt-verify` invokes a browser-based UI runner **only** for WebView /
browser UI surfaces (see `surface-taxonomy.md`). Playwright is the
canonical example documented here; the same operational pattern applies
to Cypress, Selenium WebDriver, WinAppDriver, and Unity Editor headed.
For non-UI surfaces, this runbook does not apply — the runner is not
invoked at all.

The contract is the same regardless of the specific runner:

1. The application under test must be running and reachable before the
   runner starts.
2. The runner receives the application's base URL / executable path
   through an environment variable agreed with the test harness.
3. The runner runs **headless by default** (CI conditions). Headed mode
   is opt-in via the `--headed` flag passed to `gwt-verify`.
4. Snapshot or screenshot diffs are FAIL signals; the skill never
   regenerates baselines silently.

## Project Layout (Playwright example, from the gwt repo)

In the gwt repository, the Playwright project lives under
`crates/gwt/playwright/`:

- `playwright.config.ts` — Chromium dark + Chromium light projects,
  snapshot template `{snapshotDir}/{testFilePath}/{projectName}/{platform}/{arg}{ext}`.
- `tests/*.spec.ts` — visual / interaction tests against the embedded
  gwt frontend.
- `snapshots/` — committed baseline images.
- `fixtures/` — WebSocket server responses used by individual tests.

Other projects place their UI tests wherever their convention dictates
(`tests/e2e/`, `cypress/e2e/`, `tests/playwright/`, `tests/integration/`,
`Assets/Tests/PlayMode/` for Unity, `test/ui/` for .NET, etc.). The skill
discovers the location through the project's runner-detection signals.

## Application Bring-up

The runner expects the application under test to be reachable at the URL
or path declared by its baseURL environment variable. The skill is
responsible for bringing the application up before invoking the runner.

```bash
# Playwright + WebView example (gwt repo):

# 1. Build (debug mode is fine for verify runs)
cargo build -p gwt --bin gwt

# 2. Start the application server in the background
./target/debug/gwt &
gui_pid=$!

# 3. Capture the published URL line:
#    gwt browser URL: http://127.0.0.1:<port>/
# 4. curl the URL until it returns HTTP 200 (timeout 30s)
# 5. export GWT_PLAYWRIGHT_BASE_URL=<that url>
# 6. Run Playwright
pnpm test:visual

# 7. Stop the application server
kill "$gui_pid"
```

If the baseURL environment variable is already set in the environment
(e.g. by a preceding test harness) the skill must reuse it instead of
spawning a new server, to avoid port collisions.

For other UI runner pairings the same pattern applies:

| Runner | Baseline env var | Application start pattern |
|---|---|---|
| Playwright (web / WebView) | `PLAYWRIGHT_BASE_URL` / project-specific (`GWT_PLAYWRIGHT_BASE_URL` in gwt repo) | Start the application server, wait for HTTP 200 |
| Cypress | `CYPRESS_BASE_URL` | Same as Playwright |
| Selenium WebDriver | `SELENIUM_BASE_URL` / per-test fixture | Same as Playwright; ensure driver process is running |
| WinAppDriver (.NET / WPF / WinForms) | `WINAPPDRIVER_TARGET_APP` | Launch the app exe; WinAppDriver attaches by path |
| Unity Editor headed (PlayMode) | `UNITY_PROJECT_PATH` | Unity Editor launched with `-projectPath` and `-runTests -testPlatform PlayMode` |

## Headless vs Headed

| Invocation | Behavior |
|---|---|
| Default (no `--headed`) | Run headless. Same conditions as CI. |
| `--headed` (passed to `gwt-verify`) | Run headed. The runner must surface the application URL / window so the user can also inspect the running app. Used only when the caller passed `--headed` to `gwt-verify`. |
| Update-baseline invocation (e.g. `playwright test --update-snapshots`, `cypress test --update`) | **Not** part of `gwt-verify`. The caller must run baseline updates explicitly as a separate manual step, with intent, and commit the new baselines for PR review. |

## Snapshot / Diff Policy

- Visual / interaction regression failures are evidence-bundle FAIL, not
  baseline regeneration triggers.
- The skill never passes `--update-snapshots` (or any equivalent
  baseline-rewrite flag) automatically. Baseline updates must be
  performed by a human-driven step with intent, then committed
  separately so the change appears in PR review.
- Per-runner tolerance configuration (`maxDiffPixelRatio: 0.005` in
  `playwright.config.ts`, `cypress-image-snapshot` `failureThreshold`,
  etc.) is the canonical tolerance; the skill does not override it.

## Project Selection (Playwright example)

Both `chromium-dark` and `chromium-light` projects run by default in the
gwt repo. To narrow during quick iteration, the operator may run
`pnpm test:visual --project chromium-dark` manually; the skill itself
always runs both so coverage matches CI.

Other UI runners may expose similar narrowing flags (`cypress
--browser`, `winappdriver` device-specific config). Default to the full
matrix; narrow only when the operator asks explicitly.

## Common Failure Modes (Playwright example, generalizes)

| Symptom | Likely cause | Action |
|---|---|---|
| `Error: connect ECONNREFUSED 127.0.0.1:<port>` | Application server not yet ready | Re-check the HTTP 200 wait step before invoking the runner. |
| `Snapshot doesn't match` | Real UI regression OR legitimate redesign | Treat as FAIL; do not auto-regenerate snapshots. Triage in `gwt-discussion` or escalate to the user. |
| Headed mode shows no window | Running inside a headless host (CI, container) | Detect missing display and downgrade to `Overall: FAIL` with `Headed verification: refused (no display)`. Do not silently fall back to headless. |
| `browserType.launch: Executable doesn't exist` | Browser not installed for current runner version | Trigger the tooling-bootstrap path (e.g. `pnpm exec playwright install --with-deps chromium`). |
| WinAppDriver: `Cannot find a matching application` | The target binary is stale or in a different location | Rebuild and update `WINAPPDRIVER_TARGET_APP` before retrying. |
| Unity PlayMode test: editor freezes | Scene or test code has a synchronous wait | Treat as FAIL; the operator must triage in the Editor, not the skill. |

## Boundary (do not invoke for non-UI surfaces)

UI runners are for browser / WebView / desktop GUI verification. Never
invoke them for:

- Pure backend / library / business-logic changes (use the project's
  unit / integration test runners instead).
- CLI smoke (use the project's CLI runner directly,
  e.g. `cargo run --bin <bin> -- --version`).
- Release scripts (use the project's release-flow / artifact tests).
- Documentation-only changes (markdownlint only).

This boundary is enforced by `gwt-verify` at command-selection time per
`surface-taxonomy.md`. The same restraint applies to all UI runner
families above, not just Playwright.
