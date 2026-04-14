---
name: gwt-agent
description: "Use proactively when monitoring or interacting with running agent panes. Auto-detects mode: no args lists panes, pane ID reads output, pane ID + message sends input, stop/close stops a pane. Triggers: 'list panes', 'check agent', 'send to pane', 'stop agent', 'エージェント一覧'."
allowed-tools: Bash, Read
argument-hint: "[list | <pane-id> [message] | stop <pane-id> | broadcast <message>]"
---

# gwt Agent

Unified agent pane management: discover, read, send, and lifecycle operations.

## Mode Detection

Auto-detect the operation mode from arguments:

| Arguments | Mode | Operation |
|---|---|---|
| *(none)* or `list` | **Discover** | List active panes with IDs, agent types, branches, and statuses |
| `<pane-id>` | **Read** | Read the last 50 lines of the pane's scrollback |
| `<pane-id> --lines N` | **Read** | Read the last N lines of the pane's scrollback |
| `<pane-id> <message>` | **Send** | Send key input to the specified pane |
| `broadcast <message>` | **Broadcast** | Send key input to all active panes |
| `stop <pane-id>` or `close <pane-id>` | **Lifecycle** | Stop and close the specified pane |

## Commands

### Discover

- `pane list`: list active pane IDs with agent type, branch, and status.

### Read

- `pane read <id> [--lines N]`: read the last N lines (default 50) of the specified pane's scrollback.

### Send

- `pane send <id> <input>`: send key input to a specific pane.
- `pane broadcast <input>`: send key input to all active panes.

### Lifecycle

- `pane close <id>`: stop the specified pane.

## Workflows

### Discover Mode

1. Run `pane list` to enumerate active panes.
2. Present the list with pane IDs, agent types, branches, and statuses.

### Read Mode

1. Run `pane list` first if the pane ID is not already known.
2. Run `pane read <id>` to inspect the pane's recent output.
3. Analyze the output to determine agent progress or status.

### Send Mode

1. Run `pane list` to identify target panes if not already known.
2. Run `pane read <id>` to confirm the pane is ready for input.
3. Run `pane send <id> <input>` for targeted dispatch.
4. Use `pane broadcast <input>` only when all panes need the same instruction.

### Lifecycle Mode

1. Run `pane list` to identify the target pane if not already known.
2. Run `pane read <id>` to confirm the pane is stuck or needs escalation.
3. Run `pane close <id>` to stop the pane with an explicit reason.

## Notes

- Always discover panes before reading, sending, or closing.
- Read pane output before sending follow-up instructions.
- Prefer targeted `pane send` over `pane broadcast` for deterministic dispatch.
- Only close panes when escalation is needed or the agent is unresponsive.
- Always confirm pane state before closing.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
- `GWT_PANE_ID`: pane ID of the current pane. Use to exclude self from broadcast targets.
