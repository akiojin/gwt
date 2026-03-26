#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Running commit-time backend tests..."

(
  cd "$ROOT_DIR"
  cargo test -p gwt-tauri commands::branches::tests:: -- --nocapture
  cargo test -p gwt-tauri commands::project::tests:: -- --nocapture
)

echo "Commit-time backend tests passed."
