# PR Creation Flow (Detailed)

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
- The update strategy is always `git merge origin/$base`; do not use rebase.
- After merge, push the branch so the PR branch and worktree stay aligned.
- If merge conflicts occur, inspect carefully, resolve only when coherent, and continue.
- If you cannot resolve with high confidence, stop and ask the user.

## Step 5: Check existing PR for head branch

- Use JSON operation `pr.current` as the normal path for current-branch PR discovery.
- Treat the literal line `no current pull request` as the canonical no-PR sentinel.
- Treat `merged_at` as the source of truth for "merged".
- Treat `state == open && merged_at == null` as the source of truth for "existing active PR".
- Treat open PR `mergeable: CONFLICTING`, `DIRTY`, and `BEHIND` as blocking states that must enter fix flow before any push-only path.

### Decision rules

1. **Do not create or switch branches.** Always use the current branch as head.
2. **Only `develop` may target `main`.** Refuse any other branch targeting `main`.
3. **No PR exists** --> create a new PR.
4. **Open unmerged PR exists and merge state is clean** --> push only (do not create a new PR). Only update title/body/labels if explicitly requested.
5. **Open unmerged PR exists and mergeable is `CONFLICTING` / `DIRTY` / `BEHIND`** --> switch to fix mode before push-only.
6. **No open unmerged PR; at least one merged** --> treat `git rev-list --count "origin/$base..HEAD"` as the source of truth for new work.
7. **Only closed unmerged PRs** --> create a new PR.

## Step 6: Decide PR readiness before pushing

- Normal PR delivery is Draft-first. Create every normal PR with
  `params.draft:true`, even when verification already passes.
- The only non-draft creation exemption is the exact Release PR with base
  `main` and head `develop`.
- Apply the Ready PR Gate only after CI, CodeRabbit, and review-thread handling
  have converged; it controls `pr.ready`, not normal PR creation.

## Step 7: Ensure the head branch is pushed

- Push the normal PR with its Draft-first intent recorded in the PR body and
  include the exact verification already run.
- If no upstream: `git push -u origin <head>`
- Otherwise: `git push`

## Step 8: Collect PR inputs

- Title, Summary, Context, Changes, Testing, PR Readiness, Risk/Impact, Deployment, Screenshots, Related Links, Notes
- Optional: labels, reviewers, assignees, draft
- Derive missing sections from the diff, linked Issues/SPECs, and executed tests before asking the user.

## Step 9: Build PR body from template

- Template path: the current runtime's `gwt-manage-pr/references/pr-body-template.md`
- Fill all required placeholders.
- If a conditional section does not apply, remove the entire section.
- Remove any `<!-- GUIDE: ... -->` comments from the final output.
- If any required section still contains TODO after inference, ask only for the irreducible missing information.

### Section classification

| Section | Required | Notes |
|---------|----------|-------|
| Summary | **YES** | 1-3 bullet points. Include both the what and the why. |
| Changes | **YES** | Enumerate changes by file or module. |
| Testing | **YES** | List commands or exact manual test steps. |
| PR Readiness | **YES** | Initial state is Draft for normal PRs. Include releaseable slice, Known blockers, and Remaining acceptance; update after `pr.ready`. |
| Closing Issues | **YES** | `Closes #N` or `None`. |
| Related Issues / Links | **YES** | Reference-only. `#N` or URL or `None`. |
| Checklist | **YES** | Review every item; mark checked or N/A with reason. |
| Context | Conditional | Required when 3+ files changed or non-trivial rationale. |
| Risk / Impact | Conditional | Required when breaking, performance-sensitive, or rollback needed. |
| Screenshots | Conditional | Required only for UI changes. |
| Deployment | Optional | Include only when deployment steps exist. |
| Notes | Optional | Include only when reviewers need extra context. |

### Validation (agent must check before creating PR)

1. Do not create a PR if any required section still contains `TODO`.
2. Each Summary bullet must be a single sentence. No vague wording.
3. Changes must reference specific file or module names.
4. Testing must be reproducible. No vague "tested."
5. PR Readiness must be Draft at normal creation. It may become Ready for review only after the Draft-first review flow and Ready PR Gate pass.
6. Add a reason comment to every unchecked checklist item.
7. Closing Issues: `Closes #N` or `None` only. Bare `#N` without keyword is forbidden.
8. If `#N` in Related Issues should auto-close, it must also appear in Closing Issues as `Closes #N`.

## Step 10: Create or update the PR

- Canonical path:
  - Create: JSON operation `pr.create` with `params.base`, optional `params.head`, `params.title`, `params.body`, optional `params.labels`, and `params.draft`.
    - Set `params.draft:true` for every normal PR.
    - Set `params.draft:false` only for the exact Release PR with base `main` and head `develop`.
  - Update: JSON operation `pr.edit` with `params.number`, optional `params.title`, optional `params.body`, and optional `params.add_labels`.
