#!/bin/bash
# Build macOS .pkg installer from Tauri build output
#
# Prerequisites:
#   cargo tauri build
#
# Usage:
#   ./installers/macos/build-pkg.sh
#   ./installers/macos/build-pkg.sh --version 6.30.3
#   ./installers/macos/build-pkg.sh --sign
#   ./installers/macos/build-pkg.sh --sign --notarize
#
# Environment variables for notarization:
#   APPLE_ID              Apple ID email
#   APPLE_ID_PASSWORD     App-specific password
#   APPLE_TEAM_ID         Team ID

set -euo pipefail

IDENTIFIER="com.akiojin.gwt"
APP_NAME="gwt"

# --- helpers ---------------------------------------------------------------

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

# --- argument parsing ------------------------------------------------------

VERSION=""
SIGN=""
NOTARIZE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v)
      VERSION="$2"
      shift 2
      ;;
    --sign|-s)
      SIGN=1
      shift
      ;;
    --notarize|-n)
      NOTARIZE=1
      SIGN=1
      shift
      ;;
    --help|-h)
      echo "Usage: build-pkg.sh [OPTIONS]"
      echo ""
      echo "Build macOS .pkg installer from Tauri build output."
      echo ""
      echo "Options:"
      echo "  --version, -v    Override version (default: read from Cargo.toml)"
      echo "  --sign, -s       Code-sign the .app and .pkg"
      echo "  --notarize, -n   Notarize with Apple (implies --sign)"
      echo "  --help, -h       Show this help"
      echo ""
      echo "Environment variables (for --notarize):"
      echo "  APPLE_ID              Apple ID email"
      echo "  APPLE_ID_PASSWORD     App-specific password"
      echo "  APPLE_TEAM_ID         Team ID"
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

# --- code sign .app --------------------------------------------------------

if [[ -n "$SIGN" ]]; then
  APP_SIGN_ID="Developer ID Application: Akio Jinsenji (T27VLF88ZK)"
  info "Signing .app with: ${APP_SIGN_ID}"
  codesign --deep --force --options runtime \
    --sign "$APP_SIGN_ID" \
    "$APP_PATH"
  codesign --verify --verbose "$APP_PATH"
  ok "App signed"
fi

# --- build .pkg ------------------------------------------------------------

ARCH="$(uname -m)"
PKG_DIR="$REPO_ROOT/target/release/bundle/pkg"
mkdir -p "$PKG_DIR"
PKG_UNSIGNED="${PKG_DIR}/${APP_NAME}-macos-${ARCH}-unsigned.pkg"
PKG_PATH="${PKG_DIR}/${APP_NAME}-macos-${ARCH}.pkg"

info "Building .pkg installer..."

pkgbuild \
  --component "$APP_PATH" \
  --install-location "/Applications" \
  --identifier "$IDENTIFIER" \
  --version "$VERSION" \
  "$PKG_UNSIGNED"

# --- sign .pkg -------------------------------------------------------------

if [[ -n "$SIGN" ]]; then
  PKG_SIGN_ID="Developer ID Installer: Akio Jinsenji (T27VLF88ZK)"
  info "Signing .pkg with: ${PKG_SIGN_ID}"
  productsign --sign "$PKG_SIGN_ID" "$PKG_UNSIGNED" "$PKG_PATH"
  rm -f "$PKG_UNSIGNED"
  pkgutil --check-signature "$PKG_PATH"
  ok "Pkg signed"
else
  mv "$PKG_UNSIGNED" "$PKG_PATH"
fi

# --- notarize --------------------------------------------------------------

if [[ -n "$NOTARIZE" ]]; then
  : "${APPLE_ID:?Set APPLE_ID for notarization}"
  : "${APPLE_ID_PASSWORD:?Set APPLE_ID_PASSWORD for notarization}"
  : "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID for notarization}"

  info "Submitting for notarization..."
  xcrun notarytool submit "$PKG_PATH" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_ID_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait

  info "Stapling notarization ticket..."
  xcrun stapler staple "$PKG_PATH"
  ok "Notarization complete"
fi

# --- validate --------------------------------------------------------------

if [[ ! -s "$PKG_PATH" ]]; then
  err "Built .pkg is empty"
  exit 1
fi

SIZE=$(du -h "$PKG_PATH" | cut -f1)
ok "Built: ${PKG_PATH} (${SIZE})"
