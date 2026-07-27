# User Verification Guide (Target Card + 4-step 導線 + Actionable Checks)

After automated tests pass, `gwt-verify` hands off to the user for manual
confirmation when `--mode full` or `--mode pre-pr` is active and the
changed surfaces include any `Required` or `Recommended` user-check
entries (per `surface-taxonomy.md`). This includes a backend-only diff that
`surface-taxonomy.md`'s acceptance-aware escalation promotes to a
user-facing surface because its acceptance manifests in the UI / CLI: the
handoff fires for the escalated surface even though no UI / CLI file
changed.

The handoff has four parts, always in this order:

1. **Verification Target Card** — the exact owner, purpose, code, and prepared
   instance the user must inspect.
2. **導線** — a fixed 4-step path that walks the user from a clean state
   to the changed feature.
3. **Check Items** — three required categories, each written as an executable
   Action → Expected pair.
4. **Automated-only Evidence** — named tests for boundaries that the prepared
   target cannot safely expose to the user.

## Verification Target Card

Print this card before build or navigation instructions:

```text
Owner Issue/SPEC: <Issue #N, SPEC #N, or approved standalone task label>
Work purpose: <short concrete objective that distinguishes this work from other active agents>
Success Goal: <behavior achieved and the observation that proves it>
Requesting agent/session: <agent provider/name plus GWT_SESSION_ID or equivalent stable session ID>
Branch: <exact branch>
Absolute worktree: <absolute path of the target checkout>
Commit: <full or unambiguous short HEAD SHA>
Prepared instance ID: <runtime-specific PID, window/pane ID, port, or invocation label tied to this checkout>
URL or launch target: <verified URL, GUI/editor target, or exact CLI/TUI invocation>
```

Rules:

- Resolve every value from the target checkout and prepared process. Do not
  copy identity from another running agent or production instance.
- The **Success Goal** must explain what changed and what observable result
  counts as success without requiring the user to read prior conversation.
- The **Work purpose** and **Requesting agent/session** must distinguish the
  request when concurrent agents share an Issue, branch, worktree, or commit.
- The **Prepared instance ID** must identify the exact runtime using a PID,
  window or pane ID, port, or stable invocation label and state that it was
  launched from the Absolute worktree.
- The full card is the identity. A bare URL, PID, terminal title, or phrase
  such as "the current app" is not sufficient on its own.

## The 4-step 導線

The 4 steps are **always present and always in this order**, regardless of
project type:

| # | Label    | What it contains | Examples by project type |
|---|----------|------------------|---|
| 1 | **build**    | The project's smallest build command that exercises the change. For a prepared target, state the exact command the agent already completed and its result. Skip if the project does not require a build step. | Rust: `cargo build -p <crate>`. Node: `pnpm build` or `pnpm dev` if hot-reload covers the change. Unity: Unity Editor batch build, or "agent opened the exact Unity project" when no headless build exists. .NET: `dotnet build <Solution.sln>`. Python: skip when the script runs directly. Go: `go build ./...`. Mobile: `flutter build apk --debug` / `xcodebuild -scheme <app>`. |
| 2 | **launch**   | How to focus and use the exact Prepared instance ID. Do not tell the user to start a second process when the agent has already prepared one. | Rust CLI: name the exact prepared invocation. Web/WebView: state that the agent launched the server, include its exact URL, and confirm HTTP 200 with `curl -fsS -I <URL>` before sharing. Unity: press Play in the identified Editor window. .NET: bring the identified built exe window to the front. Mobile: use the identified simulator and installed build. Long-running daemon: identify its PID/session and log. |
| 3 | **navigate** | The user-visible steps from launch to the changed feature. | "Click Logs in the top bar → select Process chip → choose `gh`." "Run `<cli> issue spec list --spec 1935` and read the output." "In Unity Editor, open `Hierarchy → MainCanvas → ReleaseNotesWindow`." "In the running .NET app, open menu `Help → About` then close → reopen." |
| 4 | **observe**  | Exactly what the user should look at, click, or interact with to confirm. | "Verify only `gh`-tagged log lines appear in the table." "Confirm `Closed:` field matches Issue state." "Verify the release-notes window snaps to the top-right and survives a `Ctrl+R` reload." "Confirm the About dialog's version string matches `package.json`." |

Rules:

- Use concrete commands, file paths, and UI affordance names that exist
  in **this** project. Do not invent paths.
- Keep build, launch, navigate, and observe bound to the branch, worktree,
  commit, and Prepared instance ID in the Verification Target Card.
- Always tell the user what HTTP / WS URL or local port to open when the
  app is a server; confirm reachability (`curl -fsS -I`) before sharing.
- Each step is one bullet or short sentence. If a step needs more than 2
  sentences, the navigation step list is too long — split into
  sub-bullets but keep the four parent labels.
