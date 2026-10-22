## Evidence Bundle

Output to stdout in the following shape (Markdown):

```text
## Verification Report

Mode: <quick|full|pre-pr>
Launch mode: autonomous | interactive
Baseline: merge-base HEAD..origin/develop (<N> commits, <M> files)
Changed surfaces: <abstract surface list>
Acceptance Surface: <user-facing surface the change is escalated to, or `non-user-facing(<justification>)`>

### Plan
Detected runners: <e.g. Cargo, pnpm, Playwright | Unity Test Framework | dotnet test | pytest | go test>
Selected commands per surface:
- <surface>: <command> — <selection reason>
Skipped (no matching surface or not applicable):
- <command>: <reason>

### Executed
- <command>: PASS|FAIL (<short metric, e.g. test count + duration>)

### Test Inventory
<command>:
- PASS  <suite> :: <test name / scenario>
- FAIL  <suite> :: <test name>  -> <short failure>
- SKIP  <suite> :: <test name>  -> <reason>
(inventory unavailable: <reason>)  # when extraction failed for a runner

### User Verification
Status: required | recommended | skipped(<reason>) | waived(autonomous launch)
Surfaces requiring user check: <list>

#### Verification Target Card
Owner Issue/SPEC: <Issue #N, SPEC #N, or approved standalone task label>
Work purpose: <short concrete objective that distinguishes this work from other active agents>
Success Goal: <user-visible behavior achieved and the observation that proves it>
Requesting agent/session: <agent provider/name plus GWT_SESSION_ID or equivalent stable session ID>
Branch: <exact branch>
Absolute worktree: <absolute path of the target checkout>
Commit: <full or unambiguous short HEAD SHA>
Prepared instance ID: <runtime-specific PID, window/pane ID, port, or invocation label tied to this checkout>
URL or launch target: <verified URL, GUI/editor target, or exact CLI/TUI invocation>

#### 導線 (How to access the changed behavior)
1. build:     <project-specific build command>
2. launch:    <how to start the app / open the editor / run the binary>
3. navigate:  <how to reach the changed feature inside the running app>
4. observe:   <action to perform and the exact result that proves the Success Goal>

#### Check Items
- [ ] Expected — Action: <concrete click/input/command> → Expected: <decidable result>
- [ ] Edge — Action: <reachable boundary action> → Expected: <decidable handling>
- [ ] Regression — Action: <adjacent feature action> → Expected: <unchanged result>

#### Automated-only Evidence
- <unreachable manual boundary>: Executed item `<exact command>` — PASS (<named test/scenario>)

Expected: <one-line summary of the intended behavior>
Observed: <user response slot>
User Verification Result: pending | confirmed | rejected(<reason>) | skipped(<reason>) | n/a | n/a (autonomous) | deferred (autonomous execution)
Agent Visual Check: pass | fail(<reason>) | n/a (no UI surface)

Headed verification: <yes|no>
Tooling installed during run: <list, or "none">

Overall: PASS|FAIL
```

Rules:

- `Overall: PASS` requires **both** every entry in `Executed` reporting `PASS`
  **and** `User Verification Result ∈ {confirmed, n/a, n/a (autonomous),
  deferred (autonomous execution), skipped(<reason>)}`. `pending` must never
  resolve to `PASS`. The legacy deferred value is accepted for autonomous
  launches under the same fresh evidence and Ready PR Gate requirements.
- When a UI surface is in scope, `Overall: PASS` additionally requires
  `Agent Visual Check: pass`. In `autonomous` mode that line carries the GUI
  quality bar on its own, so a missing or failing Agent Visual Check is
  `Overall: FAIL` even when the launch waives the human handoff. An autonomous
  UI Ready handoff also requires measured passing headed Chromium results for
  dark and light themes in the same fresh `verify.run` record.
- Every acceptance boundary in scope must map to either a reachable manual
  checkbox or an Automated-only Evidence item that names the exact command and
  test. An Automated-only Evidence item must match a `PASS` entry under
  `Executed`; missing, failed, or unlinked coverage makes `Overall: FAIL`.
- If any executed command fails, `Overall: FAIL` and the failing command's
  detail block is captured verbatim.
- If the user rejects, `Overall: FAIL` and the reason is preserved.
- `failed: tooling-missing` remains a hard completion blocker — callers
  (`gwt-build-spec`, `gwt-manage-pr`) treat it as such.

## Surface → User-Check Matrix (abstract)

The matrix below is **abstract** so it applies to any project type. For
project-specific specialization, see [references/surface-taxonomy.md](surface-taxonomy.md).

| Abstract Surface | User Check | 導線 generation guidance |
|---|---|---|
| UI surface (visible markup / theme / layout / interactive controls) | **Required** | Launch the application using the project's normal launch path → present the entry point (URL / editor Play mode / GUI binary / TUI screen) → list the click / keystroke / navigation steps to reach the changed feature → describe what the user should look at. |
| Interactive CLI / TUI surface | **Required** | Build the CLI → run a representative subcommand example → compare against expected output / screen state. |
| Build / Release pipeline | **Required** | Generate the release artifact → manually unpack / install / launch it once → verify the artifact behaves as expected. |
| Business logic (no observable surface) | **Recommended** | If there is any observable mouth (CLI, log, UI) where the change manifests, present that; otherwise mark as Optional. |
| External binding / 3rd-party integration | **Recommended** | Show a representative outbound call and expected response shape. |
| Skill asset / agent config | **Recommended** | Describe the trigger phrase or scenario that should activate the modified skill / agent, plus the expected effect. |
| Docs / config-only (markdownlint clean) | **Skipped(docs-only)** | Automated checks are sufficient. |

The `User Check` column applies to `interactive` launches. In `autonomous`
mode every row collapses to `waived(autonomous launch)`; a **Required** row
means the Agent Visual Check is mandatory instead, not that a human is
summoned.
