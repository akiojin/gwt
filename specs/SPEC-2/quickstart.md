# Quickstart: SPEC-2 - Workspace Shell

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and enter the main workspace shell.
2. Move across terminal tabs and split panes using the documented keybindings.
3. Open the management panel and verify the visible tabs match the current product shell, without assuming a live Specs tab.
4. Record any remaining persistence or help-overlay gaps against `tasks.md` before claiming the shell complete.

## Expected Result
- The reviewer sees the current implemented scope for workspace shell.
- Any missing behavior is logged against the remaining `23` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
