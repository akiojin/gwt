#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

printf '{"private":true,"type":"module"}\n' > "$TMPDIR/package.json"
if ! bun install --cwd "$TMPDIR" linkedom@0.18.12 >/dev/null 2>"$TMPDIR/bun-install.err"; then
  npm install --prefix "$TMPDIR" linkedom@0.18.12 >/dev/null
fi

ln -s "$ROOT/crates" "$TMPDIR/crates"

cd "$TMPDIR"
node --preserve-symlinks --preserve-symlinks-main --test "$@"
