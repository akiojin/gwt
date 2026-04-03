# Quickstart: SPEC-8 - Input Extensions

## Reviewer Flow
1. Use the current branch to trigger `Ctrl+G` input extensions from an active terminal session.
2. Verify that voice input stays idle when the feature is disabled in settings.
3. Verify file-paste behavior against clipboard content and active PTY injection.
4. Run the AI branch-name suggestion flow and confirm the list keeps `Manual input` at the bottom while timeout/error fallback remains usable.
5. Treat voice backend completion and manual reviewer passes as remaining work until explicitly verified.

## Repeatable Evidence
- `cargo test -p gwt-tui handle_voice_start_recording_is_noop_when_disabled -- --nocapture`
- `cargo test -p gwt-tui input::keybind -- --nocapture`
- `cargo test -p gwt-tui wizard -- --nocapture`
- `cargo test -p gwt-clipboard -- --nocapture`
- `cargo test -p gwt-ai branch_suggest -- --nocapture`

## Expected Result
- The reviewer sees the current implemented scope for input extensions.
- Any missing behavior is logged against the remaining `12` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
