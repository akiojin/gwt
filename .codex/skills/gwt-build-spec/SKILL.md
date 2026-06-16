---
name: gwt-build-spec
description: "Use when implementation should proceed from an approved SPEC or approved standalone task, and the work should run through the build/test/verify loop."
---

# gwt-build-spec

Implement code using strict TDD (Red-Green-Refactor) methodology. Operates in two modes:

- **SPEC mode**: driven by an existing SPEC directory with `tasks.md`, full progress tracking
- **Standalone mode**: user provides the task directly, TDD loop enforced, no SPEC artifacts needed

## Execution ownership

Once started, keep moving until one of these is true:

- the scoped tasks are complete
- a real product or scope decision blocks the next task
- a merge conflict or reviewer request cannot be resolved with high confidence
- required auth or tooling is unavailable
- a spec-implementation mismatch is discovered that changes the design surface
  (not just a typo fix) — use `gwt-discussion` to investigate and discuss

Routine CI failures, update-branch merges, and test-first edits are handled autonomously.

Use the current user's language for task summaries, completion reports, task
check updates, and any user-facing text generated while executing the workflow,
unless an existing artifact must keep its established language.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Exit CLI (Stop-block contract)

SPEC-1935 FR-014r routes Stop through `skill-build-spec-stop-check`, which
reads `.gwt/skill-state/build-spec.json` and blocks Stop while the skill is
active. Register the skill lifecycle with the exit CLI:

- JSON operation `build.start` with `params.spec:<n>` when the skill starts an implementation pass
- JSON operation `build.phase` with `params.spec:<n>` and
  `params.label:"red"|"green"|"refactor"|"verify"|"pr"` at each TDD milestone (logging only)
- JSON operation `build.complete` with `params.spec:<n>` once the Ready PR Gate is satisfied for a releaseable slice and verification passed
- JSON operation `build.abort` with `params.spec:<n>` and `params.reason` when implementation cannot proceed without a product decision or blocking merge conflict

The Stop-block handler honours Claude Code / Codex's built-in
`stop_hook_active` flag, so each Stop cycle allows at most one forced
continuation; a genuinely stuck turn still terminates normally.

## Mode detection

Determine the mode at entry:

- **SPEC mode** if a SPEC ID is provided or discoverable from the current branch/context, AND
  the tasks section can be read with JSON operation `issue.spec.section`
- **Standalone mode** otherwise

## Phase 1: Context Load

### SPEC mode

1. Read all SPEC sections (`spec`, `plan`, `tasks`) with JSON operation `issue.spec.read`.
2. Run the Board active-claim preflight for the target SPEC before choosing a
   task slice:
   - Read the current Board with JSON operation `board.show`.
   - Look for active `claim` entries from another session that mention the same
     owner (`#<N>` or `SPEC-<N>`) or the same phase label (`Phase <N>`,
     `Phase <label>`) as the task slice under consideration.
   - If a matching claim exists, pause before Phase 2 and present the conflict:
     propose joining that session with a Board handoff request, splitting the
     work through `gwt-discussion`, or continuing only after the user explicitly
     accepts duplicate risk.
   - Intentional parallel work is allowed only when write scopes are disjoint.
     Post a fresh Board `claim` that includes a `Boundary:` line naming the
     files/modules owned by this session before continuing.
   - Acceptance scenario: Given another session has an active Board claim for
     `SPEC-2008 Phase 24`, when `gwt-build-spec` starts for SPEC-2008, then the
     preflight reports the claim and requires user confirmation before any RED
     test or production edit is made.
3. Identify the next incomplete task slice in dependency order:
   - Setup before Foundational work
   - Foundational before story-specific work
4. Use JSON operations `issue.spec.section` / `issue.spec.edit` for SPEC section reads and writes.

### Standalone mode

1. Accept the user's task description, requirements, and acceptance criteria.
2. Identify target files and test locations from the task description and codebase exploration.
3. No SPEC artifacts are required or created.

## Phase 2: TDD Loop (Red-Green-Refactor)

This phase is **non-optional** in both modes. See `references/tdd-workflow.md` for detailed methodology.

### Step 1: Red — Write a failing test

- Add or update the narrowest failing test that proves the task.
- Reuse existing test suites when they already cover the target behavior.
- Rust: place tests in `crates/*/tests/` or `#[cfg(test)]` modules.
- Run the test and confirm it **fails** for the expected reason.
- If the test passes immediately, the behavior already exists — skip to the next task.

### Step 2: Green — Implement the minimum

- Write the minimum code to make the failing test pass.
- Edit only the files implied by the task (or `tasks.md` in SPEC mode).
- Do not silently widen scope beyond the current task.
- In SPEC mode, keep changes aligned with `spec.md` and `plan.md`.
- Run the test and confirm it **passes**.

### Step 3: Refactor

- Clean up the implementation while keeping all tests green.
- Apply project conventions: `cargo fmt`, naming, module structure.
- Run the full relevant test suite to catch regressions.

### When to skip tests

Tests may be skipped **only** when the task does not change observable behavior:

