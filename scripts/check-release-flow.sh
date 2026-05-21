#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE="$ROOT/.github/workflows/release.yml"
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
