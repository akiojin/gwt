# 実装計画: GitHub Issue連携によるブランチ作成（GUI版）

**仕様ID**: `SPEC-c6ba640a` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-c6ba640a/spec.md`

## 目的

- TUI版に存在したGitHub Issue→ブランチ作成機能をGUI版Agent Launch Formに移植する
- AgentLaunchFormのセクション順序をTUI版Wizardの流れに合わせて再配置する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）、gwt-core（`crates/gwt-core/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **外部連携**: GitHub CLI (`gh`) - Issue取得、ブランチリンク
- **テスト**: `cargo test`（Rust）、`vitest`（Svelte）
- **前提**: gwt-core `git/issue.rs` に既存のIssue操作ロジックあり（660行）

## 実装方針

### Phase 1: gwt-core API拡張（FR-001〜FR-003）

- `fetch_open_issues`にページネーション引数（`page`, `per_page`）を追加
- `GitHubIssue`に`labels`フィールドを追加、JSON取得フィールドに`labels`を追加
- `is_gh_cli_authenticated`関数を新規追加（`gh auth status`）
- 既存テストの更新 + 新規テスト追加

**対象ファイル**:

- `crates/gwt-core/src/git/issue.rs`（変更）

### Phase 2: Tauriコマンド公開（FR-010〜FR-014）

- `fetch_github_issues` コマンド: ページネーション対応Issue取得
- `check_gh_cli_status` コマンド: gh CLI利用可否チェック
- `find_existing_issue_branch` コマンド: 重複ブランチ検出
- `link_branch_to_issue` コマンド: `gh issue develop`実行
- `rollback_issue_branch` コマンド: 完全ロールバック（ローカル+リモート削除）

**対象ファイル**:

- `crates/gwt-tauri/src/commands/issue.rs`（新規）
- `crates/gwt-tauri/src/commands/mod.rs`（変更: モジュール登録）
- `crates/gwt-tauri/src/lib.rs`（変更: コマンド登録）

### Phase 3: TypeScript型定義（FR-010a〜FR-014a）

- `GitHubIssue` 型定義
- `GhCliStatus` 型定義
- `FetchIssuesResponse` 型定義
- Tauriコマンド呼び出しラッパー

**対象ファイル**:

- `gwt-gui/src/lib/types.ts`（変更）

### Phase 4: AgentLaunchFormセクション並び替え（FR-019）

- 現在の順序（Agent→Model→Version→Session→Permissions→Branch→Docker）を
  TUI版の流れ（Branch→Agent→Model→Version→Session→Permissions→Docker）に変更
- 既存の機能は一切変更せず、HTML/Svelteブロックの順序のみ変更

**対象ファイル**:

- `gwt-gui/src/lib/components/AgentLaunchForm.svelte`（変更）

### Phase 5: From Issue タブUI実装（FR-020〜FR-030）

- New Branchモード内に「Manual」「From Issue」タブ切替を追加
- Issue一覧リスト（検索フィルタ + 無限スクロール + 重複ブランチdisabled表示）
- gh CLI未検出時のタブdisabled + ツールチップ
- Issue選択時のブランチ名自動生成（編集不可）
- シングルクリックで即座に確定

**対象ファイル**:

- `gwt-gui/src/lib/components/AgentLaunchForm.svelte`（変更）

### Phase 6: Launch連携 + ロールバック（FR-028〜FR-029）

- Launch実行フローに`gh issue develop`ステップを組み込み
- 失敗時の完全ロールバック（ステップ表示付き）
- レートリミットエラーのハンドリング

**対象ファイル**:

- `gwt-gui/src/lib/components/AgentLaunchForm.svelte`（変更）
- `crates/gwt-tauri/src/commands/terminal.rs`（変更: start_launch_job内にIssue連携ステップ追加）

## テスト

### バックエンド（cargo test）

- `fetch_open_issues` ページネーション引数テスト
- `issue_list_args` にlimit/offset引数が反映されるテスト
- `GitHubIssue` labels フィールドの JSON パースフテスト
- `is_gh_cli_authenticated` のモックテスト
- `rollback_issue_branch` のステップ順序テスト

### フロントエンド（vitest）

- AgentLaunchFormのセクション順序テスト（DOMの並び順確認）
- 「Manual」「From Issue」タブ切替テスト
- gh CLI未検出時のタブdisabled表示テスト
- Issue選択→ブランチ名自動生成テスト
- 無限スクロールの次ページ取得トリガーテスト
- 重複ブランチIssueのdisabled表示テスト
