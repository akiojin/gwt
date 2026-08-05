---
name: browser-check
description: Use when a user needs browser verification of this gwt checkout, a served URL for the code just edited, or visual confirmation that must not reuse a production or already-running gwt instance.
---

# Browser Check

Launch a fresh browser-served gwt process from the current checkout and give
the user only that fresh server URL. This skill is for verifying edited code,
not for reusing production `GWT.app`, an existing tray-resident gwt process, or
an old browser tab.

## Non-negotiable Invariants

- Never share a production, installed, stale, or already-running gwt URL.
- Never use `/Applications/GWT.app`, `command -v gwt`, another worktree's
  binary, `gwt serve`, or `gwt --headless`.
- Never stop, kill, or ask the user to quit their production gwt / `GWT.app`.
- Always launch this checkout's `<repo-root>/target/debug/gwt`.
- Always isolate `HOME` / `USERPROFILE` so the fresh process owns its own
  `.gwt` state and cannot fall through to the user's tray lock.
- The URL is valid only if it comes from this fresh process's
  `GWT_BROWSER_URL_FILE` or its stdout and `curl -fsS -I <url>` succeeds.

## Workflow

1. Capture the launch checkout:
   - Treat the current working directory at skill start as the launch
     checkout.
   - Resolve the repository root and current branch from that same checkout.
   - Record the exact paths used by the launch and hook audit:

     ```bash
     REPO_ROOT="$(git rev-parse --show-toplevel)"
     CHECKOUT_GWT="$REPO_ROOT/target/debug/gwt"
     CHECKOUT_GWTD="$REPO_ROOT/target/debug/gwtd"
     ```

   - Run remaining commands from the repository root or with absolute paths
     under it.
   - Do not switch branches, switch worktrees, or reuse another checkout.

2. Build/resolve the binary:
   - Use only `<repo-root>/target/debug/gwt` for the GUI and
     `<repo-root>/target/debug/gwtd` for the read-only convergence audit.
   - If the user is verifying freshly edited code, run
     `cargo build -p gwt --bin gwt --bin gwtd` first even if the binaries
     already exist.
   - If either binary is missing, build both with the same command.

