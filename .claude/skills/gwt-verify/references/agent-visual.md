## Agent Visual Check

For a new gwt PR targeting protected `develop`, use the protected WebView CI
path after the headed workflow has landed on the base. Pass the complete
local headless matrix through `verify.run`, leaving `headed_e2e_commands`
unset. Create the Ready PR with these two lines in its body.

- `Headed Evidence: CI`
- `Agent Visual Check: pending (CI headed)`

The required WebView check runs Chromium on Xvfb and uploads
`webview-headed-<run-id>-<attempt>` before it waits for review. Download that
artifact from the PR's Test workflow. Confirm the run belongs to the exact PR
head. Check `report.json` for passing Chromium in both themes and zero failures,
open relevant screenshots in dark and light, and confirm the relevant test
results in the CI logs. Use retained traces to inspect action and console
events. Publish the
result through `pr.comment`. Include the reviewed artifact and image paths,
and put the following acknowledgement on its own line.

`Agent Visual Check: pass (headed CI <run-id>/<attempt>, <PR-head-SHA>)`

The WebView check requires that exact acknowledgement from the PR author
before it passes. Record any defect and repair it before acknowledging a
new head. On a re-run, inspect the new attempt's artifact and record that
attempt. Keep the body selection `Headed Evidence: CI` for subsequent pushes.
If the workflow is unavailable, is changed by the PR, or the target is an
existing Draft or another repository, use explicit local headed evidence.

### Explicit local headed verification

The agent's own headed run is evidence the **agent** produced. It is recorded
on its own line and is **never a User Verification Result** — a self-reported
"looks right" must not be readable as a human having confirmed anything.

Run it whenever a UI surface is in scope (by diff or by acceptance-aware
escalation) and the project ships a headed runner, in every launch mode:

- a real browser / WebView instance, not headless-only, not a DOM shim
- both `dark` and `light` themes
- zero console errors and zero page errors
- the concrete changed behavior actually exercised, not just page load

Record the result as `Agent Visual Check: pass | fail(<reason>) | n/a (no UI
surface)`, and list the executed headed command under `Executed` so the claim
is backed by a `PASS` entry. In every launch mode this is the GUI quality bar,
so `Agent Visual Check: fail(<reason>)` — or a
UI surface in scope with no Agent Visual Check at all — makes `Overall: FAIL`.

For a UI Ready handoff, a prose `Agent Visual Check: pass` alone
is insufficient. Pass `params.headed_e2e_commands` to `verify.run`, selecting
exact strings already present in `params.commands`. Use Playwright commands
or project scripts that forward extra Playwright arguments. gwtd adds
`--headed`, `--trace=on`, and its embedded reporter, then reads the actual
browser launch and context settings from Playwright traces to count passing
Chromium tests for dark and light themes. Both themes must have passing
executed tests in the same fresh verification record; skipped tests, metadata
claims, or a headless run cannot satisfy this evidence requirement. The E2E
suite remains responsible for console/page-error checks and behavior assertions.

On Windows, canonical `verify.run` normalizes `pwsh` / `powershell -File`
commands to a quoted script invocation through `-EncodedCommand` and sets
PowerShell input/output encodings to UTF-8. This preserves drive colons,
spaces, `=`, and non-ASCII paths, including Japanese user profiles. Keep the
reporter argument in its canonical `--reporter=list,<path>` format. A project
PowerShell script only needs to forward remaining arguments with `@args` (or
its declared remaining-arguments array) to Playwright; do not add per-Issue
reporter token re-joining or encoding workarounds.

If a reporter argument is split and Playwright fails before tests start, such
as `Cannot find module 'C'`, treat the verification record as a wrapper defect,
not a failing application test. Preserve the failed record, repair the
canonical wrapper, and rerun `verify.run` for fresh headed evidence. A local
argument-repair wrapper does not establish that the canonical defect is fixed.
