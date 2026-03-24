### Frontend (TypeScript)

#### LaunchAgentRequest

```typescript
interface LaunchAgentRequest {
  agentId: string;                    // エージェント識別子 ("claude"|"codex"|"gemini"|"opencode"|"copilot")
  branch: string;                     // 起動対象ブランチ名
  profile?: string;                   // プロファイル名
  model?: string;                     // モデル選択（エージェント毎の固定リストから選択）
  agentVersion?: string;              // バージョン ("installed"|"latest"|semver|dist-tag)
  mode?: "normal" | "continue" | "resume"; // セッションモード
  skipPermissions?: boolean;          // 権限スキップフラグ
  reasoningLevel?: string;            // Codex 推論レベル (low|medium|high|xhigh)
  fastMode?: boolean;                 // Codex gpt-5.4 限定 Fast mode
  extraArgs?: string[];               // 追加 CLI 引数（改行区切りテキストからパース）
  envOverrides?: Record<string, string>; // 環境変数オーバーライド（最高優先度）
  resumeSessionId?: string;           // Resume/Continue 対象セッション ID
  createBranch?: {                    // 新規ブランチ作成時
    name: string;                     //   ブランチ名 ({prefix}{suffix} or AI 生成名)
    base?: string | null;             //   ベースブランチ名
  };
  dockerService?: string;             // Docker Compose サービス名
  dockerForceHost?: boolean;          // Docker 強制スキップ
  dockerRecreate?: boolean;           // コンテナ再作成フラグ
  dockerBuild?: boolean;              // イメージビルドフラグ
  dockerKeep?: boolean;               // エージェント終了後コンテナ維持
  issueNumber?: number;               // Issue 連携時の Issue 番号
  terminalShell?: string;             // シェル選択 (ShellInfo.id)
  aiBranchDescription?: string;       // AI ブランチ名提案用の説明テキスト
}
```

#### LaunchProgressPayload / LaunchFinishedPayload

```typescript
interface LaunchProgressPayload {
  jobId: string;                      // start_launch_job が返した UUID
  step: "fetch" | "validate" | "paths" | "conflicts" | "create" | "skills" | "deps";
  detail?: string | null;             // ステップ詳細テキスト (例: "waiting for environment")
}

interface LaunchFinishedPayload {
  jobId: string;
  status: "ok" | "cancelled" | "error";
  paneId?: string | null;             // 成功時のターミナルペーン ID
  error?: string | null;              // エラーメッセージ（[E1004] 等のコード含む場合あり）
}
```

#### LaunchDefaults (localStorage)

```typescript
type LaunchDefaults = {
  selectedAgent: string;              // 最後に成功したエージェント ID
  sessionMode: "normal" | "continue" | "resume";
  modelByAgent: Record<string, string>;    // エージェント毎のモデル選択
  agentVersionByAgent: Record<string, string>; // エージェント毎のバージョン選択
  skipPermissions: boolean;
  reasoningLevel: string;
  fastMode: boolean;
  resumeSessionId: string;
  showAdvanced: boolean;              // Advanced セクション展開状態
  extraArgsText: string;              // 改行区切り追加引数テキスト
  envOverridesText: string;           // KEY=VALUE 形式環境変数テキスト
  runtimeTarget: "host" | "docker";
  dockerService: string;
  dockerBuild: boolean;
  dockerRecreate: boolean;
  dockerKeep: boolean;
  selectedShell: string;
  branchNamingMode: "direct" | "ai-suggest";
};

// localStorage エンベロープ
type StoredLaunchDefaults = {
  version: 1;                         // スキーマバージョン
  data: LaunchDefaults;
};
// Storage key: "gwt.launchAgentDefaults.v1"
```

#### DockerContext

```typescript
interface DockerContext {
  worktree_path?: string | null;
  file_type: "compose" | "devcontainer" | "dockerfile" | "none";
  compose_services: string[];         // Docker Compose サービス名リスト
  docker_available: boolean;          // docker コマンド利用可否
  compose_available: boolean;         // docker compose 利用可否
  daemon_running: boolean;            // Docker デーモン起動状態
  force_host: boolean;                // 設定による Docker 強制スキップ
  container_status?: "running" | "stopped" | "not_found" | null;
  images_exist?: boolean | null;      // イメージ存在フラグ
}
```

#### ClassifyResult / BranchSuggestResult

```typescript
interface ClassifyResult {
  status: "ok" | "ai-not-configured" | "error";
  prefix?: string;                    // 分類されたプレフィックス (例: "feature/")
  error?: string;
}

interface BranchSuggestResult {
  status: "ok" | "ai-not-configured" | "error";
  suggestion: string;                 // 提案されたブランチ名
  error?: string;
}
```

#### その他の型

```typescript
type AgentId = "claude" | "codex" | "gemini" | "opencode" | "copilot";
type BranchPrefix = "feature/" | "bugfix/" | "hotfix/" | "release/";
type RuntimeTarget = "host" | "docker";

interface DetectedAgentInfo {
  id: string;
  name: string;
  version: string;
  path?: string | null;
  available: boolean;
}

interface AgentVersionsInfo {
  agent_id: string;
  package: string;
  tags: string[];
  versions: string[];
  source: string;
}

interface GhCliStatus {
  available: boolean;
  authenticated: boolean;
}

interface ShellInfo {
  id: string;
  name: string;
  version?: string | null;
}
```

### Backend (Rust)

全構造体は `#[serde(rename_all = "camelCase")]` でフロントエンドの TypeScript interface と 1:1 マッピング。

追加の内部構造体:

```rust
// CLI コマンド解決結果（内部使用）
struct ResolvedAgentLaunchCommand {
    command: String,            // 実行コマンド (例: "claude", "bunx")
    args: Vec<String>,          // CLI 引数リスト
    label: &'static str,       // エージェント表示名
    tool_version: String,       // "installed" | "latest" | semver | dist-tag
    version_for_gates: Option<String>, // バージョンゲート用バージョン文字列
}

// パッケージランナー選択
enum LaunchRunner { Bunx, Npx }

// ビルトインエージェント定義
pub(crate) struct BuiltinAgentDef {
    pub(crate) label: &'static str,
    pub(crate) local_command: &'static str,  // ローカルコマンド名
    pub(crate) bunx_package: &'static str,   // npm パッケージ名
}
```

### Tauri イベント

| イベント名 | ペイロード | 方向 |
|-----------|-----------|------|
| `launch-progress` | `LaunchProgressPayload` | Backend → Frontend |
| `launch-finished` | `LaunchFinishedPayload` | Backend → Frontend |
| `worktrees-changed` | *(なし)* | Backend → Frontend |

---
