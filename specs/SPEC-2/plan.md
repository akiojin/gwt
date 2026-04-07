# Implementation Plan: SPEC-2 — Workspace Shell

## Summary

Complete the workspace shell with branch detail view, help overlay, session persistence, and SPECs tab removal. The branch detail view replaces the independent SPECs management tab by integrating SPEC display into a split-layout branch detail panel.

## Technical Context

- **Architecture**: Elm Architecture (Model/Message/Update/View) in gwt-tui
- **Keybind system**: Ctrl+G prefix state machine in `input/keybind.rs`
- **Screens**: `screens/branches.rs` (primary target for branch detail)
- **List rendering**: Uses `ListState` + `render_stateful_widget` for scrollable lists
- **Shared utilities**: `screens/mod.rs` — clamp_index, move_up/down, list_item_style, centered_rect

## Constitution Check

- Spec before implementation: yes, this SPEC documents all requirements
- Test-first: all phases start with RED tests
- No workaround-first: branch detail is a proper implementation, not a hack
- Minimal complexity: each phase is independent and separately verifiable

## Complexity Tracking

| Risk | Mitigation |
|------|-----------|
| Branch detail sections need data from multiple sources | Prefetch asynchronously, cache per branch, refresh explicitly |
| SPECs tab removal affects tab indexing and keybinds | Update all ManagementTab references |
| Agent launch from detail needs simplified wizard | Reuse WizardState with branch pre-filled |
| Worktree delete is destructive | Confirmation dialog required |

## Phased Implementation

### Phase 1: Help Overlay Auto-Collection (6 tasks)
Implement keybinding registry auto-collection for Ctrl+G,? help overlay.

### Phase 2: Session Persistence Improvement (7 tasks)
Extend save/restore to include display_mode, management panel state.

### Phase 3: Git View Tab (5 tasks)
Implement Git View management tab component.

### Phase 4: Branch Detail View (26 tasks)
4.1: Remove SPECs tab (4 tasks)
4.2: Branch detail split layout (4 tasks)
4.3: Detail sections — Overview/SPECs/GitStatus/Sessions/Actions (6 tasks)
4.4: Actions — agent launch, shell, worktree delete (6 tasks)
4.5: Integration and testing (6 tasks)

### Phase 5: Regression and Polish (5 tasks)

### Phase 8: Branch-First UX Restoration (5 tasks)
Reconcile remaining old-TUI branch-first UX requirements that are already present in `spec.md`
but not fully reflected in the new TUI implementation.

8.1: Branch list display (2 tasks)
- Remove category headers and locality badges from the branch list.
- Render `name + worktree indicator + HEAD indicator` in a stable old-TUI style.

8.2: Primary branch actions (2 tasks)
- Restore `Enter=Wizard`, `Shift+Enter=Shell`, `Space=select detail`, `Ctrl+C=delete worktree`
  on the Branches tab without regressing existing focus-aware routing.
- Update contextual footer hints so Branches communicates the restored actions directly.

8.3: Regression and verification (1 task)
- Add focused routing/render coverage and re-run workspace verification.

### Phase 9: Branch Mnemonic Restoration (5 tasks)
Restore the old-TUI branch-local mnemonic shortcuts that make Branches usable as a daily
entry point without requiring Ctrl+G for every follow-up action.

9.1: Branch-local shortcuts (3 tasks)
- Restore `m` as the Branches-local view-mode cycle.
- Restore `v` as a direct jump from Branches to Git View.
- Restore `f` as a search alias and `?` / `h` as local help entry points.

9.2: UX polish and regression (2 tasks)
- Update branch-specific footer hints to advertise the restored mnemonic set.
- Add focused regression coverage and re-run workspace verification.

### Phase 10: Branch Detail Sessions Restoration (5 tasks)
Restore the Branch Detail `Sessions` pane from a count-only placeholder to a branch-scoped
session summary list so the Branches view can function as the primary workspace entry point.

10.1: Session summary extraction (2 tasks)
- Build a lightweight render-time session summary from the selected branch without adding
  new persistent state.
- Limit the scope to `app.rs` and `branches.rs` to avoid reopening unrelated dirty files.

10.2: Sessions pane rendering (2 tasks)
- Replace the count-only placeholder with a typed list that shows Shell/Agent, session name,
  and an active-session marker for the current tab.
- Preserve the existing empty-state fallback when no sessions match the selected branch.

10.3: Regression and verification (1 task)
- Add focused extraction/render coverage and re-run workspace verification.

### Phase 11: Branch Detail Session Focus Actions (5 tasks)
Turn the restored `Sessions` pane into an actionable branch-first surface so the user can move
from the selected branch directly into one of its running sessions without leaving Branches first.

11.1: Session row selection (2 tasks)
- Track a lightweight selection index for branch-scoped session rows inside `BranchesState`.
- Keep the selection clamped/reset when the branch list changes or when session rows disappear.

