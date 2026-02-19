# データモデル: Windows シェル選択（Launch Agent / New Terminal）

**仕様ID**: `SPEC-1350124a` | **日付**: 2026-02-19

## Rust 側

### WindowsShell enum (`crates/gwt-core/src/terminal/shell.rs` 新規)

```text
WindowsShell
├── PowerShell    # id: "powershell", display: "PowerShell"
├── Cmd           # id: "cmd",        display: "Command Prompt"
└── Wsl           # id: "wsl",        display: "WSL"
```

- `id() -> &str`: 設定ファイル・IPC で使用する識別子
- `display_name() -> &str`: UI 表示名
- `is_available() -> bool`: 環境にインストールされているかを検証
- `detect_version() -> Option<String>`: バージョン文字列を取得（PowerShell のみ）
- Serialize/Deserialize 対応（serde）

### ShellInfo 構造体 (`crates/gwt-tauri/src/commands/terminal.rs`)

```text
ShellInfo {
    id: String,           // "powershell" | "cmd" | "wsl"
    name: String,         // "PowerShell" | "Command Prompt" | "WSL"
    version: Option<String>  // "7.4.1" | "5.1" | None
}
```

- Tauri コマンド `get_available_shells` の戻り値要素
- Serialize 対応（serde）

### TerminalSettings 構造体 (`crates/gwt-core/src/config/settings.rs`)

```text
TerminalSettings {
    default_shell: Option<String>  // None | "powershell" | "cmd" | "wsl"
}
```

- `Settings` 構造体に `pub terminal: TerminalSettings` を追加
- config.toml の `[terminal]` セクションにマッピング
- `Default` 実装: `default_shell = None`（自動検出順）

### LaunchAgentRequest 拡張 (`crates/gwt-tauri/src/commands/terminal.rs`)

```text
LaunchAgentRequest {
    ... (既存フィールド)
    terminal_shell: Option<String>  // 追加: None | "powershell" | "cmd" | "wsl"
}
```

- `#[serde(default)]` で後方互換性を保証
- `None` 時は Settings の `default_shell` → 自動検出順

### spawn_shell 拡張 (`crates/gwt-tauri/src/commands/terminal.rs`)

```text
spawn_shell(
    working_dir: Option<String>,  // 既存
    shell: Option<String>,         // 追加: None | "powershell" | "cmd" | "wsl"
    state: State<AppState>,
    app_handle: AppHandle
) -> Result<String, String>
```

## TypeScript 側

### ShellInfo インターフェース (`gwt-gui/src/lib/types.ts`)

```text
ShellInfo {
    id: string       // "powershell" | "cmd" | "wsl"
    name: string     // "PowerShell" | "Command Prompt" | "WSL"
    version?: string // "7.4.1" | "5.1" | undefined
}
```

### SettingsData 拡張 (`gwt-gui/src/lib/types.ts`)

```text
SettingsData {
    ... (既存フィールド)
    default_shell?: string | null  // 追加
}
```

### LaunchAgentRequest 拡張 (`gwt-gui/src/lib/types.ts`)

```text
LaunchAgentRequest {
    ... (既存フィールド)
    terminal_shell?: string  // 追加
}
```

### LaunchDefaults 拡張 (`gwt-gui/src/lib/agentLaunchDefaults.ts`)

```text
LaunchDefaults {
    ... (既存フィールド)
    selectedShell: string  // 追加: "" (auto) | "powershell" | "cmd" | "wsl"
}
```

## 設定ファイル (`~/.gwt/config.toml`)

```text
[terminal]
default_shell = "powershell"  # または "cmd" | "wsl" | 未設定
```

## データフロー

```text
[Settings Terminal タブ]
    ↓ save_settings
[config.toml: terminal.default_shell]
    ↓ load
[spawn_shell / launch_agent コマンド]
    ↓ shell 引数 or default_shell
[resolve_shell_for_terminal()]
    ├── "powershell" → resolve_windows_shell() (既存)
    ├── "cmd" → "cmd.exe"
    └── "wsl" → "wsl.exe" + パス変換 + プロンプト検出

[AgentLaunchForm]
    ↓ selectedShell
[LaunchAgentRequest.terminal_shell]
    ↓ saveLaunchDefaults
[localStorage: gwt.launchDefaults.v1]
```
