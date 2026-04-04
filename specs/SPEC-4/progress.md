# Progress: SPEC-4 - GitHub Integration

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `36/36` checked in `tasks.md`
- Artifact refresh: `2026-04-03T13:05:00Z`

## Done
- The missing planning support files for this SPEC are now present.
- Git View now has focused verification for file status parsing, diff preview
  truncation, commit loading, and management-tab refresh behavior.
- PR dashboard now has focused verification for parser coverage, screen
  rendering, and snapshot coverage for the list/detail flow.
- PR dashboard now loads live PR list data when the tab gains focus and when
  `r` refresh is triggered, so the screen is no longer a static renderer over
  test-only state.
- PR dashboard detail now loads a live per-PR report when detail view opens and
  refreshes that report together with the list, so CI / merge / review detail
  is no longer a static echo of the list row.
- The remaining gap is completion-gate review plus the explicitly partial
  GraphQL/REST transport note in `spec.md`, not unchecked execution tasks.

## Next
- Run the reviewer flow and capture final evidence for the live GitHub data
  surfaces that are now wired.
- Reconcile acceptance against the current partial implementation before any
  `Done` transition.
