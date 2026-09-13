#!/usr/bin/env bash
#
# Bounded apt-get for CI and local Linux installs (Issue #3701, Issue #4191).
#
# GitHub-hosted Ubuntu images often run unattended-upgrades just after boot
# and hold /var/lib/dpkg/lock-frontend. A bare `apt-get` then either fails
# immediately (`Could not get lock`) or waits without a deadline. This
# wrapper waits for the lock with a greppable log line, then runs apt-get
# with DPkg::Lock::Timeout so a lock that reappears mid-transaction still
# cannot hang the job.
#
# Issue #4191 added the rest. `Install tray + GTK dependencies (Linux)` is the
# first step of `Test (Rust)`, the only job that runs the whole workspace
# suite, and it used to run under a 5-minute GitHub step cap. Measured across
# the last 60 green runs the step takes p50 29s, p90 48s, p95 60s, max 106s,
# so that cap was ~3x the worst healthy run: a step that hit it was a stalled
# mirror, not a slow one, and the job then reported FAILURE having executed
# zero tests -- indistinguishable from a code failure.
#
# Raising the cap alone would not fix that, so the deadline now lives here:
#
#   * GWT_APT_TOTAL_DEADLINE bounds the whole invocation,
#   * a failed attempt is retried inside that deadline,
#   * GWT_APT_CACHE_DIR keeps the downloaded .debs so a warm CI cache
#     installs without touching a mirror,
#   * and giving up is reported as a dependency-install failure, in the log
#     and in the GitHub job summary, so "no tests ran" is legible at a glance.
#
# The workflow `timeout-minutes` is only the outer net and must stay above the
# total deadline; if GitHub kills the step first, none of the reporting above
# ever runs. crates/gwt/tests/ci_linux_dependency_install_contract_test.rs
# pins that relationship.
#
# Issue #4268 is what was left after all of that. The retries did not help:
# run 34575831565 burned all three attempts on one stall, and the log read
#
#   Need to get 59.2 MB of archives.
#   Get:1 file:/etc/apt/apt-mirrors.txt Mirrorlist [144 B]
#   <239 seconds of silence>
#
# with the archive cache reported as `Cache not found for input keys`. Not one
# byte of the 59.2 MB arrived. apt has no acquire timeout by default, so a
# mirror that accepts the connection and then stops sending is waited on
# forever: the `timeout` below was the only cut-off, it took the entire attempt
# with it, and the next attempt started over against the same mirror. Adding
# attempts or seconds cannot fix that — three 240s attempts against an
# unbounded wait are three unbounded waits.
#
# So the acquire is bounded instead. `harden_apt` drops
# Acquire::{http,https,ftp}::Timeout, Acquire::Retries and Acquire::ForceIPv4
# into apt.conf.d before the first fetch, which turns a stalled connection into
# a fast error apt itself retries and fails over from, inside one attempt.
# (scripts/install-playwright-browsers.sh carries the same drop-in for the
# apt-get that `playwright install-deps` spawns, where this script is not in
# the call path.)
#
# A failed attempt now also reports where its budget went — step, lock wait,
# bytes asked for, how far the fetch got, and the mirror it was sitting on —
# because "timed out after 240s" alone cannot be told apart from a slow mirror,
# a lock wait, or a fetch that never started, and eight pull requests sat
# parked on exactly that ambiguity.
#
# AC-5 (a path that runs the tests anyway when the fetch fails) is deliberately
# not taken: `Test (Rust)` runs `cargo test --workspace --all-features`, and
# the workspace links wry/tao against GTK and WebKitGTK, so without these
# packages the job cannot compile `gwt` at all. A gtk-free subset would be a
# second job with its own compile and its own cargo cache, paying a permanent
# cost to soften a failure whose cause is now bounded and legible.
#
# Usage:
#   ci-apt.sh wait
#   ci-apt.sh harden
#   ci-apt.sh update
#   ci-apt.sh install -y libgtk-3-dev ...
#   ci-apt.sh gtk-deps [extra packages...]
#
# Environment:
#   GWT_APT_LOCK_TIMEOUT     seconds to wait for a held lock (default: 180)
#   GWT_APT_LOCK_POLL        seconds between probe polls (default: 5)
#   GWT_APT_ATTEMPTS         attempts before giving up (default: 3)
#   GWT_APT_ATTEMPT_TIMEOUT  seconds allowed per apt-get call (default: 240)
#   GWT_APT_RETRY_DELAY      seconds between attempts (default: 15)
#   GWT_APT_TOTAL_DEADLINE   seconds for the whole invocation (default: 900)
#   GWT_APT_ACQUIRE_TIMEOUT  seconds apt may wait on one connection (default: 30)
#   GWT_APT_ACQUIRE_RETRIES  apt's own per-item retries (default: 3)
#   GWT_APT_CONF_DIR         apt drop-in directory (default: /etc/apt/apt.conf.d)
#   GWT_APT_CACHE_DIR        .deb archive dir reused across CI runs
#   GWT_APT_GET              apt-get binary (default: apt-get)
#   GWT_APT_LOCK_PROBE       test-only command; exit 0 means the lock is held

