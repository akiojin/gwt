---
description: Dispatch instructions from Assistant to Agent panes using the gwt-agent-dispatch skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Agent Dispatch Command

Use this command to dispatch instructions from Assistant to Agent panes.

## Usage

```text
/gwt:gwt-agent-dispatch [context]
```

## Steps

1. Load `skills/gwt-agent-dispatch/SKILL.md` and follow the workflow.
2. Inspect active panes first (`list_terminals`) before sending instructions.
3. Prefer targeted routing (`send_keys_to_pane`) over broadcast when possible.
4. Confirm progress by reading pane output (`capture_scrollback_tail`).
5. Escalate or stop stuck panes with explicit reason.

## Examples

```text
/gwt:gwt-agent-dispatch AssistantからAgentへタスク配布したい
```

```text
/gwt:gwt-agent-dispatch Agentの進捗を確認
```