11.2: Focus handoff (2 tasks)
- Route `Up/Down` inside the `Sessions` section to row selection instead of Docker controls.
- Route `Enter` inside the `Sessions` section to activate the selected session and move focus to the terminal pane.

11.3: Regression and verification (1 task)
- Add focused routing/render coverage and re-run workspace verification.

### Phase 12: Status Bar Restoration (5 tasks)
Restore the old-TUI footer model so the bottom line carries workspace context again instead of
acting as a keybind-hints-only strip.

12.1: Footer context contract (2 tasks)
- Render current session summary, current branch context, and session type / agent type in the bottom status bar.
- Preserve context-sensitive keybind hints and notification visibility within the same single-line footer surface.

12.2: Wiring and regression coverage (2 tasks)
- Route the main view footer through the shared status-bar widget again instead of the bespoke hints-only renderer.
- Add focused render coverage for shell sessions, agent sessions, and Branches focus hints so the footer contract stays stable.

12.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 13: Branch Detail Direct Actions (5 tasks)
Restore the old-TUI direct-action ergonomics inside Branch Detail so the selected branch remains
actionable even after focus leaves the top list.

13.1: Direct branch actions (2 tasks)
- Route `Shift+Enter` in Branch Detail to open a shell for the selected branch when the active
  section is not `Sessions`.
- Route `Ctrl+C` in Branch Detail to open the delete-worktree confirmation when the active
  section is not `Sessions`.

13.2: Section-sensitive hints (2 tasks)
- Replace the generic Branch Detail footer hint with section-aware hints that explain when
  `Enter` focuses a session versus when direct branch actions are available.
- Keep Docker lifecycle hints visible in the Overview section without touching shared layout code.

13.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 14: Branch Detail Title Context (5 tasks)
Restore the old-TUI branch context in the bottom pane chrome so the selected branch remains
visible even when the user is reading another detail section.

14.1: Title context contract (2 tasks)
- Keep the selected branch name visible in the Branch Detail pane title alongside the section tabs.
- Limit the change to `app.rs` so existing section renderers and shared screen utilities stay untouched.

14.2: Focused rendering coverage (2 tasks)
- Add focused render coverage for the selected-branch title contract and the no-selection fallback.
- Preserve the existing section-tab highlighting behavior while adding the branch context suffix.

14.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 15: Branch Detail Escape Back (5 tasks)
Restore the old-TUI `Esc:back` affordance in Branch Detail so the detail pane behaves like a
temporary deep-focus surface instead of a focus trap.

15.1: Escape focus contract (2 tasks)
- Route `Esc` in Branch Detail back to the Branches list focus.
- Preserve the selected branch, active detail section, and `Sessions` row selection when focus returns.

15.2: Focused routing coverage (2 tasks)
- Add focused routing coverage for the list-focus handoff.
- Add focused routing coverage proving the detail context survives the handoff.

15.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 16: Focus Border Color Parity (5 tasks)
Restore the old-TUI focus chrome contract so pane borders use the colors already documented in
the spec instead of the temporary green/white implementation.

16.1: Border color contract (2 tasks)
- Render focused panes with `Color::Cyan`.
- Render unfocused panes with `Color::Gray`.

16.2: Focused render coverage (2 tasks)
- Add focused coverage for the focused border color contract.
- Add focused coverage for the unfocused border color contract.

16.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 17: Worktree-Aware Branch Detail Direct Actions (5 tasks)
Align Branch Detail direct-action affordances with reachability so the detail pane only advertises
and executes worktree-backed actions when the selected branch can actually serve them.

17.1: Reachability contract (2 tasks)
- Hide `Shift+Enter:shell` and `Ctrl+C:delete` from Branch Detail hints when the selected branch has no worktree.
- Block worktree-backed direct actions from Branch Detail when the selected branch has no worktree.

17.2: Focused routing coverage (2 tasks)
- Add focused coverage for the no-worktree shell no-op.
- Add focused coverage for the no-worktree delete-confirm no-op and the updated hint text.

17.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 18: Management Panel Width Default (5 tasks)
Restore a more old-TUI-like workspace balance by making the management panel narrower than the
session pane while keeping the implementation closed to the existing layout code in `app.rs`.

18.1: Layout contract (2 tasks)
- Use a sensible default proportion of `40% management / 60% session` when the management panel is visible.
- Share the same split helper between render-time layout and `active_session_content_area()` so hit-testing and rendering agree.

18.2: Focused geometry coverage (2 tasks)
- Add focused coverage for the 40/60 split helper geometry.
- Add focused coverage for the session content rect geometry while management is visible.

18.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 19: Branch Detail Local Mnemonics (5 tasks)
Restore the old-TUI branch-first muscle memory inside Branch Detail so the user can keep using the
same local mnemonics after moving focus off the list.