- Documentation-only changes (`docs:`)
- Configuration/CI changes (`chore:`)
- Formatting-only changes
- CLAUDE.md / README updates

All other changes require the TDD loop.

### Repeat

- In SPEC mode: continue task-by-task in dependency order until the scoped tasks are done.
- In standalone mode: continue until the user's stated task is complete.

## Phase 3: Verification

Delegate environment-aware verification to `gwt-verify --mode full`. The
sub-skill is a project-agnostic Generic Verification Contract: it classifies
the changed surfaces against `.codex/skills/gwt-verify/references/surface-taxonomy.md`,
detects the host project's actual test runners from manifests
(Cargo.toml / package.json / pyproject.toml / go.mod / ProjectSettings /
*.sln / etc.) per `.codex/skills/gwt-verify/references/runner-detection.md`,
runs the appropriate unit / integration / E2E / visual tests, emits a
**Test Inventory** that names which tests were executed, and hands off to
the user via a 4-step verification path + Check Items before finalizing
`## Verification Report`. The right matrix is determined by what actually
changed and which runners the project ships; do not hard-code a static
cargo / pnpm / Playwright command list here.

The skill is considered green when **all** of the following hold:

- `gwt-verify --mode full` exits with `Overall: PASS`
- the evidence bundle records no `failed: tooling-missing` entry
- no visual / UI snapshot diff is unresolved (visual regression must be
  triaged before declaring PASS, not silently regenerated)
- `User Verification Result` is one of `confirmed`, `n/a`, or
  `skipped(<reason>)`. `pending` is not acceptable; `rejected(<reason>)`
  forces `Overall: FAIL` and the implementation returns to Phase 2 (TDD
  loop). The user's reason is preserved in the evidence bundle.

In SPEC mode, also verify:

- Implementation matches `spec.md` acceptance scenarios
- No scope creep beyond `tasks.md`
- If a failure indicates a spec gap (not a code bug), route back to `gwt-discussion`

## Phase 4: PR Flow

Handle PR operations autonomously using the `gwt-manage-pr` skill:

- If there is no active PR for the branch, or prior PRs are already merged, use `gwt-manage-pr` to create one.
- If the current PR has CI failures, conflicts, or review blockers, use `gwt-manage-pr` to fix.
- Let `gwt-manage-pr` handle routine merge/push/fix loops.

Do not create or update a Ready for review PR until Phase 3 verification
passes and the Ready PR Gate confirms a releaseable slice. If work is
intentionally incomplete or blocked but needs shared CI/early review,
`gwt-manage-pr` may create/update only a Draft PR that lists known
blockers and Remaining acceptance. Draft PR does not satisfy build
completion.

## Phase 5: Completion Gate

### SPEC mode

Reconcile implemented behavior against all SPEC artifacts. See `references/completion-gate.md` for details.

Required checks:

- Every completed task in `tasks.md` matches the implementation
- Acceptance checkboxes in the tasks section reflect behavior that has actually been accepted
- TDD checkboxes in the tasks section reflect real verification evidence
- Completion markers in the tasks section do not claim completion that is not backed by code
- If artifacts disagree, return to `gwt-discussion` — do not proceed to PR
- Ready PR Gate passed for a releaseable slice. An incomplete Draft PR
  handoff must be reported separately and does not satisfy completion.

Update execution tracking:

- Mark completed tasks in `tasks.md`
- Update tasks-section checkboxes with JSON operation `issue.spec.edit`

### Standalone mode

- Confirm all user-stated acceptance criteria are met
- Confirm all tests pass
- Confirm lint and type checks pass
- Summarize what was built and what was verified

### Chain suggestion

On completion, suggest `gwt-arch-review` for code review if available, or proceed to `gwt-manage-pr` if not already done.

Only call JSON operation `build.complete` after the Ready PR Gate is
satisfied for a releaseable slice. A Draft PR handoff must keep the
remaining work visible in the report and must not be represented as a
completed build.

## Stop conditions

Stop only when:

- the next task depends on a product or scope decision that is not inferable
- a merge conflict or review request is ambiguous enough to risk wrong behavior
- required repo/auth/tooling access is unavailable
- `gwt-verify` returns `failed: tooling-missing` and the auto-install path
  cannot recover (see `.codex/skills/gwt-verify/references/tooling-bootstrap.md`)
- implementation reveals a gap between spec.md and the actual behavior
  (acceptance scenario inaccuracy, missing data-model section, undocumented
  registration table, dependency chain not captured in tasks.md, etc.) that
  changes the design surface — not just a typo fix. Route to
  `gwt-discussion` if the gap requires user discussion, or update the
  SPEC artifact directly if the fix is mechanical

## Exit report

```text
## <Build Report in the current user's language>

Mode: SPEC-<id> | Standalone
Completed tasks: <N> | <summary>
Updated files: <paths>
Verification:
- <command/result>

Next:
- `gwt-manage-pr` (create/update PR)
- `gwt-arch-review` (code review)
- return to `gwt-discussion` (artifact repair)
- ask user for decision
```
