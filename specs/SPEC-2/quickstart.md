# Quickstart: SPEC-2 - Workspace Shell

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and enter the main workspace shell.
2. Move across terminal tabs and split panes using the documented keybindings.
3. Open the management panel and verify the visible tabs match the current product shell, including the Git View tab and without assuming a live Specs tab.
4. Press `Ctrl+G,?`, confirm the help overlay lists the current registered shortcuts grouped by category, and dismiss it with `Esc`.
5. Open Git View, confirm recent commits are listed for a non-empty repository, and press `r` to refresh after making a working-tree change.
6. Toggle split/grid, switch the active management tab, quit, and restart `gwt-tui`; confirm the layout and visible panel state restore from the saved session file.
7. Record any remaining gaps against `tasks.md` before claiming the shell complete.

## Expected Result
- The reviewer sees the current implemented scope for workspace shell.
- The help overlay reflects the keybinding registry without orphaned or invented shortcuts.
- Git View reflects repository status and recent commits after refresh.
- Session layout and management panel state survive a restart.
- Any missing behavior is logged against the remaining `1` unchecked task.
- No step should be treated as complete unless the code path is actually reachable today.
