# Data Model: SPEC-2 - Workspace Shell

## Primary Entities
### WorkspaceModel
- Role: Top-level shell state for tabs, splits, and focused pane identity.
- Invariant: Only one pane can own keyboard focus at a time.

### ManagementTab
- Role: Enumerates non-terminal panels shown inside the management area.
- Invariant: The documented tab set must match the routed tab set in code.

### PrefixKeyState
- Role: Tracks multi-stroke shortcuts such as `Ctrl+G` sequences.
- Invariant: Prefix handling must be reversible and timeout-safe.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
