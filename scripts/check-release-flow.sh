#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE="$ROOT/.github/workflows/release.yml"
CLIFF="$ROOT/cliff.toml"
README_EN="$ROOT/README.md"
README_JA="$ROOT/README.ja.md"
INDEX_HTML="$ROOT/crates/gwt/web/index.html"
FRONTEND_BUNDLE="$ROOT/scripts/check-frontend-bundle.sh"

fail=0

if grep -q "sync-develop" "$RELEASE"; then
  echo "[FAIL] release.yml still contains sync-develop job"
  fail=1
fi

if grep -qE "publish-npm|npm publish|registry\.npmjs\.org|NPM_TOKEN" "$RELEASE"; then
  echo "[FAIL] release.yml still contains npm publishing"
  fail=1
fi

if ! grep -q 'filter(attribute="merge_commit", value=false)' "$CLIFF"; then
  echo "[FAIL] cliff.toml does not filter merge commits from release notes"
  fail=1
fi

if ! grep -qiE 'merge .*develop|sync.*develop|develop.*sync|develop.*同期' "$CLIFF"; then
  echo "[FAIL] cliff.toml does not skip develop-sync chore commits"
  fail=1
fi

if [ -f "$ROOT/package.json" ] || [ -f "$ROOT/pnpm-lock.yaml" ]; then
  echo "[FAIL] repository still contains npm package metadata"
  fail=1
fi

if [ -f "$ROOT/docs/release-guide.md" ] || [ -f "$ROOT/docs/release-guide.ja.md" ]; then
  echo "[FAIL] release-guide docs still exist"
  fail=1
fi

if grep -q "release-guide" "$README_EN" "$README_JA"; then
  echo "[FAIL] README still references release-guide"
  fail=1
fi

if ! grep -q '<script type="module" src="/app.js"></script>' "$INDEX_HTML"; then
  echo "[FAIL] index.html no longer points at the shared /app.js frontend bundle"
  fail=1
fi

PREPARE="$ROOT/.github/workflows/prepare-release.yml"
if [ ! -f "$PREPARE" ]; then
  echo "[FAIL] prepare-release.yml workflow is missing"
  fail=1
else
  if grep -qE "uses:.*create-release-pr" "$PREPARE"; then
    echo "[FAIL] prepare-release.yml reintroduced the external create-release-pr action"
    fail=1
  fi
  if grep -qiE "sync-develop|sync main into develop" "$PREPARE"; then
    echo "[FAIL] prepare-release.yml reintroduced a main->develop sync"
    fail=1
  fi
  if grep -qE "bumped-version" "$PREPARE"; then
    echo "[FAIL] prepare-release.yml uses git-cliff --bumped-version (full-history bump regresses versions)"
    fail=1
  fi
  if ! grep -q "compute_release_version.py" "$PREPARE"; then
    echo "[FAIL] prepare-release.yml does not use scripts/compute_release_version.py for version calc"
    fail=1
  fi
fi

if ! python3 "$ROOT/scripts/test_compute_release_version.py" >/dev/null 2>&1; then
  echo "[FAIL] compute_release_version unit tests failed"
  fail=1
fi

if ! python3 "$ROOT/scripts/test_release_issue_refs.py" >/dev/null 2>&1; then
  echo "[FAIL] release_issue_refs unit tests failed"
  fail=1
fi

if ! node "$ROOT/scripts/test_release_assets.cjs"; then
  echo "[FAIL] release asset contract check failed"
  fail=1
fi

if ! bash "$FRONTEND_BUNDLE"; then
  echo "[FAIL] frontend bundle syntax check failed"
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "[OK] release flow checks passed"
