#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "CI node toolchain verification failed: $1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "Missing file: $1"
  fi
}

require_not_file() {
  if [ -f "$1" ]; then
    fail "Unexpected file exists: $1"
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

require_file "$ROOT_DIR/pnpm-lock.yaml"
require_not_file "$ROOT_DIR/package-lock.json"
require_file "$ROOT_DIR/package.json"
require_file "$ROOT_DIR/.npmrc"

require_contains "$ROOT_DIR/package.json" "\"packageManager\": \"pnpm@10.29.2\""
require_contains "$ROOT_DIR/.npmrc" "package-lock=false"

WORKFLOW="$ROOT_DIR/.github/workflows/lint.yml"
require_file "$WORKFLOW"

require_not_contains "$WORKFLOW" "npm install -g"
require_contains "$WORKFLOW" "corepack prepare pnpm@10.29.2 --activate"
require_contains "$WORKFLOW" "pnpm dlx @commitlint/cli@20.4.1"

echo "CI node toolchain verification passed."

