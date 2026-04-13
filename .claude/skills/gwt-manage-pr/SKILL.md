---
name: gwt-manage-pr
description: "Use when the user wants to create, inspect, update, or unblock a pull request and expects one visible entrypoint for the PR lifecycle."
---

# gwt-manage-pr

Public task entrypoint for PR lifecycle work.

## When to use

- The user asks to create a PR
- The user wants current PR status
- CI, mergeability, or review blockers need to be handled

## Workflow

1. Load `.claude/skills/gwt-pr/SKILL.md` and follow it as the PR engine.
2. Keep creation, status checks, push-only updates, and blocker fixes behind this visible entrypoint.
3. Return the concrete next PR action in the current user's language.

## Compatibility

- Public replacement for the visible PR role previously exposed as `gwt-pr`
- `gwt-pr` remains available as a compatibility alias for internal handoffs and older prompts
