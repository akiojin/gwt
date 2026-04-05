# Issue Resolve Workflow (Detailed Steps)

## Step 1: Verify gh authentication

- Run `gh auth status` in the repo.
- If unauthenticated, stop and ask the user to log in.

## Step 2: Resolve the Issue input

- Accept an issue number or full URL.
- Validate that the issue exists and is accessible.

## Step 3: Run `inspect_issue.py` to gather facts

- Fetch issue metadata, comments, and linked PRs.
- Parse error messages, stack traces, file references, code blocks, sections, and cross-references.
- Classify the issue type.
- Check file existence for extracted file references.

## Step 4: Already-SPEC path

- If a corresponding local SPEC directory exists (check `specs/` directory), stop generic issue handling.
- Treat Issue-body spec sections only as legacy migration hints; do not treat the Issue body as the current SPEC source of truth.
- Hand off to `gwt-spec-ops` with the SPEC ID and current context.

## Step 5: Direct-fix vs spec-needed decision

- Prefer the direct-fix path for clear bugs and small corrective work.
- Prefer the spec-needed path for features, enhancements, cross-cutting changes, or bugs that require behavioral definition before code changes.
- Record the reason for the chosen path in the analysis output.

## Step 6: Direct-fix path

- Search the codebase for relevant files and definitions.
- Produce the Issue Analysis Report.
- If all actionable items have High confidence, implement the fix immediately and keep the Issue moving.
- If any actionable item has Low confidence, ask only the minimum question needed to unblock.

## Step 7: Spec-needed path

- Use `gwt-issue-search` before creating or updating any SPEC.
- Also search local `specs/` via `spec_artifact.py --repo . --list-all`.
- Search with at least 2 semantic queries derived from the Issue.
- If a canonical existing SPEC is found, update that destination and continue with `gwt-spec-ops`.
- If no suitable SPEC exists, create a new local SPEC directory through `gwt-spec-register`.
- Do not "convert the Issue into the SPEC"; the Issue remains an Issue and the SPEC remains a local artifact set.
- After the target SPEC exists, continue with `gwt-spec-ops`, which owns clarify/plan/tasks/analyze and then implementation.

## Step 8: Issue Analysis Report format

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

## Step 9: Post progress comments to the Issue

- Use the Issue Progress Comment Template.
- Include the chosen execution path and immediate next action.

## Step 10: Execution handoff or implementation

- Direct-fix path: apply the fix, summarize diffs and tests, then update the issue and PR linkage.
- SPEC path: pass the resolved SPEC ID and context into `gwt-spec-ops`, then let that workflow continue end-to-end.
