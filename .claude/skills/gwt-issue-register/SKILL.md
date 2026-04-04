---
name: gwt-issue-register
description: "This skill should be used when the user wants to register new work, says 'register an issue', 'create a new issue', 'file a bug', 'add feature request', 'Issueを作って', 'バグを登録して', '機能要望を出して', or asks to track new work items. It searches existing Issues and SPECs first, reuses a clear existing owner when possible, otherwise creates a plain GitHub Issue or continues into the SPEC workflow."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
---

# gwt Issue Register

Use this skill when the user wants to file, register, or draft new work from a bug report,
feature request, enhancement idea, documentation task, or rough note.

`gwt-issue-register` is the main entrypoint for new work registration.

Hard routing rule:

- If the user is asking to create, file, register, or draft new work and does not already have a GitHub Issue number or URL, start with this skill.
- Do not call `gh issue create` manually and do not jump to `gwt-spec-register` before this skill completes duplicate search and ISSUE vs SPEC selection.

- If the user already has an Issue number or URL, use `gwt-issue-resolve`.
- If the target SPEC is already known, use `gwt-spec-ops`.
- If this skill determines that a new SPEC is required, create it through
  `gwt-spec-register`, then continue through `gwt-spec-ops`.
- Do not create both a plain Issue and a SPEC for the same request.
- A local SPEC directory is not a GitHub Issue. When a SPEC is needed, create or reuse the
  local SPEC as the source of truth and treat any Issue as an optional related record.

## Mandatory preflight: search existing issues first

Before creating any new Issue or SPEC, use `gwt-issue-search` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call.
2. Update the Issues index if needed.
3. Run at least 2 semantic Issue queries derived from the request.
4. Search for both plain Issues and existing SPECs (via `spec_artifact.py --repo . --list-all`).
5. Reuse a clear existing destination instead of creating a duplicate.

Do not skip duplicate search because the request "sounds new".

## Duplicate handling

- If an open plain Issue clearly matches, switch to `gwt-issue-resolve` and continue there.
- If an existing SPEC clearly owns the scope, switch to `gwt-spec-ops` and continue there.
- If the search returns plausible candidates but no single clear owner, stop and present the top 1-3 candidates instead of creating a new item.
- Do not create a fresh Issue "just in case" when the duplicate check is inconclusive.

## Plain Issue vs SPEC decision

Create a plain GitHub Issue when the request is primarily one of these:

- clear bug report or regression
- documentation task
- chore / maintenance task
- question or investigation request
- narrowly scoped enhancement that does not need new product behavior to be specified first

Create a new local SPEC directory when the request includes any of these:

- multiple user scenarios or acceptance criteria
- new or changed product behavior that must be defined first
- UI / UX flow decisions
- cross-cutting or multi-subsystem changes
- non-trivial technical or product tradeoffs

When the need for a SPEC is clear, do not create a plain Issue first. Create the SPEC through
`gwt-spec-register` and continue through `gwt-spec-ops`. Only create a plain Issue too when the
user explicitly asks for separate GitHub tracking.

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
- #<number> <title> [issue] - <why it matches>
- SPEC-<id> <title> [spec] - <why it matches>
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
   - Also search local `specs/` directory via `spec_artifact.py --repo . --list-all`.
   - Prefer open Issues and active canonical SPECs.

4. **Reuse duplicates or existing owners.**
   - If a clear open Issue already tracks the request, do not create a new item.
   - If a clear SPEC already owns the scope, do not create a new item.
   - Report the result with `Chosen Path: EXISTING` and continue with the owning workflow.

5. **Choose plain Issue or new SPEC.**
   - Use the decision rules above.
   - Plain Issue: create directly with `gh issue create`.
   - New SPEC: create through `gwt-spec-register` (local directory), then continue through `gwt-spec-ops`.

6. **Create the plain Issue when needed.**
   - Use the required title rule and issue body structure.
   - Fill the sections with concrete request context, not `_TODO_`.

7. **Return the created Issue or active owner.**
   - For plain Issue creation, return the issue number and URL.
   - For existing owner or new SPEC, state the exact destination and continue with that workflow unless an ambiguous product decision still blocks progress.

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

If `gh issue create` is rate-limited (`was submitted too quickly` / secondary rate limit), resolve the repo slug with `gh repo view --json nameWithOwner -q .nameWithOwner` and fall back to:

```bash
gh api "repos/<owner>/<repo>/issues" --method POST --input /tmp/issue-create.json
```
