---
name: gwt-execute
description: "Use when execution should start from a GitHub Issue, gwt-spec Issue, or approved standalone task through one build/test/verify loop."
---

# gwt-execute

Unified Execute-lane entrypoint for implementation work. Treat every owner as a
Work Item: a GitHub Issue number plus labels and artifacts. `gwt-spec` is a
design-required tag, not a separate execution kind.

## Execution ownership

Once started, keep moving until one of these is true:

- the scoped tasks are complete
- a real product or scope decision blocks the next task
- a merge conflict or reviewer request cannot be resolved with high confidence
- required auth or tooling is unavailable
- a spec-implementation mismatch is discovered that changes the design surface
  (not just a typo fix) - use `gwt-discussion` to investigate and discuss

Use the current user's language for task summaries, completion reports, task
check updates, and any user-facing text generated while executing the workflow,
unless an existing artifact must keep its established language.

## gwtd resolution

Before executing any `gwtd ...` command from this skill, resolve `GWT_BIN`
first: executable `GWT_BIN_PATH`, then `command -v gwtd`, then
`$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the command
as `"$GWT_BIN" ...`; if none exists, stop with an actionable `gwtd not found`
error.

## Lifecycle

Use the existing build lifecycle JSON operations for every implementation mode.
The operation names and state file remain compatibility surfaces.

- `build.start` with `params.spec:<n>` when the owner is a gwt-spec tagged Issue
  or `params.task:<description>` for standalone work.
- `build.phase` with `params.label:"red"|"green"|"refactor"|"verify"|"pr"` at
  each TDD milestone.
- `build.complete` only after verification passed and the Ready PR Gate is
  satisfied for a releaseable slice.
- `build.abort` with a concrete reason when implementation cannot proceed.

## Mode detection

1. If the invocation includes `#N` or an Issue URL, read the Issue with JSON
   operations `issue.view`, `issue.comments`, and `issue.linked_prs`.
2. If the Issue has a case-insensitive `gwt-spec` label, use design-gated mode.
3. If the Issue has no `gwt-spec` label, use direct mode.
4. If no owner Issue exists, use standalone mode.

## design-gated mode

Use this for gwt-spec tagged Issues.

1. Read SPEC sections with JSON operations `issue.spec.read` or
   `issue.spec.section`.
2. If `plan` or `tasks` is missing or incomplete, stop before production edits
   and route to `gwt-plan-spec`.
3. Run the Board active-claim preflight for the owner before choosing a task
   slice.
4. Select the next incomplete task in dependency order.
5. Execute strict TDD: write the RED test first, confirm it fails for the
   expected reason, implement the minimum GREEN change, then refactor.
6. Update the owner Issue tasks section through `issue.spec.edit` as tasks are
   completed.

## direct mode

Use this for non-gwt-spec Issues.

1. Establish the issue facts from JSON operations `issue.view`,
   `issue.comments`, and `issue.linked_prs`.
2. For bugs, prove the root cause before editing production code. Do not
   guess-fix.
3. Classify `Spec Status`: `ALIGNED`, `IMPLEMENTATION-GAP`, `SPEC-GAP`, or
   `SPEC-AMBIGUOUS`.
4. If the intended behavior needs design ownership first, route to
   `gwt-discussion` or `gwt-register-issue`; otherwise continue in direct mode.
5. Execute strict TDD with the same Red-Green-Refactor loop.
6. After verification and PR handoff, post the durable closure comment with
   root cause, changed files, verification evidence, PR link, and remaining
   work if any.

## standalone mode

Use this when the user provides an approved implementation task without an
owner Issue.

1. Capture the task description, acceptance criteria, and target files.
2. Identify the narrowest test location from the existing codebase.
3. Execute strict TDD with the same Red-Green-Refactor loop.
4. Do not create Issues or SPECs unless the work proves to need durable design
   ownership.

## Verification and PR gate

Phase 3 delegates to `gwt-verify --mode full`. Record the selected commands and
results in the evidence bundle. UI-affecting work requires a concrete user
verification handoff and a `User Verification Result`.

PR work goes through `gwt-manage-pr`. Do not create or update a Ready PR until
pre-PR verification passes and the `User Verification Result` is `confirmed` or
`n/a`.

## Legacy aliases

During the transition, `$gwt-build-spec SPEC-N` and `$gwt-fix-issue #N` are
accepted only as aliases. Continue as `$gwt-execute #N` and use the mode rules
above. Do not split behavior by SPEC versus Issue once the owner number is
known.
