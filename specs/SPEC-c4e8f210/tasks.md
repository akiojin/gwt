# タスクリスト: Worktree Cleanup（GUI）

## Phase 1: バックエンド

- [x] T1: `gwt-tauri` に `WorktreeInfo` 型定義と `list_worktrees` コマンド追加 + テスト
- [x] T2: `gwt-tauri` に `CleanupResult` 型定義と `cleanup_worktrees` コマンド追加 + テスト
- [x] T3: `gwt-tauri` に `cleanup_single_worktree` コマンド追加 + テスト
- [x] T4: `gwt-tauri` に `cleanup-progress` / `cleanup-completed` イベント emit 実装

## Phase 2: Sidebar 安全性インジケーター

- [x] T5: `Sidebar.svelte` ブランチ一覧に安全性 CSS ドットインジケーター追加（緑/黄/赤/グレー）
- [x] T6: `Sidebar.svelte` 削除中ブランチのスピナー表示 + クリック無効化

## Phase 3: Cleanup モーダル

- [x] T7: `CleanupModal.svelte` コンポーネント新規作成（テーブル一覧 + チェックボックス + 安全性順ソート）
- [x] T8: モーダル内 "Select All Safe" ボタン実装
- [x] T9: モーダル内 "Cleanup" ボタン + unsafe 選択時の確認ダイアログ実装
- [x] T10: 非同期削除の状態管理（cleanup-progress / cleanup-completed リスナー）
- [x] T11: 削除失敗時のモーダル再表示 + エラー表示

## Phase 4: トリガー統合

- [x] T12: Sidebar ヘッダーに Cleanup ボタン追加
- [x] T13: ブランチ行にコンテキストメニュー追加（Cleanup this branch / Cleanup Worktrees...）
- [x] T14: 新規 "Git" メニュー追加 + "Cleanup Worktrees..." 配置
- [x] T15: キーボードショートカット Cmd+Shift+K 登録

## Phase 5: 統合テスト

- [x] T16: `cargo test` で全バックエンドテストが通る（67/67）
- [x] T17: `cargo clippy` で警告なし
- [x] T18: `svelte-check` でフロントエンドエラーなし

## Phase 6: Agent Tab Indicator

- [x] T19: Sidebar（Local）で agent tab が開いているブランチに `@` アイコンを表示
- [x] T20: Cleanup モーダルで `@` アイコンを表示し、`@` 付き Worktree を先頭にソート
- [x] T21: Cleanup モーダルを開いたまま Worktree が更新された場合に一覧をリフレッシュ（選択維持）
- [x] T22: agent tab が開いている場合のアイコンを ASCII スピナーとしてアニメーション表示（reduced-motion は `@` 固定表示）
