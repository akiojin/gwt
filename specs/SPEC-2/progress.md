# Progress: SPEC-2 - Workspace Shell

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `96/96` checked in `tasks.md`
- Artifact refresh: `2026-04-04T07:18:34Z`

## Done
- Supporting artifacts were refreshed so they no longer describe the older shell shape.
- Current documentation now treats removed or orphaned routes as implementation drift, not as accepted behavior.
- Execution tracking stays tied to the real `tasks.md` progress instead of legacy planning assumptions.
- Ctrl+G,? now opens a grouped help overlay sourced from the keybinding registry, and Esc closes it without leaking input to underlying panes.
- Git View now loads repository status and recent commits through `load_initial_data()`, and `r` refresh reloads that data from the current repo.
- Session layout now persists `display_mode`, `panel_visible`, and `active_management_tab`, restores on startup, and auto-creates the state directory on save.
- Branch list rows now render as a flat old-TUI style list with inline worktree and HEAD indicators instead of category headers.
- Branches tab now restores the primary old-TUI actions: `Enter` opens the wizard, `Shift+Enter` opens a shell, `Space` moves into detail focus, and `Ctrl+C` opens worktree delete confirmation.
- Switching back to Branches via `Ctrl+G,b` now lands in list focus instead of keeping stale terminal/detail focus.
- Branches now also restores the old-TUI local mnemonics: `m=view`, `v=Git View`, `f=search`, and `?/h=help`.
- Branch Detail `Sessions` now renders branch-scoped shell/agent rows with an active-session marker and optional model/reasoning metadata instead of a count-only placeholder.

## Next
- Run the reviewer walkthrough in `quickstart.md` and close the remaining manual acceptance evidence.
