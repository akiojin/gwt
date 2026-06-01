---
name: headless-browser-check
description: Use when a user needs a manual browser check of this gwt project, asks to launch gwt for UI inspection, or needs a served browser URL.
---

# Headless Browser Check

Run the gwt GUI as a headless browser-served app, hand the browser URL to the
user, and wait for the user's visual confirmation.

## Scope

Use this project-local skill for manual user verification of the gwt app. Do
not use it for CI-only Playwright runs or generic web app servers.

## Workflow

1. Capture the launch checkout root:
   - Treat the current working directory at skill start as the launch directory.
   - Resolve the repository root for that same checkout and note the current
     branch.
   - Run the remaining workflow from that repository root, or use absolute
     paths prefixed by that repository root.
   - Do not switch to another worktree, reuse another checkout, or use a
     production install for this check.
2. Resolve the gwt binary:
   - Use only this launch checkout's `<repo-root>/target/debug/gwt`.
   - Otherwise run `cargo build -p gwt --bin gwt` from `<repo-root>`, then use
     `<repo-root>/target/debug/gwt`.
   - If the user explicitly asks to test freshly edited code, build first even
     when `<repo-root>/target/debug/gwt` exists.
   - Never fall back to `/Applications/GWT.app`, `command -v gwt`, another
     worktree's binary, or any installed production gwt binary.
3. Start the current gwt browser server route in a long-running exec session:
   - Create a temp URL handoff file and log path under `${TMPDIR:-/tmp}`.
   - Run `GWT_BROWSER_URL_FILE=<url-file> <repo-root>/target/debug/gwt --no-open 2>&1 | tee <log-file>`.
   - Always start a fresh `gwt` process for this check, using the binary
     resolved above. The removed `gwt serve` / `gwt --headless` verbs must not
     be used.
   - If the process reports that another tray-resident gwt instance is already
     running for this user, treat that as a blocker for a fresh checkout check.
     Do not reuse the existing instance's URL as evidence for the current
     checkout.
   - Treat this tee log as the startup/stdout log only; ordinary browser UI
     actions may not appear there.
   - Use default auto-open behavior unless the user asks not to open a browser.
   - Use `--port <port>` or `--bind <addr>` only when the user requests it.
4. Wait for readiness:
   - Read the URL from `GWT_BROWSER_URL_FILE` first.
   - Fall back to the log line `gwt browser URL: <url>`.
   - Accept only the URL produced by the fresh process launched in step 3.
   - Verify the URL with `curl -fsS -I <url>` or an equivalent HTTP 200 check.
5. Do not manually open the URL after a normal startup unless the user asks
   you to. Running an additional platform opener can create a duplicate
   browser tab or window.
   - Only run a platform opener if the user says no browser opened, or if the
     log clearly says auto-open failed.
   - macOS fallback: `open <url>`
   - Linux fallback: `xdg-open <url>`
   - Windows fallback: `cmd /c start "" <url>`
6. Tell the user:
   - The URL.
   - The log file path.
   - That the startup/stdout log may stay quiet during normal UI actions.
   - Ask them to report what they see, or say when they are done.
7. Wait until the user is done:
   - Keep the exec session running.
   - Do not poll or tail logs on a timer while the user inspects the UI.
   - Do not run periodic structured-log checks under
     `~/.gwt/projects/*/logs/gwt.log.<date>`.
   - If the user reports a UI problem, inspect recent server output, the
     startup/stdout log file, and then relevant structured logs before
     proposing fixes.
8. Shutdown:
   - When the user says the check is finished, send Ctrl-C to the headless
     process and wait for it to exit.
   - Report the tested URL and whether shutdown was clean.

## Guardrails

- Do not leave the launched `gwt` process running after the user is done.
- Do not claim the manual check passed until the user confirms the observed UI.
- If startup fails, report the last relevant log lines and fix the launch issue
  before asking the user to retry.
- If a fixed port is busy, prefer retrying with the default random port unless
  the user required that exact port.
- Keep user-facing status messages concise and in Japanese. Keep command names
  and log identifiers as-is.
- **Never reuse an already-running gwt address** from an existing browser tab,
  previous log, GUI status bar, `~/.gwt/session.json`, structured logs, or a
  reachable old server. A successful HTTP check only proves the server is
  reachable; it does not prove the URL belongs to the checkout under test.
- **Never stop, kill, or ask the user to quit the user's production gwt /
  `GWT.app` instance** (`/Applications/GWT.app`, any locally installed gwt
  GUI, or any long-running `gwt` process the user did not launch in this
  session). The current checkout's `gwt` process is launched separately by
  this skill;
  let the production instance keep running. If a shared-state concern
  arises (`~/.gwt/session.json`, `~/.gwt/app-instance.lock`, port
  conflicts), point it out and let the user decide — never preempt that
  decision by stopping their app.
