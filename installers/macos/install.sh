#!/bin/bash
# gwt installer for macOS
# Installs gwt.app to /Applications via:
# - .pkg from GitHub Releases (default)
# - local .pkg file via --pkg
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
#
# Install a specific version:
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
#
# Install from local .pkg:
#   ./installers/macos/install.sh --pkg ./target/release/bundle/pkg/gwt-macos-$(uname -m).pkg

set -euo pipefail

REPO="akiojin/gwt"
APP_NAME="gwt"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# --- helpers ---------------------------------------------------------------

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }
warn()  { printf '\033[1;33m[warn]\033[0m %s\n' "$*"; }

need_cmd() {
  if ! command -v "$1" > /dev/null 2>&1; then
    err "Required command not found: $1"
    exit 1
  fi
}

warn_if_pkg_stale() {
  local pkg_path="$1"
  local pkg_prefix="${REPO_ROOT}/target/release/bundle/pkg/"
  local latest_source_file=""
  local source_paths=(
    "${REPO_ROOT}/gwt-gui/src"
    "${REPO_ROOT}/gwt-gui/package.json"
    "${REPO_ROOT}/gwt-gui/pnpm-lock.yaml"
    "${REPO_ROOT}/gwt-gui/svelte.config.js"
    "${REPO_ROOT}/gwt-gui/vite.config.ts"
    "${REPO_ROOT}/gwt-gui/tsconfig.json"
    "${REPO_ROOT}/gwt-gui/tsconfig.node.json"
    "${REPO_ROOT}/crates/gwt-tauri/src"
    "${REPO_ROOT}/crates/gwt-tauri/Cargo.toml"
    "${REPO_ROOT}/crates/gwt-core/src"
    "${REPO_ROOT}/crates/gwt-core/Cargo.toml"
    "${REPO_ROOT}/crates/gwt-tauri/Cargo.lock"
    "${REPO_ROOT}/crates/gwt-core/Cargo.lock"
    "${REPO_ROOT}/Cargo.toml"
    "${REPO_ROOT}/Cargo.lock"
    "${REPO_ROOT}/installers/macos/install.sh"
    "${REPO_ROOT}/installers/macos/install-local.sh"
    "${REPO_ROOT}/installers/macos/build-pkg.sh"
  )

  pkg_path="$(cd "$(dirname "$pkg_path")" && pwd)/$(basename "$pkg_path")"

  case "$pkg_path" in
    "${pkg_prefix}"*)
      ;;
    *)
      return
      ;;
  esac

  for path in "${source_paths[@]}"; do
    if [[ -f "$path" ]]; then
      if [[ "$path" -nt "$pkg_path" ]]; then
        latest_source_file="$path"
        break
      fi
      continue
    fi

    if [[ -d "$path" ]]; then
      while IFS= read -r -d "" src; do
        if [[ "$src" -nt "$pkg_path" ]]; then
          latest_source_file="$src"
          break 2
        fi
      done < <(find "$path" -type f -print0)
    fi
  done

  if [[ -n "$latest_source_file" ]]; then
    warn "Local package appears older than GUI source change:"
    warn "  source: ${latest_source_file}"
    warn "  package: ${pkg_path}"
    warn "Rebuild it with: ./installers/macos/install-local.sh"
    warn "If you installed this package already, some app changes may not be included."
  fi
}

install_pkg_with_privilege() {
  local pkg_path="$1"

  # Interactive shells can use sudo directly.
  if [[ -t 0 || -t 1 ]]; then
    sudo installer -pkg "$pkg_path" -target /
    return
  fi

  # Non-interactive contexts (e.g. in-app terminals) cannot prompt via sudo.
  # Fall back to macOS admin dialog using osascript.
  if command -v osascript > /dev/null 2>&1; then
    info "No interactive terminal detected. Opening macOS admin authentication dialog..."
    osascript - "$pkg_path" <<'APPLESCRIPT'
on run argv
  set pkgPath to item 1 of argv
  do shell script "/usr/sbin/installer -pkg " & quoted form of pkgPath & " -target /" with administrator privileges
end run
APPLESCRIPT
    return
  fi

  err "This session is non-interactive and osascript is unavailable."
  err "Run from Terminal.app with sudo privileges."
  exit 1
}

