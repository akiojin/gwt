---
name: gwt-agent-send
description: "Send key input to a specific agent pane or broadcast to all panes. Use when user says 'send to pane', 'dispatch to agent', 'broadcast instructions', or when dispatching tasks to agents."
---

# gwt Agent Send

Send key input to agent panes.

## Commands

- `pane send <id> <input>`: send key input to a specific pane.
- `pane broadcast <input>`: send key input to all active panes.

## Workflow

1. Run `pane list` (via `gwt-agent-discover`) to identify target panes.
2. Run `pane read <id>` (via `gwt-agent-read`) to confirm the pane is ready for input.
3. Run `pane send <id> <input>` for targeted dispatch.
4. Use `pane broadcast <input>` only when all panes need the same instruction.

## Notes

- Prefer targeted `pane send` over `pane broadcast` for deterministic dispatch.
- Always check pane state with `gwt-agent-read` before sending follow-up instructions.

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root. Pane commands are scoped to the caller's project; panes belonging to other projects are not visible or accessible.
- `GWT_PANE_ID`: pane ID of the current pane. Use to exclude self from broadcast targets.
