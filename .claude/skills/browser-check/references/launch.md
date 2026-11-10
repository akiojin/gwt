# browser-check: launch

1. Launch the fresh server:
   - Create temp files:
     - `URL_FILE="$(mktemp -t gwt-fresh-url.XXXXXX)"`
     - `LOG_FILE="$(mktemp -t gwt-fresh-startup.XXXXXX)"`
   - Resolve a stable/public-or-logical hook-generation fallback. Never use a
     checkout-local `target/*/gwtd` as `GWT_HOOK_BIN`, even when it is inherited
     or first on `PATH`:

     ```bash
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
     ```

   - Keep the current managed caller environment for doctor: its Session and
     canonical runtime identify the source channel. `CHECK_HOME` belongs to
     the isolated GUI target; do not pass a fabricated target runtime path or
     copy source authority into it.
   - Before launch, repair existing managed hook surfaces to the selected
     fallback with the checkout `gwtd`. Automatic selection uses doctor’s
     `managed_hook_bin`, the same resolver used by ordinary launch and
     self-heal. This preserves installed fallbacks when PATH is absent and
     avoids invalidating shared-root trust with a separate bare-name default.
     A valid explicit pin remains intentional; use it consistently across
     writers of that repository. User hooks remain unchanged. The block parses
     `hook.doctor` evidence with `jq`, which
     is not a gwt runtime requirement, so it preflights `jq` explicitly and
     fails closed with install guidance instead of dying on
     `jq: command not found`. Only POSIX utilities (`grep`, `env`) are
     assumed beyond that:

     ```bash
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
     ```

   - Do not set `GWT_BIN_PATH` on the fresh GUI process. It is launch-scoped:
     the checkout GUI's agent launch pipeline injects `CHECKOUT_GWTD` into the
     agent process as `GWT_BIN_PATH`, and generated hooks select that edited
     runtime before the stable `GWT_HOOK_BIN` fallback. Keeping the two values
     separate prevents an older debug GUI from persisting its disposable
     daemon path while still exercising edited hook behavior in launched
     agents.
   - Use one persistent Bash owner shell with `set -e` for launch, readiness,
     hook audit, and automated verification. Keep that shell alive through
     shutdown; an `EXIT` trap in a short-lived launch shell would immediately
     stop the fresh server. Do not use `nohup`, `disown`, or a pipeline to
     launch gwt: `$!` must identify gwt itself, not `tee` or another wrapper.
   - Run:

     ```bash
     CHECK_GH_TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
     if [ -z "$CHECK_GH_TOKEN" ] && command -v gh >/dev/null 2>&1; then
       CHECK_GH_TOKEN="$(gh auth token 2>/dev/null || true)"
     fi
     ENV_ARGS=(
       HOME="$CHECK_HOME"
       USERPROFILE="$CHECK_HOME"
       GIT_TERMINAL_PROMPT=0
       GH_PROMPT_DISABLED=1
       GWT_BROWSER_URL_FILE="$URL_FILE"
       GWT_HOOK_BIN="$CHECK_HOOK_BIN"
     )
     if [ -n "$CHECK_GH_TOKEN" ]; then
       ENV_ARGS+=(GH_TOKEN="$CHECK_GH_TOKEN" GITHUB_TOKEN="$CHECK_GH_TOKEN")
     fi
     # browser-check-process-cleanup-begin
     set -e
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
     env -u GWT_BIN_PATH "${ENV_ARGS[@]}" \
       "$CHECKOUT_GWT" --no-tray --no-open > "$LOG_FILE" 2>&1 &
     CHECK_PID=$!
     printf 'Fresh gwt PID: %s\n' "$CHECK_PID"
     # browser-check-launch-end
     ```

   - Continue the [readiness checks](readiness.md) and any automated tests in this owner shell. A failure
     must exit it so the trap terminates and reaps the owned child. Test
     frameworks must put cleanup in `finally` so evidence failures cannot
     bypass shutdown.
   - For manual inspection, finish the owner-shell script with
     `wait "$CHECK_PID"` after readiness and audit succeed. Keep its tool
     session running until the user says the check is finished.
   - If stdout says another tray-resident gwt instance is already running, the
     launch is invalid because isolation failed. Do not share that URL.
