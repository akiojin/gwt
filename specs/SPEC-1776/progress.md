# Progress: SPEC-1776

## 2026-04-02: Parent UX spec reset

### Progress

- Reframed `SPEC-1776` from an all-in-one migration spec into a parent UX spec
- Captured a cross-spec comparison matrix so `SPEC-1776` no longer overwrites child canonical specs
- Reset the target model to `branch-first`, `permanent multi-mode`, `Profiles = Env profiles`, and tabbed management workspace
- Explicitly deferred `Settings`, `Logs`, `Versions`, and `AI summary`
- Expanded the coverage inventory to include workflow, persistence, and integration owners such as `SPEC-1579`, `SPEC-1787`, `SPEC-1714`, `SPEC-1786`, `SPEC-1542`, and `SPEC-1656`
- Marked `gwt-spec-ops` and related embedded workflow skills as covered via `SPEC-1579` / `SPEC-1787`, not redefined in the parent TUI spec
- Audited the workflow side more concretely: `SPEC-1579` remains reference-only, while `SPEC-1787` needs wording sync because it currently rejects a branch-first primary entry

### Done

- `SPEC-1776` now documents only parent UX, sequencing, and cross-spec ownership
- `research.md` now records the old TUI vs current TUI vs current backend vs new target matrix
- `tasks.md` now starts with child-spec-aware implementation phases instead of full-feature migration
- `tasks.md` also includes explicit audit tasks for workflow, persistence, issue, hooks, and profile-related specs
- `research.md` now includes a concrete `gwt-spec-ops` coverage audit and identifies `SPEC-1787` as the first workflow wording conflict
- `SPEC-1654`, `SPEC-1770`, `SPEC-1777`, and `SPEC-1782` have been rewritten to match the parent UX direction
- `SPEC-1787` has been reworded so branch-first primary entry and SPEC-first workflow are no longer framed as mutually exclusive
- `SPEC-1654` support artifacts (`research`, `data-model`, `quickstart`, checklists) are now consistent with the rebuilt shell model
- First-pass audit conclusions are now recorded for issue, hooks, persistence, launch, and assistant-related specs that did not require wording changes

### Next

- Begin implementation from the new `Branches` and session workspace model now that the first-pass child/audit sync list is closed

## 2026-04-02: Branch-first entry implementation

### Progress

- Added `session_count` to branch rows and synchronized it from open session tabs
- Changed branch `Enter` behavior from unconditional wizard-open to `no session / one session / many sessions`
- Added a `many sessions` overlay selector with `Open existing`, `Add session`, and `Full wizard` actions
- Added RED/GREEN tests for all three branch-enter paths plus row rendering of session counts

### Done

- Branches now reflects per-branch open session counts
- Enter on a branch with one matching session jumps straight into that session
- Enter on a branch with multiple matching sessions opens a selector instead of guessing

### Next

- Implement the session workspace itself: `equal grid`, maximize toggle, and maximize-time tab switching

## 2026-04-02: Equal-grid and maximize session workspace

### Progress

- Added `SessionLayoutMode` and made the session workspace switch between `Grid` and `Maximized`
- Implemented an equal-grid renderer for the main session workspace using all open sessions
- Added `Ctrl+G,z` to toggle maximize mode
- Kept maximize mode compatible with the existing session tab switch shortcuts
- Verified that toggling the management layer does not discard the current session layout mode

### Done

- Main layer now renders all open sessions in equal-grid mode by default
- Focused session can be maximized and restored
- Maximize mode continues to use tab switching for other sessions
- Management open/close preserves the last chosen session layout mode

### Next

- Remove remaining assumptions that the shell is fundamentally tab-first and finish the explicit `hidden pane` cleanup task

## 2026-04-02: Profiles tab extraction

### Progress

- Added `Profiles` as a first-class management tab and removed `Versions / Settings / Logs` from the visible management tab cycle
- Reused the existing profile/env editor so the dedicated Profiles tab already supports profile CRUD and env editing
- Kept the deeper `Settings` surface hidden from the first-class management flow

### Done

- Management tab cycle now follows `Branches / SPECs / Issues / Profiles`
- Switching to `Profiles` forces the environment-profile view rather than the old settings category tabs
- Existing profile CRUD tests and render smoke tests still pass under the new exposure model

### Next

- Implement OS environment variable reference/replacement in Profiles

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
