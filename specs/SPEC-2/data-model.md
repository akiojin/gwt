# Data Model: SPEC-2 - Workspace Shell

## Primary Entities
### WorkspaceModel
- Role: Top-level shell state for tabs, splits, and focused pane identity.
- Invariant: Only one pane can own keyboard focus at a time.

### ManagementTab
- Role: Enumerates non-terminal panels shown inside the management area.
- Invariant: The documented tab set must match the routed tab set in code.

### FocusPane

- Role: Identifies which pane currently owns keyboard focus.
- Variants: `TabContent`, `BranchDetail`, `Terminal` (3 panes).
- Invariant: Only one pane can own focus at a time. Ctrl+G, Tab/Shift+Tab cycles through all 3 in order.

### PrefixKeyState

- Role: Tracks multi-stroke shortcuts such as `Ctrl+G` sequences.
- Invariant: Prefix handling must be reversible and timeout-safe.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
