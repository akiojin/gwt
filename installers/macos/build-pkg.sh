#!/bin/bash
# Build macOS .pkg installer from Tauri build output
#
# Prerequisites:
#   cargo tauri build
#
# Usage:
#   ./installers/macos/build-pkg.sh
#   ./installers/macos/build-pkg.sh --version 6.30.3

set -euo pipefail

IDENTIFIER="com.akiojin.gwt"
APP_NAME="gwt"

# --- helpers ---------------------------------------------------------------

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

# --- argument parsing ------------------------------------------------------

VERSION=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v)
      VERSION="$2"
      shift 2
      ;;
    --help|-h)
      echo "Usage: build-pkg.sh [--version VERSION]"
      echo ""
      echo "Build macOS .pkg installer from Tauri build output."
      echo ""
      echo "Options:"
      echo "  --version, -v   Override version (default: read from Cargo.toml)"
      echo "  --help, -h      Show this help"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

# --- resolve version -------------------------------------------------------

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

if [[ -z "$VERSION" ]]; then
  VERSION=$(grep -m1 '^version = ' "$REPO_ROOT/Cargo.toml" | sed 's/version = "\(.*\)"/\1/')
  if [[ -z "$VERSION" ]]; then
    err "Failed to read version from Cargo.toml"
    exit 1
  fi
fi

info "Version: ${VERSION}"

# --- locate .app bundle ----------------------------------------------------

APP_PATH=$(find "$REPO_ROOT/target/release/bundle/macos" -maxdepth 1 -name "*.app" -print -quit 2>/dev/null || true)

if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  err "No .app bundle found in target/release/bundle/macos/"
  err "Run 'cargo tauri build' first."
  exit 1
fi

info "Found app: ${APP_PATH}"

# --- build .pkg ------------------------------------------------------------

ARCH="$(uname -m)"
PKG_DIR="$REPO_ROOT/target/release/bundle/pkg"
mkdir -p "$PKG_DIR"
PKG_PATH="${PKG_DIR}/${APP_NAME}-macos-${ARCH}.pkg"

info "Building .pkg installer..."

pkgbuild \
  --component "$APP_PATH" \
  --install-location "/Applications" \
  --identifier "$IDENTIFIER" \
  --version "$VERSION" \
  "$PKG_PATH"

# --- validate --------------------------------------------------------------

if [[ ! -s "$PKG_PATH" ]]; then
  err "Built .pkg is empty"
  exit 1
fi

SIZE=$(du -h "$PKG_PATH" | cut -f1)
ok "Built: ${PKG_PATH} (${SIZE})"
