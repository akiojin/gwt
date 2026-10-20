## Mode: Check

Detailed logic in [references/check-flow.md](check-flow.md).

**Read-only mode.** Do not create/switch branches, push, or create/edit PRs.

### Canonical Read Surface

- Current branch PR: `pr.current`
- PR detail: `pr.view`
- Checks: `pr.checks`
- Reviews: `pr.reviews`
- Unresolved threads: `pr.review_threads`
- Actions logs: `actions.logs` / `actions.job_logs`
- Actions re-run: `actions.rerun` (`run_id` + `failed_only`, or `job_id`)

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
