# Progress: SPEC-8 - Input Extensions

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `31/49` checked in `tasks.md`
- Artifact refresh: `2026-04-03T04:58:00Z`

## Done
- Supporting artifacts now reflect the current partial delivery across voice, paste, and branch naming.
- The docs no longer imply that keybindings alone equal end-to-end input-extension completion.
- Completion tracking now separates implementation progress from pending manual verification.
- Voice input now has a guarded hotkey path in the TUI, including the disabled-config no-op coverage.
- File paste now shell-quotes paths before PTY injection so spaces and shell metacharacters survive copy/paste safely.
- The wizard AI suggestion step now keeps an explicit `Manual input` option at the bottom of the list.

## Next
- Finish or confirm the voice backend path and remaining recorder lifecycle gaps.
- Add branch-suggester validation coverage (`3-5` results and git-safe names) plus reviewer evidence.
- Run repeatable reviewer walkthroughs for all three input-extension flows.
- Close the remaining clipboard extraction gaps, including platform-specific macOS handling.
