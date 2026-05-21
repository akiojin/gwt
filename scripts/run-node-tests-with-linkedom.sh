#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

bun install --cwd "$TMPDIR" linkedom@0.18.12 >/dev/null

cd "$ROOT"
NODE_PATH="$TMPDIR/node_modules" node --test "$@"
