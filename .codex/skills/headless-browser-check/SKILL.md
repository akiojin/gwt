---
name: headless-browser-check
description: Use when a user needs a manual browser check of this gwt project, asks to launch gwt in headless or serve mode, or needs a served browser URL for UI inspection.
---

# Headless Browser Check

Run the gwt GUI as a headless browser-served app, hand the browser URL to the
user, and wait for the user's visual confirmation.

## Scope

Use this project-local skill for manual user verification of the gwt app. Do
not use it for CI-only Playwright runs or generic web app servers.

## Workflow

1. Start from the repository root and note the current branch.
2. Resolve the gwt binary:
   - Prefer `target/debug/gwt` when it exists.
   - Otherwise run `cargo build -p gwt --bin gwt`, then use `target/debug/gwt`.
   - If the user explicitly asks to test freshly edited code, build first even
     when `target/debug/gwt` exists.
3. Start headless mode in a long-running exec session:
   - Create a temp URL handoff file and log path under `${TMPDIR:-/tmp}`.
   - Run `GWT_BROWSER_URL_FILE=<url-file> target/debug/gwt serve 2>&1 | tee <log-file>`.
   - Treat this tee log as the startup/stdout log only; ordinary browser UI
     actions may not appear there.
   - Use default auto-open behavior unless the user asks not to open a browser.
   - Use `--port <port>` or `--bind <addr>` only when the user requests it.
4. Wait for readiness:
   - Read the URL from `GWT_BROWSER_URL_FILE` first.
   - Fall back to the log line `gwt browser URL: <url>`.
   - Verify the URL with `curl -fsS -I <url>` or an equivalent HTTP 200 check.
5. Do not manually open the URL after a normal startup. `gwt serve` opens the
   browser by default, and running an additional platform opener will create a
   duplicate browser tab or window.
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

- Do not leave a headless `gwt serve` process running after the user is done.
- Do not claim the manual check passed until the user confirms the observed UI.
- If startup fails, report the last relevant log lines and fix the launch issue
  before asking the user to retry.
- If a fixed port is busy, prefer retrying with the default random port unless
  the user required that exact port.
- Keep user-facing status messages concise and in Japanese. Keep command names
  and log identifiers as-is.
- **Never stop, kill, or ask the user to quit the user's production gwt /
  `GWT.app` instance** (`/Applications/GWT.app`, any locally installed gwt
  GUI, or any long-running `gwt` process the user did not launch in this
  session). `gwt serve` is launched as a separate process from this skill;
  let the production instance keep running. If a shared-state concern
  arises (`~/.gwt/session.json`, `~/.gwt/app-instance.lock`, port
  conflicts), point it out and let the user decide — never preempt that
  decision by stopping their app.
