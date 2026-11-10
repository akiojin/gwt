#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
# Install the version in scripts/playwright-version.txt into target/e2e-deps before running.
# Use --serve-only from the owning session for canonical verify.run: detached
# verification workers cannot satisfy hook.doctor process ancestry.
# Additional arguments in normal mode pass to Playwright.
test -f target/e2e-deps/node_modules/@playwright/test/cli.js || { echo "Install pinned Playwright in target/e2e-deps first" >&2; exit 1; }
CHECK_HOME="$(mktemp -d "$ROOT/target/gwt-fresh-home.XXXXXX")"
mkdir -p "$CHECK_HOME/.gwt/projects/99a8660247f5bc49/project-state"
if [ -d "$HOME/.gwt/runtime" ]; then ln -s "$HOME/.gwt/runtime" "$CHECK_HOME/.gwt/runtime"; fi
for input in .config .codex .claude .docker .ssh .gitconfig .git-credentials; do
  if [ -e "$HOME/$input" ]; then ln -s "$HOME/$input" "$CHECK_HOME/$input"; fi
done
printf '%s\n' '{"settings":{"auto_start":false}}' > "$CHECK_HOME/.gwt/projects/99a8660247f5bc49/project-state/pm.json"
printf '%s\n' '{"enabled":false,"max_active_agents":1,"priority_order":[]}' > "$CHECK_HOME/.gwt/projects/99a8660247f5bc49/project-state/issue-monitor.json"
python3 - "$ROOT" "$CHECK_HOME" <<'PY'
import json,sys
root,home=sys.argv[1:]
json.dump({'active_tab_id':'labels-check','tabs':[{'id':'labels-check','title':'Labels check','project_root':root,'kind':'git'}],'recent_projects':[]},open(home+'/.gwt/session.json','w'))
PY
# Resolve the same stable fallback as managed launch on the current host.
# browser-check-hook-authority-begin
is_checkout_hook() {
  [[ "$1" =~ (^|[/\\])target[/\\]([^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?$ ]]
}
HOOK_BIN="${GWT_HOOK_BIN:-}"
if is_checkout_hook "$HOOK_BIN" || { [ -n "$HOOK_BIN" ] && [ "$HOOK_BIN" != gwtd ] && [ ! -x "$HOOK_BIN" ]; }; then
  HOOK_BIN=""
fi
if [ -z "$HOOK_BIN" ]; then
  HOOK_BIN="$(env -u GWT_BIN_PATH -u GWT_HOOK_BIN ./target/debug/gwtd <<'JSON' | python3 -c 'import json,sys; e=json.load(sys.stdin); assert e["ok"],e; print(json.loads(e["output"])["managed_hook_bin"])'
{"schema_version":1,"operation":"hook.doctor","params":{}}
JSON
)"
fi
if [ -z "$HOOK_BIN" ] || is_checkout_hook "$HOOK_BIN"; then
  echo "Cannot resolve a stable managed-hook fallback" >&2
  exit 1
fi
# browser-check-hook-authority-end
check_hooks() {
  ./target/debug/gwtd <<JSON > "$CHECK_HOME/hook-audit.json"
{"schema_version":1,"operation":"hook.doctor","params":{"purpose":"managed_browser","expected_hook_bin":"$HOOK_BIN"}}
JSON
  python3 - "$CHECK_HOME/hook-audit.json" <<'PY'
import json,sys
x=json.load(open(sys.argv[1]));assert x['ok'],x
p=json.loads(x['output'])['preflight']
assert p['result']=='proceed' and p['context']=='current' and p['normal_completion']=='normal' and p['convergence']=='converged' and p['reasons']==[],p
assert all(d['classification'] in ['recovered','fail_open','foreign'] for d in p['diagnostics']),p
PY
}
check_hooks
CHECK_PID=""
cleanup() {
  if [ -n "$CHECK_PID" ]; then
    kill -TERM "$CHECK_PID" 2>/dev/null || true
    for ((n=0;n<100;n++)); do kill -0 "$CHECK_PID" 2>/dev/null || break; sleep 0.1; done
    kill -KILL "$CHECK_PID" 2>/dev/null || true
    wait "$CHECK_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
UNSET_GWT=()
while IFS='=' read -r name value; do
  if [[ "$name" == GWT_* ]]; then UNSET_GWT+=(-u "$name"); fi
done < <(env)
env "${UNSET_GWT[@]}" HOME="$CHECK_HOME" USERPROFILE="$CHECK_HOME" GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1 GWT_BROWSER_URL_FILE="$CHECK_HOME/url" GWT_HOOK_BIN="$HOOK_BIN" GWT_DISABLE_BACKGROUND_INDEX=1 "$ROOT/target/debug/gwt" --no-tray --no-open > "$CHECK_HOME/server.log" 2>&1 &
CHECK_PID=$!
for ((n=0;n<300;n++)); do
  if [ -s "$CHECK_HOME/url" ]; then break; fi
  kill -0 "$CHECK_PID" || { cat "$CHECK_HOME/server.log"; exit 1; }
  sleep 0.2
done
BASE="$(cat "$CHECK_HOME/url")"
curl -fsS -I "$BASE" > "$CHECK_HOME/http-check.txt"
check_hooks
printf 'Isolated checkout PID=%s HOME=%s\n' "$CHECK_PID" "$CHECK_HOME"
printf '%s\n' "$CHECK_HOME" > "$ROOT/target/issue-6211-browser-home"
if [ "${1:-}" = "--serve-only" ]; then
  wait "$CHECK_PID"
  exit $?
fi
GWT_PLAYWRIGHT_BASE_URL="$BASE" GWT_PLAYWRIGHT_CHECK_HOME="$CHECK_HOME" GWT_PLAYWRIGHT_PROJECT_ROOT="$ROOT" NODE_PATH="$ROOT/target/e2e-deps/node_modules" node target/e2e-deps/node_modules/@playwright/test/cli.js test --config crates/gwt/playwright/playwright.config.ts tests/kanban.spec.ts --grep 'Issue #6211|p95 budget' --workers=1 "$@"
