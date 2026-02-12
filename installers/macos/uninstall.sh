#!/bin/bash
# Uninstall gwt from macOS
#
# Usage:
#   ./installers/macos/uninstall.sh
#   curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash

set -euo pipefail

APP_NAME="gwt"
APP_PATH="/Applications/${APP_NAME}.app"
PKG_ID="com.akiojin.gwt"

# --- helpers ---------------------------------------------------------------

info()  { printf '\033[1;34m[info]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }
warn()  { printf '\033[1;33m[warn]\033[0m %s\n' "$*"; }

# --- confirmation ----------------------------------------------------------

if [[ -t 0 ]]; then
  printf "Uninstall %s from %s? [y/N] " "$APP_NAME" "$APP_PATH"
  read -r answer
  if [[ "$answer" != [yY] ]]; then
    info "Cancelled."
    exit 0
  fi
fi

# --- remove app ------------------------------------------------------------

if [[ -d "$APP_PATH" ]]; then
  info "Removing ${APP_PATH} (requires sudo)..."
  sudo rm -rf "$APP_PATH"
  ok "Removed ${APP_PATH}"
else
  warn "${APP_PATH} not found"
fi

# --- forget package receipt ------------------------------------------------

if pkgutil --pkgs 2>/dev/null | grep -q "^${PKG_ID}$"; then
  info "Removing package receipt..."
  sudo pkgutil --forget "$PKG_ID"
  ok "Removed package receipt: ${PKG_ID}"
fi

# --- done ------------------------------------------------------------------

ok "gwt has been uninstalled."
