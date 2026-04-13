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

1. Verify that `gwt issue ...` can access GitHub. If auth fails, stop and ask the user to refresh GitHub authentication.
2. Normalize the request into summary, background, expected outcome, and constraints.
3. Run duplicate search before creating anything. Search both open Issues and existing SPEC owners.
4. When the request is a narrow bug, docs, chore, or investigation item, create a plain Issue with `gwt issue create --title ... -f ...`.
5. When the request needs new behavior definition or broader design work, hand off to `gwt-discussion`.
6. Return the chosen owner and next step in the current user's language.

## Guardrails

- Agent-facing Issue workflow must use `gwt issue ...` as the canonical CLI surface.
- Direct `gh issue ...` commands are not part of the normal path.
- Do not create both a plain Issue and a SPEC for the same request.

## Required Plain Issue Body Structure

```markdown
## Summary

(one-paragraph request summary)

## Background

(source context, problem, or motivation)

## Expected Outcome

(expected result or completion condition)

## Notes

(links, examples, constraints, or open observations)
```
