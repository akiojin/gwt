---
name: gwt-issue
description: "Use proactively when user references a GitHub Issue or reports a bug. Auto-detects register mode (no Issue number) or resolve mode (Issue number/URL provided). Searches existing Issues before creating duplicates. Triggers: 'register issue', 'file a bug', 'fix issue #N', 'resolve issue'."
---

# gwt Issue

Unified entrypoint for GitHub Issue registration and resolution.

## Mode Detection

- **Register mode:** No Issue number or URL provided. The user wants to file, register,
  or draft new work from a bug report, feature request, enhancement idea, documentation
  task, or rough note.
- **Resolve mode:** Issue number or URL provided. The user wants an existing Issue
  progressed, not merely classified.

If the target SPEC is already known, use `gwt-spec-ops` instead.

## Hard Routing Rules

- Do not call `gh issue create` manually before this skill completes duplicate search
  and ISSUE vs SPEC selection.
- Do not jump to `gwt-spec-register` before duplicate search completes.
- Do not create both a plain Issue and a SPEC for the same request.
- A local SPEC directory is not a GitHub Issue. When a SPEC is needed, create or reuse
  the local SPEC as the source of truth; treat any Issue as an optional related record.
- Agent-facing Issue workflow must use `gwt issue ...` as the canonical CLI surface.
  Direct `gh issue ...` commands are not part of the normal path.

## Mandatory Preflight: Search Existing Issues First

Before creating any new Issue or SPEC (register mode), or before creating/updating any
SPEC (resolve mode, spec-needed path), use `gwt-issue-search` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call.
2. Update the Issues index if needed.
3. Run at least 2 semantic Issue queries derived from the request.
4. Search for both plain Issues and existing SPECs (via `spec_artifact.py --repo . --list-all`).
5. Reuse a clear existing destination instead of creating a duplicate.

Do not skip duplicate search because the request "sounds new".

## Duplicate Handling

- If an open plain Issue clearly matches, switch to resolve mode and continue there.
- If an existing SPEC clearly owns the scope, switch to `gwt-spec-ops` and continue.
- If the search returns plausible candidates but no single clear owner, stop and present
  the top 1-3 candidates instead of creating a new item.
- Do not create a fresh Issue "just in case" when the duplicate check is inconclusive.

## Plain Issue vs SPEC Decision

Create a **plain GitHub Issue** when the request is primarily one of:

- clear bug report or regression
- documentation task
- chore / maintenance task
- question or investigation request
- narrowly scoped enhancement that does not need new product behavior specified first

Create a **new local SPEC directory** when the request includes any of:

- multiple user scenarios or acceptance criteria
- new or changed product behavior that must be defined first
- UI / UX flow decisions
- cross-cutting or multi-subsystem changes
- non-trivial technical or product tradeoffs

When the need for a SPEC is clear, do not create a plain Issue first. Create the SPEC
through `gwt-spec-register` and continue through `gwt-spec-ops`.

## Title Rules for Plain Issues

Use Conventional Commit style titles:

- `fix: ...` for bugs and regressions
- `feat: ...` for user-visible enhancements on the plain-Issue path
- `docs: ...` for documentation work
- `chore: ...` for maintenance or operational tasks

Prefer a short imperative summary. Do not use vague titles.

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

## Inputs

- `repo`: path inside the repo (default `.`)
- `issue`: Issue number or URL (resolve mode only; omit for register mode)
- `focus`: codebase search narrowing (optional, resolve mode)

## Quick Start

```bash
# Inspect issue (resolve mode)
python3 ".claude/skills/gwt-issue/scripts/inspect_issue.py" --repo "." --issue "<number>"

# Inspect issue by URL
python3 ".claude/skills/gwt-issue/scripts/inspect_issue.py" --repo "." --issue "https://github.com/org/repo/issues/123"

# With focus area
python3 ".claude/skills/gwt-issue/scripts/inspect_issue.py" --repo "." --issue "<number>" --focus "src/lib"

# JSON output
python3 ".claude/skills/gwt-issue/scripts/inspect_issue.py" --repo "." --issue "<number>" --json
```

---

## Register Mode Workflow

### 1. Verify GitHub access

Issue writes go through `gwt issue ...`, which resolves GitHub auth internally. If that
surface fails with an auth error, stop and ask the user to refresh GitHub authentication.

### 2. Normalize the request

- Extract the request summary, intended outcome, relevant subsystem, and any links or
  prior context.
- Classify the request type.

### 3. Search for an existing destination

- Use `gwt-issue-search` with at least 2 semantic queries.
- Also search local `specs/` directory via `spec_artifact.py --repo . --list-all`.
- Prefer open Issues and active canonical SPECs.

### 4. Reuse duplicates or existing owners

- If a clear open Issue already tracks the request, do not create a new item.
- If a clear SPEC already owns the scope, do not create a new item.
- Report the result with `Chosen Path: EXISTING` and continue with the owning workflow.

