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
  (not just a typo fix) - first apply the launch-route authority below, then
  use `gwt-discussion` when that contract requires a ruling

For autonomous investigations, apply the
[autonomous investigation authority](../../gwt-coordination/references/board.md)
before routing a mismatch to `gwt-discussion`.
In-scope root-cause fixes follow that contract: investigate, update plan/tasks,
and continue this workflow without entering a new approval handoff.
Manual or unknown launch routes retain the interactive discussion contract.

Use the current user's language for task summaries, completion reports, task
check updates, and any user-facing text generated while executing the workflow,
unless an existing artifact must keep its established language.

## gwtd resolution

Before executing any `gwtd ...` command from this skill, resolve `GWT_BIN`
first: executable `GWT_BIN_PATH`, then `command -v gwtd`, then
`$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the command
as `"$GWT_BIN" ...`; if none exists, stop with an actionable `gwtd not found`
error.
