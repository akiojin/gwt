#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRE_PUSH="$ROOT_DIR/.husky/pre-push"
PRE_COMMIT="$ROOT_DIR/.husky/pre-commit"
PACKAGE_JSON="$ROOT_DIR/package.json"

fail() {
  echo "Husky hook verification failed: $1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "Missing file: $1"
  fi
}

require_contains() {
  local file="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$file"; then
    fail "Expected pattern not found in $file: $pattern"
  fi
}

require_not_contains() {
  local file="$1"
  local pattern="$2"
  if grep -Fq "$pattern" "$file"; then
    fail "Unexpected pattern found in $file: $pattern"
  fi
}

require_file "$PACKAGE_JSON"
require_contains "$PACKAGE_JSON" "\"prepare\": \"bunx husky install\""
require_contains "$PACKAGE_JSON" "\"lint:husky\": \"bash scripts/verify-husky-hooks.sh\""

require_file "$PRE_PUSH"
require_contains "$PRE_PUSH" "cargo clippy --all-targets --all-features -- -D warnings"
require_contains "$PRE_PUSH" "cargo fmt --all -- --check"
require_contains "$PRE_PUSH" "bunx --bun markdownlint-cli . --config .markdownlint.json --ignore target --ignore CHANGELOG.md"

if [ -f "$PRE_COMMIT" ]; then
  require_not_contains "$PRE_COMMIT" "cargo clippy --all-targets --all-features -- -D warnings"
  require_not_contains "$PRE_COMMIT" "cargo fmt --all -- --check"
  require_not_contains "$PRE_COMMIT" "markdownlint-cli"
fi

echo "Husky hook verification passed."