### 5. Choose plain Issue or new SPEC

Use the decision rules above.

- Plain Issue: create directly with `gwt issue create --title ... -f ...`.
- New SPEC: create through `gwt-spec-register` (local directory), then continue through
  `gwt-spec-ops`.

### 6. Create the plain Issue when needed

Use `gwt issue create --title ... -f <body-file>` with the required title rule and issue
body structure. Fill the sections with concrete request context, not `_TODO_`.

### 7. Return the created Issue or active owner

- For plain Issue creation, return the issue number and URL.
- For existing owner or new SPEC, state the exact destination and continue with that
  workflow unless an ambiguous product decision still blocks progress.

### Registration Decision Output

Before creating anything, report the decision:

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

---

## Resolve Mode Workflow

### 1. Verify GitHub access

Resolve mode should prefer `gwt issue view <number>` and `gwt issue comments <number>` as the
canonical read path. If those commands fail with an auth error, stop and ask the user to
refresh GitHub authentication.

### 2. Resolve the Issue input

Accept an issue number or full URL. Validate that the issue exists and is accessible.

### 3. Run `inspect_issue.py` to gather facts

- Fetch issue metadata, comments, and linked PRs.
- Parse error messages, stack traces, file references, code blocks, sections, and
  cross-references.
- Classify the issue type.
- Check file existence for extracted file references.

### 4. Already-SPEC path

- If a corresponding local SPEC directory exists (check `specs/` directory), stop
  generic issue handling.
- Treat Issue-body spec sections only as legacy migration hints; do not treat the Issue
  body as the current SPEC source of truth.
- Hand off to `gwt-spec-ops` with the SPEC ID and current context.

### 5. Direct-fix vs spec-needed decision

- Prefer the direct-fix path for clear bugs and small corrective work.
- Prefer the spec-needed path for features, enhancements, cross-cutting changes, or bugs
  that require behavioral definition before code changes.
- Record the reason for the chosen path in the analysis output.

### 6. Direct-fix path

- Search the codebase for relevant files and definitions.
- Produce the Issue Analysis Report (see `references/analysis-report.md`).
- If all actionable items have High confidence, implement the fix immediately.
- If any actionable item has Low confidence, ask only the minimum question needed.

### 7. Spec-needed path

- Use `gwt-issue-search` before creating or updating any SPEC.
- Also search local `specs/` via `spec_artifact.py --repo . --list-all`.
- Search with at least 2 semantic queries derived from the Issue.
- If a canonical existing SPEC is found, update that destination and continue with
  `gwt-spec-ops`.
- If no suitable SPEC exists, create a new local SPEC directory through
  `gwt-spec-register`.
- Do not "convert the Issue into the SPEC"; the Issue remains an Issue and the SPEC
  remains a local artifact set.
- After the target SPEC exists, continue with `gwt-spec-ops`.

### 8. Post progress comments to the Issue

Use the Issue Progress Comment Template:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

Post updates at least when starting work, after meaningful progress, and when blocked.

### 9. Execution handoff or implementation

- Direct-fix path: apply the fix, summarize diffs and tests, then update the issue and
  PR linkage.
- SPEC path: pass the resolved SPEC ID and context into `gwt-spec-ops`, then let that
  workflow continue end-to-end.

---

## Analysis Report Anti-Patterns

See `references/analysis-report.md` for the full report template and structural rules.

### Prohibited Language

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 actionable items detected" |
| "This might be causing..." | "Root cause: `<error from issue>`" |
| "Consider fixing..." / "It looks like..." | "Action: Fix `<what>` in `<where>`" |
| "Various errors are reported" | "2 error messages extracted: `<msg1>`, `<msg2>`" |
| "Some files are involved" | "3 file references: `src/a.ts:42`, `src/b.rs:10`, `src/c.py`" |

### Structural Prohibitions

- Prose paragraphs for reporting. Use A1/I1 item format.
- Omitting the Evidence field in any ACTIONABLE item.
- Combining multiple independent problems into a single item.
- Omitting file paths or line numbers when the script output contains them.

## Issue/PR Comment Formatting

- Final comment text must not contain escaped newline literals such as `\n`.
- Use real line breaks in comment bodies.
- Before posting with `gwt issue comment`, verify the final body does not contain
  accidental escaped control sequences.

## Canonical Issue Commands

### Read

```bash
gwt issue view 123
gwt issue comments 123
gwt issue linked-prs 123
```

### Create plain Issue

```bash
gwt issue create --title "fix: ..." -f /tmp/issue-body.md
```

### Comment on existing Issue

```bash
gwt issue comment 123 -f /tmp/comment.md
```

## Transport Notes

The normal agent path should use `gwt issue ...`. Internal helpers may still use `gh`
or direct HTTP as transport, but that is an implementation detail, not the public workflow.

## Bundled Resources

### scripts/inspect_issue.py

GitHub Issue inspection and analysis tool. Fetches issue data, parses context, classifies
the issue, and checks file existence.
