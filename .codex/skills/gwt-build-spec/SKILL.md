---
name: gwt-build-spec
description: "Use when implementation should proceed from an approved SPEC or approved standalone task, and the work should run through the build/test/verify loop."
---

# gwt-build-spec

Public task entrypoint for implementation.

## When to use

- A SPEC is approved and ready for implementation
- A standalone implementation task is approved after discussion
- The user wants the visible implementation entrypoint rather than the internal build engine name

If the request starts from an existing GitHub Issue number or URL, prefer `gwt-fix-issue`.

## Workflow

1. Load `.claude/skills/gwt-spec-build/SKILL.md` and follow it as the implementation engine.
2. Prefer SPEC mode when a SPEC exists; use standalone mode only when the task was explicitly approved without a SPEC.
3. Keep PR flow user-facing through `gwt-manage-pr`.
4. Route back to `gwt-design-spec` if implementation reveals a real spec gap.

## Compatibility

- Public replacement for the visible implementation role previously exposed as `gwt-spec-build`
- `gwt-spec-build` remains available as a compatibility alias for internal handoffs and older prompts