set -euo pipefail

LOCK_TIMEOUT="${GWT_APT_LOCK_TIMEOUT:-180}"
POLL_SECONDS="${GWT_APT_LOCK_POLL:-5}"
ATTEMPTS="${GWT_APT_ATTEMPTS:-3}"
ATTEMPT_TIMEOUT="${GWT_APT_ATTEMPT_TIMEOUT:-240}"
RETRY_DELAY="${GWT_APT_RETRY_DELAY:-15}"
TOTAL_DEADLINE="${GWT_APT_TOTAL_DEADLINE:-900}"
ACQUIRE_TIMEOUT="${GWT_APT_ACQUIRE_TIMEOUT:-30}"
ACQUIRE_RETRIES="${GWT_APT_ACQUIRE_RETRIES:-3}"
APT_CONF_DIR="${GWT_APT_CONF_DIR:-/etc/apt/apt.conf.d}"
CACHE_DIR="${GWT_APT_CACHE_DIR:-}"
APT_GET="${GWT_APT_GET:-apt-get}"

# The tray + WebView build dependencies every Linux job installs. This is the
# one home for the set: the workflows key their actions/cache entry on a hash
# of this file, so changing the list here invalidates the cached archives
# instead of serving the previous ones.
GTK_PACKAGES=(
  libgtk-3-dev
  libwebkit2gtk-4.1-dev
  libayatana-appindicator3-dev
  libxdo-dev
)

# Sentinels above the range apt-get and GNU timeout use, so "we ran out of
# time" and "the lock never cleared" stay distinguishable from an apt failure
# in the reason line the job summary quotes.
DEADLINE_EXHAUSTED=199
LOCK_CONTENTION=198

LAST_REASON="unknown"
LAST_ATTEMPT_BUDGET="${ATTEMPT_TIMEOUT}"
ATTEMPTS_USED=0
held_lock_path=""

# Issue #4268: everything needed to say where a failed attempt's budget went.
LAST_STEP="unknown"
LAST_ELAPSED=0
LAST_LOCK_WAIT=0
LAST_DIAGNOSIS=""
CACHE_STATE="none"
CACHE_ARCHIVES=0
ATTEMPT_LOG=""

log() {
  printf '[ci-apt] %s\n' "$*"
}

cleanup() {
  [[ -n "${ATTEMPT_LOG}" ]] && rm -f "${ATTEMPT_LOG}"
  return 0
}
trap cleanup EXIT

# The workflows invoke this script under sudo, so the drop-in and the lock
# probe usually need no escalation; the fallback keeps a non-root caller
# (scripts/install-linux-deps.sh) working.
run_as_root() {
  if [[ "$(id -u)" -ne 0 ]] && command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    "$@"
  fi
}

# Seconds left before the whole invocation must stop. SECONDS counts from
# script start, so this shrinks across attempts and lock waits alike.
remaining_seconds() {
  local left=$((TOTAL_DEADLINE - SECONDS))
  if ((left < 0)); then
    left=0
  fi
  printf '%s' "${left}"
}

smaller() {
  if (($1 < $2)); then
    printf '%s' "$1"
  else
    printf '%s' "$2"
  fi
}

fuser_lock() {
  run_as_root fuser "$1" >/dev/null 2>&1
}

