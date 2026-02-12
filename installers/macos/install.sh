#!/bin/bash
# gwt installer for macOS
# Usage: curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
# Or with a specific version:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3

set -euo pipefail

REPO="akiojin/gwt"
INSTALL_DIR="${GWT_INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME="gwt"

# --- helpers ---------------------------------------------------------------

info()  { printf '\033[1;34m%s\033[0m\n' "$*"; }
ok()    { printf '\033[1;32m%s\033[0m\n' "$*"; }
err()   { printf '\033[1;31mError: %s\033[0m\n' "$*" >&2; }
warn()  { printf '\033[1;33m%s\033[0m\n' "$*"; }

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
      VERSION="$2"
      shift 2
      ;;
    --help|-h)
      echo "Usage: install.sh [--version VERSION]"
      echo ""
      echo "Options:"
      echo "  --version, -v   Install a specific version (e.g. 6.30.3)"
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

need_cmd curl
need_cmd uname

# --- detect platform -------------------------------------------------------

OS="$(uname -s)"
ARCH="$(uname -m)"

if [[ "$OS" != "Darwin" ]]; then
  err "This installer is for macOS only. Detected: $OS"
  exit 1
fi

case "$ARCH" in
  arm64|aarch64)
    ARTIFACT="gwt-macos-aarch64"
    ;;
  x86_64)
    ARTIFACT="gwt-macos-x86_64"
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
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARTIFACT}"

# --- download & install ----------------------------------------------------

info "Installing gwt v${VERSION} (${ARCH})..."
info "Downloading from: ${DOWNLOAD_URL}"

TMPDIR_INSTALL="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

HTTP_CODE=$(curl -fSL -w '%{http_code}' -o "${TMPDIR_INSTALL}/${BINARY_NAME}" "$DOWNLOAD_URL" 2>/dev/null) || true

if [[ "$HTTP_CODE" != "200" ]]; then
  err "Download failed (HTTP ${HTTP_CODE})"
  err "URL: ${DOWNLOAD_URL}"
  echo ""
  echo "Available releases: https://github.com/${REPO}/releases"
  exit 1
fi

chmod +x "${TMPDIR_INSTALL}/${BINARY_NAME}"

# Verify the binary runs
if ! "${TMPDIR_INSTALL}/${BINARY_NAME}" --version > /dev/null 2>&1; then
  warn "Binary downloaded but --version check failed (this may be expected for GUI builds)"
fi

# Install
if [[ ! -d "$INSTALL_DIR" ]]; then
  mkdir -p "$INSTALL_DIR" 2>/dev/null || sudo mkdir -p "$INSTALL_DIR"
fi

if [[ -w "$INSTALL_DIR" ]]; then
  mv "${TMPDIR_INSTALL}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
else
  info "Installing to ${INSTALL_DIR} (requires sudo)..."
  sudo mv "${TMPDIR_INSTALL}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
fi

# --- verify ----------------------------------------------------------------

if command -v gwt > /dev/null 2>&1; then
  ok "gwt v${VERSION} installed successfully to ${INSTALL_DIR}/${BINARY_NAME}"
else
  warn "gwt installed to ${INSTALL_DIR}/${BINARY_NAME}"
  warn "${INSTALL_DIR} may not be in your PATH."
  echo ""
  echo "Add it to your PATH:"
  echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
