# Progress: SPEC-4 - GitHub Integration

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `40/40` checked in `tasks.md`
- Artifact refresh: `2026-04-04T07:05:00Z`

## Done
- The missing planning support files for this SPEC are now present.
- Git View now has focused verification for file status parsing, diff preview
  truncation, commit loading, management-tab refresh behavior, and live
  divergence / current-PR metadata wiring.
- PR dashboard now has focused verification for parser coverage, screen
  rendering, and snapshot coverage for the list/detail flow.
- PR dashboard now loads live PR list data when the tab gains focus and when
  `r` refresh is triggered, so the screen is no longer a static renderer over
  test-only state.
- PR dashboard detail now loads a live per-PR report when detail view opens and
  refreshes that report together with the list, and now also re-loads detail
  when tab focus returns or selection changes while detail view stays open.
- The remaining gap is completion-gate review plus the explicitly partial
  GitHub transport note in `spec.md`, not unchecked execution tasks.

## Next
- Run the reviewer flow and capture final evidence for the live GitHub data
  surfaces that are now wired.
- Reconcile acceptance against the remaining GraphQL/REST transport gap before
  any `Done` transition.