- Transport note: the current implementation may still use GitHub REST / `gh` internally, but agent-facing workflow should stay on gwtd JSON operations.

## Step 11: Return PR URL

- Read the URL from `pr.create` / `pr.edit` output, or use JSON operation `pr.view`.

## Step 12: Draft-first review delivery (automatic)

- After Draft PR creation or push, enter fix mode to inspect CI status, merge
  state, and review feedback.
- Poll until CI finishes and a submitted review from `coderabbitai[bot]`
  exists. Inspect `pr.reviews` and `pr.review_threads` on each meaningful
  transition.
- Address every finding and resolve all review threads, including outdated
  threads, with the response-and-resolve flow.
- Re-read `pr.checks`, `pr.reviews`, and `pr.review_threads`. When CI is green,
  the CodeRabbit review exists, and the unresolved count is zero, call
  `pr.ready`; repository automation may then arm auto-merge.
- Require a human approval only when the owning Issue explicitly says so. Keep
  the PR Draft while that approval is pending.

## Command Snippet

```bash
head=$(git rev-parse --abbrev-ref HEAD)
base=develop
PR_BODY_TEMPLATE="${GWT_MANAGE_PR_TEMPLATE:-}"

if [ -z "$PR_BODY_TEMPLATE" ]; then
  for candidate in \
    ".codex/skills/gwt-manage-pr/references/pr-body-template.md" \
    ".claude/skills/gwt-manage-pr/references/pr-body-template.md"
  do
    if [ -f "$candidate" ]; then
      PR_BODY_TEMPLATE="$candidate"
      break
    fi
  done
fi

base_compare_has_diff() {
  git diff --quiet "origin/$base...HEAD" -- 2>/dev/null
  case $? in
    0) echo "no" ;;
    1) echo "yes" ;;
    *) echo "" ;;
  esac
}

if [ ! -f "$PR_BODY_TEMPLATE" ]; then
  echo "PR template not found: $PR_BODY_TEMPLATE" >&2
  exit 1
fi

# Preflight: local working tree state
status_lines=$(git status --porcelain)
if [ -n "$status_lines" ] && [ "${ALLOW_DIRTY_WORKTREE:-0}" != "1" ]; then
  echo "Detected local uncommitted/untracked changes:" >&2
  echo "$status_lines" >&2
  exit 1
fi

git fetch origin
commit_count="$(git rev-list --count "origin/$base..HEAD" 2>/dev/null || echo "")"

# Check branch sync against base
divergence=$(git rev-list --left-right --count "HEAD...origin/$base" 2>/dev/null) || {
  echo "Failed to compare HEAD with origin/$base" >&2; exit 1
}
behind_count=$(echo "$divergence" | awk '{print $2}')

if [ "${behind_count:-0}" -gt 0 ]; then
  git merge "origin/$base" || { echo "Merge conflicts." >&2; exit 1; }
  git push -u origin "$head"
fi

# Check existing PRs (canonical surface)
if ! pr_summary="$("$GWT_BIN" <<'JSON' 2>/tmp/gwt-pr-current.err
{"schema_version":1,"operation":"pr.current","params":{}}
JSON
)"; then
  echo "Failed to resolve current PR:" >&2
  cat /tmp/gwt-pr-current.err >&2
  exit 1
fi
merge_state="$(printf '%s\n' "$pr_summary" | sed -n 's/^mergeable: //p' | head -n1)"

if printf '%s\n' "$pr_summary" | grep -qx 'no current pull request'; then
  action=create
elif printf '%s\n' "$merge_state" | grep -Eq '^(CONFLICTING|DIRTY|BEHIND)$'; then
  action=fix
elif printf '%s\n' "$pr_summary" | grep -q '\[OPEN\]'; then
  action=push_only
else
  if [ "${commit_count:-0}" -gt 0 ]; then
    action=create
  else
    action=none
  fi
fi

case "$action" in
  create)
    git push -u origin "$head"
    body_json="$(jq -Rs . < /tmp/pr-body.md)"
    "$GWT_BIN" <<JSON
{"schema_version":1,"operation":"pr.create","params":{"base":"$base","head":"$head","title":"...","body":$body_json,"draft":true}}
JSON
    ;;
  fix)
    printf '%s\n' "$pr_summary"
    echo "Existing PR is blocked by merge state; enter fix workflow before push-only." >&2
    ;;
  push_only)
    git push
    printf '%s\n' "$pr_summary"
    ;;
  none)
    echo "No action needed"
    ;;
esac
```
