# Quickstart: SPEC-3 - Agent Management

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and open the agent launch or conversion flow from the current session.
2. Verify built-in detection, custom agent listing, and cached version display in the wizard.
3. Trigger session conversion and confirm the active session metadata changes while repository context is preserved.
4. Check the existing focused tests and notifications to confirm the original session remains intact on conversion failure.

## Repeatable Evidence
- `cargo test -p gwt-agent detect -- --nocapture`
- `cargo test -p gwt-agent version_cache -- --nocapture`
- `cargo test -p gwt-tui wizard -- --nocapture`
- `cargo test -p gwt-tui session_conversion`

## Expected Result
- The reviewer sees the current implemented scope for agent management.
- Any missing behavior is logged against acceptance or reviewer gaps rather than unchecked implementation tasks.
- No step should be treated as complete unless the code path is actually reachable today.
