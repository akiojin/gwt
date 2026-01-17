#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREPARE="$ROOT/.github/workflows/prepare-release.yml"
RELEASE="$ROOT/.github/workflows/release.yml"
README_EN="$ROOT/README.md"
README_JA="$ROOT/README.ja.md"

fail=0

if ! rg -q "Sync main into develop" "$PREPARE"; then
  echo "[FAIL] prepare-release.yml missing 'Sync main into develop' step"
  fail=1
fi

if rg -q "sync-develop" "$RELEASE"; then
  echo "[FAIL] release.yml still contains sync-develop job"
  fail=1
fi

if [ -f "$ROOT/docs/release-guide.md" ] || [ -f "$ROOT/docs/release-guide.ja.md" ]; then
  echo "[FAIL] release-guide docs still exist"
  fail=1
fi

if rg -q "release-guide" "$README_EN" "$README_JA"; then
  echo "[FAIL] README still references release-guide"
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "[OK] release flow checks passed"
