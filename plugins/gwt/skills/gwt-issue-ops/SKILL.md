---
name: gwt-issue-ops
description: Compatibility alias for gwt-issue-resolve. Use when legacy prompts or commands refer to gwt-issue-ops.
metadata:
  short-description: Compatibility alias for gwt-issue-resolve
---

# GitHub Issue Ops Alias

This skill name is retained for backward compatibility.

Use the `gwt-issue-resolve` workflow without semantic differences.

## Required behavior

1. Load `../gwt-issue-resolve/SKILL.md` and follow it as the source of truth.
2. Treat `gwt-issue-ops` and `gwt-issue-resolve` as exact equivalents for behavior.
3. When documentation, examples, or comments must mention the primary workflow name, prefer `gwt-issue-resolve`.
4. Keep using `gwt-spec-ops` when the resolved execution path is an existing or newly created SPEC issue.

## Quick start

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-issue-resolve/scripts/inspect_issue.py" --repo "." --issue "<number>"
```
| `--focus` | (none) | Codebase search narrowing area |
| `--max-comment-length` | 0 (unlimited) | Max characters per comment body |
| `--json` | false | Emit JSON output |

**Exit codes:**

- `0`: Success
- `1`: Error occurred

## Features

### Issue Data Fetching

Fetches comprehensive issue data:

- Title, body, state, labels, assignees, author
- All comments (with optional length truncation)
- Linked PRs via GraphQL timeline events (CrossReferencedEvent, ConnectedEvent)

### Error Context Extraction

Parses issue body and comments for:

- Error messages (`Error:`, `TypeError:`, `panicked at`, etc.)
- Stack traces (`at`, `Traceback`, `thread '...' panicked`, etc.)
- File path references (`path/to/file.ext:123` format)
- Fenced code blocks
- Well-known sections (Steps to Reproduce, Expected Behavior, Actual Behavior)
- Cross-references (`#123`, `org/repo#123`)

### Issue Classification

Classifies based on labels (highest priority) and body/title heuristics:

- **BUG**: `bug`, `defect`, `regression`, `crash`, `error` labels or error indicators in body
- **FEATURE**: `feature`, `feature-request` labels or feature request language
- **ENHANCEMENT**: `enhancement`, `improvement` labels
- **DOCUMENTATION**: `documentation`, `docs` labels
- **QUESTION**: `question`, `help`, `support` labels or question language
- **UNCLASSIFIED**: No matching signals

### File Existence Check

Validates extracted file references against the repository:

- Checks if files exist at the referenced paths
- Reports `[EXISTS]` or `[NOT FOUND]` status

## Output Examples

### Text Output

```text
Issue #42: TypeError when clicking save button
============================================================
State: OPEN
Type: BUG
Labels: bug, ui
Assignees: developer1
Author: @reporter1
URL: https://github.com/org/repo/issues/42

BODY
------------------------------------------------------------
When I click the save button on the settings page, I get this error:

```
TypeError: Cannot read properties of undefined (reading 'name')
    at SaveHandler (src/components/Settings.tsx:42)
    at onClick (src/components/Button.tsx:15)
```

### Steps to Reproduce
1. Open Settings page
2. Change any setting
3. Click Save

### Expected Behavior
Settings should be saved successfully.

### Actual Behavior
TypeError is thrown and settings are not saved.

EXTRACTED SECTIONS
------------------------------------------------------------

[Steps To Reproduce]
1. Open Settings page
2. Change any setting
3. Click Save

[Expected]
Settings should be saved successfully.

[Actual]
TypeError is thrown and settings are not saved.

ERROR MESSAGES (1)
------------------------------------------------------------
  [1] TypeError: Cannot read properties of undefined (reading 'name')

STACK TRACES (1)
------------------------------------------------------------
  [1]
    at SaveHandler (src/components/Settings.tsx:42)
    at onClick (src/components/Button.tsx:15)

FILE REFERENCES (2)
------------------------------------------------------------
  src/components/Settings.tsx:42 [EXISTS]
  src/components/Button.tsx:15 [EXISTS]

COMMENTS (1)
------------------------------------------------------------
@maintainer1 (2025-01-20):
  This might be related to the recent refactor in #38.

LINKED PULL REQUESTS (1)
------------------------------------------------------------
  PR #45: Fix settings save handler [OPEN]
    https://github.com/org/repo/pull/45
============================================================
```

### JSON Output

```json
{
  "issue": {
    "number": 42,
    "title": "TypeError when clicking save button",
    "body": "...",
    "state": "OPEN",
    "labels": [{"name": "bug"}, {"name": "ui"}],
    "assignees": [{"login": "developer1"}],
    "author": {"login": "reporter1"},
    "url": "https://github.com/org/repo/issues/42"
  },
  "issueType": "BUG",
  "comments": [
    {
      "id": 123456,
      "author": "maintainer1",
      "body": "This might be related to the recent refactor in #38.",
      "createdAt": "2025-01-20T12:00:00Z"
    }
  ],
  "linkedPRs": [
    {
      "number": 45,
      "title": "Fix settings save handler",
      "state": "OPEN",
      "url": "https://github.com/org/repo/pull/45"
    }
  ],
  "parsed": {
    "errorMessages": [
      "TypeError: Cannot read properties of undefined (reading 'name')"
    ],
    "stackTraces": [
      "at SaveHandler (src/components/Settings.tsx:42)\n    at onClick (src/components/Button.tsx:15)"
    ],
    "fileReferences": [
      "src/components/Settings.tsx:42",
      "src/components/Button.tsx:15"
    ],
    "codeBlocks": ["TypeError: Cannot read properties of undefined..."],
    "sections": {
      "steps_to_reproduce": "1. Open Settings page\n2. Change any setting\n3. Click Save",
      "expected": "Settings should be saved successfully.",
      "actual": "TypeError is thrown and settings are not saved."
    },
    "crossReferences": [
      {"repo": "", "number": 38, "ref": "#38"}
    ]
  },
  "fileChecks": [
    {"reference": "src/components/Settings.tsx:42", "path": "src/components/Settings.tsx", "exists": true},
    {"reference": "src/components/Button.tsx:15", "path": "src/components/Button.tsx", "exists": true}
  ]
}
```
