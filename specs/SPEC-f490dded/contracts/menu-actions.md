# API コントラクト: メニューアクション

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-13

## メニュー構成

### Tools メニュー（変更後）

```
Tools
├── New Terminal        (Ctrl+`)   ← 新規
├── Launch Agent...
├── List Terminals
└── Terminal Diagnostics
```

## メニューアクション

### new-terminal

**トリガー**: Tools > New Terminal メニュー項目 または Ctrl+` ショートカット

**menu-action イベントペイロード**: `"new-terminal"`

**フロントエンド処理フロー**:

1. 起動ディレクトリを決定:
   - `selectedBranch` あり → Worktree パスを解決
   - `selectedBranch` なし && `projectPath` あり → プロジェクトルート
   - `projectPath` なし → `null`（バックエンドがホームDirを使用）
2. `invoke("spawn_shell", { workingDir })` を呼び出し
3. 返却された pane_id で新しい Tab を作成:
   - `id: "terminal-{paneId}"`
   - `type: "terminal"`
   - `paneId: paneId`
   - `cwd: workingDir || $HOME`
   - `label: basename(cwd)`
4. タブを `tabs` 配列に追加し、`activeTabId` を設定
5. `sync_window_agent_tabs()` で Window メニューを更新
