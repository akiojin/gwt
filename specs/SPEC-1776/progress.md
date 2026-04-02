# Progress: SPEC-1776

## 2026-04-02: Settings re-exposure

### Progress

- Re-exposed `Settings` as a management tab without collapsing `Profiles` back into it
- Added a dedicated visible-order slot for `Settings` between `Profiles` and `Versions`
- Restricted `Settings` category navigation to non-env categories so `Env` remains owned by `Profiles`
- Added RED/GREEN coverage for tab switching, category skipping, and hidden `Env` tab rendering

### Done

- Management tab cycle now follows `Branches / SPECs / Issues / Profiles / Settings / Versions / Logs`
- Switching from `Profiles` to `Settings` resets the category from `Env` back to `General`
- `Settings` renders only `General / Worktree / Agent / Custom / AI` category tabs
- Existing settings surface is reachable again without duplicating env-profile management

### Next

- Keep the remaining deferred scope narrow: `AI summary` and custom-agent redesign only

## 2026-04-02: Versions and Logs re-exposure

### Progress

- Re-exposed `Versions` and `Logs` in the management tab cycle without reopening the broader `Settings` surface
- Kept the branch-first core intact by appending the restored tabs after `Profiles`
- Updated tab rendering to use the visible tab order so hidden `Settings` no longer distorts selection state
- Added RED/GREEN coverage for management-tab metadata and `Tab` cycling order

### Done

- Management tab cycle now follows `Branches / SPECs / Issues / Profiles / Versions / Logs`
- Existing `Versions` and `Logs` screens are reachable again from the management workspace
- The management tab bar now highlights according to visible tab order instead of legacy enum order
- `SPEC-1776` artifacts now record `Versions / Logs` as restored while `Settings` and `AI summary` remain deferred

### Next

- Continue with deferred work only where it materially improves the rebuilt shell, starting with `Settings` or later `AI summary`

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
- Selector-driven `Add session` now keeps Quick Start history, while `Full wizard` explicitly skips it
- Codex hooks confirmation remains attached to the rebuilt launch path
- Session persistence still records the actual launched worktree path under the rebuilt launch flow

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
- Branch rows now show running/stopped session summaries in addition to total session count
- Startup now loads `Issues / SPECs / Versions / Logs` through a native background thread while keeping `Branches` available immediately
- No remaining code path depends on hidden panes; the session model is now grid/maximize only

### Next

- Run broader verification on the rebuilt shell model and continue polish from the branch-first baseline

## 2026-04-02: Profiles tab extraction

### Progress

- Added `Profiles` as a first-class management tab and removed `Versions / Settings / Logs` from the visible management tab cycle
- Reused the existing profile/env editor so the dedicated Profiles tab already supports profile CRUD and env editing
- Kept the deeper `Settings` surface hidden from the first-class management flow

### Done

- Management tab cycle now follows `Branches / SPECs / Issues / Profiles`
- Switching to `Profiles` forces the environment-profile view rather than the old settings category tabs
- Existing profile CRUD tests and render smoke tests still pass under the new exposure model
- Profiles env editing now merges OS environment entries with profile overrides/additions and supports disable/override persistence
- Profiles env rows now show explicit `[OS] / [OVR] / [ADD] / [OFF]` state markers and contextual action hints
- `SPECs` launch now opens either a branch selector or a derived new-branch wizard
- `Issues` launch now opens the issue-derived branch wizard directly

### Next

- Continue polishing the branch list density and Profiles interaction details

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
