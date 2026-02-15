# データモデル: シンプルターミナルタブ

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-13

## バックエンド（Rust）

### 既存型の利用（変更なし）

```
BuiltinLaunchConfig {
    command: String,        // $SHELL or "/bin/sh"
    args: Vec<String>,      // [] (empty)
    working_dir: PathBuf,   // Worktree path / project root / home
    branch_name: String,    // "" (empty for terminal tabs)
    agent_name: String,     // "terminal"
    agent_color: AgentColor, // White
    env_vars: HashMap<String, String>, // {}
}
```

### 新規: OSC 7 パース結果

```
Osc7Cwd {
    path: String,  // decoded file path (e.g., "/Users/akio/Documents")
}
```

### 新規: Tauri イベントペイロード

```
TerminalCwdChanged {
    pane_id: String,
    cwd: String,
}
```

### 新規: SpawnShellRequest（Tauri コマンド引数）

```
SpawnShellRequest {
    working_dir: Option<String>,  // None → home directory
}
```

## フロントエンド（TypeScript）

### 既存型の拡張

```typescript
interface Tab {
  id: string;
  label: string;
  agentId?: "claude" | "codex" | "gemini" | "opencode";
  type: "summary" | "agent" | "settings" | "versionHistory" | "agentMode" | "terminal";  // ← "terminal" 追加
  paneId?: string;
  cwd?: string;  // ← 新規: ターミナルタブの現在の作業ディレクトリ（フルパス）
}
```

### 永続化スキーマ（localStorage）

```typescript
// 既存の StoredAgentTab を拡張
interface StoredAgentTab {
  paneId: string;
  label: string;
  type?: "terminal";  // ← 新規: 省略時は "agent" と見なす
  cwd?: string;       // ← 新規: terminal タブの最新 cwd
}
```

### Window メニュー同期

```typescript
// 既存の WindowAgentTabEntry を拡張
interface WindowAgentTabEntry {
  id: string;
  label: string;
  tabType?: string;  // ← 新規: "terminal" or 省略（= "agent"）
}
```

## 状態管理フロー

```
[spawn_shell] → pane_id
     ↓
Tab { id: "terminal-{paneId}", type: "terminal", paneId, cwd: workingDir }
     ↓
[stream_pty_output] → OSC 7 検出 → terminal-cwd-changed event
     ↓
Tab.cwd 更新 → Tab.label 更新（basename）
     ↓
[永続化] → localStorage に { paneId, label, type: "terminal", cwd }
```