19.1: Local mnemonic routing (2 tasks)
- Allow `m` in Branch Detail to toggle the Branches view mode and refresh the visible detail.
- Allow `v`, `f`, and `?` / `h` in Branch Detail to mirror the Branches-list local actions without requiring a focus hop first.

19.2: Focused routing coverage (2 tasks)
- Add focused coverage for Branch Detail `m` / `v` / `f` routing.
- Add focused coverage for the Branch Detail footer hint text that advertises the restored local mnemonics.

19.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 20: Compact Management Header Context (5 tasks)
Restore a more old-TUI-like management chrome by making the top header concise and contextual
instead of spending most of the narrow pane on a full repository path.
This was an intermediate restoration step and is superseded by Phase 30, which removes the
standalone banner entirely in favor of pane-title chrome.

20.1: Compact context contract (2 tasks)
- Render the repository basename instead of the full repository path in the management header.
- Show the active management context in the same line so the header carries tab/focus meaning without widening the pane chrome.

20.2: Focused render coverage (2 tasks)
- Add focused coverage for the compact repository/context header text.
- Add focused coverage for the Branch Detail focus variant so the header changes as focus moves inside the management pane.

20.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 28: Ctrl+G Focus Cycle for Management (5 tasks)
Close the remaining supplemental-surface focus gap by making `Ctrl+G, Tab` / `Ctrl+G, Shift+Tab` respect which
management tabs actually have a second pane.

28.1: Focus-cycle contract (2 tasks)
- Keep `Branches` on the existing three-surface cycle: `Terminal <-> TabContent <-> BranchDetail`.
- Restrict every other management tab to a two-surface cycle: `Terminal <-> TabContent`.

28.2: Focused routing coverage (2 tasks)
- Add focused coverage for `Ctrl+G, Tab` on a non-Branches management tab so it skips `BranchDetail`.
- Add focused coverage for `Ctrl+G, Shift+Tab` on a non-Branches management tab so it also skips `BranchDetail`.

28.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 29: Responsive Management Split (5 tasks)
Restore a more usable old-TUI workspace balance across terminal sizes without reopening shared
layout files outside `app.rs`.

29.1: Responsive split contract (2 tasks)
- Keep wide terminals (`>=120 cols`) on the `40/60` management/session split.
- Fall back to `50/50` on standard and narrower widths so the management pane keeps enough room for tab chrome and context.

29.2: Shared helper coverage (2 tasks)
- Add focused coverage for the responsive split helper at standard and wide widths.
- Add focused coverage proving `active_session_content_area()` follows the same helper at both widths.

29.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 30: Remove Redundant Management Banner (5 tasks)
Finish the old-TUI management chrome restoration by removing the standalone `gwt | ...` banner
row and relying on the existing pane titles for context.

30.1: Banner-removal contract (2 tasks)
- The management render no longer emits a standalone banner row above the panes.
- The top visible row of the management surface now belongs to pane-title chrome instead.

30.2: Layout simplification (2 tasks)
- Remove the extra header split from `app.rs` management rendering.
- Keep Branches and non-Branches panes using the full management area so list/detail density increases by one row.

30.3: Verification (1 task)
- Re-run focused and broad workspace verification, refresh snapshots if needed, and sync SPEC-2 artifacts.

### Phase 31: Restore Terminal-Focused Footer Mnemonics (5 tasks)
Bring the main workspace footer closer to old-TUI daily-driver behavior by advertising the global
workspace shortcuts while Terminal focus remains active.

31.1: Terminal-hint contract (2 tasks)
- Terminal-focused footer hints advertise the global management shortcuts (`Ctrl+G,b/i/s`, `Ctrl+G,g`).
- Terminal-focused footer hints advertise the session shortcuts (`Ctrl+G,c`, `Ctrl+G,[]/1-9`, `Ctrl+G,z`, `Ctrl+G,?`).

31.2: Focused render coverage (2 tasks)
- Add focused coverage for terminal hints exposing the global management shortcuts.
- Add focused coverage for terminal hints exposing the session/layout/help shortcuts.

31.3: Verification (1 task)
- Re-run focused and broad workspace verification and sync SPEC-2 artifacts.

### Phase 32: Make Terminal Footer Hints Survive Standard Width (5 tasks)
Fix the Phase 31 footer regression by compacting the terminal footer context and using grouped hint
notation that remains visible on standard-width terminals.

32.1: Compact-footer contract (2 tasks)
- Standard-width terminal footers use grouped shortcut notation (`Ctrl+G:b/i/s g c []/1-9 z ?`) instead of repeating the `Ctrl+G` prefix on every token.
- Standard-width terminal footers keep `C-g Tab:focus` and `^C×2` visible without requiring the user to open the management pane first.

32.2: Focused render coverage (2 tasks)
- Add RED coverage that the grouped terminal footer shortcuts remain visible at standard width.
- Add RED coverage that `C-g Tab:focus` and `^C×2` remain visible at standard width.

