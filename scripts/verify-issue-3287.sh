#!/usr/bin/env bash
# Issue #3287: isolated checkout acceptance. Build gwt/gwtd first.
set -euo pipefail

# Keep hook/caller repository state out of every fixture Git command.
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX \
  GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
  GIT_NAMESPACE GIT_COMMON_DIR

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

# Each theme clones its own local repository through the production Git path.
# Disable resident agents for the canonical source/clone identity before open.
CHECK_CLONE_SOURCES="$CHECK_HOME/clone-sources"
CHECK_CLONE_DESTINATIONS="$CHECK_HOME/clone-destinations"
for CHECK_THEME in dark light; do
  CHECK_CLONE_SOURCE="$CHECK_CLONE_SOURCES/clone-$CHECK_THEME"
  mkdir -p "$CHECK_CLONE_SOURCE" "$CHECK_CLONE_DESTINATIONS/$CHECK_THEME"
  git init --quiet --initial-branch=develop "$CHECK_CLONE_SOURCE"
  git -C "$CHECK_CLONE_SOURCE" -c user.name='GWT acceptance' -c user.email='acceptance@example.invalid' \
    commit --quiet --allow-empty -m 'chore: seed Clone acceptance'
  git -C "$CHECK_CLONE_SOURCE" remote add origin "$CHECK_CLONE_SOURCE"
  CHECK_CLONE_HASH="$(
    cd "$CHECK_CLONE_SOURCE" && printf '%s\n' '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' \
      | "$CHECKOUT_GWTD" | jq -er 'select(.ok == true) | .project_store.hash'
  )"
  CHECK_CLONE_STATE="$CHECK_HOME/.gwt/projects/$CHECK_CLONE_HASH/project-state"
  mkdir -p "$CHECK_CLONE_STATE"
  printf '%s\n' '{"settings":{"auto_start":false}}' > "$CHECK_CLONE_STATE/pm.json"
  printf '%s\n' '{"enabled":false,"max_active_agents":1,"priority_order":[]}' > "$CHECK_CLONE_STATE/issue-monitor.json"
done

# The isolation suite needs an independent Project B, with no resident agents.
if [[ "${GWT_PLAYWRIGHT_LIVE_SPEC:-}" == crates/gwt/playwright/tests/project-per-browser-tab.spec.ts \
   || "${GWT_PLAYWRIGHT_LIVE_SPEC:-}" == crates/gwt/playwright/tests/project-close-detach.spec.ts ]]; then
  CHECK_SECOND_ROOT="$CHECK_HOME/project-b"
  mkdir -p "$CHECK_SECOND_ROOT"
  # A .git marker keeps this fixture on the supported workspace route,
  # without a normal-repository migration or non-repository onboarding.
  git init --quiet --initial-branch=develop \
    --separate-git-dir "$CHECK_HOME/project-b.git" "$CHECK_SECOND_ROOT"
  CHECK_SECOND_HASH="$(
    cd "$CHECK_SECOND_ROOT" && printf '%s\n' '{"schema_version":1,"operation":"issue.monitor.status","params":{}}' \
      | "$CHECKOUT_GWTD" | jq -er 'select(.ok == true) | .project_store.hash'
  )"
  CHECK_SECOND_STATE="$CHECK_HOME/.gwt/projects/$CHECK_SECOND_HASH/project-state"
  mkdir -p "$CHECK_SECOND_STATE"
  printf '%s\n' '{"settings":{"auto_start":false}}' > "$CHECK_SECOND_STATE/pm.json"
  printf '%s\n' '{"enabled":false,"max_active_agents":1,"priority_order":[]}' > "$CHECK_SECOND_STATE/issue-monitor.json"
  jq --arg root "$CHECK_SECOND_ROOT" \
    '.tabs += [{id:"isolation-b",title:"Isolation B",project_root:$root,kind:"git"}]' \
    "$CHECK_HOME/.gwt/session.json" > "$CHECK_HOME/session-with-b.json"
  mv "$CHECK_HOME/session-with-b.json" "$CHECK_HOME/.gwt/session.json"
fi

if [ "${GWT_PLAYWRIGHT_LIVE_SPEC:-}" = crates/gwt/playwright/tests/project-close-detach.spec.ts ]; then
  # A real provider reports SessionStart from its launched process. The shell
  # fixture does the same before becoming interactive; the normal hook validates
  # its inherited Session/capability and publishes readiness to the owner.
  cat > "$CHECK_HOME/pw1-shell-start.cjs" <<'JS'
const { spawnSync } = require('node:child_process');
const { randomUUID } = require('node:crypto');
if (!process.env.GWT_BIN_PATH) throw new Error('Fixture launch has no native hook runtime');
const result = spawnSync(process.env.GWT_BIN_PATH, ['hook', 'event', 'SessionStart'], {
  input: JSON.stringify({ session_id: randomUUID(), hook_event_name: 'SessionStart',
    source: 'startup', cwd: process.cwd() }),
  stdio: ['pipe', 'ignore', 'inherit'], timeout: 60000,
});
if (result.error || result.status !== 0) throw new Error('Fixture SessionStart hook failed');
JS
  cat > "$CHECK_HOME/pw1-shell.sh" <<'SH'
