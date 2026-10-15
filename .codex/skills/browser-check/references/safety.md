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
- Always launch this checkout's `<repo-root>/target/debug/gwt` with
  `--no-tray --no-open`.
- Record the exact owned PID immediately after launch. Cleanup must run on
  success, startup/audit/test failure, and owner-shell `EXIT/HUP/INT/TERM`.
  Never terminate processes by name, command pattern, or another instance's PID.
- Always isolate `HOME` / `USERPROFILE` so the fresh process owns its own
  `.gwt` state and cannot fall through to the user's tray lock.
- The URL is valid only if it comes from this fresh process's
  `GWT_BROWSER_URL_FILE` or its stdout and `curl -fsS -I <url>` succeeds.