32.3: Verification (1 task)
- Re-run focused tests, snapshot refresh, broad workspace verification, and SPEC-2 artifact sync.

### Phase 33: Compact Management Footers at Standard Width (5 tasks)
Extend the `width <= 80`, notification-free footer compaction to management and Branch Detail
panes so pane-local guidance remains visible instead of truncating behind the shared status
context.

33.1: Compact-management contract (2 tasks)
- `width <= 80` Branches-list and generic management footers use compact hint notation that fits alongside the shared status context when no notification is showing.
- `width <= 80` Branch Detail footers keep the local branch affordances visible with a compact hint form instead of losing the trailing actions when no notification is showing.

33.2: Focused render coverage (2 tasks)
- Add RED coverage that `width <= 80` Branches list and generic management hints remain visible.
- Add RED coverage that `width <= 80` Branch Detail hints remain visible.

33.3: Verification (1 task)
- Re-run focused tests, snapshot refresh, broad workspace verification, and SPEC-2 artifact sync.

### Phase 34: Compact Narrow Management Titles (5 tasks)
When the management pane is too narrow to fit the full tab strip in the pane title, collapse the
title to the active management tab only so the current surface remains readable.

34.1: Narrow-title contract (2 tasks)
- Branches pane titles show only the active tab label whenever the full tab strip would truncate in the available title width.
- Non-Branches management pane titles also show only the active tab label whenever the full tab strip would truncate, while wider panes keep the full strip.

34.2: Focused render coverage (2 tasks)
- Add RED coverage that narrow Branches pane titles collapse to the active tab only.
- Add RED coverage that narrow and medium non-Branches management pane titles collapse to the active tab only while extra-wide panes keep the strip.

34.3: Verification (1 task)
- Re-run focused tests, snapshot refresh, broad workspace verification, and SPEC-2 artifact sync.

### Phase 35: Compact Narrow Session Titles (5 tasks)
When the session pane is too narrow to fit the full session tab strip in the pane title, collapse
the title to the active session only so the current workstream remains readable.

35.1: Narrow-title contract (2 tasks)
- Session pane titles show only the active session label whenever the full tab strip would truncate in the available title width.
- Extra-wide session panes keep the full session tab strip once every session label fits.

35.2: Focused render coverage (2 tasks)
- Add RED coverage that standard-width session panes collapse to the active session only.
- Add RED coverage that medium-width session panes still collapse while extra-wide panes keep the strip.

35.3: Verification (1 task)
- Re-run focused tests, broad workspace verification, and SPEC-2 artifact sync.

### Phase 36: Make Non-Branches Footer Hints Mode-Aware (5 tasks)
Align the visible footer hints with the routing that already differs by management tab and mode so
the status bar no longer overclaims sub-tab controls or the wrong `Esc` action.

36.1: Visible contract (2 tasks)
- Detail drill-downs such as Issues and PR Dashboard advertise `Esc:back` instead of `Esc:term`.
- Form/edit states such as Profiles create/edit/delete advertise `Esc:cancel`, while only tabs that really support sub-tabs keep `Ctrl+←→:sub-tab`.

36.2: Focused render coverage (2 tasks)
- Add RED coverage that Issues detail and Profiles create mode advertise the correct `Esc` affordance.
- Add RED coverage that Settings keeps `Ctrl+←→:sub-tab` while Git View omits it.

36.3: Verification (1 task)
- Re-run focused tests, broad workspace verification, and SPEC-2 artifact sync.

### Phase 37: Consume Branch Detail Esc Before Warn Fallback (5 tasks)
Keep the old-TUI Branch Detail `Esc:back` behavior pure even while a warn notification is visible,
so the detail pane exits first and warn dismissal remains a later fallback from the list surface.

37.1: Esc consumption contract (2 tasks)
- Branch Detail `Esc` returns focus to the Branches list without mutating detail context even when a warn notification is visible.
- That same `Esc` does not dismiss the warn notification; a later unclaimed `Esc` from the list surface may dismiss it through the existing management fallback.

37.2: Focused routing coverage (2 tasks)
- Add RED coverage that Branch Detail `Esc` with a warn toast preserves the warning while returning focus to the list.
- Add RED coverage that a second `Esc` from the list surface still dismisses the warn notification through the normal fallback path.

37.3: Verification (1 task)
- Re-run focused tests, broad workspace verification, and SPEC-2 artifact sync.

### Phase 38: Make Non-Branches Footer Hints Action-Aware (5 tasks)
Replace the last generic `Enter:action` overclaims on non-Branches management tabs so footer hints
match each tab's real primary action and refresh/search affordances.

38.1: Action contract (2 tasks)
- Git View, Versions, Issues, and PR Dashboard footers advertise their real list/detail actions instead of a shared generic action label.
- Versions remains refresh-only, while drill-down detail surfaces such as PR Dashboard advertise `Enter:close` and `Esc:back`.

