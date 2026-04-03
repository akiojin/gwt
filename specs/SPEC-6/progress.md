# Progress: SPEC-6 - Notification and Error Bus

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `27/44` checked in `tasks.md`
- Artifact refresh: `2026-04-03T02:45:09Z`

## Done
- Severity routing is now wired through structured notifications: debug logs only, info/warn feed the status bar, and errors keep their detail in the modal queue.
- The error overlay now renders `Notification` objects directly, including `source` and `detail`.
- Focused verification now exists for notification routing, error rendering, bus draining, and logs filtering.
- Warn notifications can now be dismissed with `Esc` when no overlay, search field, or settings edit session has already claimed that key.
- The Logs tab now exposes `f` to cycle severity filters and `d` to toggle the debug filter state.
- Snapshot coverage now includes a Logs tab render with an active filter state.
- The Logs list now renders stable timestamp / severity / source / message columns instead of a loose inline row.

## Next
- Add the remaining richer log affordances and end-to-end coverage.
- Close the remaining end-to-end and performance verification tasks.
- Keep the SPEC artifacts aligned as the remaining unchecked tasks are implemented.
