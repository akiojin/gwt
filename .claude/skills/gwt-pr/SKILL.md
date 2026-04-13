---
name: gwt-pr
description: "Use when a legacy prompt or internal handoff refers to gwt-pr. Prefer gwt-manage-pr as the visible PR lifecycle entrypoint."
---

# gwt-pr — Unified PR Lifecycle Manager

## Overview

Single skill for the full PR lifecycle: create, check status, and fix blockers. Auto-detects the appropriate mode from current branch PR state, or accepts an explicit mode from the user.

Visible owner: `gwt-manage-pr`.

Canonical agent-facing surface is `gwt pr ...` / `gwt actions ...` for PR inspection, create/update, and fix flows. The current implementation may still use GitHub REST / `gh` internally as transport, while GraphQL remains the transport for unresolved review threads and thread reply/resolve.

## Mode Auto-Detection

On invocation, run Shared Preflight, then route:

1. **Preflight** — always runs first (see [Shared Preflight](#shared-preflight)).
2. **Explicit mode** (user said "check" / "fix" / "create"):
   - "check status" / "PR status" / "is it merged?" → **check** mode
   - "fix CI" / "fix the PR" / "resolve blockers" → **fix** mode
   - "create PR" / "open PR" → **create** mode (with smart skip if open PR exists)
3. **Auto-detect** (no explicit mode) — use the commit-count-first
   decision from Preflight Step 7:
   - open PR + `mergeable: CONFLICTING|DIRTY|BEHIND` → **fix**
   - `N > 0` + no open PR → **create**
   - `N > 0` + open PR + clean merge state → **create** (push-only + post-push fix)
   - `N = 0` + open PR → **fix** (check CI / reviews / conflicts)
   - `N = 0` + no open PR → report **NO ACTION**

## Shared Preflight

Every mode begins with these steps. **The order is intentional — commit
count against the base branch comes before PR state lookup so that the
MERGED shortcut ("PR is done therefore nothing to do") is structurally
impossible.**

1. **Repo + branches:**
   - `git rev-parse --show-toplevel` / `git rev-parse --abbrev-ref HEAD`
   - Base defaults to `develop` unless user specifies.
2. **Branch protection:** Only `develop` may target `main`. Refuse any other branch targeting `main`.
3. **Working tree state:** `git status --porcelain`. If dirty, pause and present options (continue / abort / cleanup). Do not auto-commit/stash.
4. **Fetch:** `git fetch origin`
5. **Commit count against base (mandatory first check):**
   ```bash
   N=$(git rev-list --count "origin/$base..HEAD")
   ```
   > **Baseline ref rule**: ALWAYS compare against `origin/<base>`
   > (the PR target branch, default `origin/develop`). NEVER use
   > `origin/<head>` (the remote tracking branch of the current
   > branch) — that only tells you if commits are pushed, not
   > whether develop has your work. Confusing the two is the root
   > cause of the MERGED-state false NO ACTION bug.
6. **PR lookup + open-PR check:**
   - Prefer `gwt pr current` as the normal read path for the current branch.
   - Treat the literal line `no current pull request` as the canonical no-PR sentinel.
   - If create mode needs lower-level repo/head lookup, treat it as internal transport managed by the toolchain rather than part of the agent-facing workflow.
   - `has_open_pr` = any entry with `state == "open" && merged_at == null`
   - When an open PR exists, inspect `mergeable:`. `CONFLICTING`, `DIRTY`, and `BEHIND` are blocking merge states and immediately upgrade routing to **fix**.
7. **Route using the 2×2 matrix, with merge-state override:**

   | Commits (N) | Open PR? | Action |
   |---|---|---|
   | N > 0 | No | **CREATE** new PR |
   | N > 0 | Yes | **PUSH ONLY** to existing PR → then **FIX** |
   | N = 0 | Yes | **FIX** (CI / reviews / conflicts) |
   | N = 0 | No | **NO ACTION** |

   If the open PR reports `mergeable: CONFLICTING`, `DIRTY`, or `BEHIND`,
   use **FIX** immediately instead of push-only/create. Outside that
   override, PR state (MERGED / CLOSED) is not consulted.

## Mode: Create

Detailed logic in `references/create-flow.md`.

### Decision Rules

Create mode is entered from the Preflight 2×2 matrix when `N > 0`.

1. **Do not create or switch branches.** Always use the current branch as head.
2. **If open PR exists and is `CONFLICTING` / `DIRTY` / `BEHIND`** → enter Fix mode before any push-only path.
3. **If open PR exists and merge state is clean** → push only, return existing PR URL, enter Fix mode.
4. **If no open PR** → create new PR.
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

- Create: `gwt pr create --base <base> [--head <head>] --title "<title>" -f <file> [--label <label>]* [--draft]`
- Update: `gwt pr edit <number> [--title "<title>"] [-f <file>] [--add-label <label>]*`
- Transport note: the current implementation may still call GitHub REST / `gh` internally, but agent-facing workflow should stay on the `gwt pr` surface.

### Post-Create

After PR creation or push, automatically enter **fix** mode and use `gwt pr checks`, `gwt pr reviews`, `gwt pr review-threads`, and `gwt actions logs/job-logs` as the normal inspection path.

## Mode: Check

Detailed logic in `references/check-flow.md`.

**Read-only mode.** Do not create/switch branches, push, or create/edit PRs.

### Canonical Read Surface

- Current branch PR: `gwt pr current`
- PR detail: `gwt pr view <number>`
- Checks: `gwt pr checks <number>`
- Reviews: `gwt pr reviews <number>`
- Unresolved threads: `gwt pr review-threads <number>`
- Actions logs: `gwt actions logs --run <id>` / `gwt actions job-logs --job <id>`

### Output Contract

Human-readable summary using signal prefixes:

| Prefix | Action | Meaning |
|--------|--------|---------|
| `>>` | `CREATE PR` | Create a new PR |
| `>` | `PUSH ONLY` | Push to existing PR |
| `~` | `FIX` | Fix CI / reviews / conflicts on existing PR |
| `--` | `NO ACTION` | Nothing to do |
| `!!` | `MANUAL CHECK` | Manual check required |

Per-status templates:

Templates map directly from the Preflight 2×2 matrix:

- **N > 0, no open PR:** `>> CREATE PR -- <N> new commit(s) not covered by any PR.`
- **N > 0, open PR, clean merge state:** `> PUSH ONLY -- Unmerged PR #<number> open for <head>.` + PR URL
- **Open PR with blocking merge state:** `~ FIX -- PR #<number> is <mergeable>; resolve base sync/conflicts before push-only.`
- **N = 0, open PR:** `~ FIX -- PR #<number> open, checking CI/reviews/conflicts.`
- **N = 0, no open PR:** `-- NO ACTION -- No commits ahead of <base>, no open PR.`
- **Fallback:** `!! MANUAL CHECK -- Could not determine commit count.` + reason

Append `(!) Worktree has uncommitted changes.` when dirty.

### Commit Count (from Preflight Step 5)

The commit count `N = git rev-list --count origin/<base>..HEAD` is
computed in the Shared Preflight and is the primary routing signal.
Check mode simply reports it:

- `N > 0` + no open PR → `>> CREATE PR -- <N> new commit(s) not in any PR.`
- `N > 0` + open PR + clean merge state → `> PUSH ONLY -- Unmerged PR open.` + PR URL
- open PR + `mergeable: CONFLICTING|DIRTY|BEHIND` → `~ FIX -- PR is blocked by merge state.`
- `N = 0` + open PR → report CI / review / conflict status
- `N = 0` + no open PR → `-- NO ACTION`

The old "Post-Merge Commit Check" logic (merge_commit ancestry,
fallback chain) is subsumed by `git rev-list --count origin/<base>..HEAD`
which directly answers "does develop have all my work?" regardless of
PR state.

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
- After preparing the reply body, use `gwt pr review-threads reply-and-resolve <number> -f <file>` to reply to and resolve all unresolved threads on the PR.
- If that surface is unavailable, fall back to internal GraphQL transport with `resolveReviewThread`.
- **Verification:** After resolving, re-check that no unresolved
  threads remain. Unresolved threads block the Merge Verdict.

### Reviewer Notification (mandatory)

Post a PR summary comment via `gwt pr comment <number> -f <file>`.

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
