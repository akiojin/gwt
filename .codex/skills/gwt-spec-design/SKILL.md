---
name: gwt-spec-design
description: "Use when a legacy prompt or internal handoff refers to gwt-spec-design. Prefer gwt-discussion as the visible discussion and design entrypoint."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[topic | concern | --deepen SPEC-N]"
---

# gwt-spec-design

Legacy compatibility alias for design-oriented prompts.

Visible owner: `gwt-discussion`.

## When to use

- An older prompt, command, or handoff still refers to `gwt-spec-design`
- The intent is discussion, design clarification, or SPEC shaping

Do not present this as the public entrypoint. Route users to `gwt-discussion`.

## Workflow

1. Load `.claude/skills/gwt-discussion/SKILL.md`.
2. Follow the full discussion workflow there.
3. Keep `Intake Memo`, `Discussion TODO`, `Action Delta`, and `Action Bundle`
   terminology from `gwt-discussion`.

## Compatibility

- Compatibility alias for older prompts and handoffs
- `gwt-discussion` is the canonical visible entrypoint
