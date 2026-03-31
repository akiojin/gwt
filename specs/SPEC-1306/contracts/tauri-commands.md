# Tauri コマンド契約: Windows シェル選択

**仕様ID**: `SPEC-1350124a` | **日付**: 2026-02-19

## 新規コマンド

### `get_available_shells`

Windows 環境で利用可能なシェルの一覧を返す。macOS/Linux では空配列。

- **引数**: なし
- **戻り値**: `Vec<ShellInfo>`
- **エラー**: なし（常に成功、空配列を返す場合あり）

```text
Request:  invoke("get_available_shells")
Response: [
  { "id": "powershell", "name": "PowerShell", "version": "7.4.1" },
  { "id": "cmd",        "name": "Command Prompt", "version": null },
  { "id": "wsl",        "name": "WSL",            "version": null }
]
```

macOS/Linux の場合:

```text
Response: []
```

## 既存コマンド拡張

### `spawn_shell` — `shell` 引数追加

- **既存引数**: `working_dir: Option<String>`
- **追加引数**: `shell: Option<String>`
- **戻り値**: `String`（pane_id）
- **エラー**: シェルが見つからない場合

```text
Request:  invoke("spawn_shell", { workingDir: "/path/to/worktree", shell: "wsl" })
Response: "pane-12345"
```

`shell` が `null` の場合: Settings の `default_shell` → 自動検出順

### `launch_agent` — `terminal_shell` フィールド追加

`LaunchAgentRequest` に `terminal_shell` フィールドを追加。

```text
Request:  invoke("launch_agent", {
  request: {
    agentId: "claude",
    branch: "main",
    terminalShell: "wsl",
    ...
  }
})
```

`terminalShell` が未指定の場合: Settings の `default_shell` → 自動検出順

### `save_settings` / `get_settings` — `default_shell` フィールド追加

`SettingsData` に `default_shell` フィールドを追加。

```text
// get_settings Response に追加
{ ..., "default_shell": "powershell" }

// save_settings Request に追加
invoke("save_settings", { settings: { ..., default_shell: "wsl" } })
```
