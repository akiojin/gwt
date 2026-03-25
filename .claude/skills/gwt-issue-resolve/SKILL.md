---
name: gwt-issue-resolve
description: "Resolve an existing GitHub Issue end-to-end. Analyze the issue, decide whether it should be fixed directly, merged into an existing SPEC, or promoted to a new SPEC, and continue toward resolution. Use gwt-issue-register for brand-new work registration. Use when user says 'resolve this issue', 'fix issue #N', 'progress this issue', or brings a GitHub Issue URL to be worked on."
metadata:
  short-description: Resolve GitHub Issues through direct fixes or spec workflows
---

# GitHub Issue Resolve

Use this skill as the main entrypoint when the user brings a GitHub Issue and wants it progressed, not merely classified.

- If the user wants to register new work and no Issue exists yet, use `gwt-issue-register` first.

The skill must decide the execution path:

- Direct fix path for clear bugs or small corrective work
- Existing SPEC path when the Issue belongs in a canonical SPEC
- New SPEC path when the Issue needs spec management but no suitable SPEC exists yet

Do not require the Issue author to pre-label or pre-register the work as a SPEC.

## Overview

Use gh to inspect a GitHub Issue and:

- Fetch issue metadata (title, body, state, labels, assignees)
- Fetch all comments
- Fetch linked PRs via timeline events
- Extract error messages, stack traces, file references, code blocks, and cross-references
- Detect whether the issue is already a spec issue (has a corresponding local SPEC directory, or body contains spec section structure)
- Classify the work as BUG / FEATURE / ENHANCEMENT / DOCUMENTATION / QUESTION / UNCLASSIFIED
- Search the codebase for relevant files
- Decide whether to fix directly, update an existing SPEC, or register a new SPEC
- Continue toward resolution instead of stopping at triage

If the issue already has a corresponding local SPEC directory, switch to `gwt-spec-ops`.

If the issue is not a spec issue:

- BUG and small corrective work should normally stay on the direct fix path
- FEATURE / ENHANCEMENT and larger scoped changes should normally go through SPEC selection first
- If a bug reveals missing product behavior, interface changes, or cross-cutting requirements, promote it to the SPEC path

## Mandatory preflight for spec-needed issues

When the Issue needs SPEC handling, use `gwt-issue-search` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call
2. Update the Issues index if needed
3. Run semantic search with queries derived from the current request
4. Also search local `specs/` directory via `spec_artifact.py --repo . --list-all`
5. Prefer an existing canonical integrated SPEC over a transient point-fix/refactor SPEC
6. Create a new SPEC only when no suitable canonical SPEC exists

Do not ask the user whether to use an existing or new SPEC if the repo state and search results make the answer clear.

## Analysis Report Anti-Patterns

### Prohibited Language

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 actionable items detected" |
| "This might be causing..." | "Root cause: `<error from issue>`" |
| "Consider fixing..." / "It looks like..." | "Action: Fix `<what>` in `<where>`" |
| "Various errors are reported" | "2 error messages extracted: `<msg1>`, `<msg2>`" |
| "Some files are involved" | "3 file references: `src/a.ts:42`, `src/b.rs:10`, `src/c.py`" |
| "I'll try to fix this" | "Action: <specific fix>" |

### Structural Prohibitions

- Prose paragraphs for reporting. Use A1/I1 item format.
- Omitting the Evidence field in any ACTIONABLE item.
- Combining multiple independent problems into a single item.
- Omitting file paths or line numbers when the script output contains them.

## Issue/PR Comment Formatting

- Final comment text must not contain escaped newline literals such as `\n`.
- Use real line breaks in comment bodies.
- Before posting with `gh issue comment`, verify the final body does not contain accidental escaped control sequences.

## Issue Progress Comment Template

When posting progress updates to the issue, use:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

Post updates at least when starting work, after meaningful progress, and when blocked or unblocked.

## Inputs

- `repo`: path inside the repo (default `.`)
- `issue`: Issue number or URL (required)
- `focus`: codebase search narrowing (optional)
- `max-comment-length`: max characters per comment body (0 = unlimited)
- `gh` authentication for the repo host

## Quick start

```bash
# Inspect issue (text output)
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>"

# Inspect issue by URL
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "https://github.com/org/repo/issues/123"

# With focus area
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>" --focus "src/lib"

# JSON output
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>" --json
```

## Workflow

1. **Verify gh authentication.**
   - Run `gh auth status` in the repo.
   - If unauthenticated, stop and ask the user to log in.

