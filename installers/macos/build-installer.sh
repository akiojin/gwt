#!/usr/bin/env bash
# Unified macOS installer builder — used by both CI and local development.
#
# Supports the full pipeline: build → sign → DMG → notarize.
# Signing and notarization are optional; skipped when credentials are absent.
#
# Usage:
#   ./installers/macos/build-installer.sh                     # build DMG only
#   ./installers/macos/build-installer.sh --sign              # build + sign
#   ./installers/macos/build-installer.sh --sign --notarize   # build + sign + notarize (CI)
#   ./installers/macos/build-installer.sh --skip-build        # validate existing DMG
#
# Signing env vars (required when --sign):
#   APPLE_CERT_APP_BASE64       Base64-encoded .p12 developer certificate
#   APPLE_CERTIFICATE_PASSWORD  Certificate password
#   — OR —
#   KEYCHAIN_PATH               Pre-configured keychain (skips keychain setup)
#
# Notarization env vars (required when --notarize):
#   APPLE_ID                    Apple ID email
#   APPLE_ID_PASSWORD           App-specific password
#   APPLE_TEAM_ID               Developer Team ID

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

SKIP_BUILD=""
SIGN=""
NOTARIZE=""
VERSION=""

APP_NAME="gwt"
APP_PATH="${REPO_ROOT}/target/release/bundle/macos/${APP_NAME}.app"
DMG_DIR="${REPO_ROOT}/target/release/bundle/dmg"

# --- helpers ----------------------------------------------------------------

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m[warn]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

cleanup_keychain() {
  if [[ -n "${_TEMP_KEYCHAIN:-}" && -f "$_TEMP_KEYCHAIN" ]]; then
    info "Cleaning up temporary keychain..."
    security delete-keychain "$_TEMP_KEYCHAIN" 2>/dev/null || true
  fi
  if [[ -n "${_TEMP_CERT:-}" && -f "$_TEMP_CERT" ]]; then
    rm -f "$_TEMP_CERT"
  fi
}

# --- argument parsing -------------------------------------------------------

while [[ $# -gt 0 ]]; do
  case "$1" in
    --)          shift; continue ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --sign)       SIGN=1; shift ;;
    --notarize)   NOTARIZE=1; shift ;;
    --version|-v)
      if [[ $# -lt 2 ]]; then err "Missing value for $1"; exit 1; fi
      VERSION="${2#v}"
      shift 2
      ;;
    --help|-h)
      cat <<'EOF'
Usage: build-installer.sh [OPTIONS]

Builds macOS .dmg installer with optional signing and notarization.
The same script runs in CI and local environments.

Options:
  --skip-build      Skip cargo tauri build (use existing artifacts)
  --sign            Code-sign the .app bundle (requires signing env vars)
  --notarize        Notarize and staple the DMG (requires notarization env vars)
  --version, -v     Validate output for a specific version
  --help, -h        Show this help

Signing env vars:
  APPLE_CERT_APP_BASE64       Base64-encoded .p12 certificate
  APPLE_CERTIFICATE_PASSWORD  Certificate password
  KEYCHAIN_PATH               (optional) Pre-configured keychain path

Notarization env vars:
  APPLE_ID                    Apple ID email
  APPLE_ID_PASSWORD           App-specific password
  APPLE_TEAM_ID               Developer Team ID
EOF
      exit 0
      ;;
    *) err "Unknown option: $1"; exit 1 ;;
  esac
done

if [[ -n "$NOTARIZE" && -z "$SIGN" ]]; then
  err "--notarize requires --sign"
  exit 1
fi

# --- 1. Environment setup --------------------------------------------------

export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
export CMAKE_OSX_DEPLOYMENT_TARGET="${CMAKE_OSX_DEPLOYMENT_TARGET:-${MACOSX_DEPLOYMENT_TARGET}}"

TOOLCHAIN_FILE="${REPO_ROOT}/cmake/ci-disable-native.cmake"
if [[ -f "$TOOLCHAIN_FILE" && -z "${CMAKE_TOOLCHAIN_FILE:-}" ]]; then
  export CMAKE_TOOLCHAIN_FILE="$TOOLCHAIN_FILE"
fi

info "MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET}"
info "CMAKE_OSX_DEPLOYMENT_TARGET=${CMAKE_OSX_DEPLOYMENT_TARGET}"
if [[ -n "${CMAKE_TOOLCHAIN_FILE:-}" ]]; then
  info "CMAKE_TOOLCHAIN_FILE=${CMAKE_TOOLCHAIN_FILE}"
fi

# --- 2. Build app bundle ---------------------------------------------------

if [[ -z "$SKIP_BUILD" ]]; then
  info "Building app bundle..."
  (cd "$REPO_ROOT" && cargo tauri build -- --bin gwt-tauri)
else
  info "Skipping build (--skip-build)"
fi

# --- 3. Code signing -------------------------------------------------------

