# browser-check: readiness

1. Wait for readiness:
   - Read the URL from `URL_FILE`.
   - Fall back to the fresh process's stdout line only if the URL file is
     empty.
   - Verify with `curl -fsS -I <url>`.
   - Run the Post-launch hook convergence audit before sharing the URL. The
     canonical `hook.health` operation inspects each existing Claude, Codex,
     OpenCode, OpenClaw, and Hermes surface, requires its provider-native
     runtime indirection plus the exact stable fallback, and rejects any
     checkout-local daemon fallback:

     ```bash
     # browser-check-hook-audit-begin
     HOOK_HEALTH_ENVELOPE="$(
       jq -n \
         --arg expected_hook_bin "$CHECK_HOOK_BIN" \
         --arg runtime_state_path "$CHECK_HOME/.gwt/browser-check-missing-runtime-state.json" \
         '{schema_version:1,operation:"hook.health",params:{expected_hook_bin:$expected_hook_bin,runtime_state_path:$runtime_state_path}}' \
       | (cd "$REPO_ROOT" && env -u GWT_BIN_PATH "$CHECKOUT_GWTD")
     )"
     if ! HOOK_HEALTH_JSON="$(
       printf '%s' "$HOOK_HEALTH_ENVELOPE" \
       | jq -er 'select(.ok == true) | .output | fromjson'
     )"; then
       echo "browser-check hook convergence failed: hook.health did not return evidence" >&2
       exit 1
     fi
     ALLOW_MISSING_LOGICAL_FALLBACK=false
     if [ "$CHECK_HOOK_BIN" = "gwtd" ] && ! command -v gwtd >/dev/null 2>&1; then
       ALLOW_MISSING_LOGICAL_FALLBACK=true
     fi
     if ! printf '%s' "$HOOK_HEALTH_JSON" \
       | jq -e --argjson allow_missing_logical "$ALLOW_MISSING_LOGICAL_FALLBACK" '
           [
             .issues[]
             | select(
                 ($allow_missing_logical
                   and startswith("managed hook binary missing: ")
                   and endswith(" uses gwtd"))
                 or (startswith("managed hook failure: ")
                   and test(" state=fail-open( |$)"))
                 | not
               )
           ] as $blocking_issues
           | .status != "inactive" and ($blocking_issues | length == 0)
         ' >/dev/null; then
       echo "browser-check hook convergence failed: managed hook surfaces did not converge" >&2
       printf '%s' "$HOOK_HEALTH_JSON" | jq '{status, issues}' >&2
       exit 1
     fi
     # browser-check-hook-audit-end
     ```

   - If the audit fails, stop the fresh process and diagnose materialization.
     Do not share the URL or replace `CHECK_HOOK_BIN` with `CHECKOUT_GWTD`.
   - Optionally use browser automation once to confirm the page is past
     startup and has a project tab for the seeded checkout.
   - Do not make `Start Work` the user's verification path unless the task is
     specifically about Start Work and you have already proven GitHub branch
     creation works in the isolated home. Fresh checks run with
     `GIT_TERMINAL_PROMPT=0`; otherwise a `git push` can fail with
     `could not read Username for 'https://github.com'`.
   - If the verification needs an agent window but not Start Work itself,
     prepare or launch it on the current checkout/current branch path instead
     of asking the user to create a new work branch from the fresh browser.
