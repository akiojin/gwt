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

## Issue #546 対応
- [x] 仕様追記: コーディングエージェント起動異常/ OpenCodeモデル空/バージョン選択/安全判定中選択/ログビューア最新ログ
- [x] 計画更新: SPEC-3b0ed29b plan.md 追加、SPEC-d2f4762a と SPEC-c1d5bad7 plan.md 追記
- [x] タスク更新: 各SPECの tasks.md にTDDタスクを追加
- [ ] Clarify不要の前提で進行可否を確認（必要なら追加質問）
- [ ] TDD: 安全判定中の警告表示とバージョン選択の自動遷移防止テスト追加
- [ ] TDD: 起動直後の異常終了検知/ログ記録のテスト追加
- [ ] TDD: OpenCodeモデル選択の空回避テスト追加
- [ ] TDD: ログビューアの最新ログフォールバック/mtime判定テスト追加
- [ ] 実装: ブランチ選択の安全判定中警告とウィザード遷移抑止
- [ ] 実装: 起動異常終了の検知/ログ記録
- [ ] 実装: OpenCodeモデル選択のデフォルト/任意入力
- [ ] 実装: ログビューアの最新ログフォールバック/mtime判定
