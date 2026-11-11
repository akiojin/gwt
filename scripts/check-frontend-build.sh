#!/usr/bin/env bash
# SPEC #6943 AC-2: the committed React bundle must be reproducible from the
# lockfile-pinned sources. Builds twice into a temp dir, requires byte
# equality between the two builds and with the committed bundle.
# The working tree is never modified.
#
# Env overrides (tests only):
#   GWT_FRONTEND_BUILD_ROOT  repository root to check (default: this checkout)
#   GWT_FRONTEND_SKIP_NPM_CI=1  reuse an existing node_modules
set -euo pipefail

ROOT="${GWT_FRONTEND_BUILD_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
PKG="$ROOT/crates/gwt/web-react"
COMMITTED="$ROOT/crates/gwt/web/react/islands.js"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

recovery() {
  echo "Recovery: run 'npm ci --prefix crates/gwt/web-react --ignore-scripts && node crates/gwt/web-react/build.mjs'" >&2
  echo "          and commit crates/gwt/web/react/islands.js together with the source change." >&2
}

if [[ "${GWT_FRONTEND_SKIP_NPM_CI:-}" != "1" ]]; then
  npm ci --prefix "$PKG" --ignore-scripts --no-audit --no-fund >/dev/null
fi

node "$PKG/build.mjs" "$TMP/first.js"
node "$PKG/build.mjs" "$TMP/second.js"

if ! cmp -s "$TMP/first.js" "$TMP/second.js"; then
  echo "error: two builds of crates/gwt/web-react produced different bytes (non-deterministic build)" >&2
  exit 1
fi
if [[ ! -f "$COMMITTED" ]]; then
  echo "error: crates/gwt/web/react/islands.js is missing" >&2
  recovery
  exit 1
fi
if ! cmp -s "$TMP/first.js" "$COMMITTED"; then
  echo "error: crates/gwt/web/react/islands.js is stale: it does not match a fresh build of crates/gwt/web-react" >&2
  recovery
  exit 1
fi
echo "frontend build: reproducible and committed bundle is fresh"
