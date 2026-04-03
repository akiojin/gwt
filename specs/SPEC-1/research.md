# Research: SPEC-1 - Terminal Emulation

## Scope Snapshot
- Canonical scope: vt100 based terminal rendering, scrollback, selection, and URL handling.
- Current status: `open` / `Ready for Dev`.
- Task progress: `0/17` checked in `tasks.md`.
- Notes: Implementation has not started from the task tracker perspective, even though base rendering paths already exist in code.

## Decisions
- Keep `vt100` as the screen-state model instead of introducing a custom emulator layer.
- Treat rendering, scrollback, selection, and URL hit-testing as one terminal surface concern.
- Do not mark the SPEC complete until URL open and alt-screen behavior are both verified.

## Open Questions
- Confirm the exact input model for URL opening, including modifier keys and terminal mouse mode interactions.
- Decide whether alt-screen coverage is automated, manual, or both before closing the remaining gap.
