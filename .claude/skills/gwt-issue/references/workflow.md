# Issue Workflow (Detailed Steps)

## Register Mode

### Step 1: Verify gh authentication

- Run `gh auth status` in the repo.
- If unauthenticated, stop and ask the user to log in.

### Step 2: Normalize the request

- Extract the request summary, intended outcome, relevant subsystem, and any links or
  prior context.
- Classify the request type:
  BUG | FEATURE | ENHANCEMENT | DOCUMENTATION | CHORE | QUESTION | UNCLASSIFIED

### Step 3: Search for an existing destination

- Use `gwt-issue-search` with at least 2 semantic queries.
- Also search local `specs/` directory via `spec_artifact.py --repo . --list-all`.
- Prefer open Issues and active canonical SPECs.

### Step 4: Reuse duplicates or existing owners

- If a clear open Issue already tracks the request, do not create a new item.
  Switch to resolve mode and continue there.
- If a clear SPEC already owns the scope, do not create a new item.
  Switch to the visible SPEC flow and continue there.
- If the search returns plausible candidates but no single clear owner, stop and present
  the top 1-3 candidates instead of creating a new item.
- Report the result with `Chosen Path: EXISTING` and continue with the owning workflow.

### Step 5: Choose plain Issue or new SPEC

**Create a plain GitHub Issue when:**

- clear bug report or regression
- documentation task
- chore / maintenance task
- question or investigation request
- narrowly scoped enhancement that does not need new product behavior specified first

**Create a new local SPEC directory when:**

- multiple user scenarios or acceptance criteria
- new or changed product behavior that must be defined first
- UI / UX flow decisions
- cross-cutting or multi-subsystem changes
- non-trivial technical or product tradeoffs

When the need for a SPEC is clear, do not create a plain Issue first. Create or deepen
the SPEC through `gwt-design-spec`, then continue through the visible SPEC flow. Only create a plain
Issue too when the user explicitly asks for separate GitHub tracking.

### Step 6: Create the plain Issue when needed

- Use Conventional Commit style titles (`fix:`, `feat:`, `docs:`, `chore:`).
- Fill the required body sections with concrete request context, not `_TODO_`.

### Step 7: Return the created Issue or active owner

- For plain Issue creation, return the issue number and URL.
- For existing owner or new SPEC, state the exact destination and continue with that
  workflow unless an ambiguous product decision still blocks progress.

---

## Resolve Mode

### Step 1: Verify gh authentication

- Run `gh auth status` in the repo.
- If unauthenticated, stop and ask the user to log in.

### Step 2: Resolve the Issue input

- Accept an issue number or full URL.
- Validate that the issue exists and is accessible.

### Step 3: Run `inspect_issue.py` to gather facts

- Fetch issue metadata, comments, and linked PRs.
- Parse error messages, stack traces, file references, code blocks, sections, and
  cross-references.
- Classify the issue type.
- Check file existence for extracted file references.

### Step 4: Already-SPEC path

- If a corresponding local SPEC directory exists (check `specs/` directory), stop
  generic issue handling.
- Treat Issue-body spec sections only as legacy migration hints; do not treat the Issue
  body as the current SPEC source of truth.
- Hand off to the visible SPEC flow with the SPEC ID and current context.

### Step 5: Direct-fix vs spec-needed decision

- Prefer the direct-fix path for clear bugs and small corrective work.
- Prefer the spec-needed path for features, enhancements, cross-cutting changes, or bugs
  that require behavioral definition before code changes.
- Record the reason for the chosen path in the analysis output.

### Step 6: Direct-fix path

- Search the codebase for relevant files and definitions.
- Produce the Issue Analysis Report (see `references/analysis-report.md`).
- If all actionable items have High confidence, implement the fix immediately and keep
  the Issue moving.
- If any actionable item has Low confidence, ask only the minimum question needed to
  unblock.

### Step 7: Spec-needed path

- Use `gwt-issue-search` before creating or updating any SPEC.
- Also search local `specs/` via `spec_artifact.py --repo . --list-all`.
- Search with at least 2 semantic queries derived from the Issue.
- If a canonical existing SPEC is found, update that destination and continue with the
  visible SPEC flow.
- If no suitable SPEC exists, create or deepen the owner SPEC through `gwt-design-spec`.
- Do not "convert the Issue into the SPEC"; the Issue remains an Issue and the SPEC
  remains a local artifact set.
- After the target SPEC exists, continue with the visible SPEC flow, which owns
  design, planning, and implementation sequencing.

### Step 8: Produce Issue Analysis Report

See `references/analysis-report.md` for the full template.

### Step 9: Post progress comments to the Issue

Use the Issue Progress Comment Template:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

Post updates at least when starting work, after meaningful progress, and when blocked
or unblocked.

### Step 10: Execution handoff or implementation

- Direct-fix path: apply the fix, summarize diffs and tests, then update the issue and
  PR linkage.
- SPEC path: pass the resolved SPEC ID and context into the visible SPEC flow, then let
  that workflow continue end-to-end.
