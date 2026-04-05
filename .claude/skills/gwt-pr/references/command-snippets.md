# PR Command Snippets

## Full workflow script

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
