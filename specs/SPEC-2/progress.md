# Progress: SPEC-2 - Workspace Shell

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `37/55` checked in `tasks.md`
- Artifact refresh: `2026-04-03T11:00:00Z`

## Done
- Supporting artifacts were refreshed so they no longer describe the older shell shape.
- Current documentation now treats removed or orphaned routes as implementation drift, not as accepted behavior.
- Execution tracking stays tied to the real `tasks.md` progress instead of legacy planning assumptions.
- Git View now loads repository status and recent commits through `load_initial_data()`, and `r` refresh reloads that data from the current repo.

## Next
- Close the remaining help-overlay and layout-persistence tasks.
- Keep shell docs synchronized with the live eight-tab management shell after each UI change.
- Re-run focused shell verification once the remaining tasks land.