- When the project's AGENTS.md / README describes a project-specific
  launch ritual (e.g., the gwt repo's `gwt browser URL` line at
  `AGENTS.md` L187), reuse that ritual verbatim.

## Check Items (three required categories)

Every User Verification handoff must include **at least one** check item
in each of these three categories:

1. **Expected — the representative happy path.** What the change is
   supposed to do, stated as the user would notice it.
2. **Edge case / failure handling.** What the user should look at to
   confirm a boundary, empty input, error response, or unusual size /
   environment is handled correctly. Pick the most plausible failure
   mode for this change.
3. **Adjacent feature regression sanity.** A nearby feature the user
   should briefly try to confirm nothing else broke. For UI changes,
   this is usually a sibling screen or widget; for CLI, an adjacent
   subcommand; for release pipeline, the previously-released artifact's
   smoke test.

Format each item as a Markdown checkbox with one concrete action and one
decidable expected result:

```markdown
- [ ] Expected — Action: <click/input/command> → Expected: <decidable result>
- [ ] Edge — Action: <reachable boundary action> → Expected: <decidable handling>
- [ ] Regression — Action: <adjacent feature action> → Expected: <unchanged result>
```

You may add more items beyond the three categories, but never fewer.

## Manual Feasibility Gate

Before showing a checkbox, prove that its Action is reachable in the Prepared
instance named by the Target Card.

- Keep the checkbox only when the user can perform the Action without changing
  branches, finding another agent, inventing credentials, mutating unrelated
  external state, or creating a destructive failure.
- If the exact failure path is not reachable, use the nearest safe reachable
  boundary for the Edge checkbox and move the unavailable condition to
  **Automated-only Evidence**.
- Automated-only Evidence must name the command or test and its result; vague
  statements such as "covered by tests" are insufficient.
- Every scoped acceptance boundary must map to either one reachable manual
  checkbox or one Automated-only Evidence item. Stop with `Overall: FAIL` if a
  boundary has neither form of evidence.
- Each Automated-only Evidence item must match a `PASS` entry under `Executed`
  using the same exact command and a named test or scenario. A missing, failed,
  or differently named entry is not valid evidence.

```markdown
#### Automated-only Evidence
- Remote merge-query failure: Executed item `cargo test -p example merge_query_failure` — PASS (`merge_query_failure`)
```

Never ask the user to confirm a state that the prepared target cannot produce.

## Selection question

Ask the user via the platform's selection question tool
(`AskUserQuestionTool` for Claude Code, `request_user_input` for Codex,
the closest equivalent for other runtimes) with these three options:

| Label | Effect on `User Verification Result` |
|---|---|
| `Confirmed` | `confirmed` — `Overall: PASS` (provided automated tests also passed) |
| `Rejected(<reason>)` | `rejected(<reason>)` — `Overall: FAIL`; caller routes back to TDD loop or `gwt-discussion` |
| `Skip with reason(<reason>)` | `skipped(<reason>)` — `Overall: PASS` is allowed but the skip reason is preserved in the evidence bundle for traceability |

When no selection UI is available, ask the same three options in plain
text and parse the user's free-form reply into one of the three states.

## Rejection escalation

When the user selects `Rejected`:

1. Preserve the user's free-text reason in the evidence bundle's
   `User Verification Result: rejected(<reason>)` line.
2. The caller (`gwt-build-spec` Phase 3 / `gwt-manage-pr` Pre-PR) treats
   this as `Overall: FAIL` and does not advance.
3. If the rejection points at a spec / design gap rather than an
   implementation bug, route to `gwt-discussion` to renegotiate scope.
   Otherwise return to the TDD Red → Green → Refactor loop.

## Skip rules

The handoff is **automatically skipped** (no user prompt) when:

- `--mode quick` is in effect (TDD mid-iteration).
- `Changed surfaces: (none)` — nothing to verify.
- All changed surfaces are `docs-only` per `surface-taxonomy.md`.
- The caller passed `--skip-user-check` for an explicit non-interactive
  run.

In every skip case, the reason is recorded as
`User Verification: skipped(<reason>)` so reviewers can audit why no user
confirmation was requested.

## Worked examples

### Example A — gwt repo, Logs window filter changes

