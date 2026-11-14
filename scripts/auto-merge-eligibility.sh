#!/usr/bin/env bash
set -euo pipefail

: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"
: "${PR_AUTHOR:?PR_AUTHOR is required}"
: "${HEAD_REF:?HEAD_REF is required}"
: "${HEAD_REPO:?HEAD_REPO is required}"
: "${BASE_REF:?BASE_REF is required}"
: "${REPO:?REPO is required}"

eligible=false
lookup_ok=true
source=none
reason=not_eligible
issue_number=

if [[ "$PR_AUTHOR" == "dependabot[bot]" ]]; then
  eligible=true
  source=dependabot
  reason=dependabot
elif [[ "$PR_AUTHOR" != "akiojin" ]]; then
  reason=unauthorized_author
elif [[ "$HEAD_REPO" != "$REPO" ]]; then
  reason=foreign_head_repository
elif [[ "$HEAD_REF" == "develop" && "$BASE_REF" == "main" ]]; then
  eligible=true
  source=release
  reason=release
elif [[ "$BASE_REF" != "develop" ]]; then
  reason=unsupported_base_branch
elif [[ "$HEAD_REF" =~ ^work/issue-([1-9][0-9]*)$ ]]; then
  source=issue
  issue_number="${BASH_REMATCH[1]}"
  if has_label="$(
    gh issue view "$issue_number" \
      --repo "$REPO" \
      --json labels \
      --jq 'any(.labels[]; .name == "auto-merge")'
  )"; then
    if [[ "$has_label" == "true" ]]; then
      eligible=true
      reason=issue_has_auto_merge_label
    else
      reason=issue_missing_auto_merge_label
    fi
  else
    lookup_ok=false
    reason=issue_label_lookup_failed
    printf 'auto-merge eligibility: failed to read Issue #%s labels\n' "$issue_number" >&2
  fi
else
  reason=unmanaged_owner_branch
fi

{
  printf 'eligible=%s\n' "$eligible"
  printf 'lookup_ok=%s\n' "$lookup_ok"
  printf 'source=%s\n' "$source"
  printf 'reason=%s\n' "$reason"
  printf 'issue_number=%s\n' "$issue_number"
} >> "$GITHUB_OUTPUT"
