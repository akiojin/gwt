# Tauri コマンドコントラクト: SPEC-a1b2c3d4

## get_system_info

システムの CPU/メモリ/GPU 情報を取得する。

- **コマンド名**: `get_system_info`
- **引数**: なし
- **戻り値**: `SystemInfoResponse`

```text
入力: なし

出力:
{
  "cpu_usage_percent": 45.2,
  "memory_used_bytes": 8800000000,
  "memory_total_bytes": 17179869184,
  "gpu": {
    "name": "Apple M2 Pro",
    "vram_total_bytes": null,
    "vram_used_bytes": null,
    "usage_percent": null
  }
}

NVIDIA 環境の GPU 例:
{
  "name": "NVIDIA GeForce RTX 4090",
  "vram_total_bytes": 25769803776,
  "vram_used_bytes": 4294967296,
  "usage_percent": 35.0
}
```

- GPU が検出できない場合、`gpu` は `null`
- AppState の SystemMonitor を Mutex で保護し、呼び出しごとに refresh

## get_stats

統計データを取得する。

- **コマンド名**: `get_stats`
- **引数**: なし
- **戻り値**: `StatsResponse`

```text
入力: なし

出力:
{
  "global": {
    "agents": [
      { "agent_id": "claude-code", "model": "claude-sonnet-4-5-20250929", "count": 42 },
      { "agent_id": "codex", "model": "o3", "count": 8 }
    ],
    "worktrees_created": 25
  },
  "repos": [
    {
      "repo_path": "/Users/user/projects/my-app",
      "stats": {
        "agents": [
          { "agent_id": "claude-code", "model": "claude-sonnet-4-5-20250929", "count": 20 }
        ],
        "worktrees_created": 10
      }
    }
  ]
}
```

- stats.toml が存在しない場合: `global.agents = [], global.worktrees_created = 0, repos = []`
- stats.toml が破損している場合: 上記と同じ空レスポンス（エラーにしない）
