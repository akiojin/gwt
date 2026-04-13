---
name: gwt-design-spec
description: "Use when the user wants to create, deepen, or update a SPEC and the work needs behavior or scope definition before implementation."
---

# gwt-design-spec

Public task entrypoint for SPEC design.

## When to use

- The work needs a new SPEC or changes to an existing owner SPEC
- The user is ready to move from discussion into specification
- A visible workflow should own the SPEC boundary instead of a generic issue skill

Do not use this for open-ended discussion. Use `gwt-spec-brainstorm` first when the direction is still fluid.

## Workflow

1. Load `.claude/skills/gwt-spec-design/SKILL.md` and follow it as the design engine.
2. Reuse an existing owner SPEC when the search finds one.
3. Create or deepen the SPEC until it is planning-ready.
4. Hand off to `gwt-plan-spec` when implementation planning should begin.

## Compatibility

- Public replacement for the visible design role previously exposed as `gwt-spec-design`
- `gwt-spec-design` remains available as a compatibility alias for internal handoffs and older prompts
