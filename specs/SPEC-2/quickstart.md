# Quickstart: SPEC-2 - Workspace Shell

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and enter the main workspace shell.
2. Move across terminal tabs and split panes using the documented keybindings.
3. Open the management panel and verify the visible tabs match the current product shell, including the Git View tab and without assuming a live Specs tab.
4. Press `Ctrl+G,?`, confirm the help overlay lists the current registered shortcuts grouped by category, and dismiss it with `Esc`.
5. Return to Branches with `Ctrl+G,b`, confirm focus lands on the branch list, and verify the footer now advertises `Enter`, `Shift+Enter`, `Space`, `Ctrl+C`, `m`, `v`, `f`, and `?`.
6. On Branches, verify rows render as a flat list with inline worktree and HEAD indicators, then confirm `Enter` opens the wizard, `Shift+Enter` opens a shell for a worktree branch, `Space` moves focus to the detail pane, and `Ctrl+C` opens delete confirmation.
7. While still on Branches, verify `m` cycles the local view mode, `v` jumps directly to Git View, `f` opens search input, and both `?` and `h` open the help overlay.
8. Open Git View, confirm recent commits are listed for a non-empty repository, and press `r` to refresh after making a working-tree change.
9. Toggle split/grid, switch the active management tab, quit, and restart `gwt-tui`; confirm the layout and visible panel state restore from the saved session file.
10. Move into the `Sessions` detail section and confirm active sessions on the selected branch render as a typed list with an active-session marker instead of a count-only placeholder.
11. While still in `Sessions`, use `Up/Down` to change the selected row, press `Enter`, and confirm the chosen session becomes active and terminal focus is restored.
12. Return to Branch Detail `Overview` and confirm `Shift+Enter` opens a shell for the selected branch and `Ctrl+C` opens the delete-worktree confirmation without first moving focus back to the list.
13. Verify the bottom status bar now changes its Branch Detail hints by section: `Overview` advertises direct branch actions and Docker controls, while `Sessions` advertises `↑/↓` row selection plus `Enter:focus`.
14. Confirm the Branch Detail pane title keeps the selected branch name visible next to the section tabs, and that clearing the branch selection falls back to `No branch selected`.
15. With focus still in Branch Detail, press `Esc` and confirm focus returns to the Branches list while the selected branch, detail section, and session-row selection are unchanged.
16. Cycle focus across Tab Content, Branch Detail, and Terminal, and confirm the focused pane border is Cyan while the unfocused pane borders are Gray.
17. Move Branch Detail to a branch without a worktree and confirm the footer no longer advertises `Shift+Enter:shell` or `Ctrl+C:delete`, and that pressing those keys does not open a shell or delete confirmation.
18. Toggle the management panel on a wide terminal and confirm the left pane is visibly narrower than the session pane, with the default layout matching a `40/60` split instead of `50/50`.
19. While focus stays in Branch Detail, press `m` to change Branches view mode, `v` to jump to Git View, and `f` to return to the list with search active; confirm `?` or `h` still opens the help overlay from the detail pane.
20. Verify the management header stays compact: it should show the repository basename plus active tab/focus context instead of printing the full repository path across the narrow pane.
21. Record any remaining gaps against `tasks.md` before claiming the shell complete.

## Expected Result
- The reviewer sees the current implemented scope for workspace shell.
- The help overlay reflects the keybinding registry without orphaned or invented shortcuts.
- Branches behaves as the primary daily-entry tab again: returning there resets list focus, rows are flatter, the legacy primary actions are reachable without opening another view first, and the local mnemonics match the old-TUI muscle memory.
- Branch Detail `Sessions` shows which shell/agent tabs are active on the selected branch instead of only reporting a numeric count.
- Branch Detail `Sessions` also acts as a handoff surface: the user can choose a running session and jump directly into it.
- Branch Detail remains actionable even after focus leaves the list: direct shell launch and delete confirmation stay available from the detail pane itself.
- Branch Detail chrome keeps the selected branch name visible in the bottom-pane title instead of forcing the user to look back to the top list for context.
- Branch Detail behaves like an old-TUI deep-focus surface again: `Esc` returns to the Branches list instead of leaving the user trapped in the detail pane.
- Focus chrome now matches the documented old-TUI palette instead of the temporary green/white implementation.
- Branch Detail only advertises worktree-backed direct actions when the selected branch can actually execute them.
- The visible workspace balance favors the terminal pane again: the management panel uses a sensible default width instead of splitting the screen evenly.
- Branch Detail preserves the old-TUI local mnemonic muscle memory instead of forcing a focus hop back to the list before `m`, `v`, `f`, or help works.
- The management header now stays readable inside the narrower pane because it uses compact repository/context text instead of a full path dump.
- The footer behaves like an old-TUI status bar again: current session context stays visible while the relevant keybind hints remain discoverable.
- Git View reflects repository status and recent commits after refresh.
- Session layout and management panel state survive a restart.
- No implementation task remains unchecked in `tasks.md`.
- No step should be treated as complete unless the code path is actually reachable today.
