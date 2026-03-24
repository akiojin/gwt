# Data Model

## Top-level shell
- `WorkspaceTabId`
  - `agentCanvas`
  - `branchBrowser`
  - `settings`
  - `issues`
  - `prs`
  - `versionHistory`
  - `projectIndex`
  - `issueSpec`
- Each top-level shell tab is a full-window surface. The shell does not persist or render side-by-side detail panes as canonical layout state.

## Agent Canvas
- `AgentCanvasViewport`: `x`, `y`, `zoom`
- `AgentCanvasTile`: `id`, `type`, geometry, optional `worktreeId`, `paneId`, `branchRef`, `title`
- `AgentCanvasEdge`: `id`, `sourceTileId`, `targetTileId`, `kind: worktree-session`
- `AgentCanvasState`: `viewport`, `tiles[]`, `edges[]`, `selectedTileId?`, `openPopupTileId?`
- `AgentCanvas` is a single full-window board surface; tile details may use popup/overlay state but not persistent right-pane layout.

## Branch Browser shell projection
- `BranchBrowserShellState`: `mode: local | remote | all`, `query`, `selectedRefId?`
- `Branch Browser` is a single full-window surface; selected-ref detail is rendered inline or via popup/overlay, not via persistent side pane.

## Persistence
- `StoredWorkspaceShellState`: `activeTabId`, `agentCanvas`, `branchBrowser`
- Persistence remains keyed by project path + window label
- Legacy split metadata is ignored and pruned from the canonical shell path
