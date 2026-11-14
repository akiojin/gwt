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
- The user wants a clear intake path before deciding plain Issue vs
  design-required registration

Do not use this for an existing Issue. Use `gwt-execute #N` instead.

## Ownership

- Normalize the request and search for duplicates
- Decide plain Issue vs design-required Work Item
- Create the plain Issue when the work is narrow enough
- Create a design-required Work Item when discussion has produced complete
  artifacts, applying the `gwt-spec` design-required tag and section body safely

## Workflow

1. Verify that a simple `issue.*` JSON operation can access GitHub. If auth fails, stop and ask the user to refresh GitHub authentication.
2. Normalize the request into summary, background, expected outcome, and constraints.
3. Run duplicate search before creating anything. Search both open Issues and existing SPEC owners, using the `gwt-search` skill (a skill, not a PATH command) when possible.
4. Decide the registration outcome with the `Spec Status` contract below before creating any new owner.
5. When the request is a narrow bug, docs, chore, or investigation item and the `Spec Status` allows a plain Issue, create it with JSON operation `issue.create`.
6. When the request needs design first, stop plain-Issue creation and hand off to `gwt-discussion` until the design is complete.
7. When `gwt-discussion` returns a Register Spec action bundle with a title and body file, create the design-required Work Item from this skill:
   - validate the body against the canonical SPEC sections
   - call JSON operation `issue.spec.create`
   - inject the `spec` body with JSON operation `issue.spec.edit`
   - perform a roundtrip read with JSON operation `issue.spec.section`
   - return the Issue number and next step
8. Return the chosen owner and next step in the current user's language.

## Spec Status

Classify every intake with one of these values before deciding the owner:

- `ALIGNED`: The request is already well-defined and no spec design work is needed. A plain Issue is allowed when the work is narrow.
- `IMPLEMENTATION-GAP`: Existing behavior or an owner SPEC already defines the expected outcome, but implementation is missing, broken, or incomplete. A plain Issue is allowed.
- `SPEC-GAP`: The expected behavior is not specified well enough. Do not create a plain Issue. Route to `gwt-discussion`; once design is complete, create a design-required Work Item through the safe `issue.spec.create` -> `issue.spec.edit` -> roundtrip flow.
- `SPEC-AMBIGUOUS`: Existing SPECs or issue history conflict, overlap, or leave the decision unclear. Do not create a plain Issue. Route to `gwt-discussion`; once ownership is resolved, either update the existing owner or create a design-required Work Item through the safe flow.

If duplicate search finds an existing owner SPEC, treat that SPEC as the decision anchor. Do not create a second owner for `SPEC-GAP` or `SPEC-AMBIGUOUS`.

## Guardrails

- Agent-facing Issue workflow must use gwtd JSON operations `issue.*` as the canonical surface.
- Direct `gh issue ...` commands are not part of the normal path.
- Do not create both a plain Issue and a design-required Work Item for the same request.
- Plain Issue creation is valid only for `ALIGNED` or `IMPLEMENTATION-GAP`.
- `SPEC-GAP` and `SPEC-AMBIGUOUS` are stop rules until `gwt-discussion` produces a complete design or existing owner update path.
- Design-required registration must apply the `gwt-spec` design-required tag and use `issue.spec.create`, `issue.spec.edit`, and a roundtrip `issue.spec.section` read.

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
