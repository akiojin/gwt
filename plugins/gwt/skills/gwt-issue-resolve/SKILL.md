---
name: gwt-issue-resolve
description: Resolve a GitHub Issue end-to-end. Analyze the issue, decide whether it should be fixed directly, merged into an existing gwt-spec issue, or promoted to a new spec issue, and continue toward resolution.
metadata:
  short-description: Resolve GitHub Issues through direct fixes or spec workflows
---

# GitHub Issue Resolve

Use this skill as the main entrypoint when the user brings a GitHub Issue and wants it progressed, not merely classified.

The skill must decide the execution path:

- Direct fix path for clear bugs or small corrective work
- Existing SPEC path when the Issue belongs in a canonical `gwt-spec` issue
- New SPEC path when the Issue needs spec management but no suitable SPEC exists yet

Do not require the Issue author to pre-label or pre-register the work as `gwt-spec`.

## Overview

Use gh to inspect a GitHub Issue and:

- Fetch issue metadata (title, body, state, labels, assignees)
- Fetch all comments
- Fetch linked PRs via timeline events
- Extract error messages, stack traces, file references, code blocks, and cross-references
- Detect whether the issue is already a spec issue (`gwt-spec` label, `GWT_SPEC_ID` marker, or spec section structure)
- Classify the work as BUG / FEATURE / ENHANCEMENT / DOCUMENTATION / QUESTION / UNCLASSIFIED
- Search the codebase for relevant files
- Decide whether to fix directly, update an existing SPEC, or create a new SPEC
- Continue toward resolution instead of stopping at triage

If the issue is already a spec issue, switch to `gwt-spec-ops`.

If the issue is not a spec issue:

- BUG and small corrective work should normally stay on the direct fix path
- FEATURE / ENHANCEMENT and larger scoped changes should normally go through SPEC selection first
- If a bug reveals missing product behavior, interface changes, or cross-cutting requirements, promote it to the SPEC path

## Mandatory preflight for spec-needed issues

When the Issue needs SPEC handling, use `gwt-project-index` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call
2. Update the Issues index if needed
3. Run semantic Issue search with queries derived from the current request
4. Prefer an existing canonical integrated SPEC over a transient point-fix/refactor SPEC
5. Create a new `gwt-spec` Issue only when no suitable canonical SPEC exists

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
   - If labels include `gwt-spec`, or the body contains `<!-- GWT_SPEC_ID:#... -->`, or the body clearly follows the `## Spec` / `## Plan` / `## Tasks` / `## TDD` structure, stop generic issue handling.
   - Hand off to `gwt-spec-ops` with the issue number and current context.

5. **Direct-fix vs spec-needed decision.**
   - Prefer the direct-fix path for clear bugs and small corrective work.
   - Prefer the spec-needed path for features, enhancements, cross-cutting changes, or bugs that require behavioral definition before code changes.
   - Record the reason for the chosen path in the analysis output.

6. **Direct-fix path.**
   - Search the codebase for relevant files and definitions.
   - Produce the Issue Analysis Report.
   - If all actionable items have High confidence, propose the concrete fix and continue after approval.
   - If any actionable item has Low confidence, ask only the minimum question needed to unblock.

7. **Spec-needed path.**
   - Use `gwt-project-index` Issue search before creating or updating any SPEC.
   - Search with at least 2 semantic queries derived from the Issue.
   - If a canonical existing SPEC is found, update that destination and hand off to `gwt-spec-ops`.
   - If no suitable SPEC exists, create a new `gwt-spec` Issue with the built-in spec tooling when available, or the documented gh fallback when the built-in path is unavailable.
   - After the target SPEC exists, hand off to `gwt-spec-ops`.

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
    - Direct-fix path: apply the approved fix, summarize diffs and tests, then update the issue and PR linkage.
    - SPEC path: pass the resolved SPEC issue number and context into `gwt-spec-ops`.

## Bundled Resources

### scripts/inspect_issue.py

GitHub Issue inspection and analysis tool. Fetches issue data, parses context, classifies the issue, and checks file existence.
