#!/bin/bash
# Issue #3820 P20: launch the checkout gwt with a fresh HOME, open N fixture repos
# (3 linked worktrees each) and hold the session; usage: p20-run.sh <out-dir> <hold-seconds> <N>
set -e
REPO_ROOT="$(git rev-parse --show-toplevel)"
CHECKOUT_GWT="$REPO_ROOT/target/debug/gwt"
OUT="$1"; DURATION="${2:-360}"
mkdir -p "$OUT"
CHECK_HOME="$(mktemp -d -t gwt-fresh-home.XXXXXX)"
mkdir -p "$CHECK_HOME/.gwt"
[ -e "$HOME/.gwt/runtime" ] && ln -s "$HOME/.gwt/runtime" "$CHECK_HOME/.gwt/runtime"
for f in .gitconfig .config; do [ -e "$HOME/$f" ] && ln -s "$HOME/$f" "$CHECK_HOME/$f"; done
FIX="$CHECK_HOME/fixture"; mkdir -p "$FIX"
N="${3:-1}"
for r in $(seq $N); do
  git -C "$FIX" init -q -b main repo-$r && (cd "$FIX/repo-$r" && echo a > a.txt && git add . && git -c user.name=t -c user.email=t@e commit -qm init)
  for i in 1 2 3; do git -C "$FIX/repo-$r" worktree add -q -b "work/f$i" "$FIX/repo-$r-wt-$i"; done
done
cd "$REPO_ROOT"
URL_FILE="$(mktemp -t gwt-fresh-url.XXXXXX)"; LOG_FILE="$OUT/startup.log"
CHECK_PID=""
cleanup() { if [ -n "$CHECK_PID" ] && kill -0 "$CHECK_PID" 2>/dev/null; then kill -TERM "$CHECK_PID"; for i in $(seq 100); do kill -0 "$CHECK_PID" 2>/dev/null || break; sleep 0.1; done; kill -KILL "$CHECK_PID" 2>/dev/null || true; fi; wait "$CHECK_PID" 2>/dev/null || true; [ -n "$KEEPER_PID" ] && kill "$KEEPER_PID" 2>/dev/null || true; }
trap 'S=$?; trap - EXIT; cleanup; exit $S' EXIT
env -u GWT_BIN_PATH HOME="$CHECK_HOME" USERPROFILE="$CHECK_HOME" GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1 GWT_BROWSER_URL_FILE="$URL_FILE" \
  "$CHECKOUT_GWT" --no-tray --no-open > "$LOG_FILE" 2>&1 &
CHECK_PID=$!
for i in $(seq 600); do [ -s "$URL_FILE" ] && break; sleep 0.1; done
URL="$(head -n1 "$URL_FILE")"; curl -fsS -I "$URL" >/dev/null
echo "url=$URL home=$CHECK_HOME pid=$CHECK_PID"
node "$(dirname "$0")/drive.mjs" "$URL" "$FIX" "$DURATION" "$N" > "$OUT/driver.log" 2>&1 &
KEEPER_PID=$!
wait "$KEEPER_PID" || true
KEEPER_PID=""
cat "$CHECK_HOME"/.gwt/logs/perf/perf-*.jsonl 2>/dev/null | grep -E 'work_events.ingest|merge_status|WorkMergeStatus' > "$OUT/samples.jsonl" || true
echo "home=$CHECK_HOME" > "$OUT/env.txt"