```text
User Verification: required
Surfaces requiring user check: UI surface

Verification Target Card:
Owner Issue/SPEC: Issue #1234 / SPEC #1935
Work purpose: Verify the Logs process filter without opening another agent's gwt instance
Success Goal: Process filtering shows only the selected process and returns to the complete list without reload
Requesting agent/session: Codex / GWT session gwt-session-1234
Branch: work/issue-1234
Absolute worktree: /work/gwt/issue-1234
Commit: abc1234
Prepared instance ID: gwt PID 48152 / browser port 61234 / Logs filter, launched from /work/gwt/issue-1234
URL or launch target: http://127.0.0.1:61234/ (HTTP 200 confirmed)

導線 (How to access):
1. build:     Agent completed `cargo build -p gwt --bin gwt` for commit `abc1234` — PASS
2. launch:    Agent launched Prepared instance ID `gwt PID 48152 / browser port 61234`; do not start another process
3. navigate:  Open exactly `http://127.0.0.1:61234/` → click `Logs` in the top bar → choose `gh` from the Process chip filter
4. observe:   Select `gh`, then inspect the visible process tags; every row should be tagged `gh`

Check Items:
- [ ] Expected — Action: select `gh` in the Process filter → Expected: only `gh`-tagged rows remain
- [ ] Edge — Action: select `All` after filtering by `gh` → Expected: all process rows return without a reload
- [ ] Regression — Action: choose `Error` in the adjacent Severity filter → Expected: only error rows remain

Automated-only Evidence:
- None: every scoped check above is reachable in the prepared instance
```

### Example B — Unity package, in-game settings menu

```text
User Verification: required
Surfaces requiring user check: UI surface

Verification Target Card:
Owner Issue/SPEC: Issue #42 / SPEC #42
Work purpose: Verify the Render Scale control in the prepared Unity Editor session
Success Goal: Render Scale is editable, applied at runtime, and retained across Play mode reload
Requesting agent/session: Claude Code / GWT session gwt-session-42
Branch: work/issue-42
Absolute worktree: /work/unity-package/issue-42
Commit: def5678
Prepared instance ID: Unity Editor PID 7351 / Main project window, opened from /work/unity-package/issue-42
URL or launch target: Unity Editor → Assets/Scenes/Main.unity

導線 (How to access):
1. build:     Agent opened the exact project in Unity Editor PID 7351 (no headless build required)
2. launch:    In that exact Editor window, open `Assets/Scenes/Main.unity` and press Play
3. navigate:  In the running scene, open menu `Pause → Settings → Display`
4. observe:   Move `Render Scale` and read the displayed value; it should match the selected setting

Check Items:
- [ ] Expected — Action: move `Render Scale` from 1.0 to 0.75 → Expected: the displayed value and runtime scale become 0.75
- [ ] Edge — Action: set 0.5, exit Play mode, and press Play again → Expected: `Render Scale` reopens at 0.5
- [ ] Regression — Action: move the adjacent `Master Volume` slider → Expected: its displayed value still changes normally

Automated-only Evidence:
- None: every scoped check above is reachable in the prepared editor
```

### Example C — .NET WPF desktop application

```text
User Verification: required
Surfaces requiring user check: UI surface

Verification Target Card:
Owner Issue/SPEC: Issue #77 / SPEC #77
Work purpose: Verify the About build SHA in the prepared WPF process
Success Goal: About shows the current build SHA consistently without breaking update navigation
Requesting agent/session: Codex / GWT session gwt-session-77
Branch: work/issue-77
Absolute worktree: C:\work\app\issue-77
Commit: fedcba9
Prepared instance ID: App.exe PID 6840 / window title `App — issue-77`, launched from C:\work\app\issue-77
URL or launch target: src/App/bin/Debug/net8.0/App.exe

導線 (How to access):
1. build:     Agent completed `dotnet build src/App/App.csproj` for commit `fedcba9` — PASS
2. launch:    Bring App.exe PID 6840 / window `App — issue-77` to the front; do not run another copy
3. navigate:  In the running app, click `Help → About`
4. observe:   Read `Build SHA` below `Version`; it should match `fedcba9`

Check Items:
- [ ] Expected — Action: open `Help → About` → Expected: `Build SHA` displays `fedcba9`
- [ ] Edge — Action: close and reopen About → Expected: `Build SHA` remains `fedcba9` without a blank state
- [ ] Regression — Action: click `Check for updates` → Expected: the existing update flow opens

Automated-only Evidence:
- None: every scoped check above is reachable in the prepared application
```

### Example D — unreachable failure moved to automated-only evidence

The prepared UI cannot safely manufacture a remote merge-query failure. The
boundary therefore has no manual checkbox and is linked to an actual passed
item from the same evidence bundle:

```text
Executed:
- `cargo test -p gwt --lib cli::daemon::server::tests::daemon_scan_records_merge_reconciliation_error_and_preserves_active_slot -- --exact`: PASS (1 test)

Automated-only Evidence:
- Merge-query failure injection: Executed item `cargo test -p gwt --lib cli::daemon::server::tests::daemon_scan_records_merge_reconciliation_error_and_preserves_active_slot -- --exact` — PASS (`daemon_scan_records_merge_reconciliation_error_and_preserves_active_slot`)
```
