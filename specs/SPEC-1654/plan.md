# Plan

## Summary

Rewrite `#1654` as the canonical shell and execution-session spec. Replace Sidebar with flat top-level tabs centered on `Agent Canvas` and `Branch Browser`, while consuming ref/worktree truth from `#1644`.

## Technical Context

- Shell orchestration: `gwt-gui/src/App.svelte`
- Existing shell persistence: `gwt-gui/src/lib/agentTabsPersistence.ts`, `gwt-gui/src/lib/windowSessions.ts`
- Existing multi-window restore: `gwt-gui/src/lib/windowSessionRestore.ts`, `gwt-gui/src/lib/windowSessionRestoreLeader.ts`
- Existing worktree surfaces to retire/migrate: `gwt-gui/src/lib/components/Sidebar.svelte`, `WorktreeSummaryPanel.svelte`
- Existing runtime surfaces to reuse: agent launch flow, `spawn_shell`, terminal panes, window-local backend state

## Constitution Check

- Define shell/domain boundaries before implementation
- Add shell migration tests before removing Sidebar assumptions
- Reuse existing terminal/agent runtime and multi-window restore substrate
- Record the accepted migration complexity rather than hiding it in UI-only helpers

## Project Structure

- `App.svelte` orchestrates flat tabs, Agent Canvas, Branch Browser, and popup surfaces
- Canvas components own tiles, viewport, selection, and relation edges
- Branch Browser consumes `#1644` projections and does not own branch truth
- Window-local restore persists shell/canvas state instead of split tab groups

## Complexity Tracking

- **Accepted**: explicit shell/domain split between `#1654` and `#1644` so branch truth is not duplicated
- **Accepted**: visible-tile-only terminal mounting because fully live canvas terminals would be disproportionately expensive
- **Accepted**: migration from old split/agent-tab persistence into flat tabs + canvas/browser state
- **Rejected**: keeping Sidebar or split tab layout as parallel shell surfaces

## Phased Implementation

1. Rewrite artifacts and retire split-tab ownership
2. Replace shell topology with flat tabs + Agent Canvas + Branch Browser
3. Add canvas tiles, viewport, popup, and relation edges
4. Integrate Branch Browser against `#1644`
5. Migrate persistence/restore and remove Sidebar assumptions
6. Validate multi-window shell restore and window navigation shortcuts
