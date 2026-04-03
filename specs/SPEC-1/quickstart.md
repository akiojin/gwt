# Quickstart: SPEC-1 - Terminal Emulation

## Reviewer Flow
1. Run `cargo run -p gwt-tui` and open a workspace with an active terminal tab.
2. Emit ANSI-heavy output and confirm colors plus cursor movement render correctly.
3. Scroll upward, then make a text selection and verify copy behavior still matches the visible buffer.
4. Track the remaining gap: `Ctrl+click` URL open and alt-screen coverage are still pending.

## Expected Result
- The reviewer sees the current implemented scope for terminal emulation.
- Any missing behavior is logged against the remaining `17` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
