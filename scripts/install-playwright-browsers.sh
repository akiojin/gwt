#!/usr/bin/env bash
#
# Materializes the browsers scripts/run-visual-tests.sh needs (Issue #3659).
#
# `playwright install --with-deps chromium` bundles two unrelated network
# operations behind one command: an apt transaction run as root, and a browser
# download from the Playwright CDN. Neither is bounded, so a stalled apt mirror
# blocks until the CI step timeout fires, and the resulting
# "The action ... has timed out" says nothing about which half stalled.
#
# This script runs the two as named phases, bounds every attempt, retries a
# failed attempt, and configures apt so a stalled mirror gives up quickly
# instead of waiting forever.
#
# Usage: install-playwright-browsers.sh [system-deps|browsers|all]
#
# Environment:
#   GWT_PLAYWRIGHT_VERSION          pinned version (default: scripts/playwright-version.txt)
#   GWT_PLAYWRIGHT_CLI              Playwright CLI command (default: npx --yes playwright@<version>)
#   GWT_PLAYWRIGHT_SYSTEM_DEPS      auto | always | never (default: auto)
#   GWT_PLAYWRIGHT_INSTALL_TIMEOUT  seconds allowed per attempt (default: 150)
#   GWT_PLAYWRIGHT_INSTALL_RETRIES  attempts per phase (default: 3)
#   GWT_PLAYWRIGHT_RETRY_DELAY      seconds between attempts (default: 10)
#   GWT_PLAYWRIGHT_APT_CONF_DIR     apt drop-in directory (default: /etc/apt/apt.conf.d)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="${ROOT}/scripts/playwright-version.txt"

PHASE="${1:-all}"
case "${PHASE}" in
  all | system-deps | browsers) ;;
  *)
    echo "usage: $(basename "$0") [system-deps|browsers|all]" >&2
    exit 2
    ;;
esac

PLAYWRIGHT_VERSION="${GWT_PLAYWRIGHT_VERSION:-$(tr -d '[:space:]' <"${VERSION_FILE}")}"
SYSTEM_DEPS_MODE="${GWT_PLAYWRIGHT_SYSTEM_DEPS:-auto}"
TIMEOUT_SECONDS="${GWT_PLAYWRIGHT_INSTALL_TIMEOUT:-150}"
RETRIES="${GWT_PLAYWRIGHT_INSTALL_RETRIES:-3}"
RETRY_DELAY="${GWT_PLAYWRIGHT_RETRY_DELAY:-10}"
APT_CONF_DIR="${GWT_PLAYWRIGHT_APT_CONF_DIR:-/etc/apt/apt.conf.d}"

# `playwright install-deps` shells out to apt-get and, on Ubuntu 24.04,
# needrestart. Either one prompting means an attempt that can never finish.
export DEBIAN_FRONTEND=noninteractive
export NEEDRESTART_MODE=a

PLAYWRIGHT_CLI=()
read -r -a PLAYWRIGHT_CLI <<<"${GWT_PLAYWRIGHT_CLI:-npx --yes playwright@${PLAYWRIGHT_VERSION}}" || true

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

log() {
  printf '[playwright-install] %s\n' "$*"
}

run_as_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "error: sudo is required when not running as root" >&2
    return 1
  fi
}

TIMEOUT_BIN=""
TIMEOUT_KILLS_STRAGGLERS=0
for candidate in timeout gtimeout; do
  if command -v "${candidate}" >/dev/null 2>&1; then
    TIMEOUT_BIN="${candidate}"
    if "${candidate}" -k 1 1 true >/dev/null 2>&1; then
      TIMEOUT_KILLS_STRAGGLERS=1
    fi
    break
  fi
done

# Bounds one attempt. GNU `timeout` is used when present; the shell fallback
# keeps the script usable on hosts without coreutils (macOS) so the same
# entrypoint works locally and in CI. Exit code 124 means "timed out", matching
# GNU `timeout`.
run_with_timeout() {
  local seconds="$1"
  shift
  local status=0

  if [[ -n "${TIMEOUT_BIN}" ]]; then
    if [[ "${TIMEOUT_KILLS_STRAGGLERS}" -eq 1 ]]; then
      "${TIMEOUT_BIN}" -k 15 "${seconds}" "$@" || status=$?
    else
      "${TIMEOUT_BIN}" "${seconds}" "$@" || status=$?
    fi
    return "${status}"
  fi

  local marker="${WORK_DIR}/timed-out"
  rm -f "${marker}"
  "$@" &
  local command_pid=$!
  (
    sleep "${seconds}"
    : >"${marker}"
    kill -TERM "${command_pid}" 2>/dev/null || true
  ) >/dev/null 2>&1 &
  local watchdog_pid=$!

  wait "${command_pid}" || status=$?
  kill -TERM "${watchdog_pid}" >/dev/null 2>&1 || true
  # Reap the watchdog here, otherwise the shell reports the kill as a
  # `Terminated` job notice in the middle of the install log.
  wait "${watchdog_pid}" 2>/dev/null || true
  if [[ -e "${marker}" ]]; then
    status=124
  fi
  return "${status}"
}

