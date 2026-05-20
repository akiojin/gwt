# User Verification Guide (4-step 導線 + Check Items)

After automated tests pass, `gwt-verify` hands off to the user for manual
confirmation when `--mode full` or `--mode pre-pr` is active and the
changed surfaces include any `Required` or `Recommended` user-check
entries (per `surface-taxonomy.md`).

The handoff has two halves:

1. **導線** — a fixed 4-step path that walks the user from a clean state
   to the changed feature.
2. **Check Items** — a checklist with three required categories of things
   to verify.

## The 4-step 導線

The 4 steps are **always present and always in this order**, regardless of
project type:

| # | Label    | What it contains | Examples by project type |
|---|----------|------------------|---|
| 1 | **build**    | The project's smallest build command that exercises the change. Skip if the project does not require a build step. | Rust: `cargo build -p <crate>`. Node: `pnpm build` or `pnpm dev` if hot-reload covers the change. Unity: Unity Editor batch build, or "Open Unity Editor and load project" when no headless build exists. .NET: `dotnet build <Solution.sln>`. Python: skip when the script runs directly. Go: `go build ./...`. Mobile: `flutter build apk --debug` / `xcodebuild -scheme <app>`. |
| 2 | **launch**   | How to start the running artifact. | Rust CLI: `./target/debug/<bin>`. Web/WebView: launch the application server and surface the URL line (e.g., `gwt browser URL: http://127.0.0.1:<port>/`); confirm HTTP 200 with `curl -fsS -I <URL>` before sharing. Unity: Press Play in the Editor on `<Scene>.unity`. .NET: `dotnet run --project <Project>` or launch the built exe. Python service: `python -m <module>` or `uvicorn app:main --reload`. Mobile: open simulator and install the build. Long-running daemon: `<bin> --serve` and tail its log. |
| 3 | **navigate** | The user-visible steps from launch to the changed feature. | "Click Logs in the top bar → select Process chip → choose `gh`." "Run `<cli> issue spec list --spec 1935` and read the output." "In Unity Editor, open `Hierarchy → MainCanvas → ReleaseNotesWindow`." "In the running .NET app, open menu `Help → About` then close → reopen." |
| 4 | **observe**  | Exactly what the user should look at, click, or interact with to confirm. | "Verify only `gh`-tagged log lines appear in the table." "Confirm `Closed:` field matches Issue state." "Verify the release-notes window snaps to the top-right and survives a `Ctrl+R` reload." "Confirm the About dialog's version string matches `package.json`." |

Rules:

- Use concrete commands, file paths, and UI affordance names that exist
  in **this** project. Do not invent paths.
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

Format each item as a Markdown checkbox so the user can tick it off:

```markdown
- [ ] Expected: <one-line description of expected behavior>
- [ ] Edge: <one-line description of edge case to confirm>
- [ ] Regression: <one-line description of adjacent feature sanity>
```

You may add more items beyond the three categories, but never fewer.

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

導線 (How to access):
1. build:     cargo build -p gwt --bin gwt
2. launch:    ./target/debug/gwt — note the published `gwt browser URL: http://127.0.0.1:<port>/` and confirm 200 with `curl -fsS -I <URL>`
3. navigate:  Open the URL in a browser → click `Logs` in the top bar → choose `gh` from the Process chip filter
4. observe:   Only `gh`-tagged lines should appear; non-`gh` processes are hidden

Check Items:
- [ ] Expected: `gh`-tagged lines are visible; other process lines are hidden
- [ ] Edge: selecting "All" restores the unfiltered view without a reload
- [ ] Regression: the adjacent "Severity" filter still filters as before
```

### Example B — Unity package, in-game settings menu

```text
User Verification: required
Surfaces requiring user check: UI surface

導線 (How to access):
1. build:     Open Unity Editor on the project (no headless build required)
2. launch:    Press Play on `Assets/Scenes/Main.unity`
3. navigate:  In the running scene, open menu `Pause → Settings → Display`
4. observe:   The new `Render Scale` slider appears under `Quality` and reflects the saved value

Check Items:
- [ ] Expected: slider moves smoothly and updates the runtime render scale
- [ ] Edge: setting the slider to 0.5 and quitting Play mode persists the value across reloads
- [ ] Regression: the existing `Master Volume` slider beside it still functions
```

### Example C — .NET WPF desktop application

```text
User Verification: required
Surfaces requiring user check: UI surface

導線 (How to access):
1. build:     dotnet build src/App/App.csproj
2. launch:    dotnet run --project src/App/App.csproj
3. navigate:  In the running app, click `Help → About`
4. observe:   The About dialog now lists the new `Build SHA` field below `Version`

Check Items:
- [ ] Expected: `Build SHA` shows the current commit short hash
- [ ] Edge: closing and reopening the About dialog keeps the value (no flicker / blank state)
- [ ] Regression: the existing `Check for updates` button still launches the update flow
```
