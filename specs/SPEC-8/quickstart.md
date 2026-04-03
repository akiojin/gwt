# Quickstart: SPEC-8 - Input Extensions

## Reviewer Flow
1. Use the current branch to trigger `Ctrl+G` input extensions from an active terminal session.
2. Verify file-paste behavior against clipboard content and active PTY injection.
3. Run the AI branch-name suggestion flow and confirm error fallback remains usable.
4. Treat voice backend completion and manual reviewer passes as remaining work until explicitly verified.

## Expected Result
- The reviewer sees the current implemented scope for input extensions.
- Any missing behavior is logged against the remaining `27` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
