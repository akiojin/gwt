# Quickstart: SPEC-3 - Agent Management

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and open the agent launch or conversion flow from the current session.
2. Verify built-in detection, custom agent listing, and cached version display in the wizard.
3. Trigger session conversion and confirm the active session metadata changes while repository context is preserved.
4. Treat real PTY replacement as an explicit follow-up until the last remaining task is reconciled.

## Expected Result
- The reviewer sees the current implemented scope for agent management.
- Any missing behavior is logged against the remaining `1` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
