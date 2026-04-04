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
2. **Only `develop` may target `main`.** If the base is `main` and the head branch is anything other than `develop`, **refuse to create the PR** and instruct the user to merge into `develop` first, then create a release PR from `develop`.
3. **Check local working tree state before push/PR operations.**
   - `git status --porcelain`
   - If output is non-empty (tracked or untracked changes), pause and ask the user what to do.
   - Present 3 options: continue as-is, abort, or manual cleanup then rerun.
   - **Do not** run `git stash`, `git commit`, or `git clean` automatically unless explicitly requested.
4. **Check for an existing PR for the current head branch.**
   - Resolve the repo slug first: `gh repo view --json nameWithOwner -q .nameWithOwner`
   - Resolve the repo owner from `<owner>/<repo>`
   - Primary lookup path:
     - `gh api repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>&per_page=100`
5. **If no PR exists** → create a new PR.
6. **If any OPEN PR exists and is NOT merged** (`state == open` and `mergedAt` is null) → push only and finish (do **not** create a new PR).
   - Only an OPEN PR blocks new PR creation.
   - CLOSED and unmerged PRs do **not** block creating a new PR.
   - Only update title/body/labels if the user explicitly requests changes.
7. **If no OPEN unmerged PR exists and at least one PR for the head is merged** → check for post-merge commits (see below).
8. **If the only existing PRs are CLOSED and unmerged** → create a new PR.
9. **If multiple PRs exist for the head** → use the most recently updated PR for reporting, but the create vs push decision is based on open-unmerged vs merged state.

## Post-merge commit check (critical)

When all PRs for the head branch are merged, you **must** check whether there are new commits after the merge:

1. **Get the merge commit SHA** of the most recent merged PR from the REST PR payload (`merge_commit_sha`).
2. **Validate merge commit ancestry first**: `git merge-base --is-ancestor <merge_commit> HEAD`
3. **If the merge commit is an ancestor of `HEAD`**, count commits after the merge:
   - `git rev-list --count <merge_commit>..HEAD`
4. **If the merge commit is missing or not an ancestor of `HEAD`**, fallback to the branch upstream first:
   - `git rev-list --count origin/<head>..HEAD`
5. **If the upstream comparison returns `0` or fails**, compare against the base branch:
   - `git rev-list --count origin/<base>..HEAD`
6. **Before creating a PR from post-merge commit counts, verify the branch still differs from the base branch**:
   - `git diff --quiet origin/<base>...HEAD --`
   - Exit code `1` → a PR-worthy diff exists; proceed
   - Exit code `0` → no PR-worthy diff remains; finish with `NO ACTION`
   - Any other failure → stop with `MANUAL CHECK`
7. **Decision**:
   - If the selected count is greater than 0 → create a new PR
   - If both upstream and base comparisons report `0` → report "No new changes since merge" and finish
   - If both fallback comparisons fail → stop and report `MANUAL CHECK`

### Why this matters

- **Scenario A**: PR merged → user makes local changes → pushes → changes are NOT in the merged PR
  - Without this check, the changes would be lost or require manual intervention
- **Scenario B**: PR merged → user says "create PR" without new changes → would create empty/duplicate PR
  - This check prevents unnecessary PR creation

## PR title rules (must follow)

1. **Format**: `<type>(<scope>): <subject>` — follow Conventional Commits.
2. **type**: must be one of `feat` / `fix` / `docs` / `chore` / `refactor` / `test` / `ci` / `perf`.
3. **scope**: optional. Use a short scope that clearly identifies the affected area (for example: `gui`, `core`, `pty`).
4. **subject**: keep it within 70 characters. Use the imperative mood (for example: "add ..." / "fix ..."). Do not capitalize the first letter or end with a period.
5. If the branch name has a prefix such as `feat/` or `fix/`, **the title type must match that prefix**.

## PR body rules (must follow)

### Section classification

