# Progress: SPEC-8 - Input Extensions

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `37/49` checked in `tasks.md`
- Artifact refresh: `2026-04-03T06:40:00Z`

## Done
- Supporting artifacts now reflect the current partial delivery across voice, paste, and branch naming.
- The docs no longer imply that keybindings alone equal end-to-end input-extension completion.
- Completion tracking now separates implementation progress from pending manual verification.
- Voice input now has a guarded hotkey path in the TUI, including the disabled-config no-op coverage.
- File paste now shell-quotes paths before PTY injection so spaces and shell metacharacters survive copy/paste safely.
- File paste now also parses `file://` and `file://localhost/` clipboard payloads, improving macOS-style file URL handling when the clipboard exposes file URLs as text.
- AI branch suggestion parsing now enforces `3..=5` git-safe names before the wizard displays them.
- The wizard AI suggestion step now keeps an explicit `Manual input` option at the bottom of the list.
- The wizard now has a render-content regression test for the AI suggestion list, and the voice hotkey chord has its own keybinding test.

## Next
- Finish or confirm the voice backend path and remaining recorder lifecycle gaps.
- Close the remaining recorder backend work, the native macOS clipboard extraction path, and manual reviewer runs.
- Run repeatable reviewer walkthroughs for all three input-extension flows.
- Keep the SPEC artifacts aligned as those remaining execution slices land.
