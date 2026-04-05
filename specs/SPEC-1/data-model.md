# Data Model: SPEC-1 - Terminal Emulation

## Primary Entities
### TerminalSurface
- Role: Owns the visible vt100 screen plus scroll position for one PTY view.
- Invariant: Rendering and hit-testing must read the same snapshot.

### SelectionRange
- Role: Represents copied text bounds across wrapped terminal rows.
- Invariant: Selection must stay stable while the viewport moves.

### UrlHitTarget
- Role: Captures detected URL bounds for underline and open actions.
- Invariant: Underline and click-open must resolve the same range.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
