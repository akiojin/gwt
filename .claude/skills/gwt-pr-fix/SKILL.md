---
name: gwt-pr-fix
description: "This skill should be used when the user says 'fix CI', 'fix the PR', 'CI is failing', 'CIを直して', 'PRを直して', 'resolve PR blockers', 'マージできない', or after creating/pushing a PR when CI failures or merge blockers are detected. It inspects GitHub PRs for CI failures, merge conflicts, update-branch requirements, reviewer comments, change requests, and unresolved review threads, then autonomously fixes high-confidence blockers and replies to ALL reviewer comments."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
metadata:
  short-description: Fix failing GitHub PRs comprehensively
---

# Gh PR Checks Plan Fix

## Overview

Inspect PRs for CI failures, merge conflicts, update-branch requirements, reviewer comments, change requests, and unresolved review threads. Then fix, reply to every reviewer comment, resolve all threads, and notify reviewers.

REST-first boundary:

- PR resolution, CI/check reads, reviews, review comments, and issue comments: REST-first
- Unresolved review thread discovery: GraphQL-only
- Review thread reply / resolve: GraphQL-only

Prereq: ensure `gh` is authenticated (`gh auth login` or `GH_TOKEN` / `GITHUB_TOKEN`).

## Inputs

| Input | Default | Description |
|-------|---------|-------------|
| `repo` | `.` | Path inside the repo |
| `pr` | (current) | PR number or URL |
| `mode` | `all` | `checks`, `conflicts`, `reviews`, `all` |
| `required-only` | false | Limit CI checks to required only |

## Quick start

```bash
# Inspect all (CI, conflicts, reviews) - default mode
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>"

# CI checks only
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --mode checks

# Conflicts only
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --mode conflicts

# Reviews only (Change Requests + Unresolved Threads)
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --mode reviews

# JSON output
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --json

# Reply to all unresolved threads and resolve them
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --reply-and-resolve '[
  {"threadId":"PRRT_xxx123","body":"Fixed: refactored the method as suggested."},
  {"threadId":"PRRT_xxx456","body":"Not addressed: this is intentional because the API requires this format."}
]'

# Add a comment to notify reviewers
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --add-comment "Fixed all issues. Please re-review."
```

## Workflow summary

1. Verify gh authentication
2. Resolve the PR (current branch or user-provided)
3. Inspect based on mode (conflicts / reviews / checks / all)
4. Produce Diagnosis Report in mandatory B-item/I-item format
5. Decide execution path (auto-fix vs user confirmation)
6. Implement fixes, commit, and push
7. Reply to ALL reviewer comments and resolve threads
8. Notify reviewers via PR comment
9. Verify fix by re-running inspection

Load `references/workflow.md` for detailed step-by-step instructions, diagnosis report format, classification rules, fix strategies, and loop safety guard.

## Policies and formatting

Load `references/policies.md` for comment response policy, diagnosis report anti-patterns, prohibited language, and issue/PR comment formatting rules.

## Bundled script reference

### scripts/inspect_pr_checks.py

| Argument | Default | Description |
|----------|---------|-------------|
| `--repo` | `.` | Path inside the target Git repository |
| `--pr` | (current) | PR number or URL |
| `--mode` | `all` | Inspection mode: `checks`, `conflicts`, `reviews`, `all` |
| `--max-lines` | 160 | Max lines for log snippets |
| `--context` | 30 | Context lines around failure markers |
| `--required-only` | false | Limit CI checks to required checks only |
| `--json` | false | Emit JSON output |
| `--reply-and-resolve` | (none) | JSON array of `{threadId, body}` to reply and resolve ALL threads |
| `--add-comment` | (none) | Add a comment to the PR |

Exit codes: `0` = no issues, `1` = issues detected or error.

## Feature summary

| Feature | API | Description |
|---------|-----|-------------|
| Conflict Detection | REST | `mergeable` + `mergeStateStatus` fields |
| Change Requests | REST | Reviews with `CHANGES_REQUESTED` state |
| Reviewer Comments | REST | All review summaries, inline comments, issue comments |
| Unresolved Threads | GraphQL | Threads where `isResolved == false` |
| Reply and Resolve | GraphQL | `addPullRequestReviewThreadReply` + `resolveReviewThread` |
| Reviewer Notification | REST | `POST /repos/<owner>/<repo>/issues/<pr_number>/comments` |

## Output examples

Load `references/output-examples.md` for diagnosis report, text output, reply-and-resolve output, and JSON output examples.

## References

- `references/workflow.md`: Detailed workflow steps, diagnosis report format, fix strategies
- `references/policies.md`: Comment response policy, anti-patterns, formatting rules
- `references/output-examples.md`: Output format examples
