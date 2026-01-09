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

## Pending
- [ ] Re-run full unit suite (still stalls after utils/session.test.ts with maxConcurrency=1)
- [ ] Identify open handles or lingering mocks causing utils/session stall in full run
- [ ] Prepare PR summary once tests are green
