#!/usr/bin/env bash
# Issue #3287: isolated checkout notification verification. Build gwt/gwtd first.
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

CHECK_HOME="$(mktemp -d -t gwt-issue-3287-home.XXXXXX)"
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
  '{tabs:[{id:"issue-3287",title:"Issue 4540 Check",project_root:$root,kind:"git"}],recent_projects:[]}' \
  > "$CHECK_HOME/.gwt/session.json"
CHECK_REPO_HASH="$(
  printf '%s\n' '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' \
    | "$CHECKOUT_GWTD" | jq -er 'select(.ok == true) | .project_store.hash'
)"

# The existing metadata live regression requires one restored stopped Agent.
# This fixture has no Session/command and never starts a provider process.
mkdir -p "$CHECK_HOME/.gwt/projects/$CHECK_REPO_HASH"
jq -n '{viewport:{x:0,y:0,zoom:1},windows:[{
  id:"metadata-agent",title:"Metadata fixture",preset:"agent",
  geometry:{x:40,y:40,width:640,height:420},z_index:1,status:"stopped"
}],next_z_index:2}' > "$CHECK_HOME/.gwt/projects/$CHECK_REPO_HASH/workspace.json"

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

# Audit the managed hook fallback with the real HOME, as required by browser-check.
ALLOW_MISSING_LOGICAL_FALLBACK=false
if [ "$CHECK_HOOK_BIN" = gwtd ] && ! command -v gwtd >/dev/null 2>&1; then
  ALLOW_MISSING_LOGICAL_FALLBACK=true
fi
assert_hook_health() {
  local health_json
  health_json="$(cat)"
  if ! printf '%s' "$health_json" | jq -e --argjson allow_missing_logical "$ALLOW_MISSING_LOGICAL_FALLBACK" '
    [.issues[] | select(
      ($allow_missing_logical and startswith("managed hook binary missing: ") and endswith(" uses gwtd"))
      or (startswith("managed hook failure: ") and test(" state=fail-open( |$)")) | not)] as $blocking
    | .status != "inactive" and ($blocking | length == 0)
  ' >/dev/null; then
    echo 'browser-check hook convergence failed' >&2
    printf '%s' "$health_json" | jq '{status, issues}' >&2
    return 1
  fi
}
# browser-check-hook-repair-begin
HOOK_DOCTOR_ENVELOPE="$(
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    --arg runtime_state_path "$CHECK_HOME/.gwt/browser-check-missing-runtime-state.json" \
    '{schema_version:1,operation:"hook.doctor",params:{repair:true,expected_hook_bin:$expected_hook_bin,runtime_state_path:$runtime_state_path}}' \
    | env -u GWT_BIN_PATH GWT_HOOK_BIN="$CHECK_HOOK_BIN" "$CHECKOUT_GWTD"
)"
# Issue #5270: the shared root can retain an older managed-hook identity
# after repair. Apply the diagnostic's canonical recovery, limited to verified
# gwt-managed commands, then require the full health audit to pass.
printf '%s' "$HOOK_DOCTOR_ENVELOPE" | jq -e 'select(.ok == true)' >/dev/null
HOOK_TRUST_ENVELOPE="$(
  jq -n --arg root "$REPO_ROOT" --arg config "$HOME/.codex/config.toml" \
    '{schema_version:1,operation:"hook.register_codex_managed_hook_trust",params:{project_root:$root,codex_config:$config,codex_hook_discovery:"both"}}' \
    | env -u GWT_BIN_PATH GWT_HOOK_BIN="$CHECK_HOOK_BIN" "$CHECKOUT_GWTD"
)"
printf '%s' "$HOOK_TRUST_ENVELOPE" | jq -e 'select(.ok == true)' >/dev/null
HOOK_RECOVERY_ENVELOPE="$(
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    --arg runtime_state_path "$CHECK_HOME/.gwt/browser-check-missing-runtime-state.json" \
    '{schema_version:1,operation:"hook.health",params:{expected_hook_bin:$expected_hook_bin,runtime_state_path:$runtime_state_path}}' \
    | env -u GWT_BIN_PATH "$CHECKOUT_GWTD"
)"
HOOK_DOCTOR_HEALTH_JSON="$(printf '%s' "$HOOK_RECOVERY_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson')"
printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" | assert_hook_health
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
CHECK_KEEPALIVE_PID=""
cleanup_browser_check() {
  if [ -n "$CHECK_KEEPALIVE_PID" ]; then
    kill -TERM "$CHECK_KEEPALIVE_PID" 2>/dev/null || true
    wait "$CHECK_KEEPALIVE_PID" 2>/dev/null || true
    CHECK_KEEPALIVE_PID=""
  fi
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
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    --arg runtime_state_path "$CHECK_HOME/.gwt/browser-check-missing-runtime-state.json" \
    '{schema_version:1,operation:"hook.health",params:{expected_hook_bin:$expected_hook_bin,runtime_state_path:$runtime_state_path}}' \
    | env -u GWT_BIN_PATH "$CHECKOUT_GWTD"
)"
HOOK_HEALTH_JSON="$(printf '%s' "$HOOK_HEALTH_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson')"
printf '%s' "$HOOK_HEALTH_JSON" | assert_hook_health
# browser-check-hook-audit-end