# --- argument parsing ------------------------------------------------------

VERSION=""
LOCAL_PKG=""
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
    --pkg|-p)
      if [[ $# -lt 2 ]]; then
        err "Missing value for $1"
        exit 1
      fi
      LOCAL_PKG="$2"
      shift 2
      ;;
    --help|-h)
      echo "Usage: install.sh [--version VERSION] [--pkg PKG_PATH]"
      echo ""
      echo "Installs gwt.app to /Applications via .pkg"
      echo ""
      echo "Options:"
      echo "  --version, -v   Install a specific version (e.g. 6.30.3)"
      echo "  --pkg, -p       Install from local .pkg path"
      echo "  --help, -h      Show this help"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

if [[ -n "$VERSION" && -n "$LOCAL_PKG" ]]; then
  err "--version and --pkg cannot be used together"
  exit 1
fi

# --- prerequisites ---------------------------------------------------------

need_cmd uname
need_cmd installer
if [[ -z "$LOCAL_PKG" ]]; then
  need_cmd curl
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
    PKG_ARCH="arm64"
    ;;
  x86_64)
    PKG_ARCH="x86_64"
    ;;
  *)
    err "Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

# --- resolve installer source ----------------------------------------------

PKG_PATH=""
INSTALL_LABEL=""

if [[ -n "$LOCAL_PKG" ]]; then
  PKG_PATH="$LOCAL_PKG"
  if [[ ! -f "$PKG_PATH" ]]; then
    err "Local package not found: ${PKG_PATH}"
    exit 1
  fi
  if [[ ! -s "$PKG_PATH" ]]; then
    err "Local package is empty: ${PKG_PATH}"
    exit 1
  fi
  INSTALL_LABEL="from local package"
  warn_if_pkg_stale "$PKG_PATH"
  info "Installing gwt from local package: ${PKG_PATH}"
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
  PKG_NAME="gwt-macos-${PKG_ARCH}.pkg"
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${PKG_NAME}"

  # --- download ------------------------------------------------------------

  info "Installing gwt v${VERSION} (${ARCH})..."
  info "Downloading: ${DOWNLOAD_URL}"

  TMPDIR_INSTALL="$(mktemp -d)"
  trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

  PKG_PATH="${TMPDIR_INSTALL}/${PKG_NAME}"
  HTTP_CODE=$(curl -fSL -w '%{http_code}' -o "$PKG_PATH" "$DOWNLOAD_URL" 2>/dev/null) || true

  if [[ "$HTTP_CODE" != "200" ]]; then
    err "Download failed (HTTP ${HTTP_CODE})"
    err "URL: ${DOWNLOAD_URL}"
    echo ""
    echo "Available releases: https://github.com/${REPO}/releases"
    exit 1
  fi

  if [[ ! -s "$PKG_PATH" ]]; then
    err "Downloaded file is empty"
    exit 1
  fi

  INSTALL_LABEL="v${VERSION}"
fi

if [[ -z "$INSTALL_LABEL" ]]; then
  INSTALL_LABEL="from package"
fi

if [[ ! -s "$PKG_PATH" ]]; then
  err "Installer package is empty: ${PKG_PATH}"
  exit 1
fi

# --- install ---------------------------------------------------------------

info "Installing to /Applications (requires sudo)..."
install_pkg_with_privilege "$PKG_PATH"

# --- verify ----------------------------------------------------------------

if [[ -d "/Applications/${APP_NAME}.app" ]]; then
  ok "gwt ${INSTALL_LABEL} installed successfully to /Applications/${APP_NAME}.app"
  echo ""
  echo "Launch gwt from Applications or run:"
  echo "  open /Applications/${APP_NAME}.app"
else
  warn "Installation completed but ${APP_NAME}.app was not found in /Applications"
  warn "Check /Applications for the installed application"
fi
