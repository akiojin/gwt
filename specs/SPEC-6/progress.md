# Progress: SPEC-6 - Notification and Error Bus

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `44/44` checked in `tasks.md`
- Artifact refresh: `2026-04-08T17:24:59Z`

## Done
- Severity routing is now wired through structured notifications: debug logs only, info/warn feed the status bar, and errors keep their detail in the modal queue.
- The error overlay now renders `Notification` objects directly, including `source` and `detail`.
- Focused verification now exists for notification routing, error rendering, bus draining, logs filtering, and all-severity structured-log coverage.
- Warn notifications can now be dismissed with `Esc` when no overlay, search field, or settings edit session has already claimed that key.
- The notification primitives now live in `gwt-notification`, including configurable bus capacity and configurable structured-log capacity.
- The Logs tab now exposes `f` to cycle severity filters and `d` to toggle the debug filter state.
- Snapshot coverage now includes a Logs tab render with an active filter state.
- The Logs list now renders stable timestamp / severity / source / message columns instead of a loose inline row.
- A focused regression test now proves the error modal queue preserves order across dismiss transitions.
- A focused E2E test now proves Info notifications render in the status bar and auto-dismiss after the 5-second timeout.
- A focused E2E test now proves dismissing one error keeps the next error visible in the modal queue.
- A burst-load E2E test now proves 100 queued errors do not break rendering or queue progression.
- Log file naming, watcher rotation, and housekeeping now all follow the UTC-day contract used by `tracing_appender 0.2.4`, eliminating the previous local-date mismatch near midnight.

## Next
- Run the reviewer flow and close the remaining acceptance gate items.
