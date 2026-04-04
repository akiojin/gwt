---
name: gwt-issue-resolve
description: "This skill should be used when the user brings a GitHub Issue to be worked on, says 'resolve this issue', 'fix issue #N', 'progress this issue', 'このIssueを解決して', 'Issue #Nを直して', or provides a GitHub Issue URL. It analyzes the issue, decides whether to fix directly, merge into an existing SPEC, or promote to a new SPEC, and continues toward resolution end-to-end."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[issue-number or URL]"
metadata:
  short-description: Resolve GitHub Issues through direct fixes or spec workflows
---

# GitHub Issue Resolve

## Overview

Use this skill as the main entrypoint when the user brings a GitHub Issue and wants it progressed, not merely classified. If the user wants to register new work and no Issue exists yet, use `gwt-issue-register` first.

The skill decides the execution path:

- **Direct fix** for clear bugs or small corrective work
- **Existing SPEC** when the Issue belongs in a canonical SPEC
- **New SPEC** when the Issue needs spec management but no suitable SPEC exists yet

Do not require the Issue author to pre-label or pre-register the work as a SPEC.

## Inputs

| Input | Default | Description |
|-------|---------|-------------|
| `repo` | `.` | Path inside the repo |
| `issue` | (required) | Issue number or URL |
| `focus` | (optional) | Codebase search narrowing |
| `max-comment-length` | 0 | Max characters per comment body (0 = unlimited) |

## Quick start

```bash
# Inspect issue (text output)
python3 ".claude/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>"

# Inspect issue by URL
python3 ".claude/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "https://github.com/org/repo/issues/123"

# With focus area
python3 ".claude/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>" --focus "src/lib"

# JSON output
python3 ".claude/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>" --json
```

## Mandatory preflight for spec-needed issues

When the Issue needs SPEC handling, use `gwt-issue-search` first.

1. Ensure `gh auth status` is valid
2. Update the Issues index if needed
3. Run semantic search with queries derived from the current request
4. Search local `specs/` via `spec_artifact.py --repo . --list-all`
5. Prefer an existing canonical integrated SPEC over transient specs
6. Create a new SPEC only when no suitable canonical SPEC exists

Do not ask the user whether to use an existing or new SPEC if the repo state and search results make the answer clear.

## Workflow summary

1. Verify gh authentication
2. Resolve the Issue input
3. Run `inspect_issue.py` to gather facts
4. Check for existing local SPEC (hand off to `gwt-spec-ops` if found)
5. Decide direct-fix vs spec-needed path
6. Direct-fix: search codebase, produce analysis report, implement
7. Spec-needed: search existing SPECs, create/update SPEC, hand off to `gwt-spec-ops`
8. Produce Issue Analysis Report (for non-SPEC issues)
9. Post progress comments to the Issue
10. Execute or hand off

Load `references/workflow.md` for detailed step-by-step instructions and Issue Analysis Report format.

## Analysis Report Anti-Patterns

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 actionable items detected" |
| "This might be causing..." | "Root cause: `<error from issue>`" |
| "Consider fixing..." / "It looks like..." | "Action: Fix `<what>` in `<where>`" |
| "Various errors are reported" | "2 error messages extracted: `<msg1>`, `<msg2>`" |

### Structural Prohibitions

- No prose paragraphs for reporting. Use A1/I1 item format.
- Do not omit the Evidence field in any ACTIONABLE item.
- Do not combine multiple independent problems into a single item.
- Do not omit file paths or line numbers when the script output contains them.

## Comment formatting

- No escaped newline literals (`\n`) in comment text.
- Use real line breaks. Verify before posting with `gh issue comment`.

## Issue Progress Comment Template

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

Post updates at least when starting work, after meaningful progress, and when blocked or unblocked.

## Bundled Resources

### scripts/inspect_issue.py

GitHub Issue inspection and analysis tool. Fetch issue data, parse context, classify the issue, and check file existence.

## References

- `references/workflow.md`: Detailed workflow steps and Issue Analysis Report format
