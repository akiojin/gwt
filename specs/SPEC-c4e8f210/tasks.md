# タスクリスト: Worktree Cleanup（GUI）

## Phase 1: バックエンド

- [ ] T1: `gwt-tauri` に `WorktreeInfo` 型定義と `list_worktrees` コマンド追加 + テスト
- [ ] T2: `gwt-tauri` に `CleanupResult` 型定義と `cleanup_worktrees` コマンド追加 + テスト
- [ ] T3: `gwt-tauri` に `cleanup_single_worktree` コマンド追加 + テスト
- [ ] T4: `gwt-tauri` に `cleanup-progress` / `cleanup-completed` イベント emit 実装

## Phase 2: Sidebar 安全性インジケーター

- [ ] T5: `Sidebar.svelte` ブランチ一覧に安全性 CSS ドットインジケーター追加（緑/黄/赤/グレー）
- [ ] T6: `Sidebar.svelte` 削除中ブランチのスピナー表示 + クリック無効化

## Phase 3: Cleanup モーダル

- [ ] T7: `CleanupModal.svelte` コンポーネント新規作成（テーブル一覧 + チェックボックス + 安全性順ソート）
- [ ] T8: モーダル内 "Select All Safe" ボタン実装
- [ ] T9: モーダル内 "Cleanup" ボタン + unsafe 選択時の確認ダイアログ実装
- [ ] T10: 非同期削除の状態管理（cleanup-progress / cleanup-completed リスナー）
- [ ] T11: 削除失敗時のモーダル再表示 + エラー表示

## Phase 4: トリガー統合

- [ ] T12: Sidebar ヘッダーに Cleanup ボタン追加
- [ ] T13: ブランチ行にコンテキストメニュー追加（Cleanup this branch / Cleanup Worktrees...）
- [ ] T14: 新規 "Git" メニュー追加 + "Cleanup Worktrees..." 配置
- [ ] T15: キーボードショートカット Cmd+Shift+K 登録

## Phase 5: 統合テスト

- [ ] T16: `cargo test` で全バックエンドテストが通る
- [ ] T17: `cargo clippy` で警告なし
- [ ] T18: `svelte-check` でフロントエンドエラーなし