# Issue #4268: the root-cause fix. apt has no acquire timeout of its own, so a
# mirror that accepts the connection and stops sending is waited on until
# something outside apt gives up -- which costs the whole attempt and teaches
# the retry nothing. These bounds let apt fail the item in seconds and retry or
# fail over inside the attempt instead. Non-fatal on purpose: the per-attempt
# timeout still bounds the run, so refusing to install because a config file
# could not be written would trade a slow path for no path.
harden_apt() {
  if [[ ! -d "${APT_CONF_DIR}" ]]; then
    log "apt-hardening=skipped reason=${APT_CONF_DIR} does not exist"
    return 0
  fi

  local conf="${APT_CONF_DIR}/99-gwt-ci-apt-acquire"
  local body
  body="$(
    cat <<CONF
Acquire::Retries "${ACQUIRE_RETRIES}";
Acquire::http::Timeout "${ACQUIRE_TIMEOUT}";
Acquire::https::Timeout "${ACQUIRE_TIMEOUT}";
Acquire::ftp::Timeout "${ACQUIRE_TIMEOUT}";
Acquire::ForceIPv4 "true";
DPkg::Lock::Timeout "${LOCK_TIMEOUT}";
CONF
  )"

  if [[ -w "${APT_CONF_DIR}" ]]; then
    if ! printf '%s\n' "${body}" >"${conf}"; then
      log "apt-hardening=failed conf=${conf} (continuing; attempts stay bounded)"
      return 0
    fi
  elif ! printf '%s\n' "${body}" | run_as_root tee "${conf}" >/dev/null; then
    log "apt-hardening=failed conf=${conf} (continuing; attempts stay bounded)"
    return 0
  fi

  log "apt-hardening=applied conf=${conf} acquire_timeout=${ACQUIRE_TIMEOUT}s acquire_retries=${ACQUIRE_RETRIES}"
}

lock_is_held() {
  if [[ -n "${GWT_APT_LOCK_PROBE:-}" ]]; then
    held_lock_path="probe"
    if "${GWT_APT_LOCK_PROBE}"; then
      return 0
    fi
    return 1
  fi

  if ! command -v fuser >/dev/null 2>&1; then
    return 1
  fi

  local lock
  local locks=(
    /var/lib/dpkg/lock-frontend
    /var/lib/dpkg/lock
    /var/lib/apt/lists/lock
    /var/cache/apt/archives/lock
  )
  for lock in "${locks[@]}"; do
    if [[ -e "${lock}" ]] && fuser_lock "${lock}"; then
      held_lock_path="${lock}"
      return 0
    fi
  done
  return 1
}

wait_for_apt_lock() {
  local budget
  budget="$(smaller "${LOCK_TIMEOUT}" "$(remaining_seconds)")"
  local waited=0
  LAST_LOCK_WAIT=0
  while lock_is_held; do
    if ((waited >= budget)); then
      LAST_LOCK_WAIT="${waited}"
      log "reason=dpkg lock contention after ${waited}s holder=${held_lock_path}"
      return 1
    fi
    log "waiting for dpkg lock holder=${held_lock_path} waited=${waited}s timeout=${budget}s"
    sleep "${POLL_SECONDS}"
    waited=$((waited + POLL_SECONDS))
  done
  LAST_LOCK_WAIT="${waited}"
  if ((waited > 0)); then
    log "dpkg lock released after ${waited}s"
  fi
  return 0
}

# Bounds one apt-get call. Every Linux host that has apt-get also has
# coreutils `timeout`; the fallback keeps the script usable on hosts without
# it rather than refusing to run.
run_with_timeout() {
  local seconds="$1"
  shift
  local status=0

  if ((seconds <= 0)); then
    return "${DEADLINE_EXHAUSTED}"
  fi
  if command -v timeout >/dev/null 2>&1; then
    timeout -k 15 "${seconds}" "$@" || status=$?
  else
    "$@" || status=$?
  fi
  return "${status}"
}

apt_options() {
  APT_OPTIONS=(-o "DPkg::Lock::Timeout=${LOCK_TIMEOUT}")
  if [[ -n "${CACHE_DIR}" ]]; then
    APT_OPTIONS+=(-o "Dir::Cache::archives=${CACHE_DIR}")
  fi
}

count_archives() {
  find "${CACHE_DIR}" -maxdepth 1 -type f -name '*.deb' 2>/dev/null | wc -l | tr -d ' '
}

