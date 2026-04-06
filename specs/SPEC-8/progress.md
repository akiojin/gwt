# Progress: SPEC-8 - Input Extensions

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `49/49` checked in `tasks.md`
- Artifact refresh: `2026-04-05T12:22:37Z`

## Done
- Supporting artifacts now reflect the current split between the incomplete
  voice backend and the completed paste / branch-naming slices.
- The docs no longer imply that keybindings alone equal end-to-end input-extension completion.
- Completion tracking now separates execution-task completion from pending
  reviewer acceptance.
- Voice input now has a guarded hotkey path in the TUI, including the disabled-config no-op coverage.
- Voice input now routes start/stop/transcribe through a shared runtime seam in
  `gwt-tui`, and toggle/stop error paths are covered by focused unit tests.
- File paste now shell-quotes paths before PTY injection so spaces and shell metacharacters survive copy/paste safely.
- File paste now also parses `file://` and `file://localhost/` clipboard payloads, improving macOS-style file URL handling when the clipboard exposes file URLs as text.
- AI branch suggestion parsing now enforces `3..=5` git-safe names before the wizard displays them.
- The wizard AI suggestion step now keeps an explicit `Manual input` option at the bottom of the list.
- The standard Launch Agent new-branch flow now skips the AI suggestion step
  from Branches, SPEC detail, and Issue detail and opens manual branch input
  directly, so AI settings are no longer required just to type a branch name.
- The wizard now has a render-content regression test for the AI suggestion list, and the voice hotkey chord has its own keybinding test.
- The gwt-voice, gwt-clipboard, and gwt-ai suites now provide focused
  verification evidence for the currently implemented slices.

## Next
- Replace the stub Qwen3-ASR backend with a real recorder implementation, then
  close manual voice walkthrough evidence.
- Close the remaining reviewer runs for the now-complete paste and branch-name flows.
- Run repeatable reviewer walkthroughs for all three input-extension flows.
- Keep the SPEC artifacts aligned as those remaining execution slices land.