#!/bin/sh
set -eu
node "$HOME/pw1-shell-start.cjs"
exec /bin/sh -i
SH
  cat > "$CHECK_HOME/.gwt/config.toml" <<TOML
[tools.customCodingAgents.pw1-shell]
displayName = "PW1 acceptance shell"
agentType = "command"
command = "/bin/sh"
defaultArgs = ["$CHECK_HOME/pw1-shell.sh"]
TOML
  # The production owner reserves a registered person with a fixed tool.
  # Seed the disposable shell member before the first roster read can create
  # a default Codex member that cannot run this custom Agent.
  jq '.agent_roster = {schema_version:1,revision:1,members:[{
    agent_member_id:"pw1-shell-acceptance",agent_id:"pw1-shell",
    name:"PW1 acceptance member",enabled:true,created_at:(now | todateiso8601),
    last_issue_number:null,last_spec_number:null,assignment:null
  }]}' "$CHECK_PROJECT_STATE/issue-monitor.json" > "$CHECK_HOME/pw1-issue-monitor.json"
  mv "$CHECK_HOME/pw1-issue-monitor.json" "$CHECK_PROJECT_STATE/issue-monitor.json"
  export GWT_PLAYWRIGHT_CLOSE_AGENT_ID=pw1-shell
  export GWT_PLAYWRIGHT_BRANCH_NAME="$(git symbolic-ref --short HEAD)"
fi

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

# These suites audit their own ordinary managed Source before/after UI checks.
# A canonical daemon runner is not a descendant of the caller's managed pane.
# Other suites retain the current-caller preflight below.
CHECK_SUITE_SOURCE=0
case "${GWT_PLAYWRIGHT_LIVE_SPEC:-}" in
  crates/gwt/playwright/tests/project-close-detach.spec.ts|\
  crates/gwt/playwright/tests/project-open-surfaces.spec.ts|\
  crates/gwt/playwright/tests/project-per-browser-tab.spec.ts) CHECK_SUITE_SOURCE=1 ;;
esac

# Audit the managed hook fallback with the real HOME, as required by browser-check.
assert_hook_health() {
  local health_json
  health_json="$(cat)"
  if ! printf '%s' "$health_json" | jq -e '
    .preflight.result == "proceed"
    and .preflight.schema_version == 1
    and .preflight.context == "current"
    and .preflight.normal_completion == "normal"
    and .preflight.convergence == "converged"
    and (.preflight.reasons | type == "array" and length == 0)
    and (.preflight.diagnostics | type == "array")
    and all(.preflight.diagnostics[];
      (.id | type == "string" and length > 0)
      and (.classification as $kind | ["recovered", "fail_open", "foreign"] | index($kind) != null))
  ' >/dev/null; then
    echo 'browser-check hook convergence failed' >&2
    printf '%s' "$health_json" | jq '{preflight, health}' >&2
    return 1
  fi
}
# browser-check-hook-repair-begin
if [ "$CHECK_SUITE_SOURCE" = 0 ]; then
HOOK_DOCTOR_ENVELOPE="$(
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    '{schema_version:1,operation:"hook.doctor",params:{purpose:"managed_browser",repair:true,expected_hook_bin:$expected_hook_bin}}' \
    | env -u GWT_BIN_PATH GWT_HOOK_BIN="$CHECK_HOOK_BIN" "$CHECKOUT_GWTD"
)"
HOOK_DOCTOR_HEALTH_JSON="$(printf '%s' "$HOOK_DOCTOR_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson')"
printf '%s' "$HOOK_DOCTOR_HEALTH_JSON" | assert_hook_health
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
if [ "$CHECK_SUITE_SOURCE" = 0 ]; then
HOOK_HEALTH_ENVELOPE="$(
  jq -n --arg expected_hook_bin "$CHECK_HOOK_BIN" \
    '{schema_version:1,operation:"hook.doctor",params:{purpose:"managed_browser",expected_hook_bin:$expected_hook_bin}}' \
    | env -u GWT_BIN_PATH "$CHECKOUT_GWTD"
)"
HOOK_HEALTH_JSON="$(printf '%s' "$HOOK_HEALTH_ENVELOPE" | jq -er 'select(.ok == true) | .output | fromjson')"
printf '%s' "$HOOK_HEALTH_JSON" | assert_hook_health
fi
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
    crates/gwt/playwright/tests/project-hub.spec.ts
  )
fi
GWT_PLAYWRIGHT_BASE_URL="$CHECK_URL" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" \
  GWT_PLAYWRIGHT_CLONE_SOURCES="$CHECK_CLONE_SOURCES" GWT_PLAYWRIGHT_CLONE_DESTINATIONS="$CHECK_CLONE_DESTINATIONS" \
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
