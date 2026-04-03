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
10. Record any remaining gaps against `tasks.md` before claiming the shell complete.

## Expected Result
- The reviewer sees the current implemented scope for workspace shell.
- The help overlay reflects the keybinding registry without orphaned or invented shortcuts.
- Branches behaves as the primary daily-entry tab again: returning there resets list focus, rows are flatter, the legacy primary actions are reachable without opening another view first, and the local mnemonics match the old-TUI muscle memory.
- Git View reflects repository status and recent commits after refresh.
- Session layout and management panel state survive a restart.
- No implementation task remains unchecked in `tasks.md`.
- No step should be treated as complete unless the code path is actually reachable today.
