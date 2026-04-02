# Tasks: SPEC-1648 — セッション保存・復元

## Phase 1: Scope

- [ ] T001: 永続データと一時データの境界を spec.md に明記する。
- [ ] T002: 保存する session metadata 項目を整理する。

## Phase 2: Restore

- [ ] T003: 起動時 restore / resume 契約を plan.md に書き下ろす。
- [ ] T004: 不正データや消えた worktree の復旧方針を tasks 化する。

## Phase 3: Verification

- [ ] T005: session restore の主要失敗ケースを受け入れ条件へ追加する。
- [x] T006: launch 時に保存する worktree path と session-id detection に使う path を actual launched worktree に揃える。
- [x] T007: branch は一致しても worktree path が一致しない stale resume history を Quick Start 候補から除外する。