| Section | Required | Notes |
|---------|----------|-------|
| Summary | **YES** | 1-3 bullet points. Include both the what and the why. |
| Changes | **YES** | Enumerate changes by file or module. |
| Testing | **YES** | List the commands run or the exact manual test steps. |
| Closing Issues | **YES** | `Closes #N` 形式。クローズ対象がなければ "None"。 |
| Related Issues / Links | **YES** | 参照のみ（自動クローズしない）。 |
| Checklist | **YES** | Review every item and mark it checked or N/A. |
| Context | Conditional | Required when 3 or more files changed or the rationale is non-trivial. |
| Risk / Impact | Conditional | Required when the change is breaking, performance-sensitive, or needs rollback steps. |
| Screenshots | Conditional | Required only for UI changes. |
| Deployment | Optional | Include only when deployment steps exist. |
| Notes | Optional | Include only when reviewers need extra context. |

### Validation (agent must check before creating PR)

1. **Do not create a PR if any required section still contains `TODO`.**
2. If a conditional section does not apply, remove the entire section instead of leaving an empty TODO.
3. Each Summary bullet must be a **single sentence**. Do not use vague wording such as "several changes" or "various fixes."
4. Changes must be specific and include the changed file or module names.
5. Testing must be reproducible. Do not use vague wording such as "tested."
6. Add a reason comment to every unchecked checklist item (for example: `- [ ] Docs updated — N/A: no user-facing change`).
7. Related Issues must be written as `#123` or as a URL. If nothing applies, explicitly write "None".
8. Closing Issues セクションは `Closes #N` または `None` のみ許可。`- #N`（キーワードなし）は不可。
9. `Related Issues / Links` に `#N` があり、その Issue をリリースで閉じたい場合は、同じ番号を `Closing Issues` にも `Closes #N` で必ず記載する。`Related Issues / Links` のみでは auto-close されない。

## Issue/PR Comment Formatting (must follow)

- Final comment text must not contain escaped newline literals such as `\n`.
- Use real line breaks in comment bodies. Do not rely on escaped sequences for formatting.
- Before posting, verify the final body does not accidentally include escaped control sequences (`\n`, `\t`).
- If a raw escape sequence must be shown for explanation, include it only inside a fenced code block and clarify it is intentional.

## Issue Progress Comment Template (required for issue-based work)

When work is tracked in GitHub Issues, progress updates must use this template:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

- Post updates at least when starting work, after meaningful progress, and when blocked/unblocked.
- In `Next`, explicitly state blockers or the immediate next action.

## Workflow (recommended)

1. **Confirm repo + branches**
   - Repo root: `git rev-parse --show-toplevel`
   - Current branch (head): `git rev-parse --abbrev-ref HEAD`
   - Base branch defaults to `develop` unless user specifies.

2. **Check local working tree state (preflight)**
   - Run `git status --porcelain`.
   - If empty, continue.
   - If non-empty, show detected files and ask the user to choose:
     - Continue as-is
     - Abort
     - Manual cleanup first (`git commit` / `git stash` / `git clean`) and rerun
   - Proceed only when the user explicitly chooses continue.

3. **Fetch latest remote state**
   - `git fetch origin` to ensure accurate comparison

4. **Check branch sync against base (critical)**
   - Run `git rev-list --left-right --count "HEAD...origin/$base"`.
   - Parse the result as `ahead behind`.
   - If `behind == 0`, continue.
   - If `behind > 0`, merge `origin/$base` into the current branch before PR creation.
   - The update strategy is always `git merge origin/$base`; do not use rebase for this workflow.
   - After merge, push the branch so the PR branch and worktree stay aligned with gwt's remote-first flow.
   - If merge conflicts occur, inspect the affected files carefully, resolve only when the resulting behavior is coherent, and continue.
   - If you cannot resolve the conflict with high confidence, stop and ask the user before proceeding.

5. **Check existing PR for head branch**
   - Use the REST pull-request list endpoint as the primary transport.
   - Use decision rules above to pick action.
   - Treat `merged_at` as the source of truth for "merged".
   - Treat `state == open && merged_at == null` as the source of truth for "existing active PR".

6. **If no OPEN unmerged PR exists and at least one PR is merged, perform post-merge commit check**
   - Get merge commit from the latest merged item returned by `GET /repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>`
   - If the merge commit is an ancestor of `HEAD`, count `git rev-list --count <merge_commit>..HEAD`
   - If the merge commit is missing or not an ancestor, count `git rev-list --count origin/<head>..HEAD` first
   - If the upstream count is `0`, still count `git rev-list --count origin/<base>..HEAD` before concluding `NO_ACTION`
   - If any count is greater than `0`, verify `git diff --quiet origin/<base>...HEAD --` before creating a PR
   - If the base compare is empty, return `NO ACTION` because merged base commits alone do not justify a new PR
   - Only if both fallback comparisons fail, stop with `MANUAL CHECK`
   - If the selected count is 0 → finish with message "No new changes since merge"
   - If the selected count is >0 → proceed to create new PR
   - If neither comparison is usable → stop with `MANUAL CHECK`

