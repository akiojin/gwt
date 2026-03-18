---
name: gwt-issue-register
description: Register new GitHub work items from a request. Search existing Issues and `gwt-spec` Issues first, stop on duplicates, then either create a plain GitHub Issue or hand off to `gwt-spec-register` for a new SPEC. Use as the main entrypoint for new Issue/SPEC registration requests.
---

# gwt Issue Register

Use this skill when the user wants to file, register, or draft new work from a bug report,
feature request, enhancement idea, documentation task, or rough note.

`gwt-issue-register` is the main entrypoint for new work registration.

- If the user already has an Issue number or URL, use `gwt-issue-resolve`.
- If the target `gwt-spec` issue is already known, use `gwt-spec-ops`.
- If this skill determines that a new SPEC is required, hand off to `gwt-spec-register`,
  then continue through `gwt-spec-clarify` -> `gwt-spec-plan` -> `gwt-spec-tasks` ->
  `gwt-spec-analyze` -> `gwt-spec-ops`.
- Do not create both a plain Issue and a SPEC for the same request.

## Mandatory preflight: search existing issues first

Before creating any new Issue or SPEC, use `gwt-issue-search` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call.
2. Update the Issues index if needed.
3. Run at least 2 semantic Issue queries derived from the request.
4. Search for both plain Issues and `gwt-spec` Issues.
5. Stop new creation when a clear existing destination or duplicate already exists.

Do not skip duplicate search because the request "sounds new".

## Duplicate handling

- If an open plain Issue clearly matches, stop registration and switch to `gwt-issue-resolve`.
- If an open `gwt-spec` clearly owns the scope, stop registration and switch to `gwt-spec-ops`.
- If the search returns plausible candidates but no single clear owner, stop registration and present the top 1-3 candidates instead of creating a new item.
- Do not create a fresh Issue "just in case" when the duplicate check is inconclusive.

## Plain Issue vs SPEC decision

Create a plain GitHub Issue when the request is primarily one of these:

- clear bug report or regression
- documentation task
- chore / maintenance task
- question or investigation request
- narrowly scoped enhancement that does not need new product behavior to be specified first

Create a new SPEC when the request includes any of these:

- multiple user scenarios or acceptance criteria
- new or changed product behavior that must be defined first
- UI / UX flow decisions
- cross-cutting or multi-subsystem changes
- non-trivial technical or product tradeoffs

When the need for a SPEC is clear, do not create a plain Issue first. Switch to `gwt-spec-register`.

## Title rules for plain Issues

Use Conventional Commit style titles:

- `fix: ...` for bugs and regressions
- `feat: ...` for user-visible enhancements that still stay on the plain-Issue path
- `docs: ...` for documentation work
- `chore: ...` for maintenance or operational tasks

Prefer a short imperative summary. Do not use vague titles like "some issue" or "improvement".

## Required plain Issue body structure

New plain Issues must use this section structure:

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

## Registration decision output

Before creating anything, report the decision in this structure:

````text
## Registration Decision

**Request Type:** BUG | FEATURE | ENHANCEMENT | DOCUMENTATION | CHORE | QUESTION | UNCLASSIFIED
**Duplicate Check:** CLEAN | EXISTING-ISSUE | EXISTING-SPEC | AMBIGUOUS
**Chosen Path:** ISSUE | NEW-SPEC | EXISTING
**Action:** <specific next step>

### Candidates
- #<number> <title> [issue|gwt-spec] - <why it matches>
````

Use `CLEAN` only when the duplicate search found no credible owner.

## Workflow

1. **Verify gh authentication.**
   - Run `gh auth status` in the repo.
   - If unauthenticated, stop and ask the user to log in.

2. **Normalize the request.**
   - Extract the request summary, intended outcome, relevant subsystem, and any links or prior context.
   - Classify the request type.

3. **Search for an existing destination.**
   - Use `gwt-issue-search` with at least 2 semantic queries.
   - Prefer open Issues and active canonical `gwt-spec` Issues.

4. **Stop on duplicates or existing owners.**
   - If a clear open Issue already tracks the request, do not create a new item.
   - If a clear `gwt-spec` already owns the scope, do not create a new item.
   - Report the result with `Chosen Path: EXISTING` and switch to the owning workflow.

5. **Choose plain Issue or new SPEC.**
   - Use the decision rules above.
   - Plain Issue: create directly with `gh issue create`.
   - New SPEC: switch to `gwt-spec-register` and continue through the spec artifact flow.

6. **Create the plain Issue when needed.**
   - Use the required title rule and issue body structure.
   - Fill the sections with concrete request context, not `_TODO_`.

7. **Return the created Issue or handoff target.**
   - For plain Issue creation, return the issue number and URL.
   - For existing owner or new SPEC, state the exact destination and next skill.

## Operations (gh CLI fallback)

### Create plain Issue

```bash
gh issue create --title "fix: ..." --body "$(cat <<'EOF'
## Summary

Concrete summary

## Background

Why this request exists

## Expected Outcome

What done looks like

## Notes

Links, examples, or constraints
EOF
)"
```
