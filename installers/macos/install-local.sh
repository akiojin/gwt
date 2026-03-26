#!/usr/bin/env bash
# Build and install gwt locally from source.
#
# Usage:
#   ./installers/macos/install-local.sh
#   ./installers/macos/install-local.sh --skip-build
#   ./installers/macos/install-local.sh --app /path/to/gwt.app

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
APP_PATH=""
SKIP_BUILD=""

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

need_cmd() {
  if ! command -v "$1" > /dev/null 2>&1; then
    err "Required command not found: $1"
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      if [[ $# -lt 2 ]]; then
        err "Missing value for $1"
        exit 1
      fi
      APP_PATH="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --help|-h)
      echo "Usage: install-local.sh [--skip-build] [--app PATH]"
      echo ""
      echo "Builds gwt and installs gwt.app to /Applications."
      echo ""
      echo "Options:"
      echo "  --skip-build      Skip cargo tauri build"
      echo "  --app PATH        Install specified .app bundle"
      echo "  --help, -h        Show this help"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

if [[ -z "$APP_PATH" ]]; then
  APP_PATH="${REPO_ROOT}/target/release/bundle/macos/gwt.app"

  if [[ -z "$SKIP_BUILD" ]]; then
    need_cmd cargo

    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
    export CMAKE_OSX_DEPLOYMENT_TARGET="${CMAKE_OSX_DEPLOYMENT_TARGET:-${MACOSX_DEPLOYMENT_TARGET}}"

    info "Building app bundle only (skip dmg for local install)..."
    info "MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET}"
    info "CMAKE_OSX_DEPLOYMENT_TARGET=${CMAKE_OSX_DEPLOYMENT_TARGET}"
    (cd "$REPO_ROOT" && cargo tauri build --bundles app -- --bin gwt-tauri)
  fi
fi

if [[ ! -d "$APP_PATH" ]]; then
  err "App bundle not found: ${APP_PATH}"
  err "Run 'cargo tauri build' first."
  exit 1
fi

info "Installing ${APP_PATH} ..."
"${SCRIPT_DIR}/install.sh" --app "$APP_PATH"
