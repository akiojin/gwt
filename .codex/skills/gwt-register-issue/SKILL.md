---
name: gwt-register-issue
description: "Use when the user wants to register new work from a bug report, idea, or task description and an existing GitHub Issue number is not already known."
---

# gwt-register-issue

Public task entrypoint for registering new work.

## When to use

- The user has a bug report, enhancement idea, docs task, or rough request
- No existing Issue number or URL is provided
- The user wants a clear intake path before deciding plain Issue vs SPEC

Do not use this for an existing Issue. Use `gwt-fix-issue` instead.

## Ownership

- Normalize the request and search for duplicates
- Decide plain Issue vs SPEC
- Create the plain Issue when the work is narrow enough
- Hand off to the visible SPEC flow when the work needs design first

## Workflow

1. Load `.claude/skills/gwt-issue/SKILL.md` and follow **Register Mode** only.
2. Run duplicate search before creating anything.
3. If the request is a narrow bug/docs/chore/investigation item, create the plain Issue through `gwt issue create`.
4. If the request needs new behavior definition, create or deepen the owner SPEC through `gwt-discussion`.
5. Return the chosen owner and next step without exposing internal routing names.

## Compatibility

- Public replacement for `gwt-issue` register mode
- `gwt-issue` remains available as a compatibility alias for older prompts and commands
