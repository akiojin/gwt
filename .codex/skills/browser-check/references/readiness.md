# browser-check: readiness

1. Wait for readiness:
   - Read the URL from `URL_FILE`.
   - Fall back to the fresh process's stdout line only if the URL file is
     empty.
   - Verify with `curl -fsS -I <url>`.
   - Run the Post-launch hook convergence audit before sharing the URL. The
     canonical `hook.doctor` operation with `purpose:"managed_browser"` inspects each existing Claude, Codex,
     OpenCode, OpenClaw, and Hermes surface, requires its provider-native
     runtime indirection plus the exact stable fallback, and rejects any
     checkout-local daemon fallback:

     ```bash
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
     ```

   - This is a point-in-time diagnosis, not an execution grant. Retained
     failures may remain recovered/NeedsAttention while the typed preflight
     proceeds. Missing, blocked, or unknown evidence never authorizes launch;
     native operations still recheck their own authority.
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
