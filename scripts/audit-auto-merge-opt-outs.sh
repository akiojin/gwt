#!/usr/bin/env bash
set -euo pipefail

: "${REPO:?REPO is required}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
eligibility_script="$script_dir/auto-merge-eligibility.sh"

if ! inventory="$({
  gh pr list \
    --repo "$REPO" \
    --state open \
    --limit 1000 \
    --json number,author,headRefName,headRepository,baseRefName,autoMergeRequest \
    --jq '.[] | select(.autoMergeRequest != null) | [.number, .author.login, .headRefName, (.headRepository.nameWithOwner // ""), .baseRefName] | join("|")'
} 2>&1)"; then
  printf 'auto-merge audit: failed to list open PRs: %s\n' "$inventory" >&2
  exit 1
fi

[[ -z "$inventory" ]] && exit 0

lookup_failed=false
while IFS='|' read -r pr_number pr_author head_ref head_repo base_ref; do
  output_file="$(mktemp)"
  GITHUB_OUTPUT="$output_file" \
    PR_AUTHOR="$pr_author" \
    HEAD_REF="$head_ref" \
    HEAD_REPO="$head_repo" \
    BASE_REF="$base_ref" \
    REPO="$REPO" \
    "$eligibility_script"

  eligible="$(sed -n 's/^eligible=//p' "$output_file")"
  lookup_ok="$(sed -n 's/^lookup_ok=//p' "$output_file")"
  reason="$(sed -n 's/^reason=//p' "$output_file")"
  issue_number="$(sed -n 's/^issue_number=//p' "$output_file")"
  rm -f "$output_file"

  if [[ "$eligible" != "true" ]]; then
    gh pr merge "$pr_number" --disable-auto --repo "$REPO"
    printf '::notice title=Auto-merge opt-out::Disarmed PR #%s (%s)\n' "$pr_number" "$reason"
  fi
  if [[ "$lookup_ok" != "true" ]]; then
    printf '::error title=Auto-merge eligibility unavailable::Could not read Issue #%s for PR #%s\n' \
      "$issue_number" "$pr_number"
    lookup_failed=true
  fi
done <<< "$inventory"

[[ "$lookup_failed" == "false" ]]