38.2: Focused render coverage (2 tasks)
- Add RED coverage that Git View and Versions expose the correct action/refresh hints without falling back to `Enter:action`.
- Add RED coverage that Issues list and PR Dashboard detail expose `detail/search/close/refresh` hints that match real routing.

38.3: Verification (1 task)
- Re-run focused tests, broad workspace verification, and SPEC-2 artifact sync.

### Phase 39: Remove Redundant Branch Detail Inner Titles (5 tasks)
Keep Branch Detail chrome aligned with the old-TUI border contract by letting the pane title carry
section and branch context, while the inner detail renderer stays borderless and title-free.

39.1: Chrome contract (2 tasks)
- Branch Detail content no longer repeats nested titles such as `Overview` or `Sessions` inside the body when the surrounding pane title already names the active section.
- Empty states and populated states both remain readable without relying on inner block titles.

39.2: Focused render coverage (2 tasks)
- Add RED coverage that `render_detail_content()` keeps Overview body text while omitting the redundant `Overview` title.
- Add RED coverage that `render_detail_content()` keeps session rows while omitting the redundant `Sessions` title.

39.3: Verification (1 task)
- Re-run focused tests, snapshot verification, broad workspace verification, and SPEC-2 artifact sync.

### Phase 21: Focus-Preserving Layer Toggle (5 tasks)
Make the management panel behave like a supplemental surface again so toggling it on/off does not
leave the workspace in a stale management-focus state.

21.1: Focus contract (2 tasks)
- Showing the management panel with `Ctrl+G,g` keeps terminal focus instead of stealing focus into the left pane.
- Hiding the management panel always normalizes focus back to Terminal so Main layer status hints stay correct.

21.2: Focused routing coverage (2 tasks)
- Add focused coverage for showing the management panel from Main while preserving terminal focus.
- Add focused coverage for hiding the management panel from TabContent/BranchDetail and normalizing focus to Terminal.

21.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 22: Focus-Preserving Global Tab Switches (5 tasks)
Align the global management-tab shortcuts with the new supplemental-panel contract so opening a
requested management tab from Terminal does not yank focus out of the main workstream.

22.1: Focus contract (2 tasks)
- Switching to a management tab from Terminal shows the requested tab while keeping terminal focus.
- Switching tabs from within management list/detail focus still lands on `TabContent`.

22.2: Focused routing coverage (2 tasks)
- Add focused coverage for terminal-origin tab switching that preserves focus and still loads tab data.
- Add focused coverage for management-origin tab switching that normalizes focus to `TabContent`.

22.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 23: Esc-Back for Issues and PR Detail Views (5 tasks)
Restore the old-TUI expectation that read-only detail panes inside management are temporary drill
downs that close with `Esc` instead of trapping the user until another action key is used.

23.1: Detail-close contract (2 tasks)
- `Esc` closes `Issues` detail view and returns to the list without changing the selected row.
- `Esc` closes `PR Dashboard` detail view and returns to the list without changing the selected row.

23.2: Focused routing coverage (2 tasks)
- Add focused coverage for `Issues` detail closing via `Esc`.
- Add focused coverage for `PR Dashboard` detail closing via `Esc`.

23.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 24: Esc-Back for Logs Detail View (5 tasks)
Bring the Logs tab into the same management-detail contract so the log detail pane behaves like a
temporary drill-down instead of forcing the user to use Enter again to escape.

24.1: Detail-close contract (2 tasks)
- `Esc` closes `Logs` detail view and returns to the list without changing the selected log entry.
- Existing log filter/detail routing (`f`, `d`, Enter) remains unchanged.

24.2: Focused routing coverage (2 tasks)
- Add focused coverage for `Logs` detail closing via `Esc`.
- Add focused coverage that existing log routing still works after the change.

24.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 25: Esc Returns from Management Pane to Terminal (5 tasks)
Complete the supplemental-surface contract by making plain `Esc` in management list/pane focus
return to the terminal when no tab-specific search/detail/edit flow owns the key.

25.1: Escape fallback contract (2 tasks)
- In management list/pane focus, `Esc` returns focus to `Terminal` when no warn notification is pending.
- If a warn notification is visible, `Esc` still dismisses the warn notification instead of changing focus.

25.2: Focused routing coverage (2 tasks)
- Add focused coverage for terminal-return behavior from a management pane.
- Add focused coverage that warn-dismiss still wins over terminal-return.

25.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 26: Profiles Esc Uses Supplemental Fallback in List Mode (6 tasks)
Close the remaining tab-specific hole in the management-pane escape contract by making `Profiles`
use the generic supplemental fallback only when the tab is in plain list mode.

