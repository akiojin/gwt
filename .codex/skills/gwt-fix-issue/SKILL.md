---
name: gwt-fix-issue
description: "Use when the user wants to resolve an existing GitHub Issue by number or URL, especially when the workflow should continue through a direct fix unless a SPEC is needed."
---

# gwt-fix-issue

Public task entrypoint for resolving an existing GitHub Issue.

## When to use

- The Issue number or URL is already known
- The user says "fix issue #N", "resolve issue", or similar
- The user wants an issue-first workflow without choosing between issue/spec/build internals

Do not use this for new work intake. Use `gwt-register-issue` instead.

## Ownership

- Gather the issue facts and related code context
- Decide direct-fix vs SPEC-needed
- Carry direct-fix work through implementation and verification
- Hand off to the visible SPEC flow only when broader design work is required

## Workflow

1. Verify that `gwt issue view` and `gwt issue comments` can access the target issue.
2. Gather facts with `gwt issue view`, `gwt issue comments`, linked PR inspection, and `python3 ".codex/skills/gwt-fix-issue/scripts/inspect_issue.py" --repo "." --issue "<number or URL>"`.
3. Decide direct-fix vs spec-needed. Prefer direct fix for clear corrective work; prefer `gwt-discussion` when behavior design or broader scope definition is required.
4. If the change is a clear direct fix, continue through `gwt-build-spec` and finish verification.
5. Post progress and closure comments through `gwt issue comment`.
6. Return the result in the current user's language.

## Guardrails

- Agent-facing Issue workflow must use `gwt issue ...` as the canonical CLI surface.
- Direct `gh issue ...` commands are not part of the normal path.
- Do not treat the issue body as the source of truth for SPEC artifacts once a SPEC owner exists.
