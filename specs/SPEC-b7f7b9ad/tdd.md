# TDD: メニューショートカット + 終了保護

## テスト観点

### Rust

1. `menu_action_from_id` が `edit-copy` / `edit-paste` を正しく返す
2. `ExitRequested` の挙動（`should_prevent_exit_request`）の基礎ガードは変更しない

### フロントエンド

1. `isCopyShortcut`
   - `ctrl+c`, `cmd+c` を認識
   - `shift` 同時押しは除外

2. `TerminalView` のキーイベント
   - `Cmd/Ctrl + C` の選択コピー
   - `Paste` イベント時に `write_terminal` 呼び出し

3. `TerminalView` のメニュー連携
   - `gwt-terminal-edit-action` の `copy` / `paste` が paneId マッチ時のみ反映される

## 回帰テスト

- `cargo test -p gwt-tauri --no-default-features`
- `pnpm --dir gwt-gui test src/lib/terminal/shortcuts.test.ts src/lib/terminal/TerminalView.test.ts`
- `pnpm --dir gwt-gui check`
