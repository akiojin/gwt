---
name: gwt-pr-check
description: "This skill should be used when the user asks about PR status, says 'check PR status', 'is the PR merged?', 'PR state', 'PRの状態', 'PRどうなった?', or wants to know the current branch's pull request progress. It uses REST-first PR lookups including unmerged PR detection and post-merge new-commit detection."
allowed-tools: Bash, Read, Glob, Grep
argument-hint: "[PR number]"
---

# GH PR Check

## Overview

Check PR status for the current branch with `gh` and report a recommended next action, using REST-first pull-request lookups instead of `gh pr list` as the primary path.

This skill is **check-only**:

- Do not create/switch branches
- Do not push
- Do not create/edit PRs

## Quick reference

| Status | Prefix | Action | Meaning |
| --- | --- | --- | --- |
| `NO_PR` | `>>` | `CREATE PR` | No PR exists |
| `UNMERGED_PR_EXISTS` | `>` | `PUSH ONLY` | Unmerged PR open |
| `CLOSED_UNMERGED_ONLY` | `>>` | `CREATE PR` | Only closed unmerged PRs |
| `ALL_MERGED_WITH_NEW_COMMITS` | `>>` | `CREATE PR` | New commit(s) after merge |
| `ALL_MERGED_NO_PR_DIFF` | `--` | `NO ACTION` | All PRs merged, no diff |
| `CHECK_FAILED` | `!!` | `MANUAL CHECK` | Could not determine |

## Decision rules

See [references/decision-rules.md](references/decision-rules.md) for the full classification algorithm including:

- Branch resolution and REST API lookup
- PR state classification (NO_PR, UNMERGED, CLOSED, MERGED)
- Post-merge commit check with merge commit ancestry verification
- Fallback logic when merge commit SHA is unavailable

## Output contract

See [references/output-format.md](references/output-format.md) for the full output specification including:

- Status and action value enumerations
- Per-status output templates with examples
- Language rules and dirty-worktree indicator

## Workflow (recommended)

1. Verify repo context:
   - `git rev-parse --show-toplevel`
   - `git rev-parse --abbrev-ref HEAD`
2. Confirm auth:
   - `gh auth status`
   - If `GH_TOKEN` / `GITHUB_TOKEN` is already set for REST calls, do not treat `gh auth status` as the sole readiness gate.
3. Collect context:
   - `git status --porcelain`
   - `git fetch origin`
4. Prefer the REST pull-request list endpoint over `gh pr list` when checking branch PR state.
5. List PRs for head branch and classify using decision rules.
6. Treat only `state == open && merged_at == null` as an active blocking PR.
7. When at least one PR is merged and no active blocking PR exists, validate merge commit ancestry before counting commits.
8. If any post-merge count is greater than `0`, verify `git diff --quiet origin/<base>...HEAD --` before recommending `CREATE_PR`.
9. If merge commit is not usable, fallback to `origin/<head>..HEAD` first and then `origin/<base>..HEAD` before returning `NO_ACTION`.
10. Print human-readable result using the default template.
11. Append JSON only if the user explicitly asks for machine-readable output.

## Quick start

```bash
# Human-readable output
python3 ".claude/skills/gwt-pr-check/scripts/check_pr_status.py" --repo "."

# Explicit base branch
python3 ".claude/skills/gwt-pr-check/scripts/check_pr_status.py" --repo "." --base develop

# Append machine-readable JSON after the summary
python3 ".claude/skills/gwt-pr-check/scripts/check_pr_status.py" --repo "." --json
```

## Command snippet

See [references/command-snippet.md](references/command-snippet.md) for the full bash implementation.

## Related skill

- `gwt-pr`: creates/updates PRs
- `gwt-pr-fix`: diagnoses and fixes failing PRs
