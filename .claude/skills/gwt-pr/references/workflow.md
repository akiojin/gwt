# PR Creation Workflow (Detailed Steps)

## Step 1: Confirm repo + branches

- Repo root: `git rev-parse --show-toplevel`
- Current branch (head): `git rev-parse --abbrev-ref HEAD`
- Base branch defaults to `develop` unless user specifies.

## Step 2: Check local working tree state (preflight)

- Run `git status --porcelain`.
- If empty, continue.
- If non-empty, show detected files and ask the user to choose:
  - Continue as-is
  - Abort
  - Manual cleanup first (`git commit` / `git stash` / `git clean`) and rerun
- Proceed only when the user explicitly chooses continue.

## Step 3: Fetch latest remote state

- `git fetch origin` to ensure accurate comparison

## Step 4: Check branch sync against base (critical)

- Run `git rev-list --left-right --count "HEAD...origin/$base"`.
- Parse the result as `ahead behind`.
- If `behind == 0`, continue.
- If `behind > 0`, merge `origin/$base` into the current branch before PR creation.
- The update strategy is always `git merge origin/$base`; do not use rebase for this workflow.
- After merge, push the branch so the PR branch and worktree stay aligned with gwt's remote-first flow.
- If merge conflicts occur, inspect the affected files carefully, resolve only when the resulting behavior is coherent, and continue.
- If you cannot resolve the conflict with high confidence, stop and ask the user before proceeding.

## Step 5: Check existing PR for head branch

- Use the REST pull-request list endpoint as the primary transport.
- Use decision rules (see SKILL.md) to pick action.
- Treat `merged_at` as the source of truth for "merged".
- Treat `state == open && merged_at == null` as the source of truth for "existing active PR".

## Step 6: Post-merge commit check (when all PRs are merged)

- Get merge commit from the latest merged item returned by `GET /repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>`
- If the merge commit is an ancestor of `HEAD`, count `git rev-list --count <merge_commit>..HEAD`
- If the merge commit is missing or not an ancestor, count `git rev-list --count origin/<head>..HEAD` first
- If the upstream count is `0`, still count `git rev-list --count origin/<base>..HEAD` before concluding `NO_ACTION`
- If any count is greater than `0`, verify `git diff --quiet origin/<base>...HEAD --` before creating a PR
- If the base compare is empty, return `NO ACTION` because merged base commits alone do not justify a new PR
- Only if both fallback comparisons fail, stop with `MANUAL CHECK`
- If the selected count is 0 -> finish with message "No new changes since merge"
- If the selected count is >0 -> proceed to create new PR
- If neither comparison is usable -> stop with `MANUAL CHECK`

## Step 7: Ensure the head branch is pushed

- If no upstream: `git push -u origin <head>`
- Otherwise: `git push`

## Step 8: Collect PR inputs (for new PR or explicit update)

- Title, Summary, Context, Changes, Testing, Risk/Impact, Deployment, Screenshots, Related Links, Notes
- Optional: labels, reviewers, assignees, draft

## Step 9: Build PR body from template

- Read the template from the gwt-pr skill path (not the current project path):
  - `PR_BODY_TEMPLATE=".claude/skills/gwt-pr/references/pr-body-template.md"`
- Read `${PR_BODY_TEMPLATE}` and fill all required placeholders.
- Derive missing sections from the diff, linked Issues/SPECs, and executed tests before asking the user.
- **If a conditional section does not apply, remove the entire section.**
- **Remove any `<!-- GUIDE: ... -->` comments from the final output.**
- **If any required section still contains TODO after inference, ask only for the irreducible missing information.**

## Step 10: Create or update the PR

- Primary path (REST-first):
  - Create: `gh api repos/<owner>/<repo>/pulls --method POST --input <json-file>`
  - Update (only if user asked): `gh api repos/<owner>/<repo>/pulls/<number> --method PATCH --input <json-file>`
- Fallback path:
  - Create: `gh pr create -B <base> -H <head> --title "<title>" --body-file <file>`
  - Update: `gh pr edit <number> --title "<title>" --body-file <file>`
- If one path fails with a secondary rate limit or `was submitted too quickly`, retry with the other path before stopping.
- Keep the same title/body content across the REST and `gh pr` paths.

## Step 11: Return PR URL

- `gh api repos/<owner>/<repo>/pulls/<number> --jq .html_url`

## Step 12: Post-PR CI/merge check (automatic)

- After PR creation or push, load `.claude/skills/gwt-pr-fix/SKILL.md` and follow its workflow to inspect CI status, merge state, and review feedback.
- If all CI checks are still pending, poll (30s interval) until complete.
- If conflicts, review issues, or CI failures are detected, proceed with the gwt-pr-fix workflow to diagnose and fix.
