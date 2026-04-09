---
name: gwt-pr
description: "Use proactively after implementation work to create, check, or fix PRs. Auto-detects mode from branch PR state: no PR creates one, open PR pushes updates, CI failures/conflicts/reviews triggers fix mode. Triggers: 'create PR', 'check PR', 'fix CI', 'PR status'."
---

# gwt-pr — Unified PR Lifecycle Manager

## Overview

Single skill for the full PR lifecycle: create, check status, and fix blockers. Auto-detects the appropriate mode from current branch PR state, or accepts an explicit mode from the user.

REST-first `gh api` flows for all PR operations. GraphQL only for unresolved review threads and thread reply/resolve.

## Mode Auto-Detection

On invocation, resolve the current branch's PR state and select mode:

1. **Preflight** — Resolve repo, head, base, and list PRs via REST (see [Shared Preflight](#shared-preflight)).
2. **Route:**
   - User explicitly says "check status" / "PR status" / "is it merged?" --> **check** mode
   - User explicitly says "fix CI" / "fix the PR" / "resolve blockers" --> **fix** mode
   - User explicitly says "create PR" / "open PR" --> **create** mode (with smart skip if open PR exists)
   - No explicit mode --> auto-detect:
     - No PR exists --> **create**
     - Open unmerged PR exists, user has new commits --> **create** (push-only + post-push fix)
     - Open unmerged PR exists, user asks about status --> **check**
     - PR has CI failures / review comments / conflicts --> **fix**
     - All PRs merged with new commits --> **create**
     - All PRs merged, no diff --> report NO ACTION

## Shared Preflight

Every mode begins with these steps:

1. **Repo + branches:**
   - `git rev-parse --show-toplevel` / `git rev-parse --abbrev-ref HEAD`
   - Base defaults to `develop` unless user specifies.
2. **Branch protection:** Only `develop` may target `main`. Refuse any other branch targeting `main`.
3. **Working tree state:** `git status --porcelain`. If dirty, pause and present options (continue / abort / cleanup). Do not auto-commit/stash.
4. **Fetch:** `git fetch origin`
5. **PR lookup (REST-first):**
   - `repo_slug=$(gh repo view --json nameWithOwner -q .nameWithOwner)`
   - `owner="${repo_slug%%/*}"`
   - `gh api "repos/$repo_slug/pulls?state=all&head=$owner:$head&per_page=100"`
6. **Classify PR state:**
   - No PR --> `NO_PR`
   - Open + unmerged (`state == "open" && merged_at == null`) --> `UNMERGED_PR_EXISTS`
   - Only closed + unmerged --> `CLOSED_UNMERGED_ONLY`
   - At least one merged, no open unmerged --> perform post-merge commit check (see `references/check-flow.md`)

## Mode: Create

Detailed logic in `references/create-flow.md`.

### Decision Rules

1. **Do not create or switch branches.** Always use the current branch as head.
2. **If `UNMERGED_PR_EXISTS`** --> push only, return existing PR URL.
3. **If `NO_PR` or `CLOSED_UNMERGED_ONLY`** --> create new PR.
4. **If all PRs merged** --> post-merge commit check determines create vs no-action.
5. **Branch sync:** If behind `origin/$base`, merge `origin/$base` first (never rebase). Push after merge.

### PR Title Rules

- Format: `<type>(<scope>): <subject>` (Conventional Commits)
- type: `feat`/`fix`/`docs`/`chore`/`refactor`/`test`/`ci`/`perf`
- subject: imperative mood, under 70 chars, no capital, no period
- Branch prefix `feat/`/`fix/` must match title type

### PR Body Rules

- Template: `.claude/skills/gwt-pr/references/pr-body-template.md`
- Required sections: Summary, Changes, Testing, Closing Issues, Related Issues, Checklist
- Conditional sections (remove if N/A): Context, Risk/Impact, Screenshots, Deployment
- Remove all `<!-- GUIDE: ... -->` comments from final output
- No `TODO` in required sections; derive content from diff/Issues/SPECs before asking user
- `Closing Issues`: `Closes #N` or `None` only. Bare `#N` without keyword is forbidden.
- Add reason to every unchecked checklist item

### Create/Update Commands

- Primary: `gh api repos/<owner>/<repo>/pulls --method POST --input <json-file>`
- Fallback: `gh pr create -B <base> -H <head> --title "<title>" --body-file <file>`
- Update (only if user asks): `PATCH` via REST or `gh pr edit`

### Post-Create

After PR creation or push, automatically enter **fix** mode to check CI/conflicts/reviews.

## Mode: Check

Detailed logic in `references/check-flow.md`.

**Read-only mode.** Do not create/switch branches, push, or create/edit PRs.

### Output Contract

Human-readable summary using signal prefixes:

| Prefix | Action | Meaning |
|--------|--------|---------|
| `>>` | `CREATE PR` | Create a new PR |
| `>` | `PUSH ONLY` | Push to existing PR |
| `--` | `NO ACTION` | Nothing to do |
| `!!` | `MANUAL CHECK` | Manual check required |

Per-status templates:

- **NO_PR:** `>> CREATE PR -- No PR exists for <head> -> <base>.`
- **UNMERGED_PR_EXISTS:** `> PUSH ONLY -- Unmerged PR open for <head>.` + PR URL
- **CLOSED_UNMERGED_ONLY:** `>> CREATE PR -- No open PR; only closed unmerged PRs found.` + last closed PR
- **ALL_MERGED_WITH_NEW_COMMITS:** `>> CREATE PR -- <N> new commit(s) after last merge (#<pr>).`
- **ALL_MERGED_NO_PR_DIFF:** `-- NO ACTION -- All PRs merged, no PR-worthy diff.`
- **CHECK_FAILED:** `!! MANUAL CHECK -- Could not determine PR status.` + reason

Append `(!) Worktree has uncommitted changes.` when dirty.

### Post-Merge Commit Check

When all PRs are merged:

1. Get `merge_commit_sha` from latest merged PR.
2. Verify ancestry: `git merge-base --is-ancestor <merge_commit> HEAD`
3. If ancestor, count: `git rev-list --count <merge_commit>..HEAD`
4. Fallback chain: `origin/<head>..HEAD` --> `origin/<base>..HEAD`
5. Before recommending CREATE_PR, verify diff exists: `git diff --quiet origin/<base>...HEAD --`
6. Empty diff --> NO ACTION. Both fallbacks fail --> MANUAL CHECK.

## Mode: Fix

Detailed logic in `references/fix-flow.md`.

### Inspection Targets

- **CI failures:** REST check-runs + GitHub Actions log extraction
- **Merge conflicts:** `mergeable` / `mergeStateStatus` fields (CONFLICTING/DIRTY/BEHIND)
- **Reviewer comments:** REST reviews + inline comments + issue comments (full text, no truncation)
- **Unresolved threads:** GraphQL only
- **Change requests:** Reviews with `CHANGES_REQUESTED` state

### Diagnosis Report (mandatory format)

```text
## Diagnosis Report: PR #<number>

**Merge Verdict: BLOCKED | CLEAR**
Blocking items: <N>

---

### BLOCKING

#### B1. [CATEGORY] <1-line title>
- **What:** Factual statement
- **Where:** file_path:line / check name / branch ref
- **Evidence:** Verbatim quote from output
- **Action:** Specific fix with file path/command
- **Auto-fix:** Yes | No (needs confirmation)

---

### INFORMATIONAL
#### I1. [CATEGORY] <1-line title>
- **What / Note**

---

**Summary:** <N> blocking, <M> informational.
```

**Categories:** `CONFLICT`, `BRANCH-BEHIND`, `CI-FAILURE`, `CHANGE-REQUEST`, `UNRESOLVED-THREAD`, `REVIEW-COMMENT`

**Classification:**

- BLOCKING: merge conflicts, BEHIND, CI failure/cancelled/timed_out, CHANGES_REQUESTED, unresolved threads, unanswered review comments (any reviewer comment with no reply)
- INFORMATIONAL: review comments that already have a reply, pending CI, outdated threads

**Each CHANGE-REQUEST, UNRESOLVED-THREAD, and UNANSWERED-COMMENT is a separate B-item.**

### Execution Path

1. All `Auto-fix: Yes` --> proceed directly to fix.
2. Any `Auto-fix: No` --> ask user about those items only.

### Fix Implementation

- Apply fixes, commit, push.
- For BRANCH-BEHIND: `git fetch origin <base> && git merge origin/<base> && git push`
- For CONFLICT: inspect both sides, resolve if clear, ask user if ambiguous.

### Comment Response Policy

> **No reviewer comment may be left unanswered or unresolved.**

- Every unresolved thread MUST receive a reply AND be resolved.
- Reply content (every comment gets a reply, even if no code change was needed):
  - Fixed: "Fixed: <what was done>." with commit reference.
  - Not applicable: "Not applicable: <reason why no change is needed>."
  - Already addressed: "Already addressed in commit <sha>: <summary>."
  - Acknowledged: "Acknowledged: <brief response>." for informational comments.
- After replying, **resolve the conversation** via GraphQL
  `--reply-and-resolve` JSON array covering ALL threads.
- If `--reply-and-resolve` is not available, resolve manually via
  `gh api graphql` with `resolveReviewThread` mutation.
- **Verification:** After resolving, re-check that no unresolved
  threads remain. Unresolved threads block the Merge Verdict.

### Reviewer Notification (mandatory)

Post PR comment via REST summarizing all fixes. Fallback: `gh pr comment`.

### Verify Fix (mandatory)

Re-run inspection with `--mode all`. Loop until exit code 0.
- CI pending --> poll 30s intervals until complete.
- After fix push, re-poll for new CI run.

### Loop Safety Guard

Same CI check fails 3 consecutive iterations --> report to user, ask continue/abort/change approach.

## Diagnosis Report Anti-Patterns

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 blocking items detected" |
| "This might be causing..." | "Root cause: `<error from log>`" |
| "Consider fixing..." | "Action: Fix `<what>` in `<where>`" |
| "Various CI checks are failing" | "2 CI checks failing: `build`, `lint`" |

## Comment Formatting Rules

- No escaped newline literals (`\n`) in final comment text.
- Use real line breaks. Verify before posting.
- If raw escape sequences needed for explanation, use fenced code blocks only.

## Issue Progress Comment Template

When work is tracked in Issues:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

## Bundled Scripts

| Script | Path | Purpose |
|--------|------|---------|
| check_pr_status.py | `.claude/skills/gwt-pr-check/scripts/check_pr_status.py` | PR status check |
| inspect_pr_checks.py | `.claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py` | CI/conflict/review inspection |

### inspect_pr_checks.py Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `--repo` | `.` | Path inside the target Git repository |
| `--pr` | (current) | PR number or URL |
| `--mode` | `all` | `checks`, `conflicts`, `reviews`, `all` |
| `--max-lines` | 160 | Max lines for log snippets |
| `--context` | 30 | Context lines around failure markers |
| `--required-only` | false | Limit to required checks only |
| `--json` | false | Emit JSON output |
| `--reply-and-resolve` | (none) | JSON array of `{threadId, body}` |
| `--add-comment` | (none) | Post comment to PR |

## References

- `references/create-flow.md` -- PR creation workflow details
- `references/check-flow.md` -- PR status checking details
- `references/fix-flow.md` -- CI/conflict/review fix details
- `references/pr-body-template.md` -- PR body template
