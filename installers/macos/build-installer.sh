#!/usr/bin/env bash
# Build macOS installer (.dmg) in one command.
#
# Usage:
#   ./installers/macos/build-installer.sh
#   ./installers/macos/build-installer.sh --skip-build
#   ./installers/macos/build-installer.sh --version 7.10.2

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SKIP_BUILD=""
VERSION=""

info() { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
err()  { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --)
      shift
      continue
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --version|-v)
      if [[ $# -lt 2 ]]; then
        err "Missing value for $1"
        exit 1
      fi
      VERSION="${2#v}"
      shift 2
      ;;
    --help|-h)
      cat <<'EOF'
Usage: build-installer.sh [--skip-build] [--version VERSION]

Builds macOS .dmg installer.

Options:
  --skip-build      Skip `cargo tauri build --bundles dmg`
  --version, -v     Validate output for a specific version
  --help, -h        Show this help
EOF
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
done

if [[ -z "$SKIP_BUILD" ]]; then
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.15}"
  export CMAKE_OSX_DEPLOYMENT_TARGET="${CMAKE_OSX_DEPLOYMENT_TARGET:-$MACOSX_DEPLOYMENT_TARGET}"

  info "Building macOS dmg installer..."
  info "MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET}"
  (cd "$REPO_ROOT" && cargo tauri build --bundles dmg)
fi

DMG_DIR="$REPO_ROOT/target/release/bundle/dmg"
if [[ ! -d "$DMG_DIR" ]]; then
  err "DMG directory not found: $DMG_DIR"
  exit 1
fi

dmg_paths=()
if [[ -n "$VERSION" ]]; then
  while IFS= read -r path; do
    dmg_paths+=("$path")
  done < <(find "$DMG_DIR" -maxdepth 1 -type f -name "gwt_${VERSION}_*.dmg" | sort)
else
  while IFS= read -r path; do
    dmg_paths+=("$path")
  done < <(find "$DMG_DIR" -maxdepth 1 -type f -name "gwt_*.dmg" | sort)
fi

if [[ ${#dmg_paths[@]} -eq 0 ]]; then
  if [[ -n "$VERSION" ]]; then
    err "No dmg found for version ${VERSION} under ${DMG_DIR}"
  else
    err "No dmg found under ${DMG_DIR}"
  fi
  exit 1
fi

info "Prepared macOS installer(s):"
for path in "${dmg_paths[@]}"; do
  printf '  - %s\n' "$path"
done
