---
name: headless-browser-check
description: Use when a user needs a manual browser check of this gwt project, asks to launch gwt in headless or serve mode, or wants the agent to monitor logs while they inspect the UI.
---

# Headless Browser Check

Run the gwt GUI as a headless browser-served app, hand the browser URL to the
user, and stay on log watch while the user checks the UI.

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
   - That the agent is also watching recent project structured logs under
     `~/.gwt/projects/*/logs/gwt.log.<date>` for backend events, access logs,
     warnings, and errors.
   - Ask them to report what they see, or say when they are done.
7. Monitor until the user is done:
   - Keep the exec session running.
   - Poll output about every 30 seconds.
   - Also tail recently modified structured logs:
     `find ~/.gwt/projects -path "*/logs/gwt.log.$(date -u +%F)" -mmin -30`.
   - Watch for `panic`, `ERROR`, `WARN`, `failed`, `refused`, `500`, WebSocket
     failures, asset load failures, and unexpected process exit.
   - Do not promise that every window open or click will be logged. Normal UI
     actions may show only HTTP/WebSocket/access activity, not semantic window
     names.
   - Summarize only meaningful new log events. If nothing changed, say so
     briefly.
   - If the user reports a UI problem, immediately inspect recent server output
     and the log file before proposing fixes.
8. Shutdown:
   - When the user says the check is finished, send Ctrl-C to the headless
     process and wait for it to exit.
   - Report any observed log issues, the tested URL, and whether shutdown was
     clean.

## Guardrails

- Do not leave a headless `gwt serve` process running after the user is done.
- Do not claim the manual check passed until the user confirms the observed UI.
- If startup fails, report the last relevant log lines and fix the launch issue
  before asking the user to retry.
- If a fixed port is busy, prefer retrying with the default random port unless
  the user required that exact port.
- Keep user-facing status messages concise and in Japanese. Keep command names
  and log identifiers as-is.