# Runs one phase until it succeeds or the retry budget is spent. Every attempt
# is logged with its ordinal so a bootstrap that only passes on retry is
# visibly different from a healthy one.
run_phase() {
  local phase="$1"
  shift
  local attempt=1
  local status

  while [[ "${attempt}" -le "${RETRIES}" ]]; do
    log "phase=${phase} attempt=${attempt}/${RETRIES} timeout=${TIMEOUT_SECONDS}s cmd=$*"
    status=0
    run_with_timeout "${TIMEOUT_SECONDS}" "$@" || status=$?

    if [[ "${status}" -eq 0 ]]; then
      log "phase=${phase} attempt=${attempt}/${RETRIES} status=ok"
      return 0
    fi

    if [[ "${status}" -eq 124 || "${status}" -eq 137 ]]; then
      log "phase=${phase} attempt=${attempt}/${RETRIES} status=failed reason=timed out after ${TIMEOUT_SECONDS}s"
    else
      log "phase=${phase} attempt=${attempt}/${RETRIES} status=failed exit=${status}"
    fi

    attempt=$((attempt + 1))
    if [[ "${attempt}" -le "${RETRIES}" && "${RETRY_DELAY}" -gt 0 ]]; then
      log "phase=${phase} status=retrying in ${RETRY_DELAY}s"
      sleep "${RETRY_DELAY}"
    fi
  done

  log "phase=${phase} status=failed after ${RETRIES} attempts"
  return 1
}

# The observed hang was apt waiting on a mirror that had accepted the
# connection and then stopped sending. apt has no acquire timeout by default,
# so the wait is unbounded. Bounding it converts the stall into a fast error
# the retry loop can absorb, and ForceIPv4 avoids the IPv6 path that produced
# the `Ign:` mirror failures ahead of the stall.
harden_apt() {
  if [[ ! -d "${APT_CONF_DIR}" ]]; then
    log "phase=system-deps apt-hardening=skipped reason=${APT_CONF_DIR} does not exist"
    return 0
  fi

  local conf="${APT_CONF_DIR}/99-gwt-playwright-acquire"
  local body
  body="$(
    cat <<'CONF'
Acquire::Retries "3";
Acquire::http::Timeout "30";
Acquire::https::Timeout "30";
Acquire::ftp::Timeout "30";
Acquire::ForceIPv4 "true";
CONF
  )"

  # Non-fatal on purpose: the per-attempt timeout below already keeps a stalled
  # mirror from hanging the job, so failing the whole bootstrap because a
  # config file could not be written would trade a slow path for no path.
  if [[ -w "${APT_CONF_DIR}" ]]; then
    if ! printf '%s\n' "${body}" >"${conf}"; then
      log "phase=system-deps apt-hardening=failed conf=${conf} (continuing; attempts stay bounded)"
      return 0
    fi
  elif ! printf '%s\n' "${body}" | run_as_root tee "${conf}" >/dev/null; then
    log "phase=system-deps apt-hardening=failed conf=${conf} (continuing; attempts stay bounded)"
    return 0
  fi
  log "phase=system-deps apt-hardening=applied conf=${conf}"
}

should_install_system_deps() {
  case "${SYSTEM_DEPS_MODE}" in
    always) return 0 ;;
    never) return 1 ;;
    auto) command -v apt-get >/dev/null 2>&1 ;;
    *)
      echo "error: GWT_PLAYWRIGHT_SYSTEM_DEPS must be auto, always, or never" >&2
      exit 2
      ;;
  esac
}

install_system_deps() {
  if ! should_install_system_deps; then
    log "phase=system-deps status=skipped reason=no apt-get on this host (mode=${SYSTEM_DEPS_MODE})"
    return 0
  fi
  harden_apt
  run_phase system-deps "${PLAYWRIGHT_CLI[@]}" install-deps chromium
}

install_browsers() {
  run_phase browsers "${PLAYWRIGHT_CLI[@]}" install chromium
}

log "version=${PLAYWRIGHT_VERSION} phase=${PHASE} retries=${RETRIES} timeout=${TIMEOUT_SECONDS}s"

if [[ "${PHASE}" == "all" || "${PHASE}" == "system-deps" ]]; then
  install_system_deps
fi

if [[ "${PHASE}" == "all" || "${PHASE}" == "browsers" ]]; then
  install_browsers
fi

log "phase=${PHASE} status=complete"
