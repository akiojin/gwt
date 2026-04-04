---
name: gwt-pr
description: "This skill should be used when the user asks to open, create, edit, or update a GitHub Pull Request, says 'open a PR', 'create a PR', 'gh pr', 'PRを作って', 'PRを開いて', or needs to generate a PR body or template. It prefers REST-first gh api flows for PR list/create/update/view and decides whether to create a new PR or only push based on existing PR merge status. Defaults: base=develop, head=current branch."
allowed-tools: Bash, Read, Glob, Grep
argument-hint: "[optional context or PR number]"
---

# GH PR

## Overview

Create or update GitHub Pull Requests with the gh CLI using a detailed body template, strict same-branch rules, and REST-first transport for PR list/create/update/view operations.

## Decision rules (must follow)

1. **Do not create or switch branches.** Always use the current branch as the PR head.
2. **Only `develop` may target `main`.** If the base is `main` and the head branch is anything other than `develop`, refuse to create the PR and instruct the user to merge into `develop` first.
3. **Check local working tree state before push/PR operations.**
   - `git status --porcelain`
   - If output is non-empty, pause and ask the user what to do (continue / abort / manual cleanup).
   - Do not run `git stash`, `git commit`, or `git clean` automatically unless explicitly requested.
4. **Check for an existing PR for the current head branch.**
   - Resolve the repo slug: `gh repo view --json nameWithOwner -q .nameWithOwner`
   - Primary lookup: `gh api repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>&per_page=100`
5. **If no PR exists** -> create a new PR.
6. **If any OPEN PR exists and is NOT merged** -> push only (do not create a new PR). Only update title/body/labels if user explicitly requests.
7. **If no OPEN unmerged PR exists and at least one PR is merged** -> perform post-merge commit check.
8. **If the only existing PRs are CLOSED and unmerged** -> create a new PR.
9. **If multiple PRs exist** -> use the most recently updated PR for reporting.

## Post-merge commit check (critical)

When all PRs for the head branch are merged, check for new commits after the merge:

1. Get `merge_commit_sha` from the most recent merged PR
2. Validate ancestry: `git merge-base --is-ancestor <merge_commit> HEAD`
3. Count commits after merge: `git rev-list --count <merge_commit>..HEAD`
4. Fallback to upstream: `git rev-list --count origin/<head>..HEAD`
5. Fallback to base: `git rev-list --count origin/<base>..HEAD`
6. Verify PR-worthy diff: `git diff --quiet origin/<base>...HEAD --`
7. Create new PR only if count > 0 and diff exists

## PR title rules (must follow)

1. **Format**: `<type>(<scope>): <subject>` (Conventional Commits)
2. **type**: `feat` / `fix` / `docs` / `chore` / `refactor` / `test` / `ci` / `perf`
3. **scope**: optional, short identifier (e.g., `gui`, `core`, `pty`)
4. **subject**: under 70 chars, imperative mood, no capital, no period
5. Branch prefix (e.g., `feat/`) must match the title type

## PR body rules (must follow)

| Section | Required | Notes |
|---------|----------|-------|
| Summary | **YES** | 1-3 bullets with what and why |
| Changes | **YES** | Enumerate by file or module |
| Testing | **YES** | Commands or exact manual steps |
| Closing Issues | **YES** | `Closes #N` or "None" |
| Related Issues / Links | **YES** | `#N` or URL, or "None" |
| Checklist | **YES** | Mark checked or N/A with reason |
| Context | Conditional | Required when 3+ files or non-trivial rationale |
| Risk / Impact | Conditional | Required for breaking/performance/rollback changes |
| Screenshots | Conditional | Required for UI changes only |
| Deployment | Optional | Only when deployment steps exist |
| Notes | Optional | Only when reviewers need extra context |

### Validation before creating PR

1. No required section may contain `TODO`.
2. Remove inapplicable conditional sections entirely.
3. Each Summary bullet must be a single sentence.
4. Changes must include file or module names.
5. Testing must be reproducible.
6. Add reason to every unchecked checklist item.
7. Closing Issues: `Closes #N` or `None` only.

## Workflow summary

1. Confirm repo + branches
2. Preflight: check working tree state
3. Fetch latest remote state
4. Check branch sync against base (merge if behind)
5. Check existing PR for head branch
6. Post-merge commit check (if applicable)
7. Push the head branch
8. Collect PR inputs and build body from template
9. Create or update the PR (REST-first, `gh pr` fallback)
10. Return PR URL
11. Post-PR CI/merge check via `gwt-pr-fix`

Load `references/workflow.md` for detailed step-by-step instructions.

Load `references/command-snippets.md` for the full bash workflow script.

## Comment formatting rules

- No escaped newline literals (`\n`) in comment text.
- Use real line breaks. Verify before posting.

## Issue Progress Comment Template

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

## References

- `references/workflow.md`: Detailed workflow steps
- `references/command-snippets.md`: Full bash workflow script
- `references/pr-body-template.md`: PR body template