if [[ -n "$SIGN" ]]; then
  info "=== Code Signing ==="

  # 3a. Setup keychain if certificate is provided via env
  if [[ -z "${KEYCHAIN_PATH:-}" ]]; then
    if [[ -z "${APPLE_CERT_APP_BASE64:-}" || -z "${APPLE_CERTIFICATE_PASSWORD:-}" ]]; then
      err "Signing requires APPLE_CERT_APP_BASE64 + APPLE_CERTIFICATE_PASSWORD, or KEYCHAIN_PATH"
      exit 1
    fi

    info "Setting up temporary keychain..."
    _TEMP_KEYCHAIN="$(mktemp -u).keychain-db"
    _TEMP_CERT="$(mktemp).p12"
    _KEYCHAIN_PASSWORD="$(uuidgen)"
    trap cleanup_keychain EXIT

    echo "$APPLE_CERT_APP_BASE64" | base64 --decode > "$_TEMP_CERT"

    security create-keychain -p "$_KEYCHAIN_PASSWORD" "$_TEMP_KEYCHAIN"
    security set-keychain-settings -lut 21600 "$_TEMP_KEYCHAIN"
    security unlock-keychain -p "$_KEYCHAIN_PASSWORD" "$_TEMP_KEYCHAIN"
    security import "$_TEMP_CERT" \
      -k "$_TEMP_KEYCHAIN" \
      -P "$APPLE_CERTIFICATE_PASSWORD" \
      -T /usr/bin/codesign \
      -T /usr/bin/security
    security list-keychains -d user -s "$_TEMP_KEYCHAIN"
    security set-key-partition-list -S apple-tool:,apple: -s \
      -k "$_KEYCHAIN_PASSWORD" "$_TEMP_KEYCHAIN"

    KEYCHAIN_PATH="$_TEMP_KEYCHAIN"
    ok "Temporary keychain configured"
  fi

  # 3b. Sign the .app bundle
  if [[ ! -d "$APP_PATH" ]]; then
    err "${APP_NAME}.app not found at ${APP_PATH}"
    exit 1
  fi

  IDENTITY="$(security find-identity -p codesigning -v "$KEYCHAIN_PATH" \
    | awk 'NR==1 {print $2}')"
  if [[ -z "$IDENTITY" ]]; then
    err "No codesigning identity found in keychain"
    exit 1
  fi

  info "Signing ${APP_NAME}.app with identity ${IDENTITY:0:8}..."
  codesign --force --options runtime --sign "$IDENTITY" --deep "$APP_PATH"
  codesign --verify --deep --strict --verbose=4 "$APP_PATH"
  ok "Code signing verified"

  # 3c. Rebuild DMG with signed app
  #     cargo tauri build --bundles dmg re-creates the .app bundle, which strips
  #     the codesign. Use hdiutil directly to preserve the signed .app in the DMG.
  info "Rebuilding DMG with signed app..."

  # Find the existing DMG to determine the output filename
  EXISTING_DMG="$(find "$DMG_DIR" -maxdepth 1 -type f -name '*.dmg' | head -n 1)"
  if [[ -z "$EXISTING_DMG" ]]; then
    err "No existing DMG found in ${DMG_DIR} to replace"
    exit 1
  fi
  DMG_FILENAME="$(basename "$EXISTING_DMG")"

  # Remove old DMG and create a new one from the signed .app
  rm -f "$EXISTING_DMG"
  STAGING_DIR="$(mktemp -d)"
  cp -R "$APP_PATH" "${STAGING_DIR}/${APP_NAME}.app"
  hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$STAGING_DIR" \
    -ov \
    -format UDZO \
    "${DMG_DIR}/${DMG_FILENAME}"
  rm -rf "$STAGING_DIR"
  ok "DMG rebuilt (${DMG_FILENAME})"
fi

# --- 4. Notarization -------------------------------------------------------

if [[ -n "$NOTARIZE" ]]; then
  info "=== Notarization ==="

  for var in APPLE_ID APPLE_ID_PASSWORD APPLE_TEAM_ID; do
    if [[ -z "${!var:-}" ]]; then
      err "Notarization requires ${var}"
      exit 1
    fi
  done

  DMG_PATH="$(find "$DMG_DIR" -maxdepth 1 -type f -name '*.dmg' | head -n 1)"
  if [[ -z "$DMG_PATH" ]]; then
    err "DMG not found under ${DMG_DIR}"
    exit 1
  fi

  info "Submitting ${DMG_PATH} for notarization..."
  xcrun notarytool submit "$DMG_PATH" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_ID_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait

  info "Stapling notarization ticket..."
  xcrun stapler staple "$DMG_PATH"

  info "Verifying Gatekeeper assessment..."
  spctl --assess --verbose=4 "$DMG_PATH"
  spctl --assess --verbose=4 "$APP_PATH"
  ok "Notarization complete"
fi

# --- 5. Validate output ----------------------------------------------------

if [[ ! -d "$DMG_DIR" ]]; then
  # If signing was skipped, DMG was built in step 2 with all bundles
  # but DMG dir might still exist from the full build
  if [[ -z "$SIGN" && -z "$SKIP_BUILD" ]]; then
    err "DMG directory not found: ${DMG_DIR}"
    exit 1
  fi
fi

dmg_paths=()
if [[ -d "$DMG_DIR" ]]; then
  if [[ -n "$VERSION" ]]; then
    while IFS= read -r path; do
      dmg_paths+=("$path")
    done < <(find "$DMG_DIR" -maxdepth 1 -type f -name "gwt_${VERSION}_*.dmg" | sort)
  else
    while IFS= read -r path; do
      dmg_paths+=("$path")
    done < <(find "$DMG_DIR" -maxdepth 1 -type f -name "gwt_*.dmg" | sort)
  fi
fi

if [[ ${#dmg_paths[@]} -eq 0 ]]; then
  if [[ -n "$VERSION" ]]; then
    err "No DMG found for version ${VERSION} under ${DMG_DIR}"
  else
    err "No DMG found under ${DMG_DIR}"
  fi
  exit 1
fi

ok "macOS installer(s) ready:"
for path in "${dmg_paths[@]}"; do
  printf '  - %s\n' "$path"
done
