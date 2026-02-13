#!/bin/bash
# gwt installer for macOS
# Downloads and installs gwt.app from GitHub Releases (.dmg)
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
#
# Install a specific version:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 7.1.0

set -euo pipefail

REPO="akiojin/gwt"
APP_NAME="gwt"

# --- helpers ---------------------------------------------------------------

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

need_cmd() {
  if ! command -v "$1" > /dev/null 2>&1; then
    err "Required command not found: $1"
    exit 1
  fi
}

# --- argument parsing ------------------------------------------------------

VERSION=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v)
      if [[ $# -lt 2 ]]; then
        err "Missing value for $1"
        exit 1
      fi
      VERSION="$2"
      shift 2
      ;;
    --help|-h)
      echo "Usage: install.sh [--version VERSION]"
      echo ""
      echo "Downloads and installs gwt.app from GitHub Releases (.dmg)"
      echo ""
      echo "Options:"
      echo "  --version, -v   Install a specific version (e.g. 7.1.0)"
      echo "  --help, -h      Show this help"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

# --- prerequisites ---------------------------------------------------------

need_cmd uname
need_cmd curl
need_cmd hdiutil

# --- detect platform -------------------------------------------------------

OS="$(uname -s)"
ARCH="$(uname -m)"

if [[ "$OS" != "Darwin" ]]; then
  err "This installer is for macOS only. Detected: $OS"
  exit 1
fi

case "$ARCH" in
  arm64|aarch64)
    DMG_ARCH="aarch64"
    ;;
  x86_64)
    DMG_ARCH="x86_64"
    ;;
  *)
    err "Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

# --- resolve version -------------------------------------------------------

if [[ -z "$VERSION" ]]; then
  info "Fetching latest release version..."
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | head -1 | sed 's/.*"v\([^"]*\)".*/\1/')
  if [[ -z "$VERSION" ]]; then
    err "Failed to determine latest version"
    exit 1
  fi
fi

# Strip leading 'v' if present
VERSION="${VERSION#v}"
DMG_NAME="gwt_${VERSION}_${DMG_ARCH}.dmg"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${DMG_NAME}"

# --- download --------------------------------------------------------------

info "Installing gwt v${VERSION} (${ARCH})..."
info "Downloading: ${DOWNLOAD_URL}"

TMPDIR_INSTALL="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

DMG_PATH="${TMPDIR_INSTALL}/${DMG_NAME}"
HTTP_CODE=$(curl -fSL -w '%{http_code}' -o "$DMG_PATH" "$DOWNLOAD_URL" 2>/dev/null) || true

if [[ "$HTTP_CODE" != "200" ]]; then
  err "Download failed (HTTP ${HTTP_CODE})"
  err "URL: ${DOWNLOAD_URL}"
  echo ""
  echo "Available releases: https://github.com/${REPO}/releases"
  exit 1
fi

if [[ ! -s "$DMG_PATH" ]]; then
  err "Downloaded file is empty"
  exit 1
fi

# --- install ---------------------------------------------------------------

info "Mounting disk image..."
MOUNT_POINT="${TMPDIR_INSTALL}/gwt-mount"
mkdir -p "$MOUNT_POINT"
hdiutil attach "$DMG_PATH" -mountpoint "$MOUNT_POINT" -nobrowse -quiet

APP_SRC="${MOUNT_POINT}/${APP_NAME}.app"
if [[ ! -d "$APP_SRC" ]]; then
  hdiutil detach "$MOUNT_POINT" -quiet 2>/dev/null || true
  err "${APP_NAME}.app not found in disk image"
  exit 1
fi

info "Installing to /Applications..."
if [[ -d "/Applications/${APP_NAME}.app" ]]; then
  info "Removing existing installation..."
  rm -rf "/Applications/${APP_NAME}.app" 2>/dev/null || sudo rm -rf "/Applications/${APP_NAME}.app"
fi
cp -a "$APP_SRC" "/Applications/${APP_NAME}.app" 2>/dev/null || sudo cp -a "$APP_SRC" "/Applications/${APP_NAME}.app"

hdiutil detach "$MOUNT_POINT" -quiet 2>/dev/null || true

# --- verify ----------------------------------------------------------------

if [[ -d "/Applications/${APP_NAME}.app" ]]; then
  ok "gwt v${VERSION} installed successfully to /Applications/${APP_NAME}.app"
  echo ""
  echo "Launch gwt from Applications or run:"
  echo "  open /Applications/${APP_NAME}.app"
else
  err "Installation failed: ${APP_NAME}.app was not found in /Applications"
  exit 1
fi
