# Progress: SPEC-2 - Workspace Shell

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `203/203` checked in `tasks.md`
- Artifact refresh: `2026-04-04T09:28:32Z`

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
- Branch Detail `Sessions` now supports row selection and `Enter` handoff into the selected running session, so the branch detail pane can act as a real branch-first launcher again.
- The bottom footer now behaves like an old-TUI status bar again: current session context, branch context, agent type, notifications, and focus-specific hints share the same surface.
- Branch Detail now also restores old-TUI direct branch actions after focus leaves the list: `Shift+Enter` opens a shell, `Ctrl+C` opens delete confirmation, and footer hints explain the active section's semantics.
- Branch Detail bottom-pane chrome now keeps the selected branch name visible next to the section tabs, so context survives after focus moves off the branch list.
- Branch Detail now honors its advertised `Esc:back` affordance: `Esc` returns focus to the Branches list while preserving the selected branch and the current detail context.
- Pane focus chrome now matches the documented old-TUI contract again: focused borders render in Cyan and unfocused borders render in Gray instead of the temporary green/white colors.
- Branch Detail direct shell/delete affordances are now worktree-aware: branches without a worktree no longer advertise or trigger `Shift+Enter` shell launch or `Ctrl+C` delete confirmation.
- The workspace shell now uses a more old-TUI-like `40/60` default split while management is visible, so terminal sessions keep more horizontal space than the management pane.
- Branch Detail now keeps the old-TUI local mnemonics alive after focus leaves the list: `m=view`, `v=Git View`, `f=search`, and `?` / `h=help` all work directly from the detail pane.
- Ctrl+G,g now treats the management panel as a supplemental surface again: showing it keeps terminal focus, and hiding it always normalizes focus back to Terminal so Main layer hints never leak stale management focus.
- Global management-tab shortcuts now follow the same supplemental contract: opening Branches/Issues/Settings/etc. from Terminal surfaces the requested tab without stealing focus, while management-local tab switches still land on the list pane.
- `Issues` and `PR Dashboard` detail panes now honor the documented `Esc` contract as well: `Esc` closes the detail view and returns to the list without changing the selected row.
- The `Logs` detail pane now matches that same contract: `Esc` closes the detail drill-down and returns to the list without disturbing the selected entry.
- Management list/pane focus now has the matching supplemental escape hatch: unclaimed `Esc` returns focus to `Terminal`, while warn notifications still consume `Esc` for dismissal first.
- `Profiles` now follows that same contract in plain list mode: `Esc` returns to the terminal or dismisses a warn notification first, while create/edit/delete flows still keep `Esc=Cancel`.
- The status-bar hints now expose that restored contract as well: Branches list and generic management lists both advertise `Esc:term` instead of hiding the return-to-terminal path.
- Management focus cycling now matches the pane topology again: `Branches` still cycles through `BranchDetail`, while every other management tab stays on the two real surfaces (`Terminal` and `TabContent`).
- The management/session split now responds to terminal width again: wide terminals keep the 40/60 old-TUI balance, while standard widths fall back to 50/50 so management chrome remains readable.
- The redundant standalone management banner is gone; pane titles now carry the management context so the left-side list/detail surfaces reclaim one full row of content.
- Terminal-focused footer hints now advertise the global workspace mnemonics again instead of collapsing to management-only guidance while the terminal remains active.

## Next
- Run the reviewer walkthrough in `quickstart.md` and close the remaining manual acceptance evidence.
