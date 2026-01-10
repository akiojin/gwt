# PLANS
## Spec update (SPEC-f47db390)
- [x] Update spec.md with branch-aware Quick Start and rescan requirements
- [x] Update plan.md and tasks.md to reflect new scope and tests

## Quick Start branch-aware resume
- [x] Add worktreePath to SelectedBranchState and filter history by branch+worktree
- [x] Render per-tool Quick Start rows; show Resume only when sessionId exists
- [x] Rescan tool session files on Quick Start display using selected branch/worktree

## Branch-resolved session detection (Claude)
- [x] Add branch/worktree filtering to Claude session detection
- [x] Pass branch info through Claude launch and post-run fallbacks

## Tests
- [x] Update Quick Start UI tests for multi-tool rows and resume visibility
- [x] Add branch-filter tests for Claude/Gemini/OpenCode session parsing
- [x] Add unit tests for Quick Start rescan helper

## Test isolation & Bun preload stabilization
- [x] Add Solid-aware test preload to avoid React transform collisions
- [x] Cap Bun test concurrency to reduce mock bleed between files
- [x] Replace node:fs/promises module mocks with spies in config/worktree tests
- [x] Align CLI launch output assertions with terminal stream writes
- [x] Adjust dependency-installer imports to allow access spying
- [x] Align worktree fs/promises imports with test spies
- [x] Close worktree spinner test stderr stream to avoid dangling handles

## Quality checks
- [x] Run build:opentui
- [x] Run dist bundle integrity test

## Mock isolation fix
- [x] Replace mock.restore() with mockReset() in test files to preserve module mocks
- [x] Fix gemini.test.ts to reset resetTerminalModes mock in beforeEach
- [x] Fix consoleLogSpy restoration in gemini.test.ts

## Remaining issues (for follow-up)
- [ ] Full test suite may still experience timing issues in CI - investigate if needed

## Codex skills flag compatibility (v0.80.0+)
- [x] Update SPEC-3b0ed29b spec/plan/tasks for skills flag gating
- [x] Add unit tests for Codex skills flag gating (codex + resolver)
- [x] Implement version-aware skills flag handling for Codex CLI launches
- [x] Run targeted unit tests for Codex launch/resolver changes
