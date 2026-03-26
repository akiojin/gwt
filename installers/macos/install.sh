#!/bin/bash
# gwt installer for macOS
# Installs gwt.app to /Applications via:
# - .dmg from GitHub Releases (default)
# - local .app bundle via --app
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
#
# Install a specific version:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 7.10.2
#
# Install from local .app bundle:
#   ./installers/macos/install.sh --app ./target/release/bundle/macos/gwt.app

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

copy_app_with_privilege() {
  local app_path="$1"
  local dest="/Applications/${APP_NAME}.app"
  local dest_parent="/Applications"

  if [[ ! -e "$dest" && -w "$dest_parent" ]]; then
    info "Copying ${APP_NAME}.app to /Applications..."
    /usr/bin/ditto "$app_path" "$dest"
    return
  fi

  if [[ -d "$dest" && -w "$dest" ]]; then
    info "Copying ${APP_NAME}.app to /Applications..."
    /usr/bin/ditto "$app_path" "$dest"
    return
  fi

  # Remove existing installation
  if [[ -d "$dest" ]]; then
    info "Removing existing installation..."
    if [[ -t 0 || -t 1 ]]; then
      sudo rm -rf "$dest"
    elif command -v osascript > /dev/null 2>&1; then
      osascript -e "do shell script \"rm -rf '${dest}'\" with administrator privileges"
    else
      err "Non-interactive session and osascript unavailable."
      exit 1
    fi
  fi

  info "Copying ${APP_NAME}.app to /Applications..."
  if [[ -t 0 || -t 1 ]]; then
    sudo cp -R "$app_path" "$dest"
  elif command -v osascript > /dev/null 2>&1; then
    osascript -e "do shell script \"cp -R '${app_path}' '${dest}'\" with administrator privileges"
  else
    err "Non-interactive session and osascript unavailable."
    exit 1
  fi
}

# --- argument parsing ------------------------------------------------------

VERSION=""
LOCAL_APP=""
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
    --app|-a)
      if [[ $# -lt 2 ]]; then
        err "Missing value for $1"
        exit 1
      fi
      LOCAL_APP="$2"
      shift 2
      ;;
    --help|-h)
      echo "Usage: install.sh [--version VERSION] [--app APP_PATH]"
      echo ""
      echo "Installs gwt.app to /Applications via .dmg"
      echo ""
      echo "Options:"
      echo "  --version, -v   Install a specific version (e.g. 7.10.2)"
      echo "  --app, -a       Install from local .app bundle path"
      echo "  --help, -h      Show this help"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

if [[ -n "$VERSION" && -n "$LOCAL_APP" ]]; then
  err "--version and --app cannot be used together"
  exit 1
fi

# --- prerequisites ---------------------------------------------------------

need_cmd uname
if [[ -z "$LOCAL_APP" ]]; then
  need_cmd curl
  need_cmd hdiutil
fi

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

# --- resolve installer source ----------------------------------------------

APP_PATH=""
INSTALL_LABEL=""

if [[ -n "$LOCAL_APP" ]]; then
  APP_PATH="$LOCAL_APP"
  if [[ ! -d "$APP_PATH" ]]; then
    err "App bundle not found: ${APP_PATH}"
    exit 1
  fi
  INSTALL_LABEL="from local bundle"
  info "Installing gwt from local app bundle: ${APP_PATH}"
else
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
  DMG_NAME="${APP_NAME}_${VERSION}_${DMG_ARCH}.dmg"
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${DMG_NAME}"

  # --- download ------------------------------------------------------------

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

  # --- mount and extract ---------------------------------------------------

  info "Mounting disk image..."
  MOUNT_DIR="${TMPDIR_INSTALL}/mnt"
  mkdir -p "$MOUNT_DIR"
  hdiutil attach "$DMG_PATH" -mountpoint "$MOUNT_DIR" -nobrowse -quiet

  APP_PATH="${MOUNT_DIR}/${APP_NAME}.app"
  if [[ ! -d "$APP_PATH" ]]; then
    hdiutil detach "$MOUNT_DIR" -quiet 2>/dev/null || true
    err "${APP_NAME}.app not found in disk image"
    exit 1
  fi

  INSTALL_LABEL="v${VERSION}"
fi

# --- install ---------------------------------------------------------------

info "Installing to /Applications (may require password)..."
copy_app_with_privilege "$APP_PATH"

# Unmount DMG if we mounted one
if [[ -n "${MOUNT_DIR:-}" ]]; then
  hdiutil detach "$MOUNT_DIR" -quiet 2>/dev/null || true
fi

# --- verify ----------------------------------------------------------------

if [[ -d "/Applications/${APP_NAME}.app" ]]; then
  ok "gwt ${INSTALL_LABEL} installed successfully to /Applications/${APP_NAME}.app"
  echo ""
  echo "Launch gwt from Applications or run:"
  echo "  open /Applications/${APP_NAME}.app"
else
  err "Installation completed but ${APP_NAME}.app was not found in /Applications"
fi
