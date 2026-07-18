---
name: gwt-verify
description: "Use when implementation needs verification before completion or before opening a PR. Defines a project-agnostic Generic Verification Contract: classify changed surfaces, autodetect the project's test runners from manifests (Cargo.toml / package.json / pyproject.toml / go.mod / ProjectSettings / *.sln / etc.), run the appropriate unit / integration / E2E / visual tests for the project, emit an evidence bundle that lists exactly which tests were executed, then hand off to the user with a 4-step 導線 (build → launch → navigate → observe) and check items before declaring Overall: PASS. Triggers: 'verify', 'run tests', 'pre-PR check', 'gwt-verify'."
---

# gwt-verify

Project-agnostic verification skill. `gwt-verify` is bundled into every gwt-managed
worktree, including Rust, Node, Unity, Visual Studio / .NET, Python, Go, and any other
project type gwt opens. It defines a **Generic Verification Contract** — what to verify,
how to report, and how to involve the user — without hard-coding any particular test
runner or build tool.

This skill owns "verification time" for the active project. `gwt-build-spec` Phase 3
delegates to `gwt-verify --mode full`; `gwt-manage-pr` requires `gwt-verify --mode pre-pr`
before opening or updating a PR; users may also invoke it directly through
`/gwt:gwt-verify`.

## Contract overview

`gwt-verify` is **project-agnostic**. It does not own a fixed cargo / pnpm /
Playwright recipe. Instead it executes the following four-part contract:

1. **Change Classification** — bucket changed files into abstract surfaces
   (UI surface / interactive CLI / business logic / build-release pipeline /
   docs / config / skill-asset / external-binding). See
   `references/surface-taxonomy.md`.
2. **Appropriate Test Selection** — detect the project's actual test runners
   from its manifests (Cargo.toml, package.json, pyproject.toml, go.mod,
   ProjectSettings/ProjectVersion.txt, `*.sln` / `*.csproj`, Gemfile, pom.xml,
   build.gradle, Makefile) and choose the right unit / integration / E2E /
   visual commands for each changed surface. See
   `references/runner-detection.md`.
3. **Test Inventory Emission** — extract the actual test names / scenarios /
   suite paths from each runner's structured output and list them in the
   evidence bundle so the user can see **what was tested**, not just a pass /
   fail count.
4. **User Verification Handoff** — after automated tests pass, present a
   surface-specific checklist plus a 4-step 導線 ("How to access": build →
   launch → navigate → observe) and ask the user to confirm or reject. See
   `references/user-verification-guide.md`.

Project-local rules always win. If the project root has an AGENTS.md / README
section describing its own testing approach (e.g., "run `make verify`",
"use Unity Editor batch mode test runner"), the agent follows that instead of
the generic matrix in this skill. The matrix here is the fallback frame.

## Modes

| Mode | When to use | Scope |
|---|---|---|
| `--mode quick` (default) | TDD loop, narrow check during implementation | Only surfaces touched by uncommitted / working-tree changes; the narrowest representative command per runner (e.g. single-package test invocation). Skip heavy integration / E2E / visual unless the diff explicitly touches them. |
| `--mode full` | gwt-build-spec Phase 3, standalone completion gate | Full matched matrix per changed surface, including integration / E2E / visual when a UI surface is in scope — by the diff, or by acceptance-aware escalation (`references/surface-taxonomy.md`). User Verification Handoff is required (see below). |
| `--mode pre-pr` | gwt-manage-pr before PR create / update | `full` matrix + release-flow tests when a release surface changed; visual / UI regression always included if any UI surface is in scope by diff or by acceptance-aware escalation. User Verification Handoff is required. |
| `--headed` (flag) | Manual UI / design verification | When supplied alongside any mode that runs a browser-based test runner (Playwright / Cypress / Selenium / WinAppDriver / Unity Editor headed), launch the runner in headed mode so the user can watch. Default is headless to match CI. |

Additional flag:

- `--skip-user-check` — explicitly opt out of the User Verification Handoff
  phase for non-interactive runs (CI smoke, automated regression sweeps).
  Default is **off**; `--mode full` and `--mode pre-pr` require user
  verification unless this flag is set. The reason is recorded in the
  evidence bundle as `User Verification: skipped(--skip-user-check)`.

## Invocation Sequence

