# 実装計画: Worktree 作成時に upstream tracking を自動設定

**仕様ID**: `SPEC-b3f1a4e2` | **日付**: 2026-02-22 | **仕様書**: `specs/SPEC-b3f1a4e2/spec.md`

## 目的

- worktree 作成時に `branch.<name>.remote` と `branch.<name>.merge` を自動設定し、初回 push 時の upstream 未設定エラーを解消する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + gwt-core（`crates/gwt-core/`）
- **テスト**: cargo test
- **前提**: `git config` コマンドが利用可能、ネットワーク不要

## 実装方針

### Phase 1: 基盤メソッド追加

- `Branch::set_upstream_config()` を `crates/gwt-core/src/git/branch.rs` に追加
- `Remote::default_name()` を `crates/gwt-core/src/git/remote.rs` に追加

### Phase 2: worktree 作成フローへの統合

- `WorktreeManager::create_new_branch()` のサブモジュール初期化後に upstream 設定を追加
- `WorktreeManager::create_for_branch()` のサブモジュール初期化後に upstream 設定を追加
- `create_new_worktree_remote_first()` の worktree 作成後に upstream 設定を追加

## テスト

### バックエンド

- `test_set_upstream_config_sets_remote_and_merge` — config 設定の検証
- `test_set_upstream_config_no_remote_ref_needed` — リモートブランチ未存在でも成功
- `test_default_name_prefers_origin` — "origin" 優先の検証
- `test_default_name_falls_back_to_first` — フォールバックの検証
- `test_default_name_returns_none_when_no_remotes` — リモートなしの検証
- 既存テストの upstream 検証追加
