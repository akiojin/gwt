# データモデル: SPEC-a1b2c3d4

## 1. 統計データ（stats.toml）

### Rust 構造体

```text
Stats
├── global: StatsEntry
│   ├── agents: HashMap<String, u64>    // キー: "{agent_id}.{model}"
│   └── worktrees_created: u64
└── repos: HashMap<String, StatsEntry>  // キー: リポジトリ絶対パス
    ├── agents: HashMap<String, u64>
    └── worktrees_created: u64
```

### TOML 構造

```toml
[global]
worktrees_created = 25

[global.agents]
"claude-code.claude-sonnet-4-5-20250929" = 42
"claude-code.claude-opus-4-6" = 15
"codex.o3" = 8
"gemini.gemini-2.5-pro" = 5
"custom-agent.default" = 3

[repos."/Users/user/projects/my-app"]
worktrees_created = 10

[repos."/Users/user/projects/my-app".agents]
"claude-code.claude-sonnet-4-5-20250929" = 20
"claude-code.claude-opus-4-6" = 5
```

### キー命名規則

- **エージェントキー**: `"{agent_id}.{model}"` 形式
  - agent_id: `"claude-code"`, `"codex"`, `"gemini"`, `"opencode"`, カスタム名
  - model: モデルID。未指定の場合は `"default"`
- **リポジトリキー**: 絶対パス文字列（TOML では引用符で囲む）

## 2. システム情報（Tauri コマンドレスポンス）

### get_system_info レスポンス

```text
SystemInfo
├── cpu_usage_percent: f32           // 0.0 - 100.0
├── memory_used_bytes: u64
├── memory_total_bytes: u64
└── gpu: Option<GpuInfo>
    ├── name: String                 // "NVIDIA GeForce RTX 4090"
    ├── vram_total_bytes: Option<u64>
    ├── vram_used_bytes: Option<u64> // NVIDIA のみ
    └── usage_percent: Option<f32>   // NVIDIA のみ
```

### get_stats レスポンス

```text
StatsResponse
├── global: StatsEntryResponse
│   ├── agents: Vec<AgentStatEntry>
│   │   ├── agent_id: String
│   │   ├── model: String
│   │   └── count: u64
│   └── worktrees_created: u64
└── repos: Vec<RepoStatsEntry>
    ├── repo_path: String
    └── stats: StatsEntryResponse
```

## 3. ファイルシステム

| パス | 形式 | 用途 |
|---|---|---|
| `~/.gwt/stats.toml` | TOML | 統計データ永続化 |

書き込みは temp ファイル（`~/.gwt/stats.toml.tmp`）→ rename のアトミックパターン。
