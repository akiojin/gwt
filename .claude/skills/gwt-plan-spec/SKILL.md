---
name: gwt-plan-spec
description: "Use when a SPEC exists and the next task is to generate or refresh implementation planning artifacts such as plan.md and tasks.md."
---

# gwt-plan-spec

Public task entrypoint for SPEC planning.

## When to use

- `spec.md` exists and the implementation plan is missing or stale
- The user asks to generate tasks or prepare implementation
- The visible workflow should expose planning as its own step

## Workflow

1. Load `.claude/skills/gwt-spec-plan/SKILL.md` and follow it as the planning engine.
2. Generate or refresh the planning artifacts for the target SPEC.
3. Resolve any artifact-quality gate failures before handing off.
4. Hand off to `gwt-build-spec` when the artifact set is clear for implementation.

## Compatibility

- Public replacement for the visible planning role previously exposed as `gwt-spec-plan`
- `gwt-spec-plan` remains available as a compatibility alias for internal handoffs and older prompts
