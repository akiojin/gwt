#!/bin/bash
# Build macOS installers (.dmg from Tauri + .pkg wrapper) in one command.
#
# Usage:
#   ./installers/macos/build-installer.sh
#   ./installers/macos/build-installer.sh --version 7.7.0
#   ./installers/macos/build-installer.sh --skip-build

set -euo pipefail

VERSION=""
SKIP_BUILD=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v)
      VERSION="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --help|-h)
      echo "Usage: build-installer.sh [--version VERSION] [--skip-build]"
      exit 0
      ;;
    *)
      echo "[error] Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

if [[ "$SKIP_BUILD" == "false" ]]; then
  echo "[info] Building app with Tauri..."
  (cd "$REPO_ROOT" && cargo tauri build)
fi

echo "[info] Building .pkg installer..."
if [[ -n "$VERSION" ]]; then
  "$REPO_ROOT/installers/macos/build-pkg.sh" --version "$VERSION"
else
  "$REPO_ROOT/installers/macos/build-pkg.sh"
fi
