# 実装計画: GitHub リモート起点の Worktree 作成

**仕様ID**: `SPEC-a4fb2db2`

## 概要

新規ブランチの Worktree 作成時に GitHub API 経由でリモートブランチを先に作成し、fetch してから Worktree を構成するフローを追加する。

## 実装フェーズ

### Phase 1: gwt-core — GitHub API 関数追加

**ファイル**: `crates/gwt-core/src/git/gh_cli.rs`

1. `resolve_remote_branch_sha(repo_path, branch)` を追加
   - `gh api repos/{owner}/{repo}/git/ref/heads/{branch}` で SHA 取得
   - レスポンス JSON から `object.sha` を抽出
2. `create_remote_branch(repo_path, branch, sha)` を追加
   - `gh api --method POST repos/{owner}/{repo}/git/refs -f ref=refs/heads/{branch} -f sha={sha}`
   - エラー分類: 422（既存）、403（権限）、404（リポジトリ未発見）

**パターン**: 既存の `delete_remote_branch()` と同一パターン（`resolve_owner_repo` → `gh api` → `wait_with_timeout`）

### Phase 2: gwt-core — re-export

**ファイル**: `crates/gwt-core/src/git.rs`

- `pub use gh_cli::create_remote_branch;` 追加
- `pub use gh_cli::resolve_remote_branch_sha;` 追加

### Phase 3: gwt-tauri — リモート起点フロー

**ファイル**: `crates/gwt-tauri/src/commands/terminal.rs`

1. `create_new_worktree_remote_first()` ヘルパー関数を追加
   - ベースブランチの SHA をリモートから取得
   - GitHub にブランチ作成
   - `git fetch origin branch:branch` でローカルに取得
   - `WorktreeManager::create_for_branch()` で Worktree 作成
2. `create_new_worktree_path()` を変更
   - `gh` 利用可能 → `create_new_worktree_remote_first()` 試行
   - 失敗時（422 以外）→ `tracing::warn` + 従来の `create_new_branch()` にフォールバック

## 依存関係

- Phase 2 は Phase 1 に依存
- Phase 3 は Phase 1, 2 に依存

## リスク

- GitHub API レート制限: 通常利用では問題なし（1 Worktree あたり最大 2 API コール）
- `fetch` 失敗時のリモート残留ブランチ: エラーメッセージで通知、手動クリーンアップを案内
