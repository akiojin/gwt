# PR Check Flow (Detailed)

**Read-only mode.** Do not create/switch branches, push, or create/edit PRs.

## Decision Rules

1. Resolve repository, `head` branch, and `base` branch.
   - `head`: current branch (`git rev-parse --abbrev-ref HEAD`)
   - `base`: default `develop` unless user specifies
2. Optionally collect local working tree state:
   - `git status --porcelain`
   - Report as context only; do not mutate files.
3. Fetch latest remote refs: `git fetch origin`
4. Resolve the current branch PR with `gwt pr current`.
   - Use `gwt pr view <number>` for detailed inspection when a PR exists.
   - Treat the literal line `no current pull request` as the canonical no-PR sentinel.
   - If the PR reports `mergeable: CONFLICTING`, `DIRTY`, or `BEHIND`, route to `FIX` immediately.
   - Treat any lower-level GitHub REST lookup as internal transport, not the normal path.
5. Classify:
   - No PR found --> `NO_PR` + `CREATE_PR`
   - Any OPEN PR where `merged_at == null` and merge state is clean --> `UNMERGED_PR_EXISTS` + `PUSH_ONLY`
   - Any OPEN PR where `merged_at == null` and `mergeable: CONFLICTING|DIRTY|BEHIND` --> `OPEN_PR_BLOCKED` + `FIX`
   - Only CLOSED and unmerged PRs exist --> `CLOSED_UNMERGED_ONLY` + `CREATE_PR`
   - At least one merged, no open unmerged --> post-merge commit check

## Output Contract

Human-readable summary by default. JSON only if explicitly requested.

### Status values

| Status | Prefix | Action |
|--------|--------|--------|
| `NO_PR` | `>>` | `CREATE PR` |
| `UNMERGED_PR_EXISTS` | `>` | `PUSH ONLY` |
| `OPEN_PR_BLOCKED` | `~` | `FIX` |
| `CHECK_FAILED` | `!!` | `MANUAL CHECK` |

### Per-status format

- **NO_PR:**
  `>> CREATE PR -- No PR exists for <head> -> <base>.`

- **UNMERGED_PR_EXISTS:**

  ```text
  > PUSH ONLY -- Unmerged PR open for `<head>`.
     PR: #<number> <url>
  ```

- **OPEN_PR_BLOCKED:**

  ```text
  ~ FIX -- PR #<number> is blocked by `mergeable: CONFLICTING|DIRTY|BEHIND`.
     PR: #<number> <url>
  ```

- **CLOSED_UNMERGED_ONLY:**

  ```text
  >> CREATE PR -- No open PR exists for <head> -> <base>; only closed unmerged PRs were found.
     Last closed PR: #<number> <url>
  ```

- **CHECK_FAILED:**

  ```text
  !! MANUAL CHECK -- Could not determine PR status.
     Reason: <reason>
     head: <head> -> base: <base>
  ```

Append when worktree is dirty:

```text
   (!) Worktree has uncommitted changes.
```

## Command Snippet

```bash
head="${HEAD_BRANCH:-$(git rev-parse --abbrev-ref HEAD)}"
base="${BASE_BRANCH:-develop}"

dirty=0
if [ -n "$(git status --porcelain)" ]; then dirty=1; fi

git fetch origin

commit_count="$(git rev-list --count "origin/$base..HEAD")"
pr_summary="$(gwt pr current 2>/tmp/gwt-pr-current.err || true)"
merge_state="$(printf '%s\n' "$pr_summary" | sed -n 's/^mergeable: //p' | head -n1)"

if printf '%s\n' "$pr_summary" | grep -qx 'no current pull request'; then
  status="NO_PR"; action="CREATE_PR"
elif printf '%s\n' "$merge_state" | grep -Eq '^(CONFLICTING|DIRTY|BEHIND)$'; then
  status="OPEN_PR_BLOCKED"; action="FIX"
elif printf '%s\n' "$pr_summary" | grep -q '\[OPEN\]'; then
  status="UNMERGED_PR_EXISTS"; action="PUSH_ONLY"
else
  status="NO_ACTION"; action="NO_ACTION"
fi

# For checks/reviews/threads, continue with:
# gwt pr view <number>
# gwt pr checks <number>
# gwt pr reviews <number>
# gwt pr review-threads <number>
```