```text
agent → /gwt:gwt-verify [--mode quick|full|pre-pr] [--headed] [--skip-user-check]
  ↓
gwt-verify
  1. Establish baseline:
       git diff --name-only $(git merge-base HEAD origin/develop)..HEAD
     (fall back to origin/main when origin/develop is absent). For
     `--mode quick` also include `git diff --name-only` (working tree).
  2. Classify each path into an abstract surface using
     `references/surface-taxonomy.md`.
  3. Discover the project's test runners from its manifests using
     `references/runner-detection.md` (Cargo.toml → cargo,
     package.json → npm / pnpm / yarn scripts, pyproject.toml → pytest,
     go.mod → go test, ProjectSettings/ProjectVersion.txt → Unity Editor
     batch test runner, *.sln / *.csproj → dotnet test, Gemfile → rspec
     / minitest, pom.xml / build.gradle → mvn / gradle test,
     Makefile → make test targets). Multiple runners may coexist
     (e.g. Cargo + pnpm).
  4. Select appropriate test commands per changed surface. Record the
     selection rationale in the evidence bundle's `Plan` section. Honor
     project-local AGENTS.md / README testing instructions when present;
     the generic matrix is the fallback.
  5. Tooling presence check: verify each chosen runner / browser / editor
     is installed. Missing entries follow `references/tooling-bootstrap.md`
     (best-effort install attempt for runners with auto-install paths;
     otherwise exit with `failed: tooling-missing`).
  6. Run each selected command sequentially. Capture stdout / stderr /
     exit code. For browser-based UI runners, bring up the application
     server per `references/playwright-runbook.md` before invoking the
     runner. When the runner supports a structured reporter (JSON / TAP /
     list / JUnit XML), use it so test-level results can be extracted.
     In gwt-managed execution worktrees, run the final command matrix
     through JSON operation `verify.run` with `params.commands:[...]`
     (one plain command per entry, no shell operators): gwtd executes the
     commands itself and writes the tool-generated Verification Run Record
     that `execution.complete` and Ready PR handoffs require (SPEC-3248
     P8b). Prose summaries of test runs do not satisfy those gates.
  7. Extract a Test Inventory from each runner's output: test name,
     describe block, scenario / snapshot title, lint rule name, etc.
     If extraction fails for a runner, record
     `(inventory unavailable: <reason>)` rather than silently degrading.
  8. Emit the evidence bundle described below, then run the User
     Verification Handoff phase (unless skipped per the rules above) and
     finalize Overall once the user response is recorded.
```

The skill does not invoke Playwright — or any other UI runner (Cypress /
Selenium / WinAppDriver / Unity Editor headed) — when no user-facing surface
is in scope. A UI surface is in scope either by the diff touching WebView /
browser UI files, or by `references/surface-taxonomy.md`'s acceptance-aware
escalation promoting a backend-only diff whose acceptance manifests in a
user-facing surface. A change with no user-facing surface in scope by diff or
acceptance does not invoke Playwright. This restraint is part of the
gwt-verify verification contract that prevents over-eager UI-runner
invocation; it is not an external SPEC requirement.

## Evidence Bundle

Output to stdout in the following shape (Markdown):

```text
## Verification Report

Mode: <quick|full|pre-pr>
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
Status: required | recommended | skipped(<reason>)
Surfaces requiring user check: <list>

#### 導線 (How to access the changed behavior)
1. build:     <project-specific build command>
2. launch:    <how to start the app / open the editor / run the binary>
3. navigate:  <how to reach the changed feature inside the running app>
4. observe:   <what the user should look at / interact with>

#### Check Items
- [ ] <expected behavior — the representative happy path>
- [ ] <edge case / failure handling — at least one>
- [ ] <adjacent feature regression sanity — at least one>

Expected: <one-line summary of the intended behavior>
Observed: <user response slot>
User Verification Result: pending | confirmed | rejected(<reason>) | n/a

Headed verification: <yes|no>
Tooling installed during run: <list, or "none">

Overall: PASS|FAIL
```

Rules:

- `Overall: PASS` requires **both** every entry in `Executed` reporting `PASS`
  **and** `User Verification Result ∈ {confirmed, n/a, skipped(<reason>)}`.
  `pending` must never resolve to `PASS`.
- If any executed command fails, `Overall: FAIL` and the failing command's
  detail block is captured verbatim.
- If the user rejects, `Overall: FAIL` and the reason is preserved.
- `failed: tooling-missing` remains a hard completion blocker — callers
  (`gwt-build-spec`, `gwt-manage-pr`) treat it as such.

## Surface → User-Check Matrix (abstract)

The matrix below is **abstract** so it applies to any project type. For
project-specific specialization, see `references/surface-taxonomy.md`.

| Abstract Surface | User Check | 導線 generation guidance |
|---|---|---|
| UI surface (visible markup / theme / layout / interactive controls) | **Required** | Launch the application using the project's normal launch path → present the entry point (URL / editor Play mode / GUI binary / TUI screen) → list the click / keystroke / navigation steps to reach the changed feature → describe what the user should look at. |
| Interactive CLI / TUI surface | **Required** | Build the CLI → run a representative subcommand example → compare against expected output / screen state. |
| Build / Release pipeline | **Required** | Generate the release artifact → manually unpack / install / launch it once → verify the artifact behaves as expected. |
| Business logic (no observable surface) | **Recommended** | If there is any observable mouth (CLI, log, UI) where the change manifests, present that; otherwise mark as Optional. |
| External binding / 3rd-party integration | **Recommended** | Show a representative outbound call and expected response shape. |
| Skill asset / agent config | **Recommended** | Describe the trigger phrase or scenario that should activate the modified skill / agent, plus the expected effect. |
| Docs / config-only (markdownlint clean) | **Skipped(docs-only)** | Automated checks are sufficient. |

## User Verification Handoff (post-Executed)

