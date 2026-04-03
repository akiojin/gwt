# Quickstart: SPEC-6 - Notification and Error Bus

## Reviewer Flow
1. Trigger representative debug, info, warning, and error events in the current branch.
2. Verify what appears in the status bar, what persists, and what enters the error queue.
3. Inspect the logs view to confirm structured records match the emitted notifications.
4. Record any missing severity routing as remaining SPEC-6 execution work.

## Expected Result
- The reviewer sees the current implemented scope for notification and error bus.
- Any missing behavior is logged against the remaining `30` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
