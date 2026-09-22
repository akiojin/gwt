#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRE_PUSH="$ROOT_DIR/.husky/pre-push"
PRE_COMMIT="$ROOT_DIR/.husky/pre-commit"
COMMIT_MSG="$ROOT_DIR/.husky/commit-msg"

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
  if grep -Fq -- "$pattern" "$file"; then
    fail "Unexpected pattern found in $file: $pattern"
  fi
}

# Same as require_not_contains, but ignores comment lines so a hook may still
# explain in prose which command it deliberately does not run.
require_code_not_contains() {
  local file="$1"
  local pattern="$2"
  if sed 's/[[:space:]]*#.*$//' "$file" | grep -Fq -- "$pattern"; then
    fail "Unexpected command found in $file: $pattern"
  fi
}

require_file "$PRE_PUSH"
require_contains "$PRE_PUSH" "cargo fmt --all -- --check"
require_contains "$PRE_PUSH" "bunx --bun markdownlint-cli . --config .markdownlint.json --ignore target --ignore CHANGELOG.md --ignore tasks"
require_contains "$PRE_PUSH" "bash scripts/validate-skill-frontmatter.sh"

# SPEC #3576: `git push` must never start a heavy Cargo job. Those jobs compile
# the workspace and saturate the host's CPU, and they run outside the
# host-wide verification lease because the hook hangs off `git push`, not off
# `gwtd`. Once #4339 made every worktree materialize its hooks, a single push
# per worktree was enough to run several instrumented suites at once, which is
# what produced the wall-clock fixture failures and the misread lease holders.
# Every check removed here is already enforced per pull request by the Lint and
# Test workflows, so this keeps the gate and drops only the duplicate.
require_code_not_contains "$PRE_PUSH" "cargo clippy"
require_code_not_contains "$PRE_PUSH" "cargo llvm-cov"
require_code_not_contains "$PRE_PUSH" "cargo install cargo-llvm-cov --locked"
require_code_not_contains "$PRE_PUSH" "rustup component add llvm-tools-preview"
require_code_not_contains "$PRE_PUSH" "ensure_coverage_tooling"
require_code_not_contains "$PRE_PUSH" "check-coverage-threshold.mjs"
require_code_not_contains "$PRE_PUSH" "cargo test"
require_code_not_contains "$PRE_PUSH" "cargo build"

require_file "$COMMIT_MSG"
require_contains "$COMMIT_MSG" 'bunx --package @commitlint/cli commitlint --edit "$1"'

if [ -f "$PRE_COMMIT" ]; then
  require_contains "$PRE_COMMIT" "bash scripts/validate-skill-frontmatter.sh"
  require_contains "$PRE_COMMIT" "bash scripts/run-local-backend-tests-on-commit.sh"
  require_not_contains "$PRE_COMMIT" "cargo clippy --all-targets --all-features -- -D warnings"
  require_not_contains "$PRE_COMMIT" "cargo fmt --all -- --check"
  require_not_contains "$PRE_COMMIT" "markdownlint-cli"
fi

echo "Husky hook verification passed."
