# タスクリスト: Worktree 作成時に upstream tracking を自動設定

## Phase 1: テスト作成（TDD）

- [ ] T001 [US1] [テスト] branch.rs に set_upstream_config のテスト追加 `crates/gwt-core/src/git/branch.rs`
- [ ] T002 [US2] [テスト] remote.rs に default_name のテスト追加 `crates/gwt-core/src/git/remote.rs`
- [ ] T003 [US1] [テスト] manager.rs の既存テストに upstream 検証追加 `crates/gwt-core/src/worktree/manager.rs`

## Phase 2: 基盤実装

- [ ] T004 [US1] [実装] Branch::set_upstream_config() 追加 `crates/gwt-core/src/git/branch.rs`
- [ ] T005 [US2] [実装] Remote::default_name() 追加 `crates/gwt-core/src/git/remote.rs`

## Phase 3: 統合実装

- [ ] T006 [US1] [実装] create_new_branch() に upstream 設定追加 `crates/gwt-core/src/worktree/manager.rs`
- [ ] T007 [US1] [実装] create_for_branch() に upstream 設定追加 `crates/gwt-core/src/worktree/manager.rs`
- [ ] T008 [US3] [実装] create_new_worktree_remote_first() に upstream 設定追加 `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 4: 検証

- [ ] T009 [P] [共通] [検証] cargo test / clippy / fmt 通過確認
