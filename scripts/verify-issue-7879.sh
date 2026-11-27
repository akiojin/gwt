#!/usr/bin/env bash
# Issue #7879: isolated checkout UI verification of the command palette removal.
# Build gwt/gwtd first.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CHECKOUT_GWT="$REPO_ROOT/target/debug/gwt"
CHECKOUT_GWTD="$REPO_ROOT/target/debug/gwtd"
cd "$REPO_ROOT"
for CHECK_BIN in "$CHECKOUT_GWT" "$CHECKOUT_GWTD"; do
  if [ ! -x "$CHECK_BIN" ]; then
    echo 'Build the checkout first: cargo build -p gwt --bin gwt --bin gwtd' >&2
    exit 1
  fi
done
if ! command -v jq >/dev/null 2>&1; then
  echo 'browser-check preflight failed: jq is required; install jq and rerun' >&2
  exit 1
fi

CHECK_HOME="$(mktemp -d -t gwt-issue-7879-home.XXXXXX)"
URL_FILE="$CHECK_HOME/url"
LOG_FILE="$CHECK_HOME/startup.log"
mkdir -p "$CHECK_HOME/.gwt"
if [ -d "$HOME/.gwt/runtime" ]; then
  if command -v cygpath >/dev/null 2>&1; then
    # MSYS ln can copy directories; a junction keeps this cache input linked.
    CHECK_RUNTIME_SOURCE="$(cygpath -w "$HOME/.gwt/runtime")" \
      CHECK_RUNTIME_LINK="$(cygpath -w "$CHECK_HOME/.gwt/runtime")" \
      powershell.exe -NoProfile -Command \
      'New-Item -ItemType Junction -Path $env:CHECK_RUNTIME_LINK -Target $env:CHECK_RUNTIME_SOURCE -ErrorAction Stop | Out-Null'
  else
    ln -s "$HOME/.gwt/runtime" "$CHECK_HOME/.gwt/runtime"
  fi
fi
# This shell-only check needs the runtime input above; GitHub authentication
# is forwarded below. Provider and SSH credential trees are unnecessary.
jq -n --arg root "$REPO_ROOT" \
  '{tabs:[{id:"issue-7879",title:"Issue 7879",project_root:$root,kind:"git"}],recent_projects:[]}' \
  > "$CHECK_HOME/.gwt/session.json"
CHECK_REPO_HASH="$(
  printf '%s\n' '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' \
    | "$CHECKOUT_GWTD" | jq -er 'select(.ok == true) | .project_store.hash'
)"

# browser-check-agent-seed-begin
CHECK_PROJECT_STATE="$CHECK_HOME/.gwt/projects/${CHECK_REPO_HASH:?Resolve project_store.hash first}/project-state"
mkdir -p "$CHECK_PROJECT_STATE"
cat > "$CHECK_PROJECT_STATE/pm.json" <<'JSON'
{"settings":{"auto_start":false}}
JSON
cat > "$CHECK_PROJECT_STATE/issue-monitor.json" <<'JSON'
{"enabled":false,"max_active_agents":1,"priority_order":[]}
JSON
# browser-check-agent-seed-end
jq -n --arg project "$REPO_ROOT" --arg repo_hash "$CHECK_REPO_HASH" \
  '{project:$project,repo_hash:$repo_hash}' > "$CHECK_HOME/issue-7879-isolated.json"

# browser-check-hook-authority-begin
CHECKOUT_LOCAL_HOOK_PATTERN='(^|[/\\])target[/\\]([^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?([^[:alnum:]_.-]|$)'

is_checkout_local_hook_bin() {
  [ -n "$1" ] \
    && printf '%s\n' "$1" | grep -qiE -- "$CHECKOUT_LOCAL_HOOK_PATTERN"
}

CHECK_HOOK_BIN="${GWT_HOOK_BIN:-}"
if is_checkout_local_hook_bin "$CHECK_HOOK_BIN"; then
  CHECK_HOOK_BIN=""
fi
if [ -n "$CHECK_HOOK_BIN" ] && [ "$CHECK_HOOK_BIN" != "gwtd" ] \
  && [ ! -x "$CHECK_HOOK_BIN" ]; then
  CHECK_HOOK_BIN=""
