# タスク一覧: SPEC-c6ba640a GitHub Issue連携（GUI版）

**仕様書**: `specs/SPEC-c6ba640a/spec.md`
**計画書**: `specs/SPEC-c6ba640a/plan.md`

## Phase 1: gwt-core API拡張

### T-101: fetch_open_issues にページネーション引数を追加

- **FR**: FR-001, FR-001a, FR-001b
- **ファイル**: `crates/gwt-core/src/git/issue.rs`
- **内容**:
  - `fetch_open_issues(repo_path, page, per_page)` にシグネチャ変更
  - `issue_list_args` に `--limit` と `--jq` でのオフセット制御を追加
  - 戻り値に `has_next_page: bool` を含む構造体を返す
- **TDD**: テストを先に書く
  - `test_issue_list_args_with_pagination` — page=2, per_page=30 のargs確認
  - `test_fetch_result_has_next_page` — 50件取得時の has_next_page 判定
- **依存**: なし
- **ステータス**: 未着手

### T-102: GitHubIssue に labels フィールドを追加

- **FR**: FR-002
- **ファイル**: `crates/gwt-core/src/git/issue.rs`
- **内容**:
  - `GitHubIssue` 構造体に `labels: Vec<String>` 追加
  - `issue_list_args` の `--json` フィールドに `labels` 追加
  - `parse_gh_issues_json` でラベル名をパース
- **TDD**: テストを先に書く
  - `test_parse_gh_issues_json_with_labels` — ラベル付きJSONのパース
  - `test_parse_gh_issues_json_without_labels` — ラベルなしの後方互換
- **依存**: なし
- **ステータス**: 未着手

### T-103: is_gh_cli_authenticated 関数を追加

- **FR**: FR-003, FR-003a
- **ファイル**: `crates/gwt-core/src/git/issue.rs`
- **内容**:
  - `is_gh_cli_authenticated() -> bool` 関数を追加
  - `gh auth status` コマンドの終了コードで判定
- **TDD**: テストを先に書く
  - `test_gh_auth_status_args` — コマンド引数の確認（ユニットテスト可能な部分）
- **依存**: なし
- **ステータス**: 未着手

## Phase 2: Tauriコマンド公開

### T-201: issue.rs Tauriコマンドモジュール作成

- **FR**: FR-010〜FR-014
- **ファイル**: `crates/gwt-tauri/src/commands/issue.rs`（新規）, `crates/gwt-tauri/src/commands/mod.rs`, `crates/gwt-tauri/src/lib.rs`
- **内容**:
  - `fetch_github_issues(project_path, page, per_page)` コマンド
  - `check_gh_cli_status(project_path)` コマンド
  - `find_existing_issue_branch(project_path, issue_number)` コマンド
  - `link_branch_to_issue(project_path, issue_number, branch_name)` コマンド
  - `rollback_issue_branch(project_path, branch_name, delete_remote)` コマンド
  - モジュール登録・コマンド登録
- **TDD**: テストを先に書く
  - Tauri コマンドの引数・戻り値の型テスト
  - ロールバック関数のステップ順序テスト
- **依存**: T-101, T-102, T-103
- **ステータス**: 未着手

## Phase 3: TypeScript型定義

### T-301: フロントエンド型定義の追加

- **FR**: FR-010a〜FR-014a
- **ファイル**: `gwt-gui/src/lib/types.ts`
- **内容**:
  - `GitHubIssue` interface（number, title, updated_at, labels）
  - `GhCliStatus` interface（available, authenticated）
  - `FetchIssuesResponse` interface（issues, has_next_page）
- **依存**: T-201
- **ステータス**: 未着手

## Phase 4: AgentLaunchForm セクション並び替え

### T-401: フォームセクションの順序変更

- **FR**: FR-019, FR-019a
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - 現在の順序（Agent→Model→Version→Session→Permissions→Branch→Docker）を変更
  - 新順序: Branch Mode → Branch Config → Agent → Model + Provider → Version → Reasoning → Session → Permissions → Advanced → Docker
  - HTML/Svelteブロックの移動のみ。ロジック変更なし
- **TDD**: テストを先に書く
  - DOM内のセクション順序を確認するテスト
- **依存**: なし
- **ステータス**: 未着手

## Phase 5: From Issue タブUI

### T-501: Manual/From Issue タブ切替UI

- **FR**: FR-020, FR-020a, FR-021, FR-021a
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - New Branchモード内に「Manual」「From Issue」タブを追加
  - デフォルトは「Manual」
  - gh CLI未検出時: 「From Issue」をdisabled + ツールチップ
