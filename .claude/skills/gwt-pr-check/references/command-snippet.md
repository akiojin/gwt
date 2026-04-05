# Command Snippet (bash)

```bash
head="${HEAD_BRANCH:-$(git rev-parse --abbrev-ref HEAD)}"
base="${BASE_BRANCH:-develop}"

dirty=0
if [ -n "$(git status --porcelain)" ]; then
  dirty=1
fi

git fetch origin

repo_slug="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
owner="${repo_slug%%/*}"
pr_json="$(gh api "repos/$repo_slug/pulls?state=all&head=$owner:$head&per_page=100")"
pr_count="$(echo "$pr_json" | jq 'length')"
open_unmerged_count="$(echo "$pr_json" | jq 'map(select(.state == "open" and .merged_at == null)) | length')"
merged_count="$(echo "$pr_json" | jq 'map(select(.merged_at != null)) | length')"
closed_unmerged_pr="$(
  echo "$pr_json" \
    | jq -r 'map(select(.state == "closed" and .merged_at == null)) | sort_by(.updated_at) | last | .number // empty'
)"
closed_unmerged_pr_url="$(
  echo "$pr_json" \
    | jq -r 'map(select(.state == "closed" and .merged_at == null)) | sort_by(.updated_at) | last | .html_url // empty'
)"

if [ "$pr_count" -eq 0 ]; then
  status="NO_PR"
  action="CREATE_PR"
  reason="No PR found for head branch"
elif [ "$open_unmerged_count" -gt 0 ]; then
  status="UNMERGED_PR_EXISTS"
  action="PUSH_ONLY"
  reason="At least one OPEN PR for the head branch is not merged"
elif [ "$merged_count" -eq 0 ]; then
  status="CLOSED_UNMERGED_ONLY"
  action="CREATE_PR"
  reason="Only closed, unmerged PRs exist for the head branch"
else
  merge_commit="$(echo "$pr_json" | jq -r 'map(select(.merged_at != null)) | sort_by(.updated_at) | last | .merge_commit_sha')"
  merge_commit_ancestor=0
  if [ -n "$merge_commit" ] && [ "$merge_commit" != "null" ] && \
     git merge-base --is-ancestor "$merge_commit" HEAD 2>/dev/null; then
    merge_commit_ancestor=1
    new_commits="$(
      git rev-list --count "$merge_commit"..HEAD 2>/dev/null || echo ""
    )"
  else
    new_commits=""
  fi

  if [ -n "$new_commits" ]; then
    if [ "$new_commits" -gt 0 ]; then
      git diff --quiet "origin/$base...HEAD" -- 2>/dev/null
      compare_rc=$?
      if [ "$compare_rc" -eq 1 ]; then
        status="ALL_MERGED_WITH_NEW_COMMITS"
        action="CREATE_PR"
        reason="$new_commits commits found after last merge with a diff against origin/$base"
      elif [ "$compare_rc" -eq 0 ]; then
        status="ALL_MERGED_NO_PR_DIFF"
        action="NO_ACTION"
        reason="No PR-worthy diff remains against origin/$base"
      else
        status="CHECK_FAILED"
        action="MANUAL_CHECK"
        reason="Could not compare HEAD against origin/$base"
      fi
    else
      status="ALL_MERGED_NO_PR_DIFF"
      action="NO_ACTION"
      reason="No commits found after last merge"
    fi
  else
    upstream_commits="$(
      git rev-list --count "origin/$head"..HEAD 2>/dev/null || echo ""
    )"
    fallback_commits="$(
      git rev-list --count "origin/$base"..HEAD 2>/dev/null || echo ""
    )"

    if [ -n "$upstream_commits" ] && [ "$upstream_commits" -gt 0 ]; then
      git diff --quiet "origin/$base...HEAD" -- 2>/dev/null
      compare_rc=$?
      if [ "$compare_rc" -eq 1 ]; then
        status="ALL_MERGED_WITH_NEW_COMMITS"
        action="CREATE_PR"
        reason="Fallback check found commits ahead of origin/$head with a diff against origin/$base"
      elif [ "$compare_rc" -eq 0 ]; then
        status="ALL_MERGED_NO_PR_DIFF"
        action="NO_ACTION"
        reason="No PR-worthy diff remains against origin/$base"
      else
        status="CHECK_FAILED"
        action="MANUAL_CHECK"
        reason="Could not compare HEAD against origin/$base"
      fi
    elif [ -n "$fallback_commits" ]; then
      if [ "$fallback_commits" -gt 0 ]; then
        git diff --quiet "origin/$base...HEAD" -- 2>/dev/null
        compare_rc=$?
        if [ "$compare_rc" -eq 1 ]; then
          status="ALL_MERGED_WITH_NEW_COMMITS"
          action="CREATE_PR"
          reason="Fallback check found commits ahead of origin/$base with a diff"
        elif [ "$compare_rc" -eq 0 ]; then
          status="ALL_MERGED_NO_PR_DIFF"
          action="NO_ACTION"
          reason="No PR-worthy diff remains against origin/$base"
        else
          status="CHECK_FAILED"
          action="MANUAL_CHECK"
          reason="Could not compare HEAD against origin/$base"
        fi
      else
        status="ALL_MERGED_NO_PR_DIFF"
        action="NO_ACTION"
        reason="Fallback check found no commits ahead of origin/$head or origin/$base"
      fi
    else
      status="CHECK_FAILED"
      action="MANUAL_CHECK"
      reason="Could not resolve merge commit and fallback comparison failed"
    fi
  fi
fi

latest_merged_pr="$(
  echo "$pr_json" \
    | jq -r 'map(select(.merged_at != null)) | sort_by(.updated_at) | last | .number // empty'
)"
unmerged_pr="$(
  echo "$pr_json" \
    | jq -r 'map(select(.state == "open" and .merged_at == null)) | sort_by(.updated_at) | last | .number // empty'
)"
unmerged_pr_url="$(
  echo "$pr_json" \
    | jq -r 'map(select(.state == "open" and .merged_at == null)) | sort_by(.updated_at) | last | .html_url // empty'
)"

case "$status" in
  NO_PR)
    echo ">> CREATE PR — No PR exists for \`$head\` -> \`$base\`."
    ;;
  UNMERGED_PR_EXISTS)
    echo "> PUSH ONLY — Unmerged PR open for \`$head\`."
    echo "   PR: #$unmerged_pr $unmerged_pr_url"
    ;;
  CLOSED_UNMERGED_ONLY)
    echo ">> CREATE PR — No open PR exists for \`$head\` -> \`$base\`; only closed unmerged PRs were found."
    echo "   Last closed PR: #$closed_unmerged_pr $closed_unmerged_pr_url"
    ;;
  ALL_MERGED_WITH_NEW_COMMITS)
    n="${new_commits:-$upstream_commits}"
    echo ">> CREATE PR — $n new commit(s) after last merge (#$latest_merged_pr)."
    echo "   head: $head -> base: $base"
    ;;
  ALL_MERGED_NO_PR_DIFF)
    echo "-- NO ACTION — All PRs merged, no PR-worthy diff on \`$head\`."
    ;;
  *)
    echo "!! MANUAL CHECK — Could not determine PR status."
    echo "   Reason: $reason"
    echo "   head: $head -> base: $base"
    ;;
esac

if [ "$dirty" -eq 1 ]; then
  echo "   (!) Worktree has uncommitted changes."
fi
```
