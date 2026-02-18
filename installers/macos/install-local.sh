#!/usr/bin/env bash
# Build a local macOS package and install it.
#
# Usage:
#   ./installers/macos/install-local.sh
#   ./installers/macos/install-local.sh --skip-build
#   ./installers/macos/install-local.sh --pkg /path/to/gwt-macos-arch.pkg

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PKG_PATH=""
SKIP_BUILD=""

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

need_cmd() {
  if ! command -v "$1" > /dev/null 2>&1; then
    err "Required command not found: $1"
    exit 1
  fi
}

need_either_cmd() {
  if ! command -v "$1" > /dev/null 2>&1 && ! command -v "$2" > /dev/null 2>&1; then
    err "Required command not found: $1 (or $2)"
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pkg)
      if [[ $# -lt 2 ]]; then
        err "Missing value for $1"
        exit 1
      fi
      PKG_PATH="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --help|-h)
      echo "Usage: install-local.sh [--skip-build] [--pkg PATH]"
      echo ""
      echo "Builds a local .pkg from cargo build artifacts and installs it."
      echo ""
      echo "Options:"
      echo "  --skip-build      Skip cargo tauri build + pkg creation"
      echo "  --pkg PATH        Install specified local .pkg path"
      echo "  --help, -h        Show this help"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

need_cmd uname

if [[ -z "$PKG_PATH" ]]; then
  ARCH="$(uname -m)"
  PKG_PATH="${REPO_ROOT}/target/release/bundle/pkg/gwt-macos-${ARCH}.pkg"

  if [[ -z "$SKIP_BUILD" ]]; then
    need_cmd cargo
    need_either_cmd tauri cargo-tauri

    info "Building app bundle..."
    (cd "$REPO_ROOT" && cargo tauri build)

    info "Building installer package..."
    "${SCRIPT_DIR}/build-pkg.sh"
  fi
fi

if [[ ! -f "$PKG_PATH" ]]; then
  err "Package not found: ${PKG_PATH}"
  exit 1
fi

if [[ ! -s "$PKG_PATH" ]]; then
  err "Package is empty: ${PKG_PATH}"
  exit 1
fi

info "Installing ${PKG_PATH} ..."
"${SCRIPT_DIR}/install.sh" --pkg "$PKG_PATH"