2. **Resolve the Issue input.**
   - Accept an issue number or full URL.
   - Validate that the issue exists and is accessible.

3. **Run `inspect_issue.py` to gather facts.**
   - Fetch issue metadata, comments, and linked PRs.
   - Parse error messages, stack traces, file references, code blocks, sections, and cross-references.
   - Classify the issue type.
   - Check file existence for extracted file references.

4. **Already-SPEC path.**
   - If a corresponding local SPEC directory exists (check `specs/` directory), or the body clearly follows the `## Spec` / `## Plan` / `## Tasks` / `## TDD` structure, stop generic issue handling.
   - Hand off to `gwt-spec-ops` with the SPEC ID and current context.

5. **Direct-fix vs spec-needed decision.**
   - Prefer the direct-fix path for clear bugs and small corrective work.
   - Prefer the spec-needed path for features, enhancements, cross-cutting changes, or bugs that require behavioral definition before code changes.
   - Record the reason for the chosen path in the analysis output.

6. **Direct-fix path.**
   - Search the codebase for relevant files and definitions.
   - Produce the Issue Analysis Report.
   - If all actionable items have High confidence, implement the fix immediately and keep the Issue moving.
   - If any actionable item has Low confidence, ask only the minimum question needed to unblock.

7. **Spec-needed path.**
   - Use `gwt-issue-search` before creating or updating any SPEC.
   - Also search local `specs/` via `spec_artifact.py --repo . --list-all`.
   - Search with at least 2 semantic queries derived from the Issue.
   - If a canonical existing SPEC is found, update that destination and continue with `gwt-spec-ops`.
   - If no suitable SPEC exists, create a new local SPEC directory through `gwt-spec-register`.
   - After the target SPEC exists, continue with `gwt-spec-ops`, which owns clarify/plan/tasks/analyze and then implementation.

8. **Produce Issue Analysis Report for non-SPEC issues before execution.**

   Output must use this structure:

   ````text
   ## Issue Analysis Report: #<number>

   **Issue Type:** BUG | FEATURE | ENHANCEMENT | DOCUMENTATION | QUESTION | UNCLASSIFIED
   **Title:** <issue title>
   **State:** OPEN | CLOSED
   **Labels:** <label1>, <label2>, ...
   **Assignees:** <assignee1>, <assignee2>, ...
   **Execution Path:** DIRECT-FIX | EXISTING-SPEC | NEW-SPEC
   Actionable items: <N>

   ---

   ### EXTRACTED CONTEXT

   #### Error Messages
   - `<error message 1>`

   #### Stack Traces
   ~~~
   <stack trace>
   ~~~

   #### File References
   - `path/to/file.ext:42` [EXISTS]

   #### Repro Steps
   <extracted Steps to Reproduce section>

   #### Expected vs Actual
   - **Expected:** <extracted expected behavior>
   - **Actual:** <extracted actual behavior>

   ---

   ### CODEBASE MATCHES

   #### M1. <file or symbol>
   - **Path:** `path/to/file.ext:line`
   - **Relevance:** Why this file matters

   ---

   ### ACTIONABLE

   #### A1. [CATEGORY] <1-line title>
   - **What:** Factual statement
   - **Where:** file_path:line_number
   - **Evidence:** Verbatim quote from issue or codebase
   - **Action:** Specific fix or handoff action
   - **Confidence:** High | Medium | Low

   ---

   ### INFORMATIONAL

   #### I1. [CATEGORY] <1-line title>
   - **What / Note**

   ---

   ### LINKED CONTEXT

   #### Linked PRs
   - PR #<number>: <title> [<state>]

   #### Cross-references
   - #<number>

   #### Comments Summary
   - <N> comments from <M> authors
   - Key points: ...

   ---

   **Summary:** <N> actionable items, <M> informational items, <K> codebase matches.
   ````

9. **Post progress comments to the Issue.**
   - Use the Issue Progress Comment Template.
   - Include the chosen execution path and immediate next action.

10. **Execution handoff or implementation.**
    - Direct-fix path: apply the fix, summarize diffs and tests, then update the issue and PR linkage.
    - SPEC path: pass the resolved SPEC ID and context into `gwt-spec-ops`, then let that workflow continue end-to-end.

## Bundled Resources

### scripts/inspect_issue.py

GitHub Issue inspection and analysis tool. Fetches issue data, parses context, classifies the issue, and checks file existence.
