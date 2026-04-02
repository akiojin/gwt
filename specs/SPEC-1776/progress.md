# Progress: SPEC-1776

## 2026-04-02: Parent UX spec reset

### Progress

- Reframed `SPEC-1776` from an all-in-one migration spec into a parent UX spec
- Captured a cross-spec comparison matrix so `SPEC-1776` no longer overwrites child canonical specs
- Reset the target model to `branch-first`, `permanent multi-mode`, `Profiles = Env profiles`, and tabbed management workspace
- Explicitly deferred `Settings`, `Logs`, `Versions`, and `AI summary`

### Done

- `SPEC-1776` now documents only parent UX, sequencing, and cross-spec ownership
- `research.md` now records the old TUI vs current TUI vs current backend vs new target matrix
- `tasks.md` now starts with child-spec-aware implementation phases instead of full-feature migration

### Next

- Sync child specs that now conflict with the parent direction, starting with `SPEC-1654`, `SPEC-1770`, `SPEC-1777`, and `SPEC-1782`
- Begin implementation from the new `Branches` and session workspace model after the child sync list is closed

## 2026-04-02: Normal-mode virtual terminal viewport

### Progress

- Replaced the explicit PTY copy mode with an always-on transcript-backed viewport for Agent and Shell tabs
- Enabled mouse capture in the Main layer so wheel / trackpad scroll and drag-selection work directly in normal mode
- Kept session-scoped raw PTY transcripts as the source of truth for history rendering, while preserving live follow at the bottom
- Added RED/GREEN coverage for keyboard scrollback, wheel scrollback, drag-copy, viewport freeze during new PTY output, and historical ANSI rendering
- Removed the `LIVE` / `SCROLLED` status label after it proved to be diagnostic noise, and made PTY-bound key input / paste immediately snap the viewport back to the live tail

### Done

- Agent/Shell tabs now support scrollback and drag-copy directly in normal mode
- Scrolling away from the live tail no longer snaps back when new PTY output arrives
- Returning to the bottom or pressing `End` restores live follow
- Typing or pasting while scrolled back immediately restores the live viewport before forwarding the input

### Next

- Manual E2E: run a chatty agent, scroll up with the trackpad, confirm the viewport stays fixed while output continues, then drag-copy text and return to live with `End`