# apt refuses an archive directory without its partial/ subdirectory, and the
# cache action runs as the unprivileged runner user while apt writes as root,
# so the tree is made world-readable before it is saved.
#
# Issue #4268 AC-2: whether the restore actually hit is the difference between
# fetching 59.2 MB from a mirror and fetching nothing, and the failing runs were
# all cold. A run that starts cold says so and annotates it, so a cache key that
# never hits is visible from the run summary instead of only in the log body.
prepare_cache_dir() {
  [[ -n "${CACHE_DIR}" ]] || return 0
  mkdir -p "${CACHE_DIR}/partial"
  CACHE_ARCHIVES="$(count_archives)"
  if ((CACHE_ARCHIVES > 0)); then
    CACHE_STATE="warm"
  else
    CACHE_STATE="cold"
  fi
  log "cache=${CACHE_DIR} cache_state=${CACHE_STATE} archives=${CACHE_ARCHIVES}"
  if [[ "${CACHE_STATE}" == "cold" ]]; then
    printf '::warning title=Linux dependency cache miss::The apt archive cache at %s restored empty, so every package in this job is fetched from a mirror. A key that misses run after run is the reason a stalled mirror can cost the whole job.\n' \
      "${CACHE_DIR}"
  fi
}

publish_cache_dir() {
  [[ -n "${CACHE_DIR}" ]] || return 0
  chmod -R a+rX "${CACHE_DIR}" 2>/dev/null || true
  local saved
  saved="$(count_archives)"
  log "cache=${CACHE_DIR} cache_state=saved archives=${saved} fetched=$((saved - CACHE_ARCHIVES))"
}

# Issue #4268 AC-1: turns the captured apt output into the one line a reviewer
# needs. `timed out after 240s` says nothing about which mirror, which package
# or whether anything was downloaded at all; these fields do. `stalled_after`
# is last because it carries an unquoted mirror URL.
summarize_attempt() {
  local needed="unknown"
  local throughput="none"
  local acquired=0
  local stalled_after="nothing"

  if [[ -n "${ATTEMPT_LOG}" && -s "${ATTEMPT_LOG}" ]]; then
    local match
    match="$(grep -aoE 'Need to get [0-9.]+ ?[kMG]?B' "${ATTEMPT_LOG}" | tail -n 1 || true)"
    [[ -n "${match}" ]] && needed="${match#Need to get }"
    match="$(grep -aoE '\([0-9.]+ ?[kMG]?B/s\)' "${ATTEMPT_LOG}" | tail -n 1 || true)"
    [[ -n "${match}" ]] && throughput="${match//[()]/}"
    acquired="$(grep -acE '^(Get|Hit):' "${ATTEMPT_LOG}" || true)"
    match="$(grep -aE '^(Get|Hit|Ign|Err):' "${ATTEMPT_LOG}" | tail -n 1 || true)"
    [[ -n "${match}" ]] && stalled_after="${match}"
  fi

  LAST_DIAGNOSIS="step=${LAST_STEP} elapsed=${LAST_ELAPSED}s budget=${LAST_ATTEMPT_BUDGET}s lock_wait=${LAST_LOCK_WAIT}s cache_state=${CACHE_STATE} needed=${needed// /} acquired=${acquired:-0} throughput=${throughput// /} stalled_after=${stalled_after}"
}

run_apt_get() {
  LAST_STEP="$1"
  wait_for_apt_lock || return "${LOCK_CONTENTION}"
  # The last attempt before the total deadline is truncated to whatever is
  # left, so the reason line has to quote the budget actually applied rather
  # than the configured one.
  LAST_ATTEMPT_BUDGET="$(smaller "${ATTEMPT_TIMEOUT}" "$(remaining_seconds)")"
  log "cmd=${APT_GET} ${APT_OPTIONS[*]} $* timeout=${LAST_ATTEMPT_BUDGET}s"

  # Tee rather than redirect: the apt output still streams into the job log as
  # before, and the copy is what the diagnosis is read from. `timeout` signals
  # only the process group it created for apt-get, so this pipeline survives
  # the kill with everything apt managed to print already written.
  local started="${SECONDS}"
  local status=0
  : >"${ATTEMPT_LOG}"
  run_with_timeout "${LAST_ATTEMPT_BUDGET}" "${APT_GET}" "${APT_OPTIONS[@]}" "$@" 2>&1 |
    tee "${ATTEMPT_LOG}" || status=$?
  LAST_ELAPSED=$((SECONDS - started))
  return "${status}"
}

# One attempt at whatever this invocation was asked to do. `gtk-deps` refreshes
# the index and installs in the same attempt on purpose: a retry that reused a
# half-fetched index would install from stale metadata.
run_once() {
  local status=0
  if [[ "${MODE}" == "gtk-deps" ]]; then
    run_apt_get update || return $?
    run_apt_get install -y "${PACKAGES[@]}" || return $?
    return 0
  fi
  run_apt_get "${ARGS[@]}" || status=$?
  return "${status}"
}

