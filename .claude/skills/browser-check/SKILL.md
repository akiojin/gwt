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
   - Run remaining commands from the repository root or with absolute paths
     under it.
   - Do not switch branches, switch worktrees, or reuse another checkout.

2. Build/resolve the binary:
   - Use only `<repo-root>/target/debug/gwt`.
   - If the user is verifying freshly edited code, run
     `cargo build -p gwt --bin gwt` first even if the binary already exists.
   - If the binary is missing, build it.

3. Prepare an isolated check home:
   - Create `CHECK_HOME="$(mktemp -d -t gwt-fresh-home.XXXXXX)"`.
   - Create `"$CHECK_HOME/.gwt"`.
   - If the real `$HOME/.gwt/runtime` exists, symlink it to
     `"$CHECK_HOME/.gwt/runtime"` so startup does not rebuild the runtime.
   - Symlink only credential/config inputs needed by the checkout, such as
     `.codex`, `.claude`, `.config`, `.ssh`, `.gitconfig`, `.git-credentials`,
     `.npmrc`, and `.bunfig.toml`, when they exist.
   - Seed `"$CHECK_HOME/.gwt/session.json"` with one active project tab for
     the current repository root so the user lands in the actual app instead
     of the Open Project picker.

4. Launch the fresh server:
   - Create temp files:
     - `URL_FILE="$(mktemp -t gwt-fresh-url.XXXXXX)"`
     - `LOG_FILE="$(mktemp -t gwt-fresh-startup.XXXXXX)"`
   - Run:

     ```bash
     HOME="$CHECK_HOME" USERPROFILE="$CHECK_HOME" \
       GIT_TERMINAL_PROMPT=0 \
       GWT_BROWSER_URL_FILE="$URL_FILE" \
       <repo-root>/target/debug/gwt --no-tray --no-open 2>&1 | tee "$LOG_FILE"
     ```

   - Keep this process running until the user says the check is finished.
   - If stdout says another tray-resident gwt instance is already running, the
     launch is invalid because isolation failed. Do not share that URL.

5. Wait for readiness:
   - Read the URL from `URL_FILE`.
   - Fall back to the fresh process's stdout line only if the URL file is
     empty.
   - Verify with `curl -fsS -I <url>`.
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

- Do not claim manual verification passed until the user confirms the UI.
- Do not leave the fresh gwt process running after the user is done.
- If launch fails, report the last relevant startup log lines and fix the
  fresh-launch problem before asking the user to retry.
- If the user sees a failed Agent window from remote branch creation, close
  that fresh-check window before asking the user to retry. Explain it as a
  verification setup problem, not as evidence about the feature under test.
- Keep user-facing status messages concise and in Japanese. Keep commands,
  flags, paths, and code examples as-is.
