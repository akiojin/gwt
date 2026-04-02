# Data Model: SPEC-1654

## Branch Dashboard

- `BranchRowInfo`
  - `branch_name`
  - `session_count`
  - `pr_status`
  - `divergence`
  - `quick_start_available`
- `BranchEnterAction`
  - `OpenSession`
  - `OpenSelector`
  - `OpenWizard`

## Session Workspace

- `SessionRecord`
  - `session_id`
  - `pane_id`
  - `branch_name`
  - `tool_id`
  - `status`
- `SessionLayoutMode`
  - `EqualGrid`
  - `Maximized`
- `SessionWorkspaceState`
  - `records`
  - `focused_session_id`
  - `layout_mode`
  - `last_non_management_layout`

## Management Workspace

- `ManagementTab`
  - `Branches`
  - `SPECs`
  - `Issues`
  - `Profiles`
- `ManagementWorkspaceState`
  - `active_tab`
  - `last_tab`

## Persistence Boundary

- Session metadata persistence is owned by `SPEC-1648`
- Local git/worktree truth is owned by `SPEC-1644`
- Agent launch contract is owned by `SPEC-1646`
