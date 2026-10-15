## Workflows

### Discover Mode

1. Run JSON operation `pane.list` to enumerate active panes.
2. Present the list with pane IDs, agent types, branches, and statuses.

### Read Mode

1. Run JSON operation `pane.list` first if the pane ID is not already known.
2. Run JSON operation `pane.read` to inspect the pane's recent output.
3. Analyze the output to determine agent progress or status.

### Coordination Mode

1. Use JSON operation `pane.list` or recent Board context to identify the target session,
   branch, or agent ID.
2. Post to Board with JSON operation `board.post`.
3. Use `params.targets` for a specific recipient; omit it for repo-wide coordination.
4. Use `params.parent` when replying to an existing Board thread.

### Lifecycle Mode

1. Run JSON operation `pane.list` to identify the target pane if not already known.
2. Run JSON operation `pane.read` to confirm the pane is stuck or needs escalation.
3. Run JSON operation `pane.close` to stop the pane with an explicit reason.

## Notes

- Discover panes when the target is unknown; confirm current state before closing.
- Read pane output before posting follow-up requests that depend on pane state.
- Prefer targeted Board posts over untargeted posts for deterministic handoff.
- Only close panes when escalation is needed or the agent is unresponsive.
- Always confirm pane state before closing.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
- `GWT_PANE_ID`: pane ID of the current pane.
- `GWT_BIN_PATH`: absolute path to the current `gwtd` binary injected by gwt launches when available.
