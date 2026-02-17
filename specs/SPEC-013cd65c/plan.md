# 実装計画: SPEC-013cd65c

## 概要

ベアリポジトリでリモートブランチからワークツリー作成時の E1003 エラーを修正する。

## 変更箇所

### Change A: `Branch::remote_exists` の `ls-remote` を `run_git_with_timeout` に統一

- **ファイル**: `crates/gwt-core/src/git/branch.rs` (L750-758)
- **内容**: 素の `Command` を `run_git_with_timeout` に置き換え
- **リスク**: 低（独立した変更）

### Change B: `create_for_branch` — `fetch_all` 後にローカルブランチをフォールバック確認

- **ファイル**: `crates/gwt-core/src/worktree/manager.rs` (L372-377)
- **内容**: `resolve_remote_branch` が None を返した場合、`Branch::exists` もチェック
- **リスク**: 中（コアロジックの変更）

### Change C: `create_for_branch` — リモート解決前にローカルブランチを先行チェック

- **ファイル**: `crates/gwt-core/src/worktree/manager.rs` (L354-367, L369+)
- **内容**: `ls-remote` の前に `Branch::exists` をチェック
- **リスク**: 中（フロー変更）

### Change D: `create_new_branch` — ベースブランチの `remote_exists` 失敗時に `fetch_all` フォールバック追加

- **ファイル**: `crates/gwt-core/src/worktree/manager.rs` (L596-631)
- **内容**: `remote_exists` 失敗時に `fetch_all` + ローカル存在チェック
- **リスク**: 中

## 実装順序

1. テスト作成（TDD）
2. Change A（独立、低リスク）
3. Change B + C（コアフィックス、同時実装）
4. Change D
5. 全テスト実行・lint確認