26.1: Mode-aware escape contract (3 tasks)
- In `Profiles` list mode, `Esc` returns focus to `Terminal` when no warn notification is pending.
- In `Profiles` list mode, a visible warn notification still consumes `Esc` for dismissal first.
- In `Profiles` create/edit/delete flows, `Esc` continues to cancel the current flow instead of changing focus.

26.2: Focused routing coverage (2 tasks)
- Add focused coverage for `Profiles` list-mode terminal-return behavior.
- Add focused coverage for warn-dismiss priority and create-mode cancel priority.

26.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 27: Status Bar Hints Reflect Esc-to-Terminal Contract (5 tasks)
Bring the visible keybind guidance back in line with the restored behavior by teaching the
management-list hints to advertise `Esc` as a return-to-terminal action.

27.1: Hint parity contract (2 tasks)
- The Branches list status-bar hint includes `Esc:term` alongside the existing branch-local actions.
- The generic management list status-bar hint includes `Esc:term` alongside the generic list actions.

27.2: Focused rendering coverage (2 tasks)
- Add focused rendering coverage for Branches-list hint parity.
- Add focused rendering coverage for generic management-list hint parity.

27.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 40: Preserve Session Count in Compact Session Titles (5 tasks)
Keep the standard-width session pane chrome old-TUI-friendly by retaining the active session's
position/count even after the full strip collapses to a single active label.

40.1: Compact title context contract (2 tasks)
- Compact session titles show the active session as `n/N` plus the active session label whenever the full strip would truncate.
- Extra-wide session panes continue to use the full session strip instead of the compact `n/N` chrome.

40.2: Focused rendering coverage (2 tasks)
- Add focused coverage for standard-width compact titles keeping the active index/count visible.
- Add focused coverage that extra-wide titles still use the full strip and omit the compact `n/N` badge.

40.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 41: Restore Split-Grid Session Title Identity (5 tasks)
Bring split/grid mode closer to old-TUI muscle memory by making each pane title carry the same
session identity cues that tab mode already exposes.

41.1: Grid-title identity contract (2 tasks)
- Grid pane titles show each pane's stable `n:` session position before the session label.
- Grid pane titles keep the session-type icon alongside the label instead of rendering name-only chrome.

41.2: Focused rendering coverage (2 tasks)
- Add RED coverage that grid pane titles expose the numeric session position for multiple panes.
- Add RED coverage that the grid pane titles keep the session-type icon visible.

41.3: Verification (1 task)
- Re-run focused tests, snapshot verification, broad workspace verification, and refresh SPEC-2 artifacts.

### Phase 42: Cache Branch Detail Data Off The Input Path (5 tasks)
Remove synchronous branch-detail reloads from `Branches` navigation so the list stays responsive
even when Docker, git status, git log, or worktree filesystem reads are slow.

42.1: Cached detail contract (2 tasks)
- Prefetch branch detail data asynchronously at startup and keep it in a branch-keyed cache.
- Route `Branches` selection changes to cached detail only, and use `r` for asynchronous refresh.

42.2: Focused coverage (2 tasks)
- Add RED coverage that cached detail switches immediately on `Up` / `Down` without reloading from disk.
- Add RED coverage that startup/refresh asynchronous loads populate or refresh the cache without blocking navigation.

42.3: Verification (1 task)
- Re-run focused tests, broad workspace verification, and refresh SPEC-2 artifacts and progress evidence.

### Phase 43: Normalize Reverse Focus Keys And Startup PTY Geometry (5 tasks)
Close two remaining usability regressions in the workspace shell: reverse focus cycling must honor
all real `Shift+Tab` encodings, and the default shell PTY must start at the actual session-pane size
instead of waiting for a later resize event.

43.1: Input and geometry contract (2 tasks)
- Accept both `BackTab` and `Tab`+`Shift` as reverse pane-focus navigation across Branches and non-Branches management tabs.
- Seed startup PTY geometry from the live terminal frame size before the default shell is spawned so the right-side session pane is not born as `80x24`.

43.2: Focused coverage (2 tasks)
- Add RED coverage for reverse focus cycling when the key arrives as `KeyCode::Tab` plus the Shift modifier.
- Add RED coverage for startup terminal-size synchronization so the initial shell geometry matches the computed session pane before any later resize event.

43.3: Verification (1 task)
- Re-run focused tests, broad workspace verification, and refresh SPEC-2 artifacts and progress evidence.

### Phase 44: Stop Branch List Wrap, Prefer Local Branches, And Keep Nearby Tabs Visible (5 tasks)
Tighten the daily Branches workflow by removing list wraparound, promoting local branches in the
mixed list, and making narrow management titles preserve nearby tab context instead of showing the
active tab alone.

44.1: Branch list movement contract (2 tasks)
- Make Branches list `Up`/`Down` stop at the first/last visible row instead of wrapping around.
- Keep this change scoped to the Branches list so other management lists keep their current behavior.

