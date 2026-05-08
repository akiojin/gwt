---
name: gwt-agent
description: "Use proactively when monitoring or controlling running agent panes. Auto-detects mode: no args lists panes, pane ID reads output, stop/close stops a pane. For agent-to-agent communication, use the shared Board. Triggers: 'list panes', 'check agent', 'stop agent'."
allowed-tools: Bash, Read
argument-hint: "[list | <pane-id> [--lines N] | stop <pane-id>]"
---

# gwt Agent

Unified agent pane management: discover, read, and lifecycle operations.

Use the Board for agent-to-agent communication:

```bash
gwtd board post --kind request --target <session-id|branch|agent-id> --body '<message>'
gwtd board post --kind handoff --target <session-id|branch|agent-id> --body '<message>'
```

Use `--kind request`, `next`, `blocked`, `handoff`, or `decision` for
coordination. `--target` highlights the entry as `[for-you]` in the recipient's
Board reminder injection. Omit `--target` only for repo-wide Board updates.

Direct pane input is not part of the normal communication path. Prefer Board
posts so requests, decisions, blockers, and handoffs remain visible to every
agent and to the Workspace projection.

## gwtd Resolution

Resolve the `gwtd` executable once before running pane commands:

```bash
GWT_BIN="${GWT_BIN_PATH:-$(command -v gwtd || true)}"
if [ -z "$GWT_BIN" ] && [ -n "${GWT_PROJECT_ROOT:-}" ] && [ -x "$GWT_PROJECT_ROOT/target/debug/gwtd" ]; then
  GWT_BIN="$GWT_PROJECT_ROOT/target/debug/gwtd"
fi
if [ -z "$GWT_BIN" ] && [ -x "./target/debug/gwtd" ]; then
  GWT_BIN="./target/debug/gwtd"
fi
```

If `GWT_BIN` is empty, stop and report that `gwtd` could not be found.

## Mode Detection

Auto-detect the operation mode from arguments:

| Arguments | Mode | Operation |
|---|---|---|
| *(none)* or `list` | **Discover** | List active panes with IDs, agent types, branches, and statuses |
| `<pane-id>` | **Read** | Read the last 50 lines of the pane's scrollback |
| `<pane-id> --lines N` | **Read** | Read the last N lines of the pane's scrollback |
| `stop <pane-id>` or `close <pane-id>` | **Lifecycle** | Stop and close the specified pane |

## Commands

### Discover

- `"$GWT_BIN" pane list` (`gwtd pane list`): list active pane IDs with agent type, branch, and status.

### Read

- `"$GWT_BIN" pane read <id> [--lines N]` (`gwtd pane read <id> [--lines N]`): read the last N lines (default 50) of the specified pane's scrollback.

### Coordinate

- `gwtd board post --kind request --target <id> --body <message>`: ask a
  specific agent to act or respond.
- `gwtd board post --kind handoff --target <id> --body <message>`: hand off
  context or next ownership.
- `gwtd board post --kind blocked --body <message>`: expose a blocker and ask
  for unblock help.

### Lifecycle

- `"$GWT_BIN" pane close <id>` (`gwtd pane close <id>`): stop the specified pane.

## Workflows

### Discover Mode

1. Run `"$GWT_BIN" pane list` to enumerate active panes.
2. Present the list with pane IDs, agent types, branches, and statuses.

### Read Mode

1. Run `"$GWT_BIN" pane list` first if the pane ID is not already known.
2. Run `"$GWT_BIN" pane read <id>` to inspect the pane's recent output.
3. Analyze the output to determine agent progress or status.

### Coordination Mode

1. Use `"$GWT_BIN" pane list` or recent Board context to identify the target session,
   branch, or agent ID.
2. Post to Board with `gwtd board post`.
3. Use `--target` for a specific recipient; omit it for repo-wide coordination.
4. Use `--parent` when replying to an existing Board thread.

### Lifecycle Mode

1. Run `"$GWT_BIN" pane list` to identify the target pane if not already known.
2. Run `"$GWT_BIN" pane read <id>` to confirm the pane is stuck or needs escalation.
3. Run `"$GWT_BIN" pane close <id>` to stop the pane with an explicit reason.

## Notes

- Always discover panes before reading or closing.
- Read pane output before posting follow-up requests that depend on pane state.
- Prefer targeted Board posts over untargeted posts for deterministic handoff.
- Only close panes when escalation is needed or the agent is unresponsive.
- Always confirm pane state before closing.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
- `GWT_PANE_ID`: pane ID of the current pane.
- `GWT_BIN_PATH`: absolute path to the current `gwtd` binary injected by gwt launches when available.
