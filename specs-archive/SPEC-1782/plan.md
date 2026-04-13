# Plan: SPEC-1782 — branch-scoped Quick Start in a multi-session shell

## Summary

Keep Quick Start as the branch-scoped fast-launch contract, but integrate it with the rebuilt `no/one/many` branch enter flow so it coexists with active sessions and the session selector.

- Preserve persisted Quick Start entries for `Resume` / `Start new`
- Add a separate `Focus existing session` branch that opens a live-session selector and switches the active session without spawning a new launch
- Restrict the selector to current live sessions for the selected branch/worktree
