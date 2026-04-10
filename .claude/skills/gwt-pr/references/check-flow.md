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
   - Treat any lower-level GitHub REST lookup as internal transport, not the normal path.
5. Classify:
   - No PR found --> `NO_PR` + `CREATE_PR`
   - Any OPEN PR where `merged_at == null` --> `UNMERGED_PR_EXISTS` + `PUSH_ONLY`
   - Only CLOSED and unmerged PRs exist --> `CLOSED_UNMERGED_ONLY` + `CREATE_PR`
   - At least one merged, no open unmerged --> post-merge commit check

## Post-Merge Commit Check (critical)

When all PRs for the head are merged:

1. Select latest merged PR by `merged_at`.
2. Get `merge_commit_sha` from REST payload.
3. Verify merge commit ancestry: `git merge-base --is-ancestor <merge_commit> HEAD`
4. **If ancestor of HEAD**, count commits after merge:
   - `git rev-list --count <merge_commit>..HEAD`
5. **If count > 0**, verify diff against base:
   - `git diff --quiet origin/<base>...HEAD --`
   - Exit 1 (diff exists) --> `ALL_MERGED_WITH_NEW_COMMITS` + `CREATE_PR`
   - Exit 0 (no diff) --> `ALL_MERGED_NO_PR_DIFF` + `NO_ACTION`
   - Other --> `CHECK_FAILED` + `MANUAL_CHECK`
6. **If count == 0** --> `ALL_MERGED_NO_PR_DIFF` + `NO_ACTION`
7. **Fallback** when merge commit SHA missing or not ancestor:
   - First: `git rev-list --count origin/<head>..HEAD`
   - If > 0 and diff exists --> `CREATE_PR`
   - If upstream count is 0, still check: `git rev-list --count origin/<base>..HEAD`
   - Base count > 0 and diff exists --> `CREATE_PR` (fallback)
   - Base count > 0, no diff --> `NO_ACTION`
   - Base count == 0 --> `NO_ACTION`
   - Both fail --> `CHECK_FAILED` + `MANUAL_CHECK`

### Why this matters

- **Scenario A**: PR merged --> user pushes local changes --> changes NOT in merged PR. Without this check, changes would be lost.
- **Scenario B**: PR merged --> user says "create PR" without new changes --> would create empty/duplicate PR.

## Output Contract

Human-readable summary by default. JSON only if explicitly requested.

### Status values

| Status | Prefix | Action |
|--------|--------|--------|
| `NO_PR` | `>>` | `CREATE PR` |
| `UNMERGED_PR_EXISTS` | `>` | `PUSH ONLY` |
| `CLOSED_UNMERGED_ONLY` | `>>` | `CREATE PR` |
| `ALL_MERGED_WITH_NEW_COMMITS` | `>>` | `CREATE PR` |
| `ALL_MERGED_NO_PR_DIFF` | `--` | `NO ACTION` |
| `CHECK_FAILED` | `!!` | `MANUAL CHECK` |

### Per-status format

- **NO_PR:**
  `>> CREATE PR -- No PR exists for <head> -> <base>.`

- **UNMERGED_PR_EXISTS:**

  ```text
  > PUSH ONLY -- Unmerged PR open for `<head>`.
     PR: #<number> <url>
  ```

- **CLOSED_UNMERGED_ONLY:**

  ```text
  >> CREATE PR -- No open PR exists for <head> -> <base>; only closed unmerged PRs were found.
     Last closed PR: #<number> <url>
  ```

- **ALL_MERGED_WITH_NEW_COMMITS:**

  ```text
  >> CREATE PR -- <N> new commit(s) after last merge (#<pr_number>).
     head: <head> -> base: <base>
  ```

- **ALL_MERGED_NO_PR_DIFF:**
  `-- NO ACTION -- All PRs merged, no PR-worthy diff on <head>.`

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

if [ -z "$pr_summary" ]; then
  status="NO_PR"; action="CREATE_PR"
elif printf '%s\n' "$pr_summary" | grep -q '\[OPEN\]'; then
  status="UNMERGED_PR_EXISTS"; action="PUSH_ONLY"
else
  status="CHECK_FAILED"; action="MANUAL_CHECK"
fi

# For checks/reviews/threads, continue with:
# gwt pr view <number>
# gwt pr checks <number>
# gwt pr reviews <number>
# gwt pr review-threads <number>
```