44.2: Branch ordering and title-context coverage (2 tasks)
- In `ViewMode::All`, keep local branches ahead of remote branches while preserving the active sort mode inside each group.
- Change narrow management titles from active-only collapse to a nearby-tab window that keeps the active tab and adjacent tabs visible, using ellipsis when tabs remain hidden off-screen.

44.3: Verification (1 task)
- Re-run focused and broad workspace verification, then refresh SPEC-2 artifacts and progress tracking.

### Phase 45: Default Branches Filter To Local (5 tasks)
Close the remaining mismatch between the intended branch-first workflow and the initial Branches
view by making the screen open on local branches instead of `All`.

45.1: Initial-view contract (2 tasks)
- Change the default `Branches` filter from `All` to `Local`.
- Keep the existing `m` / view-mode cycle semantics, but start the cycle from `Local`.

45.2: Coverage and artifact sync (2 tasks)
- Add RED coverage proving the default Branches state starts in `Local` and that the cycle still reaches `Remote` and `All`.
- Refresh reviewer guidance and snapshots so the initial management render shows `View: Local`.

45.3: Verification (1 task)
- Re-run broad workspace verification, snapshot verification, and refresh SPEC-2 progress artifacts.

### Phase 46: Stabilize Branch Detail Prefetch (5 tasks)
Fix the async Branch Detail preload so repeated refreshes do not leave detached workers behind
and so Docker state is captured once per refresh instead of once per branch.

46.1: Worker lifecycle safety (2 tasks)
- Keep the active branch-detail preload worker tracked in the model.
- When a newer preload is scheduled, cancel the superseded worker and reap finished worker handles so stale refreshes do not continue shelling out in the background.

46.2: Per-refresh Docker snapshot (2 tasks)
- Move Docker container discovery out of `load_branch_detail()` and into the refresh worker so each preload performs at most one Docker scan.
- Pass the shared Docker snapshot into each branch-detail load while keeping the visible Branch Detail payload unchanged.

46.3: Verification (1 task)
- Add focused coverage for canceling superseded workers and for single-snapshot Docker loading, then rerun broad verification and refresh SPEC-2 artifacts.

### Phase 47: Keep Branches List Responsive During Detail Backfill (3 tasks)
Fix the recurrence where Branch Detail backfill can still make Branches navigation feel sticky under larger branch sets.

47.1: Tick budget for preload event application (1 task)
- Bound branch-detail event draining per tick so one frame cannot consume the entire queue when preload has accumulated many branch payloads.

47.2: Regression coverage (1 task)
- Add a RED test proving one tick leaves remaining branch-detail preload events queued, then verify subsequent ticks continue draining incrementally.

47.3: Verification and artifact sync (1 task)
- Re-run focused preload/responsiveness tests and update SPEC-2 artifacts with the incremental drain contract.

### Phase 48: Keep Terminal Sessions Immediate And Self-Cleaning (8 tasks)
Close the remaining workspace-shell regressions where PTY output feels one tick late, management toggles leave session geometry stale, and exited sessions stay behind as dead tabs.

48.1: PTY latency and geometry contract (3 tasks)
- Drain PTY output before the event loop blocks on the next crossterm poll so interactive typing does not wait for the tick cadence.
- On `Ctrl+G,g`, recompute the visible session content area immediately and resize every live PTY and vt100 parser in the same update.
- Keep this geometry contract aligned with the terminal scrollbar gutter so the PTY width matches the actual text viewport.

48.2: Session exit cleanup contract (2 tasks)
- Treat PTY exit detection as session cleanup: remove exited tabs automatically instead of only posting a notification.
- Clamp the active session index and terminal focus after automatic cleanup so the workspace shell never points at a dead session.

48.3: Verification (3 tasks)
- Add RED coverage for pre-poll PTY draining, immediate management-toggle resize, and automatic PTY-exit tab removal.
- Re-run focused session/event-loop tests after the implementation turns green.
- Refresh SPEC-2 artifacts and progress evidence with the new terminal responsiveness and auto-cleanup contract.

### Phase 49: Add Prefixed Focus Escape From Session Panes (5 tasks)
Close the remaining workspace-shell usability gap where session PTYs rightfully keep raw `Tab`, but users still need an explicit `Ctrl+G`-based route to cycle focus out of Shell/Agent panes.

49.1: Prefix focus contract (2 tasks)
- Teach the `Ctrl+G` keybind registry to treat `Tab` / `Shift+Tab` as explicit focus-cycle commands.
- Apply those commands at the app layer so hidden management chrome becomes visible before focus moves, matching the existing pane topology.

49.2: Verification and artifact sync (3 tasks)
- Add RED coverage for prefixed forward and reverse focus escape from session panes.
- Re-run focused keybind/workspace-shell verification after the implementation turns green.
- Refresh SPEC-2 artifacts and progress evidence with the prefixed focus-escape contract.

