# Decision Rules (must follow)

1. Resolve repository, `head` branch, and `base` branch.
   - `head`: current branch (`git rev-parse --abbrev-ref HEAD`)
   - `base`: default `develop` unless user specifies
2. Optionally collect local working tree state:
   - `git status --porcelain`
   - Report as context only; do not mutate files.
3. Fetch latest remote refs before comparing:
   - `git fetch origin`
4. List PRs for head branch:
   - Resolve repo slug: `gh repo view --json nameWithOwner -q .nameWithOwner`
   - Resolve owner from `<owner>/<repo>`
   - Primary lookup path:
     - `gh api repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>&per_page=100`
5. Classify:
   - No PR found -> `NO_PR` + recommended action `CREATE_PR`
   - Any OPEN PR where `mergedAt == null`
     -> `UNMERGED_PR_EXISTS` + recommended action `PUSH_ONLY`
   - Only CLOSED and unmerged PRs exist
     -> `CLOSED_UNMERGED_ONLY` + recommended action `CREATE_PR`
   - Otherwise, when at least one PR is merged -> perform post-merge commit check
6. Post-merge commit check (critical when all PRs are merged):
   - Select latest merged PR by `mergedAt`
   - Get merge commit SHA from `mergeCommit.oid`
   - Verify merge commit ancestry before counting:
     - `git merge-base --is-ancestor <merge_commit> HEAD`
   - If merge commit is ancestor of `HEAD`, count commits after merge:
     - `git rev-list --count <merge_commit>..HEAD`
   - If count > 0, verify `git diff --quiet origin/<base>...HEAD --` before recommending `CREATE_PR`
   - If count > 0 and the base compare is empty -> `ALL_MERGED_NO_PR_DIFF` + `NO_ACTION`
   - If count > 0 and the base compare has a diff -> `ALL_MERGED_WITH_NEW_COMMITS` + `CREATE_PR`
   - If count == 0 -> `ALL_MERGED_NO_PR_DIFF` + `NO_ACTION`
7. Fallback when merge commit SHA is missing or not an ancestor of `HEAD`:
   - First compare against branch upstream (preferred):
     - `git rev-list --count origin/<head>..HEAD`
   - Count > 0 -> verify `git diff --quiet origin/<base>...HEAD --` before recommending `CREATE_PR`
   - If upstream count is `0`, still compare against base:
     - `git rev-list --count origin/<base>..HEAD`
   - Base count > 0 and the base compare still has a diff -> `ALL_MERGED_WITH_NEW_COMMITS` + `CREATE_PR` (fallback)
   - Base count > 0 and the base compare is empty -> `ALL_MERGED_NO_PR_DIFF` + `NO_ACTION`
   - Base count == 0 -> `ALL_MERGED_NO_PR_DIFF` + `NO_ACTION`
   - If both comparisons fail -> `CHECK_FAILED` + `MANUAL_CHECK`