fi
if [ -z "$CHECK_HOOK_BIN" ]; then
  if ! command -v jq >/dev/null 2>&1; then
    echo "browser-check preflight failed: jq is required to parse hook.doctor evidence; install jq (for example: brew install jq) and rerun" >&2
    exit 1
  fi
  if ! CHECK_HOOK_RESOLVER="$(
    cd "$REPO_ROOT" && printf '%s\n' '{"schema_version":1,"operation":"hook.doctor","params":{}}' \
      | env -u GWT_BIN_PATH -u GWT_HOOK_BIN "$CHECKOUT_GWTD"
  )" || ! CHECK_HOOK_BIN="$(
    printf '%s' "$CHECK_HOOK_RESOLVER" \
      | jq -er 'select(.ok == true) | .output | fromjson | .managed_hook_bin | strings | select(length > 0)'
  )"; then
    echo "browser-check managed-hook fallback resolver failed: rebuild checkout gwt/gwtd and rerun" >&2
    exit 1
  fi
  if is_checkout_local_hook_bin "$CHECK_HOOK_BIN"; then
    echo "browser-check managed-hook fallback resolver returned a checkout-local binary" >&2
    exit 1
  fi
fi
# browser-check-hook-authority-end

# browser-check-hook-repair-begin
if ! command -v jq >/dev/null 2>&1; then
  echo "browser-check preflight failed: jq is required to parse hook.doctor evidence; install jq (for example: brew install jq) and rerun" >&2
  exit 1
fi
HOOK_DOCTOR_ENVELOPE="$(
  jq -n \
    --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    '{schema_version:1,operation:"hook.doctor",params:{purpose:"managed_browser",repair:true,expected_hook_bin:$expected_hook_bin}}' \
  | (cd "$REPO_ROOT" && env -u GWT_BIN_PATH GWT_HOOK_BIN="$CHECK_HOOK_BIN" "$CHECKOUT_GWTD")
)"
if ! HOOK_DOCTOR_HEALTH_JSON="$(
  printf '%s' "$HOOK_DOCTOR_ENVELOPE" \
  | jq -er 'select(.ok == true) | .output | fromjson'
)"; then
  echo "browser-check hook convergence failed: hook.doctor did not return evidence" >&2
  exit 1
fi
if ! printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" \
  | jq -e '
      .preflight.result == "proceed"
      and .preflight.schema_version == 1
      and .preflight.context == "current"
      and .preflight.normal_completion == "normal"
      and .preflight.convergence == "converged"
      and (.preflight.reasons | type == "array" and length == 0)
      and (.preflight.diagnostics | type == "array")
      and all(.preflight.diagnostics[];
        (.id | type == "string" and length > 0)
        and (.classification as $kind
          | ["recovered", "fail_open", "foreign"]
          | index($kind) != null))
    ' >/dev/null; then
  echo "browser-check hook convergence failed: hook.doctor could not converge managed surfaces and current source" >&2
  printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" | jq '{preflight, health}' >&2
  echo 'Recovery: inspect preflight reason codes; repair managed configuration, use normal trust/launch recovery, and wait for a real normal hook completion in the current Session. Rebuild an older doctor that has no typed preflight.' >&2
  exit 1
fi
# browser-check-hook-repair-end

CHECK_GH_TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
if [ -z "$CHECK_GH_TOKEN" ] && command -v gh >/dev/null 2>&1; then
  CHECK_GH_TOKEN="$(gh auth token 2>/dev/null || true)"
fi
ENV_ARGS=(HOME="$CHECK_HOME" USERPROFILE="$CHECK_HOME" GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1
  GWT_BROWSER_URL_FILE="$URL_FILE" GWT_HOOK_BIN="$CHECK_HOOK_BIN")
if [ -n "$CHECK_GH_TOKEN" ]; then ENV_ARGS+=(GH_TOKEN="$CHECK_GH_TOKEN" GITHUB_TOKEN="$CHECK_GH_TOKEN"); fi