7. **Ensure the head branch is pushed**
   - If no upstream: `git push -u origin <head>`
   - Otherwise: `git push`

8. **Collect PR inputs (for new PR or explicit update)**
   - Title, Summary, Context, Changes, Testing, Risk/Impact, Deployment, Screenshots, Related Links, Notes
   - Optional: labels, reviewers, assignees, draft

9. **Build PR body from template**
   - Read the template from the gwt-pr skill path (not the current project path):
     - `PR_BODY_TEMPLATE=".claude/skills/gwt-pr/references/pr-body-template.md"`
   - Read `${PR_BODY_TEMPLATE}` and fill all required placeholders.
   - Derive missing sections from the diff, linked Issues/SPECs, and executed tests before asking the user.
   - **If a conditional section does not apply, remove the entire section.**
   - **Remove any `<!-- GUIDE: ... -->` comments from the final output.**
   - **If any required section still contains TODO after inference, ask only for the irreducible missing information.**

10. **Create or update the PR**
    - Primary path (REST-first):
      - Create: `gh api repos/<owner>/<repo>/pulls --method POST --input <json-file>`
      - Update (only if user asked): `gh api repos/<owner>/<repo>/pulls/<number> --method PATCH --input <json-file>`
    - Fallback path:
      - Create: `gh pr create -B <base> -H <head> --title "<title>" --body-file <file>`
      - Update: `gh pr edit <number> --title "<title>" --body-file <file>`
    - If one path fails with a secondary rate limit or `was submitted too quickly`, retry with the other path before stopping.
    - Keep the same title/body content across the REST and `gh pr` paths.

11. **Return PR URL**
    - `gh api repos/<owner>/<repo>/pulls/<number> --jq .html_url`

12. **Post-PR CI/merge check (automatic).**
    - After PR creation or push, load `.claude/skills/gwt-pr-fix/SKILL.md` and follow its workflow to inspect CI status, merge state, and review feedback.
    - If all CI checks are still pending, poll (30s interval) until complete.
    - If conflicts, review issues, or CI failures are detected, proceed with the gwt-pr-fix workflow to diagnose and fix.

## Command snippets (bash)

