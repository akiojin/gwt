---
name: gwt-register-issue
description: "Use when the user wants to register new work from a bug report, idea, or task description and an existing GitHub Issue number is not already known."
---

# gwt-register-issue

Public task entrypoint for registering new work.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

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

1. Verify that `gwtd issue ...` can access GitHub. If auth fails, stop and ask the user to refresh GitHub authentication.
2. Normalize the request into summary, background, expected outcome, and constraints.
3. Run duplicate search before creating anything. Search both open Issues and existing SPEC owners, using the visible `gwt-search` surface when possible.
4. Decide the registration outcome with the `Spec Status` contract below before creating any new owner.
5. When the request is a narrow bug, docs, chore, or investigation item and the `Spec Status` allows a plain Issue, create it with `gwtd issue create --title ... -f ...`.
6. When the request reveals a missing or unclear owner SPEC, stop plain-Issue creation and hand off to `gwt-discussion`.
7. Return the chosen owner and next step in the current user's language.

## Spec Status

Classify every intake with one of these values before deciding the owner:

- `ALIGNED`: The request is already well-defined and no spec design work is needed. A plain Issue is allowed when the work is narrow.
- `IMPLEMENTATION-GAP`: Existing behavior or an owner SPEC already defines the expected outcome, but implementation is missing, broken, or incomplete. A plain Issue is allowed.
- `SPEC-GAP`: The expected behavior is not specified well enough. Do not create a plain Issue. Route to `gwt-discussion` and update the owner SPEC first.
- `SPEC-AMBIGUOUS`: Existing SPECs or issue history conflict, overlap, or leave the decision unclear. Do not create a plain Issue. Route to `gwt-discussion` and resolve the owner SPEC path first.

If duplicate search finds an existing owner SPEC, treat that SPEC as the decision anchor. Do not create a second owner for `SPEC-GAP` or `SPEC-AMBIGUOUS`.

## Guardrails

- Agent-facing Issue workflow must use `gwtd issue ...` as the canonical CLI surface.
- Direct `gh issue ...` commands are not part of the normal path.
- Do not create both a plain Issue and a SPEC for the same request.
- Plain Issue creation is valid only for `ALIGNED` or `IMPLEMENTATION-GAP`.
- `SPEC-GAP` and `SPEC-AMBIGUOUS` are stop rules that route back to `gwt-discussion`.

## Required Plain Issue Body Structure

```markdown
## Summary

(one-paragraph request summary)

## Background

(source context, problem, or motivation)

## Spec Status

(`ALIGNED`, `IMPLEMENTATION-GAP`, `SPEC-GAP`, or `SPEC-AMBIGUOUS` plus one-line rationale)

## Related SPECs

(- SPEC-1234, or `- None` when no owner SPEC applies)

## Expected Outcome

(expected result or completion condition)

## Notes

(links, examples, constraints, or open observations)
```
