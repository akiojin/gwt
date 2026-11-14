#!/usr/bin/env bash
# Issue #5023: isolated checkout UI verification. Build gwt/gwtd first.
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

CHECK_HOME="$(mktemp -d -t gwt-issue-5023-home.XXXXXX)"
URL_FILE="$CHECK_HOME/url"
LOG_FILE="$CHECK_HOME/startup.log"
mkdir -p "$CHECK_HOME/.gwt"
if [ -d "$HOME/.gwt/runtime" ]; then
  ln -s "$HOME/.gwt/runtime" "$CHECK_HOME/.gwt/runtime"
fi
jq -n --arg root "$REPO_ROOT" \
  '{tabs:[{id:"issue-5023",title:"Issue 5023",project_root:$root,kind:"git"}],recent_projects:[]}' \
  > "$CHECK_HOME/.gwt/session.json"
CHECK_REPO_HASH="$(
  printf '%s\n' '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' \
    | "$CHECKOUT_GWTD" | jq -er 'select(.ok == true) | .project_store.hash'
)"

# browser-check-agent-seed-begin
CHECK_PROJECT_STATE="$CHECK_HOME/.gwt/projects/${CHECK_REPO_HASH:?Resolve project_store.hash first}/project-state"
mkdir -p "$CHECK_PROJECT_STATE"
jq -n --arg root "$REPO_ROOT" \
  '{settings:{auto_start:false},registration:{session_id:"pm-reports-fixture",agent_id:"codex",worktree_path:$root,created_at:"2026-10-11T00:00:00Z"}}' \
  > "$CHECK_PROJECT_STATE/pm.json"
cat > "$CHECK_PROJECT_STATE/issue-monitor.json" <<'JSON'
{"enabled":false,"max_active_agents":1,"priority_order":[]}
JSON
# A stopped registered PM exposes Reports without launching another agent.
jq -n '{viewport:{x:0,y:0,zoom:1},windows:[{id:"pm-reports",title:"PM",preset:"agent",geometry:{x:30,y:30,width:1050,height:720},z_index:1,status:"stopped",session_id:"pm-reports-fixture",agent_id:"codex",persist:true}],next_z_index:2}' \
  > "$CHECK_HOME/.gwt/projects/$CHECK_REPO_HASH/workspace.json"
mkdir -p "$CHECK_HOME/.gwt/sessions"
cat > "$CHECK_HOME/.gwt/sessions/pm-reports-fixture.toml" <<TOML
id = "pm-reports-fixture"
worktree_path = "$REPO_ROOT"
branch = "work/issue-5023"
agent_id = { type = "Codex" }
status = "Stopped"
created_at = "2026-10-11T00:00:00Z"
updated_at = "2026-10-11T00:00:00Z"
last_activity_at = "2026-10-11T00:00:00Z"
TOML
# browser-check-agent-seed-end
jq -n --arg project "$REPO_ROOT" --arg repo_hash "$CHECK_REPO_HASH" \
  '{project:$project,repo_hash:$repo_hash}' > "$CHECK_HOME/issue-5023-isolated.json"

# browser-check-hook-authority-begin
CHECKOUT_LOCAL_HOOK_PATTERN='(^|[/\\])target[/\\]([^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?([^[:alnum:]_.-]|$)'
is_checkout_local_hook_bin() {
  [ -n "$1" ] && printf '%s\n' "$1" | grep -qiE -- "$CHECKOUT_LOCAL_HOOK_PATTERN"
}
CHECK_HOOK_BIN="${GWT_HOOK_BIN:-}"
if is_checkout_local_hook_bin "$CHECK_HOOK_BIN"; then CHECK_HOOK_BIN=""; fi
if [ -n "$CHECK_HOOK_BIN" ] && [ "$CHECK_HOOK_BIN" != gwtd ] && [ ! -x "$CHECK_HOOK_BIN" ]; then
  CHECK_HOOK_BIN=""
fi
if [ -z "$CHECK_HOOK_BIN" ]; then
  PATH_HOOK_BIN="$(command -v gwtd 2>/dev/null || true)"
  if [ -n "$PATH_HOOK_BIN" ] && ! is_checkout_local_hook_bin "$PATH_HOOK_BIN"; then
    CHECK_HOOK_BIN=gwtd
  fi
fi
if [ -z "$CHECK_HOOK_BIN" ] && [ -x /Applications/GWT.app/Contents/MacOS/gwtd ]; then
  CHECK_HOOK_BIN=/Applications/GWT.app/Contents/MacOS/gwtd