# browser-check-process-cleanup-begin
CHECK_PID=""
cleanup_browser_check() {
  if [ -z "$CHECK_PID" ]; then return 0; fi
  if kill -0 "$CHECK_PID" 2>/dev/null; then
    kill -TERM "$CHECK_PID" 2>/dev/null || true
    for ((CHECK_WAIT=0; CHECK_WAIT<100; CHECK_WAIT++)); do
      if ! kill -0 "$CHECK_PID" 2>/dev/null; then break; fi
      sleep 0.1
    done
    if kill -0 "$CHECK_PID" 2>/dev/null; then kill -KILL "$CHECK_PID" 2>/dev/null || true; fi
  fi
  wait "$CHECK_PID" 2>/dev/null || true
  if kill -0 "$CHECK_PID" 2>/dev/null; then
    echo "browser-check cleanup failed: owned PID $CHECK_PID remains" >&2
    return 1
  fi
  CHECK_PID=""
}
trap 'CHECK_EXIT_STATUS=$?; trap - EXIT; cleanup_browser_check || CHECK_EXIT_STATUS=1; exit "$CHECK_EXIT_STATUS"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
# browser-check-process-cleanup-end
# browser-check-launch-begin
env -u GWT_BIN_PATH "${ENV_ARGS[@]}" "$CHECKOUT_GWT" --no-tray --no-open > "$LOG_FILE" 2>&1 &
CHECK_PID=$!
printf 'Fresh gwt PID: %s; isolated HOME: %s\n' "$CHECK_PID" "$CHECK_HOME"
# browser-check-launch-end
CHECK_URL=""
for ((CHECK_WAIT=0; CHECK_WAIT<600; CHECK_WAIT++)); do
  if ! kill -0 "$CHECK_PID" 2>/dev/null; then
    tail -n 60 "$LOG_FILE" >&2
    exit 1
  fi
  if [ -s "$URL_FILE" ]; then
    CHECK_URL="$(cat "$URL_FILE")"
  else
    CHECK_URL="$(sed -nE 's/.*(http:\/\/127\.0\.0\.1:[0-9]+\/).*/\1/p' "$LOG_FILE" | tail -n 1)"
  fi
  if [ -n "$CHECK_URL" ] && curl -fsS -I "$CHECK_URL" >/dev/null 2>&1; then break; fi
  CHECK_URL=""
  sleep 0.1
done
if [ -z "$CHECK_URL" ] || grep -qi 'another tray-resident gwt instance is already running' "$LOG_FILE"; then
  tail -n 60 "$LOG_FILE" >&2
  echo 'browser-check startup failed' >&2
  exit 1
fi
printf '%s\n' "$CHECK_URL" > "$URL_FILE"

# browser-check-hook-audit-begin
HOOK_HEALTH_ENVELOPE="$(
  jq -n \
    --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    '{schema_version:1,operation:"hook.doctor",params:{purpose:"managed_browser",expected_hook_bin:$expected_hook_bin}}' \
  | (cd "$REPO_ROOT" && env -u GWT_BIN_PATH "$CHECKOUT_GWTD")
)"
if ! HOOK_HEALTH_JSON="$(
  printf '%s' "$HOOK_HEALTH_ENVELOPE" \
  | jq -er 'select(.ok == true) | .output | fromjson'
)"; then
  echo "browser-check hook convergence failed: hook.doctor did not return evidence" >&2
  exit 1
fi
if ! printf '%s' "$HOOK_HEALTH_JSON" \
  | jq -e '
      .preflight.result == "proceed"
      and .preflight.schema_version == 1
      and .preflight.context == "current"
      and .preflight.normal_completion == "normal"
      and .preflight.convergence == "converged"
      and (.preflight.reasons | type == "array" and length == 0)
      and (.preflight.diagnostics | type == "array")
      and all(.preflight.diagnostics[];
        (.id | type == "string" and length > 0)
        and (.classification as $kind
          | ["recovered", "fail_open", "foreign"]
          | index($kind) != null))
    ' >/dev/null; then
  echo "browser-check hook convergence failed: managed hook surfaces did not converge or current source is unproven" >&2
  printf '%s' "$HOOK_HEALTH_JSON" | jq '{preflight, health}' >&2
  echo 'Recovery: inspect the fixed preflight reasons; use normal configuration/trust/launch recovery and a real normal hook event. Missing or unknown typed evidence fails closed.' >&2
  exit 1
fi
# browser-check-hook-audit-end

# Keep the runner on the real HOME so its pinned browser cache is available.
# Only the owned checkout process receives CHECK_HOME. Extra arguments include
# canonical verify.run's --headed and embedded reporter.
GWT_PLAYWRIGHT_BASE_URL="$CHECK_URL" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" \
  GWT_PLAYWRIGHT_PROJECT_ROOT="$REPO_ROOT" \
  bash "$REPO_ROOT/scripts/run-visual-tests.sh" \
    --workers=1 \
    crates/gwt/playwright/tests/chrome.spec.ts \
    crates/gwt/playwright/tests/clone-project-modal.spec.ts \
    crates/gwt/playwright/tests/launch-wizard-picker-pending.spec.ts \
    crates/gwt/playwright/tests/migration-modal.spec.ts \
    crates/gwt/playwright/tests/pm-terminal-rendering.spec.ts \
    crates/gwt/playwright/tests/project-hub.spec.ts \
    crates/gwt/playwright/tests/project-open-surfaces.spec.ts \
    crates/gwt/playwright/tests/prompt-runtime-responsiveness-live.spec.ts \
    crates/gwt/playwright/tests/ui-trace-feedback.spec.ts "$@"
