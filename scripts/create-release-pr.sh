#!/usr/bin/env bash
# create-release-pr.sh
# developブランチからmain向けのリリースPRを自動生成する

set -euo pipefail

if ! command -v gh >/dev/null 2>&1; then
  echo "ERROR: GitHub CLI (gh) が見つかりません" >&2
  exit 1
fi

CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$CURRENT_BRANCH" != "develop" ]]; then
  echo "ERROR: developブランチで実行してください" >&2
  exit 1
fi

echo "同期: origin/develop を取得"
git fetch origin develop

echo "Pull: develop の最新を取得"
git pull --ff-only origin develop

TITLE="Release: $(date +%Y-%m-%d)"
read -r -d '' BODY <<'BODY'
Automatic release PR from develop to main.

After merge, semantic-release will:
- Determine version from Conventional Commits
- Update package.json and CHANGELOG.md
- Commit the release artifacts to main
- Create the vX.Y.Z tag and GitHub Release
- Publish to npm (enable via configuration)
BODY

EXISTING_URL=$(gh pr list --base main --head develop --state open --json url --jq '.[0].url' 2>/dev/null || true)
if [[ -n "$EXISTING_URL" ]]; then
  PR_NUMBER=$(gh pr view "$EXISTING_URL" --json number --jq '.number')
  gh pr edit "$PR_NUMBER" \
    --title "$TITLE" \
    --body "$BODY" \
    --add-label release \
    --add-label auto-merge >/dev/null
  echo "既存のリリースPRを更新しました: $EXISTING_URL"
  exit 0
fi

echo "PR作成: develop → main"
PR_URL=$(gh pr create \
  --base main \
  --head develop \
  --title "$TITLE" \
  --body "$BODY" \
  --label release \
  --label auto-merge)

PR_NUMBER=$(gh pr view "$PR_URL" --json number --jq '.number')
gh pr merge "$PR_NUMBER" --auto --merge >/dev/null || true

if [[ -z "$PR_URL" ]]; then
  echo "PRの作成に失敗しました" >&2
  exit 1
fi

echo "✓ Release PR created successfully"
echo "$PR_URL"
