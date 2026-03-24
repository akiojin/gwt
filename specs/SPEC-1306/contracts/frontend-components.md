# フロントエンドコンポーネント契約: Windows シェル選択

**仕様ID**: `SPEC-1350124a` | **日付**: 2026-02-19

## コンポーネント変更

### WorktreeSummaryPanel.svelte

**追加 Props**:

```text
onNewTerminal?: () => void  // New Terminal ボタンクリック時のコールバック
```

**UI 変更**:

- Launch Agent ボタンの横に New Terminal ボタン（`>_`）を追加
- 全 OS で表示

### Sidebar.svelte

**追加 Props**:

```text
onNewTerminal?: () => void  // WorktreeSummaryPanel に伝播
```

### AgentLaunchForm.svelte

**新規 State**:

```text
availableShells: ShellInfo[]  // get_available_shells の結果
selectedShell: string         // 選択中のシェル ID（"" = auto）
```

**UI 変更**:

- Advanced Options セクションにシェル選択ドロップダウンを追加
- `availableShells` が空（macOS/Linux）の場合は非表示
- Docker モード時は disabled + "Container default" テキスト

**handleLaunch() 変更**:

- `LaunchAgentRequest.terminal_shell` に `selectedShell` を設定
- `saveLaunchDefaults()` に `selectedShell` を含める

### SettingsPanel.svelte

**SettingsTabId 拡張**:

```text
type SettingsTabId = "appearance" | "voiceInput" | "mcpBridge" | "profiles" | "terminal"
```

**新規セクション（Terminal タブ）**:

- `get_available_shells` でシェル一覧を取得
- 各シェルの名前 + バージョンをドロップダウンで表示
- 選択値を `SettingsData.default_shell` に保存
- `availableShells` が空（macOS/Linux）の場合はタブ自体を非表示

### App.svelte

**新規ハンドラ**:

```text
handleNewTerminal(worktreePath: string): void
  → invoke("spawn_shell", { workingDir: worktreePath, shell: null })
  → ターミナルタブを作成
```

## Props フロー

```text
App.svelte
  ├── onNewTerminal={handleNewTerminal}
  │   ↓
  ├── Sidebar.svelte
  │   ├── onNewTerminal={onNewTerminal}
  │   │   ↓
  │   └── WorktreeSummaryPanel.svelte
  │       └── [>_ ボタン] → onNewTerminal()
  │
  └── AgentLaunchForm.svelte
      └── [Shell ドロップダウン] → selectedShell → LaunchAgentRequest.terminal_shell
```
