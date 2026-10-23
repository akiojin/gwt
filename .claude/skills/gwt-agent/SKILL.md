---
name: gwt-agent
description: Use when inspecting or controlling gwt agent panes or the Issue Monitor queue.
allowed-tools: Bash, Read
argument-hint: "[list | <pane-id> [--lines N] | stop <pane-id>]"
---

## When to use

Inspect pane state, manage its lifecycle, or operate the Issue Monitor queue. Coordinate agents through the shared Board.

## Completion

Report the observed pane/queue state and the action taken. Confirm the exact target before lifecycle changes; keep question handoffs and launch authority within the canonical operations.

## References

- When selecting a pane mode and resolving gwtd: [routing](references/routing.md).
- For pane discovery, reads, and lifecycle commands: [pane operations](references/pane-operations.md).
- For queue ordering, project-store checks, and parked questions: [issue monitor](references/issue-monitor.md).
- For mode-specific examples and environment details: [workflows](references/workflows.md).
- For the constraint review: [router audit](references/router-audit.md).
