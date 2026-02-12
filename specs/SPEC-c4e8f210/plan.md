# 実装計画: Worktree Cleanup（GUI）

## 目的

- TUI 版にあった Worktree 一括クリーンアップ機能と安全性表示を GUI 版に実装する
- Sidebar に安全性インジケーターを追加し、Worktree の状態を常に可視化する
- 専用モーダルダイアログで一括クリーンアップを提供する

## 実装方針

### Phase 1: バックエンド（gwt-core / gwt-tauri）

#### gwt-tauri: 新規 Tauri コマンド

- `list_worktrees` コマンド:
  - gwt-core の `WorktreeManager::list()` を呼び出し、各 Worktree の安全性情報を含む `WorktreeInfo` を返す
  - `is_current`, `is_protected`, `is_agent_running` のフラグを追加で判定する
  - `BranchInfo` から `ahead`, `behind`, `is_gone`, `last_tool_usage` を結合する
- `cleanup_worktrees` コマンド:
  - 指定されたブランチ群に対して順次 `remove_with_branch()` を実行する
  - `force` フラグが true の場合は `force_worktree=true`, `force_branch=true` で呼び出す
  - 各ブランチの削除ごとに `cleanup-progress` イベントを emit する
  - 失敗をスキップして継続し、最終的に `cleanup-completed` イベントで結果サマリーを emit する
  - 完了後に `worktrees-changed` イベントを emit する
- `cleanup_single_worktree` コマンド:
  - 単一ブランチの Worktree + ローカルブランチを削除する
  - `cleanup_worktrees` と同じロジックだが、単一ブランチ向け

### Phase 2: フロントエンド - Sidebar 安全性インジケーター

- `Sidebar.svelte` のブランチ一覧に CSS ドットインジケーターを追加:
  - 緑: safe（`has_changes=false` AND `has_unpushed=false`）
  - 黄: warning（`has_changes` XOR `has_unpushed`）
  - 赤: danger（`has_changes=true` AND `has_unpushed=true`）
  - グレー: protected / current
- インジケーターはブランチ名の左に配置する
- 削除中のブランチにはスピナーを表示し、クリックを無効にする

### Phase 3: フロントエンド - Cleanup モーダル

- `CleanupModal.svelte` コンポーネントを新規作成:
  - Worktree 一覧をテーブル形式で表示
  - 各行: チェックボックス + 安全性インジケーター + ブランチ名 + status + changes/unpushed + ahead/behind + gone + tool
  - ソート: 安全性順（safe → warn → danger → disabled）
  - "Select All Safe" ボタン: safe な項目のみを一括チェック
  - "Cleanup" ボタン: 選択された Worktree を削除
  - disabled 行: protected, current, agent running
- unsafe 項目が選択されている場合のみ確認ダイアログを表示
- Cleanup 実行後はモーダルを即座に閉じ、非同期で削除を進行
- 失敗があった場合はモーダルを再表示してエラーを表示

### Phase 4: フロントエンド - トリガー統合

- Sidebar ヘッダーに Cleanup ボタンを追加
- ブランチ行にコンテキストメニューを追加:
  - "Cleanup this branch": 確認ダイアログ → 単体削除
  - "Cleanup Worktrees...": モーダルを開く（該当ブランチをプリセレクト）
- 新規 "Git" メニューをメニューバーに追加し、"Cleanup Worktrees..." を配置
- キーボードショートカット Cmd+Shift+K を登録

### Phase 5: 非同期削除の状態管理

- `App.svelte` で `cleanup-progress` と `cleanup-completed` イベントをリッスン
  - `cleanup-progress`: Sidebar の対象ブランチ行を「削除中」状態に更新
  - `cleanup-completed`: 失敗がある場合はモーダルを再表示、なければ Sidebar をリフレッシュ
- 削除中の状態は Svelte の reactive store で管理する

## テスト

### gwt-core（既存テスト拡充）

- `remove_with_branch()` の正常系/異常系テスト（ロック中、protected branch 等）
- `has_changes`, `has_unpushed` の正確性テスト

### gwt-tauri

- `list_worktrees` コマンド: 安全性情報が正しく結合されるテスト
- `cleanup_worktrees` コマンド:
  - 全成功パターン
  - 一部失敗パターン（スキップ継続の確認）
  - force=true でのunsafe ブランチ削除
  - protected branch の削除拒否
  - current worktree の削除拒否

### gwt-gui（Svelte）

- Sidebar インジケーター: 各安全性レベルの色表示テスト
- Cleanup モーダル: ソート順、Select All Safe、disabled 行の確認
- コンテキストメニュー: 2項目の表示と動作
- 非同期削除: スピナー表示、クリック無効化、完了後のリフレッシュ
