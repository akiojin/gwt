# Progress: SPEC-2 - Workspace Shell

## Progress
- Status: `done`
- Phase: `Done`
- Task progress: `81/81` checked in `tasks.md`
- Artifact refresh: `2026-04-03T15:20:00Z`

## Done
- Supporting artifacts were refreshed so they no longer describe the older shell shape.
- Current documentation now treats removed or orphaned routes as implementation drift, not as accepted behavior.
- Execution tracking stays tied to the real `tasks.md` progress instead of legacy planning assumptions.
- Ctrl+G,? now opens a grouped help overlay sourced from the keybinding registry, and Esc closes it without leaking input to underlying panes.
- Git View now loads repository status and recent commits through `load_initial_data()`, and `r` refresh reloads that data from the current repo.
- Session layout now persists `display_mode`, `panel_visible`, and `active_management_tab`, restores on startup, and auto-creates the state directory on save.

## Next
- Run the reviewer walkthrough in `quickstart.md` if manual release evidence is still required.