fi
CHECK_HOOK_BIN="${CHECK_HOOK_BIN:-gwtd}"
# browser-check-hook-authority-end

# Audit the managed hook fallback with the real HOME, as required by browser-check.
ALLOW_MISSING_LOGICAL_FALLBACK=false
if [ "$CHECK_HOOK_BIN" = gwtd ] && ! command -v gwtd >/dev/null 2>&1; then
  ALLOW_MISSING_LOGICAL_FALLBACK=true
fi
assert_hook_health() {
  if ! jq -e --argjson allow_missing_logical "$ALLOW_MISSING_LOGICAL_FALLBACK" '
    [.issues[] | select(
      ($allow_missing_logical and startswith("managed hook binary missing: ") and endswith(" uses gwtd"))
      or (startswith("managed hook failure: ") and test(" state=fail-open( |$)")) | not)] as $blocking
    | .status != "inactive" and ($blocking | length == 0)
  ' >/dev/null; then
    echo 'browser-check hook convergence failed' >&2
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
HOOK_DOCTOR_HEALTH_JSON="$(printf '%s' "$HOOK_DOCTOR_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson | .health')"
printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" > "$CHECK_HOME/hook-doctor-health.json"
if ! printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" | assert_hook_health; then
  printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" | jq '{status,issues}' >&2
  exit 1
fi
# browser-check-hook-repair-end

ENV_ARGS=(HOME="$CHECK_HOME" USERPROFILE="$CHECK_HOME" GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1
  GWT_BROWSER_URL_FILE="$URL_FILE" GWT_HOOK_BIN="$CHECK_HOOK_BIN")

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
for CHECK_LAUNCH in 1 2; do
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
if [ "$CHECK_LAUNCH" = 1 ]; then
  # Save while the first app is alive, then verify the same history after a
  # real app shutdown/relaunch (not merely a browser reload).
  printf '%s\n' '{"schema_version":1,"operation":"pm.report.post","params":{"kind":"decision","body":"# Persisted before restart\n\nThis report survives the application restart."}}' \
    | env -u GWT_SESSION_ID -u GWT_SESSION_RUNTIME_PATH -u GWT_HOOK_FORWARD_URL \
      -u GWT_HOOK_FORWARD_TOKEN -u GWT_PANE_WS_URL -u GWT_BIN_PATH \
      HOME="$CHECK_HOME" USERPROFILE="$CHECK_HOME" "$CHECKOUT_GWTD" \
    | jq -e 'select(.ok == true) | .output | fromjson | select(.ok == true) | .report.id' >/dev/null
  cleanup_browser_check
  cp "$LOG_FILE" "$CHECK_HOME/startup-before-restart.log"
  : > "$URL_FILE"
fi
done

# browser-check-hook-audit-begin
HOOK_HEALTH_ENVELOPE="$(
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    --arg runtime_state_path "$CHECK_HOME/.gwt/browser-check-missing-runtime-state.json" \
    '{schema_version:1,operation:"hook.health",params:{expected_hook_bin:$expected_hook_bin,runtime_state_path:$runtime_state_path}}' \
    | env -u GWT_BIN_PATH "$CHECKOUT_GWTD"
)"
HOOK_HEALTH_JSON="$(printf '%s' "$HOOK_HEALTH_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson')"
printf '%s' "$HOOK_HEALTH_JSON" > "$CHECK_HOME/hook-health.json"
if ! printf '%s' "$HOOK_HEALTH_JSON" | assert_hook_health; then
  printf '%s' "$HOOK_HEALTH_JSON" | jq '{status,issues}' >&2
  exit 1
fi
# browser-check-hook-audit-end

# Keep the runner on the real HOME so its pinned browser cache is available.
# Only the owned checkout process receives CHECK_HOME. Extra arguments include
# canonical verify.run's --headed and embedded reporter.
GWT_PLAYWRIGHT_BASE_URL="$CHECK_URL" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" \
  GWT_PLAYWRIGHT_PROJECT_ROOT="$REPO_ROOT" \
  bash "$REPO_ROOT/scripts/run-visual-tests.sh" \
    --workers=1 --output "$CHECK_HOME/playwright-artifacts" \
    crates/gwt/playwright/tests/pm-chat-live.spec.ts \
    crates/gwt/playwright/tests/release-notes-live.spec.ts "$@"
