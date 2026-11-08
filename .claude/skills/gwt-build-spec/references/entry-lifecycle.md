# gwt-build-spec

## Transition alias

`gwt-build-spec` is a temporary alias for `gwt-execute`. If this skill is
invoked with `SPEC-N`, continue as `$gwt-execute #N` and load
the matching `gwt-execute/SKILL.md` asset from the active provider skill tree.
Keep this file only for transition compatibility during the one-release alias
window.

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

Linked-owner launches additionally carry an Execution Control Record
(SPEC-3248 P8a). A successful `build.complete` settles it; when the execution
must end without completion, run JSON operation `execution.blocked` with a
non-empty `params.reason` (and optional `params.missing_verification`) so the
blocked exit is recorded instead of looping the Stop gate.

The settlement consumes tool-generated verification evidence (SPEC-3248
P8b): run the verification matrix through JSON operation `verify.run` with
`params.commands:[...]` before `build.complete` / `execution.complete`. A
completion without a fresh all-passing record leaves the execution active
and the Stop gate engaged.