# Issue #5219: fixture-only tests leave no live browser connected. Retain
# one read-only Hub connection until the suite ends so --no-tray cannot exit
# during that gap. This client and the server belong to this owner shell.
CHECK_KEEPALIVE_READY="$CHECK_HOME/keepalive-ready"
node --input-type=module - "$CHECK_URL" "$CHECK_KEEPALIVE_READY" \
  > "$CHECK_HOME/keepalive.log" 2>&1 <<'JS' &
import { writeFileSync } from "node:fs";
const url = new URL("/ws", process.argv[2]);
url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
const socket = new WebSocket(url);
socket.addEventListener("open", () => writeFileSync(process.argv[3], "ready"));
socket.addEventListener("error", (event) => { console.error(event.error); process.exit(1); });
socket.addEventListener("close", () => process.exit(1));
JS
CHECK_KEEPALIVE_PID=$!
for ((CHECK_WAIT=0; CHECK_WAIT<100; CHECK_WAIT++)); do
  if [ -s "$CHECK_KEEPALIVE_READY" ]; then break; fi
  if ! kill -0 "$CHECK_KEEPALIVE_PID" 2>/dev/null; then break; fi
  sleep 0.1
done
if [ ! -s "$CHECK_KEEPALIVE_READY" ] || ! kill -0 "$CHECK_KEEPALIVE_PID" 2>/dev/null; then
  cat "$CHECK_HOME/keepalive.log" >&2
  echo 'browser-check Hub connection failed' >&2
  exit 1
fi

# Keep the runner on the real HOME so its pinned browser cache is available.
# Only the owned checkout process receives CHECK_HOME. Extra arguments include
# canonical verify.run's --headed and embedded reporter.
if [ -n "${GWT_PLAYWRIGHT_LIVE_SPEC:-}" ]; then
  CHECK_SPECS=("$GWT_PLAYWRIGHT_LIVE_SPEC")
else
  CHECK_SPECS=(
    crates/gwt/playwright/tests/agent-state-notifications.spec.ts
    crates/gwt/playwright/tests/project-notifications.spec.ts
    crates/gwt/playwright/tests/project-page-metadata.spec.ts
  )
fi
GWT_PLAYWRIGHT_BASE_URL="$CHECK_URL" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" \
  GWT_PLAYWRIGHT_PROJECT_ROOT="$REPO_ROOT" \
  GWT_PLAYWRIGHT_GWTD="$CHECKOUT_GWTD" GWT_PLAYWRIGHT_PROJECT_HASH="$CHECK_REPO_HASH" \
  GWT_PLAYWRIGHT_PROJECT_KEY="$CHECK_REPO_HASH" \
  bash "$REPO_ROOT/scripts/run-visual-tests.sh" \
    --workers=1 "${CHECK_SPECS[@]}" "$@"

if ! kill -0 "$CHECK_KEEPALIVE_PID" 2>/dev/null; then
  cat "$CHECK_HOME/keepalive.log" >&2
  echo 'browser-check Hub connection dropped during the suite' >&2
  exit 1
fi