3. Prepare an isolated check home:
   - Create `CHECK_HOME="$(mktemp -d -t gwt-fresh-home.XXXXXX)"`.
   - Create `"$CHECK_HOME/.gwt"`.
   - If the real `$HOME/.gwt/runtime` exists, symlink it to
     `"$CHECK_HOME/.gwt/runtime"` so startup does not rebuild the runtime.
   - Symlink only credential/config inputs needed by the checkout, such as
     `.codex`, `.claude`, `.config`, `.docker`, `.ssh`, `.gitconfig`,
     `.git-credentials`, `.npmrc`, and `.bunfig.toml`, when they exist.
     `.docker` is required for Docker-runtime launches: on macOS Docker
     Desktop, `docker compose` is a CLI plugin discovered via
     `$HOME/.docker/cli-plugins`, so an isolated HOME without this symlink
     fails the launch preflight with `docker compose is not available`
     (Issue #3029).
   - If the visual check needs GitHub-backed actions, bridge authentication
     into the isolated process without printing secrets:
     - Prefer an existing non-empty `GH_TOKEN`.
     - Else prefer an existing non-empty `GITHUB_TOKEN`.
     - Else, if `gh` is available, run `gh auth token` from the real HOME and
       capture stdout into a shell variable.
     - Never echo, log, write, or Board-post the token. Pass it only as
       `GH_TOKEN` / `GITHUB_TOKEN` in the fresh process environment.
     - If no token can be obtained and the check requires a GitHub mutation,
       stop before sharing a URL and ask the user to run `gh auth login`.
   - Seed `"$CHECK_HOME/.gwt/session.json"` with one active project tab for
     the current repository root so the user lands in the actual app instead
     of the Open Project picker.

4. Launch the fresh server:
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
         && printf '%s\n' "$1" | rg -qi -- "$CHECKOUT_LOCAL_HOOK_PATTERN"
     }

     CHECK_HOOK_BIN="${GWT_HOOK_BIN:-}"
     if is_checkout_local_hook_bin "$CHECK_HOOK_BIN"; then
       CHECK_HOOK_BIN=""
     fi
     if [ -n "$CHECK_HOOK_BIN" ] && [ "$CHECK_HOOK_BIN" != "gwtd" ] \
       && [ ! -x "$CHECK_HOOK_BIN" ]; then
       CHECK_HOOK_BIN=""
     fi
     if [ -z "$CHECK_HOOK_BIN" ] \
       && [ -x /Applications/GWT.app/Contents/MacOS/gwtd ]; then
       CHECK_HOOK_BIN=/Applications/GWT.app/Contents/MacOS/gwtd
     fi
     if [ -z "$CHECK_HOOK_BIN" ]; then
       PATH_HOOK_BIN="$(command -v gwtd 2>/dev/null || true)"
       if ! is_checkout_local_hook_bin "$PATH_HOOK_BIN"; then
         CHECK_HOOK_BIN="$PATH_HOOK_BIN"
       fi
     fi
     if [ -z "$CHECK_HOOK_BIN" ]; then
       CHECK_HOOK_BIN="gwtd"
     fi
     # browser-check-hook-authority-end
     ```

   - Do not set `GWT_BIN_PATH` on the fresh GUI process. It is launch-scoped:
     the checkout GUI's agent launch pipeline injects `CHECKOUT_GWTD` into the
     agent process as `GWT_BIN_PATH`, and generated hooks select that edited
     runtime before the stable `GWT_HOOK_BIN` fallback. Keeping the two values
     separate prevents an older debug GUI from persisting its disposable
     daemon path while still exercising edited hook behavior in launched
     agents.
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
     # browser-check-launch-begin
     env -u GWT_BIN_PATH "${ENV_ARGS[@]}" \
       "$CHECKOUT_GWT" --no-tray --no-open 2>&1 | tee "$LOG_FILE"
     # browser-check-launch-end
     ```

   - Keep this process running until the user says the check is finished.
   - If stdout says another tray-resident gwt instance is already running, the
     launch is invalid because isolation failed. Do not share that URL.

5. Wait for readiness:
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

6. Tell the user:
   - The fresh checkout URL.
   - The startup/stdout log path.
   - The isolated `CHECK_HOME` path.
   - That the startup/stdout log may stay quiet during normal UI actions.
   - Ask them to report what they see, or say when they are done.

7. While the user inspects:
   - Do not poll logs on a timer.
   - Do not inspect production `~/.gwt` logs as routine evidence.
   - If the user reports a problem, inspect this fresh process's stdout log,
     isolated `CHECK_HOME/.gwt` state, and then relevant structured logs under
     the isolated home.

8. Shutdown:
   - When the user says the check is finished, send Ctrl-C to the launched
     process and wait for it to exit.
   - Report the tested URL and whether shutdown was clean.

## Guardrails

- Never write `workspace.update params.purpose` (the Agent/window title) from
  this skill. The title must keep the stable work purpose across this
  verification phase; report this skill's activity (fresh server launch,
  served URL, verification progress) through `params.current_focus` /
  `params.status_text` or Board posts instead. `gwtd` rejects transient
  activity labels such as `browser check` as a purpose (Issue #3184).
- Do not claim manual verification passed until the user confirms the UI.
- Do not leave the fresh gwt process running after the user is done.
- If launch fails, report the last relevant startup log lines and fix the
  fresh-launch problem before asking the user to retry.
- If the user sees a failed Agent window from remote branch creation, close
  that fresh-check window before asking the user to retry. Explain it as a
  verification setup problem, not as evidence about the feature under test.
- Keep user-facing status messages concise and in Japanese. Keep commands,
  flags, paths, and code examples as-is.