### Phase 50: Make Agent Session Titles Branch-First And Color-Stable (5 tasks)
Make the right-side agent tabs identify workstreams by branch name instead of by agent name, while
keeping agent identity visible through fixed colors that do not collapse when Claude Code is active.

50.1: Session-title contract (2 tasks)
- Agent session titles prefer persisted branch names over agent display names in both full-width and compact title chrome, while shell tabs keep their existing session-name labels.
- Claude Code, Codex, and Gemini titles keep fixed Yellow/Cyan/Magenta identity colors, and active-state emphasis uses modifiers instead of a shared active-only foreground color.

50.2: Focused coverage (2 tasks)
- Add RED coverage for full-width and compact agent titles showing persisted branch names with `n/N` context preserved in compact mode.
- Add RED coverage for identity-color stability plus modifier-based active emphasis, and refresh launch/conversion expectations to the new agent color contract.

50.3: Verification (1 task)
- Re-run focused session-title / launch-color tests, broad workspace verification, and refresh SPEC-2 artifacts and progress tracking.

### Phase 51: Branch Cleanup Multi-Select Flow (FR-018, US-8)

Port the merged-branch bulk cleanup workflow that lived in the old TUI/GUI `CleanupModal` (`crates/gwt-tauri/src/commands/cleanup.rs`, `gwt-gui/src/lib/components/CleanupModal.svelte`) into the rewritten `gwt-tui` Branches surface, and remove the old per-branch `Ctrl+C` delete-worktree shortcut that has been carrying single-branch deletion in the meantime.

51.1: gwt-git layer
- Add `is_protected_branch`, `is_branch_merged_into` (`git cherry`-based), `detect_cleanable_target` (multi-base + gone), `list_gone_branches`, `delete_local_branch`, `WorktreeManager::remove_force`, and `WorktreeManager::cleanup_branch` (worktree force-remove + branch force-delete, idempotent against missing artifacts). Test-first against tempdir bare repo + worktree to cover squash merge, rebase merge, true merge, gone-upstream, and protected names.

51.2: gwt-tui state
- Extend `BranchesState` with `selected: HashSet<String>`, `merged_state: HashMap<String, MergeState>`, `cleanup_settings: CleanupSettings`, and `cleanup_run: Option<CleanupRunState>`. RED: selection toggle is a no-op for protected/computing/unmerged branches, `select_all_visible_cleanable` only picks `Cleanable` rows, selection persists across `set_view_mode`/`set_sort`/`set_search`/`set_active_tab` and clears only on cleanup completion.

51.3: Modals
- Add `screens/cleanup_confirm.rs` (lists selected branches with merge target, `r` toggles `Also delete remote`, `Enter` confirms, `Esc` cancels) and `screens/cleanup_progress.rs` (determinate progress bar plus per-branch outcome list, blocks all input while `phase == Running`, accepts `Enter`/`Esc` only after `phase == Done`).

51.4: app.rs integration
- Wire `Space` → `BranchesMessage::ToggleCleanupSelection`, `Shift+C` → `BranchesMessage::OpenCleanupConfirm`, `a` → `BranchesMessage::SelectAllCleanable` on the Branches list. Remove the `Ctrl+C` → `BranchesMessage::DeleteWorktree` route on both `route_key_to_branch_list` and `route_key_to_branch_detail`, drop `pending_delete_worktree` plumbing, and update the `confirm` consumer. Wire merge-state preload events into `merged_state`, run cleanup as a `spawn_blocking` job that emits `CleanupProgress` / `CleanupCompleted` messages, and update the footer hints (`Space:select(N) Shift+C:cleanup a:all Esc:clear`).

51.5: Verification
- `cargo test -p gwt-core -p gwt-git -p gwt-tui`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt`
- Manual: in a repo with merged + unmerged + protected + gone branches, open Branches, wait for `✔` markers, multi-select with `Space`/`a`, run `Shift+C`, confirm, observe progress modal, dismiss, verify selection cleared and list refreshed.

## Dependencies

- SPEC-3 (Agent Management): Agent detection for agent launch action
- SPEC-4 (GitHub): Git status and PR data for detail sections
- SPEC-10 (Workspace): Worktree management for delete action

## Verification

1. `cargo test -p gwt-tui` — all pass
2. `cargo test -p gwt-tui --test snapshot_e2e` — all E2E pass
3. `cargo clippy -p gwt-tui --all-targets -- -D warnings` — clean
4. Manual: launch gwt-tui, navigate branches, verify cached detail switches immediately on cursor move and `r` refresh repopulates details asynchronously
5. Manual: verify `Shift+Tab` moves focus backward and the initial right-side session pane starts at the expected size without requiring a window resize
6. Manual: in a repository with many branches, open Branches during preload and verify `Up`/`Down` remains responsive while detail rows continue to backfill.
