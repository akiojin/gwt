# browser-check: handoff cleanup

1. Tell the user:
   - The fresh checkout URL.
   - The startup/stdout log path.
   - The isolated `CHECK_HOME` path.
   - That the startup/stdout log may stay quiet during normal UI actions.
   - Ask them to report what they see, or say when they are done.

2. While the user inspects:
   - Do not poll logs on a timer.
   - Do not inspect production `~/.gwt` logs as routine evidence.
   - If the user reports a problem, inspect this fresh process's stdout log,
     isolated `CHECK_HOME/.gwt` state, and then relevant structured logs under
     the isolated home.

3. Shutdown:
   - When the user says the check is finished, send Ctrl-C to the owner-shell
     tool session. Its trap terminates only `CHECK_PID`, escalates if needed,
     waits for exit, and checks that the PID is gone. Wait for the owner shell
     to exit before reporting completion.
   - Never assume a shell trap runs after `SIGKILL` or a crash. Keep the
     `--no-tray` child attached to its owner parent; parent-death behavior
     must be covered by the real-process verification harness.
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
