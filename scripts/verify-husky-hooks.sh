#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRE_PUSH="$ROOT_DIR/.husky/pre-push"
PRE_COMMIT="$ROOT_DIR/.husky/pre-commit"
COMMIT_MSG="$ROOT_DIR/.husky/commit-msg"
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

require_order() {
  local file="$1"
  local first="$2"
  local second="$3"
  local first_line
  local second_line
  first_line=$(grep -Fn "$first" "$file" | head -n 1 | cut -d: -f1 || true)
  second_line=$(grep -Fn "$second" "$file" | head -n 1 | cut -d: -f1 || true)
  if [ -z "$first_line" ] || [ -z "$second_line" ] || [ "$first_line" -ge "$second_line" ]; then
    fail "Expected '$first' to appear before '$second' in $file"
  fi
}

require_file "$PACKAGE_JSON"
require_contains "$PACKAGE_JSON" '"prepare": "test -n \"$CI\" || bunx husky install"'
require_contains "$PACKAGE_JSON" "\"lint:skills\": \"bash scripts/validate-skill-frontmatter.sh\""
require_contains "$PACKAGE_JSON" "\"lint:husky\": \"bash scripts/verify-husky-hooks.sh\""

require_file "$PRE_PUSH"
require_contains "$PRE_PUSH" "cargo clippy --all-targets --all-features -- -D warnings"
require_contains "$PRE_PUSH" "cargo fmt --all -- --check"
require_contains "$PRE_PUSH" "ensure_coverage_tooling"
require_contains "$PRE_PUSH" "rustup component add llvm-tools-preview"
require_contains "$PRE_PUSH" "cargo install cargo-llvm-cov --locked"
require_contains "$PRE_PUSH" "cargo llvm-cov --version"
require_order "$PRE_PUSH" "if cargo llvm-cov --version >/dev/null 2>&1; then" "if command -v rustup >/dev/null 2>&1; then"
require_contains "$PRE_PUSH" "cargo llvm-cov -p gwt-core -p gwt --all-features --json --summary-only --output-path target/coverage-summary.json"
require_contains "$PRE_PUSH" "node scripts/check-coverage-threshold.mjs target/coverage-summary.json 90"
require_contains "$PRE_PUSH" "bunx --bun markdownlint-cli . --config .markdownlint.json --ignore target --ignore CHANGELOG.md --ignore tasks/todo.md"
require_contains "$PRE_PUSH" "pnpm lint:skills"

require_file "$COMMIT_MSG"
require_contains "$COMMIT_MSG" 'bunx --package @commitlint/cli commitlint --edit "$1"'

if [ -f "$PRE_COMMIT" ]; then
  require_contains "$PRE_COMMIT" "pnpm lint:skills"
  require_contains "$PRE_COMMIT" "bash scripts/run-local-backend-tests-on-commit.sh"
  require_not_contains "$PRE_COMMIT" "cargo clippy --all-targets --all-features -- -D warnings"
  require_not_contains "$PRE_COMMIT" "cargo fmt --all -- --check"
  require_not_contains "$PRE_COMMIT" "markdownlint-cli"
fi

echo "Husky hook verification passed."
