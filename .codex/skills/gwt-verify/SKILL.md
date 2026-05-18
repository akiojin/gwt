---
name: gwt-verify
description: "Use when implementation work needs verification before completion or before opening a PR. Selects the correct test matrix per changed surface (cargo for Rust crates, pnpm for frontend JS, Playwright only for WebView/browser UI, release scripts only for release-system changes) and runs them with evidence-bundle output. Triggers: 'verify', 'run tests', 'pre-PR check', 'gwt-verify'."
---

# gwt-verify

Environment-aware verification skill. Inspects the changed surfaces in the
current worktree, selects the correct test matrix per surface, runs the
selected commands, and returns an evidence bundle.

This skill owns "verification time" for the gwt project. `gwt-build-spec`
Phase 3 delegates to `gwt-verify --mode full`; `gwt-manage-pr` requires
`gwt-verify --mode pre-pr` before opening or updating a PR; users may also
invoke it directly through `/gwt:gwt-verify`.

Playwright is invoked **only** for WebView/browser UI surfaces. Non-browser
surfaces (Rust crates, the gwtd CLI, backend domain code, and release
scripts) do not invoke Playwright — they each have their own dedicated
execution path documented in `references/test-matrix.md`.

## Modes

| Mode | When to use | Scope |
|---|---|---|
| `--mode quick` (default) | TDD loop, narrow check during implementation | Only the surface(s) touched by uncommitted/working-tree changes; minimum useful subset (e.g. single-crate `cargo test`, single-suite `pnpm test:frontend-unit`). |
| `--mode full` | gwt-build-spec Phase 3, standalone completion gate | Full matrix per changed surface (cargo test/clippy/fmt, all relevant `pnpm test:frontend-*`, `pnpm test:visual` headless when UI surface changed, skill lint when skill assets changed). |
| `--mode pre-pr` | gwt-manage-pr before PR create/update | `full` matrix + release-flow tests when release surface changed; visual regression always included for any UI surface. |
| `--headed` (flag) | Manual UI/design verification | When supplied alongside any mode that runs Playwright, Chromium starts headed. Default is headless to match CI. |

## Invocation Sequence

```text
agent → /gwt:gwt-verify [--mode quick|full|pre-pr] [--headed]
  ↓
gwt-verify
  1. Collect changed files via
       git diff --name-only $(git merge-base HEAD origin/develop)..HEAD
     (fall back to origin/main when origin/develop is absent). For `--mode
     quick` also include `git diff --name-only` (working tree).
  2. Classify each path into a surface category using
     references/test-matrix.md.
  3. Apply mode-specific filtering (quick → minimum subset; full → all matched
     commands; pre-pr → full + release-flow + always include visual when any
     UI surface changed).
  4. Tooling presence check: cargo, pnpm, node_modules, Playwright Chromium.
     Missing entries follow references/tooling-bootstrap.md (auto-install
     attempt, otherwise exit with `failed: tooling-missing`).
  5. Run each selected command sequentially. Capture stdout/stderr and exit
     code. For Playwright commands also start the GUI server per
     references/playwright-runbook.md and inject `GWT_PLAYWRIGHT_BASE_URL`.
  6. Emit the evidence bundle described below.
```

The skill does **not** invoke Playwright when no WebView/browser UI surface
changed. This is a hard contract from SPEC-1935 FR-124.

## Evidence Bundle

Output to stdout in the following shape (Markdown):

```text
## Verification Report
Mode: <quick|full|pre-pr>
Baseline: merge-base HEAD..origin/develop (<N> commits, <M> files)
Changed surfaces: <comma-separated list>

Executed:
- <command>: PASS|FAIL (<short detail, e.g., test count + duration>)

Skipped / not-applicable:
- <command>: <reason>

Headed verification: <yes|no>
Tooling installed during run: <list, or "none">
Overall: PASS|FAIL
```

If any executed command fails, `Overall: FAIL` and the failing command's
detail block is captured verbatim. `gwt-build-spec` treats `FAIL` and any
`failed: tooling-missing` entry as a hard completion blocker.

## Stop Conditions

Stop and surface a blocker to the caller when:

- `git merge-base HEAD origin/develop` (and `origin/main`) both fail —
  cannot establish a baseline.
- Tooling bootstrap exhausts auto-install options (see
  `references/tooling-bootstrap.md`) and emits `failed: tooling-missing`.
- A required Playwright snapshot diff is non-zero in `--mode full` /
  `--mode pre-pr` — agent must triage rather than silently regenerate
  snapshots.
- The agent attempts to invoke Playwright for a surface not listed under the
  "Browser (Playwright)" column of `references/test-matrix.md`. This is a
  contract violation, not a runtime error.

## gwtd resolution

Before invoking `gwtd` from this skill or its references, resolve `GWT_BIN`
first: executable `GWT_BIN_PATH`, then `command -v gwtd`, then
`$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. If none
exists, stop with `gwtd not found`.

## References

- `references/test-matrix.md` — canonical surface→execution-system table.
- `references/playwright-runbook.md` — Chromium project selection, GUI
  server bring-up, baseURL handoff, headed/headless toggling, snapshot
  baseline policy.
- `references/tooling-bootstrap.md` — pnpm / node_modules / Playwright
  Chromium auto-install contract and `failed: tooling-missing` shape.

## Chain Suggestion

On `Overall: PASS`, the caller proceeds:

- `gwt-build-spec` Phase 3 → Phase 4 (PR Flow via `gwt-manage-pr`).
- `gwt-manage-pr` → PR create/update.
- Manual invocation → return evidence bundle to the user.

On `Overall: FAIL`, the caller stays in their current phase and routes the
failure for repair (typically back into the TDD loop or `gwt-discussion`).
