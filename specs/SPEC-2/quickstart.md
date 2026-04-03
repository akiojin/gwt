# Quickstart: SPEC-2 - Workspace Shell

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and enter the main workspace shell.
2. Move across terminal tabs and split panes using the documented keybindings.
3. Open the management panel and verify the visible tabs match the current product shell, including the Git View tab and without assuming a live Specs tab.
4. Open Git View, confirm recent commits are listed for a non-empty repository, and press `r` to refresh after making a working-tree change.
5. Record any remaining persistence or help-overlay gaps against `tasks.md` before claiming the shell complete.

## Expected Result
- The reviewer sees the current implemented scope for workspace shell.
- Git View reflects repository status and recent commits after refresh.
- Any missing behavior is logged against the remaining `18` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
