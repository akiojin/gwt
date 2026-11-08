# gwt-discussion

Unified discussion entrypoint for exploration, design clarification, and
mid-implementation investigation. Use it before a SPEC exists, while a SPEC or
plan is being refined, or when implementation reveals a gap that needs user
discussion before work can continue.

Use the current user's language for decision summaries, `Discussion TODO`,
`Action Delta`, `Action Bundle`, and any user-facing artifact text generated
during this workflow, unless an existing artifact must keep its established
language.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## When to use

- A rough idea may or may not become a SPEC
- An existing SPEC or plan feels incomplete or wrong
- Mid-implementation work uncovered a gap, inconsistency, or dependency chain
- The next step is unclear and needs investigation before `gwt-plan-spec` or
  `gwt-build-spec`

Do not use this when the work is already decision-complete and ready for
planning or implementation. Use `gwt-plan-spec` or `gwt-build-spec` directly in
that case.
