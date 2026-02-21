# タスク分割: GitHub リモート起点の Worktree 作成

**仕様ID**: `SPEC-a4fb2db2`

## タスク一覧

### T001: TDD テスト作成

- **ファイル**: `crates/gwt-core/src/git/gh_cli.rs`
- **内容**: `#[cfg(test)]` モジュールに新規テスト追加
  - `create_remote_branch_returns_result`: シグネチャテスト
  - `resolve_remote_branch_sha_returns_result`: シグネチャテスト
  - `create_remote_branch_endpoint_format`: エンドポイント文字列構築テスト
  - `resolve_remote_branch_sha_endpoint_format`: エンドポイント文字列構築テスト
  - `resolve_remote_branch_sha_parses_json`: SHA 抽出テスト
- **依存**: なし

### T002: `create_remote_branch()` 実装

- **ファイル**: `crates/gwt-core/src/git/gh_cli.rs`
- **内容**: GitHub API 経由でリモートブランチを作成する関数
- **依存**: T001

### T003: `resolve_remote_branch_sha()` 実装

- **ファイル**: `crates/gwt-core/src/git/gh_cli.rs`
- **内容**: GitHub API 経由でブランチ SHA を取得する関数
- **依存**: T001

### T004: re-export 追加

- **ファイル**: `crates/gwt-core/src/git.rs`
- **内容**: 新関数の pub use 追加
- **依存**: T002, T003

### T005: `create_new_worktree_path()` リモート起点フロー

- **ファイル**: `crates/gwt-tauri/src/commands/terminal.rs`
- **内容**: `create_new_worktree_remote_first()` ヘルパー追加、`create_new_worktree_path()` 変更
- **依存**: T004

### T006: テスト GREEN 確認 + Lint

- **内容**: `cargo test`, `cargo clippy`, `cargo fmt` 全パス確認
- **依存**: T005