When `Overall` would otherwise be `PASS` (every `Executed` entry passed) and
`--mode full` or `--mode pre-pr` is active and `--skip-user-check` was not
supplied:

1. Compute the `User Verification` block:
   - `Status: required` if any changed surface is marked Required above.
   - `Status: recommended` if only Recommended surfaces changed.
   - `Status: skipped(<reason>)` for `--mode quick`, `Changed surfaces:
     (none)`, docs-only changes, `--skip-user-check`, or any explicit
     skip rule above. The reason must be human-readable and specific.
2. Generate the 4-step 導線 ("How to access the changed behavior") using the
   project's build / launch conventions. The structure is fixed across all
   project types — **always exactly four labelled steps in this order**:
   1. **build** — the project's build command for the smallest target that
      exercises the change.
   2. **launch** — how to start the running artifact (binary / server /
      editor / interpreter / web page).
   3. **navigate** — how to reach the changed feature from the entry point.
   4. **observe** — what the user should look at, click, or interact with to
      confirm the change.
   See `references/user-verification-guide.md` for project-type-specific
   launch patterns (Rust CLI / WebView / Unity Editor / .NET WPF / Python
   service / generic TUI / long-running service).
3. Produce a **Check Items** list with **three categories minimum**:
   (1) expected happy-path behavior, (2) at least one edge case or failure
   path, (3) at least one adjacent-feature regression sanity check.
4. Ask the user via the platform's selection question tool
   (`AskUserQuestionTool` for Claude Code, `request_user_input` for Codex,
   the closest equivalent for other runtimes) to choose one of:
   - `Confirmed` — observed behavior matches expectations; record
     `User Verification Result: confirmed`.
   - `Rejected(<reason>)` — observed behavior does not match; downgrade
     `Overall: FAIL` and preserve the user's reason.
   - `Skip with reason(<reason>)` — user defers verification with an
     explicit justification; record `User Verification Result:
     skipped(<reason>)`.

If no selection UI exists in the current runtime, fall back to plain-text
prompting but keep the same three-option discipline.

## Stop Conditions

Stop and surface a blocker to the caller when:

- `git merge-base HEAD origin/develop` (and `origin/main`) both fail —
  cannot establish a baseline.
- Tooling bootstrap exhausts auto-install options (see
  `references/tooling-bootstrap.md`) and emits `failed: tooling-missing`.
- A required visual / UI snapshot diff is non-zero in `--mode full` /
  `--mode pre-pr` — agent must triage rather than silently regenerate
  snapshots.
- The agent attempts to invoke Playwright (or any other browser UI runner)
  for a surface not classified as a UI surface under
  `references/surface-taxonomy.md` (including its acceptance-aware
  escalation). This is a contract violation, not a runtime error.
- The User Verification phase is required but no platform question tool
  and no plain-text channel are available to ask the user.

## Project-local AGENTS.md takes precedence

This skill is distributed into every gwt-managed worktree. The host project
defines its own testing reality. When the project root holds an AGENTS.md /
CLAUDE.md / README with a "ローカル検証" / "Testing" / "Verification" section,
the agent must read those instructions and prefer them over the generic
matrix in this skill. Use the generic matrix only to fill gaps.

This mirrors the broader gwt rule: gwt's own AGENTS.md is project-local to
the gwt repo, and any other project gwt opens has its own AGENTS.md as the
authority.

## gwtd resolution

Before invoking `gwtd` from this skill or its references, resolve `GWT_BIN`
first: executable `GWT_BIN_PATH`, then `command -v gwtd`, then
`$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. If none
exists, stop with `gwtd not found`.

## References

- `references/surface-taxonomy.md` — abstract surface classification and
  per-project path-pattern examples.
- `references/runner-detection.md` — manifest → test-runner detection
  table (Rust / Node / Python / Go / .NET / Unity / Ruby / Java / Make /
  generic).
- `references/user-verification-guide.md` — 4-step 導線 generation guide
  with project-type-specific launch patterns and Check Items rules.
- `references/test-matrix.md` — example matrix for the gwt repo itself,
  referenced as a worked example of how to specialize the generic
  contract.
- `references/playwright-runbook.md` — operational details for browser /
  WebView UI runners (Playwright is the canonical example; the same
  pattern applies to Cypress / Selenium / WinAppDriver / Unity Editor
  headed).
- `references/tooling-bootstrap.md` — best-effort installer contract and
  the `failed: tooling-missing` shape.

## Chain Suggestion

On `Overall: PASS`, the caller proceeds:

- `gwt-build-spec` Phase 3 → Phase 4 (PR Flow via `gwt-manage-pr`), provided
  `User Verification Result ∈ {confirmed, n/a, skipped(<reason>)}`.
- `gwt-manage-pr` → PR create / update, provided the same User Verification
  Result gate is satisfied.
- Manual invocation → return the evidence bundle to the user.

On `Overall: FAIL` (including `User Verification Result: rejected(...)`),
the caller stays in their current phase and routes the failure for repair
(typically back into the TDD loop or `gwt-discussion`).
