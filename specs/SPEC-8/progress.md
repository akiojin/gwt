# Progress: SPEC-8 - Input Extensions

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `33/49` checked in `tasks.md`
- Artifact refresh: `2026-04-03T05:15:00Z`

## Done
- Supporting artifacts now reflect the current partial delivery across voice, paste, and branch naming.
- The docs no longer imply that keybindings alone equal end-to-end input-extension completion.
- Completion tracking now separates implementation progress from pending manual verification.
- Voice input now has a guarded hotkey path in the TUI, including the disabled-config no-op coverage.
- File paste now shell-quotes paths before PTY injection so spaces and shell metacharacters survive copy/paste safely.
- AI branch suggestion parsing now enforces `3..=5` git-safe names before the wizard displays them.
- The wizard AI suggestion step now keeps an explicit `Manual input` option at the bottom of the list.

## Next
- Finish or confirm the voice backend path and remaining recorder lifecycle gaps.
- Add explicit suggestion-list rendering coverage in the wizard, then close the combined verification task for the display section.
- Run repeatable reviewer walkthroughs for all three input-extension flows.
- Close the remaining clipboard extraction gaps, including platform-specific macOS handling.
