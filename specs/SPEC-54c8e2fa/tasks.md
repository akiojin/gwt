# タスク分割: Issue連携ブランチのリンク保証と起動フロー一元化

**仕様ID**: `SPEC-54c8e2fa`

## タスク一覧

### T001: TDD失敗テスト追加（gwt-core）

- **ファイル**: `crates/gwt-core/src/git/issue.rs`
- **内容**:
  - `issue_develop_args` が `--base` を条件付き付与するテスト
  - `gh issue develop --list` 出力のブランチ判定テスト
  - 未リンク時に `[E1012]` エラーとなる分岐テスト（判定関数単体）

### T002: IssueリンクAPI実装（gwt-core）

- **ファイル**: `crates/gwt-core/src/git/issue.rs`, `crates/gwt-core/src/git.rs`
- **内容**:
  - `IssueLinkedBranchStatus` と `create_or_verify_linked_branch` 実装
  - `create_linked_branch` 互換ラッパ化
  - re-export 追加

### T003: TDD失敗テスト追加（gwt-tauri backend）

- **ファイル**: `crates/gwt-tauri/src/commands/terminal.rs`
- **内容**:
  - `LaunchAgentRequest` に `issue_number` が入るケースのシリアライズ/初期化テスト
  - issue起点分岐で create path が切り替わることの単体テスト（可能な範囲でヘルパ化）

### T004: backend launch フロー実装

- **ファイル**: `crates/gwt-tauri/src/commands/terminal.rs`
- **内容**:
  - issue起点専用の create/link/resolve/rollback フロー追加
  - 通常launchフロー維持

### T005: frontend 後追い連携撤去

- **ファイル**: `gwt-gui/src/App.svelte`
- **内容**:
  - followup queue/state削除
  - `link_branch_to_issue` / `rollback_issue_branch` の launch完了後呼び出し削除

### T006: テスト更新（gwt-gui）

- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.test.ts` ほか必要箇所
- **内容**:
  - 既存の "後追い link を呼ばない" 前提テストが引き続き成立することを確認
  - 必要に応じて `start_launch_job` リクエスト中の `issueNumber` アサーションを追加

### T007: 全体検証

- **内容**:
  - `cargo test -p gwt-core`
  - `cargo test -p gwt-tauri`
  - `cd gwt-gui && pnpm test`（関連テスト）
  - 必要最小限のフォーマット/lint確認

### T008: TDD失敗テスト追加（Issue #1278 / branch一覧自己修復）

- **ファイル**: `crates/gwt-core/src/git/branch.rs`
- **内容**:
  - `.git/config` に `branch..gh-merge-base` を注入した状態で `Branch::list` が復旧することを検証するテスト追加
  - `.git/config` に `[branch ""]` セクションを注入した状態で `Branch::list_remote` が復旧することを検証するテスト追加
  - 追加直後は RED を確認する

### T009: TDD失敗テスト追加（空ブランチ名拒否）

- **ファイル**: `crates/gwt-core/src/git/issue.rs`
- **内容**:
  - `create_or_verify_linked_branch(..., \"   \", ...)` が入力検証エラーを返すテスト追加
  - 追加直後は RED を確認する

### T010: branch一覧自己修復実装

- **ファイル**: `crates/gwt-core/src/git/branch.rs`
- **内容**:
  - `for-each-ref` 失敗時の `gh-merge-base` 判定と repo config 修復処理を実装
  - 修復対象を `branch..gh-merge-base` と `[branch \"\"]` のみに限定
  - 修復後1回のみ再試行する

### T011: 空ブランチ名入力バリデーション実装

- **ファイル**: `crates/gwt-core/src/git/issue.rs`
- **内容**:
  - `create_or_verify_linked_branch` で `branch_name` の空白入力を即時エラー化
  - `gh` 実行前に終了する

### T012: GREEN確認（対象テスト）

- **内容**:
  - `cargo test -p gwt-core git::branch`
  - `cargo test -p gwt-core git::issue`
