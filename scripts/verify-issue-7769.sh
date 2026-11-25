#!/usr/bin/env bash
# Issue #7769: run the Launch Wizard live specs against an isolated instance of
# this checkout (browser-check procedure). Build first:
#   cargo build -p gwt --bin gwt --bin gwtd
# Host prerequisites: Claude Code and Codex installed, Codex started at least
# once (it writes models_cache.json), `agy` and `openclaw` not installed.
# Extra arguments go to Playwright, including verify.run's --headed and reporter.
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CHECKOUT_GWT="$REPO_ROOT/target/debug/gwt"
CHECKOUT_GWTD="$REPO_ROOT/target/debug/gwtd"
REAL_HOME="$HOME"
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

CHECK_HOME="$(mktemp -d -t gwt-issue-7769-home.XXXXXX)"
URL_FILE="$CHECK_HOME/url"
LOG_FILE="$CHECK_HOME/startup.log"
mkdir -p "$CHECK_HOME/.gwt" "$CHECK_HOME/bin"
# The Hermes spec needs an installed but unconfigured Hermes. Detection only
# asks the CLI for its version, so a stand-in keeps the host free of a real one.
cat > "$CHECK_HOME/bin/hermes" <<'SH'
#!/bin/sh
echo "Hermes Agent v0.9.0"
SH
chmod +x "$CHECK_HOME/bin/hermes"
if [ -e "$REAL_HOME/.gwt/runtime" ]; then
  ln -s "$REAL_HOME/.gwt/runtime" "$CHECK_HOME/.gwt/runtime"
fi
# The wizard reads the detected CLIs' own configuration; `.codex` also holds
# the models cache that the Codex picker spec compares against.
for CHECK_INPUT in .codex .claude .config .docker .ssh .gitconfig .git-credentials .npmrc .bunfig.toml; do
  if [ -e "$REAL_HOME/$CHECK_INPUT" ]; then
    ln -s "$REAL_HOME/$CHECK_INPUT" "$CHECK_HOME/$CHECK_INPUT"
  fi
done
jq -n --arg root "$REPO_ROOT" \
  '{tabs:[{id:"issue-7769",title:"Issue 7769",project_root:$root,kind:"git"}],active_tab_id:"issue-7769",recent_projects:[]}' \
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
  if ! CHECK_HOOK_RESOLVER="$(
    printf '%s\n' '{"schema_version":1,"operation":"hook.doctor","params":{}}' \
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

assert_hook_preflight() {
  if ! HOOK_PREFLIGHT_JSON="$(jq -er 'select(.ok == true) | .output | fromjson')"; then
    echo "browser-check hook convergence failed: hook.doctor did not return evidence" >&2
    exit 1
  fi
  if ! printf '%s' "$HOOK_PREFLIGHT_JSON" \
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
    printf '%s' "$HOOK_PREFLIGHT_JSON" | jq '{preflight, health}' >&2
    echo 'Recovery: inspect the fixed preflight reasons; use normal configuration/trust/launch recovery and a real normal hook event. Missing or unknown typed evidence fails closed.' >&2
    exit 1
  fi
}

# browser-check-hook-repair-begin
jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
  '{schema_version:1,operation:"hook.doctor",params:{purpose:"managed_browser",repair:true,expected_hook_bin:$expected_hook_bin}}' \
  | env -u GWT_BIN_PATH GWT_HOOK_BIN="$CHECK_HOOK_BIN" "$CHECKOUT_GWTD" \
  | assert_hook_preflight
# browser-check-hook-repair-end

CHECK_GH_TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
if [ -z "$CHECK_GH_TOKEN" ] && command -v gh >/dev/null 2>&1; then
  CHECK_GH_TOKEN="$(gh auth token 2>/dev/null || true)"
fi
ENV_ARGS=(
  HOME="$CHECK_HOME"
  USERPROFILE="$CHECK_HOME"
  PATH="$CHECK_HOME/bin:$PATH"
  GIT_TERMINAL_PROMPT=0
  GH_PROMPT_DISABLED=1
  GWT_BROWSER_URL_FILE="$URL_FILE"
  GWT_HOOK_BIN="$CHECK_HOOK_BIN"
  # The wizard needs no search index, and an indexer outlives this instance.
  GWT_DISABLE_BACKGROUND_INDEX=1
)
if [ -n "$CHECK_GH_TOKEN" ]; then
  ENV_ARGS+=(GH_TOKEN="$CHECK_GH_TOKEN" GITHUB_TOKEN="$CHECK_GH_TOKEN")
fi
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
    if kill -0 "$CHECK_PID" 2>/dev/null; then
      kill -KILL "$CHECK_PID" 2>/dev/null || true
    fi
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
  if [ -s "$URL_FILE" ]; then CHECK_URL="$(tr -d '[:space:]' < "$URL_FILE")"; fi
  if [ -n "$CHECK_URL" ] && curl -fsS -I "$CHECK_URL" >/dev/null 2>&1; then break; fi
  CHECK_URL=""
  sleep 0.1
done
if [ -z "$CHECK_URL" ] || grep -qi 'another tray-resident gwt instance is already running' "$LOG_FILE"; then
  tail -n 60 "$LOG_FILE" >&2
  echo 'browser-check startup failed' >&2
  exit 1
fi

# browser-check-hook-audit-begin
jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
  '{schema_version:1,operation:"hook.doctor",params:{purpose:"managed_browser",expected_hook_bin:$expected_hook_bin}}' \
  | env -u GWT_BIN_PATH "$CHECKOUT_GWTD" \
  | assert_hook_preflight
# browser-check-hook-audit-end

# Keep the runner on the real HOME so its pinned browser cache is available.
# Only the owned checkout process receives CHECK_HOME. One worker: both specs
# drive the same wizard on the same backend.
GWT_PLAYWRIGHT_BASE_URL="$CHECK_URL" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" \
  GWT_PLAYWRIGHT_PROJECT_ROOT="$REPO_ROOT" \
  bash "$REPO_ROOT/scripts/run-visual-tests.sh" --workers=1 \
    crates/gwt/playwright/tests/launch-wizard-model-fallback.spec.ts \
    crates/gwt/playwright/tests/launch-wizard-agent-setup-live.spec.ts "$@"
