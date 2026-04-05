# Analysis: SPEC-2 - Workspace shell — tabs, split grid, management panel, keybindings

## Analysis Report: SPEC-2

Status: CLEAR

## Blocking Items
- None.

## Checks
- Clarification completeness: no `[NEEDS CLARIFICATION]` markers remain in `spec.md`.
- Artifact completeness: `spec.md`, `plan.md`, `tasks.md`, supporting docs, `checklists/*`, `progress.md`, and `analysis.md` are present.
- Task traceability snapshot: `tasks.md` currently records `253/253` completed items after closing Phase 41 for the split-grid session-title identity follow-up.
- Notes: Core and supporting artifacts are present and internally usable for further work.
- Notes: Help overlay is now reachable from `Ctrl+G,?`, grouped by category, and backed by the keybinding registry.
- Notes: Git View is now backed by live repository status and recent-commit loading.
- Notes: Session persistence now restores layout, panel visibility, and active management tab on startup.
- Notes: `T077` was retired after confirming that the referenced `simplify` skill is not exposed in the current session tool list or repository skill set.
- Notes: Phase 8 closed the remaining branch-first UX gaps by restoring inline branch rows, primary Branches actions, and Branches list focus when returning via `Ctrl+G,b`.
- Notes: Phase 9 restored the old-TUI branch-local mnemonic set on Branches without reopening layout or session-summary work.
- Notes: Phase 10 closed the remaining branch-detail UX gap by replacing the count-only `Sessions` pane with branch-scoped shell/agent summaries built at render time from live tabs and persisted agent-session metadata.
- Notes: Phase 11 closed the next branch-first UX gap by making the `Sessions` pane actionable: `Up/Down` selects branch-scoped session rows and `Enter` focuses the selected running session in the terminal pane.
- Notes: Phase 12 closed the footer UX gap by restoring the shared status-bar widget in the main view; the bottom line now carries session/branch/agent context, notifications, and focus-specific hints together.
- Notes: Phase 13 closed the next Branch Detail affordance gap by restoring `Shift+Enter` and `Ctrl+C` as direct branch actions outside the `Sessions` section and by making Branch Detail footer hints section-sensitive.
- Notes: Phase 14 closed the remaining Branch Detail chrome gap in this area by keeping the selected branch name visible in the bottom-pane title next to the section tabs, with a graceful fallback when no branch is selected.
- Notes: Phase 15 closed the Branch Detail focus-loop gap by making the advertised `Esc:back` affordance real; `Esc` now returns focus to the Branches list without clearing the current detail context.
- Notes: Phase 16 closed the remaining pane-focus color drift by restoring `pane_block()` to the documented `Color::Cyan` / `Color::Gray` border contract.
- Notes: Phase 17 aligned Branch Detail affordances with actual reachability; shell/delete direct actions are now offered only for selected branches that have a worktree.
- Notes: Phase 18 restored a more sensible old-TUI-like workspace balance by moving the visible management/session split from `50/50` to `40/60` through a shared layout helper.
- Notes: Phase 19 restored the old-TUI Branches local mnemonic set inside Branch Detail as well, so `m`, `v`, `f`, and help remain available after focus moves off the list.
- Notes: Phase 21 restores the old-TUI supplemental-panel contract for `Ctrl+G,g`; showing the management panel no longer steals terminal focus, and hiding it resets focus to Terminal so Main-layer hints stay accurate.
- Notes: Phase 22 brings the global tab shortcuts (`Ctrl+G,b/i/s/...`) in line with that supplemental-panel contract so tab opening no longer steals terminal focus from the main workstream, while management-local tab switches still normalize to `TabContent`.
- Notes: Phase 23 brings `Issues` and `PR Dashboard` detail panes into parity with the documented `Esc` contract so they close back to the list instead of falling through to warn-dismiss behavior.
- Notes: Phase 24 closes the remaining `Logs` detail gap so `Esc` now closes that drill-down as well, instead of behaving inconsistently with the other management detail views.
- Notes: Phase 25 completes the management-pane side of that supplemental contract: once search/detail/edit flows are out of the way, plain `Esc` now returns focus to `Terminal`, while warn notifications still keep dismissal priority.
- Notes: Phase 26 closes the one remaining tab-specific hole in that contract: `Profiles` list mode now uses the same supplemental `Esc` fallback, while create/edit/delete flows still keep `Esc=Cancel`.
- Notes: Phase 27 aligns the visible guidance with the restored behavior: Branches-list and generic management-list status-bar hints now advertise `Esc:term` so the supplemental escape hatch is discoverable again.
- Notes: Phase 28 closes the remaining focus-topology drift inside management: `Tab` / `BackTab` now keep non-Branches tabs on the two real surfaces (`Terminal` and `TabContent`), while Branches retains the old three-surface cycle that includes `BranchDetail`.
- Notes: Phase 29 closes the remaining fixed-width layout drift by making the management split responsive again: `>=120 cols` keeps `40/60`, while narrower widths fall back to `50/50` through the same shared helper used for render-time layout and active-session geometry.
- Notes: The canonical spec/plan artifacts now state that the three-surface focus loop is Branches-only, eliminating the earlier contradiction with the Phase 28 implementation.
- Notes: Phase 30 removes the standalone management banner entirely; pane titles now carry the management context so the management pane reclaims the top row for actual list/detail content.
- Notes: Phase 20 remains in the artifact set only as an intermediate restoration step; Phase 30 supersedes it as the final management-chrome contract.
- Notes: Phase 31 introduced the wider terminal footer guidance, and Phase 32 follows immediately with the compact grouped notation needed to keep those terminal mnemonics visible at terminal widths `<= 80` when no notification is occupying the footer.
- Notes: Phase 33 extends that `width <= 80`, notification-free footer compaction to management and Branch Detail hints so the pane-local guidance also remains visible without truncation.
- Notes: Phase 34 closes the next standard-width chrome gap by collapsing management pane titles to the active tab only whenever the full tab strip would truncate, while extra-wide panes keep the original strip.
- Notes: Phase 35 mirrors that fit-based compaction on the session pane title, so standard-width multi-session workspaces now keep the active session readable instead of showing a truncated strip.
- Notes: Phase 36 closes the remaining visible-footer gap on non-Branches tabs by making the hints match each tab's real routing and mode-specific `Esc` behavior instead of advertising a generic contract everywhere.
- Notes: Phase 37 closes the remaining Branch Detail `Esc` interference bug by consuming the local back action before warn-dismiss fallback can run, while leaving the second-escape dismissal path on the list surface intact.
- Notes: Phase 38 removes the last generic `Enter:action` overclaims on non-Branches tabs, so Git View, Versions, Issues, and PR Dashboard now advertise only the real expand/detail/close/search/refresh affordances that their routing actually supports.
- Notes: Phase 39 removes the last redundant nested chrome inside Branch Detail: once the pane border already names the active section and selected branch, the inner Overview / SPECs / Git / Sessions renderers now stay title-free and let the body content start immediately.
- Notes: Phase 40 restores the last missing piece of compact session-title context by keeping the active `n/N` position visible alongside the active session label whenever the full strip collapses, while extra-wide panes continue to show the full strip.
- Notes: Phase 41 extends that session-identity parity into split/grid mode: each pane title now carries its stable `n:` shortcut position and the session-type icon instead of a plain name-only title.