```bash
head=$(git rev-parse --abbrev-ref HEAD)
base=develop
PR_BODY_TEMPLATE=".claude/skills/gwt-pr/references/pr-body-template.md"

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
  echo "Choose one before continuing: continue as-is, abort, or manual cleanup then rerun." >&2
  echo "Set ALLOW_DIRTY_WORKTREE=1 only after explicit user confirmation to continue." >&2
  exit 1
fi

# Fetch latest remote state
git fetch origin

# Check branch sync against base before PR creation
divergence=$(git rev-list --left-right --count "HEAD...origin/$base" 2>/dev/null) || {
  echo "Failed to compare HEAD with origin/$base" >&2
  exit 1
}
ahead_count=$(echo "$divergence" | awk '{print $1}')
behind_count=$(echo "$divergence" | awk '{print $2}')

if [ "${behind_count:-0}" -gt 0 ]; then
  echo "Merging origin/$base into $head before PR creation"
  git merge "origin/$base" || {
    echo "Base-branch merge produced conflicts. Inspect and resolve them before continuing." >&2
    exit 1
  }
  git push -u origin "$head"
fi

# Check existing PRs for the head branch (REST-first)
repo_slug=$(gh repo view --json nameWithOwner -q .nameWithOwner)
owner="${repo_slug%%/*}"
pr_json=$(gh api "repos/$repo_slug/pulls?state=all&head=$owner:$head&per_page=100")
pr_count=$(echo "$pr_json" | jq 'length')
open_unmerged_count=$(echo "$pr_json" | jq 'map(select(.state == "open" and .merged_at == null)) | length')
merged_count=$(echo "$pr_json" | jq 'map(select(.merged_at != null)) | length')

if [ "$pr_count" -eq 0 ]; then
  action=create
elif [ "$open_unmerged_count" -gt 0 ]; then
  action=push_only
elif [ "$merged_count" -eq 0 ]; then
  # Only closed, unmerged PRs exist for this head. They do not block a new PR.
  action=create
else
  # All PRs are merged - check for post-merge commits
  merge_commit=$(echo "$pr_json" | jq -r 'map(select(.merged_at != null)) | sort_by(.updated_at) | last | .merge_commit_sha')
  new_commits=""

  if [ -n "$merge_commit" ] && [ "$merge_commit" != "null" ] && \
     git merge-base --is-ancestor "$merge_commit" HEAD 2>/dev/null; then
    new_commits=$(git rev-list --count "$merge_commit"..HEAD 2>/dev/null || echo "")
  fi

  if [ -n "$new_commits" ]; then
    if [ "$new_commits" -gt 0 ]; then
      compare_has_diff="$(base_compare_has_diff)"
      if [ "$compare_has_diff" = "yes" ]; then
        echo "Found $new_commits commit(s) after merge and a diff against origin/$base - creating new PR"
        action=create
      elif [ "$compare_has_diff" = "no" ]; then
        echo "No PR-worthy diff remains against origin/$base - nothing to do"
        action=none
      else
        echo "Manual check required: could not compare HEAD against origin/$base" >&2
        action=manual_check
      fi
    else
      echo "No new commits since merge - nothing to do"
      action=none
    fi
  else
    upstream_commits=$(git rev-list --count "origin/$head"..HEAD 2>/dev/null || echo "")

    if [ -n "$upstream_commits" ] && [ "$upstream_commits" -gt 0 ]; then
      compare_has_diff="$(base_compare_has_diff)"
      if [ "$compare_has_diff" = "yes" ]; then
        echo "Found $upstream_commits commit(s) ahead of origin/$head and a diff against origin/$base - creating new PR"
        action=create
      elif [ "$compare_has_diff" = "no" ]; then
        echo "No PR-worthy diff remains against origin/$base - nothing to do"
        action=none
      else
        echo "Manual check required: could not compare HEAD against origin/$base" >&2
        action=manual_check
      fi
    else
      fallback_commits=$(git rev-list --count "origin/$base"..HEAD 2>/dev/null || echo "")

      if [ -n "$fallback_commits" ]; then
        if [ "$fallback_commits" -gt 0 ]; then
          compare_has_diff="$(base_compare_has_diff)"
          if [ "$compare_has_diff" = "yes" ]; then
            echo "Upstream comparison unavailable; found $fallback_commits commit(s) ahead of origin/$base with a diff - creating new PR"
            action=create
          elif [ "$compare_has_diff" = "no" ]; then
            echo "No PR-worthy diff remains against origin/$base - nothing to do"
            action=none
          else
            echo "Manual check required: could not compare HEAD against origin/$base" >&2
            action=manual_check
          fi
        else
          echo "No commits ahead of origin/$head or origin/$base - nothing to do"
          action=none
        fi
      else
        echo "Manual check required: could not determine whether new commits exist after the last merge" >&2
        action=manual_check
      fi
    fi
  fi
fi

# Execute action
case "$action" in
  create)
    cp "$PR_BODY_TEMPLATE" /tmp/pr-body.md

    git push -u origin "$head"
    jq -n --arg title "..." --arg head "$head" --arg base "$base" --rawfile body /tmp/pr-body.md \
      '{title:$title, head:$head, base:$base, body:$body}' >/tmp/pr-create.json
    gh api "repos/$repo_slug/pulls" --method POST --input /tmp/pr-create.json || {
      gh pr create -B "$base" -H "$head" --title "..." --body-file /tmp/pr-body.md
    }
    ;;
  push_only)
    echo "Existing unmerged PR found - pushing changes only"
    git push
    echo "$pr_json" | jq -r 'map(select(.merged_at == null)) | sort_by(.updated_at) | last | .html_url'
    ;;
  none)
    echo "No action needed - no new changes since last merge"
    ;;
  manual_check)
    echo "Manual check required - post-merge commit status could not be determined" >&2
    exit 1
    ;;
esac
```

## References

- `.claude/skills/gwt-pr/references/pr-body-template.md`: PR body template
