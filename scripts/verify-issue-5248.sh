#!/usr/bin/env bash
# Issue #5248: isolated checkout UI verification. Build gwt/gwtd first.
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

CHECK_HOME="$(mktemp -d -t gwt-issue-5248-home.XXXXXX)"
URL_FILE="$CHECK_HOME/url"
LOG_FILE="$CHECK_HOME/startup.log"
mkdir -p "$CHECK_HOME/.gwt"
if [ -d "$HOME/.gwt/runtime" ]; then
  ln -s "$HOME/.gwt/runtime" "$CHECK_HOME/.gwt/runtime"
fi
for CHECK_INPUT in .codex .claude .config .docker .ssh .gitconfig .git-credentials .npmrc .bunfig.toml; do
  if [ -e "$HOME/$CHECK_INPUT" ]; then
    ln -s "$HOME/$CHECK_INPUT" "$CHECK_HOME/$CHECK_INPUT"
  fi
done
jq -n --arg root "$REPO_ROOT" \
  '{tabs:[{id:"issue-5248",title:"Issue 5248",project_root:$root,kind:"git"}],recent_projects:[]}' \
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
  '{project:$project,repo_hash:$repo_hash}' > "$CHECK_HOME/issue-5248-isolated.json"

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

# The managed surfaces are credential/config symlinks, so audit their stable
# fallback with the real HOME, as required by browser-check.
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
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    --arg runtime_state_path "$CHECK_HOME/.gwt/browser-check-missing-runtime-state.json" \
    '{schema_version:1,operation:"hook.health",params:{expected_hook_bin:$expected_hook_bin,runtime_state_path:$runtime_state_path}}' \
    | env -u GWT_BIN_PATH "$CHECKOUT_GWTD"
)"
HOOK_HEALTH_JSON="$(printf '%s' "$HOOK_HEALTH_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson')"
printf '%s' "$HOOK_HEALTH_JSON" | assert_hook_health
# browser-check-hook-audit-end

# Keep the runner on the real HOME so its pinned browser cache is available.
# Only the owned checkout process receives CHECK_HOME. Extra arguments include
# canonical verify.run's --headed and embedded reporter.
GWT_PLAYWRIGHT_BASE_URL="$CHECK_URL" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" \
  GWT_PLAYWRIGHT_PROJECT_ROOT="$REPO_ROOT" \
  bash "$REPO_ROOT/scripts/run-visual-tests.sh" \
    --workers=1 crates/gwt/playwright/tests/issue-monitor-close-snapshot-live.spec.ts "$@"