- **TDD**: テストを先に書く
  - タブ切替テスト
  - gh CLI未検出時のdisabledテスト
- **依存**: T-301, T-401
- **ステータス**: 未着手

### T-502: Issue一覧リスト表示 + バックグラウンド取得

- **FR**: FR-022, FR-025, FR-025a
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - フォーム開時にバックグラウンドでIssue一覧を取得
  - 各行: `#{number}: {title}` + ラベルバッジ
  - ローディング表示
- **TDD**: テストを先に書く
  - Issue一覧のレンダリングテスト
  - ローディング状態テスト
- **依存**: T-501
- **ステータス**: 未着手

### T-503: テキスト検索フィルタ

- **FR**: FR-023, FR-023a
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - 検索入力欄
  - クライアントサイドでタイトル部分一致フィルタ（大小文字不問）
  - 0件時「No matching issues」表示
- **TDD**: テストを先に書く
  - フィルタリングロジックテスト
- **依存**: T-502
- **ステータス**: 未着手

### T-504: 無限スクロール

- **FR**: FR-024, FR-024a, FR-024b
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - リスト末尾到達時にオンデマンドで次ページ取得
  - 末尾にローディングインジケータ
  - `has_next_page=false`で追加取得停止
- **TDD**: テストを先に書く
  - スクロールトリガーテスト
- **依存**: T-502
- **ステータス**: 未着手

### T-505: 重複ブランチ検出 + disabled表示

- **FR**: FR-026, FR-026a
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - Issue一覧表示時に各Issueの既存ブランチを検出
  - 既存ブランチありのIssueはグレーアウト + 既存ブランチ名表示 + クリック不可
- **TDD**: テストを先に書く
  - disabled行のレンダリングテスト
- **依存**: T-502
- **ステータス**: 未着手

### T-506: Issue選択 → ブランチ名自動生成

- **FR**: FR-027, FR-027a, FR-027b
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - Issueをシングルクリックで選択 → `{prefix}/issue-{number}` ブランチ名を即座に確定
  - ブランチ名は編集不可（読み取り専用表示）
  - 選択解除でManualモードに戻せる
- **TDD**: テストを先に書く
  - Issue選択→ブランチ名生成テスト
  - 読み取り専用表示テスト
- **依存**: T-505
- **ステータス**: 未着手

## Phase 6: Launch連携 + ロールバック

### T-601: Launch フローに gh issue develop ステップを組み込み

- **FR**: FR-028
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`, `crates/gwt-tauri/src/commands/terminal.rs`
- **内容**:
  - Issue選択時のLaunchフロー: Worktree作成 → `gh issue develop` → エージェント起動
  - Manual時は従来通り（Issue連携ステップなし）
- **TDD**: テストを先に書く
  - フロー分岐テスト
- **依存**: T-506
- **ステータス**: 未着手

### T-602: 完全ロールバック実装

- **FR**: FR-029, FR-029a, FR-029b
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`, `crates/gwt-tauri/src/commands/issue.rs`
- **内容**:
  - 各ステップの失敗検知 → ロールバック実行
  - ステップ表示（「Rolling back: deleting worktree...」等）
  - リモートブランチ削除失敗は非致命的エラーとして処理
- **TDD**: テストを先に書く
  - ロールバックステップの順序テスト
  - 部分失敗時の挙動テスト
- **依存**: T-601
- **ステータス**: 未着手

### T-603: レートリミットエラーハンドリング

- **FR**: FR-030
- **ファイル**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **内容**:
  - GitHub APIレートリミットエラーの検出
  - ユーザーへの通知表示
  - Issue一覧取得の中止
- **TDD**: テストを先に書く
  - レートリミットエラー検出テスト
- **依存**: T-502
- **ステータス**: 未着手

## 依存関係グラフ

```text
T-101 ─┐
T-102 ─┼──> T-201 ──> T-301 ──> T-501 ──> T-502 ──> T-503
T-103 ─┘                                     │         T-504
                                              │         T-505 ──> T-506 ──> T-601 ──> T-602
                                              └──> T-603
T-401（独立）
```

## 並列化方針

- **Phase 1の T-101/T-102/T-103 は並列実行可能**
- **T-401（セクション並び替え）は他の全タスクと独立して並列実行可能**
- Phase 5の T-503/T-504/T-505/T-603 は T-502 完了後に並列実行可能