describe_failure() {
  local status="$1"
  if ((status == 124 || status == 137)); then
    printf 'timed out after %ss' "${LAST_ATTEMPT_BUDGET}"
  elif ((status == DEADLINE_EXHAUSTED)); then
    printf 'total deadline of %ss exhausted' "${TOTAL_DEADLINE}"
  elif ((status == LOCK_CONTENTION)); then
    printf 'dpkg lock contention'
  else
    printf 'apt-get exited %s' "${status}"
  fi
}

run_with_retries() {
  local attempt=1
  local status=0
  while ((attempt <= ATTEMPTS)); do
    log "phase=${MODE} attempt=${attempt}/${ATTEMPTS} elapsed=${SECONDS}s deadline=${TOTAL_DEADLINE}s"
    status=0
    run_once || status=$?
    ATTEMPTS_USED="${attempt}"
    if ((status == 0)); then
      log "phase=${MODE} attempt=${attempt}/${ATTEMPTS} status=ok"
      return 0
    fi

    LAST_REASON="$(describe_failure "${status}")"
    log "phase=${MODE} attempt=${attempt}/${ATTEMPTS} status=failed reason=${LAST_REASON}"
    summarize_attempt
    log "diagnosis attempt=${attempt}/${ATTEMPTS} ${LAST_DIAGNOSIS}"

    if ((status == DEADLINE_EXHAUSTED)); then
      break
    fi
    attempt=$((attempt + 1))
    if ((attempt <= ATTEMPTS)); then
      if (($(remaining_seconds) <= RETRY_DELAY)); then
        LAST_REASON="total deadline of ${TOTAL_DEADLINE}s exhausted"
        break
      fi
      log "phase=${MODE} status=retrying in ${RETRY_DELAY}s"
      sleep "${RETRY_DELAY}"
    fi
  done
  return 1
}

# The whole point of owning the deadline: a dependency install that gives up
# says so, instead of leaving a job that ran no tests looking like a job whose
# tests failed.
report_failure() {
  log "phase=${MODE} status=failed attempts=${ATTEMPTS_USED}/${ATTEMPTS} reason=${LAST_REASON}"
  log "diagnosis final ${LAST_DIAGNOSIS}"
  printf '::error title=Linux dependency install failed::scripts/ci-apt.sh %s gave up after %s attempt(s): %s. No test or build step in this job ran. %s\n' \
    "${MODE}" "${ATTEMPTS_USED}" "${LAST_REASON}" "${LAST_DIAGNOSIS}"
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    {
      printf '### Linux dependency install failed - no tests were executed\n\n'
      printf '`scripts/ci-apt.sh %s` gave up after %s attempt(s): %s.\n\n' \
        "${MODE}" "${ATTEMPTS_USED}" "${LAST_REASON}"
      printf 'Where the last attempt spent its budget:\n\n```\n%s\n```\n\n' "${LAST_DIAGNOSIS}"
      printf 'This is an infrastructure failure in the dependency install step, not a test or code failure. Every later step in this job was skipped, so this run says nothing about the change under review.\n'
    } >>"${GITHUB_STEP_SUMMARY}"
  fi
}

if [[ "$#" -eq 0 ]]; then
  echo "usage: $(basename "$0") wait | harden | gtk-deps [packages...] | <apt-get arguments...>" >&2
  exit 2
fi

MODE="$1"

if [[ "${MODE}" == "wait" ]]; then
  wait_for_apt_lock
  exit $?
fi

# Exposed on its own so a caller that spawns apt-get itself can still get the
# acquire bounds without routing the install through this script.
if [[ "${MODE}" == "harden" ]]; then
  harden_apt
  exit 0
fi

PACKAGES=()
ARGS=()
if [[ "${MODE}" == "gtk-deps" ]]; then
  shift
  PACKAGES=("${GTK_PACKAGES[@]}")
  if (($# > 0)); then
    PACKAGES+=("$@")
  fi
else
  ARGS=("$@")
fi

ATTEMPT_LOG="$(mktemp "${TMPDIR:-/tmp}/ci-apt-attempt.XXXXXX")"
apt_options
harden_apt
prepare_cache_dir

if run_with_retries; then
  publish_cache_dir
  exit 0
fi

report_failure
exit 1
